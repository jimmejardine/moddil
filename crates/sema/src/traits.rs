//! Traits: unifying types, deciding whether a type implements a trait (and with which impl),
//! normalising associated types, and finding methods.
//!
//! The same [`Solver`] serves the type checker, which has inference variables, and later phases
//! (MIR building, the interpreter's witness tables), which ask about types with type parameters
//! or concrete types only.

use std::sync::Arc;

use crate::Ty;
use crate::items::{COPY_CAP, Container, ItemTree, Pred, SelfKind};
use crate::ty::{AssocTy, TraitRef};

/// Inference variables and what they are bound to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Table {
    vars: Vec<Option<Ty>>,
}

impl Table {
    /// A new variable.
    pub fn fresh(&mut self) -> Ty {
        self.vars.push(None);
        Ty::Infer(self.vars.len() as u32 - 1)
    }

    /// `t`, with the variable at its top replaced by what it is bound to, repeatedly.
    pub fn shallow(&self, t: &Ty) -> Ty {
        let mut t = t.clone();
        while let Ty::Infer(v) = t {
            match self.vars.get(v as usize).and_then(Option::as_ref) {
                Some(b) => t = b.clone(),
                None => break,
            }
        }
        t
    }

    /// `t` with every bound variable inside it replaced.
    pub fn resolve(&self, t: &Ty) -> Ty {
        match self.shallow(t) {
            Ty::Infer(v) => Ty::Infer(v),
            t => t.map(&mut |x| self.resolve(x)),
        }
    }

    fn occurs(&self, v: u32, t: &Ty) -> bool {
        self.resolve(t).any(&|x| *x == Ty::Infer(v))
    }

    fn bind(&mut self, v: u32, t: Ty) -> bool {
        if t == Ty::Infer(v) {
            return true;
        }
        if self.occurs(v, &t) {
            return false;
        }
        self.vars[v as usize] = Some(t);
        true
    }
}

/// How a trait is implemented for a type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// By an impl, with the impl's type arguments.
    Impl(u32, Vec<Ty>),
    /// By a bound in scope (`T: Clone`), by its index in the solver's environment.
    Bound(usize),
    /// By the language: `Copy` and `Clone` for tuples, shared borrows and `!`.
    Builtin,
}

/// Whether a type implements a trait.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    /// Yes, this way.
    Yes(Source),
    /// No.
    No,
    /// Not known until more is inferred.
    Maybe,
}

/// How a method call's receiver is adjusted to the method's `self`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Adjust {
    /// The receiver is a borrow, and the method takes what it points to (`*r`).
    pub deref: bool,
    /// The method borrows the receiver: `Some(true)` for `&mut`.
    pub autoref: Option<bool>,
}

/// The method a call resolves to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pick {
    /// The method's declaration: in an inherent impl, or in a trait.
    pub func: u32,
    /// The type arguments for the method's parent (the impl's, or `Self` and the trait's).
    pub parent_args: Vec<Ty>,
    /// For a trait's method: the trait.
    pub trait_ref: Option<TraitRef>,
    /// How the receiver is adjusted.
    pub adjust: Adjust,
}

/// A solver: an item tree, the bounds in scope and inference variables.
pub struct Solver<'a> {
    /// The items.
    pub tree: &'a ItemTree,
    /// The bounds in scope, with the bounds they imply through supertraits.
    pub env: Vec<Pred>,
    /// The inference variables.
    pub table: Table,
    depth: u32,
}

/// `preds` with the supertrait bounds they imply, transitively.
pub fn elaborate(tree: &ItemTree, preds: &[Pred]) -> Vec<Pred> {
    let mut out: Vec<Pred> = Vec::new();
    let mut work: Vec<Pred> = preds.to_vec();
    while let Some(p) = work.pop() {
        if out
            .iter()
            .any(|q| q.ty == p.ty && q.trait_ref == p.trait_ref)
        {
            continue;
        }
        let def = &tree.traits[p.trait_ref.trait_ as usize];
        let mut args = vec![p.ty.clone()];
        args.extend(p.trait_ref.args.iter().cloned());
        for s in &def.supertraits {
            work.push(Pred {
                ty: p.ty.clone(),
                trait_ref: s.subst(&args),
                assoc: Vec::new(),
                range: p.range,
            });
        }
        out.push(p);
    }
    out.reverse();
    out
}

