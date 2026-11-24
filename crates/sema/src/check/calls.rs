//! Calls: what a callee names, its signature instantiated, and its arguments checked.

use moddil_syntax::TextRange;

use crate::hir::{ExprId, ExprKind};
use crate::items::FnSig;
use crate::traits::Answer;
use crate::ty::TraitRef;
use crate::{Code, Diagnostic, Item, Ty};

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
            let (ty, tr) = (p.ty.subst(args), p.trait_ref.subst(args));
            // A closure bound that lists control effects takes them (`expr.closure.control.1`).
            if p.control != 0 {
                self.bound_control.push((ty.clone(), p.control));
            }
            for (i, fixed) in &p.assoc {
                let name = self.items.traits[tr.trait_ as usize].assoc[*i as usize]
                    .0
                    .clone();
                let a = Ty::Assoc(std::sync::Arc::new(crate::ty::AssocTy {
                    self_ty: ty.clone(),
                    trait_ref: tr.clone(),
                    index: *i,
                    name: name.as_str().into(),
                }));
                self.assoc_eqs.push((a, fixed.subst(args), range));
            }
            self.require(ty, tr, range);
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
            // The function without `self`, rather than a method of the same name
            // (`item.impl.same-name.1`).
            let named = |f: &&u32| items.fns[**f as usize].name == name;
            let Some(&f) = imp
                .methods
                .iter()
                .filter(named)
                .find(|&&f| items.fns[f as usize].self_kind.is_none())
                .or_else(|| imp.methods.iter().find(named))
            else {
                continue;
            };
            let saved = self.solver.table.clone();
            let mut args = self.solver.fresh_n(imp.generics.params.len());
            if self.solver.unify(&imp.self_ty.subst(&args), ty) {
                let sig = &items.fns[f as usize];
                let own = sig.generics.params.len() - sig.generics.parent as usize;
                args.extend(self.own_args(given, own, range, name));
                self.method_visible(f, range);
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
    pub(super) fn args(
        &mut self,
        e: ExprId,
        range: TextRange,
        func: u32,
        params: &[Ty],
        args: &[ExprId],
    ) {
        let sig = &self.items.fns[func as usize];
        let offset = sig.params.len().saturating_sub(params.len());
        let named = args.iter().any(|a| self.body.labels.contains_key(a));
        let defaults = (offset..sig.params.len()).any(|k| self.items.default_of(func, k).is_some());
        if named || defaults {
            self.arranged_args(e, range, func, params, args, offset);
            return;
        }
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
            ) && let Some(t) = self.closure_call(e, callee, args)
            {
                return t;
            }
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
        // A C function is called only in an `unsafe` block (`ffi.extern.1`).
        if self.items.fns[func as usize].foreign && !self.body.in_unsafe.contains(&e) {
            self.report(
                Code::E0218,
                range,
                "a C function is called only in an `unsafe(\"why\") { … }` block: nothing checks \
                 what it does",
            );
        }
        let (params, ret) = self.instantiate_sig(func, target.args(), range);
        self.needs_of_call(e, func, target.args(), range);
        let targs = target.args().to_vec();
        self.out.calls.insert(e, target);
        self.args(e, range, func, &params, args);
        self.settle_assoc_eqs();
        // After the arguments, since a closure argument's errors are the call's.
        self.fallible_call(e, func, &targs, range);
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
        // A literal whose type nothing else gives yet has its default here (`num.lit.1`).
        self.solver.table.default_literal(&rt);
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
        // A method of what the receiver dereferences to, through `Deref`, if it has none of its
        // own (`expr.method.deref.1`): a `Box`'s value, a lock's guard's.
        let mut lookup = self.solver.lookup_method(&r, name);
        let mut through = None;
        if matches!(lookup, Ok(None))
            && let Some((base, target)) = self.deref_target(&r)
            && let Ok(Some(p)) = self.solver.lookup_method(&target, name)
        {
            lookup = Ok(Some(p));
            through = Some((base, target));
        }
        let r = match &through {
            Some((_, target)) => target.clone(),
            None => r,
        };
        let pick = match lookup {
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
        self.method_visible(pick.func, name_range);
        // An `Rc` is shared, not cloned (`rc.clone.1`): its `Clone` is for generic code, and
        // for derived and template code, which no one reads as written.
        let written = match self.owner.owner() {
            Item::Fn(i) => {
                let sig = &self.items.fns[i as usize];
                sig.src.is_file() && sig.origin.is_none() && sig.template.is_none()
            }
            _ => true,
        };
        if written
            && name == "clone"
            && pick
                .trait_ref
                .as_ref()
                .is_some_and(|t| self.items.lang("Clone") == Some(t.trait_))
            && let Some(a) = self.norm(r.strip_ref()).adt()
            && self.items.adts[a as usize].src == crate::items::Src::Prelude
            && matches!(self.items.adts[a as usize].name.as_str(), "Rc" | "Weak")
        {
            let ty = self.items.adts[a as usize].name.clone();
            self.report(
                Code::E0417,
                name_range,
                format!(
                    "`{ty}` is shared, not cloned: write `.share()`, which gives another handle \
                     to the same value"
                ),
            );
        }
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
        if let Some((base, _)) = through {
            let mutable = pick.adjust.autoref == Some(true);
            let (tr, method) = if mutable {
                ("DerefMut", "deref_mut")
            } else {
                ("Deref", "deref")
            };
            if let Some((trait_ref, func)) = self.op_trait(tr, method) {
                if mutable && self.solver.implements(&base, &trait_ref) == Answer::No {
                    let s = self.show(&base);
                    self.report(
                        Code::E0205,
                        self.range(receiver),
                        format!(
                            "`{name}` takes `&mut self`, and `{s}` does not implement `DerefMut`"
                        ),
                    );
                }
                self.require(base.clone(), trait_ref, range);
                self.out.method_derefs.insert(
                    e,
                    Callee::Trait {
                        func,
                        args: vec![base],
                    },
                );
            }
            if mutable && let Ty::Ref(false, _) = self.norm(&self.out.expr(receiver)) {
                self.report(
                    Code::E0302,
                    self.range(receiver),
                    format!("`{name}` takes `&mut self`, but this is behind a shared borrow (`&`)"),
                );
            } else if mutable && !matches!(self.norm(&self.out.expr(receiver)), Ty::Ref(..)) {
                self.check_mutable(receiver, "borrowed as `&mut`");
            }
        } else if pick.adjust.autoref == Some(true) {
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
        // A task's closure, whose handlers are checked against its scope (`task.with.1`).
        if self.items.fns[pick.func as usize].name == "spawn"
            && let crate::items::Container::Impl(k) = self.items.fns[pick.func as usize].container
            && let Some(a) = self.items.impls[k as usize].self_ty.adt()
            && self.items.adts[a as usize].name == "Scope"
            && self.items.adts[a as usize].src == crate::items::Src::Prelude
            && let Some(&f) = args.first()
        {
            self.spawns.push((e, receiver, f));
        }
        self.needs_of_call(e, pick.func, target.args(), range);
        let targs = target.args().to_vec();
        self.out.calls.insert(e, target);
        let func = pick.func;
        self.args(e, range, func, params.get(1..).unwrap_or(&[]), args);
        self.settle_assoc_eqs();
        self.fallible_call(e, func, &targs, range);
        self.norm(&ret)
    }
}

impl Checker<'_> {
    /// For a receiver of type `r` (or a borrow of one) that implements `Deref`: that type, and
    /// what it dereferences to.
    fn deref_target(&mut self, r: &Ty) -> Option<(Ty, Ty)> {
        let base = self.norm(r.strip_ref());
        if !matches!(base, Ty::Adt(..)) {
            return None;
        }
        let (trait_ref, _) = self.op_trait("Deref", "deref")?;
        if self.solver.implements(&base, &trait_ref) == Answer::No {
            return None;
        }
        let def = &self.items.traits[trait_ref.trait_ as usize];
        let i = def.assoc.iter().position(|(n, _)| n == "Output")?;
        let out = Ty::Assoc(std::sync::Arc::new(crate::ty::AssocTy {
            self_ty: base.clone(),
            trait_ref,
            index: i as u32,
            name: "Output".into(),
        }));
        let target = self.norm(&out);
        Some((base, target))
    }

    /// The arguments of a call of `func` that names some, or leaves some out for their
    /// defaults (`expr.call.named.1`, `expr.call.default.1`): positional ones first, then named
    /// ones in any order, each matched with its parameter; `offset` parameters (a receiver) are
    /// passed otherwise.
    fn arranged_args(
        &mut self,
        e: ExprId,
        range: TextRange,
        func: u32,
        params: &[Ty],
        args: &[ExprId],
        offset: usize,
    ) {
        let sig = &self.items.fns[func as usize];
        let names: Vec<Option<String>> = sig.params[offset..]
            .iter()
            .map(|p| p.name.clone())
            .collect();
        let fname = sig.name.clone();
        let mut slots: Vec<Option<usize>> = vec![None; params.len()];
        let mut seen_named = false;
        let mut next = 0;
        let mut extra = Vec::new();
        let mut misplaced = false;
        for (i, &a) in args.iter().enumerate() {
            match self.body.labels.get(&a).cloned() {
                None => {
                    if seen_named {
                        misplaced = true;
                        self.report(
                            Code::E0215,
                            self.range(a),
                            "positional arguments come before named ones",
                        );
                        extra.push(a);
                        continue;
                    }
                    if next < slots.len() {
                        slots[next] = Some(i);
                        next += 1;
                    } else {
                        extra.push(a);
                    }
                }
                Some((name, at)) => {
                    seen_named = true;
                    match names
                        .iter()
                        .position(|n| n.as_deref() == Some(name.as_str()))
                    {
                        Some(k) if slots[k].is_none() => slots[k] = Some(i),
                        Some(_) => {
                            self.report(
                                Code::E0215,
                                at,
                                format!("`{name}` is given more than once"),
                            );
                            extra.push(a);
                        }
                        None => {
                            self.report(
                                Code::E0215,
                                at,
                                format!("`{fname}` has no parameter called `{name}`"),
                            );
                            extra.push(a);
                        }
                    }
                }
            }
        }
        if !misplaced
            && !extra.is_empty()
            && extra.iter().all(|a| !self.body.labels.contains_key(a))
        {
            self.report(
                Code::E0202,
                range,
                format!(
                    "`{fname}` takes at most {} argument{}, but {} were given",
                    params.len(),
                    if params.len() == 1 { "" } else { "s" },
                    args.len()
                ),
            );
        }
        let mut order = Vec::new();
        let mut missing = Vec::new();
        for (k, slot) in slots.iter().enumerate() {
            match slot {
                Some(i) => {
                    self.check(args[*i], &params[k]);
                    order.push(super::ArgSlot::Arg(*i));
                }
                None => match self.items.default_of(func, k + offset) {
                    Some(d) => order.push(super::ArgSlot::Default(d)),
                    None => {
                        missing.push(names[k].clone().unwrap_or_else(|| format!("#{k}")));
                        order.push(super::ArgSlot::Arg(usize::MAX));
                    }
                },
            }
        }
        for a in extra {
            self.infer(a);
        }
        if !missing.is_empty() && !misplaced {
            let shown: Vec<String> = missing.iter().map(|m| format!("`{m}`")).collect();
            self.report(
                Code::E0202,
                range,
                format!("this call of `{fname}` does not pass {}", shown.join(", ")),
            );
        }
        self.out.arg_order.insert(e, order);
    }
}
