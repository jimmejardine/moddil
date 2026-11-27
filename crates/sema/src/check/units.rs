//! Quantities in expressions (`unit.op.*`, `unit.conv.*`, `unit.affine.*`): the units of
//! arithmetic and comparisons, and the conversions between units of one dimension, recorded on
//! the expressions whose values are converted and computed once the units are known.

use moddil_syntax::TextRange;

use super::Checker;
use crate::hir::{BinaryOp, ExprId, ExprKind};
use crate::units::{Factor, UnitAtom, UnitTy};
use crate::{Code, Ty};

/// A conversion found while checking, of expression `expr`'s value (of type `base` in unit
/// `from`) to unit `to`: of a point (applying offsets) or of a difference.
pub(super) struct Pending {
    expr: ExprId,
    base: Ty,
    from: UnitTy,
    to: UnitTy,
    point: bool,
    range: TextRange,
}

impl Checker<'_> {
    /// Records that `e`'s value, of type `base` in unit `from`, is converted to unit `to`.
    pub(super) fn convert_unit(
        &mut self,
        e: ExprId,
        base: &Ty,
        from: &UnitTy,
        to: &UnitTy,
        point: bool,
    ) {
        if from == to {
            return;
        }
        // A block's value is reported at its tail.
        let mut at = e;
        while let ExprKind::Block { tail: Some(t), .. } = &self.body.exprs[at].kind {
            at = *t;
        }
        let range = self.range(at);
        self.unit_pending.push(Pending {
            expr: e,
            base: base.clone(),
            from: from.clone(),
            to: to.clone(),
            point,
            range,
        });
    }

    /// The units' names, for messages.
    fn show_unit(&self, u: &UnitTy) -> String {
        let u = self.solver.resolve_units(u);
        if u.is_one() {
            "a plain number".to_string()
        } else {
            format!("`{u:?}`")
        }
    }

    /// E0224: `what` needs `a` and `b` in one dimension.
    fn dimensions_differ(&mut self, range: TextRange, what: &str, a: &UnitTy, b: &UnitTy) {
        let (a, b) = (self.show_unit(a), self.show_unit(b));
        self.report(
            Code::E0224,
            range,
            format!("{what} needs one dimension, and these are {a} and {b}"),
        );
    }

    /// Whether `u` is a dimension's affine unit (`unit.affine.1`).
    fn affine(&self, u: &UnitTy) -> bool {
        let u = self.solver.resolve_units(u);
        self.items.is_affine(&u)
    }

    /// The finer of two units of one dimension, both known: the smaller, or, of two the same
    /// size, the first in the units' own order, so the answer does not depend on which operand
    /// is which (`unit.op.add.1`).
    fn finer(&self, a: &UnitTy, b: &UnitTy) -> bool {
        let (sa, sb) = (
            self.items.scale_of(a).magnitude(),
            self.items.scale_of(b).magnitude(),
        );
        sa < sb || (sa == sb && a <= b)
    }

    /// The base unit of dimension `u`'s, if it has one: of scale one and not affine.
    fn base_unit(&self, u: &UnitTy) -> Option<UnitTy> {
        let d = self.items.dim_of(u)?;
        let i = self.items.units.iter().position(|x| {
            x.dim == d && x.offset.is_none() && x.scale == crate::units::Scale::ONE
        })?;
        let name = self.items.units[i].name.as_str();
        Some(UnitTy::atom(UnitAtom::Named(i as u32, name.into())))
    }

    /// `l op r` where an operand is a quantity (`unit.op.*`): `lt` is the left's type, and `rt`
    /// the right's if it was inferred already. `l` is the left operand, which may be converted
    /// (not in a compound assignment, where the right one is converted to it).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn quantity_binary(
        &mut self,
        e: ExprId,
        range: TextRange,
        op: BinaryOp,
        lt: &Ty,
        l: Option<ExprId>,
        r: ExprId,
        rt: Option<Ty>,
    ) -> Ty {
        let (lb, lu) = self.norm(lt).split_unit();
        let lb = self.norm(&lb);
        let (rb, ru) = match rt {
            Some(t) => self.norm(&t).split_unit(),
            None if self.literal_operand(r) && !self.body.units.contains_key(&r) => {
                // A plain literal: of the left's base, with no unit.
                self.check(r, &lb);
                (lb.clone(), UnitTy::one())
            }
            None => {
                let t = self.infer(r);
                let t = self.read_through(r, &t, true);
                self.norm(&t).split_unit()
            }
        };
        let rb = self.norm(&rb);
        let builtin =
            matches!(lb, Ty::Int | Ty::Num(_)) || self.solver.table.lit_kind(&lb).is_some();
        let names = match op {
            BinaryOp::Add => ("Add", "add"),
            BinaryOp::Sub => ("Sub", "sub"),
            BinaryOp::Mul => ("Mul", "mul"),
            BinaryOp::Div => ("Div", "div"),
            BinaryOp::Rem => ("Rem", "rem"),
            _ => {
                self.report(
                    Code::E0227,
                    range,
                    format!("`{}` is not defined on a quantity", op.symbol()),
                );
                return Ty::Error;
            }
        };
        // The bases: one number type, or the base's own operator.
        let base = if builtin {
            if !self.coerce(&rb, &lb) {
                self.mismatch(r, &lb, &rb);
                return Ty::Error;
            }
            lb.clone()
        } else {
            match self.operator_with(e, range, op.symbol(), &lb, Some(&rb), names) {
                Some(Ty::Error) | None => return Ty::Error,
                Some(t) => t,
            }
        };
        let (lu, ru) = (
            self.solver.resolve_units(&lu),
            self.solver.resolve_units(&ru),
        );
        let (la, ra) = (self.affine(&lu), self.affine(&ru));
        let unit = match op {
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Rem => {
                if lu.has_infer() || ru.has_infer() {
                    if !self.solver.unify_units(&lu, &ru) {
                        self.dimensions_differ(range, &format!("`{}`", op.symbol()), &lu, &ru);
                        return Ty::Error;
                    }
                    self.solver.resolve_units(&lu)
                } else if !self.items.converts(&ru, &lu) {
                    self.dimensions_differ(range, &format!("`{}`", op.symbol()), &lu, &ru);
                    return Ty::Error;
                } else if la || ra {
                    // Points and differences (`unit.affine.1`).
                    match (op, la, ra) {
                        // A point and a difference: a point, the difference in its scale.
                        (BinaryOp::Add | BinaryOp::Sub, true, false) => {
                            self.convert_unit(r, &rb, &ru, &lu, false);
                            lu
                        }
                        (BinaryOp::Add, false, true) if l.is_some() => {
                            self.convert_unit(l.expect("left"), &lb, &lu, &ru, false);
                            ru
                        }
                        // Two points apart: a difference, in the dimension's base unit.
                        (BinaryOp::Sub, true, true) => {
                            let Some(b) = self.base_unit(&lu) else {
                                self.report(
                                    Code::E0228,
                                    range,
                                    "the difference of two points is in their dimension's base \
                                     unit, and this dimension has none",
                                );
                                return Ty::Error;
                            };
                            let Some(l) = l else {
                                self.affine_misuse(range, op);
                                return Ty::Error;
                            };
                            self.convert_unit(l, &lb, &lu, &b, true);
                            self.convert_unit(r, &rb, &ru, &b, true);
                            b
                        }
                        _ => {
                            self.affine_misuse(range, op);
                            return Ty::Error;
                        }
                    }
                } else if lu == ru {
                    lu
                } else if let Some(target) = self.add_target(e, &lu)
                    && let Some(l) = l
                {
                    // The unit the sum is wanted in (`unit.op.add.1`).
                    self.convert_unit(l, &lb, &lu, &target, false);
                    self.convert_unit(r, &rb, &ru, &target, false);
                    target
                } else {
                    match l {
                        Some(l) if !self.finer(&lu, &ru) => {
                            self.convert_unit(l, &lb, &lu, &ru, false);
                            ru
                        }
                        _ => {
                            self.convert_unit(r, &rb, &ru, &lu, false);
                            lu
                        }
                    }
                }
            }
            BinaryOp::Mul | BinaryOp::Div => {
                if la || ra {
                    self.affine_misuse(range, op);
                    return Ty::Error;
                }
                let div = op == BinaryOp::Div;
                // A quotient of one dimension: both in the finer unit, then a plain number.
                if div
                    && !lu.has_vars()
                    && !ru.has_vars()
                    && lu != ru
                    && !lu.is_one()
                    && self.items.converts(&lu, &ru)
                    && let Some(l) = l
                {
                    if self.finer(&lu, &ru) {
                        self.convert_unit(r, &rb, &ru, &lu, false);
                    } else {
                        self.convert_unit(l, &lb, &lu, &ru, false);
                    }
                    UnitTy::one()
                } else {
                    let u = if div { lu.div(&ru) } else { lu.mul(&ru) };
                    // A product with no dimension left: a plain number, its scale applied
                    // (`unit.op.mul.2`).
                    if !u.has_vars()
                        && !u.is_one()
                        && self.items.dim_of(&u).is_some_and(|d| d.is_empty())
                    {
                        self.convert_unit(e, &base, &u, &UnitTy::one(), false);
                        UnitTy::one()
                    } else {
                        u
                    }
                }
            }
            _ => unreachable!("an arithmetic operator"),
        };
        Ty::quantity(base, unit)
    }

    /// The unit sum `e` is wanted in, if a type says (`let x: f64[m] = a + b`) and it is one of
    /// the operands' dimension (unit `u`'s).
    fn add_target(&mut self, e: ExprId, u: &UnitTy) -> Option<UnitTy> {
        let (x, t) = self.sum_target.clone()?;
        let t = self.solver.resolve_units(&t);
        (x == e && !t.has_vars() && self.items.converts(u, &t) && !self.items.is_affine(&t))
            .then_some(t)
    }

    fn affine_misuse(&mut self, range: TextRange, op: BinaryOp) {
        self.report(
            Code::E0228,
            range,
            format!(
                "`{}` is not defined on a point of an affine unit: a point and a difference add, \
                 and two points subtract",
                op.symbol()
            ),
        );
    }

    /// The right operand of `op` on a plain number of type `lt`, if it is a quantity, inferred
    /// (`2.0 * x`); `None` if it is better checked against `lt`, a literal without a unit.
    pub(super) fn quantity_right(&mut self, op: BinaryOp, r: ExprId) -> Option<Ty> {
        let arith = matches!(
            op,
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem
        );
        if !arith || (self.literal_operand(r) && !self.body.units.contains_key(&r)) {
            return None;
        }
        let t = self.infer(r);
        let t = self.read_through(r, &t, true);
        Some(self.norm(&t))
    }

    /// Each operand of a comparison chain whose types are `ts`, if one is a quantity
    /// (`unit.op.add.1`): all in one dimension, converted to the finest unit among them. The
    /// types become their bases.
    pub(super) fn compare_units(&mut self, xs: &[ExprId], ts: &mut [Ty], range: TextRange) {
        let split: Vec<(Ty, UnitTy)> = ts.iter().map(|t| self.norm(t).split_unit()).collect();
        if split.iter().all(|(_, u)| u.is_one()) {
            return;
        }
        let units: Vec<UnitTy> = split
            .iter()
            .map(|(_, u)| self.solver.resolve_units(u))
            .collect();
        let mut target = units[0].clone();
        if units.iter().any(UnitTy::has_infer) {
            for u in &units[1..] {
                if !self.solver.unify_units(&target, u) {
                    self.dimensions_differ(range, "a comparison", &target, u);
                    return;
                }
            }
        } else {
            for u in &units[1..] {
                if !self.items.converts(u, &target) {
                    self.dimensions_differ(range, "a comparison", &target, u);
                    return;
                }
                if self.finer(u, &target) {
                    target = u.clone();
                }
            }
            let point = units.iter().any(|u| self.items.is_affine(u));
            for (k, u) in units.iter().enumerate() {
                self.convert_unit(xs[k], &split[k].0, u, &target, point);
            }
        }
        for (t, (b, _)) in ts.iter_mut().zip(split) {
            *t = b;
        }
    }

    /// `e`'s value, of type `found`, where a quantity of type `expected` is wanted
    /// (`unit.conv.1`): converted if both are quantities of one dimension in different units,
    /// with the same base. Whether it was.
    pub(super) fn unit_coercion(&mut self, e: ExprId, found: &Ty, expected: &Ty) -> bool {
        let (Ty::Quantity(fb, fu), Ty::Quantity(xb, xu)) = (self.norm(found), self.norm(expected))
        else {
            return false;
        };
        let (fu, xu) = (
            self.solver.resolve_units(&fu),
            self.solver.resolve_units(&xu),
        );
        if fu == xu || fu.has_infer() || xu.has_infer() || !self.items.converts(&fu, &xu) {
            return false;
        }
        if !self.solver.unify(&fb, &xb) {
            return false;
        }
        let point = self.items.is_affine(&fu) || self.items.is_affine(&xu);
        self.convert_unit(e, &fb, &fu, &xu, point);
        true
    }

    /// Computes each conversion's factor now the units are known (`unit.conv.1`): an integer's
    /// must be whole (E0225), and a type with no built-in arithmetic converts only to a unit of
    /// the same scale (E0226).
    pub(super) fn finish_unit_convs(&mut self) {
        for p in std::mem::take(&mut self.unit_pending) {
            let from = self.solver.resolve_units(&p.from);
            let to = self.solver.resolve_units(&p.to);
            if from == to || from.has_infer() || to.has_infer() || !self.items.converts(&from, &to)
            {
                continue;
            }
            let base = self.solver.deep(&p.base);
            let conv = self.items.conversion(&from, &to, p.point);
            let identity = conv.factor == Factor::Exact(1, 1) && conv.offset.is_none();
            if identity {
                continue;
            }
            let int = match &base {
                Ty::Int => Some((
                    i128::from(i64::MIN),
                    i128::from(i64::MAX),
                    "int".to_string(),
                )),
                Ty::Num(n) if !n.is_float() => {
                    let (lo, hi) = n.int_range();
                    Some((lo, hi, n.name().to_string()))
                }
                Ty::Num(_) => None,
                Ty::Error => continue,
                t => {
                    let t = t.clone();
                    self.report(
                        Code::E0226,
                        p.range,
                        format!(
                            "`{t}` has no built-in arithmetic, so it converts only between units \
                             of one scale, not from {} to {}",
                            self.show_unit(&from),
                            self.show_unit(&to)
                        ),
                    );
                    continue;
                }
            };
            if let Some((lo, hi, name)) = int {
                let whole = |f: Factor| f.as_int().filter(|n| *n >= lo && *n <= hi);
                let ok =
                    whole(conv.factor).is_some() && conv.offset.is_none_or(|o| whole(o).is_some());
                if !ok {
                    self.report(
                        Code::E0225,
                        p.range,
                        format!(
                            "an `{name}` in {} is not always a whole number of {}: convert it with \
                             `.to_unit::<…>(Round::…)`, which says how to round",
                            self.show_unit(&from),
                            self.show_unit(&to)
                        ),
                    );
                    continue;
                }
            }
            self.out.unit_convs.insert(p.expr, conv);
        }
    }
}

