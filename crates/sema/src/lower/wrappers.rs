//! A wrapped function's body (`attr.around.1`): with `#[around(w₁(c…))] … #[around(wₙ)]`, the
//! body is `w₁(c…, &INFO, Args { p₁, … }, |Args { p₁, … }| w₂(…, |Args { p₁, … }| body))`,
//! each wrapper call's errors passing on as a marked call's do.

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{SyntaxKind as K, SyntaxNode, TextRange};

use super::Lowerer;
use crate::hir::{ExprId, ExprKind, Local, LocalId, PatKind};
use crate::items::{Def, FnSig};
use crate::{Code, Ty};

/// One wrapper, lowered: the attribute's range, the function and its compile-time arguments.
struct Wrapper {
    range: TextRange,
    callee: ExprId,
    args: Vec<ExprId>,
}

impl Lowerer<'_> {
    /// The body of `sig`, wrapped by the `#[around]` attributes `attrs`, its original `body`
    /// innermost.
    pub(super) fn wrapped(
        &mut self,
        sig: &FnSig,
        attrs: &[SyntaxNode],
        body: Option<&ast::Block>,
    ) -> ExprId {
        let whole = attrs[0].text_range();
        let args_adt = match self.item(&crate::wrap::args_name(&sig.name)) {
            Some(Def::Adt(a)) => Some(a),
            _ => None,
        };
        let info = match self.item(&crate::wrap::info_name(&sig.name)) {
            Some(Def::Fn(f)) => Some(f),
            _ => None,
        };
        let (Some(args_adt), Some(info)) = (args_adt, info) else {
            return self.missing(whole);
        };
        // The wrappers and their arguments are compile-time code, outside the parameters' scope.
        let names = std::mem::take(&mut self.names);
        let wrappers: Vec<Option<Wrapper>> = attrs.iter().map(|a| self.wrapper(a)).collect();
        self.names = names;
        let Some(wrappers) = wrappers.into_iter().collect::<Option<Vec<_>>>() else {
            return self.missing(whole);
        };
        // Outermost first: each wrapper's call, and the closure it is given.
        let mut params: Vec<LocalId> = self.out.body.params.clone();
        let mut calls = Vec::new();
        let mark = self.names.mark();
        let outer_closure = self.closure;
        for w in &wrappers {
            let range = w.range;
            let fields: Vec<(u32, ExprId)> = params
                .iter()
                .enumerate()
                .map(|(i, &l)| (i as u32, self.alloc(ExprKind::Local(l), range)))
                .collect();
            let value = self.alloc(
                ExprKind::Adt {
                    adt: args_adt,
                    variant: 0,
                    fields,
                    args: vec![Ty::Infer(Ty::HOLE); sig.params.len()],
                },
                range,
            );
            let info_fn = self.alloc(
                ExprKind::Fn {
                    func: info,
                    args: Vec::new(),
                },
                range,
            );
            let info_value = self.alloc(
                ExprKind::Call {
                    callee: info_fn,
                    args: Vec::new(),
                },
                range,
            );
            let info_ref = self.alloc(
                ExprKind::Ref {
                    mutable: false,
                    expr: info_value,
                },
                range,
            );
            // The closure taking the arguments back, binding the parameters' names again.
            let closure = self.alloc(ExprKind::Missing, range);
            let parent = self.closure.replace(closure);
            let outer = self.names.enter_closure();
            let mut pats = Vec::new();
            let mut locals = Vec::new();
            for (i, p) in sig.params.iter().enumerate() {
                let name = p.name.clone().unwrap_or_else(|| "_".to_string());
                let local = self.out.body.locals.alloc(Local {
                    name: name.clone(),
                    mutable: p.mutable,
                    range: p.range,
                    closure: Some(closure),
                });
                let pat = self.alloc_pat(PatKind::Bind { local, sub: None }, p.range);
                pats.push((i as u32, pat));
                self.names.bind(name, local);
                locals.push(local);
            }
            let pat = self.alloc_pat(
                PatKind::Adt {
                    adt: args_adt,
                    variant: 0,
                    fields: pats,
                },
                range,
            );
            calls.push((w, info_ref, value, closure, pat, parent, outer));
            params = locals;
        }
        let mut inner = match body {
            Some(b) => self.block(b),
            None => self.missing(whole),
        };
        // Innermost first: close each closure over what it calls, and make the call.
        for (w, info_ref, value, closure, pat, parent, outer) in calls.into_iter().rev() {
            self.names.exit_closure(outer);
            self.closure = parent;
            self.out.body.exprs[closure].kind = ExprKind::Closure {
                params: vec![(pat, None)],
                // It stands for the body, so it returns what the function does.
                ret: Some(sig.ret.clone()),
                body: inner,
                parent,
            };
            let mut args = w.args.clone();
            args.extend([info_ref, value, closure]);
            let call = self.alloc(
                ExprKind::Call {
                    callee: w.callee,
                    args,
                },
                w.range,
            );
            inner = self.alloc(ExprKind::Pass(call), w.range);
        }
        self.names.restore(mark);
        self.closure = outer_closure;
        inner
    }

    /// The wrapper `#[around(…)]` attribute `attr` names: a function's path, or a call of one
    /// with its compile-time arguments (E1005 otherwise).
    fn wrapper(&mut self, attr: &SyntaxNode) -> Option<Wrapper> {
        let range = attr.text_range();
        let e = crate::wrap::input(self.text, attr);
        match e.as_ref().map(|e| e.syntax().kind()) {
            Some(K::PATH_EXPR) => {
                let callee = self.expr(e.as_ref()?);
                Some(Wrapper {
                    range,
                    callee,
                    args: Vec::new(),
                })
            }
            Some(K::CALL_EXPR) => {
                let node = e?.syntax().clone();
                let callee = node.children().find_map(ast::Expr::cast)?;
                if callee.syntax().kind() != K::PATH_EXPR {
                    self.not_a_wrapper(range);
                    return None;
                }
                let callee = self.expr(&callee);
                let args = self
                    .args(&node)
                    .into_iter()
                    .map(|a| {
                        let at = self.out.body.exprs[a].range;
                        self.alloc(ExprKind::ConstBlock(a), at)
                    })
                    .collect();
                Some(Wrapper {
                    range,
                    callee,
                    args,
                })
            }
            _ => {
                self.not_a_wrapper(range);
                None
            }
        }
    }

    fn not_a_wrapper(&mut self, range: TextRange) {
        self.report(
            Code::E1005,
            range,
            "`#[around(w)]` names a function, `w`, or calls one with compile-time arguments, \
             `w(a, …)`",
        );
    }
}
