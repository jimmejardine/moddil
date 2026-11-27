//! Units of measure (`unit.*`, `specs/units.md`): the algebra the type checker works with.
//!
//! A unit in a type is a product of named units, unit parameters and unit inference variables,
//! each to a rational power ([`UnitTy`]). A unit's dimension is a product of declared base
//! dimensions to rational powers ([`Dim`]), and its scale is how many of its dimension's base
//! unit it is ([`Scale`]). Units exist only while a program is checked: they are erased before
//! MIR, and a conversion between two units of one dimension is a multiplication by a constant.

#![allow(clippy::should_implement_trait)]

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

/// A rational number, reduced, with a positive denominator: an exponent, or an exact scale.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct Ratio {
    /// The numerator.
    pub num: i128,
    /// The denominator, positive.
    pub den: i128,
}

fn gcd(mut a: i128, mut b: i128) -> i128 {
    a = a.abs();
    b = b.abs();
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

impl Ratio {
    /// One.
    pub const ONE: Ratio = Ratio { num: 1, den: 1 };
    /// Zero.
    pub const ZERO: Ratio = Ratio { num: 0, den: 1 };

    /// `num / den`, reduced; `None` if `den` is zero.
    pub fn new(num: i128, den: i128) -> Option<Ratio> {
        if den == 0 {
            return None;
        }
        let g = gcd(num, den).max(1);
        let (mut n, mut d) = (num / g, den / g);
        if d < 0 {
            n = n.checked_neg()?;
            d = d.checked_neg()?;
        }
        Some(Ratio { num: n, den: d })
    }

    /// The integer `n`.
    pub fn int(n: i128) -> Ratio {
        Ratio { num: n, den: 1 }
    }

    /// Whether it is zero.
    pub fn is_zero(self) -> bool {
        self.num == 0
    }

    /// Whether it is an integer.
    pub fn is_int(self) -> bool {
        self.den == 1
    }

    /// The sum, if it does not overflow.
    pub fn add(self, o: Ratio) -> Option<Ratio> {
        let d = self.den.checked_mul(o.den)?;
        let n = self
            .num
            .checked_mul(o.den)?
            .checked_add(o.num.checked_mul(self.den)?)?;
        Ratio::new(n, d)
    }

    /// The difference, if it does not overflow.
    pub fn sub(self, o: Ratio) -> Option<Ratio> {
        self.add(Ratio {
            num: o.num.checked_neg()?,
            den: o.den,
        })
    }

    /// The product, if it does not overflow.
    pub fn mul(self, o: Ratio) -> Option<Ratio> {
        let a = gcd(self.num, o.den).max(1);
        let b = gcd(o.num, self.den).max(1);
        Ratio::new(
            (self.num / a).checked_mul(o.num / b)?,
            (self.den / b).checked_mul(o.den / a)?,
        )
    }

    /// The quotient, if `o` is not zero and it does not overflow.
    pub fn div(self, o: Ratio) -> Option<Ratio> {
        if o.num == 0 {
            return None;
        }
        self.mul(Ratio::new(o.den, o.num)?)
    }

    /// It as a float.
    pub fn to_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }

    /// It raised to the power `e`, exactly if it can be (an integer power, or a root that is
    /// exact).
    pub fn pow(self, e: Ratio) -> Option<Ratio> {
        let base = if e.num < 0 {
            Ratio::new(self.den, self.num)?
        } else {
            self
        };
        let n = u32::try_from(e.num.unsigned_abs()).ok()?;
        let mut r = Ratio::ONE;
        for _ in 0..n {
            r = r.mul(base)?;
        }
        if e.den == 1 {
            return Some(r);
        }
        let k = u32::try_from(e.den).ok()?;
        Ratio::new(exact_root(r.num, k)?, exact_root(r.den, k)?)
    }

    /// A decimal number's exact value: `0.3048`, `273.15`, `1e-3`, `6.02214076e23`.
    pub fn parse_decimal(text: &str) -> Option<Ratio> {
        let t = text.replace('_', "");
        let (mant, exp) = match t.find(['e', 'E']) {
            Some(i) => (&t[..i], t[i + 1..].parse::<i32>().ok()?),
            None => (t.as_str(), 0),
        };
        let (int, frac) = match mant.find('.') {
            Some(i) => (&mant[..i], &mant[i + 1..]),
            None => (mant, ""),
        };
        let digits = format!("{int}{frac}");
        let mut n: i128 = 0;
        for c in digits.chars() {
            n = n
                .checked_mul(10)?
                .checked_add(i128::from(c.to_digit(10)?))?;
        }
        let shift = exp - frac.len() as i32;
        let ten = |k: u32| 10i128.checked_pow(k);
        if shift >= 0 {
            Ratio::new(n.checked_mul(ten(shift as u32)?)?, 1)
        } else {
            Ratio::new(n, ten(shift.unsigned_abs())?)
        }
    }
}