/// What a base's method does to a quantity's unit (`unit.method.1`).
enum MethodUnit {
    /// It keeps it, and takes its other operands in it: `abs`, `min`, `clamp`.
    Keep,
    /// Its result, a number of another type, has the unit: `round::<int>(mode)`.
    Convert,
    /// Its result is the unit to a power: `sqrt`, `powi`.
    Power,
    /// It takes an angle, as radians, and gives a plain number: `sin`.
    Angle,
    /// It takes another operand in the unit, and gives a plain number: `atan2`.
    Ratio,
    /// It gives something with no unit: `is_nan`.
    Plain,
}

fn method_unit(name: &str) -> Option<MethodUnit> {
    Some(match name {
        "abs" | "floor" | "ceil" | "trunc" | "fract" | "min" | "max" | "clamp" => MethodUnit::Keep,
        "round" | "to" | "checked_to" | "truncate" | "saturate" | "wrap" => MethodUnit::Convert,
        "sqrt" | "powi" => MethodUnit::Power,
        "sin" | "cos" | "tan" => MethodUnit::Angle,
        "atan2" => MethodUnit::Ratio,
        "is_nan" | "is_finite" | "signum" => MethodUnit::Plain,
        _ => return None,
    })
}

impl Checker<'_> {
    /// The unit argument of `name::<u>`, which it takes one of.
    fn unit_arg(
        &mut self,
        name: &str,
        name_range: TextRange,
        given: Option<&[Ty]>,
    ) -> Option<UnitTy> {
        if let Some([t]) = given {
            match self.norm(t) {
                Ty::Units(v) => return Some(self.solver.resolve_units(&v)),
                Ty::Error => return None,
                _ => {}
            }
        }
        self.report(
            Code::E0202,
            name_range,
            format!("`{name}` takes a unit: `{name}::<cm>`"),
        );
        None
    }

    /// A method call on a quantity, or a quantity's method on a number (`unit.method.*`): its
    /// type, or `None` if it is an ordinary method call (a trait's, `clone`).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn quantity_method(
        &mut self,
        e: ExprId,
        receiver: ExprId,
        rt: &Ty,
        name: &str,
        name_range: TextRange,
        given: Option<&[Ty]>,
        args: &[ExprId],
    ) -> Option<Ty> {
        let range = self.range(e);
        let rn = self.norm(rt);
        let through = matches!(rn, Ty::Ref(..));
        let inner = self.norm(rn.strip_ref());
        let (b, u) = inner.split_unit();
        self.solver.table.default_literal(&b);
        let b = self.norm(&b);
        let u = self.solver.resolve_units(&u);
        match name {
            // A plain number given a unit (`unit.with.1`): it is the same number.
            "with_unit" if u.is_one() && matches!(b, Ty::Int | Ty::Num(_) | Ty::Adt(..)) => {
                let v = self.unit_arg(name, name_range, given)?;
                self.no_args(args);
                self.out.unit_passthrough.insert(e, through);
                return Some(Ty::quantity(b, v));
            }
            _ if u.is_one() => return None,
            // The quantity in another unit (`unit.to.1`), or its number in one (`unit.with.1`).
            "to_unit" | "value_in" => {
                let v = self.unit_arg(name, name_range, given)?;
                if !self.items.converts(&u, &v) {
                    self.dimensions_differ(range, &format!("`{name}`"), &u, &v);
                    return Some(Ty::Error);
                }
                let point = self.items.is_affine(&u) || self.items.is_affine(&v);
                let result = if name == "to_unit" {
                    Ty::quantity(b.clone(), v.clone())
                } else {
                    b.clone()
                };
                // With a rounding mode, an `int`'s (`unit.to.1`).
                if let [mode] = args {
                    let mode_ty = self.round_ty();
                    self.check(*mode, &mode_ty);
                    if b != Ty::Int {
                        self.report(
                            Code::E0226,
                            range,
                            "a rounding mode converts an `int` quantity; a float converts without one",
                        );
                        return Some(Ty::Error);
                    }
                    let conv = self.items.conversion(&u, &v, point);
                    let Factor::Exact(n, d) = conv.factor else {
                        self.report(
                            Code::E0225,
                            range,
                            "this conversion is not by a ratio of whole numbers",
                        );
                        return Some(Ty::Error);
                    };
                    if conv.offset.is_some() {
                        self.report(
                            Code::E0228,
                            range,
                            "a point of an affine unit converts without a rounding mode",
                        );
                        return Some(Ty::Error);
                    }
                    self.out.unit_rescales.insert(e, (n, d, through));
                    return Some(result);
                }
                self.no_args(args);
                // Through a borrow, the value read is converted, not the borrow.
                let at = if through { e } else { receiver };
                self.convert_unit(at, &b, &u, &v, point);
                self.out.unit_passthrough.insert(e, through);
                return Some(result);
            }
            // An angle-bearing quantity's number per radian (`unit.angle.2`).
            "per_radian" => {
                self.no_args(args);
                let (Some(angle), Some(rad)) = (self.items.lang("Angle"), self.items.lang("rad"))
                else {
                    return Some(Ty::Error);
                };
                let one_angle = crate::units::Dim::from([(angle, crate::units::Ratio::ONE)]);
                let mut rest = UnitTy::one();
                let mut k = crate::units::Ratio::ZERO;
                for (a, x) in u.parts() {
                    let is_angle = matches!(a, UnitAtom::Named(i, _)
                        if self.items.units[*i as usize].dim == one_angle);
                    if is_angle {
                        k = k.add(*x).unwrap_or(k);
                    } else {
                        rest = rest.mul(&UnitTy::atom(a.clone()).pow(*x));
                    }
                }
                if k.is_zero() {
                    self.report(
                        Code::E0227,
                        range,
                        "`per_radian` takes a quantity with an angle in its unit",
                    );
                    return Some(Ty::Error);
                }
                let rad = UnitTy::atom(UnitAtom::Named(rad, "rad".into())).pow(k);
                let at = if through { e } else { receiver };
                self.convert_unit(at, &b, &u, &rest.mul(&rad), false);
                self.out.unit_passthrough.insert(e, through);
                return Some(Ty::quantity(b, rest));
            }
            _ => {}
        }
        let Some(kind) = method_unit(name) else {
            // A trait's method (`clone`, `cmp`) is the ordinary call's.
            if self
                .solver
                .lookup_method(&rn, name)
                .is_ok_and(|p| p.is_some())
            {
                return None;
            }
            for &a in args {
                self.infer(a);
            }
            let t = self.show(&rn);
            self.report(
                Code::E0227,
                name_range,
                format!(
                    "`{t}` has no `{name}`: a quantity has the methods that keep or change its \
                     unit sensibly, and its number in a unit is `value_in::<u>()`"
                ),
            );
            return Some(Ty::Error);
        };
        if self.items.is_affine(&u) {
            for &a in args {
                self.infer(a);
            }
            self.report(
                Code::E0228,
                name_range,
                format!("`{name}` is not defined on a point of an affine unit"),
            );
            return Some(Ty::Error);
        }
        let Some(func) = self.items.builtin_method(&b, name) else {
            for &a in args {
                self.infer(a);
            }
            let t = self.show(&rn);
            self.report(Code::E0227, name_range, format!("`{t}` has no `{name}`"));
            return Some(Ty::Error);
        };
        let sig = &self.items.fns[func as usize];
        let own = sig.generics.params.len() - sig.generics.parent as usize;
        let self_kind = sig.self_kind;
        let targs = self.own_args(given, own, name_range, name);
        let (params, ret) = self.instantiate_sig(func, &targs, range);
        let q = Ty::quantity(b.clone(), u.clone());
        let lift = |t: &Ty| if *t == b { q.clone() } else { t.clone() };
        let others = params.get(1..).unwrap_or(&[]).to_vec();
        let (rest, ret): (Vec<Ty>, Ty) = match kind {
            MethodUnit::Keep => (others.iter().map(lift).collect(), lift(&ret)),
            MethodUnit::Convert => (others, Ty::quantity(ret, u.clone())),
            MethodUnit::Plain => (others, ret),
            MethodUnit::Ratio => (others.iter().map(lift).collect(), ret),
            MethodUnit::Angle => {
                let rad = self
                    .items
                    .lang("rad")
                    .map(|r| UnitTy::atom(UnitAtom::Named(r, "rad".into())));
                match rad {
                    Some(rad) if self.items.converts(&u, &rad) && !through => {
                        self.convert_unit(receiver, &b, &u, &rad, false);
                    }
                    Some(rad) if self.items.converts(&u, &rad) => {
                        for &a in args {
                            self.infer(a);
                        }
                        self.report(
                            Code::E0227,
                            name_range,
                            format!(
                                "`{name}` of an angle behind a borrow is written `(*x).{name}()`"
                            ),
                        );
                        return Some(Ty::Error);
                    }
                    _ => {
                        for &a in args {
                            self.infer(a);
                        }
                        let what = self.show_unit(&u);
                        self.report(
                            Code::E0227,
                            name_range,
                            format!("`{name}` takes an angle, and this is {what}"),
                        );
                        return Some(Ty::Error);
                    }
                }
                (others, ret)
            }
            MethodUnit::Power => {
                let k = if name == "sqrt" {
                    crate::units::Ratio::new(1, 2)
                } else {
                    match args.first().map(|a| &self.body.exprs[*a].kind) {
                        Some(ExprKind::Int(v)) => v.to_i128().map(crate::units::Ratio::int),
                        _ => None,
                    }
                };
                let Some(k) = k else {
                    for &a in args {
                        self.infer(a);
                    }
                    self.report(
                        Code::E0227,
                        name_range,
                        "`powi` of a quantity takes a literal exponent, its unit's power",
                    );
                    return Some(Ty::Error);
                };
                (others, Ty::quantity(ret, u.pow(k)))
            }
        };
        let autoref = match self_kind {
            Some(crate::items::SelfKind::Ref) if !through => Some(false),
            Some(crate::items::SelfKind::RefMut) => Some(true),
            _ => None,
        };
        let deref = through && !matches!(self_kind, Some(crate::items::SelfKind::Ref));
        self.out
            .adjusts
            .insert(e, crate::traits::Adjust { deref, autoref });
        self.out.calls.insert(
            e,
            crate::check::Callee::Fn {
                func,
                args: targs.clone(),
            },
        );
        self.args(e, range, func, &rest, args);
        self.fallible_call(e, func, &targs, range);
        Some(self.norm(&ret))
    }

    /// The prelude's `Round`, as a type.
    fn round_ty(&self) -> Ty {
        self.items
            .modules
            .first()
            .and_then(|m| m.items.get("Round"))
            .and_then(|x| match x.def {
                crate::items::Def::Adt(a) => {
                    Some(self.items.adts[a as usize].ty_with(a, Vec::new()))
                }
                _ => None,
            })
            .unwrap_or(Ty::Error)
    }

    /// Infers `args`, which a call takes none of (E0202 if there are some).
    fn no_args(&mut self, args: &[ExprId]) {
        for &a in args {
            self.infer(a);
            self.report(Code::E0202, self.range(a), "this takes no arguments");
        }
    }
}

