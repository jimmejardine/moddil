//! Operators and comparison chains: built in on `int` and `bool`, otherwise a call of the
//! operator's trait method.

use moddil_syntax::TextRange;

use crate::hir::{BinaryOp, CompareOp, ExprId, UnaryOp};
use crate::traits::Answer;
use crate::ty::TraitRef;
use crate::{Code, Ty};

use super::Checker;
use crate::Callee;

impl Checker<'_> {
    /// The prelude trait for an operator, and its method.
    pub(super) fn op_trait(&self, name: &str, method: &str) -> Option<(TraitRef, u32)> {
        let tr = self.items.lang_ref(name)?;
        let def = &self.items.traits[tr.trait_ as usize];
        let m = def
            .methods
            .iter()
            .copied()
            .find(|&m| self.items.fns[m as usize].name == method)?;
        Some((tr, m))
    }

    /// An operator on a type other than `int` and `bool`: through the trait `tr` and its method
    /// `method`, with `operand` as `Self`. The result type is the trait's `Output`.
    pub(super) fn operator(
        &mut self,
        e: ExprId,
        range: TextRange,
        symbol: &str,
        operand: &Ty,
        (tr, method): (&str, &str),
    ) -> Option<Ty> {
        let (trait_ref, func) = self.op_trait(tr, method)?;
        if self.solver.implements(operand, &trait_ref) == Answer::No {
            let t = self.show(operand);
            self.report(
                Code::E0205,
                range,
                format!("`{symbol}` is not defined for `{t}`: it does not implement `{tr}`"),
            );
            return Some(Ty::Error);
        }
        self.require(operand.clone(), trait_ref.clone(), range);
        self.out.calls.insert(
            e,
            Callee::Trait {
                func,
                args: vec![operand.clone()],
            },
        );
        let def = &self.items.traits[trait_ref.trait_ as usize];
        let out = match def.assoc.iter().position(|(n, _)| n == "Output") {
            Some(i) => Ty::Assoc(std::sync::Arc::new(crate::ty::AssocTy {
                self_ty: operand.clone(),
                trait_ref,
                index: i as u32,
                name: "Output".into(),
            })),
            None => Ty::Bool,
        };
        Some(self.norm(&out))
    }

    pub(super) fn unary(&mut self, e: ExprId, op: UnaryOp, x: ExprId) -> Ty {
        let range = self.range(e);
        let t = self.infer(x);
        let t = self.norm(&t);
        let sym = if op == UnaryOp::Neg { "-" } else { "!" };
        match (op, &t) {
            (_, Ty::Error | Ty::Never) => Ty::Error,
            (UnaryOp::Neg | UnaryOp::Not, Ty::Int) => Ty::Int,
            (UnaryOp::Not, Ty::Bool) => Ty::Bool,
            (_, Ty::Infer(_)) => self.unknown(range, &format!("to apply `{sym}` to it")),
            (_, Ty::Ref(..)) => {
                let t = self.show(&t);
                self.report(
                    Code::E0205,
                    range,
                    format!("`{sym}` is not defined for `{t}`: dereference it with `*`"),
                );
                Ty::Error
            }
            _ => {
                let names = if op == UnaryOp::Neg {
                    ("Neg", "neg")
                } else {
                    ("Not", "not")
                };
                self.operator(e, range, sym, &t, names).unwrap_or(Ty::Error)
            }
        }
    }

    /// `l op r`, where `l` has type `lt`.
    pub(super) fn binary(
        &mut self,
        e: ExprId,
        range: TextRange,
        op: BinaryOp,
        lt: &Ty,
        r: ExprId,
    ) -> Ty {
        let lt = self.norm(lt);
        let names = match op {
            BinaryOp::Add => Some(("Add", "add")),
            BinaryOp::Sub => Some(("Sub", "sub")),
            BinaryOp::Mul => Some(("Mul", "mul")),
            BinaryOp::Div => Some(("Div", "div")),
            BinaryOp::Rem => Some(("Rem", "rem")),
            _ => None,
        };
        match &lt {
            Ty::Int => {
                self.check(r, &Ty::Int);
                Ty::Int
            }
            Ty::Bool if op.applies_to_bool() => {
                self.check(r, &Ty::Bool);
                Ty::Bool
            }
            Ty::Error | Ty::Never => {
                self.infer(r);
                Ty::Error
            }
            Ty::Infer(_) => {
                self.infer(r);
                self.unknown(range, &format!("to apply `{}` to it", op.symbol()))
            }
            Ty::Ref(..) => {
                self.infer(r);
                let lt = self.show(&lt);
                self.report(
                    Code::E0205,
                    range,
                    format!(
                        "`{}` is not defined for `{lt}`: dereference it with `*`",
                        op.symbol()
                    ),
                );
                Ty::Error
            }
            _ if names.is_some() => {
                let t = self.operator(e, range, op.symbol(), &lt, names.expect("a trait"));
                if t.as_ref().is_some_and(|t| *t != Ty::Error) {
                    self.check(r, &lt);
                } else {
                    self.infer(r);
                }
                t.unwrap_or(Ty::Error)
            }
            _ => {
                self.infer(r);
                let lt = self.show(&lt);
                self.report(
                    Code::E0205,
                    range,
                    format!("`{}` is not defined for `{lt}`", op.symbol()),
                );
                Ty::Error
            }
        }
    }

    pub(super) fn compare(&mut self, e: ExprId, first: ExprId, rest: &[(CompareOp, ExprId)]) -> Ty {
        let range = self.range(e);
        let t0 = self.infer(first);
        let mut prev = (first, t0);
        for (i, &(op, x)) in rest.iter().enumerate() {
            let t = self.infer(x);
            let (_, pt) = prev.clone();
            let Some(j) = self.join(&pt, &t) else {
                if op.is_ordering() {
                    let st = self.show(&t);
                    let sp = self.show(&pt);
                    self.report(
                        Code::E0201,
                        self.range(x),
                        format!(
                            "`{}` compares values of one type: `{sp}` and `{st}`",
                            op.symbol()
                        ),
                    );
                } else {
                    self.mismatch(x, &pt, &t);
                }
                prev = (x, t);
                continue;
            };
            let j = self.norm(&j);
            let j = match &j {
                Ty::Ref(_, t) if **t == Ty::Str => Ty::Str,
                _ => j,
            };
            match &j {
                // `&str`s compare their text, built in.
                Ty::Error | Ty::Never | Ty::Int | Ty::Char | Ty::Str => {}
                Ty::Bool | Ty::Unit if !op.is_ordering() => {}
                Ty::Infer(_) => {
                    self.unknown(range, &format!("to compare it with `{}`", op.symbol()));
                }
                Ty::Ref(..) => self.report(
                    Code::E0205,
                    range,
                    format!(
                        "`{}` compares values, not borrows: dereference both sides with `*`",
                        op.symbol()
                    ),
                ),
                _ => {
                    let (tr, method) = match op {
                        CompareOp::Eq => ("PartialEq", "eq"),
                        CompareOp::Ne => ("PartialEq", "ne"),
                        CompareOp::Lt => ("PartialOrd", "lt"),
                        CompareOp::Le => ("PartialOrd", "le"),
                        CompareOp::Gt => ("PartialOrd", "gt"),
                        CompareOp::Ge => ("PartialOrd", "ge"),
                    };
                    if let Some((trait_ref, func)) = self.op_trait(tr, method) {
                        if self.solver.implements(&j, &trait_ref) == Answer::No {
                            let js = self.show(&j);
                            self.report(
                                Code::E0205,
                                range,
                                format!(
                                    "`{}` is not defined for `{js}`: it does not implement `{tr}`",
                                    op.symbol()
                                ),
                            );
                        } else {
                            self.require(j.clone(), trait_ref, range);
                            self.out.compare_calls.insert(
                                (e, i),
                                Callee::Trait {
                                    func,
                                    args: vec![j.clone()],
                                },
                            );
                        }
                    }
                }
            }
            prev = (x, t);
        }
        Ty::Bool
    }
}