impl<'a> Solver<'a> {
    /// A solver with the bounds `preds` in scope.
    pub fn new(tree: &'a ItemTree, preds: &[Pred]) -> Self {
        Solver {
            tree,
            env: elaborate(tree, preds),
            table: Table::default(),
            depth: 0,
        }
    }

    /// `t` resolved at its top, with an associated type at its top normalised if it can be.
    pub fn norm(&mut self, t: &Ty) -> Ty {
        match self.table.shallow(t) {
            Ty::Assoc(a) => self.normalize_assoc(&a),
            t => t,
        }
    }

    /// `t` resolved, with every associated type inside it normalised where it can be.
    pub fn deep(&mut self, t: &Ty) -> Ty {
        let t = self.norm(t);
        t.map(&mut |x| self.deep(x))
    }

    fn normalize_assoc(&mut self, a: &AssocTy) -> Ty {
        let self_ty = self.deep(&a.self_ty);
        let trait_ref = a.trait_ref.map(&mut |x| self.deep(x));
        let unchanged = || {
            Ty::Assoc(Arc::new(AssocTy {
                self_ty: self_ty.clone(),
                trait_ref: trait_ref.clone(),
                index: a.index,
                name: a.name.clone(),
            }))
        };
        if matches!(self_ty, Ty::Infer(_)) {
            return unchanged();
        }
        if self_ty == Ty::Error {
            return Ty::Error;
        }
        // A closure's `Output` and `Fails` are its result and errors.
        if let Ty::Closure(c) = &self_ty
            && self.tree.lang("FnOnce") == Some(trait_ref.trait_)
        {
            return match a.name.as_ref() {
                "Output" => self.norm(&c.ret),
                _ => self.norm(&c.fails),
            };
        }
        // A bound that fixes it: `T: Add<Output = int>`.
        for i in 0..self.env.len() {
            let p = &self.env[i];
            if p.ty == self_ty
                && p.trait_ref == trait_ref
                && let Some((_, t)) = p.assoc.iter().find(|(k, _)| *k == a.index)
            {
                let t = t.clone();
                return self.norm(&t);
            }
        }
        // An opaque type's bound that fixes it: `impl Iterator<Item = int>`.
        for p in self.opaque_preds(&self_ty) {
            if p.trait_ref.trait_ == trait_ref.trait_
                && let Some((_, t)) = p.assoc.iter().find(|(k, _)| *k == a.index)
            {
                return self.norm(t);
            }
        }
        match self.implements(&self_ty, &trait_ref) {
            Answer::Yes(Source::Impl(i, args)) => {
                match self.tree.impls[i as usize].assoc.get(a.index as usize) {
                    Some(Some(t)) => {
                        let t = t.subst(&args);
                        self.norm(&t)
                    }
                    _ => Ty::Error,
                }
            }
            _ => unchanged(),
        }
    }

