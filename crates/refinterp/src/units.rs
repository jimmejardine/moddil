//! Units of measure (`unit.*`), carried at run time: a quantity is a number with its unit
//! ([`Value::Quantity`]), and a value meeting a type with a unit is converted to it there. The
//! compiler erases units and folds each conversion into a constant; this keeps them, as an
//! independent check of where conversions happen and what they give.
//!
//! The rules are the reference's: `+`, `-`, `%` and comparisons take one dimension, in the unit
//! a type gives the sum, or else the finer of the two; `*` and `/` combine units, and one with
//! no dimension left is a plain number; a value converts by its scale ratio, multiplied (a float
//! divided, when the ratio is one over a whole number), then an affine unit's offset added.

use std::collections::BTreeMap;
use std::sync::Arc;

use moddil_syntax::{SyntaxKind as K, SyntaxNode, TextRange};

use crate::Eval;

use crate::value::Value;

/// A rational number, reduced, its denominator positive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct Ratio {
    pub(crate) n: i128,
    pub(crate) d: i128,
}

fn gcd(a: i128, b: i128) -> i128 {
    let (mut a, mut b) = (a.abs(), b.abs());
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

impl Ratio {
    pub(crate) const ONE: Ratio = Ratio { n: 1, d: 1 };
    pub(crate) const ZERO: Ratio = Ratio { n: 0, d: 1 };

    pub(crate) fn new(n: i128, d: i128) -> Option<Ratio> {
        if d == 0 {
            return None;
        }
        let g = gcd(n, d).max(1);
        let (n, d) = (n / g, d / g);
        if d < 0 {
            Some(Ratio {
                n: n.checked_neg()?,
                d: d.checked_neg()?,
            })
        } else {
            Some(Ratio { n, d })
        }
    }

    fn add(self, o: Ratio) -> Option<Ratio> {
        Ratio::new(
            self.n
                .checked_mul(o.d)?
                .checked_add(o.n.checked_mul(self.d)?)?,
            self.d.checked_mul(o.d)?,
        )
    }

    fn mul(self, o: Ratio) -> Option<Ratio> {
        let a = gcd(self.n, o.d).max(1);
        let b = gcd(o.n, self.d).max(1);
        Ratio::new(
            (self.n / a).checked_mul(o.n / b)?,
            (self.d / b).checked_mul(o.d / a)?,
        )
    }

    fn div(self, o: Ratio) -> Option<Ratio> {
        if o.n == 0 {
            return None;
        }
        self.mul(Ratio::new(o.d, o.n)?)
    }

    fn to_f64(self) -> f64 {
        self.n as f64 / self.d as f64
    }

    fn pow(self, e: Ratio) -> Option<Ratio> {
        let base = if e.n < 0 {
            Ratio::new(self.d, self.n)?
        } else {
            self
        };
        let mut r = Ratio::ONE;
        for _ in 0..u32::try_from(e.n.unsigned_abs()).ok()? {
            r = r.mul(base)?;
        }
        if e.d == 1 {
            return Some(r);
        }
        let k = u32::try_from(e.d).ok()?;
        Ratio::new(root(r.n, k)?, root(r.d, k)?)
    }

    /// A decimal number's exact value.
    fn decimal(text: &str) -> Option<Ratio> {
        let t: String = text.chars().filter(|&c| c != '_').collect();
        let (mant, exp) = match t.find(['e', 'E']) {
            Some(i) => (&t[..i], t[i + 1..].parse::<i32>().ok()?),
            None => (t.as_str(), 0),
        };
        let (int, frac) = mant.split_once('.').unwrap_or((mant, ""));
        let mut n: i128 = 0;
        for c in int.chars().chain(frac.chars()) {
            n = n
                .checked_mul(10)?
                .checked_add(i128::from(c.to_digit(10)?))?;
        }
        let shift = exp - frac.len() as i32;
        if shift >= 0 {
            Ratio::new(n.checked_mul(10i128.checked_pow(shift as u32)?)?, 1)
        } else {
            Ratio::new(n, 10i128.checked_pow(shift.unsigned_abs())?)
        }
    }
}

fn root(n: i128, k: u32) -> Option<i128> {
    if n < 0 {
        return if k % 2 == 1 {
            root(-n, k).map(|r| -r)
        } else {
            None
        };
    }
    let g = (n as f64).powf(1.0 / f64::from(k)).round() as i128;
    (g.saturating_sub(1)..=g + 1).find(|r| r.checked_pow(k) == Some(n))
}

/// A scale: exact, or, past what 128 bits hold or for an irrational root, a float.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Sc {
    Exact(Ratio),
    Approx(f64),
}

