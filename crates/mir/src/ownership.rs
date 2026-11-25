//! Ownership and borrow checking on MIR (`specs/memory.md`: Rules): the `borrowck` query.
//!
//! The check is intraprocedural and flow-sensitive, in the style of Rust's non-lexical
//! lifetimes:
//!
//! - **Moves.** A forward analysis tracks the places that may have been moved out of. Using one
//!   again, or part of one, is an error (E0303) until it is assigned. Moving out of a place
//!   reached through a borrow is an error (E0304).
//! - **Loans.** Each `&` or `&mut` creates a loan of a place. A forward analysis tracks which
//!   loans each local may hold (copies, reborrows and calls returning borrows pass them on), and
//!   a backward liveness analysis says which locals are used later. A loan is *live* where a local
//!   holding it is live, so a borrow ends at its last use.
//! - **Conflicts.** Where a loan is live, its place may not be borrowed in a conflicting way
//!   (E0305), assigned or moved (E0306), or, for a `&mut` loan, used at all (E0307); and the
//!   local it borrows may not leave scope (E0308).
//! - **Returns.** A returned borrow must come from a borrowed parameter that the signature allows
//!   (E0308, E0310).
//! - **Drops.** Dropping a value is like moving it out, except that one that may have been moved
//!   already is not an error (a drop flag decides at run time). A value whose type has a
//!   destructor cannot be partly moved out of (E0314), except by its own destructor.

use std::collections::{BTreeSet, HashSet};

use moddil_sema::{
    Code, Container, Db, Diagnostic, Item, ItemTree, Owner, Program, Severity, Src, Ty,
};
use moddil_syntax::TextRange;

use crate::{Body, Local, Operand, Place, Projection, Rvalue, Statement, Terminator, mir};

/// Every diagnostic for `program`: the front end's, then, if it found no errors, the ownership
/// and borrow errors of each function and test.
#[salsa::tracked(returns(ref))]
pub fn diagnostics(db: &dyn Db, program: Program) -> Vec<Diagnostic> {
    let mut out = moddil_sema::diagnostics(db, program).clone();
    if !moddil_sema::has_errors(db, program) {
        let items = moddil_sema::item_tree(db, program);
        for owner in moddil_sema::owners(db, program) {
            out.extend(moddil_sema::in_file(
                items,
                owner.item(db),
                borrowck(db, owner),
            ));
            // Its closures' bodies too.
            let hir = &moddil_sema::body(db, owner).body;
            for (e, x) in hir.exprs.iter() {
                if let moddil_sema::hir::ExprKind::Closure { .. } = x.kind {
                    let item = Item::closure(owner.item(db), e.into_raw().into_u32());
                    let closure = Owner::new(db, program, item);
                    out.extend(moddil_sema::in_file(items, item, borrowck(db, closure)));
                }
            }
        }
        out.sort_by_key(|d| (d.file, d.range.start, d.range.end, d.code));
        out.dedup();
    }
    out
}

/// Whether `program` has any errors, including ownership and borrow errors.
pub fn has_errors(db: &dyn Db, program: Program) -> bool {
    diagnostics(db, program)
        .iter()
        .any(|d| d.severity == Severity::Error)
}

/// The ownership and borrow errors in `owner`.
#[salsa::tracked(returns(ref))]
pub fn borrowck<'db>(db: &'db dyn Db, owner: Owner<'db>) -> Vec<Diagnostic> {
    let Some(body) = mir(db, owner).as_ref() else {
        return Vec::new();
    };
    let items = moddil_sema::item_tree(db, owner.program(db));
    let ret_from = match owner.item(db) {
        // An `impl Trait` result may keep the handlers the function was given, after its
        // parameters (`type.impl.ret.2`).
        Item::Fn(i) if matches!(items.fns[i as usize].ret, Ty::Opaque(_)) => {
            let sig = &items.fns[i as usize];
            let mut from = sig.ret_from.clone();
            from.extend(sig.params.len() as u32..body.arg_count as u32);
            from
        }
        Item::Fn(i) => items.fns[i as usize].ret_from.clone(),
        Item::Test(_) => Vec::new(),
        // A closure may return a borrow of what it captures or is given.
        Item::Closure(..) => vec![0, 1],
    };
    // A destructor may move the fields out of its `self`.
    let drop = items.lang("Drop");
    let destructor = match owner.item(db) {
        Item::Fn(i) => match items.fns[i as usize].container {
            Container::Impl(k) => {
                drop.is_some()
                    && items.impls[k as usize].trait_ref.as_ref().map(|t| t.trait_) == drop
            }
            _ => false,
        },
        _ => false,
    };
    // An impl's method whose trait's declared result holds no borrow (`view.borrow.1`).
    let returns_held = match owner.item(db) {
        Item::Fn(i) => {
            let sig = &items.fns[i as usize];
            match sig.container {
                Container::Impl(k) => items.impls[k as usize].trait_ref.as_ref().is_some_and(|t| {
                    items.traits[t.trait_ as usize]
                        .methods
                        .iter()
                        .find(|&&f| items.fns[f as usize].name == sig.name)
                        .is_some_and(|&f| !items.is_view(&items.fns[f as usize].ret))
                        && items.is_view(&sig.ret)
                }),
                _ => false,
            }
        }
        _ => false,
    };
    let mut cx = Checker::new(body, items, ret_from);
    cx.destructor = destructor;
    cx.returns_held = returns_held;
    if let Item::Fn(i) = owner.item(db) {
        cx.written_params = items.fns[i as usize].params.len() as u32;
    }
    cx.run();
    cx.diagnostics
}