    /// Makes `a` and `b` the same type, binding variables as needed. `Error` is the same as
    /// anything.
    pub fn unify(&mut self, a: &Ty, b: &Ty) -> bool {
        let a = self.norm(a);
        let b = self.norm(b);
        match (&a, &b) {
            (Ty::Error, _) | (_, Ty::Error) => true,
            (Ty::Infer(x), Ty::Infer(y)) if x == y => true,
            (Ty::Infer(x), t) | (t, Ty::Infer(x)) => self.table.bind(*x, t.clone()),
            (Ty::Tuple(xs), Ty::Tuple(ys)) => {
                xs.len() == ys.len() && xs.iter().zip(ys.iter()).all(|(x, y)| self.unify(x, y))
            }
            (Ty::Adt(i, _, xs), Ty::Adt(j, _, ys)) => {
                i == j && xs.iter().zip(ys.iter()).all(|(x, y)| self.unify(x, y))
            }
            (Ty::Ref(m, x), Ty::Ref(n, y)) => m == n && self.unify(x, y),
            (Ty::Array(x, n), Ty::Array(y, k)) => self.unify(x, y) && self.unify(n, k),
            (Ty::Slice(x), Ty::Slice(y)) | (Ty::Buf(x), Ty::Buf(y)) => self.unify(x, y),
            (Ty::Anon(x), Ty::Anon(y)) => x.owner == y.owner && x.expr == y.expr,
            (Ty::Opaque(x), Ty::Opaque(y)) => {
                x.func == y.func
                    && x.args.len() == y.args.len()
                    && x.args
                        .iter()
                        .zip(y.args.iter())
                        .all(|(a, b)| self.unify(a, b))
            }
            (Ty::Closure(x), Ty::Closure(y)) => {
                x.owner == y.owner
                    && x.expr == y.expr
                    && x.params.len() == y.params.len()
                    && x.params
                        .iter()
                        .zip(y.params.iter())
                        .all(|(p, q)| self.unify(p, q))
                    && self.unify(&x.ret, &y.ret)
            }
            (Ty::Dyn(x), Ty::Dyn(y)) => {
                x.trait_ == y.trait_
                    && x.args
                        .iter()
                        .zip(y.args.iter())
                        .all(|(p, q)| self.unify(p, q))
            }
            (Ty::Errors(xs), Ty::Errors(ys)) => {
                xs.len() == ys.len() && xs.iter().zip(ys.iter()).all(|(x, y)| self.unify(x, y))
            }
            (Ty::Assoc(x), Ty::Assoc(y)) => {
                x.index == y.index
                    && x.trait_ref.trait_ == y.trait_ref.trait_
                    && self.unify(&x.self_ty, &y.self_ty)
                    && x.trait_ref
                        .args
                        .iter()
                        .zip(y.trait_ref.args.iter())
                        .all(|(p, q)| self.unify(p, q))
            }
            (x, y) => x == y,
        }
    }

    /// Whether `a` and `b` can be unified, without binding anything.
    pub fn can_unify(&mut self, a: &Ty, b: &Ty) -> bool {
        let saved = self.table.clone();
        let ok = self.unify(a, b);
        self.table = saved;
        ok
    }

    /// `n` fresh variables.
    pub fn fresh_n(&mut self, n: usize) -> Vec<Ty> {
        (0..n).map(|_| self.table.fresh()).collect()
    }

    fn is_lang(&self, trait_: u32, name: &str) -> bool {
        self.tree.lang(name) == Some(trait_)
    }

    /// Whether `ty` implements `trait_ref`, and how. If exactly one impl applies, the variables
    /// it fixes stay bound.
    pub fn implements(&mut self, ty: &Ty, trait_ref: &TraitRef) -> Answer {
        if self.depth > 32 {
            return Answer::Maybe;
        }
        self.depth += 1;
        let a = self.implements_inner(ty, trait_ref);
        self.depth -= 1;
        a
    }

