//! Effects (`effects.md`): what each function needs, and where each need is met.
//!
//! A call needs the effects its callee's signature lists, and those inferred for it; each need
//! is met by the innermost `with` around the call that installs a handler for exactly that
//! effect, or else by the calling function's own effects. A private function's effects and
//! errors are inferred: the [`effects`] query checks the file's private functions over and over,
//! starting from what they write, until what they need stops growing. A `pub` function, a trait's
//! method and an entry point write theirs out, and a need they do not list is an error.

use std::collections::HashMap;

use moddil_syntax::TextRange;

use crate::collect::item_tree;
use crate::db::Program;
use crate::hir::{ExprId, Handler};
use crate::items::{Container, ItemTree, Vis};
use crate::{Code, Db, Diagnostic, Item, Owner, Ty};

use super::{Checker, check_body, index};

/// Every function's effects and errors: as written, or as inferred for a private function.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Effects {
    /// For each function of the item tree, by index.
    pub fns: Vec<FnEffects>,
    /// Functions whose effects never settled (E0607).
    pub diagnostics: Vec<Diagnostic>,
}

/// What a function needs and how it can fail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FnEffects {
    /// The effects it needs (as types: `dyn Time`, `AppConfig`), in a canonical order, which is
    /// also the order of its hidden handler parameters.
    pub needs: Vec<Ty>,
    /// The errors it can fail with, as an error set (`!` if none).
    pub fails: Ty,
    /// The control effects it needs (`eff.control.1`).
    pub control: crate::Control,
}

/// Where a call's need is met.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Evidence {
    /// By the calling function's own effect: its index in the caller's needs.
    Param(usize),
    /// By a handler of a `with` around the call: the `with` expression, and the handler's index
    /// in it.
    Install(ExprId, usize),
}

/// How a body's needs are checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    /// They are inferred: what is not met inside goes to the function's own needs.
    Inferred,
    /// They are written out, by a `pub` function's signature: a need it does not list is an
    /// error (E0603).
    Written,
    /// It is a trait's method, or an impl's, which lists no effects (E0603).
    Method,
    /// It is an entry point (a test, `main`): every need must be met inside (E0602).
    Entry(&'static str),
}

/// A call's needs, recorded until the body's types are known.
pub(super) struct Need {
    call: ExprId,
    needs: Vec<Ty>,
    /// The handlers installed around the call, innermost last: the `with`, the handler's index
    /// and its effect.
    installs: Vec<(ExprId, usize, Ty)>,
    range: TextRange,
}

/// Whether function `i`'s effects and errors are inferred: it is a private function or method
/// of an inherent impl of the file, with a body, other than `main`.
pub(crate) fn inferred(items: &ItemTree, i: u32) -> bool {
    let f = &items.fns[i as usize];
    let container = match f.container {
        Container::Free => true,
        Container::Impl(k) => items.impls[k as usize].trait_ref.is_none(),
        Container::Trait(_) => false,
    };
    f.src.is_file() && f.has_body && f.vis != Vis::Pub && container && !items.is_main(i)
}

/// `ts` in the canonical order of an effect set, each once.
fn canonical(mut ts: Vec<Ty>) -> Vec<Ty> {
    ts.retain(|t| !t.has_error());
    ts.sort_by_key(|t| format!("{t:?}"));
    ts.dedup();
    ts
}

/// How an effect is written after `with`: `Time`, `AppConfig`.
pub(crate) fn show_effect(t: &Ty) -> String {
    match t {
        Ty::Dyn(tr) => tr.to_string(),
        t => t.to_string(),
    }
}

/// `f`'s errors as written, and those of its closure parameters (`F: FnOnce(…)`), which are
/// its too (`expr.closure.fails.1`), but those a bound on `F::Fails` takes
/// (`expr.closure.fails.2`).
fn with_closure_fails(items: &ItemTree, f: &crate::FnSig) -> Ty {
    let mut members = f.fails.members();
    let once = items.lang("FnOnce");
    let taken = |ty: &Ty| {
        f.generics.preds.iter().any(
            |q| matches!(&q.ty, Ty::Assoc(a) if a.name.as_ref() == "Fails" && a.self_ty == *ty),
        )
    };
    for p in &f.generics.preds {
        if Some(p.trait_ref.trait_) == once
            && f.generics.parent as usize <= param_index(&p.ty)
            && !taken(&p.ty)
        {
            members.push(Ty::Assoc(std::sync::Arc::new(crate::ty::AssocTy {
                self_ty: p.ty.clone(),
                trait_ref: p.trait_ref.clone(),
                index: 1,
                name: "Fails".into(),
            })));
        }
    }
    Ty::errors(members)
}

/// The index of a type parameter, or 0.
fn param_index(t: &Ty) -> usize {
    match t {
        Ty::Param(i, _) => *i as usize,
        _ => 0,
    }
}

