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
            }
        }
        out
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
        let out = self.memory.temp(Value::String(String::new()));
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
            fields: vec![Value::String(t.name), Value::String(clause), shown],
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
                for i in 0..fields.len() {
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
                    unreachable!("a `&mut String`")
                };
                let info = &self.program.adts[ty];
                let path = if info.is_enum {
                    format!("{}::{variant}", info.name)
                } else {
                    info.name.clone()
                };
                let v = self.program.variant(ty, variant).clone();
                match v.shape {
                    crate::program::Shape::Unit => self.push_text(&out, &path),
                    crate::program::Shape::Named => {
                        self.push_text(&out, &format!("{path} {{ "));
                        for (i, f) in v.fields.iter().enumerate() {
                            let sep = if i == 0 { "" } else { ", " };
                            self.push_text(&out, &format!("{sep}{f}: "));
                            let x = vec![Value::Ref(field(&a, i), false), args[1].clone()];
                            self.call_trait("Debug", "fmt", x, range)?;
                        }
                        self.push_text(&out, " }");
                    }
                    crate::program::Shape::Tuple => {
                        self.push_text(&out, &format!("{path}("));
                        for i in 0..fields.len() {
                            if i > 0 {
                                self.push_text(&out, ", ");
                            }
                            let x = vec![Value::Ref(field(&a, i), false), args[1].clone()];
                            self.call_trait("Debug", "fmt", x, range)?;
                        }
                        self.push_text(&out, ")");
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
        mut types: HashMap<String, Head>,
        range: TextRange,
    ) -> Eval {
        if self.memory.depth() >= MAX_DEPTH {
            return Err(trap(TrapClass::StackOverflow, range));
        }
        // The prelude's built-in methods have no body (`#[intrinsic]`).
        if f.node.body().is_none() {
            let head = types.get("Self").cloned().unwrap_or(Head::Unit);
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
        // A closure (`expr.closure.call.1`).
        if local {
            let slot = self.memory.slot(&segments[0]).expect("a local");
            let v = self.target(self.memory.load(&self.memory.here(slot)));
            if let Value::Closure(c) = v {
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
        let resolved = self.resolve(&segments);
        let is_fn = !local
            && matches!(&resolved, Some((Def::Item(k), n)) if *n == segments.len() && self.program.fns.contains_key(k));
        if !local
            && !is_fn
            && let Some((ty, info)) = self.ctor(&segments)
        {
            let variant = info.name.clone();
            let fields = self.eval_all(&args)?;
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
                let head = match ty {
                    [t] if t == "int" => Head::Int,
                    [t] if t == "bool" => Head::Bool,
                    [t] if t == "char" => Head::Char,
                    [t] if t == "str" => Head::Str,
                    [t] if t == "String" => Head::String,
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
                    .methods(&head, name)
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
                    return self.invoke(&f, values, types, range);
                }
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
        let mut steps = vec![(place.clone(), value.clone(), indirect)];
        if let Value::Ref(q, _) = &value {
            steps.push((q.clone(), self.memory.load(q), true));
        }
        let mut found = None;
        'search: for (p, v, ind) in &steps {
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
        let (m, head, recv) = found.expect("a checked method call");
        let mut values = vec![recv];
        values.extend(self.eval_all(&args)?);
        if let Method::Code(f, imp) = &m {
            let mut types = HashMap::new();
            types.insert("Self".to_string(), head.clone());
            if let Some(k) = *imp {
                let i = &self.program.impls[k];
                let sv = self.target(values[0].clone());
                self.infer_types(&i.self_ty, &sv, &i.params, &mut types);
            }
            types.extend(self.turbofish(node.child(K::GENERIC_ARGS), &f.params));
            let f = f.clone();
            return self.invoke(&f, values, types, range);
        }
        self.run_method(m, &head, values, range)
    }
}