    fn implements_inner(&mut self, ty: &Ty, trait_ref: &TraitRef) -> Answer {
        let ty = self.norm(ty);
        let tr = trait_ref.trait_;
        match &ty {
            Ty::Infer(_) => return Answer::Maybe,
            Ty::Error | Ty::Never => return Answer::Yes(Source::Builtin),
            // A refined type has its base's traits (`refine.base.1`).
            Ty::Refined(..) => {
                let base = self.tree.base(&ty);
                return self.implements(&base, trait_ref);
            }
            _ => {}
        }
        // A closure implements the closure traits for its parameters' types; whether it may be
        // called through `FnMut` or `Fn` is checked once its captures are known.
        if let Ty::Closure(c) = &ty
            && (self.is_lang(tr, "FnOnce") || self.is_lang(tr, "FnMut") || self.is_lang(tr, "Fn"))
        {
            let args = Ty::tuple(c.params.to_vec());
            return match trait_ref.args.first() {
                Some(a) if self.unify(a, &args) => Answer::Yes(Source::Builtin),
                Some(_) => Answer::No,
                None => Answer::Yes(Source::Builtin),
            };
        }
        // An anonymous impl implements its trait.
        if let Ty::Anon(a) = &ty
            && a.trait_ref.trait_ == tr
        {
            return if a
                .trait_ref
                .args
                .iter()
                .zip(trait_ref.args.iter())
                .all(|(x, y)| self.unify(x, y))
            {
                Answer::Yes(Source::Builtin)
            } else {
                Answer::No
            };
        }
        // `dyn Trait` implements its trait and the trait's supertraits.
        if let Ty::Dyn(d) = &ty {
            let pred = Pred {
                ty: ty.clone(),
                trait_ref: d.clone(),
                assoc: Vec::new(),
                range: moddil_syntax::TextRange::new(0, 0),
            };
            for p in elaborate(self.tree, &[pred]) {
                if p.trait_ref.trait_ != tr {
                    continue;
                }
                let saved = self.table.clone();
                if p.trait_ref
                    .args
                    .iter()
                    .zip(trait_ref.args.iter())
                    .all(|(x, y)| self.unify(x, y))
                {
                    return Answer::Yes(Source::Builtin);
                }
                self.table = saved;
            }
        }
        if self.is_lang(tr, "Copy") || self.is_lang(tr, "Clone") {
            match &ty {
                Ty::Ref(false, _) => return Answer::Yes(Source::Builtin),
                Ty::Ref(true, _) => return Answer::No,
                // A tuple is `Copy` and `Clone` if its elements are `Copy`; it is then cloned by
                // copying it.
                Ty::Tuple(ts) => {
                    let mut all = Answer::Yes(Source::Builtin);
                    let Some(copy) = self.tree.lang_ref("Copy") else {
                        return Answer::No;
                    };
                    for t in ts.iter() {
                        match self.implements(t, &copy) {
                            Answer::Yes(_) => {}
                            Answer::No => return Answer::No,
                            Answer::Maybe => all = Answer::Maybe,
                        }
                    }
                    return all;
                }
                // So is an array of `Copy` elements, within the size cap.
                Ty::Array(t, _) => {
                    let Some(copy) = self.tree.lang_ref("Copy") else {
                        return Answer::No;
                    };
                    let t = (**t).clone();
                    return match self.implements(&t, &copy) {
                        Answer::Yes(_)
                            if ty.has_infer()
                                || ty.has_param()
                                || self.tree.size_of(&ty).0 <= COPY_CAP =>
                        {
                            Answer::Yes(Source::Builtin)
                        }
                        Answer::Yes(_) | Answer::No => Answer::No,
                        Answer::Maybe => Answer::Maybe,
                    };
                }
                _ => {}
            }
        }
        // An opaque type's bounds (`type.impl.ret.1`).
        for p in self.opaque_preds(&ty) {
            if p.trait_ref.trait_ == tr {
                let saved = self.table.clone();
                if p.trait_ref
                    .args
                    .iter()
                    .zip(trait_ref.args.iter())
                    .all(|(x, y)| self.unify(x, y))
                {
                    return Answer::Yes(Source::Builtin);
                }
                self.table = saved;
            }
        }
        // A bound in scope.
        for i in 0..self.env.len() {
            let p = self.env[i].clone();
            if p.trait_ref.trait_ == tr && self.can_unify(&p.ty, &ty) {
                let saved = self.table.clone();
                let ok = self.unify(&p.ty, &ty)
                    && p.trait_ref
                        .args
                        .iter()
                        .zip(trait_ref.args.iter())
                        .all(|(x, y)| self.unify(x, y));
                if ok {
                    return Answer::Yes(Source::Bound(i));
                }
                self.table = saved;
            }
        }
        // Impls: try each, and keep the one that applies.
        let mut found: Option<(u32, Table, Vec<Ty>)> = None;
        let mut maybe = false;
        for i in 0..self.tree.impls.len() {
            let imp = &self.tree.impls[i];
            let Some(itr) = &imp.trait_ref else { continue };
            if itr.trait_ != tr {
                continue;
            }
            let saved = self.table.clone();
            let args = self.fresh_n(imp.generics.params.len());
            let self_ty = imp.self_ty.subst(&args);
            let itr = itr.subst(&args);
            let mut ok = self.unify(&self_ty, &ty)
                && itr
                    .args
                    .iter()
                    .zip(trait_ref.args.iter())
                    .all(|(x, y)| self.unify(x, y));
            let mut this_maybe = false;
            if ok {
                for p in imp.generics.preds.clone() {
                    let pty = self.deep(&p.ty.subst(&args));
                    let ptr = p.trait_ref.subst(&args).map(&mut |t| self.deep(t));
                    match self.implements(&pty, &ptr) {
                        Answer::Yes(_) => {}
                        Answer::No => {
                            ok = false;
                            break;
                        }
                        Answer::Maybe => this_maybe = true,
                    }
                    // What the bound fixes binds the impl's parameters: `F: FnMut(A) -> B`
                    // makes `B` the closure's result.
                    for (k, fixed) in &p.assoc {
                        let name = self.tree.traits[ptr.trait_ as usize].assoc[*k as usize]
                            .0
                            .clone();
                        let a = Ty::Assoc(Arc::new(AssocTy {
                            self_ty: pty.clone(),
                            trait_ref: ptr.clone(),
                            index: *k,
                            name: name.as_str().into(),
                        }));
                        let got = self.norm(&a);
                        if !matches!(got, Ty::Assoc(_)) && !self.unify(&got, &fixed.subst(&args)) {
                            ok = false;
                        }
                    }
                    if !ok {
                        break;
                    }
                }
            }
            if ok && self.is_lang(tr, "Copy") && !ty.has_infer() && !ty.has_param() {
                // A generic type is `Copy` only for arguments that keep it within the cap.
                ok = self.tree.size_of(&ty).0 <= COPY_CAP;
            }
            if ok && !this_maybe {
                let args: Vec<Ty> = args.iter().map(|a| self.table.resolve(a)).collect();
                if found.is_some() {
                    maybe = true;
                }
                found = Some((i as u32, self.table.clone(), args));
            } else if ok {
                maybe = true;
            }
            self.table = saved;
        }
        match found {
            Some((i, table, args)) if !maybe => {
                self.table = table;
                Answer::Yes(Source::Impl(i, args))
            }
            // No impl's type unifies with `ty`, even where it is still being inferred.
            _ if maybe => Answer::Maybe,
            _ => Answer::No,
        }
    }