/// The exact `k`th root of `n`, if it is an integer.
fn exact_root(n: i128, k: u32) -> Option<i128> {
    if n < 0 {
        return if k % 2 == 1 {
            exact_root(-n, k).map(|r| -r)
        } else {
            None
        };
    }
    let guess = (n as f64).powf(1.0 / f64::from(k)).round() as i128;
    (guess.saturating_sub(1)..=guess + 1).find(|r| r.checked_pow(k) == Some(n))
}

impl fmt::Display for Ratio {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.den == 1 {
            write!(f, "{}", self.num)
        } else {
            write!(f, "{}/{}", self.num, self.den)
        }
    }
}

/// How many of its dimension's base unit a unit is: exactly, or (an irrational root, or past
/// what 128 bits hold) as a float.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Scale {
    /// Exactly.
    Exact(Ratio),
    /// As a float, only floats convert through it.
    Approx(f64),
}

impl Scale {
    /// One.
    pub const ONE: Scale = Scale::Exact(Ratio::ONE);

    /// It as a float.
    pub fn to_f64(self) -> f64 {
        match self {
            Scale::Exact(r) => r.to_f64(),
            Scale::Approx(x) => x,
        }
    }

    /// The product.
    pub fn mul(self, o: Scale) -> Scale {
        match (self, o) {
            (Scale::Exact(a), Scale::Exact(b)) => match a.mul(b) {
                Some(r) => Scale::Exact(r),
                None => Scale::Approx(a.to_f64() * b.to_f64()),
            },
            (a, b) => Scale::Approx(a.to_f64() * b.to_f64()),
        }
    }

    /// The quotient.
    pub fn div(self, o: Scale) -> Scale {
        match (self, o) {
            (Scale::Exact(a), Scale::Exact(b)) => match a.div(b) {
                Some(r) => Scale::Exact(r),
                None => Scale::Approx(a.to_f64() / b.to_f64()),
            },
            (a, b) => Scale::Approx(a.to_f64() / b.to_f64()),
        }
    }

    /// It to the power `e`.
    pub fn pow(self, e: Ratio) -> Scale {
        match self {
            Scale::Exact(r) => match r.pow(e) {
                Some(r) => Scale::Exact(r),
                None => Scale::Approx(r.to_f64().powf(e.to_f64())),
            },
            Scale::Approx(x) => Scale::Approx(x.powf(e.to_f64())),
        }
    }

    /// The sum: offsets add.
    pub fn add(self, o: Scale) -> Scale {
        match (self, o) {
            (Scale::Exact(a), Scale::Exact(b)) => match a.add(b) {
                Some(r) => Scale::Exact(r),
                None => Scale::Approx(a.to_f64() + b.to_f64()),
            },
            (a, b) => Scale::Approx(a.to_f64() + b.to_f64()),
        }
    }

    /// The difference.
    pub fn sub(self, o: Scale) -> Scale {
        self.add(o.neg())
    }

    /// Its negation.
    pub fn neg(self) -> Scale {
        match self {
            Scale::Exact(r) => Scale::Exact(Ratio {
                num: -r.num,
                den: r.den,
            }),
            Scale::Approx(x) => Scale::Approx(-x),
        }
    }

    /// Whether it is zero.
    pub fn is_zero(self) -> bool {
        match self {
            Scale::Exact(r) => r.is_zero(),
            Scale::Approx(x) => x == 0.0,
        }
    }

    /// Its absolute value, for comparing which of two units is the finer.
    pub fn magnitude(self) -> f64 {
        self.to_f64().abs()
    }
}

/// A dimension: declared base dimensions (by index among the `dimension` items) to their
/// powers. One is the empty map: a dimensionless quantity is a plain number.
pub type Dim = BTreeMap<u32, Ratio>;

/// Multiplies `d` by `e` to the power `k`.
pub fn dim_mul(d: &mut Dim, e: &Dim, k: Ratio) -> Option<()> {
    for (b, x) in e {
        let v = d.get(b).copied().unwrap_or(Ratio::ZERO).add(x.mul(k)?)?;
        if v.is_zero() {
            d.remove(b);
        } else {
            d.insert(*b, v);
        }
    }
    Some(())
}

