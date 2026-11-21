//! Calls: what a callee names, its signature instantiated, and its arguments checked.

use moddil_syntax::TextRange;

use crate::hir::{ExprId, ExprKind};
use crate::items::FnSig;
use crate::traits::Answer;
use crate::ty::TraitRef;
use crate::{Code, Diagnostic, Ty};

use super::{Checker, index};
use crate::Callee;

impl Checker<'_> {
    /// The parameter types and return type of `sig` with type arguments `args`, after bounds on
    /// the arguments are recorded for checking.
    pub(super) fn instantiate_sig(
        &mut self,
        func: u32,
        args: &[Ty],
        range: TextRange,
    ) -> (Vec<Ty>, Ty) {
        let sig: &FnSig = &self.items.fns[func as usize];
        let params: Vec<Ty> = sig.params.iter().map(|p| p.ty.subst(args)).collect();
        let ret = sig.ret.subst(args);
        for p in sig.generics.preds.clone() {
            self.require(p.ty.subst(args), p.trait_ref.subst(args), range);
        }
        (params, ret)
    }

    /// Type arguments for a function's own parameters: those written (`want` of them), or new
    /// variables.
    pub(super) fn own_args(
        &mut self,
        given: Option<&[Ty]>,
        want: usize,
        range: TextRange,
        name: &str,
    ) -> Vec<Ty> {
        match given {
            Some(g) => {
                let mut ds = Vec::new();
                let ok = crate::resolve::arity(&format!("`{name}`"), want, g.len(), range, &mut ds);
                self.out.diagnostics.extend(ds);
                if ok {
                    g.iter().map(|t| self.instantiate(t)).collect()
                } else {
                    vec![Ty::Error; want]
                }
            }
            None => self.solver.fresh_n(want),
        }
    }

    /// Reports E0412 if `func` is a destructor, which is run by dropping, not by calling.
    pub(super) fn not_drop(&mut self, func: u32, range: TextRange) {
        let drop = self.items.lang("Drop");
        if drop.is_some() && self.items.trait_of(func) == drop {
            self.report(
                Code::E0412,
                range,
                "a destructor runs when its value is dropped, and cannot be called: to end a \
                 value early, write `drop(x)`",
            );
        }
    }

    /// What a callee expression calls: its function and type arguments.
    pub(super) fn callee(&mut self, callee: ExprId) -> Option<Callee> {
        let range = self.range(callee);
        match self.body.exprs[callee].kind.clone() {
            ExprKind::Fn { func, args } => {
                let args: Vec<Ty> = args.iter().map(|a| self.instantiate(a)).collect();
                Some(Callee::Fn { func, args })
            }
            ExprKind::TraitFn {
                func,
                trait_args,
                args,
                ..
            } => {
                let sig = &self.items.fns[func as usize];
                let own = sig.generics.params.len() - sig.generics.parent as usize;
                let name = sig.name.clone();
                let mut all = vec![self.fresh()];
                all.extend(trait_args.iter().map(|a| self.instantiate(a)));
                all.extend(self.own_args(args.as_deref(), own, range, &name));
                Some(Callee::Trait { func, args: all })
            }
            ExprKind::AssocFn {
                ty,
                name,
                name_range,
                args,
            } => {
                let ty = self.instantiate(&ty);
                self.assoc_fn(&ty, &name, name_range, args.as_deref())
            }
            _ => None,
        }
    }

    /// `Type::name`: a function of an inherent impl of `ty`, or else a method of a trait `ty`
    /// implements.
    pub(super) fn assoc_fn(
        &mut self,
        ty: &Ty,
        name: &str,
        range: TextRange,
        given: Option<&[Ty]>,
    ) -> Option<Callee> {
        let items = self.items;
        for imp in &items.impls {
            if imp.trait_ref.is_some() {
                continue;
            }
            let Some(&f) = imp
                .methods
                .iter()
                .find(|&&f| items.fns[f as usize].name == name)
            else {
                continue;
            };
            let saved = self.solver.table.clone();
            let mut args = self.solver.fresh_n(imp.generics.params.len());
            if self.solver.unify(&imp.self_ty.subst(&args), ty) {
                let sig = &items.fns[f as usize];
                let own = sig.generics.params.len() - sig.generics.parent as usize;
                args.extend(self.own_args(given, own, range, name));
                return Some(Callee::Fn { func: f, args });
            }
            self.solver.table = saved;
        }
        let mut found: Vec<(u32, Vec<Ty>)> = Vec::new();
        for (t, def) in items.traits.iter().enumerate() {
            let Some(&f) = def
                .methods
                .iter()
                .find(|&&f| items.fns[f as usize].name == name)
            else {
                continue;
            };
            let trait_args = self.solver.fresh_n(def.generics.params.len() - 1);
            let trait_ref = TraitRef {
                trait_: t as u32,
                name: def.name.as_str().into(),
                args: trait_args.clone().into(),
            };
            let saved = self.solver.table.clone();
            let answer = self.solver.implements(ty, &trait_ref);
            self.solver.table = saved;
            if answer != Answer::No {
                let mut args = vec![ty.clone()];
                args.extend(trait_args);
                found.push((f, args));
            }
        }
        match found.len() {
            0 => {
                let t = self.show(ty);
                let (code, what) = match &t {
                    Ty::Adt(a, ..) if items.adts[*a as usize].is_enum => {
                        (Code::E0101, "variant or function")
                    }
                    _ => (Code::E0403, "function"),
                };
                self.report(code, range, format!("`{t}` has no {what} called `{name}`"));
                None
            }
            1 => {
                let (f, mut args) = found.pop().expect("one");
                let sig = &items.fns[f as usize];
                let own = sig.generics.params.len() - sig.generics.parent as usize;
                args.extend(self.own_args(given, own, range, name));
                Some(Callee::Trait { func: f, args })
            }
            _ => {
                let names: Vec<String> = found
                    .iter()
                    .filter_map(|(f, _)| items.trait_of(*f))
                    .map(|t| format!("`{}::{name}`", items.traits[t as usize].name))
                    .collect();
                let t = self.show(ty);
                self.report(
                    Code::E0410,
                    range,
                    format!(
                        "more than one trait gives `{t}` a function `{name}`: write {}",
                        names.join(" or ")
                    ),
                );
                None
            }
        }
    }

    /// Checks the arguments of a call of `func` with parameter types `params`.
    pub(super) fn args(&mut self, range: TextRange, func: u32, params: &[Ty], args: &[ExprId]) {
        let sig = &self.items.fns[func as usize];
        if args.len() != params.len() {
            let d = Diagnostic::error(
                Code::E0202,
                range,
                format!(
                    "`{}` takes {} argument{}, but {} {} given",
                    sig.name,
                    params.len(),
                    if params.len() == 1 { "" } else { "s" },
                    args.len(),
                    if args.len() == 1 { "was" } else { "were" },
                ),
            )
            .with_label(sig.name_range, "defined here");
            self.out.diagnostics.push(d);
        }
        for (i, &a) in args.iter().enumerate() {
            match params.get(i) {
                Some(t) => {
                    self.check(a, t);
                }
                None => {
                    self.infer(a);
                }
            }
        }
    }

    pub(super) fn call(&mut self, e: ExprId, callee: ExprId, args: &[ExprId]) -> Ty {
        let range = self.range(e);
        let Some(target) = self.callee(callee) else {
            if !matches!(
                self.body.exprs[callee].kind,
                ExprKind::AssocFn { .. } | ExprKind::Missing
            ) {
                let t = self.infer(callee);
                let t = self.show(&t);
                if t != Ty::Error {
                    self.report(
                        Code::E0203,
                        self.range(callee),
                        format!("this is a `{t}`, not a function"),
                    );
                }
            }
            for &a in args {
                self.infer(a);
            }
            return Ty::Error;
        };
        self.out.expr_types[index(callee)] = Ty::Unit;
        let func = target.func();
        self.not_drop(func, range);
        let (params, ret) = self.instantiate_sig(func, target.args(), range);
        self.fallible_call(e, func, target.args(), range);
        self.out.calls.insert(e, target);
        self.args(range, func, &params, args);
        self.norm(&ret)
    }

    pub(super) fn method_call(
        &mut self,
        e: ExprId,
        receiver: ExprId,
        name: &str,
        name_range: TextRange,
        given: Option<&[Ty]>,
        args: &[ExprId],
    ) -> Ty {
        let range = self.range(e);
        let rt = self.infer(receiver);
        let r = self.norm(&rt);
        match r {
            Ty::Error | Ty::Never => {
                for &a in args {
                    self.infer(a);
                }
                return Ty::Error;
            }
            Ty::Infer(_) => {
                for &a in args {
                    self.infer(a);
                }
                return self.unknown(self.range(receiver), "to call a method on it");
            }
            _ => {}
        }
        let pick = match self.solver.lookup_method(&r, name) {
            Ok(Some(p)) => p,
            Ok(None) if self.show(&r).has_infer() => {
                for &a in args {
                    self.infer(a);
                }
                return self.unknown(self.range(receiver), "to call a method on it");
            }
            Ok(None) => {
                for &a in args {
                    self.infer(a);
                }
                let t = self.show(&r);
                let hint = match r.strip_ref() {
                    Ty::Adt(a, ..)
                        if !self.items.adts[*a as usize].is_enum
                            && self.items.variant(*a, 0).field(name).is_some() =>
                    {
                        format!(": `{name}` is a field; to call it, write `({name})`")
                    }
                    _ => String::new(),
                };
                self.report(
                    Code::E0403,
                    name_range,
                    format!("`{t}` has no method called `{name}`{hint}"),
                );
                return Ty::Error;
            }
            Err(traits) => {
                for &a in args {
                    self.infer(a);
                }
                let t = self.show(&r);
                self.report(
                    Code::E0410,
                    name_range,
                    format!(
                        "more than one trait gives `{t}` a method `{name}` ({}): call it as \
                         `Trait::{name}(x)`",
                        traits.join(", ")
                    ),
                );
                return Ty::Error;
            }
        };
        let sig = &self.items.fns[pick.func as usize];
        let own = sig.generics.params.len() - sig.generics.parent as usize;
        let mut targs = pick.parent_args.clone();
        targs.extend(self.own_args(given, own, name_range, name));
        let target = match &pick.trait_ref {
            Some(_) => Callee::Trait {
                func: pick.func,
                args: targs,
            },
            None => Callee::Fn {
                func: pick.func,
                args: targs,
            },
        };
        self.not_drop(pick.func, name_range);
        let (params, ret) = self.instantiate_sig(pick.func, target.args(), range);
        // The receiver, adjusted to the method's `self`.
        let base = match (&r, pick.adjust.deref) {
            (Ty::Ref(_, inner), true) => (**inner).clone(),
            _ => r.clone(),
        };
        let adjusted = match pick.adjust.autoref {
            None => base,
            Some(m) => Ty::reference(m, base),
        };
        if let Some(p0) = params.first() {
            self.solver.unify(&adjusted, p0);
        }
        if pick.adjust.autoref == Some(true) {
            if pick.adjust.deref {
                if let Ty::Ref(false, _) = r {
                    self.report(
                        Code::E0302,
                        self.range(receiver),
                        format!(
                            "`{name}` takes `&mut self`, but this is behind a shared borrow (`&`)"
                        ),
                    );
                }
            } else {
                self.check_mutable(receiver, "borrowed as `&mut`");
            }
        }
        self.out.adjusts.insert(e, pick.adjust);
        self.fallible_call(e, pick.func, target.args(), range);
        self.out.calls.insert(e, target);
        let func = pick.func;
        self.args(range, func, params.get(1..).unwrap_or(&[]), args);
        self.norm(&ret)
    }
}
