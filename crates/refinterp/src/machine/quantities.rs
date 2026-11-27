//! Quantities as the machine meets them (`unit.*`): unit literals, values meeting a type with a
//! unit, arithmetic, comparisons and methods on quantities, with the program's units worked out
//! from their declarations (`crate::units`).

use std::collections::HashMap;

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{SyntaxKind as K, SyntaxNode, TextRange};

use super::Machine;
use crate::Eval;
use crate::units::{self as u, DynUnit, Ratio};
use crate::value::{Head, Value};

impl Machine<'_> {
    /// The unit unit expression `node` writes here: its units, and the units this call's unit
    /// parameters stand for. A parameter not bound yet is `unknown`'s key, if given.
    fn unit_here(&self, node: &SyntaxNode, unknown: Option<&str>) -> DynUnit {
        let module = self.memory.module();
        u::walk::<DynUnit>(node, &mut |a| {
            let path = u::atom_path(a);
            if let [one] = path.as_slice() {
                if let Some(Head::Units(x)) = self.memory.type_of(one) {
                    return x;
                }
                if unknown == Some(one.as_str()) {
                    return DynUnit::of(&format!("\u{0}{one}"));
                }
            }
            match self.program.unit_named(module, a) {
                Some(k) => DynUnit::of(&k),
                None => DynUnit::one(),
            }
        })
    }

    /// The unit text `name` names here: `mm` in `{x:[mm]}`.
    pub(super) fn unit_named_text(&self, name: &str) -> DynUnit {
        let parse = moddil_syntax::parse(&format!("fn f() -> f64[{name}] {{}}"));
        match parse
            .syntax()
            .descendants()
            .into_iter()
            .find(|n| n.kind() == K::UNIT_SUFFIX)
        {
            Some(s) => self.unit_of(&s),
            None => DynUnit::one(),
        }
    }

    /// The unit `node` (a `[…]` after a literal or a type) writes.
    pub(super) fn unit_of(&self, node: &SyntaxNode) -> DynUnit {
        self.unit_here(node, None)
    }

    /// For a type written with a unit (`f64[m]`): its base's text and its unit.
    pub(super) fn type_unit(&self, t: &ast::Type) -> Option<(String, DynUnit)> {
        let node = t.syntax();
        let suffix = node.child(K::UNIT_SUFFIX)?;
        let base = node.child(K::PATH)?.text().to_string();
        Some((base.trim().to_string(), self.unit_of(&suffix)))
    }

    /// `v` where type `t`, a number type with a unit, is written: its number of `t`'s base, in
    /// `t`'s unit (`unit.conv.1`).
    pub(super) fn typed_quantity(&mut self, v: Value, t: &ast::Type) -> Option<Value> {
        let (base, unit) = self.type_unit(t)?;
        let (x, from) = u::split(v);
        let x = crate::num::typed_text(x, &base);
        if from.is_one() || from == unit || !u::same_dim(self.program, &from, &unit) {
            return Some(u::quantity(x, unit));
        }
        let point = u::affine(self.program, &from) || u::affine(self.program, &unit);
        let x = u::convert(
            self.program,
            x.clone(),
            &from,
            &unit,
            point,
            TextRange::default(),
        )
        .unwrap_or(x);
        Some(u::quantity(x, unit))
    }

    /// For a parameter of type `t` given `v`: what `t`'s unit parameters stand for
    /// (`unit.param.2`), solved from `v`'s unit.
    pub(super) fn infer_units(
        &self,
        t: &ast::Type,
        v: &Value,
        names: &[String],
        out: &mut HashMap<String, Head>,
    ) {
        let Some(suffix) = t.syntax().child(K::UNIT_SUFFIX) else {
            return;
        };
        let atoms: Vec<String> = suffix
            .descendants()
            .into_iter()
            .filter(|n| n.kind() == K::PATH_TYPE)
            .map(|n| u::atom_path(&n))
            .filter(|p| p.len() == 1)
            .map(|p| p[0].clone())
            .collect();
        let vu = match v {
            Value::Quantity(_, x) => (**x).clone(),
            _ => DynUnit::one(),
        };
        for name in atoms {
            if !names.contains(&name)
                || out.contains_key(&name)
                || self.memory.type_of(&name).is_some()
            {
                continue;
            }
            let whole = self.unit_here(&suffix, Some(&name));
            let key = format!("\u{0}{name}");
            let k = whole
                .0
                .iter()
                .find(|(x, _)| *x == key)
                .map_or(Ratio::ZERO, |(_, e)| *e);
            if k.n == 0 {
                continue;
            }
            let rest = DynUnit(whole.0.iter().filter(|(x, _)| *x != key).cloned().collect());
            let solved = vu
                .div(&rest)
                .pow(Ratio::new(k.d, k.n).unwrap_or(Ratio::ONE));
            out.insert(name, Head::Units(solved));
        }
    }

    /// The number op `k` gives for numbers `x` and `y` (no units).
    fn plain_op(&mut self, k: K, sym: &str, x: Value, y: Value, range: TextRange) -> Eval {
        if crate::num::is_num(&x, &y) {
            return crate::num::arith(sym, x, y, range);
        }
        if let (Value::Int(_), Value::Int(_)) = (&x, &y) {
            return super::ops::arith(k, x, y, range);
        }
        let (tr, method) = match k {
            K::PLUS => ("Add", "add"),
            K::MINUS => ("Sub", "sub"),
            K::STAR => ("Mul", "mul"),
            K::SLASH => ("Div", "div"),
            _ => ("Rem", "rem"),
        };
        self.call_trait(tr, method, vec![x, y], range)
    }

    /// `a op b` where an operand is a quantity (`unit.op.*`). `target` is the unit a type gives
    /// a sum, or, for a compound assignment, the left's, which the right is converted to.
    pub(super) fn quantity_arith(
        &mut self,
        k: K,
        sym: &str,
        a: Value,
        b: Value,
        target: Option<(DynUnit, bool)>,
        range: TextRange,
    ) -> Eval {
        let p = self.program;
        let (x, xu) = u::split(a);
        let (y, yu) = u::split(b);
        let (x0, y0) = (x.clone(), y.clone());
        let (x, y) = (u::settled_like(x, &y0), u::settled_like(y, &x0));
        let conv =
            |v: Value, f: &DynUnit, t: &DynUnit, point: bool| u::convert(p, v, f, t, point, range);
        match k {
            K::PLUS | K::MINUS | K::PERCENT => {
                let (la, ra) = (u::affine(p, &xu), u::affine(p, &yu));
                if la || ra {
                    if la && ra {
                        let base = u::base_unit(p, &xu).unwrap_or_else(|| xu.clone());
                        let x = conv(x, &xu, &base, true)?;
                        let y = conv(y, &yu, &base, true)?;
                        let v = self.plain_op(k, sym, x, y, range)?;
                        return Ok(u::quantity(v, base));
                    }
                    if la {
                        let y = conv(y, &yu, &xu, false)?;
                        let v = self.plain_op(k, sym, x, y, range)?;
                        return Ok(u::quantity(v, xu));
                    }
                    let x = conv(x, &xu, &yu, false)?;
                    let v = self.plain_op(k, sym, x, y, range)?;
                    return Ok(u::quantity(v, yu));
                }
                if xu == yu {
                    let v = self.plain_op(k, sym, x, y, range)?;
                    return Ok(u::quantity(v, xu));
                }
                let to = match target {
                    Some((t, true)) => t,
                    Some((t, false)) if u::same_dim(p, &t, &xu) => t,
                    _ if u::finer(p, &xu, &yu) => xu.clone(),
                    _ => yu.clone(),
                };
                let x = conv(x, &xu, &to, false)?;
                let y = conv(y, &yu, &to, false)?;
                let v = self.plain_op(k, sym, x, y, range)?;
                Ok(u::quantity(v, to))
            }
            _ => {
                let div = k == K::SLASH;
                if div && !xu.is_one() && xu != yu && u::same_dim(p, &xu, &yu) {
                    let to = if u::finer(p, &xu, &yu) {
                        xu.clone()
                    } else {
                        yu.clone()
                    };
                    let x = conv(x, &xu, &to, false)?;
                    let y = conv(y, &yu, &to, false)?;
                    return self.plain_op(k, sym, x, y, range);
                }
                let unit = if div { xu.div(&yu) } else { xu.mul(&yu) };
                let v = self.plain_op(k, sym, x, y, range)?;
                if !unit.is_one() && u::dims(p, &unit).is_empty() {
                    return conv(v, &unit, &DynUnit::one(), false);
                }
                Ok(u::quantity(v, unit))
            }
        }
    }

    /// Two compared values, if one is a quantity: both numbers in the finer unit.
    pub(super) fn compared_numbers(
        &self,
        a: Value,
        b: Value,
        range: TextRange,
    ) -> Eval<(Value, Value)> {
        if !matches!(a, Value::Quantity(..)) && !matches!(b, Value::Quantity(..)) {
            return Ok((a, b));
        }
        let p = self.program;
        let (x, xu) = u::split(a);
        let (y, yu) = u::split(b);
        if xu == yu {
            return Ok((x, y));
        }
        let point = u::affine(p, &xu) || u::affine(p, &yu);
        let to = if u::finer(p, &xu, &yu) {
            xu.clone()
        } else {
            yu.clone()
        };
        let x = u::convert(p, x, &xu, &to, point, range)?;
        let y = u::convert(p, y, &yu, &to, point, range)?;
        Ok((x, y))
    }

    /// The unit a method's turbofish names: `to_unit::<cm>`.
    fn turbofish_unit(&self, node: &SyntaxNode) -> Option<DynUnit> {
        let args = node.child(K::GENERIC_ARGS)?;
        let t = args
            .descendants()
            .into_iter()
            .find(|n| n.kind() == K::PATH_TYPE)?;
        Some(u::walk::<DynUnit>(
            &t,
            &mut |a| match self.program.unit_named(self.memory.module(), a) {
                Some(k) => DynUnit::of(&k),
                None => {
                    let path = u::atom_path(a);
                    match path.as_slice() {
                        [one] => match self.memory.type_of(one) {
                            Some(Head::Units(x)) => x,
                            _ => DynUnit::one(),
                        },
                        _ => DynUnit::one(),
                    }
                }
            },
        ))
    }

    /// A method call on a quantity, or `with_unit` on a number (`unit.method.*`), its receiver
    /// `recv` evaluated already: its value, or `None` if it is an ordinary call.
    pub(super) fn quantity_method(
        &mut self,
        node: &SyntaxNode,
        name: &str,
        recv: &Value,
        args: &[ast::Expr],
        range: TextRange,
    ) -> Eval<Option<Value>> {
        let mut v = recv.clone();
        while let Value::Ref(p, _) = &v {
            v = self.memory.load(p);
        }
        let p = self.program;
        if name == "with_unit" && !matches!(v, Value::Quantity(..)) {
            let Some(to) = self.turbofish_unit(node) else {
                return Ok(None);
            };
            return Ok(Some(u::quantity(u::settled(v), to)));
        }
        let Value::Quantity(x, xu) = v else {
            return Ok(None);
        };
        let (x, xu) = (u::settled(*x), (*xu).clone());
        match name {
            "to_unit" | "value_in" => {
                let to = self.turbofish_unit(node).unwrap_or_else(DynUnit::one);
                let point = u::affine(p, &xu) || u::affine(p, &to);
                let y = if let [mode] = args {
                    // An integer rounded (`unit.to.1`): `x · n / d` by the mode.
                    let mode = self.eval(mode)?;
                    let (n, d) = u::ratio_of(p, &xu, &to);
                    let f = self.program.fns["__rescale"].clone();
                    let args = vec![x, Value::Int(n as i64), Value::Int(d as i64), mode];
                    self.invoke(&f, args, HashMap::new(), range)?
                } else {
                    u::convert(p, x, &xu, &to, point, range)?
                };
                return Ok(Some(if name == "to_unit" {
                    u::quantity(y, to)
                } else {
                    y
                }));
            }
            "per_radian" => {
                let rad = DynUnit::of("rad");
                let mut rest = DynUnit::one();
                let mut k = Ratio::ZERO;
                for (key, e) in &xu.0 {
                    if u::dims(p, &DynUnit::of(key)) == u::dims(p, &rad) {
                        k = Ratio::new(k.n * e.d + e.n * k.d, k.d * e.d).unwrap_or(k);
                    } else {
                        rest = rest.mul(&DynUnit(vec![(key.clone(), *e)]));
                    }
                }
                let to = rest.mul(&rad.pow(k));
                let y = u::convert(p, x, &xu, &to, false, range)?;
                return Ok(Some(u::quantity(y, rest)));
            }
            _ => {}
        }
        // A base's own method: on the number, its result's unit by what it does.
        let (arg_vals, unit) = match name {
            "abs" | "floor" | "ceil" | "trunc" | "fract" | "min" | "max" | "clamp" => {
                let mut vs = Vec::new();
                for a in args {
                    let av = self.eval(a)?;
                    let (ay, au) = u::split(av);
                    vs.push(u::convert(p, ay, &au, &xu, false, range)?);
                }
                (vs, Some(xu.clone()))
            }
            "round" | "to" | "checked_to" | "truncate" | "saturate" | "wrap" => {
                (self.eval_all(args)?, Some(xu.clone()))
            }
            "sqrt" => (
                Vec::new(),
                Some(xu.pow(Ratio::new(1, 2).unwrap_or(Ratio::ONE))),
            ),
            "powi" => {
                let vs = self.eval_all(args)?;
                let n = match vs.first() {
                    Some(Value::Int(n)) => *n as i128,
                    _ => 1,
                };
                (vs, Some(xu.pow(Ratio::new(n, 1).unwrap_or(Ratio::ONE))))
            }
            "sin" | "cos" | "tan" => {
                let x = u::convert(p, x.clone(), &xu, &DynUnit::of("rad"), false, range)?;
                let r = self.base_method(node, name, x, Vec::new(), range)?;
                return Ok(Some(r));
            }
            "atan2" => {
                let mut vs = Vec::new();
                for a in args {
                    let av = self.eval(a)?;
                    let (ay, au) = u::split(av);
                    vs.push(u::convert(p, ay, &au, &xu, false, range)?);
                }
                (vs, None)
            }
            "is_nan" | "is_finite" | "signum" => (self.eval_all(args)?, None),
            _ => return Ok(None),
        };
        let r = self.base_method(node, name, x, arg_vals, range)?;
        Ok(Some(match unit {
            Some(w) => match r {
                // A conversion's `Result`/`Option` keeps the unit on its value.
                r @ Value::Adt { .. } => r,
                r => u::quantity(r, w),
            },
            None => r,
        }))
    }

    /// Method `name` of number `x`'s type, called with `args`.
    fn base_method(
        &mut self,
        node: &SyntaxNode,
        name: &str,
        x: Value,
        args: Vec<Value>,
        range: TextRange,
    ) -> Eval {
        let head = x.head();
        let Some(crate::program::Method::Code(f, _)) =
            self.program.methods(&head, name).into_iter().next()
        else {
            unreachable!("a checked number method");
        };
        let mut all = vec![x];
        all.extend(args);
        let mut types = HashMap::new();
        types.insert("Self".to_string(), head);
        // A conversion's target type (`round::<int>`).
        if let Some(t) = node
            .child(K::GENERIC_ARGS)
            .and_then(|g| g.descendants().into_iter().find_map(ast::Type::cast))
        {
            let text = t.syntax().text().to_string();
            if let Some(k) = crate::num::Kind::named(text.trim()) {
                types.insert("T".to_string(), Head::Num(k));
            } else if text.trim() == "int" || text.trim() == "i64" {
                types.insert("T".to_string(), Head::Int);
            }
        }
        self.invoke(&f, all, types, range)
    }
}