/// A loan: a borrow of a place made at some point, or the borrow a parameter arrives with.
#[derive(Clone, Debug)]
struct Loan {
    place: Place,
    mutable: bool,
    span: TextRange,
    /// For the borrow a parameter holds on entry: which parameter (0-based).
    param: Option<u32>,
    /// A borrow that only stops the matched value changing while guards run: it conflicts with
    /// assignments and `&mut` borrows, not with reads or moves.
    guard: bool,
}

/// A point in the body: a block and a statement index (the terminator is at `statements.len()`).
type Point = (usize, usize);

/// How a place is used.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Access {
    Read,
    Move,
    Write,
    Borrow(bool),
}

struct Checker<'a> {
    body: &'a Body,
    items: &'a ItemTree,
    ret_from: Vec<u32>,
    loans: Vec<Loan>,
    /// The loan created at each `Rvalue::Ref`.
    loan_at: std::collections::HashMap<Point, usize>,
    /// Locals live on exit from each point.
    live_out: Vec<Vec<HashSet<Local>>>,
    diagnostics: Vec<Diagnostic>,
    reported: HashSet<(Code, u32, u32)>,
    /// Whether the body is an impl's method whose trait declares a result that holds no borrow
    /// (`fn next(&mut self) -> Option<Self::Item>`): its callers take a borrow it returns as one
    /// of what its arguments hold (`view.borrow.1`), so it may return only those.
    returns_held: bool,
    /// How many of its parameters are written: the handlers it is given come after them.
    written_params: u32,
    /// Whether the body is a destructor, which may move fields out of its `self` (`_1`).
    destructor: bool,
    /// The locals holding the address of a const item's static data (`comptime.static.1`).
    statics: HashSet<Local>,
}

/// The loans each local may hold.
/// Who holds each loan: by local, and, last, the loans kept in a view the caller lent, which live
/// until the body ends.
type Holders = Vec<BTreeSet<usize>>;

/// The places that may have been moved out of, or have no value yet.
#[derive(Clone, Default)]
struct Moved {
    places: BTreeSet<PlaceKey>,
    /// The locals declared without a value (`let x;`) that may have been given one.
    given: BTreeSet<u32>,
}

impl Moved {
    /// On entry: the locals declared without a value have none yet (`stmt.let.uninit.1`).
    fn entry(body: &Body) -> Moved {
        Moved {
            places: body
                .late
                .iter()
                .map(|&(l, _)| PlaceKey::of(&Place::local(l)))
                .collect(),
            given: BTreeSet::new(),
        }
    }

    /// Adds what `other` may have moved or given; whether that changed anything.
    fn join(&mut self, other: &Moved) -> bool {
        let before = (self.places.len(), self.given.len());
        self.places.extend(other.places.iter().cloned());
        self.given.extend(other.given.iter().copied());
        before != (self.places.len(), self.given.len())
    }
}

/// A place without `Deref`, ordered so it can live in a set.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct PlaceKey {
    local: u32,
    path: Vec<(u8, u32)>,
}

impl PlaceKey {
    fn of(p: &Place) -> PlaceKey {
        PlaceKey {
            local: p.local.0,
            path: p
                .projection
                .iter()
                .map(|e| match e {
                    Projection::Field(i) => (0, *i),
                    // Different variants share the same storage, so they overlap.
                    Projection::Downcast(_) => (1, 0),
                    Projection::Deref => (2, 0),
                    // Which element is known only when it runs, so any two overlap.
                    Projection::Index(_) | Projection::Subslice(..) => (3, 0),
                })
                .collect(),
        }
    }

    /// Whether one of the places contains the other.
    fn overlaps(&self, other: &PlaceKey) -> bool {
        self.local == other.local && self.path.iter().zip(&other.path).all(|(a, b)| a == b)
    }

    /// Whether `self` contains `other` (or is it).
    fn contains(&self, other: &PlaceKey) -> bool {
        self.local == other.local && other.path.starts_with(&self.path)
    }

    /// Whether `self` is reached through a dereference of `other`, as `(*r).x` is of `r`: it does
    /// not live in `other`, so assigning `other` does not touch it.
    fn through(&self, other: &PlaceKey) -> bool {
        self.local == other.local
            && self.path.len() > other.path.len()
            && self.path.starts_with(&other.path)
            && self.path[other.path.len()] == (2, 0)
    }
}

/// The part of `p` before its first `Deref`: the local storage a use of `p` touches directly.
fn direct(p: &Place) -> Place {
    let end = p
        .projection
        .iter()
        .position(|e| *e == Projection::Deref)
        .unwrap_or(p.projection.len());
    Place {
        local: p.local,
        projection: p.projection[..end].to_vec(),
    }
}

impl<'a> Checker<'a> {
    fn new(body: &'a Body, items: &'a ItemTree, ret_from: Vec<u32>) -> Self {
        let mut loans = Vec::new();
        // Each borrowed parameter, and each view, arrives holding a borrow of something outside
        // the function.
        for i in 0..body.arg_count {
            let ty = &body.locals[i + 1].ty;
            if items.is_view(ty) {
                loans.push(Loan {
                    place: Place::local(Local(i as u32 + 1)).deref(),
                    mutable: matches!(ty, Ty::Ref(true, _)),
                    span: TextRange::default(),
                    param: Some(i as u32),
                    guard: false,
                });
            }
        }
        let mut loan_at = std::collections::HashMap::new();
        for (b, block) in body.blocks.iter().enumerate() {
            for (s, stmt) in block.statements.iter().enumerate() {
                if let Statement::Assign(dest, Rvalue::Ref(m, place)) = stmt {
                    loan_at.insert((b, s), loans.len());
                    let guard = body.locals[dest.local.index()].name.is_none()
                        && !*m
                        && is_guard_borrow(body, dest.local);
                    loans.push(Loan {
                        place: place.clone(),
                        mutable: *m,
                        span: block.spans[s],
                        param: None,
                        guard,
                    });
                }
            }
        }
        let live_out = liveness(body);
        Checker {
            body,
            items,
            ret_from,
            loans,
            loan_at,
            live_out,
            diagnostics: Vec::new(),
            reported: HashSet::new(),
            returns_held: false,
            written_params: u32::MAX,
            destructor: false,
            statics: body
                .blocks
                .iter()
                .flat_map(|b| &b.statements)
                .filter_map(|st| match st {
                    Statement::Assign(p, Rvalue::StaticRef(_) | Rvalue::StaticBlock(_))
                        if p.projection.is_empty() =>
                    {
                        Some(p.local)
                    }
                    _ => None,
                })
                .collect(),
        }
    }

