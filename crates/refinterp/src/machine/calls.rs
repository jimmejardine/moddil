//! Calls: functions, methods and trait methods, the kinds of their type parameters, and
//! the built-in methods of derived traits.

use std::collections::HashMap;

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{SyntaxKind as K, SyntaxNode, TextRange};

use crate::program::{Def, FnDef, Method, SelfKind, first_name};
use crate::value::{Head, Pointer, Value};
use crate::{Eval, Flow, MAX_DEPTH, TrapClass, trap};

use super::patterns::Src;
use super::{Machine, segments};

impl Machine<'_> {
    /// The kind a type written in this function stands for.
    pub(super) fn head_of(&self, t: &ast::Type) -> Head {
        let name = first_name(t.syntax());
        if let Some(h) = self.memory.type_of(&name)
            && matches!(t, ast::Type::Path(_))
        {
            return h.clone();
        }
        self.program.head_in(self.memory.module(), t)
    }

    /// What `path` names in the running code's module.
    pub(super) fn resolve(&self, path: &[String]) -> Option<(Def, usize)> {
        self.program.resolve(self.memory.module(), path)
    }

    /// The kinds the type parameters `names` get from the types written in `args`.
    pub(super) fn turbofish(
        &self,
        args: Option<SyntaxNode>,
        names: &[String],
    ) -> HashMap<String, Head> {
        let mut out = HashMap::new();
        if let Some(a) = args {
            let types = a
                .children()
                .filter(|c| c.kind() == K::TYPE_ARG)
                .filter_map(|c| c.children().find_map(ast::Type::cast));
            for (n, t) in names.iter().zip(types) {
                out.insert(n.clone(), self.head_of(&t));
                // A refinement named for a parameter is remembered beside its base's kind, for
                // `T::refine` (`refine.trait.1`).
                if let Some(k) = self.refinement_named(&t) {
                    out.insert(refined_key(n), Head::Adt(k));
                }
            }
        }
        out
    }

    /// The refinement type `t` names, by key: written out, or a type parameter bound to one.
    fn refinement_named(&self, t: &ast::Type) -> Option<String> {
        let path = crate::program::type_path(t.syntax());
        if let [p] = path.as_slice()
            && let Some(Head::Adt(k)) = self.memory.type_of(&refined_key(p))
        {
            return Some(k);
        }
        match self.resolve(&path) {
            Some((Def::Item(k), n))
                if n == path.len()
                    && self.program.types.get(&k).is_some_and(|t| t.pred.is_some()) =>
            {
                Some(k)
            }
            _ => None,
        }
    }

    /// Finds the kinds of type parameters `names` from a value of type `t` (`v`), as far as
    /// the value shows them.
    pub(super) fn infer_types(
        &self,
        t: &ast::Type,
        v: &Value,
        names: &[String],
        out: &mut HashMap<String, Head>,
    ) {
        match (t, v) {
            (_, Value::Moved) => {}
            (ast::Type::Ref(_), Value::Ref(p, _)) => {
                if let Some(inner) = t.syntax().children().find_map(ast::Type::cast) {
                    let target = self.memory.load(p);
                    self.infer_types(&inner, &target, names, out);
                }
            }
            (ast::Type::Paren(_), v) => {
                if let Some(inner) = t.syntax().children().find_map(ast::Type::cast) {
                    self.infer_types(&inner, v, names, out);
                }
            }
            (ast::Type::Tuple(_), Value::Tuple(xs)) => {
                for (inner, x) in t.syntax().children().filter_map(ast::Type::cast).zip(xs) {
                    self.infer_types(&inner, x, names, out);
                }
            }
            // `[T; N]`: `N` is the array's length, and `T` its elements' kind.
            (ast::Type::Array(_) | ast::Type::Slice(_), Value::Array(xs) | Value::Buf(_, xs)) => {
                if let Some(inner) = t.syntax().children().find_map(ast::Type::cast)
                    && let Some(x) = xs.first()
                {
                    self.infer_types(&inner, x, names, out);
                }
                if let Some(len) = t.syntax().children().find_map(ast::Expr::cast) {
                    let name = len.syntax().text().to_string();
                    if names.contains(&name) {
                        out.entry(name).or_insert(Head::Len(xs.len() as i64));
                    }
                }
            }
            (ast::Type::Path(_), v) => {
                let name = first_name(t.syntax());
                if names.contains(&name) {
                    out.entry(name).or_insert_with(|| v.head());
                    return;
                }
                // A generic struct or enum: what its fields' values say about its arguments.
                let Value::Adt {
                    ty,
                    variant,
                    fields,
                } = v
                else {
                    return;
                };
                let Some(adt) = self.program.adts.get(ty) else {
                    return;
                };
                let Some(args) = t
                    .syntax()
                    .descendants()
                    .into_iter()
                    .find(|n| n.kind() == K::GENERIC_ARGS)
                else {
                    return;
                };
                let mut inner = HashMap::new();
                let info = self.program.variant(ty, variant);
                for (ft, fv) in info.types.iter().zip(fields) {
                    if let Some(ft) = ft {
                        self.infer_types(ft, fv, &adt.params, &mut inner);
                    }
                }
                let written = args
                    .children()
                    .filter(|c| c.kind() == K::TYPE_ARG)
                    .filter_map(|c| c.children().find_map(ast::Type::cast));
                for (p, w) in adt.params.iter().zip(written) {
                    let wn = first_name(w.syntax());
                    if names.contains(&wn)
                        && let Some(h) = inner.get(p)
                    {
                        out.entry(wn).or_insert_with(|| h.clone());
                    }
                }
            }
            _ => {}
        }
    }

    /// Calls method `m` (found for `self_head`) with `args`, the first being `self` if it has one.
    pub(super) fn run_method(
        &mut self,
        m: Method,
        self_head: &Head,
        args: Vec<Value>,
        range: TextRange,
    ) -> Eval {
        match m {
            Method::Copy => match &args[0] {
                Value::Ref(p, _) => Ok(self.memory.load(p)),
                v => Ok(v.clone()),
            },
            Method::Derived(tr) if tr == "Default" => self.derived_default(self_head, range),
            Method::Derived(tr) => self.derived(&tr, &args, range),
            Method::Code(f, imp) => {
                let mut types = HashMap::new();
                types.insert("Self".to_string(), self_head.clone());
                if let Some(k) = imp {
                    let i = &self.program.impls[k];
                    if let Some(first) = args.first() {
                        let self_value = self.target(first.clone());
                        self.infer_types(&i.self_ty, &self_value, &i.params, &mut types);
                    }
                }
                self.invoke(&f, args, types, range)
            }
        }
    }

    /// Calls trait `tr`'s method `name` for the kind of value its first argument (or, through a
    /// borrow, what it points to) is.
    pub(super) fn call_trait(
        &mut self,
        tr: &str,
        name: &str,
        args: Vec<Value>,
        range: TextRange,
    ) -> Eval {
        let head = self.target(args[0].clone()).head();
        // A borrow compares as what it points to (`trait.borrow.eq.1`): each argument, a borrow
        // of a borrow, read once.
        if head == Head::Ref && matches!(tr, "PartialEq" | "Eq" | "PartialOrd" | "Ord") {
            let args = args
                .into_iter()
                .map(|a| match a {
                    Value::Ref(p, _) => self.memory.load(&p),
                    v => v,
                })
                .collect();
            return self.call_trait(tr, name, args, range);
        }
        let m = self
            .program
            .trait_method(tr, &head, name)
            .expect("a checked trait call");
        self.run_method(m, &head, args, range)
    }

    /// Whether refinement `key`'s predicate holds for `v`: the predicate, run in a call of its
    /// own with `self` bound to `v`.
    pub(super) fn holds(&mut self, key: &str, v: &Value, range: TextRange) -> Eval<bool> {
        let t = self.program.types[key].clone();
        let (pred, _) = t.pred.expect("a refinement");
        let file = self.memory.file();
        self.memory.push_frame(HashMap::new(), file, t.module, true);
        self.memory.push("self".to_string(), v.clone());
        let r = self.eval(&pred);
        self.memory.pop_frame();
        match r {
            Ok(Value::Bool(b)) => Ok(b),
            Ok(_) => unreachable!("a checked predicate"),
            // A trap in the predicate is reported where it was asked.
            Err(Flow::Trap(t, _)) => Err(trap(t.class, range)),
            Err(f) => Err(f),
        }
    }

    /// `X::new(v)`: `v`, if the predicate holds for it, or else the error
    /// `RefinementError { ty, clause, value }`.
    pub(super) fn refine(&mut self, key: &str, v: Value, range: TextRange) -> Eval {
        if self.holds(key, &v, range)? {
            return Ok(v);
        }
        let t = self.program.types[key].clone();
        let clause = t.pred.expect("a refinement").1;
        let empty = self.string_call("new", Vec::new(), range)?;
        let out = self.memory.temp(empty);
        let at = self.memory.temp(v);
        self.call_trait(
            "Debug",
            "fmt",
            vec![Value::Ref(at, false), Value::Ref(out.clone(), true)],
            range,
        )?;
        let shown = self.memory.load(&out);
        let error = Value::Adt {
            ty: "RefinementError".to_string(),
            variant: "RefinementError".to_string(),
            fields: vec![
                crate::value::make_string(&t.name),
                crate::value::make_string(&clause),
                shown,
            ],
        };
        Err(Flow::Fail(error, range))
    }

    /// The derived `Default` (`attr.derive.default.1`): a struct of each field's default, by
    /// the field's type as written, or an enum's `#[default]` variant.
    pub(super) fn derived_default(&mut self, head: &Head, range: TextRange) -> Eval {
        let Head::Adt(ty) = head else {
            unreachable!("a struct or enum")
        };
        let info = self.program.adts[ty].clone();
        if info.is_enum {
            let v = info
                .variants
                .iter()
                .find(|v| v.default)
                .expect("a checked derive");
            return Ok(Value::Adt {
                ty: ty.clone(),
                variant: v.name.clone(),
                fields: Vec::new(),
            });
        }
        let v = &info.variants[0];
        let mut fields = Vec::new();
        for t in &v.types {
            let t = t.as_ref().expect("a field's type");
            fields.push(self.default_of(&self.program.head_in(info.module, t), range)?);
        }
        Ok(Value::Adt {
            ty: ty.clone(),
            variant: v.name.clone(),
            fields,
        })
    }

    /// The default value of the kind of value `head`.
    pub(super) fn default_of(&mut self, head: &Head, range: TextRange) -> Eval {
        let m = self
            .program
            .trait_method("Default", head, "default")
            .expect("a checked `Default`");
        self.run_method(m, head, Vec::new(), range)
    }

    /// The derived methods, built in and structural (`attr.derive.1`): `self` and `other` are
    /// borrows.
    pub(super) fn derived(&mut self, tr: &str, args: &[Value], range: TextRange) -> Eval {
        let ptr = |v: &Value| match v {
            Value::Ref(p, _) => p.clone(),
            _ => unreachable!("a borrow"),
        };
        let a = ptr(&args[0]);
        let av = self.memory.load(&a);
        let Value::Adt {
            ty,
            variant,
            fields,
        } = &av
        else {
            unreachable!("a struct or enum")
        };
        let field = |p: &Pointer, i: usize| {
            let mut q = p.clone();
            q.path.push(i);
            q
        };
        match tr {
            "Clone" => {
                let mut out = Vec::new();
                for (i, f) in fields.iter().enumerate() {
                    // A borrow (a view's field) is copied: borrows are `Copy`.
                    if matches!(f, Value::Ref(..) | Value::Str(_)) {
                        out.push(f.clone());
                        continue;
                    }
                    let x = vec![Value::Ref(field(&a, i), false)];
                    out.push(self.call_trait("Clone", "clone", x, range)?);
                }
                Ok(Value::Adt {
                    ty: ty.clone(),
                    variant: variant.clone(),
                    fields: out,
                })
            }
            // As Moddil source writes the value (`attr.derive.debug.1`).
            "Debug" => {
                let Value::Ref(out, _) = args[1].clone() else {
                    unreachable!("a `&mut dyn TextWrite`")
                };
                let info = &self.program.adts[ty];
                let path = if info.is_enum {
                    format!("{}::{variant}", info.name)
                } else {
                    info.name.clone()
                };
                let v = self.program.variant(ty, variant).clone();
                match v.shape {
                    crate::program::Shape::Unit => self.push_text(&out, &path)?,
                    crate::program::Shape::Named => {
                        self.push_text(&out, &format!("{path} {{ "))?;
                        for (i, f) in v.fields.iter().enumerate() {
                            let sep = if i == 0 { "" } else { ", " };
                            self.push_text(&out, &format!("{sep}{f}: "))?;
                            let x = vec![Value::Ref(field(&a, i), false), args[1].clone()];
                            self.call_trait("Debug", "fmt", x, range)?;
                        }
                        self.push_text(&out, " }")?;
                    }
                    crate::program::Shape::Tuple => {
                        self.push_text(&out, &format!("{path}("))?;
                        for i in 0..fields.len() {
                            if i > 0 {
                                self.push_text(&out, ", ")?;
                            }
                            let x = vec![Value::Ref(field(&a, i), false), args[1].clone()];
                            self.call_trait("Debug", "fmt", x, range)?;
                        }
                        self.push_text(&out, ")")?;
                    }
                }
                Ok(Value::Unit)
            }
            // An enum's variant's index, then each field in order (`coll.hash.derive.1`).
            "Hash" => {
                let state = args[1].clone();
                let info = &self.program.adts[ty];
                if info.is_enum {
                    let k = info
                        .variants
                        .iter()
                        .position(|v| v.name == *variant)
                        .expect("a variant");
                    let x = vec![state.clone(), Value::Int(k as i64)];
                    self.call_trait("Hasher", "write_int", x, range)?;
                }
                for i in 0..fields.len() {
                    let x = vec![Value::Ref(field(&a, i), false), state.clone()];
                    self.call_trait("Hash", "hash", x, range)?;
                }
                Ok(Value::Unit)
            }
            "PartialEq" => {
                let b = ptr(&args[1]);
                let Value::Adt { variant: bv, .. } = self.memory.load(&b) else {
                    unreachable!("a struct or enum")
                };
                if *variant != bv {
                    return Ok(Value::Bool(false));
                }
                for i in 0..fields.len() {
                    let x = vec![
                        Value::Ref(field(&a, i), false),
                        Value::Ref(field(&b, i), false),
                    ];
                    if self.call_trait("PartialEq", "eq", x, range)? != Value::Bool(true) {
                        return Ok(Value::Bool(false));
                    }
                }
                Ok(Value::Bool(true))
            }
            _ => {
                // `PartialOrd` and `Ord`: by variant, in declaration order, then field by field.
                let partial = tr == "PartialOrd";
                let b = ptr(&args[1]);
                let Value::Adt { variant: bv, .. } = self.memory.load(&b) else {
                    unreachable!("a struct or enum")
                };
                let order = |n: &str| {
                    self.program.adts[ty]
                        .variants
                        .iter()
                        .position(|v| v.name == n)
                        .expect("a variant")
                };
                let wrap = |o: &str| {
                    let ordering = Value::Adt {
                        ty: "Ordering".to_string(),
                        variant: o.to_string(),
                        fields: Vec::new(),
                    };
                    if partial {
                        Value::Adt {
                            ty: "Option".to_string(),
                            variant: "Some".to_string(),
                            fields: vec![ordering],
                        }
                    } else {
                        ordering
                    }
                };
                let (i, j) = (order(variant), order(&bv));
                if i != j {
                    return Ok(wrap(if i < j { "Less" } else { "Greater" }));
                }
                let method = if partial { "partial_cmp" } else { "cmp" };
                for k in 0..fields.len() {
                    let x = vec![
                        Value::Ref(field(&a, k), false),
                        Value::Ref(field(&b, k), false),
                    ];
                    let c = self.call_trait(tr, method, x, range)?;
                    if c != wrap("Equal") {
                        return Ok(c);
                    }
                }
                Ok(wrap("Equal"))
            }
        }
    }

    /// Runs function `f` with `args` and the kinds of its parent's type parameters `types`
    /// (`expr.call.1`). Its own type parameters' kinds come from the arguments.
    pub(super) fn invoke(
        &mut self,
        f: &FnDef,
        args: Vec<Value>,
        types: HashMap<String, Head>,
        range: TextRange,
    ) -> Eval {
        let result = self.invoke_inner(f, args, types, range);
        // Whether a `?` on this call takes its errors, rather than an `Option` it gives: its
        // signature says it can fail (`err.question.1`).
        self.failing_call = f.node.syntax().child(K::EFFECT_CLAUSE).is_some_and(|c| {
            c.descendants()
                .iter()
                .any(|d| d.token(K::FAIL_KW).is_some())
        });
        result
    }

    fn invoke_inner(
        &mut self,
        f: &FnDef,
        args: Vec<Value>,
        mut types: HashMap<String, Head>,
        range: TextRange,
    ) -> Eval {
        if self.memory.depth() >= MAX_DEPTH {
            return Err(trap(TrapClass::StackOverflow, range));
        }
        // A stub mod's function without a body (`mod.stub.1`).
        if f.node.body().is_none() && f.user {
            return Err(trap(TrapClass::Unimplemented, range));
        }
        // The prelude's built-in methods have no body (`#[intrinsic]`).
        if f.node.body().is_none() {
            let head = types.get("Self").cloned().unwrap_or(Head::Unit);
            // A conversion's target (`num.convert.1`).
            self.convert_to = types.get("T").cloned();
            return self.intrinsic(f, &head, &args, range);
        }
        let params: Vec<ast::Param> = f
            .node
            .params()
            .map(|l| l.params().collect())
            .unwrap_or_default();
        let skip = usize::from(f.self_kind.is_some());
        for (p, v) in params.iter().zip(&args[skip..]) {
            if let Some(t) = p.ty() {
                self.infer_types(&t, v, &f.params, &mut types);
            }
        }
        // A template's method: its parameter names the struct (`comptime.template.1`).
        if let Some((var, head)) = &f.template {
            types.insert(var.clone(), head.clone());
        }
        let caller_user = self.memory.user();
        let caller_tracked = self.memory.tracked();
        let caller_file = self.memory.file().unwrap_or(0);
        // The callee sees only its parameters, in a frame of its own.
        let file = f.user.then_some(f.file);
        self.memory
            .push_frame(types, file, f.module as usize, f.tracked);
        let mut args = args.into_iter();
        if f.self_kind.is_some() {
            let v = args.next().expect("`self`");
            self.memory.push("self".to_string(), v);
        }
        for (p, v) in params.iter().zip(args) {
            let pat = p.pat().expect("a parameter pattern");
            let v = self.deref_coerced(v, p.ty().as_ref(), range)?;
            let v = self.typed(v, p.ty().as_ref());
            let slot = self.memory.temp(v);
            let mut binds = Vec::new();
            let matched = self.matches(&pat, &Src::Place(slot, false), &mut binds);
            debug_assert!(matched, "parameter patterns are irrefutable");
            self.bind(binds);
        }
        self.bind_effects(&f.node);
        // Contracts (`contract.1`): the preconditions hold, or the caller is at fault; then the
        // values `old(e)` keeps.
        let contracts: Vec<(bool, ast::Expr)> = f
            .node
            .syntax()
            .children()
            .filter(|c| c.kind() == K::CONTRACT)
            .filter_map(|c| {
                let requires = c.token(K::REQUIRES_KW).is_some();
                c.children()
                    .find_map(ast::Expr::cast)
                    .map(|e| (requires, e))
            })
            .collect();
        for (_, e) in contracts.iter().filter(|(r, _)| *r) {
            match self.eval(e) {
                Ok(Value::Bool(true)) => {}
                Ok(_) => {
                    self.memory.pop_frame();
                    let t = crate::Trap {
                        class: TrapClass::Precondition,
                        file: caller_file,
                        range,
                    };
                    return Err(Flow::Trap(t, caller_user && !caller_tracked));
                }
                Err(flow) => {
                    self.memory.pop_frame();
                    return Err(flow);
                }
            }
        }
        let mut olds = HashMap::new();
        for (_, e) in contracts.iter().filter(|(r, _)| !*r) {
            for call in e.syntax().descendants() {
                if call.kind() != K::CALL_EXPR {
                    continue;
                }
                let Some(callee) = call.children().find_map(ast::Expr::cast) else {
                    continue;
                };
                if segments(callee.syntax()) != ["old"] {
                    continue;
                }
                let Some(arg) = call
                    .child(K::ARG_LIST)
                    .and_then(ast::ArgList::cast)
                    .and_then(|l| l.args().next())
                    .and_then(|a| a.expr())
                else {
                    continue;
                };
                match self.eval(&arg) {
                    Ok(v) => {
                        olds.insert(call.text_range(), v);
                    }
                    Err(flow) => {
                        self.memory.pop_frame();
                        return Err(flow);
                    }
                }
            }
        }
        self.olds.push(olds);
        let result = self.wrapped_body(f, 0, range);
        let result = match result {
            Ok(v) | Err(Flow::Return(v)) => Ok(v),
            // `?` on `None` in a function returning an `Option` returns `None`.
            Err(Flow::Absent) => Ok(Value::Adt {
                ty: "Option".to_string(),
                variant: "None".to_string(),
                fields: Vec::new(),
            }),
            Err(f) => Err(f),
        };
        // The result, where the function's type is a borrow of what it dereferences to
        // (`type.deref.coerce.1`).
        let result = match result {
            Ok(v) => {
                let ret = f.node.ret_type().and_then(|r| r.ty());
                self.deref_coerced(v, ret.as_ref(), range)
            }
            r => r,
        };
        // The postconditions, with `result` the value returned.
        let result = match result {
            Ok(v) if contracts.iter().any(|(r, _)| !*r) => {
                let mark = self.memory.mark();
                self.memory.push("result".to_string(), v);
                let mut broken = None;
                for (_, e) in contracts.iter().filter(|(r, _)| !*r) {
                    match self.eval(e) {
                        Ok(Value::Bool(true)) => {}
                        Ok(_) => {
                            broken = Some(trap(TrapClass::Postcondition, e.syntax().text_range()));
                            break;
                        }
                        Err(flow) => {
                            broken = Some(flow);
                            break;
                        }
                    }
                }
                let slot = self.memory.slot("result").expect("`result`");
                let v = self.memory.load(&self.memory.here(slot));
                self.memory.truncate(mark);
                match broken {
                    Some(flow) => Err(flow),
                    None => Ok(v),
                }
            }
            r => r,
        };
        self.olds.pop();
        // The parameters are dropped at the end of the call; a destructor's `self` only by its
        // fields.
        let result = if matches!(result, Err(Flow::Trap(..))) {
            result
        } else {
            let mut result = result;
            if let Ok(v) = &mut result {
                self.keep_captures(v, 0);
            }
            while let Some((name, v)) = self.memory.pop() {
                let own = !(f.destructor && name == "self");
                if let Err(e) = self.drop_value(v, own, range) {
                    result = Err(e);
                    break;
                }
            }
            result
        };
        self.memory.pop_frame();
        // A trap in the prelude's code is reported where the file's code called it.
        match result {
            Err(Flow::Trap(mut t, false)) if f.user && !f.tracked => {
                t.file = f.file;
                Err(Flow::Trap(t, true))
            }
            // A trap in the prelude's code or a `#[track_caller]` function is reported where it
            // was called from, and on up while that is in one too (`attr.track-caller.1`).
            Err(Flow::Trap(mut t, false)) if caller_user => {
                t.range = range;
                t.file = caller_file;
                Err(Flow::Trap(t, !caller_tracked))
            }
            r => r,
        }
    }

    /// The declaration, in its trait, of the method `name` that impl `k` defines, if `k` is an
    /// impl of a trait: where the method's defaults are written.
    fn trait_method(&self, k: usize, name: &str) -> Option<FnDef> {
        let t = self.program.impls[k].trait_.as_ref()?;
        self.program.traits.get(t)?.methods.get(name).cloned()
    }

    /// The arguments of a call of `f` at `node`, in its parameters' order (`expr.call.named.1`):
    /// `first` (a receiver), then the `written` ones, positional ones in order and named ones by
    /// their names, and for each one left out its default (`expr.call.default.1`), its own or
    /// `inherited`'s (the trait's method's), evaluated in its module.
    fn arranged(
        &mut self,
        f: &FnDef,
        inherited: Option<&FnDef>,
        node: &SyntaxNode,
        first: Vec<Value>,
        written: Vec<Value>,
    ) -> Eval<Vec<Value>> {
        let params: Vec<ast::Param> = f
            .node
            .params()
            .map(|l| l.params().collect())
            .unwrap_or_default();
        let labels: Vec<Option<String>> = node
            .child(K::ARG_LIST)
            .and_then(ast::ArgList::cast)
            .map(|l| {
                l.args()
                    .filter(|a| a.expr().is_some())
                    .map(|a| a.label().map(|t| t.text().to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let mut out = first;
        // `Type::method(x, …)` passes `self` as its first argument.
        let mut written = written;
        let mut labels = labels;
        if f.self_kind.is_some() && out.is_empty() && !written.is_empty() {
            out.push(written.remove(0));
            if !labels.is_empty() {
                labels.remove(0);
            }
        }
        if labels.iter().all(Option::is_none) && written.len() == params.len() {
            out.extend(written);
            return Ok(out);
        }
        let name = |p: &ast::Param| {
            p.pat().map(|q| {
                q.syntax()
                    .text()
                    .to_string()
                    .trim_start_matches("mut ")
                    .trim()
                    .to_string()
            })
        };
        let mut slots: Vec<Option<Value>> = vec![None; params.len()];
        let mut next = 0;
        for (v, label) in written.into_iter().zip(labels) {
            let k = match label {
                None => {
                    next += 1;
                    next - 1
                }
                Some(l) => params
                    .iter()
                    .position(|p| name(p).as_deref() == Some(l.as_str()))
                    .expect("a checked named argument"),
            };
            slots[k] = Some(v);
        }
        for (k, slot) in slots.into_iter().enumerate() {
            let v = match slot {
                Some(v) => v,
                None => {
                    let own = params[k].syntax().children().find_map(ast::Expr::cast);
                    let (e, from) = match own {
                        Some(e) => (e, f),
                        None => {
                            let g = inherited.expect("a checked default");
                            let p = g
                                .node
                                .params()
                                .and_then(|l| l.params().nth(k))
                                .expect("the trait's parameter");
                            let e = p
                                .syntax()
                                .children()
                                .find_map(ast::Expr::cast)
                                .expect("a checked default");
                            (e, g)
                        }
                    };
                    let file = from.user.then_some(from.file);
                    self.memory
                        .push_frame(HashMap::new(), file, from.module as usize, false);
                    let v = self.eval(&e);
                    self.memory.pop_frame();
                    v?
                }
            };
            out.push(v);
        }
        Ok(out)
    }

    /// A call of a function, a tuple struct or variant, a function of a type (`Type::f`,
    /// `T::f`), or a trait's method (`Trait::f`) (`expr.call.1`, `expr.struct.1`,
    /// `expr.path.assoc.1`).
    pub(super) fn call(&mut self, node: &SyntaxNode, e: &ast::Expr, range: TextRange) -> Eval {
        let callee = &e.sub_exprs()[0];
        let args: Vec<ast::Expr> = node
            .child(K::ARG_LIST)
            .and_then(ast::ArgList::cast)
            .map(|l| l.args().filter_map(|a| a.expr()).collect())
            .unwrap_or_default();
        let segments = segments(callee.syntax());
        let local = matches!(segments.as_slice(), [one] if self.memory.slot(one).is_some());
        // `old(e)` in a postcondition: `e` as it was on entry (`contract.old.1`).
        if segments == ["old"]
            && !local
            && let Some(v) = self.olds.last().and_then(|o| o.get(&node.text_range()))
        {
            return Ok(v.clone());
        }
        // A closure (`expr.closure.call.1`), or one kept in a box (`expr.closure.stored.1`).
        if local || !matches!(callee, ast::Expr::Path(_)) {
            let p = self.place(callee)?;
            if let Some(c) = self.closure_at(p, range)? {
                let values = self.eval_all(&args)?;
                return self.call_closure(&c, values, range);
            }
        }
        let item = |one: &str| self.program.lookup(self.memory.module(), one).is_some();
        // `discard` is in the prelude too (`err.discard.1`).
        if !local && matches!(segments.as_slice(), [one] if one == "discard" && !item(one)) {
            return self.discard(&args, range);
        }
        // `assert` is in the prelude, so a local or item of that name comes first
        // (`expr.assert.1`).
        if !local && matches!(segments.as_slice(), [one] if one == "assert" && !item(one)) {
            return match self.eval(&args[0])? {
                Value::Bool(true) => Ok(Value::Unit),
                _ => Err(trap(TrapClass::Assertion, range)),
            };
        }
        // So are `unreachable` and `todo` (`expr.unreachable.1`, `expr.todo.1`).
        if !local && matches!(segments.as_slice(), [one] if one == "unreachable" && !item(one)) {
            return Err(trap(TrapClass::Unreachable, range));
        }
        if !local && matches!(segments.as_slice(), [one] if one == "todo" && !item(one)) {
            return Err(trap(TrapClass::Unimplemented, range));
        }
        let resolved = self.resolve(&segments);
        let is_fn = !local
            && matches!(&resolved, Some((Def::Item(k), n)) if *n == segments.len() && self.program.fns.contains_key(k));
        if !local
            && !is_fn
            && let Some((ty, info)) = self.ctor(&segments)
        {
            let variant = info.name.clone();
            let types = info.types.clone();
            let fields: Vec<Value> = self
                .eval_all(&args)?
                .into_iter()
                .enumerate()
                .map(|(i, v)| self.typed(v, types.get(i).and_then(Option::as_ref)))
                .collect();
            return Ok(Value::Adt {
                ty,
                variant,
                fields,
            });
        }
        let turbofish = callee
            .syntax()
            .child(K::PATH)
            .and_then(|p| p.children().find(|c| c.kind() == K::GENERIC_ARGS));
        if is_fn && let Some((Def::Item(k), _)) = &resolved {
            let f = self.program.fns[k].clone();
            let types = self.turbofish(turbofish, &f.params);
            let values = self.eval_all(&args)?;
            let values = self.arranged(&f, None, node, Vec::new(), values)?;
            return self.invoke(&f, values, types, range);
        }
        let (ty, name) = segments.split_at(segments.len() - 1);
        let name = &name[0];
        match ty {
            [] => unreachable!("a checked call"),
            ty => {
                let values = self.eval_all(&args)?;
                // `X::new(v)` of a refinement (`refine.new.1`).
                if let Some((Def::Item(k), n)) = self.resolve(ty)
                    && n == ty.len()
                    && name == "new"
                    && self.program.types.get(&k).is_some_and(|t| t.pred.is_some())
                {
                    return self.refine(&k, values.into_iter().next().expect("a value"), range);
                }
                // `Trait::method(x, …)`: for the kind of value `x` is.
                if let Some((Def::Item(k), n)) = self.resolve(ty)
                    && n == ty.len()
                    && self.program.traits.contains_key(&k)
                {
                    // With no value to choose the impl by (`Default::default()`): by the type
                    // its context names.
                    if values.is_empty() {
                        let head = self.context_head(node).expect("a checked call");
                        let m = self
                            .program
                            .trait_method(&k, &head, name)
                            .expect("a checked call");
                        return self.run_method(m, &head, values, range);
                    }
                    return self.call_trait(&k, name, values, range);
                }
                // `T::refine(v)`, `T` a refinement (`refine.trait.1`).
                if name == "refine"
                    && let [t] = ty
                    && let Some(Head::Adt(k)) = self.memory.type_of(&refined_key(t))
                {
                    return self.refine(&k, values.into_iter().next().expect("a value"), range);
                }
                let head = match ty {
                    [t] if t == "int" || t == "i64" => Head::Int,
                    [t] if crate::num::Kind::named(t).is_some() => {
                        Head::Num(crate::num::Kind::named(t).expect("a number type"))
                    }
                    [t] if t == "bool" => Head::Bool,
                    [t] if t == "char" => Head::Char,
                    [t] if t == "str" => Head::Str,
                    [t] if t == "Buf" => Head::Buf,
                    [t] if self.memory.type_of(t).is_some() => {
                        self.memory.type_of(t).expect("known").clone()
                    }
                    path => match self.resolve(path) {
                        Some((Def::Item(k), _)) => Head::Adt(k),
                        _ => Head::Adt(path.join("::")),
                    },
                };
                let m = self
                    .program
                    .type_fns(&head, name)
                    .into_iter()
                    .next()
                    .expect("a checked function of a type");
                if let Method::Code(f, imp) = m {
                    let mut types = HashMap::new();
                    types.insert("Self".to_string(), head.clone());
                    if let Some(k) = imp
                        && f.self_kind.is_some()
                        && let Some(first) = values.first()
                    {
                        let i = &self.program.impls[k];
                        let v = self.target(first.clone());
                        self.infer_types(&i.self_ty, &v, &i.params, &mut types);
                    }
                    let own = callee
                        .syntax()
                        .child(K::PATH)
                        .and_then(|p| p.children().filter(|c| c.kind() == K::GENERIC_ARGS).last());
                    types.extend(self.turbofish(own, &f.params));
                    let inherited = imp.and_then(|k| self.trait_method(k, name));
                    let values = self.arranged(&f, inherited.as_ref(), node, Vec::new(), values)?;
                    return self.invoke(&f, values, types, range);
                }
                self.run_method(m, &head, values, range)
            }
        }
    }

    /// Whether `segments` name a function, or a trait's method (`Trait::method`).
    pub(super) fn fn_path(&self, segments: &[String]) -> bool {
        // A tuple struct or variant (`Some`), as a value.
        if let Some((_, info)) = self.ctor(segments) {
            return info.shape == crate::program::Shape::Tuple;
        }
        match self.resolve(segments) {
            Some((Def::Item(k), n)) if n == segments.len() => self.program.fns.contains_key(&k),
            Some((Def::Item(k), n))
                if n + 1 == segments.len() && self.program.traits.contains_key(&k) =>
            {
                true
            }
            // A type's function (`Vec::new`).
            _ if segments.len() >= 2 => {
                let (ty, name) = segments.split_at(segments.len() - 1);
                let head = self.type_head(ty);
                self.program
                    .methods(&head, &name[0])
                    .iter()
                    .any(|m| matches!(m, Method::Code(..)))
            }
            _ => false,
        }
    }

    /// The kind of value the type path `ty` names (`int`, `Vec`, a type parameter).
    pub(super) fn type_head(&self, ty: &[String]) -> Head {
        match ty {
            [t] if t == "int" || t == "i64" => Head::Int,
            [t] if crate::num::Kind::named(t).is_some() => {
                Head::Num(crate::num::Kind::named(t).expect("a number type"))
            }
            [t] if t == "bool" => Head::Bool,
            [t] if t == "char" => Head::Char,
            [t] if t == "str" => Head::Str,
            [t] if t == "Buf" => Head::Buf,
            [t] if self.memory.type_of(t).is_some() => {
                self.memory.type_of(t).expect("known").clone()
            }
            path => match self.resolve(path) {
                Some((Def::Item(k), _)) => Head::Adt(k),
                _ => Head::Adt(path.join("::")),
            },
        }
    }

    /// A call of the function or trait method path `node` names, with `values`
    /// (`expr.fn-value.1`).
    pub(super) fn call_fn_path(
        &mut self,
        node: &SyntaxNode,
        values: Vec<Value>,
        range: TextRange,
    ) -> Eval {
        let segments = segments(node);
        // A tuple struct or variant: built from the values.
        if let Some((ty, info)) = self.ctor(&segments) {
            let variant = info.name.clone();
            let types = info.types.clone();
            let fields = values
                .into_iter()
                .enumerate()
                .map(|(i, v)| self.typed(v, types.get(i).and_then(Option::as_ref)))
                .collect();
            return Ok(Value::Adt {
                ty,
                variant,
                fields,
            });
        }
        match self.resolve(&segments) {
            Some((Def::Item(k), n)) if n == segments.len() => {
                let f = self.program.fns[&k].clone();
                let turbofish = node
                    .child(K::PATH)
                    .and_then(|p| p.children().find(|c| c.kind() == K::GENERIC_ARGS));
                let types = self.turbofish(turbofish, &f.params);
                self.invoke(&f, values, types, range)
            }
            Some((Def::Item(k), n))
                if n + 1 == segments.len() && self.program.traits.contains_key(&k) =>
            {
                let name = segments.last().expect("a method's name").clone();
                self.call_trait(&k, &name, values, range)
            }
            // A type's function.
            _ => {
                let (ty, name) = segments.split_at(segments.len() - 1);
                let head = self.type_head(ty);
                let m = self
                    .program
                    .type_fns(&head, &name[0])
                    .into_iter()
                    .find(|m| matches!(m, Method::Code(..)))
                    .expect("a checked function of a type");
                self.run_method(m, &head, values, range)
            }
        }
    }

    /// `receiver.name(args)`: the method is looked for on the receiver's value, then on a
    /// borrow of it, then on a `&mut` borrow of it, and then the same for what it points to if
    /// it is a borrow (`expr.method.1`).
    pub(super) fn method_call(
        &mut self,
        node: &SyntaxNode,
        e: &ast::Expr,
        range: TextRange,
    ) -> Eval {
        // A selectable function's receive, done already by a `select` (`task.select.4`).
        if self.received.as_ref().is_some_and(|(r, _)| *r == range) {
            return Ok(self.received.take().expect("received").1);
        }
        let receiver = &e.sub_exprs()[0];
        // A method's name may be a keyword (`s.spawn(…)`).
        let name = ast::member_name(node)
            .expect("a method name")
            .text()
            .to_string();
        let args: Vec<ast::Expr> = node
            .child(K::ARG_LIST)
            .and_then(ast::ArgList::cast)
            .map(|l| l.args().filter_map(|a| a.expr()).collect())
            .unwrap_or_default();
        // A reflected variant's `is(x)` and `make()` (`comptime.template.variant.1`).
        if let Some((key, i)) = self.reflected_variant(receiver.syntax()) {
            let info = &self.program.adts[&key].variants[i];
            let variant = info.name.clone();
            return match name.as_str() {
                // Its shape, in `if const` (`comptime.template.if.1`).
                "is_unit" => Ok(Value::Bool(info.fields.is_empty())),
                "is_positional" => Ok(Value::Bool(
                    !info.fields.is_empty() && info.shape != crate::program::Shape::Named,
                )),
                "make" => Ok(Value::Adt {
                    ty: key,
                    variant,
                    fields: Vec::new(),
                }),
                _ => {
                    let mut v = self.eval(&args[0])?;
                    while let Value::Ref(p, _) = v {
                        v = self.memory.load(&p);
                    }
                    Ok(Value::Bool(
                        matches!(&v, Value::Adt { variant: w, .. } if *w == variant),
                    ))
                }
            };
        }
        // `t.is_enum()`, `t.is_unit()` and `t.is_positional()` in a template
        // (`comptime.template.if.1`): what kind of type `t` is.
        if matches!(name.as_str(), "is_enum" | "is_unit" | "is_positional")
            && let Some(head) = self.template_type(receiver)
        {
            let adt = match &head {
                Head::Adt(k) => self.program.adts.get(k),
                _ => None,
            };
            let first = adt.filter(|a| !a.is_enum).map(|a| &a.variants[0]);
            let answer = match name.as_str() {
                "is_enum" => adt.is_some_and(|a| a.is_enum),
                "is_unit" => first.is_some_and(|v| v.fields.is_empty()),
                _ => first.is_some_and(|v| {
                    !v.fields.is_empty() && v.shape != crate::program::Shape::Named
                }),
            };
            return Ok(Value::Bool(answer));
        }
        // A reflected field's `is_refined()` and `new(e)` (`comptime.template.field.1`).
        if let Some((key, i)) = self.reflected(receiver.syntax()) {
            let refined = self.field_refinement(&key, i);
            return match name.as_str() {
                "is_refined" => Ok(Value::Bool(refined.is_some())),
                _ => {
                    let v = self.eval(&args[0])?;
                    let k = refined.expect("a checked refinement");
                    self.refine(&k, v, range)
                }
            };
        }
        let (place, indirect) = match self.place_of(receiver)? {
            Some(p) => p,
            None => {
                let v = self.eval(receiver)?;
                (self.memory.temp(v), false)
            }
        };
        let value = self.memory.load(&place);
        // An anonymous impl's method is its closure; one it leaves out, the trait's default.
        if let Value::Anon(tr, methods) = self.target(value.clone()) {
            let values = self.eval_all(&args)?;
            if let Some((_, c)) = methods.iter().find(|(n, _)| *n == name) {
                return self.call_closure(c, values, range);
            }
            let f = self.program.traits[&tr]
                .methods
                .get(&name)
                .cloned()
                .expect("a checked method");
            let recv = match value {
                Value::Ref(..) => value,
                _ => Value::Ref(place, false),
            };
            let mut all = vec![recv];
            all.extend(values);
            return self.invoke(&f, all, HashMap::new(), range);
        }
        // The steps: the receiver itself, then what it points to.
        // Through every borrow: `self` of a method of an impl for `&str` is a `&&str`.
        let mut steps = vec![(place.clone(), value.clone(), indirect)];
        let mut at = value.clone();
        while let Value::Ref(q, _) = &at {
            let next = self.memory.load(q);
            steps.push((q.clone(), next.clone(), true));
            at = next;
        }
        let mut found = None;
        let mut from = 0;
        loop {
            'search: for (p, v, ind) in &steps[from..] {
                let head = v.head();
                // By value: `self` of the receiver's kind, or `&self` / `&mut self` of what a borrow
                // points to.
                let mut by_value = Vec::new();
                for m in self.program.methods(&head, &name) {
                    if m.self_kind() == Some(SelfKind::Value) {
                        by_value.push((m, head.clone()));
                    }
                }
                if let Value::Ref(q, mutable) = v {
                    let inner = self.memory.load(q).head();
                    for m in self.program.methods(&inner, &name) {
                        match m.self_kind() {
                            Some(SelfKind::Ref) => by_value.push((m, inner.clone())),
                            Some(SelfKind::RefMut) if *mutable => by_value.push((m, inner.clone())),
                            _ => {}
                        }
                    }
                }
                if let Some((m, h)) = by_value.into_iter().next() {
                    let recv = if matches!(m.self_kind(), Some(SelfKind::Value)) {
                        self.take(p, *ind)
                    } else {
                        v.clone()
                    };
                    found = Some((m, h, recv));
                    break 'search;
                }
                for (kind, mutable) in [(SelfKind::Ref, false), (SelfKind::RefMut, true)] {
                    if let Some(m) = self
                        .program
                        .methods(&head, &name)
                        .into_iter()
                        .find(|m| m.self_kind() == Some(kind))
                    {
                        // A `&str` is its text already: it is passed as it is, not borrowed again.
                        let recv = match v {
                            Value::Str(_) if !mutable => v.clone(),
                            _ => Value::Ref(p.clone(), mutable),
                        };
                        found = Some((m, head.clone(), recv));
                        break 'search;
                    }
                }
            }
            // None: what the receiver dereferences to, through `Deref` (`expr.method.deref.1`).
            if found.is_some() {
                break;
            }
            from = steps.len();
            let (p, v, _) = steps.last().cloned().expect("a receiver");
            let Value::Adt { .. } = &v else { break };
            let Some(q) = self.deref_place(p, range)? else {
                break;
            };
            let mut at = self.memory.load(&q);
            steps.push((q, at.clone(), true));
            while let Value::Ref(q, _) = &at {
                let next = self.memory.load(q);
                steps.push((q.clone(), next.clone(), true));
                at = next;
            }
        }
        let (m, head, recv) = found.unwrap_or_else(|| {
            panic!(
                "a checked method call: `{name}` on {:?}",
                self.target(value.clone())
            )
        });
        // Each argument where its parameter's type is written (`num.lit.1`), when none is
        // named: positional arguments are the parameters in order.
        let labelled = node
            .child(K::ARG_LIST)
            .and_then(ast::ArgList::cast)
            .is_some_and(|l| l.args().any(|a| a.label().is_some()));
        let written = match &m {
            Method::Code(f, _) if !labelled => {
                let params: Vec<ast::Param> = f
                    .node
                    .params()
                    .map(|l| l.params().collect())
                    .unwrap_or_default();
                let mut out = Vec::new();
                for (i, a) in args.iter().enumerate() {
                    let ty = params.get(i).and_then(ast::Param::ty);
                    out.push(self.eval_as(a, ty.as_ref())?);
                }
                out
            }
            _ => self.eval_all(&args)?,
        };
        let values = match &m {
            Method::Code(f, imp) => {
                let inherited = imp.and_then(|k| self.trait_method(k, &name));
                self.arranged(f, inherited.as_ref(), node, vec![recv], written)?
            }
            _ => {
                let mut values = vec![recv];
                values.extend(written);
                values
            }
        };
        if let Method::Code(f, imp) = &m {
            let mut types = HashMap::new();
            types.insert("Self".to_string(), head.clone());
            if let Some(k) = *imp {
                let i = &self.program.impls[k];
                let sv = self.target(values[0].clone());
                self.infer_types(&i.self_ty, &sv, &i.params, &mut types);
            }
            types.extend(self.turbofish(node.child(K::GENERIC_ARGS), &f.params));
            // A type parameter only the result names (`collect`'s `C`): the type the context
            // names (`iter.collect.1`).
            if let Some(ret) = f.node.ret_type().and_then(|r| r.ty()) {
                let ret = ret.syntax().text().to_string();
                if f.params.contains(&ret)
                    && !types.contains_key(&ret)
                    && let Some(head) = self.context_head(node)
                {
                    types.insert(ret, head);
                }
            }
            let f = f.clone();
            return self.invoke(&f, values, types, range);
        }
        self.run_method(m, &head, values, range)
    }
}

/// Where a frame's types keep the refinement type parameter `name` is bound to.
fn refined_key(name: &str) -> String {
    format!("{name}#refined")
}