    /// Whether values of `ty` are copied rather than moved.
    pub fn is_copy(&mut self, ty: &Ty) -> bool {
        let Some(copy) = self.tree.lang_ref("Copy") else {
            return false;
        };
        matches!(self.implements(ty, &copy), Answer::Yes(_))
    }

    /// Whether dropping a value of `ty` may run code: it has a destructor, or contains a value
    /// that has one, or its type is not known here (a type parameter).
    pub fn needs_drop(&mut self, ty: &Ty) -> bool {
        let ty = self.deep(ty);
        self.needs_drop_inner(&ty, 0)
    }

    fn needs_drop_inner(&mut self, ty: &Ty, depth: u32) -> bool {
        if depth > 16 {
            return true;
        }
        match ty {
            Ty::Int
            | Ty::Bool
            | Ty::Unit
            | Ty::Never
            | Ty::Error
            | Ty::Ref(..)
            | Ty::Dyn(_)
            | Ty::Char
            | Ty::Str
            | Ty::String
            | Ty::Len(_) => false,
            Ty::Array(t, _) | Ty::Slice(t) | Ty::Buf(t) => self.needs_drop_inner(t, depth + 1),
            Ty::Refined(..) => {
                let base = self.tree.base(ty);
                self.needs_drop_inner(&base, depth + 1)
            }
            Ty::Param(..) | Ty::Opaque(_) | Ty::Assoc(_) | Ty::Infer(_) => true,
            Ty::Errors(ts) => ts.iter().any(|t| self.needs_drop_inner(t, depth + 1)),
            Ty::Closure(c) => c
                .captures
                .iter()
                .any(|t| self.needs_drop_inner(t, depth + 1)),
            Ty::Anon(a) => a
                .methods
                .iter()
                .any(|(_, t)| self.needs_drop_inner(t, depth + 1)),
            Ty::Tuple(ts) => ts.iter().any(|t| self.needs_drop_inner(t, depth + 1)),
            Ty::Adt(a, _, args) => {
                if self.tree.has_drop_impl(*a) {
                    return true;
                }
                let def = &self.tree.adts[*a as usize];
                let fields: Vec<Ty> = def
                    .variants
                    .iter()
                    .flat_map(|v| &v.fields)
                    .map(|f| f.ty.subst(args))
                    .collect();
                fields.iter().any(|t| self.needs_drop_inner(t, depth + 1))
            }
        }
    }