impl Sc {
    const ONE: Sc = Sc::Exact(Ratio::ONE);

    fn f(self) -> f64 {
        match self {
            Sc::Exact(r) => r.to_f64(),
            Sc::Approx(x) => x,
        }
    }

    fn mul(self, o: Sc) -> Sc {
        match (self, o) {
            (Sc::Exact(a), Sc::Exact(b)) => a
                .mul(b)
                .map_or(Sc::Approx(a.to_f64() * b.to_f64()), Sc::Exact),
            (a, b) => Sc::Approx(a.f() * b.f()),
        }
    }

    fn div(self, o: Sc) -> Sc {
        match (self, o) {
            (Sc::Exact(a), Sc::Exact(b)) => a
                .div(b)
                .map_or(Sc::Approx(a.to_f64() / b.to_f64()), Sc::Exact),
            (a, b) => Sc::Approx(a.f() / b.f()),
        }
    }

    fn add(self, o: Sc) -> Sc {
        match (self, o) {
            (Sc::Exact(a), Sc::Exact(b)) => a
                .add(b)
                .map_or(Sc::Approx(a.to_f64() + b.to_f64()), Sc::Exact),
            (a, b) => Sc::Approx(a.f() + b.f()),
        }
    }

    fn neg(self) -> Sc {
        match self {
            Sc::Exact(r) => Sc::Exact(Ratio { n: -r.n, d: r.d }),
            Sc::Approx(x) => Sc::Approx(-x),
        }
    }

    fn pow(self, e: Ratio) -> Sc {
        match self {
            Sc::Exact(r) => r
                .pow(e)
                .map_or(Sc::Approx(r.to_f64().powf(e.to_f64())), Sc::Exact),
            Sc::Approx(x) => Sc::Approx(x.powf(e.to_f64())),
        }
    }

    fn is_zero(self) -> bool {
        self.f() == 0.0
    }
}

/// A unit at run time: its units, by key, to their (nonzero) powers, sorted. Empty is one.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub(crate) struct DynUnit(pub(crate) Vec<(String, Ratio)>);

impl DynUnit {
    pub(crate) fn one() -> DynUnit {
        DynUnit(Vec::new())
    }

    pub(crate) fn of(key: &str) -> DynUnit {
        DynUnit(vec![(key.to_string(), Ratio::ONE)])
    }

    pub(crate) fn is_one(&self) -> bool {
        self.0.is_empty()
    }

    /// `self · o^k`.
    pub(crate) fn mul_pow(&self, o: &DynUnit, k: Ratio) -> DynUnit {
        let mut m: BTreeMap<String, Ratio> = self.0.iter().cloned().collect();
        for (u, e) in &o.0 {
            let v = m.get(u).copied().unwrap_or(Ratio::ZERO);
            let v = e.mul(k).and_then(|x| v.add(x)).unwrap_or(v);
            m.insert(u.clone(), v);
        }
        DynUnit(m.into_iter().filter(|(_, e)| e.n != 0).collect())
    }

    pub(crate) fn mul(&self, o: &DynUnit) -> DynUnit {
        self.mul_pow(o, Ratio::ONE)
    }

    pub(crate) fn div(&self, o: &DynUnit) -> DynUnit {
        self.mul_pow(o, Ratio { n: -1, d: 1 })
    }

    pub(crate) fn pow(&self, k: Ratio) -> DynUnit {
        DynUnit::mul_pow(&DynUnit::one(), self, k)
    }
}