impl Machine<'_> {
    /// `s.parse_quantity::<f64[m]>()` (`unit.parse.1`): the prelude reads the text, with a table
    /// of the units of the quantity's dimension and how each converts to its unit.
    pub(super) fn parse_quantity(
        &mut self,
        node: &SyntaxNode,
        recv: &Value,
        range: TextRange,
    ) -> Eval<Option<Value>> {
        let Some(t) = node
            .child(K::GENERIC_ARGS)
            .and_then(|g| g.descendants().into_iter().find_map(ast::Type::cast))
        else {
            return Ok(None);
        };
        let Some((base, to)) = self.type_unit(&t) else {
            return Ok(None);
        };
        let int = base == "int" || base == "i64";
        let p = self.program;
        let dims = u::dims(p, &to);
        let mut table = String::new();
        let mut seen = Vec::new();
        for key in p.unit_order.clone() {
            let from = DynUnit::of(&key);
            if u::dims(p, &from) != dims {
                continue;
            }
            let name = key.rsplit(':').next().unwrap_or(&key).to_string();
            if seen.contains(&name) {
                continue;
            }
            seen.push(name.clone());
            if let Some(line) = u::parse_line(p, &name, &from, &to, int) {
                table.push_str(&line);
            }
        }
        let func = if int {
            "__parse_quantity_int"
        } else {
            "__parse_quantity_f64"
        };
        let f = self.program.fns[func].clone();
        let r = self.invoke(
            &f,
            vec![recv.clone(), Value::Str(table.into())],
            HashMap::new(),
            range,
        )?;
        Ok(Some(match r {
            Value::Adt {
                ty,
                variant,
                mut fields,
            } if variant == "Ok" => {
                let v = fields.remove(0);
                fields.insert(0, u::quantity(v, to));
                Value::Adt {
                    ty,
                    variant,
                    fields,
                }
            }
            r => r,
        }))
    }
}