    /// The bounds of opaque type `t`, with those they imply, for its type arguments; none if `t`
    /// is not one.
    fn opaque_preds(&self, t: &Ty) -> Vec<Pred> {
        let Ty::Opaque(o) = t else {
            return Vec::new();
        };
        let Some(sig) = self.tree.fns.get(o.func as usize) else {
            return Vec::new();
        };
        elaborate(self.tree, &sig.opaque)
            .into_iter()
            .map(|p| Pred {
                ty: t.clone(),
                trait_ref: p.trait_ref.subst(&o.args),
                assoc: p
                    .assoc
                    .iter()
                    .map(|(k, x)| (*k, x.subst(&o.args)))
                    .collect(),
                range: p.range,
            })
            .collect()
    }

    /// The method called `name` for a receiver of type `receiver`, trying the receiver itself,
    /// then a borrow of it, then a `&mut` borrow, and then the same for what it points to if it
    /// is a borrow. Methods of inherent impls come before trait methods. `Err(names)` when
    /// several traits' methods apply.
    pub fn lookup_method(
        &mut self,
        receiver: &Ty,
        name: &str,
    ) -> Result<Option<Pick>, Vec<String>> {
        let r = self.norm(receiver);
        let mut steps = vec![(r.clone(), false)];
        if let Ty::Ref(_, inner) = &r {
            steps.push(((**inner).clone(), true));
        }
        // A refined value has its base's methods (`refine.base.1`).
        let last = steps.last().cloned();
        if let Some((t @ Ty::Refined(..), deref)) = last {
            steps.push((self.tree.base(&t), deref));
        }
        for (u, deref) in steps {
            for autoref in [None, Some(false), Some(true)] {
                let x = match autoref {
                    None => u.clone(),
                    Some(m) => Ty::reference(m, u.clone()),
                };
                let adjust = Adjust { deref, autoref };
                if let Some(p) = self.inherent(&x, name, adjust) {
                    return Ok(Some(p));
                }
                let picks = self.trait_methods(&x, name, adjust);
                match picks.len() {
                    0 => {}
                    1 => return Ok(picks.into_iter().next()),
                    _ => {
                        return Err(picks
                            .iter()
                            .filter_map(|p| p.trait_ref.as_ref().map(|t| t.to_string()))
                            .collect());
                    }
                }
            }
        }
        Ok(None)
    }

    /// The `Self` type a method with `self` of kind `kind` has when called with a receiver of
    /// type `x`.
    fn self_for(&mut self, kind: SelfKind, x: &Ty) -> Option<Ty> {
        match (kind, self.norm(x)) {
            (SelfKind::Value, t) => Some(t),
            (SelfKind::Ref, Ty::Ref(false, t)) | (SelfKind::RefMut, Ty::Ref(true, t)) => {
                Some((*t).clone())
            }
            _ => None,
        }
    }