/// What a declared unit is: its dimension (base dimensions, by key, to their powers), how many
/// of its dimension's base unit it is, and an affine unit's zero in that base unit.
#[derive(Clone, Debug)]
pub(crate) struct UnitInfo {
    pub(crate) dims: BTreeMap<String, Ratio>,
    pub(crate) scale: Sc,
    pub(crate) offset: Option<Sc>,
}

/// A unit expression's exponent: `2`, `-1`, `(1/2)`.
pub(crate) fn exponent(node: &SyntaxNode) -> Option<Ratio> {
    let neg = node.child_tokens().any(|t| t.kind() == K::MINUS);
    let nums: Vec<i128> = node
        .child_tokens()
        .filter(|t| t.kind() == K::INTEGER)
        .filter_map(|t| t.text().replace('_', "").parse().ok())
        .collect();
    let (n, d) = match nums.as_slice() {
        [n] => (*n, 1),
        [n, d] => (*n, *d),
        _ => return None,
    };
    Ratio::new(if neg { -n } else { n }, d)
}

/// A value multiplied by `o^k`, for a unit expression's walk.
pub(crate) trait Prod: Sized {
    fn one() -> Self;
    fn mul_pow(self, o: Self, k: Ratio) -> Self;
}

impl Prod for DynUnit {
    fn one() -> Self {
        DynUnit::one()
    }
    fn mul_pow(self, o: Self, k: Ratio) -> Self {
        DynUnit::mul_pow(&self, &o, k)
    }
}

impl Prod for (BTreeMap<String, Ratio>, Sc) {
    fn one() -> Self {
        (BTreeMap::new(), Sc::ONE)
    }
    fn mul_pow(self, o: Self, k: Ratio) -> Self {
        let mut d = self.0;
        for (b, e) in o.0 {
            let v = d.get(&b).copied().unwrap_or(Ratio::ZERO);
            let v = e.mul(k).and_then(|x| v.add(x)).unwrap_or(v);
            if v.n == 0 {
                d.remove(&b);
            } else {
                d.insert(b, v);
            }
        }
        (d, self.1.mul(o.1.pow(k)))
    }
}

/// The value of unit expression `node`, each atom (a path, or a number) being what `atom`
/// gives.
pub(crate) fn walk<V: Prod>(node: &SyntaxNode, atom: &mut dyn FnMut(&SyntaxNode) -> V) -> V {
    let parts: Vec<SyntaxNode> = node.children().collect();
    match node.kind() {
        K::UNIT_BIN => {
            let k = if node.token(K::SLASH).is_some() {
                Ratio { n: -1, d: 1 }
            } else {
                Ratio::ONE
            };
            let l = walk(&parts[0], atom);
            let r = walk(&parts[1], atom);
            l.mul_pow(r, k)
        }
        K::UNIT_POWER => {
            let b = walk(&parts[0], atom);
            let e = node
                .child(K::EXPONENT)
                .and_then(|x| exponent(&x))
                .unwrap_or(Ratio::ONE);
            V::one().mul_pow(b, e)
        }
        K::UNIT_PAREN | K::UNIT_SUFFIX => walk(&parts[0], atom),
        _ => atom(node),
    }
}

/// The names of a unit atom's path.
pub(crate) fn atom_path(node: &SyntaxNode) -> Vec<String> {
    node.child(K::PATH)
        .into_iter()
        .flat_map(|p| p.children().collect::<Vec<_>>())
        .filter(|c| c.kind() == K::PATH_SEGMENT)
        .filter_map(|c| c.token(K::IDENT).map(|t| t.text().to_string()))
        .collect()
}

/// A number's literal value, for a unit's definition.
pub(crate) fn decimal(text: &str) -> Option<Ratio> {
    Ratio::decimal(text)
}

/// The units declared in a program, for working out what they are.
pub(crate) trait Units {
    fn info(&self, key: &str) -> UnitInfo;
    /// The units, in the order they are declared.
    fn order(&self) -> Vec<String>;
}

