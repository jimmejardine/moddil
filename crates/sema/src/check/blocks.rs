//! Blocks and their statements.

use crate::hir::{ExprId, PatKind, Stmt};
use crate::{Code, Ty};

use super::Checker;
use super::errors::FailCx;
use super::finish::Deferred;

impl Checker<'_> {
    pub(super) fn block(&mut self, stmts: &[Stmt], tail: Option<ExprId>) -> Ty {
        let mut diverges = false;
        for s in stmts {
            match *s {
                Stmt::Let {
                    pat,
                    ref ty,
                    init,
                    else_,
                } => {
                    let t = match ty {
                        Some(ty) => {
                            let ty = self.instantiate(ty);
                            self.check(init, &ty);
                            ty
                        }
                        None => self.infer(init),
                    };
                    diverges |= self.norm(&t) == Ty::Never;
                    if matches!(self.body.pats[pat].kind, PatKind::Wild) {
                        let at = self.body.pats[pat].range;
                        self.dropped_result(&t, at);
                    }
                    self.check_pat(pat, &t, None);
                    match else_ {
                        None => self
                            .deferred
                            .push(Deferred::Irrefutable(pat, "`let` without `else`")),
                        Some(x) => {
                            let et = self.infer(x);
                            let et = self.norm(&et);
                            if !matches!(et, Ty::Never | Ty::Error) {
                                let et = self.show(&et);
                                self.report(
                                    Code::E0210,
                                    self.range(x),
                                    format!(
                                        "the `else` block of `let … else` must not finish \
                                         normally (end it with `return`, `break` or \
                                         `continue`), but it has type `{et}`"
                                    ),
                                );
                            }
                        }
                    }
                }
                // `defer` cannot fail; `errdefer`'s errors are suppressed (`stmt.defer.1`,
                // `stmt.errdefer.1`).
                Stmt::Defer(body) => {
                    self.within(FailCx::Never("a `defer` block"), body, &Ty::Unit);
                }
                Stmt::ErrDefer(body) => {
                    self.within(FailCx::Suppressed, body, &Ty::Unit);
                }
                Stmt::Expr {
                    expr,
                    semi,
                    observe,
                } => {
                    let t = self.infer(expr);
                    let t = self.norm(&t);
                    diverges |= t == Ty::Never;
                    if !observe && !self.is_discard(expr) {
                        self.dropped_result(&t, self.range(expr));
                    }
                    // A value followed by a `// =>` expectation is used by it.
                    if !semi && !observe && !self.coerce(&t, &Ty::Unit) {
                        let t = self.show(&t);
                        self.report(
                            Code::E0201,
                            self.range(expr),
                            format!(
                                "expected `()`, found `{t}`: a value in the middle of a block \
                                 needs a `;` after it"
                            ),
                        );
                    }
                }
            }
        }
        match tail {
            Some(t) => self.infer(t),
            None if diverges => Ty::Never,
            None => Ty::Unit,
        }
    }
}