    /// The type of the part of `p` before its projection `upto`.
    fn type_upto(&self, p: &Place, upto: usize) -> Ty {
        let mut ty = self.body.locals[p.local.index()].ty.clone();
        let mut variant = 0;
        for proj in &p.projection[..upto] {
            ty = match (proj, &ty) {
                (Projection::Deref, Ty::Ref(_, t) | Ty::Ptr(_, t))
                | (Projection::Index(_), Ty::Array(t, _) | Ty::Slice(t)) => (**t).clone(),
                (Projection::Subslice(..), Ty::Array(t, _) | Ty::Slice(t)) => Ty::Slice(t.clone()),
                (Projection::Downcast(v), t) => {
                    variant = *v;
                    t.clone()
                }
                (Projection::Field(i), Ty::Tuple(ts)) => ts[*i as usize].clone(),
                (Projection::Field(i), t @ Ty::Adt(..)) => {
                    let f = self.items.field_types(t, variant);
                    variant = 0;
                    f.get(*i as usize).cloned().unwrap_or(Ty::Error)
                }
                _ => Ty::Error,
            };
        }
        ty
    }

    /// Moving out a field of a value whose type has a destructor (E0314): the destructor would
    /// then see a value with a part missing.
    fn move_out_of_drop(&mut self, span: TextRange, p: &Place) {
        for (k, proj) in p.projection.iter().enumerate() {
            if !matches!(proj, Projection::Field(_)) {
                continue;
            }
            let base = self.type_upto(p, k);
            if let Some(a) = base.adt()
                && self.items.has_drop_impl(a)
                && !(self.destructor && p.local == Local(1) && k == 0)
            {
                let what = self.name(p);
                let ty = self.items.adts[a as usize].name.clone();
                self.report(Diagnostic::error(
                    Code::E0314,
                    span,
                    format!(
                        "{what} cannot be moved out: `{ty}` has a destructor, which needs all of \
                         the value (borrow it, or match it by reference)"
                    ),
                ));
                return;
            }
        }
    }

    fn report(&mut self, d: Diagnostic) {
        if self.reported.insert((d.code, d.range.start, d.range.end)) {
            self.diagnostics.push(d);
        }
    }

    fn name(&self, p: &Place) -> String {
        let mut s = self.body.locals[p.local.index()]
            .name
            .clone()
            .unwrap_or_else(|| "a temporary".to_string());
        let mut indirect = false;
        for e in &p.projection {
            match e {
                Projection::Field(i) => s = format!("{s}.{i}"),
                Projection::Downcast(_) => {}
                Projection::Deref => {
                    indirect = true;
                    s = format!("*{s}");
                }
                Projection::Index(_) => s = format!("{s}[…]"),
                Projection::Subslice(..) => s = format!("{s}[…..…]"),
            }
        }
        let _ = indirect;
        // A name that is a description already (a handler's) is not quoted.
        if s.starts_with("the ") {
            return s;
        }
        format!("`{s}`")
    }

    fn run(&mut self) {
        let n = self.body.blocks.len();
        if n == 0 {
            return;
        }
        let locals = self.body.locals.len();
        // Forward analyses to a fixed point: loan holders and moved places at each block entry.
        let mut holders_in: Vec<Option<Holders>> = vec![None; n];
        let mut moved_in: Vec<Option<Moved>> = vec![None; n];
        let mut entry = vec![BTreeSet::new(); locals + 1];
        for (i, l) in self.loans.iter().enumerate() {
            if l.param.is_some() {
                entry[l.place.local.index()].insert(i);
            }
        }
        holders_in[0] = Some(entry);
        moved_in[0] = Some(Moved::entry(self.body));
        let mut work: Vec<usize> = vec![0];
        while let Some(b) = work.pop() {
            let (mut h, mut m) = (
                holders_in[b].clone().expect("a state"),
                moved_in[b].clone().expect("a state"),
            );
            let block = &self.body.blocks[b];
            for (s, stmt) in block.statements.iter().enumerate() {
                self.step(b, s, stmt, &mut h, &mut m, false);
            }
            self.step_terminator(b, &mut h, &mut m, false);
            for succ in block.terminator.successors() {
                let i = succ.index();
                let changed = match (&mut holders_in[i], &mut moved_in[i]) {
                    (Some(hi), Some(mi)) => {
                        let mut changed = false;
                        for (a, b) in hi.iter_mut().zip(&h) {
                            let before = a.len();
                            a.extend(b.iter().copied());
                            changed |= a.len() != before;
                        }
                        changed | mi.join(&m)
                    }
                    (hi, mi) => {
                        *hi = Some(h.clone());
                        *mi = Some(m.clone());
                        true
                    }
                };
                if changed {
                    work.push(i);
                }
            }
        }
        // Check each reachable point against the fixed point.
        for b in 0..n {
            let (Some(mut h), Some(mut m)) = (holders_in[b].clone(), moved_in[b].clone()) else {
                continue;
            };
            let block = &self.body.blocks[b];
            for (s, stmt) in block.statements.iter().enumerate() {
                self.step(b, s, stmt, &mut h, &mut m, true);
            }
            self.step_terminator(b, &mut h, &mut m, true);
        }
    }