/// How many times the private functions are checked, at most, before their effects are taken
/// not to settle.
const ROUNDS: usize = 64;

/// Every function's effects and errors (`eff.infer.1`).
#[salsa::tracked(returns(ref))]
pub fn effects(db: &dyn Db, program: Program) -> Effects {
    let items = item_tree(db, program);
    let mut fx = Effects {
        fns: items
            .fns
            .iter()
            .map(|f| FnEffects {
                needs: canonical(f.effects.iter().map(|e| e.ty.clone()).collect()),
                fails: with_closure_fails(items, f),
                control: f.control,
            })
            .collect(),
        diagnostics: Vec::new(),
    };
    let open: Vec<u32> = (0..items.fns.len() as u32)
        .filter(|&i| inferred(items, i))
        .collect();
    if open.is_empty() {
        return fx;
    }
    let mut changed: Vec<u32> = Vec::new();
    for _ in 0..ROUNDS {
        let mut next = fx.fns.clone();
        for &i in &open {
            let out = check_body(db, Owner::new(db, program, Item::Fn(i)), &fx);
            let f = &mut next[i as usize];
            let mut needs = f.needs.clone();
            needs.extend(out.escaping_needs.iter().cloned());
            f.needs = canonical(needs);
            let mut fails = f.fails.members();
            fails.extend(out.escaping_errors.iter().cloned());
            fails.retain(|t| !t.has_error());
            f.fails = Ty::errors(fails);
            f.control |= out.escaping_control;
        }
        if next == fx.fns {
            return fx;
        }
        changed = open
            .iter()
            .copied()
            .filter(|&i| next[i as usize] != fx.fns[i as usize])
            .collect();
        fx.fns = next;
    }
    // Only the functions whose effects still grow: those whose needs settled are not at fault.
    for &i in &changed {
        let f = &items.fns[i as usize];
        fx.diagnostics.push(Diagnostic::error(
            Code::E0607,
            f.name_range,
            format!(
                "the effects of `{}` grow without end (it calls itself with ever larger types): \
                 make it `pub` and write them out",
                f.name
            ),
        ));
    }
    fx
}