/// A part of a unit in a type.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum UnitAtom {
    /// A declared unit, by its index among the `unit` items, with its name.
    Named(u32, Arc<str>),
    /// A unit parameter of the item being checked (`unit.param.1`), as a type parameter's index.
    Param(u32, Arc<str>),
    /// A unit being inferred, by its variable's index in the checker's table.
    Infer(u32),
}

/// A unit in a type: its parts to their (nonzero) powers, sorted, so two units are the same
/// exactly when their `UnitTy`s are equal. The empty product is one: dimensionless.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct UnitTy(Arc<[(UnitAtom, Ratio)]>);

impl UnitTy {
    /// One: no unit.
    pub fn one() -> UnitTy {
        UnitTy(Arc::new([]))
    }

    /// `a` to the first power.
    pub fn atom(a: UnitAtom) -> UnitTy {
        UnitTy(Arc::new([(a, Ratio::ONE)]))
    }

    /// The parts and their powers.
    pub fn parts(&self) -> &[(UnitAtom, Ratio)] {
        &self.0
    }

    /// Whether it is one.
    pub fn is_one(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether it has a parameter or a variable in it.
    pub fn has_vars(&self) -> bool {
        self.0
            .iter()
            .any(|(a, _)| matches!(a, UnitAtom::Param(..) | UnitAtom::Infer(_)))
    }

    /// Whether it has a variable in it.
    pub fn has_infer(&self) -> bool {
        self.0.iter().any(|(a, _)| matches!(a, UnitAtom::Infer(_)))
    }

    /// Whether it has a declared unit in it.
    pub fn has_named(&self) -> bool {
        self.0.iter().any(|(a, _)| matches!(a, UnitAtom::Named(..)))
    }

    fn from_map(m: BTreeMap<UnitAtom, Ratio>) -> UnitTy {
        UnitTy(m.into_iter().filter(|(_, e)| !e.is_zero()).collect())
    }

    fn map_of(&self) -> BTreeMap<UnitAtom, Ratio> {
        self.0.iter().cloned().collect()
    }

    /// The product `self · other^k`, or `None` if an exponent overflows.
    pub fn mul_pow(&self, other: &UnitTy, k: Ratio) -> Option<UnitTy> {
        let mut m = self.map_of();
        for (a, e) in other.0.iter() {
            let v = m.get(a).copied().unwrap_or(Ratio::ZERO).add(e.mul(k)?)?;
            m.insert(a.clone(), v);
        }
        Some(UnitTy::from_map(m))
    }

    /// The product.
    pub fn mul(&self, other: &UnitTy) -> UnitTy {
        self.mul_pow(other, Ratio::ONE)
            .unwrap_or_else(|| self.clone())
    }

    /// The quotient.
    pub fn div(&self, other: &UnitTy) -> UnitTy {
        self.mul_pow(other, Ratio::int(-1))
            .unwrap_or_else(|| self.clone())
    }

    /// It to the power `k`.
    pub fn pow(&self, k: Ratio) -> UnitTy {
        UnitTy::one()
            .mul_pow(self, k)
            .unwrap_or_else(|| self.clone())
    }

    /// It without part `a`, and `a`'s power in it.
    pub fn without(&self, a: &UnitAtom) -> (UnitTy, Ratio) {
        let mut m = self.map_of();
        let e = m.remove(a).unwrap_or(Ratio::ZERO);
        (UnitTy::from_map(m), e)
    }

    /// It with each parameter and variable replaced by what `f` gives for it (`None`: kept).
    pub fn replace(&self, f: &mut impl FnMut(&UnitAtom) -> Option<UnitTy>) -> UnitTy {
        let mut out = UnitTy::one();
        for (a, e) in self.0.iter() {
            let part = match a {
                UnitAtom::Named(..) => None,
                _ => f(a),
            }
            .unwrap_or_else(|| UnitTy::atom(a.clone()));
            out = out.mul_pow(&part, *e).unwrap_or(out);
        }
        out
    }
}

impl fmt::Debug for UnitTy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            return f.write_str("1");
        }
        let name = |a: &UnitAtom| match a {
            UnitAtom::Named(_, n) | UnitAtom::Param(_, n) => n.to_string(),
            UnitAtom::Infer(_) => "_".to_string(),
        };
        let power = |e: Ratio| {
            if e == Ratio::ONE {
                String::new()
            } else if e.is_int() {
                format!("^{}", e.num)
            } else {
                format!("^({e})")
            }
        };
        let up: Vec<String> = self
            .0
            .iter()
            .filter(|(_, e)| e.num > 0)
            .map(|(a, e)| format!("{}{}", name(a), power(*e)))
            .collect();
        let down: Vec<String> = self
            .0
            .iter()
            .filter(|(_, e)| e.num < 0)
            .map(|(a, e)| {
                format!(
                    "{}{}",
                    name(a),
                    power(Ratio {
                        num: -e.num,
                        den: e.den
                    })
                )
            })
            .collect();
        let top = if up.is_empty() {
            "1".to_string()
        } else {
            up.join("*")
        };
        match down.len() {
            0 => f.write_str(&top),
            1 => write!(f, "{top}/{}", down[0]),
            _ => write!(f, "{top}/({})", down.join("*")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(i: u32, n: &str) -> UnitTy {
        UnitTy::atom(UnitAtom::Named(i, n.into()))
    }

    #[test]
    fn powers_normalise() {
        let m = unit(0, "m");
        let half = Ratio::new(1, 2).unwrap();
        assert_eq!(m.pow(half).mul(&m.pow(half)), m);
        assert!(m.div(&m).is_one());
        let s = unit(1, "s");
        assert_eq!(format!("{:?}", m.div(&s.pow(Ratio::int(2)))), "m/s^2");
    }

    #[test]
    fn decimals_are_exact() {
        assert_eq!(Ratio::parse_decimal("0.3048"), Ratio::new(381, 1250));
        assert_eq!(Ratio::parse_decimal("1e-3"), Ratio::new(1, 1000));
        assert_eq!(Ratio::parse_decimal("273.15"), Ratio::new(5463, 20));
    }

    #[test]
    fn roots_are_exact_where_they_can_be() {
        let r = Ratio::int(1_000_000);
        assert_eq!(r.pow(Ratio::new(1, 2).unwrap()), Some(Ratio::int(1000)));
        assert_eq!(Ratio::int(2).pow(Ratio::new(1, 2).unwrap()), None);
    }
}

impl crate::items::ItemTree {
    /// The tree with every unit erased (`unit.erase.1`): what MIR and the implementations see.
    pub fn erase_units(&self) -> crate::items::ItemTree {
        use crate::Ty;
        use crate::items::{Generics, Pred};
        use crate::ty::TraitRef;
        let e = |t: &Ty| t.erase_units();
        let tr = |t: &TraitRef| t.map(&mut |x| x.erase_units());
        let pred = |p: &Pred| Pred {
            ty: e(&p.ty),
            trait_ref: tr(&p.trait_ref),
            assoc: p.assoc.iter().map(|(i, t)| (*i, e(t))).collect(),
            control: p.control,
            range: p.range,
        };
        let generics = |g: &Generics| {
            let mut g = g.clone();
            for p in &mut g.params {
                p.default = p.default.as_ref().map(e);
            }
            g.preds = g.preds.iter().map(pred).collect();
            g
        };
        let mut out = self.clone();
        for f in &mut out.fns {
            f.generics = generics(&f.generics);
            for p in &mut f.params {
                p.ty = e(&p.ty);
            }
            f.ret = e(&f.ret);
            f.fails = e(&f.fails);
            for x in &mut f.effects {
                x.ty = e(&x.ty);
            }
            f.opaque = f.opaque.iter().map(pred).collect();
            f.opaque_fn = f.opaque_fn.as_ref().map(e);
            if let Some(t) = &mut f.template {
                t.ty = e(&t.ty);
            }
        }
        for a in &mut out.adts {
            a.generics = generics(&a.generics);
            for v in &mut a.variants {
                for fd in &mut v.fields {
                    fd.ty = e(&fd.ty);
                }
            }
        }
        for t in &mut out.types {
            t.ty = e(&t.ty);
            for p in &mut t.params {
                p.default = p.default.as_ref().map(e);
            }
        }
        for t in &mut out.traits {
            t.generics = generics(&t.generics);
            t.supertraits = t.supertraits.iter().map(tr).collect();
            t.assoc_defaults = t.assoc_defaults.iter().map(|x| x.as_ref().map(e)).collect();
        }
        for i in &mut out.impls {
            i.generics = generics(&i.generics);
            i.trait_ref = i.trait_ref.as_ref().map(tr);
            i.self_ty = e(&i.self_ty);
            i.assoc = i.assoc.iter().map(|x| x.as_ref().map(e)).collect();
        }
        out.has_quantities = false;
        out
    }

    /// Whether a type in the tree has a unit in it.
    pub(crate) fn any_units(&self) -> bool {
        let fns = self.fns.iter().any(|f| {
            f.params.iter().any(|p| p.ty.has_units())
                || f.ret.has_units()
                || f.generics.preds.iter().any(|p| p.ty.has_units())
        });
        let adts = self
            .adts
            .iter()
            .flat_map(|a| &a.variants)
            .flat_map(|v| &v.fields)
            .any(|f| f.ty.has_units());
        let rest = self.types.iter().any(|t| t.ty.has_units())
            || self
                .impls
                .iter()
                .any(|i| i.self_ty.has_units() || i.assoc.iter().flatten().any(|t| t.has_units()));
        fns || adts || rest
    }
}

/// A conversion of a value from one unit to another of its dimension (`unit.conv.1`): it is
/// multiplied by `factor`, then `offset` is added (for an affine unit). Integers convert only by
/// whole factors and offsets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UnitConv {
    /// What the value is multiplied by.
    pub factor: Factor,
    /// What is added after, if anything.
    pub offset: Option<Factor>,
}