    /// The loans live after point `at`: those held by a local used later.
    fn live_loans(&self, at: Point, h: &Holders) -> BTreeSet<usize> {
        let live = &self.live_out[at.0][at.1];
        live.iter()
            .flat_map(|l| h[l.index()].iter().copied())
            .chain(h.last().into_iter().flatten().copied())
            .collect()
    }

    /// Checks one access of `place` against the moved places and the live loans.
    fn access(
        &mut self,
        (at, span): (Point, TextRange),
        place: &Place,
        kind: Access,
        h: &Holders,
        m: &Moved,
        except: Option<usize>,
    ) {
        // Use after move: anything touching a place that may have been moved out of.
        let key = PlaceKey::of(&direct(place));
        if kind != Access::Write || !place.projection.is_empty() {
            let moved_part = m.places.iter().find(|mv| {
                if kind == Access::Write {
                    // Assigning a part of a moved value is an error; assigning all of it is not.
                    mv.contains(&key) && *mv != &key
                } else {
                    mv.overlaps(&key)
                }
            });
            if let Some(mv) = moved_part {
                let what = self.name(&direct(place));
                // A local declared without a value that no path to here gives one.
                let unset = mv.path.is_empty()
                    && self.body.late.iter().any(|&(l, _)| l.0 == mv.local)
                    && !m.given.contains(&mv.local);
                let why = if unset {
                    format!("{what} is used before it has a value: assign it first")
                } else {
                    format!("{what} was moved out of earlier, so it cannot be used here")
                };
                self.report(Diagnostic::error(Code::E0303, span, why));
            }
        }
        // A local declared without a value and not `mut` is given one once.
        if kind == Access::Write
            && place.projection.is_empty()
            && m.given.contains(&place.local.0)
            && self.body.late.contains(&(place.local, false))
        {
            let what = self.name(place);
            self.report(Diagnostic::error(
                Code::E0301,
                span,
                format!(
                    "{what} may already have a value, and it is not declared `mut`, so it cannot                      be assigned again"
                ),
            ));
        }
        if kind == Access::Move
            && place
                .projection
                .iter()
                .any(|p| matches!(p, Projection::Index(_)))
        {
            let what = self.name(place);
            self.report(Diagnostic::error(
                Code::E0315,
                span,
                format!(
                    "{what} is an element of an array or slice, which cannot be moved out of                      (it is not `Copy`): swap or clone it instead"
                ),
            ));
        } else if kind == Access::Move && place.is_indirect() && self.statics.contains(&place.local)
        {
            self.report(Diagnostic::error(
                Code::E1009,
                span,
                "a const that is not `Copy` is static data, which is borrowed, not moved: \
                 borrow it (`&NAME`), or clone it",
            ));
        } else if kind == Access::Move && place.is_indirect() {
            let what = self.name(place);
            self.report(Diagnostic::error(
                Code::E0304,
                span,
                format!(
                    "{what} is behind a borrow, which does not own it, so it cannot be moved out \
                     (it is not `Copy`)"
                ),
            ));
        }
        // Conflicts with live loans.
        let full = PlaceKey::of(place);
        for li in self.live_loans(at, h) {
            if Some(li) == except {
                continue;
            }
            let loan = &self.loans[li];
            let key = PlaceKey::of(&loan.place);
            if loan.param.is_some() || !key.overlaps(&full) {
                continue;
            }
            // Assigning a borrow ends the borrows made through it, which point past it:
            // `at = &(*at).next` walks a list.
            if kind == Access::Write && key.through(&full) {
                continue;
            }
            let (code, message) = match kind {
                Access::Read | Access::Move if loan.guard => continue,
                Access::Read if !loan.mutable => continue,
                Access::Read => (
                    Code::E0307,
                    "it is borrowed with `&mut`, so it cannot be used until that borrow ends",
                ),
                Access::Borrow(false) if !loan.mutable || loan.guard => continue,
                Access::Borrow(_) => (
                    Code::E0305,
                    "it is already borrowed, and this borrow conflicts with that one",
                ),
                Access::Write | Access::Move => (
                    Code::E0306,
                    "it is borrowed, so it cannot be assigned to or moved until that borrow ends",
                ),
            };
            let what = self.name(place);
            let loan_span = loan.span;
            let d = Diagnostic::error(code, span, format!("{what}: {message}"))
                .with_label(loan_span, "borrowed here, and the borrow is used later");
            self.report(d);
        }
    }

    fn operand(
        &mut self,
        at: Point,
        span: TextRange,
        o: &Operand,
        h: &Holders,
        m: &mut Moved,
        check: bool,
    ) {
        match o {
            Operand::Copy(p) => {
                if check {
                    self.access((at, span), p, Access::Read, h, m, None);
                }
            }
            Operand::Move(p) => {
                if check {
                    self.access((at, span), p, Access::Move, h, m, None);
                    self.move_out_of_drop(span, p);
                }
                if !p.is_indirect() {
                    m.places.insert(PlaceKey::of(p));
                }
            }
            Operand::Const(_) => {}
        }
    }

    /// The loans flowing out of an operand: those its local holds, if it is a borrow or a view,
    /// or a part of one that is a borrow or a view (a view's field).
    fn flows(&self, o: &Operand, h: &Holders) -> BTreeSet<usize> {
        match o {
            Operand::Copy(p) | Operand::Move(p)
                if p.projection.is_empty()
                    || self.items.is_view(&self.type_upto(p, p.projection.len())) =>
            {
                h[p.local.index()].clone()
            }
            _ => BTreeSet::new(),
        }
    }

