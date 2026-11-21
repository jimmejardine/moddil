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

use moddil_sema::{Code, Container, Db, Diagnostic, Item, ItemTree, Owner, Program, Severity, Ty};
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
    let mut cx = Checker::new(body, items, ret_from);
    cx.destructor = destructor;
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
    /// Whether the body is a destructor, which may move fields out of its `self` (`_1`).
    destructor: bool,
}

/// The loans each local may hold.
/// Who holds each loan: by local, and, last, the loans kept in a view the caller lent, which live
/// until the body ends.
type Holders = Vec<BTreeSet<usize>>;

/// The places that may have been moved out of.
type Moved = BTreeSet<PlaceKey>;

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
                    Projection::Index(_) => (3, 0),
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
            destructor: false,
        }
    }

    /// The type of the part of `p` before its projection `upto`.
    fn type_upto(&self, p: &Place, upto: usize) -> Ty {
        let mut ty = self.body.locals[p.local.index()].ty.clone();
        let mut variant = 0;
        for proj in &p.projection[..upto] {
            ty = match (proj, &ty) {
                (Projection::Deref, Ty::Ref(_, t))
                | (Projection::Index(_), Ty::Array(t, _) | Ty::Slice(t)) => (**t).clone(),
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
                        "{what} cannot be moved out: `{ty}` has a destructor, which needs all of                          the value (borrow it, or match it by reference)"
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
            }
        }
        let _ = indirect;
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
        moved_in[0] = Some(BTreeSet::new());
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
                        let before = mi.len();
                        mi.extend(m.iter().cloned());
                        changed | (mi.len() != before)
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
            let moved_part = m.iter().find(|mv| {
                if kind == Access::Write {
                    // Assigning a part of a moved value is an error; assigning all of it is not.
                    mv.contains(&key) && *mv != &key
                } else {
                    mv.overlaps(&key)
                }
            });
            if moved_part.is_some() {
                let what = self.name(&direct(place));
                self.report(Diagnostic::error(
                    Code::E0303,
                    span,
                    format!("{what} was moved out of earlier, so it cannot be used here"),
                ));
            }
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
            if loan.param.is_some() || !PlaceKey::of(&loan.place).overlaps(&full) {
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
                    m.insert(PlaceKey::of(p));
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
            m.retain(|mv| !key.contains(mv));
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
                    Rvalue::Use(o) | Rvalue::Unsize(o, _) => {
                        flowing = self.flows(o, h);
                        self.operand(at, span, o, h, m, check);
                    }
                    Rvalue::Length(_) | Rvalue::StaticRef(_) => {}
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
                    Rvalue::Discriminant(p) | Rvalue::ErrorIs(p, _) => {
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
                        // A reborrow keeps the borrow it goes through alive.
                        if p.is_indirect() {
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
                m.retain(|mv| mv.local != l.0);
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
                                "this borrows {what}, which is dropped while the borrow is still                                  used"
                            ),
                        )
                        .with_label(span, "dropped here");
                        self.report(d);
                    }
                }
                if !place.is_indirect() {
                    m.insert(PlaceKey::of(place));
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
                if self.items.is_view(&sig.ret) {
                    for &i in &sig.ret_from {
                        if let Some(f) = flows.get(i as usize) {
                            flowing.extend(f.iter().copied());
                        }
                    }
                } else if self.items.is_view(&sig.ret.subst(callee.args())) {
                    // The signature's result holds no borrow, so any borrow in it comes from a
                    // type argument: from what the arguments are or point to, not from the
                    // borrows passed (`it.next()` gives what the iterator holds, not `&mut it`).
                    for (i, f) in flows.iter().enumerate() {
                        let by_ref = sig
                            .params
                            .get(i)
                            .is_some_and(|p| matches!(p.ty.subst(callee.args()), Ty::Ref(..)));
                        for &li in f {
                            let loan = &self.loans[li];
                            if by_ref && loan.param.is_none() && !loan.place.is_indirect() {
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
                        .find(|(st, _)| {
                            matches!(st, Statement::Assign(p, _) if p.local == Local::RETURN)
                        })
                        .map_or(span, |(_, sp)| *sp);
                    self.returned(set, h);
                }
            }
            Terminator::Goto(_) | Terminator::Unreachable | Terminator::Trap(..) => {}
        }
    }

    /// A returned borrow must come from a parameter the signature allows.
    fn returned(&mut self, span: TextRange, h: &Holders) {
        let mut found = Vec::new();
        for &li in &h[Local::RETURN.index()] {
            let loan = &self.loans[li];
            // A borrow through a borrow borrows what that one points to, whose own loans are
            // checked with it.
            let from_param = loan.place.projection.first() == Some(&Projection::Deref);
            match loan.param {
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
        | Rvalue::Unary(_, a)
        | Rvalue::NegOverflows(a)
        | Rvalue::Repeat(a, _) => uses(a),
        Rvalue::Length(_) | Rvalue::StaticRef(_) => false,
        Rvalue::InBounds(a, p) => uses(a) || p.local == local,
        Rvalue::Binary(_, a, b) | Rvalue::Overflows(_, a, b) => uses(a) || uses(b),
        Rvalue::Aggregate(_, ops) => ops.iter().any(uses),
        Rvalue::Discriminant(p) | Rvalue::ErrorIs(p, _) | Rvalue::Ref(_, p) => p.local == local,
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
                    | Rvalue::Unary(_, a)
                    | Rvalue::NegOverflows(a)
                    | Rvalue::Repeat(a, _) => {
                        op(a, &mut uses);
                    }
                    Rvalue::Length(_) | Rvalue::StaticRef(_) => {}
                    Rvalue::InBounds(a, p) => {
                        op(a, &mut uses);
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
            Terminator::Goto(_) | Terminator::Unreachable | Terminator::Trap(..) => {}
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
    for b in 0..n {
        let block = &body.blocks[b];
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