/// A conversion's constant: exact, or (for floats only) a float's bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Factor {
    /// `num / den`, exactly.
    Exact(i128, i128),
    /// A float, by its bits.
    Float(u64),
}

impl Factor {
    /// The factor a scale is.
    pub fn of(s: Scale) -> Factor {
        match s {
            Scale::Exact(r) => Factor::Exact(r.num, r.den),
            Scale::Approx(x) => Factor::Float(x.to_bits()),
        }
    }

    /// It as a float.
    pub fn to_f64(self) -> f64 {
        match self {
            Factor::Exact(n, d) => n as f64 / d as f64,
            Factor::Float(b) => f64::from_bits(b),
        }
    }

    /// The whole number it is, if it is one.
    pub fn as_int(self) -> Option<i128> {
        match self {
            Factor::Exact(n, 1) => Some(n),
            _ => None,
        }
    }
}

impl crate::items::ItemTree {
    /// Unit `u`'s dimension, if it has no parameters or variables in it.
    pub fn dim_of(&self, u: &UnitTy) -> Option<Dim> {
        let mut d = Dim::new();
        for (a, e) in u.parts() {
            match a {
                UnitAtom::Named(i, _) => {
                    dim_mul(&mut d, &self.units[*i as usize].dim, *e)?;
                }
                _ => return None,
            }
        }
        Some(d)
    }