    /// Assigning `flowing` loans to `dest` and marking it initialised.
    fn define(&self, dest: &Place, flowing: BTreeSet<usize>, h: &mut Holders, m: &mut Moved) {
        if dest.projection.is_empty() {
            h[dest.local.index()] = flowing;
        } else {
            h[dest.local.index()].extend(flowing);
        }
        if !dest.is_indirect() {
            let key = PlaceKey::of(dest);
            // The borrows made through what `dest` held end: they no longer depend on it.
            for held in h.iter_mut() {
                held.retain(|&li| !PlaceKey::of(&self.loans[li].place).through(&key));
            }
            m.places.retain(|mv| !key.contains(mv));
            if dest.projection.is_empty() && self.body.late.iter().any(|&(l, _)| l == dest.local) {
                m.given.insert(dest.local.0);
            }
        }
    }

    fn step(
        &mut self,
        b: usize,
        s: usize,
        stmt: &Statement,
        h: &mut Holders,
        m: &mut Moved,
        check: bool,
    ) {
        let at = (b, s);
        let span = self.body.blocks[b].spans[s];
        match stmt {
            Statement::Assign(dest, r) => {
                let mut flowing = BTreeSet::new();
                match r {
                    Rvalue::Use(o) | Rvalue::Unsize(o, _) | Rvalue::Widen(o, _) => {
                        flowing = self.flows(o, h);
                        self.operand(at, span, o, h, m, check);
                    }
                    Rvalue::Length(_)
                    | Rvalue::StaticRef(_)
                    | Rvalue::StaticBlock(_)
                    | Rvalue::FnPtr(_) => {}
                    Rvalue::SpanOk(a, b, p) => {
                        self.operand(at, span, a, h, m, check);
                        self.operand(at, span, b, h, m, check);
                        if check {
                            self.access((at, span), p, Access::Read, h, m, None);
                        }
                    }
                    Rvalue::InBounds(a, p) => {
                        self.operand(at, span, a, h, m, check);
                        if check {
                            self.access((at, span), p, Access::Read, h, m, None);
                        }
                    }
                    Rvalue::Unary(_, a) | Rvalue::NegOverflows(a) | Rvalue::Repeat(a, _) => {
                        self.operand(at, span, a, h, m, check);
                    }
                    Rvalue::Binary(_, a, b2) | Rvalue::Overflows(_, a, b2) => {
                        self.operand(at, span, a, h, m, check);
                        self.operand(at, span, b2, h, m, check);
                    }
                    // A tuple, struct or view holds the loans of what it is built from.
                    Rvalue::Aggregate(_, ops) => {
                        for o in ops {
                            flowing.extend(self.flows(o, h));
                            self.operand(at, span, o, h, m, check);
                        }
                    }
                    Rvalue::Discriminant(p) | Rvalue::ErrorIs(p, _) | Rvalue::Len(p) => {
                        if check {
                            self.access((at, span), p, Access::Read, h, m, None);
                        }
                    }
                    Rvalue::Ref(mutable, p) => {
                        let loan = self.loan_at[&at];
                        if check {
                            self.access((at, span), p, Access::Borrow(*mutable), h, m, Some(loan));
                        }
                        flowing.insert(loan);
                        // A reborrow keeps the borrow it goes through alive, and so does a
                        // borrow of a place that holds borrows (`&r`, `borrow.type.1`): what
                        // it points to is reached through them.
                        if p.is_indirect()
                            || self.items.is_view(&self.type_upto(p, p.projection.len()))
                        {
                            flowing.extend(h[p.local.index()].iter().copied());
                        }
                    }
                }
                if check {
                    self.access((at, span), dest, Access::Write, h, m, None);
                }
                self.define(dest, flowing, h, m);
            }
            Statement::Assert { cond, .. } => self.operand(at, span, cond, h, m, check),
            Statement::Observe { value, .. } => self.operand(at, span, value, h, m, check),
            Statement::Read(p) => {
                if check {
                    self.access((at, span), p, Access::Read, h, m, None);
                }
            }
            Statement::StorageDead(l) => {
                if check {
                    self.storage_dead(at, *l, h);
                }
                h[l.index()].clear();
                m.places.retain(|mv| mv.local != l.0);
                m.given.remove(&l.0);
            }
        }
    }

    /// A local leaving scope while a live loan still borrows it (E0308).
    fn storage_dead(&mut self, at: Point, l: Local, h: &Holders) {
        for li in self.live_loans(at, h) {
            let loan = &self.loans[li];
            if loan.param.is_none()
                && loan.place.local == l
                && loan.place.projection.first() != Some(&Projection::Deref)
            {
                let what = self.name(&Place::local(l));
                let d = Diagnostic::error(
                    Code::E0308,
                    loan.span,
                    format!(
                        "this borrows {what}, which goes out of scope while the borrow is still used"
                    ),
                );
                self.report(d);
            }
        }
    }

