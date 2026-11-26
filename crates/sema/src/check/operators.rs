//! Operators and comparison chains: built in on `int` and `bool`, otherwise a call of the
//! operator's trait method.

use moddil_syntax::TextRange;

use crate::hir::{BinaryOp, CompareOp, ExprId, ExprKind, UnaryOp};
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

    /// Operand `x` of an operator, of type `t`, read through its borrows (`borrow.deref.2`): the
    /// type it is read as. With `copy_only` (arithmetic), only a borrow of a `Copy` type is read
    /// through, since reading it gives a copy; a comparison borrows what it compares, so it reads
    /// through any borrow. A `&str` or a slice borrow is not read through: it is compared as it
    /// is (`text.cmp.1`).
    pub(super) fn read_through(&mut self, x: ExprId, t: &Ty, copy_only: bool) -> Ty {
        let mut t = self.norm(t);
        let mut n = 0;
        while let Ty::Ref(_, inner) = &t {
            let inner = self.norm(inner);
            if matches!(inner, Ty::Str | Ty::Slice(_)) || copy_only && !self.solver.is_copy(&inner)
            {
                break;
            }
            t = inner;
            n += 1;
        }
        if n > 0 {
            self.out.operand_derefs.insert(x, n);
        }
        t
    }

    /// Whether `r` is a literal, or built of them, whose type an operator's other operand gives
    /// (`num.lit.1`): checked against that type as it is.
    fn literal_operand(&self, r: ExprId) -> bool {
        matches!(
            self.body.exprs[r].kind,
            ExprKind::Int(_)
                | ExprKind::Float(_)
                | ExprKind::Bool(_)
                | ExprKind::Char(_)
                | ExprKind::Str(_)
                | ExprKind::Array(_)
                | ExprKind::Repeat(..)
                | ExprKind::RepeatN(..)
                | ExprKind::Unary(..)
                | ExprKind::Closure { .. }
        )
    }

    /// The right operand `r` of an arithmetic operator, checked against `expected` after it is
    /// read through its borrows. A literal is checked against `expected` as it is, so the
    /// type is the literal's from the start.
    fn check_operand(&mut self, r: ExprId, expected: &Ty) {
        if matches!(
            self.body.exprs[r].kind,
            ExprKind::Int(_)
                | ExprKind::Float(_)
                | ExprKind::Bool(_)
                | ExprKind::Char(_)
                | ExprKind::Str(_)
                | ExprKind::Array(_)
                | ExprKind::Repeat(..)
                | ExprKind::RepeatN(..)
                | ExprKind::Unary(..)
                | ExprKind::Closure { .. }
        ) {
            self.check(r, expected);
            return;
        }
        let t = self.infer(r);
        let t = self.read_through(r, &t, true);
        if !self.coerce(&t, expected) {
            self.mismatch(r, expected, &t);
        }
    }

    /// An operator on a type other than `int` and `bool`: through the trait `tr` and its method
    /// `method`, with `operand` as `Self`. The result type is the trait's `Output`.
    pub(super) fn operator(
        &mut self,
        e: ExprId,
        range: TextRange,
        symbol: &str,
        operand: &Ty,
        names: (&str, &str),
    ) -> Option<Ty> {
        self.operator_with(e, range, symbol, operand, None, names)
    }

    /// As [`Self::operator`], with `rhs` as the trait's type argument where it takes one
    /// (`Add<Rhs = Self>`, `generic.default.1`).
    pub(super) fn operator_with(
        &mut self,
        e: ExprId,
        range: TextRange,
        symbol: &str,
        operand: &Ty,
        rhs: Option<&Ty>,
        (tr, method): (&str, &str),
    ) -> Option<Ty> {
        let (mut trait_ref, func) = self.op_trait(tr, method)?;
        let generic = self.items.traits[trait_ref.trait_ as usize]
            .generics
            .params
            .len()
            > 1;
        if generic {
            trait_ref.args = vec![rhs.unwrap_or(operand).clone()].into();
        }
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
        let mut args = vec![operand.clone()];
        args.extend(trait_ref.args.iter().cloned());
        self.out.calls.insert(e, Callee::Trait { func, args });
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
        let t = self.read_through(x, &t, true);
        let t = self.items.base(&t);
        let sym = if op == UnaryOp::Neg { "-" } else { "!" };
        match (op, &t) {
            (_, Ty::Error | Ty::Never) => Ty::Error,
            (UnaryOp::Neg | UnaryOp::Not, Ty::Int) => Ty::Int,
            // Negation on a signed integer or a float, `!` on an integer (`num.ops.1`).
            (UnaryOp::Neg, Ty::Num(n)) if n.is_signed() || n.is_float() => t.clone(),
            (UnaryOp::Not, Ty::Num(n)) if !n.is_float() => t.clone(),
            (UnaryOp::Neg, Ty::Infer(_)) if self.solver.table.lit_kind(&t).is_some() => t.clone(),
            (UnaryOp::Not, Ty::Infer(_))
                if self.solver.table.lit_kind(&t) == Some(crate::Lit::Int) =>
            {
                t.clone()
            }
            (UnaryOp::Not, Ty::Bool) => Ty::Bool,
            (_, Ty::Infer(_)) => self.unknown(range, &format!("to apply `{sym}` to it")),
            (_, Ty::Ref(..)) => {
                let t = self.show(&t);
                self.report(
                    Code::E0205,
                    range,
                    format!(
                        "`{sym}` is not defined for `{t}`: what it borrows is not `Copy`, so                          reading it would move it out of the borrow"
                    ),
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
        // A refined operand is its base (`refine.base.1`).
        let lt = self.norm(lt);
        let lt = self.items.base(&lt);
        let names = match op {
            BinaryOp::Add => Some(("Add", "add")),
            BinaryOp::Sub => Some(("Sub", "sub")),
            BinaryOp::Mul => Some(("Mul", "mul")),
            BinaryOp::Div => Some(("Div", "div")),
            BinaryOp::Rem => Some(("Rem", "rem")),
            _ => None,
        };
        let lit = self.solver.table.lit_kind(&lt).is_some();
        match &lt {
            Ty::Int => {
                self.check_operand(r, &Ty::Int);
                Ty::Int
            }
            // On the sized integers and the floats, both operands have one type
            // (`num.ops.1`); a float has no bitwise operators.
            Ty::Num(n) if !n.is_float() || !(op.is_bitwise() || op.wraps()) => {
                self.check_operand(r, &lt);
                lt.clone()
            }
            // A literal's type is decided by the other operand, or by default.
            Ty::Infer(_) if lit => {
                let rt = self.infer(r);
                let rt = self.read_through(r, &rt, true);
                if !self.coerce(&rt, &lt) {
                    self.mismatch(r, &lt, &rt);
                }
                lt.clone()
            }
            Ty::Bool if op.applies_to_bool() => {
                self.check_operand(r, &Ty::Bool);
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
                        "`{}` is not defined for `{lt}`: what it borrows is not `Copy`, so                          reading it would move it out of the borrow",
                        op.symbol()
                    ),
                );
                Ty::Error
            }
            _ if names.is_some() => {
                let names = names.expect("a trait");
                // The right operand's own type, if the left's has an impl of the trait for it
                // (`impl Add<Duration> for Instant`); or else the left's, which a literal takes
                // (`expr.op.trait.1`).
                if self.literal_operand(r) {
                    let t = self.operator_with(e, range, op.symbol(), &lt, None, names);
                    if t.as_ref().is_some_and(|t| *t != Ty::Error) {
                        self.check_operand(r, &lt);
                    } else {
                        self.infer(r);
                    }
                    return t.unwrap_or(Ty::Error);
                }
                let rt = self.infer(r);
                let rt = self.read_through(r, &rt, true);
                let rt = self.norm(&rt);
                let mixed = !self.solver.can_unify(&rt, &lt)
                    && self.op_trait(names.0, names.1).is_some_and(|(mut tr, _)| {
                        tr.args = vec![rt.clone()].into();
                        self.solver.implements(&lt, &tr) != Answer::No
                    });
                let rhs = if mixed { rt.clone() } else { lt.clone() };
                let t = self.operator_with(e, range, op.symbol(), &lt, Some(&rhs), names);
                if !mixed && t.as_ref().is_some_and(|t| *t != Ty::Error) && !self.coerce(&rt, &lt) {
                    self.mismatch(r, &lt, &rt);
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
        let t0 = self.read_through(first, &t0, false);
        let mut prev = (first, self.items.base(&t0));
        for (i, &(op, x)) in rest.iter().enumerate() {
            let t = self.infer(x);
            let t = self.read_through(x, &t, false);
            let t = self.items.base(&t);
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
            // Two `&str`s, or two slices, compare what they point to.
            let j = match &j {
                Ty::Ref(_, t) if **t == Ty::Str => Ty::Str,
                Ty::Ref(_, t) if matches!(**t, Ty::Slice(_)) => (**t).clone(),
                _ => j,
            };
            match &j {
                // `&str`s compare their text, built in.
                Ty::Error | Ty::Never | Ty::Int | Ty::Num(_) | Ty::Char | Ty::Str => {}
                Ty::Infer(_) if self.solver.table.lit_kind(&j).is_some() => {}
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
