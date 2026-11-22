//! Arithmetic and comparisons, with the checks that trap on overflow and division by zero.

use moddil_sema::Ty;
use moddil_sema::hir::{BinaryOp, CompareOp, ExprId};
use moddil_syntax::TextRange;

use super::Builder;
use crate::{BinOp, Local, Operand, Place, Rvalue, Statement, Terminator, TrapClass};

impl Builder<'_> {
    /// Traps with `trap` if `flag` holds.
    pub(super) fn check(&mut self, flag: Local, trap: TrapClass, range: TextRange) {
        self.cfg.push(Statement::Assert {
            cond: Operand::Copy(Place::local(flag)),
            expected: false,
            trap,
            range,
            parts: Vec::new(),
        });
    }

    /// `dest = a op b`, with the checks the operator needs.
    pub(super) fn arith(
        &mut self,
        dest: Place,
        op: BinaryOp,
        ty: &Ty,
        a: Operand,
        b: Operand,
        range: TextRange,
    ) {
        let op_hir = op;
        let op = match op {
            BinaryOp::Add | BinaryOp::WrapAdd => BinOp::Add,
            BinaryOp::Sub | BinaryOp::WrapSub => BinOp::Sub,
            BinaryOp::Mul | BinaryOp::WrapMul => BinOp::Mul,
            BinaryOp::Div => BinOp::Div,
            BinaryOp::Rem => BinOp::Rem,
            BinaryOp::BitAnd => BinOp::BitAnd,
            BinaryOp::BitOr => BinOp::BitOr,
            BinaryOp::BitXor => BinOp::BitXor,
            BinaryOp::Shl => BinOp::Shl,
            BinaryOp::Shr => BinOp::Shr,
        };
        let wraps = matches!(
            op_hir,
            BinaryOp::WrapAdd | BinaryOp::WrapSub | BinaryOp::WrapMul
        );
        // An integer of any width traps on overflow and on division by zero; a float does not
        // (`num.ops.1`).
        let int = match ty {
            Ty::Int => true,
            Ty::Num(n) => !n.is_float(),
            _ => false,
        };
        if int {
            if matches!(op, BinOp::Div | BinOp::Rem) {
                let t = self.temp(Ty::Bool);
                self.cfg.assign(
                    t,
                    Rvalue::Binary(
                        BinOp::Eq,
                        b.clone(),
                        Operand::Const(crate::int_const(ty, 0)),
                    ),
                );
                self.check(t, TrapClass::DivisionByZero, range);
            }
            if !wraps
                && matches!(
                    op,
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Shl | BinOp::Shr
                )
            {
                let t = self.temp(Ty::Bool);
                self.cfg
                    .assign(t, Rvalue::Overflows(op, a.clone(), b.clone()));
                self.check(t, TrapClass::Overflow, range);
            }
        }
        self.cfg.assign(dest, Rvalue::Binary(op, a, b));
    }

    /// `a < b <= c`: each operand once, left to right, stopping at the first comparison that
    /// fails.
    pub(super) fn compare(&mut self, dest: Local, first: ExprId, rest: &[(CompareOp, ExprId)]) {
        let e = self.compare_expr;
        if self.types.compare_callee(e, 0).is_some() {
            return self.compare_traits(e, dest, first, rest);
        }
        let operands: Vec<ExprId> = rest.iter().map(|&(_, x)| x).collect();
        let mut prev = self.operand(first, &operands);
        if let [(op, x)] = rest {
            let cur = self.operand(*x, &[]);
            self.cfg
                .assign(dest, Rvalue::Binary(compare_op(*op), prev, cur));
            return;
        }
        let join = self.cfg.new_block();
        for (i, &(op, x)) in rest.iter().enumerate() {
            let cur = self.operand(x, &operands[i + 1..]);
            self.cfg
                .assign(dest, Rvalue::Binary(compare_op(op), prev, cur.clone()));
            if i + 1 < rest.len() {
                let next = self.cfg.new_block();
                self.cfg.terminate_into(
                    Terminator::SwitchBool {
                        cond: Operand::Copy(Place::local(dest)),
                        then: next,
                        else_: join,
                    },
                    next,
                );
            }
            prev = cur;
        }
        self.cfg.terminate_into(Terminator::Goto(join), join);
    }

    /// A comparison chain on a type other than `int`, `bool` and `()`: each comparison calls
    /// `PartialEq` or `PartialOrd` with borrows of its operands, which are each evaluated once.
    pub(super) fn compare_traits(
        &mut self,
        e: ExprId,
        dest: Local,
        first: ExprId,
        rest: &[(CompareOp, ExprId)],
    ) {
        let join = self.cfg.new_block();
        let mut prev = self.place(first);
        let mut prev_ty = self.ty(first);
        for (i, &(_, x)) in rest.iter().enumerate() {
            let cur = self.place(x);
            let cur_ty = self.ty(x);
            // A slice borrow is passed as it is; other operands are borrowed.
            let borrow = |b: &mut Self, place: Place, ty: &Ty| match ty {
                // Of its own type, as it is: the method takes `&[T]`, `Self` being `[T]`.
                Ty::Ref(_, inner) if matches!(**inner, Ty::Slice(_)) => {
                    let t = b.temp(ty.clone());
                    b.cfg.assign(t, Rvalue::Use(Operand::Copy(place)));
                    t
                }
                _ => {
                    let t = b.temp(Ty::reference(false, ty.clone()));
                    b.cfg.assign(t, Rvalue::Ref(false, place));
                    t
                }
            };
            let a = borrow(self, prev, &prev_ty);
            let b = borrow(self, cur.clone(), &cur_ty);
            let callee = self
                .types
                .compare_callee(e, i)
                .expect("a checked comparison")
                .clone();
            let range = self.range(e);
            self.call_into(
                None,
                callee,
                Place::local(dest),
                vec![
                    Operand::Move(Place::local(a)),
                    Operand::Move(Place::local(b)),
                ],
                range,
            );
            if i + 1 < rest.len() {
                let next = self.cfg.new_block();
                self.cfg.terminate_into(
                    Terminator::SwitchBool {
                        cond: Operand::Copy(Place::local(dest)),
                        then: next,
                        else_: join,
                    },
                    next,
                );
            }
            prev = cur;
            prev_ty = cur_ty;
        }
        self.cfg.terminate_into(Terminator::Goto(join), join);
    }
}

fn compare_op(op: CompareOp) -> BinOp {
    match op {
        CompareOp::Eq => BinOp::Eq,
        CompareOp::Ne => BinOp::Ne,
        CompareOp::Lt => BinOp::Lt,
        CompareOp::Le => BinOp::Le,
        CompareOp::Gt => BinOp::Gt,
        CompareOp::Ge => BinOp::Ge,
    }
}