impl Checker<'_> {
    /// Records what a call of `func` with type arguments `args` needs.
    pub(super) fn needs_of_call(&mut self, e: ExprId, func: u32, args: &[Ty], range: TextRange) {
        let control = self.fx.fns[func as usize].control;
        if control != 0 {
            self.control_uses.push((control, range));
        }
        let needs: Vec<Ty> = self.fx.fns[func as usize]
            .needs
            .iter()
            .map(|t| t.subst(args))
            .collect();
        if needs.is_empty() {
            return;
        }
        let installs = self
            .installs
            .iter()
            .flat_map(|(w, effects)| {
                effects
                    .iter()
                    .enumerate()
                    .map(move |(k, t)| (*w, k, t.clone()))
            })
            .collect();
        self.needs.push(Need {
            call: e,
            needs,
            installs,
            range,
        });
    }

    /// `with E = value, … { body }` (`eff.with.1`).
    pub(super) fn with_expr(&mut self, e: ExprId, handlers: &[Handler], body: ExprId) -> Ty {
        let mut effects = Vec::new();
        for h in handlers {
            let t = self.infer(h.value);
            self.handler_fits(h, &t);
            effects.push(h.effect.clone());
        }
        self.installs.push((e, effects));
        let t = self.infer(body);
        self.installs.pop();
        t
    }

    /// Whether a handler of type `t` can be installed for `h.effect`: a value of a type that
    /// implements the trait (or of the context data's type), or a borrow of one.
    fn handler_fits(&mut self, h: &Handler, t: &Ty) {
        let range = self.range(h.value);
        self.solver.table.default_literal(t);
        let t = self.norm(t);
        let inner = match &t {
            Ty::Ref(_, x) => self.norm(x),
            x => x.clone(),
        };
        match (&h.effect, &inner) {
            (Ty::Error, _) | (_, Ty::Error | Ty::Never) => {}
            (_, Ty::Infer(_)) => {
                self.unknown(range, "to install it as a handler");
            }
            (Ty::Dyn(tr), Ty::Dyn(d)) if d == tr => {}
            (Ty::Dyn(tr), _) => self.require(inner, tr.clone(), range),
            (data, _) => {
                if !self.solver.unify(&inner, data) {
                    let found = self.show(&t);
                    self.report(
                        Code::E0201,
                        range,
                        format!(
                            "a handler for `{data}` is a `{data}` or a borrow of one, and this is \
                             a `{found}`"
                        ),
                    );
                }
            }
        }
    }

    /// Meets each call's needs, once the body's types are known: by the innermost handler
    /// installed for exactly that effect, or by the function's own effects; records the need
    /// as the function's if they are inferred, and reports it if not.
    pub(super) fn meet_needs(&mut self) {
        // The control effects (`eff.control.1`): an entry point has them from the runtime.
        for (c, range) in std::mem::take(&mut self.control_uses) {
            let missing = c & !self.own_control;
            if missing == 0 {
                continue;
            }
            let name = crate::items::show_control(missing);
            match self.mode {
                Mode::Inferred => self.out.escaping_control |= missing,
                Mode::Entry(_) => {}
                Mode::Written | Mode::Method => self.report(
                    Code::E0603,
                    range,
                    format!(
                        "this call needs `{name}`, which this function's signature does not list: \
                         add `with {name}`"
                    ),
                ),
            }
        }
        let needs = std::mem::take(&mut self.needs);
        for n in needs {
            let mut found = Vec::new();
            for t in &n.needs {
                let t = super::finish::erase(&self.solver.deep(t));
                if t.has_error() {
                    found.push(Evidence::Param(usize::MAX));
                    continue;
                }
                if let Some(&(w, k, _)) = n.installs.iter().rev().find(|(_, _, x)| *x == t) {
                    found.push(Evidence::Install(w, k));
                    continue;
                }
                if let Some(i) = self.own.iter().position(|x| *x == t) {
                    found.push(Evidence::Param(i));
                    continue;
                }
                found.push(Evidence::Param(usize::MAX));
                let name = show_effect(&t);
                match self.mode {
                    Mode::Inferred => {
                        if !self.out.escaping_needs.contains(&t) {
                            self.out.escaping_needs.push(t);
                        }
                    }
                    Mode::Written => self.report(
                        Code::E0603,
                        n.range,
                        format!(
                            "this call needs `{name}`, which this function's signature does not \
                             list: add `with {name}`, or install a handler around the call"
                        ),
                    ),
                    Mode::Method => self.report(
                        Code::E0603,
                        n.range,
                        format!(
                            "this call needs `{name}`, which a trait's method cannot use: install \
                             a handler around the call, or give the handler what it needs when it \
                             is built"
                        ),
                    ),
                    Mode::Entry(what) => self.report(
                        Code::E0602,
                        n.range,
                        format!(
                            "this call needs `{name}`, and nothing installs a handler for it: \
                             {what} is an entry point, so install one around the call with \
                             `with {name} = … {{ … }}`"
                        ),
                    ),
                }
            }
            self.out.evidence.insert(n.call, found);
        }
    }

    /// Where a value of type `found` is used as an `expected` that is a `dyn` borrow of a trait
    /// it implements (`&Clock` as `&dyn Time`), or a `&String` as a `&str`: records the conversion
    /// of `e` (and the bound), and says whether it is one.
    pub(super) fn unsize(&mut self, e: ExprId, found: &Ty, expected: &Ty) -> bool {
        let (f, x) = (self.norm(found), self.norm(expected));
        // A number where a wider number type that holds all its values is wanted
        // (`num.widen.1`).
        if crate::check::widens(&f, &x) {
            self.out.dyn_coercions.insert(e, x.clone());
            return true;
        }
        let (Ty::Ref(m1, a), Ty::Ref(m2, b)) = (&f, &x) else {
            return false;
        };
        if self.norm(a) == Ty::String && self.norm(b) == Ty::Str && (*m1 || !*m2) {
            self.out.dyn_coercions.insert(e, x.clone());
            return true;
        }
        // `&[T; N]` as `&[T]`.
        if let (Ty::Array(t, _), Ty::Slice(u)) = (self.norm(a), self.norm(b))
            && (*m1 || !*m2)
            && self.solver.unify(&t, &u)
        {
            self.out.dyn_coercions.insert(e, x.clone());
            return true;
        }
        let Ty::Dyn(tr) = self.norm(b) else {
            return false;
        };
        let a = self.norm(a);
        match &a {
            Ty::Infer(_) | Ty::Error | Ty::Never => return false,
            Ty::Dyn(d) if d.trait_ == tr.trait_ => return false,
            _ => {}
        }
        if *m2 && !*m1 {
            return false;
        }
        let range = self.range(e);
        if let Some(why) = self.items.not_dyn(tr.trait_, false) {
            self.report(
                Code::E0605,
                range,
                format!("`{}` cannot be used as `dyn`: {why}", tr.name),
            );
            return true;
        }
        self.require(a, tr.clone(), range);
        self.out.dyn_coercions.insert(e, x.clone());
        true
    }

    /// Gives each named effect of the signature (`with time: Time`) its type: a shared borrow
    /// of the effect.
    pub(super) fn effect_locals(&mut self, func: u32) {
        let sig = &self.items.fns[func as usize];
        for &(local, k) in &self.body.effect_locals {
            let t = match &sig.effects[k].ty {
                Ty::Error => Ty::Error,
                t => Ty::reference(false, t.clone()),
            };
            self.out.local_types[index(local)] = t;
        }
    }
}

/// For each call that needs effects: where each is met.
pub(super) type EvidenceMap = HashMap<ExprId, Vec<Evidence>>;