    fn step_terminator(&mut self, b: usize, h: &mut Holders, m: &mut Moved, check: bool) {
        let block = &self.body.blocks[b];
        let at = (b, block.statements.len());
        let span = block.terminator_span;
        match &block.terminator {
            Terminator::SwitchBool { cond, .. } => self.operand(at, span, cond, h, m, check),
            Terminator::Drop { place, .. } => {
                // Like a move, but a value already moved out of is simply not dropped.
                if check {
                    let full = PlaceKey::of(place);
                    for li in self.live_loans(at, h) {
                        let loan = &self.loans[li];
                        if loan.param.is_some() || !PlaceKey::of(&loan.place).overlaps(&full) {
                            continue;
                        }
                        let what = self.name(place);
                        let d = Diagnostic::error(
                            Code::E0308,
                            loan.span,
                            format!(
                                "this borrows {what}, which is dropped while the borrow is still used"
                            ),
                        )
                        .with_label(span, "dropped here");
                        self.report(d);
                    }
                }
                if !place.is_indirect() {
                    m.places.insert(PlaceKey::of(place));
                }
            }
            Terminator::Call {
                callee,
                args,
                dest,
                on_error,
                ..
            } => {
                let sig = &self.items.fns[callee.func() as usize];
                let mut flowing = BTreeSet::new();
                let flows: Vec<BTreeSet<usize>> = args.iter().map(|a| self.flows(a, h)).collect();
                if matches!(sig.ret, Ty::Opaque(_))
                    && self.items.is_view(&self.body.locals[dest.local.index()].ty)
                {
                    // An `impl Trait` result that holds a borrow holds the handlers passed,
                    // after the arguments (`type.impl.ret.2`).
                    for f in flows.iter().skip(sig.params.len()) {
                        flowing.extend(f.iter().copied());
                    }
                } else if self.items.is_view(&sig.ret) {
                    for &i in &sig.ret_from {
                        if let Some(f) = flows.get(i as usize) {
                            flowing.extend(f.iter().copied());
                        }
                    }
                } else if self.items.is_view(&sig.ret.subst(callee.args())) {
                    // The signature's result holds no borrow, so any borrow in it comes from a
                    // type argument: from what the arguments are or point to, not from the
                    // borrows passed (`it.next()` gives what the iterator holds, not `&mut it`).
                    // A parameter declared a borrow lends what it points to; one of a type
                    // parameter is given the borrow itself, which the result may be (`id(&x)`).
                    for (i, f) in flows.iter().enumerate() {
                        let by_ref = sig
                            .params
                            .get(i)
                            .is_some_and(|p| matches!(p.ty, Ty::Ref(..)));
                        for &li in f {
                            let loan = &self.loans[li];
                            // A view borrowed in place, or one a borrowed parameter points to
                            // (`self.inner.next()`): what it holds, as the parameter holds it.
                            if by_ref
                                && loan.param.is_none()
                                && (!loan.place.is_indirect() || self.of_borrowed_view(&loan.place))
                            {
                                flowing.extend(h[loan.place.local.index()].iter().copied());
                            } else {
                                flowing.insert(li);
                            }
                        }
                    }
                }
                // A view changed through a `&mut` argument may keep what the other arguments
                // borrow (`v.push(&x)`).
                for (i, p) in sig.params.iter().enumerate() {
                    let Ty::Ref(true, inner) = p.ty.subst(callee.args()) else {
                        continue;
                    };
                    if !self.items.is_view(&inner) {
                        continue;
                    }
                    let others: BTreeSet<usize> = flows
                        .iter()
                        .enumerate()
                        .filter(|&(j, _)| j != i)
                        .flat_map(|(_, f)| f.iter().copied())
                        .collect();
                    if others.is_empty() {
                        continue;
                    }
                    let Some(target) = flows.get(i) else { continue };
                    for &li in target {
                        let loan = &self.loans[li];
                        // A view the caller lent: a parameter's, or reached through a local
                        // holding a parameter's borrow (`&mut *s`).
                        let lent = loan.param.is_some()
                            || (loan.place.is_indirect()
                                && h[loan.place.local.index()]
                                    .iter()
                                    .any(|&p| self.loans[p].param.is_some()));
                        if lent {
                            // A borrow through a reference (a closure's capture, a parameter)
                            // is not of a local: what that reference borrows is in `others` too.
                            if check
                                && others.iter().any(|&o| {
                                    self.loans[o].param.is_none()
                                        && !self.loans[o].place.is_indirect()
                                })
                            {
                                self.report(Diagnostic::error(
                                    Code::E0308,
                                    span,
                                    "this stores a borrow of a local where it outlives the \
                                     function: in a view the caller lent",
                                ));
                            }
                            // What it keeps lives as long as the caller's view: until the end.
                            if let Some(kept) = h.last_mut() {
                                kept.extend(others.iter().copied());
                            }
                        } else {
                            h[loan.place.local.index()].extend(others.iter().copied());
                        }
                    }
                }
                if check && self.is_text_between(callee.func()) {
                    self.positions_of(span, &flows);
                }
                for a in args {
                    self.operand(at, span, a, h, m, check);
                }
                if check {
                    self.access((at, span), dest, Access::Write, h, m, None);
                }
                self.define(dest, flowing, h, m);
                // The error is defined on the error edge; defining it on both is harmless, as
                // nothing reads it on the other.
                if let Some((e, _)) = on_error {
                    self.define(e, BTreeSet::new(), h, m);
                }
            }
            Terminator::Fail(e) => self.operand(at, span, e, h, m, check),
            Terminator::Return => {
                if check {
                    // Point at where the returned value was last set, if it was in this block.
                    let set = block
                        .statements
                        .iter()
                        .zip(&block.spans)
                        .rev()
                        .find(|(st, sp)| {
                            **sp != TextRange::default()
                                && matches!(st, Statement::Assign(p, _) if p.local == Local::RETURN)
                        })
                        .map(|(_, sp)| *sp);
                    // Or else at the call that gives it (`Box::new(…)` as the body's value).
                    let set = set.or_else(|| {
                        self.body
                            .blocks
                            .iter()
                            .rev()
                            .find_map(|bl| match &bl.terminator {
                                Terminator::Call { .. }
                                    if bl.terminator_span != TextRange::default() =>
                                {
                                    Some(bl.terminator_span)
                                }
                                _ => None,
                            })
                    });
                    self.returned(set.unwrap_or(span), h);
                }
            }
            Terminator::Goto(_)
            | Terminator::Resume
            | Terminator::Unreachable
            | Terminator::Trap(..) => {}
        }
    }