/// `u`'s dimension.
pub(crate) fn dims(p: &dyn Units, u: &DynUnit) -> BTreeMap<String, Ratio> {
    let mut d: BTreeMap<String, Ratio> = BTreeMap::new();
    for (k, e) in &u.0 {
        for (b, x) in p.info(k).dims {
            let v = d.get(&b).copied().unwrap_or(Ratio::ZERO);
            let v = x.mul(*e).and_then(|y| v.add(y)).unwrap_or(v);
            if v.n == 0 {
                d.remove(&b);
            } else {
                d.insert(b, v);
            }
        }
    }
    d
}

/// `u`'s scale.
pub(crate) fn scale(p: &dyn Units, u: &DynUnit) -> Sc {
    u.0.iter()
        .fold(Sc::ONE, |s, (k, e)| s.mul(p.info(k).scale.pow(*e)))
}

fn offset(p: &dyn Units, u: &DynUnit) -> Option<Sc> {
    match u.0.as_slice() {
        [(k, e)] if *e == Ratio::ONE => p.info(k).offset,
        _ => None,
    }
}

/// Whether `u` has an affine unit in it.
pub(crate) fn affine(p: &dyn Units, u: &DynUnit) -> bool {
    u.0.iter().any(|(k, _)| p.info(k).offset.is_some())
}

/// Whether `a` and `b` are of one dimension.
pub(crate) fn same_dim(p: &dyn Units, a: &DynUnit, b: &DynUnit) -> bool {
    dims(p, &a.div(b)).is_empty()
}

/// Whether `a` is the finer of two units of one dimension.
pub(crate) fn finer(p: &dyn Units, a: &DynUnit, b: &DynUnit) -> bool {
    let (x, y) = (scale(p, a).f().abs(), scale(p, b).f().abs());
    x < y || (x == y && order_of(p, a) <= order_of(p, b))
}

fn order_of(p: &dyn Units, u: &DynUnit) -> Vec<usize> {
    let order = p.order();
    u.0.iter()
        .map(|(k, _)| order.iter().position(|o| o == k).unwrap_or(usize::MAX))
        .collect()
}

/// The base unit of `u`'s dimension: of scale one and not affine, the first declared.
pub(crate) fn base_unit(p: &dyn Units, u: &DynUnit) -> Option<DynUnit> {
    let d = dims(p, u);
    p.order().into_iter().find_map(|k| {
        let i = p.info(&k);
        (i.dims == d && i.offset.is_none() && i.scale == Sc::ONE).then(|| DynUnit::of(&k))
    })
}

/// The value `v` (a number) in unit `from`, in unit `to`: of a point, applying offsets. An
/// integer that overflows traps, at `range`.
pub(crate) fn convert(
    p: &dyn Units,
    v: Value,
    from: &DynUnit,
    to: &DynUnit,
    point: bool,
    range: TextRange,
) -> Eval {
    if from == to {
        return Ok(v);
    }
    let v = settled(v);
    let factor = scale(p, from).div(scale(p, to));
    let off = if point {
        let zero = Sc::Exact(Ratio::ZERO);
        let a = offset(p, from).unwrap_or(zero);
        let b = offset(p, to).unwrap_or(zero);
        let o = a.add(b.neg()).div(scale(p, to));
        (!o.is_zero()).then_some(o)
    } else {
        None
    };
    let v = scaled(v, factor, range)?;
    match off {
        Some(o) => {
            let c = constant(&v, o);
            add(v, c, range)
        }
        None => Ok(v),
    }
}

fn add(v: Value, c: Value, range: TextRange) -> Eval {
    match &v {
        Value::Int(_) => crate::machine::int_arith(K::PLUS, v, c, range),
        _ => crate::num::arith("+", v, c, range),
    }
}

/// `v` scaled by `f`: an integer multiplied, a float multiplied, or divided when `f` is one over
/// a whole number.
fn scaled(v: Value, f: Sc, range: TextRange) -> Eval {
    if f == Sc::ONE {
        return Ok(v);
    }
    let v = settled(v);
    let float = matches!(&v, Value::Num(k, _) if k.is_float());
    let (op, c) = match f {
        Sc::Exact(r) if float && r.n == 1 && r.d > 1 => (
            "/",
            constant(&v, Sc::Exact(Ratio::new(r.d, 1).unwrap_or(r))),
        ),
        f => ("*", constant(&v, f)),
    };
    match &v {
        Value::Int(_) => {
            let k = if op == "*" { K::STAR } else { K::SLASH };
            crate::machine::int_arith(k, v, c, range)
        }
        _ => crate::num::arith(op, v, c, range),
    }
}

