//! Unit conversions (`unit.conv.1`, `unit.conv.2`): units are erased before MIR, so a value
//! converted to another unit of its dimension is multiplied by a constant, and an affine unit's
//! offset added. An integer is multiplied by a whole factor, with the overflow check any
//! multiplication has; a float by the factor, or divided by it when it is one over a whole
//! number, so that `1[cm]` as metres is `1.0 / 100.0`, exactly as written.

use moddil_sema::Ty;
use moddil_sema::hir::{BinaryOp, ExprId};
use moddil_sema::units::{Factor, UnitConv};

use super::Builder;
use crate::{Const, Local, Operand, Place};

impl Builder<'_> {
    /// The conversion of `e`'s value, if it is converted and not being evaluated for it.
    pub(super) fn unit_conv(&self, e: ExprId) -> Option<UnitConv> {
        if self.unconverted == Some(e) {
            return None;
        }
        self.types.unit_conv(e).copied()
    }

    /// Evaluates `e` into `dest`, converted to the unit it is wanted in.
    pub(super) fn eval_converted(&mut self, dest: Local, e: ExprId, conv: UnitConv) {
        let ty = self.ty(e);
        let t = self.temp(ty.clone());
        let outer = self.unconverted.replace(e);
        self.eval_into(t, e);
        self.unconverted = outer;
        let range = self.range(e);
        let mut value = self.use_place(Place::local(t), &ty);
        let factor = match conv.factor {
            Factor::Exact(1, 1) => None,
            f => Some(f),
        };
        let steps: Vec<(BinaryOp, Const)> = factor
            .map(|f| scale_step(&ty, f))
            .into_iter()
            .chain(conv.offset.map(|o| (BinaryOp::Add, constant(&ty, o))))
            .collect();
        let n = steps.len();
        for (k, (op, c)) in steps.into_iter().enumerate() {
            let to = if k + 1 == n {
                dest
            } else {
                self.temp(ty.clone())
            };
            self.arith(Place::local(to), op, &ty, value, Operand::Const(c), range);
            value = self.use_place(Place::local(to), &ty);
        }
        if n == 0 {
            self.cfg.assign(dest, crate::Rvalue::Use(value));
        }
    }
}

impl Builder<'_> {
    /// `x.to_unit::<u>()` and the like, on a quantity (`unit.to.1`): `x`'s value (read through
    /// its borrow, if it is one), converted where the call's conversion says; or, for
    /// `x.to_unit::<u>(mode)`, `x · n / d` rounded by the prelude's `__rescale`.
    pub(super) fn unit_method(
        &mut self,
        dest: Local,
        e: ExprId,
        receiver: ExprId,
        args: &[ExprId],
    ) {
        let (through, rescale) = match self.types.unit_rescale(e) {
            Some((n, d, through)) => (through, Some((n, d))),
            None => (self.types.unit_passthrough(e).unwrap_or(false), None),
        };
        let value = if through {
            let r = self.operand(receiver, args);
            let rt = self.temp(self.ty(receiver));
            self.cfg.assign(rt, crate::Rvalue::Use(r));
            let ty = self.ty(e);
            self.use_place(Place::local(rt).deref(), &ty)
        } else {
            self.operand(receiver, args)
        };
        match rescale {
            None => self.cfg.assign(dest, crate::Rvalue::Use(value)),
            Some((n, d)) => {
                let mode = self.operand(args[0], &[]);
                let callee = moddil_sema::Callee::Fn {
                    func: self.prelude_fn("__rescale"),
                    args: Vec::new(),
                };
                let int = |v: i128| Operand::Const(Const::Int(v as i64));
                let range = self.range(e);
                self.call_into(
                    None,
                    callee,
                    Place::local(dest),
                    vec![value, int(n), int(d), mode],
                    range,
                );
            }
        }
    }
}

/// The step that scales a value of type `ty` by `f`: a multiplication, or for a float and one
/// over a whole number, a division.
fn scale_step(ty: &Ty, f: Factor) -> (BinaryOp, Const) {
    let float = matches!(ty, Ty::Num(n) if n.is_float());
    match f {
        Factor::Exact(1, d) if float && d > 1 => (BinaryOp::Div, constant(ty, Factor::Exact(d, 1))),
        f => (BinaryOp::Mul, constant(ty, f)),
    }
}

/// `f` as a constant of number type `ty`: a float rounded to it, or a whole number.
fn constant(ty: &Ty, f: Factor) -> Const {
    use moddil_sema::Num;
    match ty {
        Ty::Num(Num::F32) => Const::Num(Num::F32, u128::from((f.to_f64() as f32).to_bits())),
        Ty::Num(Num::F64) => Const::Num(Num::F64, u128::from(f.to_f64().to_bits())),
        Ty::Num(n) => {
            let v = f.as_int().unwrap_or(0);
            let mask = if n.bits() == 128 {
                u128::MAX
            } else {
                (1u128 << n.bits()) - 1
            };
            Const::Num(*n, (v as u128) & mask)
        }
        _ => Const::Int(f.as_int().unwrap_or(0) as i64),
    }
}