    /// Whether `func` is the prelude's `str::between`, which `&s[a..b]` calls (`text.pos.1`).
    fn is_text_between(&self, func: u32) -> bool {
        let sig = &self.items.fns[func as usize];
        sig.name == "between"
            && matches!(sig.container, Container::Impl(k) if {
                let i = &self.items.impls[k as usize];
                i.src == Src::Prelude && i.trait_ref.is_none() && i.self_ty == Ty::Str
            })
    }

    /// `&s[a..b]`, whose arguments' loans are `flows` (the text, then the positions): a position
    /// found in other text than `s` is an error (E0316, `text.pos.1`), where both are known to be
    /// a local's. A position or text given as a parameter, or text that borrows nothing (a
    /// literal), is checked when it runs.
    fn positions_of(&mut self, span: TextRange, flows: &[BTreeSet<usize>]) {
        let [text, ends @ ..] = flows else {
            return;
        };
        if text.is_empty() {
            return;
        }
        for end in ends {
            if end.is_empty() || end.iter().any(|&l| self.loans[l].param.is_some()) {
                continue;
            }
            let same = end.iter().any(|&e| {
                text.iter().any(|&t| {
                    PlaceKey::of(&self.loans[e].place).overlaps(&PlaceKey::of(&self.loans[t].place))
                })
            });
            if !same {
                let found = end
                    .iter()
                    .next()
                    .map(|&e| self.name(&self.loans[e].place.clone()));
                let found = found.unwrap_or_else(|| "other text".to_string());
                self.report(Diagnostic::error(
                    Code::E0316,
                    span,
                    format!("this slices text with a position in {found}, other text"),
                ));
            }
        }
    }

    /// Whether `place` is part of a view a borrowed parameter points to, reached without going
    /// through a borrow that view holds (`(*self).spare`, not `*(*self).xs`).
    fn of_borrowed_view(&self, place: &Place) -> bool {
        let l = place.local.index();
        if l == 0 || l > self.body.arg_count {
            return false;
        }
        let Ty::Ref(_, inner) = &self.body.locals[l].ty else {
            return false;
        };
        self.items.is_view(inner)
            && place.projection.first() == Some(&Projection::Deref)
            && !place.projection[1..].contains(&Projection::Deref)
    }

    /// A returned borrow must come from a parameter the signature allows.
    fn returned(&mut self, span: TextRange, h: &Holders) {
        let mut found = Vec::new();
        for &li in &h[Local::RETURN.index()] {
            let loan = &self.loans[li];
            // A borrow through a borrow borrows what that one points to, whose own loans are
            // checked with it.
            let from_param = loan.place.projection.first() == Some(&Projection::Deref);
            // A borrow of the view a parameter borrows, not of what it holds: `&self.spare`
            // where the trait's result holds no borrow (`view.borrow.1`).
            if self.returns_held && loan.param.is_none() && self.of_borrowed_view(&loan.place) {
                found.push(Diagnostic::error(
                    Code::E0308,
                    loan.span,
                    format!(
                        "this returns a borrow of the view {} points to, not of what it holds: \
                         the trait's result holds no borrow, so callers take it as one the view \
                         holds, and may drop the view while using it",
                        self.name(&Place::local(loan.place.local))
                    ),
                ));
                continue;
            }
            match loan.param {
                // A handler the function was given, which a boxed closure it returns borrows
                // (`expr.closure.stored.1`).
                Some(p) if p >= self.written_params && !self.ret_from.contains(&p) => {
                    found.push(Diagnostic::error(
                        Code::E0310,
                        span,
                        "the returned value holds a borrow of a handler the function was given, \
                     which lives only as long as the call: install the handler inside it, or \
                     move the `with` out to where the value is used",
                    ));
                }
                Some(p) if !self.ret_from.contains(&p) => found.push(Diagnostic::error(
                    Code::E0310,
                    span,
                    format!(
                        "the returned borrow may come from {}, which the signature does not \
                         allow (see its `from` clause)",
                        self.name(&Place::local(Local(p + 1)))
                    ),
                )),
                None if !from_param => found.push(Diagnostic::error(
                    Code::E0308,
                    loan.span,
                    format!(
                        "this borrows {}, which belongs to the function, so the borrow cannot be \
                         returned",
                        self.name(&direct(&loan.place))
                    ),
                )),
                _ => {}
            }
        }
        for d in found {
            self.report(d);
        }
    }
}

/// Whether `local` is the borrow `match` makes so guards cannot change the matched value: one
/// that is used only by `Statement::Read`, at least once.
fn is_guard_borrow(body: &Body, local: Local) -> bool {
    let mut read = false;
    for b in &body.blocks {
        for s in &b.statements {
            match s {
                Statement::Read(p) if p.local == local => read = true,
                Statement::Assign(_, r) if rvalue_uses(r, local) => return false,
                Statement::Assert { cond: o, .. } | Statement::Observe { value: o, .. }
                    if operand_local(o) == Some(local) =>
                {
                    return false;
                }
                _ => {}
            }
        }
        match &b.terminator {
            Terminator::SwitchBool { cond, .. } if operand_local(cond) == Some(local) => {
                return false;
            }
            Terminator::Call { args, .. }
                if args.iter().any(|a| operand_local(a) == Some(local)) =>
            {
                return false;
            }
            _ => {}
        }
    }
    read
}

fn operand_local(o: &Operand) -> Option<Local> {
    match o {
        Operand::Copy(p) | Operand::Move(p) => Some(p.local),
        Operand::Const(_) => None,
    }
}