/// A literal's value as its type will be: a fractional literal an `f64`.
pub(crate) fn settled(v: Value) -> Value {
    match v {
        Value::FloatLit(_) | Value::BigLit(_) | Value::HugeLit(_) => crate::num::settle(v),
        v => v,
    }
}

/// `f` as a number of `v`'s type.
fn constant(v: &Value, f: Sc) -> Value {
    match v {
        Value::Num(k, _) if k.is_float() => crate::num::float_value(*k, f.f()),
        Value::Num(k, _) => match f {
            Sc::Exact(r) => crate::num::int_value(*k, r.n).unwrap_or(Value::Int(r.n as i64)),
            Sc::Approx(x) => Value::Int(x as i64),
        },
        _ => match f {
            Sc::Exact(r) => Value::Int(r.n as i64),
            Sc::Approx(x) => Value::Int(x as i64),
        },
    }
}

/// `v` as a quantity of unit `u`: a plain number if `u` is one.
pub(crate) fn quantity(v: Value, u: DynUnit) -> Value {
    if u.is_one() {
        return v;
    }
    match v {
        Value::Quantity(inner, w) => quantity(*inner, w.mul(&u)),
        v => Value::Quantity(Box::new(v), Arc::new(u)),
    }
}

/// A value's number and unit.
pub(crate) fn split(v: Value) -> (Value, DynUnit) {
    match v {
        Value::Quantity(inner, u) => (*inner, (*u).clone()),
        v => (v, DynUnit::one()),
    }
}

/// A literal's value as its partner's type: a fractional literal beside an `f32` is one, and
/// otherwise as nothing gives it a type.
pub(crate) fn settled_like(v: Value, partner: &Value) -> Value {
    match (&v, partner) {
        (Value::FloatLit(_) | Value::Int(_) | Value::BigLit(_), Value::Num(k, _)) => {
            crate::num::as_kind(v, *k)
        }
        (Value::Int(_), Value::FloatLit(_)) => crate::num::as_kind(v, crate::num::Kind::F64),
        (Value::Int(_), _) => v,
        _ => settled(v),
    }
}

/// The exact ratio a value in `from` is multiplied by in `to`: `(n, d)`.
pub(crate) fn ratio_of(p: &dyn Units, from: &DynUnit, to: &DynUnit) -> (i128, i128) {
    match scale(p, from).div(scale(p, to)) {
        Sc::Exact(r) => (r.n, r.d),
        Sc::Approx(_) => (1, 1),
    }
}

/// A unit's line of the table `parse_quantity` reads (`name,kind,factor,offset`), for a value
/// in `from` converted to `to`: multiplied (`m`) or divided (`d`) by a whole number, or
/// multiplied by a float (`f`), then its offset added. An `int`'s only if both are whole.
pub(crate) fn parse_line(
    p: &dyn Units,
    name: &str,
    from: &DynUnit,
    to: &DynUnit,
    int: bool,
) -> Option<String> {
    let f = scale(p, from).div(scale(p, to));
    let (kind, factor) = match f {
        Sc::Exact(r) if r.d == 1 => ("m", r.n.to_string()),
        _ if int => return None,
        Sc::Exact(r) if r.n == 1 => ("d", r.d.to_string()),
        f => ("f", format!("{:?}", f.f())),
    };
    let zero = Sc::Exact(Ratio::ZERO);
    let o = offset(p, from)
        .unwrap_or(zero)
        .add(offset(p, to).unwrap_or(zero).neg())
        .div(scale(p, to));
    let off = if o.is_zero() {
        String::new()
    } else {
        match o {
            Sc::Exact(r) if int && r.d == 1 => r.n.to_string(),
            _ if int => return None,
            o => format!("{:?}", o.f()),
        }
    };
    Some(format!("{name},{kind},{factor},{off}\n"))
}