    fn inherent(&mut self, x: &Ty, name: &str, adjust: Adjust) -> Option<Pick> {
        for i in 0..self.tree.impls.len() {
            let imp = &self.tree.impls[i];
            if imp.trait_ref.is_some() {
                continue;
            }
            let Some(&f) = imp
                .methods
                .iter()
                .find(|&&f| self.tree.fns[f as usize].name == name)
            else {
                continue;
            };
            let Some(kind) = self.tree.fns[f as usize].self_kind else {
                continue;
            };
            let Some(self_ty) = self.self_for(kind, x) else {
                continue;
            };
            let saved = self.table.clone();
            let args = self.fresh_n(imp.generics.params.len());
            let imp_self = imp.self_ty.subst(&args);
            if self.unify(&imp_self, &self_ty) {
                return Some(Pick {
                    func: f,
                    parent_args: args,
                    trait_ref: None,
                    adjust,
                });
            }
            self.table = saved;
        }
        None
    }

    fn trait_methods(&mut self, x: &Ty, name: &str, adjust: Adjust) -> Vec<Pick> {
        let mut out = Vec::new();
        for t in 0..self.tree.traits.len() {
            let def = &self.tree.traits[t];
            let Some(&f) = def
                .methods
                .iter()
                .find(|&&f| self.tree.fns[f as usize].name == name)
            else {
                continue;
            };
            let Some(kind) = self.tree.fns[f as usize].self_kind else {
                continue;
            };
            let Some(self_ty) = self.self_for(kind, x) else {
                continue;
            };
            let saved = self.table.clone();
            let args = self.fresh_n(def.generics.params.len() - 1);
            let trait_ref = TraitRef {
                trait_: t as u32,
                name: def.name.as_str().into(),
                args: args.into(),
            };
            match self.implements(&self_ty, &trait_ref) {
                Answer::No => self.table = saved,
                _ => {
                    let mut parent_args = vec![self_ty];
                    parent_args.extend(trait_ref.args.iter().cloned());
                    out.push(Pick {
                        func: f,
                        parent_args,
                        trait_ref: Some(trait_ref),
                        adjust,
                    });
                    if out.len() > 1 {
                        self.table = saved;
                    }
                }
            }
        }
        out
    }

    /// The function an impl uses for trait method `func` (a method declared in a trait), and
    /// the type arguments for that function's parent: the impl's method and arguments, or, if
    /// the impl does not define it, the trait's default with `Self` and the trait's arguments.
    pub fn impl_method(
        &mut self,
        func: u32,
        self_ty: &Ty,
        trait_ref: &TraitRef,
    ) -> Option<(u32, Vec<Ty>)> {
        let name = self.tree.fns[func as usize].name.clone();
        match self.implements(self_ty, trait_ref) {
            Answer::Yes(Source::Impl(i, args)) => {
                let imp = &self.tree.impls[i as usize];
                match imp
                    .methods
                    .iter()
                    .find(|&&f| self.tree.fns[f as usize].name == name)
                {
                    Some(&f) => Some((f, args)),
                    None => {
                        let mut parent = vec![self.deep(self_ty)];
                        parent.extend(trait_ref.args.iter().map(|t| self.deep(t)));
                        Some((func, parent))
                    }
                }
            }
            _ => None,
        }
    }
}

impl ItemTree {
    /// Whether struct or enum `adt` has a hand-written destructor.
    pub fn has_drop_impl(&self, adt: u32) -> bool {
        let Some(drop) = self.lang("Drop") else {
            return false;
        };
        self.impls.iter().any(|i| {
            i.trait_ref.as_ref().is_some_and(|t| t.trait_ == drop) && i.self_ty.adt() == Some(adt)
        })
    }

    /// Whether `func` is a method of a trait, or of an impl of one: the trait.
    pub fn trait_of(&self, func: u32) -> Option<u32> {
        match self.fns[func as usize].container {
            Container::Trait(t) => Some(t),
            Container::Impl(i) => self.impls[i as usize].trait_ref.as_ref().map(|t| t.trait_),
            Container::Free => None,
        }
    }
}