fn rvalue_uses(r: &Rvalue, local: Local) -> bool {
    let uses = |o: &Operand| operand_local(o) == Some(local);
    match r {
        Rvalue::Use(a)
        | Rvalue::Unsize(a, _)
        | Rvalue::Widen(a, _)
        | Rvalue::Unary(_, a)
        | Rvalue::NegOverflows(a)
        | Rvalue::Repeat(a, _) => uses(a),
        Rvalue::Length(_) | Rvalue::StaticRef(_) | Rvalue::StaticBlock(_) | Rvalue::FnPtr(_) => {
            false
        }
        Rvalue::InBounds(a, p) => uses(a) || p.local == local,
        Rvalue::SpanOk(a, b, p) => uses(a) || uses(b) || p.local == local,
        Rvalue::Binary(_, a, b) | Rvalue::Overflows(_, a, b) => uses(a) || uses(b),
        Rvalue::Aggregate(_, ops) => ops.iter().any(uses),
        Rvalue::Discriminant(p) | Rvalue::ErrorIs(p, _) | Rvalue::Ref(_, p) | Rvalue::Len(p) => {
            p.local == local
        }
    }
}

/// The locals each statement and terminator uses, and the local it defines wholly.
fn uses_defs(stmt: Option<&Statement>, term: Option<&Terminator>) -> (Vec<Local>, Option<Local>) {
    let mut uses = Vec::new();
    let op = |o: &Operand, uses: &mut Vec<Local>| {
        if let Some(l) = operand_local(o) {
            uses.push(l);
        }
    };
    let mut def = None;
    if let Some(s) = stmt {
        match s {
            Statement::Assign(dest, r) => {
                match r {
                    Rvalue::Use(a)
                    | Rvalue::Unsize(a, _)
                    | Rvalue::Widen(a, _)
                    | Rvalue::Unary(_, a)
                    | Rvalue::NegOverflows(a)
                    | Rvalue::Repeat(a, _) => {
                        op(a, &mut uses);
                    }
                    Rvalue::Length(_)
                    | Rvalue::StaticRef(_)
                    | Rvalue::StaticBlock(_)
                    | Rvalue::FnPtr(_) => {}
                    Rvalue::InBounds(a, p) => {
                        op(a, &mut uses);
                        uses.push(p.local);
                    }
                    Rvalue::Len(p) => uses.push(p.local),
                    Rvalue::SpanOk(a, b, p) => {
                        op(a, &mut uses);
                        op(b, &mut uses);
                        uses.push(p.local);
                    }
                    Rvalue::Binary(_, a, b) | Rvalue::Overflows(_, a, b) => {
                        op(a, &mut uses);
                        op(b, &mut uses);
                    }
                    Rvalue::Aggregate(_, ops) => ops.iter().for_each(|o| op(o, &mut uses)),
                    Rvalue::Discriminant(p) | Rvalue::ErrorIs(p, _) | Rvalue::Ref(_, p) => {
                        uses.push(p.local);
                    }
                }
                if dest.projection.is_empty() {
                    def = Some(dest.local);
                } else {
                    // Writing through a borrow, or into part of a local, uses the local.
                    uses.push(dest.local);
                }
            }
            Statement::Assert { cond, .. } => op(cond, &mut uses),
            Statement::Observe { value, .. } => op(value, &mut uses),
            Statement::Read(p) => uses.push(p.local),
            Statement::StorageDead(l) => def = Some(*l),
        }
    }
    if let Some(t) = term {
        match t {
            Terminator::SwitchBool { cond, .. } => op(cond, &mut uses),
            Terminator::Call { args, dest, .. } => {
                args.iter().for_each(|a| op(a, &mut uses));
                if dest.projection.is_empty() {
                    def = Some(dest.local);
                } else {
                    uses.push(dest.local);
                }
            }
            Terminator::Return => uses.push(Local::RETURN),
            Terminator::Drop { place, .. } => uses.push(place.local),
            Terminator::Fail(e) => op(e, &mut uses),
            Terminator::Goto(_)
            | Terminator::Resume
            | Terminator::Unreachable
            | Terminator::Trap(..) => {}
        }
    }
    (uses, def)
}

/// For each point, the locals live on exit from it (used later on some path before being
/// defined again).
fn liveness(body: &Body) -> Vec<Vec<HashSet<Local>>> {
    let n = body.blocks.len();
    let mut live_in: Vec<HashSet<Local>> = vec![HashSet::new(); n];
    let mut changed = true;
    while changed {
        changed = false;
        for b in (0..n).rev() {
            let block = &body.blocks[b];
            let mut live: HashSet<Local> = block
                .terminator
                .successors()
                .iter()
                .flat_map(|s| live_in[s.index()].iter().copied())
                .collect();
            let (uses, def) = uses_defs(None, Some(&block.terminator));
            if let Some(d) = def {
                live.remove(&d);
            }
            live.extend(uses);
            for s in block.statements.iter().rev() {
                let (uses, def) = uses_defs(Some(s), None);
                if let Some(d) = def {
                    live.remove(&d);
                }
                live.extend(uses);
            }
            if live != live_in[b] {
                live_in[b] = live;
                changed = true;
            }
        }
    }
    let mut out = Vec::with_capacity(n);
    for block in body.blocks.iter().take(n) {
        let mut points = vec![HashSet::new(); block.statements.len() + 1];
        let mut live: HashSet<Local> = block
            .terminator
            .successors()
            .iter()
            .flat_map(|s| live_in[s.index()].iter().copied())
            .collect();
        points[block.statements.len()] = live.clone();
        let (uses, def) = uses_defs(None, Some(&block.terminator));
        if let Some(d) = def {
            live.remove(&d);
        }
        live.extend(uses);
        for (i, s) in block.statements.iter().enumerate().rev() {
            points[i] = live.clone();
            let (uses, def) = uses_defs(Some(s), None);
            if let Some(d) = def {
                live.remove(&d);
            }
            live.extend(uses);
        }
        out.push(points);
    }
    out
}