    /// Unit `u`'s scale: how many of its dimension's base unit it is. Parameters in it are left
    /// out, as they cancel in a conversion between two units with the same ones.
    pub fn scale_of(&self, u: &UnitTy) -> Scale {
        let mut s = Scale::ONE;
        for (a, e) in u.parts() {
            if let UnitAtom::Named(i, _) = a {
                s = s.mul(self.units[*i as usize].scale.pow(*e));
            }
        }
        s
    }

    /// The offset of `u`, if it is one affine unit to the first power (`unit.affine.1`).
    pub fn offset_of(&self, u: &UnitTy) -> Option<Scale> {
        match u.parts() {
            [(UnitAtom::Named(i, _), e)] if *e == Ratio::ONE => self.units[*i as usize].offset,
            _ => None,
        }
    }

    /// Whether `u` has an affine unit in it (`unit.affine.1`).
    pub fn is_affine(&self, u: &UnitTy) -> bool {
        u.parts().iter().any(
            |(a, _)| matches!(a, UnitAtom::Named(i, _) if self.units[*i as usize].offset.is_some()),
        )
    }

    /// Whether a value in unit `from` converts to unit `to` (`unit.type.3`): its parameters
    /// cancel, and what is left has no dimension.
    pub fn converts(&self, from: &UnitTy, to: &UnitTy) -> bool {
        let r = from.div(to);
        !r.has_vars() && self.dim_of(&r).is_some_and(|d| d.is_empty())
    }

    /// The conversion of a value in `from` to `to`, of the same dimension: of a point, applying
    /// the affine units' offsets (`unit.affine.1`); of a difference, only their scales.
    pub fn conversion(&self, from: &UnitTy, to: &UnitTy, point: bool) -> UnitConv {
        let factor = self.scale_of(from).div(self.scale_of(to));
        let offset = if point {
            let zero = Scale::Exact(Ratio::ZERO);
            let a = self.offset_of(from).unwrap_or(zero);
            let b = self.offset_of(to).unwrap_or(zero);
            let o = a.sub(b).div(self.scale_of(to));
            (!o.is_zero()).then(|| Factor::of(o))
        } else {
            None
        };
        UnitConv {
            factor: Factor::of(factor),
            offset,
        }
    }
}