impl Checker<'_> {
    /// Hole `h`, of type `t`, written in unit `u` (`unit.fmt.1`): a quantity of `u`'s dimension,
    /// converted to it.
    pub(super) fn fmt_in_unit(&mut self, h: ExprId, t: &Ty, u: &UnitTy) {
        let range = self.range(h);
        let t = self.norm(t);
        if let Ty::Ref(..) = t {
            self.report(
                Code::E0227,
                range,
                "a quantity written in a unit is written by value: `{*x:[u]}`",
            );
            return;
        }
        let (b, from) = t.split_unit();
        let from = self.solver.resolve_units(&from);
        if from.has_infer() {
            self.solver.unify_units(&from, u);
            return;
        }
        if !self.items.converts(&from, u) {
            self.dimensions_differ(range, "writing it in a unit", &from, u);
            return;
        }
        let point = self.items.is_affine(&from) || self.items.is_affine(u);
        self.convert_unit(h, &b, &from, u, point);
    }
}

impl Checker<'_> {
    /// The prelude's struct or enum `name`, with type arguments `args`.
    fn prelude_adt(&self, name: &str, args: Vec<Ty>) -> Ty {
        self.items
            .modules
            .first()
            .and_then(|m| m.items.get(name))
            .and_then(|x| match x.def {
                crate::items::Def::Adt(a) => Some(self.items.adts[a as usize].ty_with(a, args)),
                _ => None,
            })
            .unwrap_or(Ty::Error)
    }

    /// `s.parse_quantity::<f64[m]>()` on a `&str` (`unit.parse.1`): the text as a quantity of
    /// that type, in any declared unit of its dimension, or `UnitParseError`. The units and how
    /// each converts are a table the call is given; `None` if the receiver is not text.
    pub(super) fn parse_quantity(
        &mut self,
        e: ExprId,
        rt: &Ty,
        name_range: TextRange,
        given: Option<&[Ty]>,
        args: &[ExprId],
    ) -> Option<Ty> {
        let text = Ty::reference(false, Ty::Str);
        if !self.solver.can_unify(rt, &text) {
            return None;
        }
        self.no_args(args);
        let range = self.range(e);
        let q = match given {
            Some([t]) => self.norm(t),
            _ => {
                self.report(
                    Code::E0202,
                    name_range,
                    "`parse_quantity` takes the quantity type: `parse_quantity::<f64[m]>()`",
                );
                return Some(Ty::Error);
            }
        };
        let (b, to) = q.split_unit();
        let to = self.solver.resolve_units(&to);
        let int = match b {
            Ty::Int => true,
            Ty::Num(crate::Num::F64) => false,
            _ => {
                self.report(
                    Code::E0226,
                    range,
                    "`parse_quantity` gives an `f64` or `int` quantity",
                );
                return Some(Ty::Error);
            }
        };
        let Some(dim) = self.items.dim_of(&to).filter(|_| !to.is_one()) else {
            self.report(
                Code::E0227,
                range,
                "`parse_quantity` gives a quantity of a unit written out",
            );
            return Some(Ty::Error);
        };
        let mut table = String::new();
        let mut seen = std::collections::HashSet::new();
        for (i, def) in self.items.units.iter().enumerate() {
            if def.dim != dim || !seen.insert(def.name.clone()) {
                continue;
            }
            let from = UnitTy::atom(UnitAtom::Named(i as u32, def.name.as_str().into()));
            let conv = self.items.conversion(&from, &to, true);
            let line = parse_line(&def.name, conv, int);
            if let Some(l) = line {
                table.push_str(&l);
            }
        }
        let err = self.prelude_adt("UnitParseError", Vec::new());
        self.out.unit_parses.insert(e, (table, int));
        Some(self.prelude_adt("Result", vec![q, err]))
    }
}

/// A unit's line of a `parse_quantity` table (`name,kind,factor,offset`): its value multiplied
/// (`m`) or divided (`d`) by a whole factor, or multiplied by a float (`f`), then the offset, if
/// any, added; as the conversion does (`unit.conv.2`). An `int`'s, only if it converts by a
/// whole factor and offset.
fn parse_line(name: &str, conv: crate::units::UnitConv, int: bool) -> Option<String> {
    let (kind, factor) = match conv.factor {
        Factor::Exact(n, 1) => ("m", n.to_string()),
        _ if int => return None,
        Factor::Exact(1, d) => ("d", d.to_string()),
        f => ("f", format!("{:?}", f.to_f64())),
    };
    let offset = match conv.offset {
        None => String::new(),
        Some(o) if int => o.as_int()?.to_string(),
        Some(o) => format!("{:?}", o.to_f64()),
    };
    Some(format!("{name},{kind},{factor},{offset}\n"))
}
