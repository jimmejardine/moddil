//! A `#[selectable]` function's two parts (`task.select.4`): its body receives first
//! (`self.incoming.recv()`), then works with what it received. Lowering makes each part a
//! closure in the function's own body, taking the function's parameters: one gives the
//! receiver, borrowed, which a `select` waits on, and one is the rest of the body with what was
//! received in place of the receive. A `select` arm calls them; the function itself, called
//! as any other, runs its body as written.

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{SyntaxKind as K, SyntaxNode, TextRange};

use super::Lowerer;
use crate::hir::{ExprId, ExprKind, Local, LocalId, PatKind, SelectParts};
use crate::items::FnSig;
use crate::{Code, Ty};

impl Lowerer<'_> {
    /// The two parts of selectable function `sig`, whose body is `block`, recorded in the
    /// body; E0217 if the body does not receive first.
    pub(super) fn selectable_parts(&mut self, sig: &FnSig, block: &ast::Block) {
        // Where its parts are: the body's expression.
        let range = block
            .syntax()
            .children()
            .find_map(ast::Expr::cast)
            .map_or(block.syntax().text_range(), |e| e.syntax().text_range());
        let Some((recv, place)) = receive_first(block) else {
            self.report(
                Code::E0217,
                sig.name_range,
                "a `#[selectable]` function receives first, then works with what it received: \
                 its body is one expression that starts with `self.incoming.recv()`, a channel \
                 its parameters reach",
            );
            return;
        };
        let tys: Vec<Ty> = sig.params.iter().map(|p| p.ty.clone()).collect();
        let names: Vec<String> = sig
            .params
            .iter()
            .map(|p| p.name.clone().unwrap_or_else(|| "_".to_string()))
            .collect();
        // The receiver, borrowed.
        let chan = self.part(range, &names, &tys, None, |this| {
            let x = this.expr(&place);
            this.alloc(
                ExprKind::Ref {
                    mutable: false,
                    expr: x,
                },
                place.syntax().text_range(),
            )
        });
        // The rest, with what was received in place of the receive.
        let mut got = None;
        let done = self.part(range, &names, &tys, Some((recv, &mut got)), |this| {
            this.block(block)
        });
        self.out.body.select = Some(SelectParts {
            chan,
            done,
            received: got.expect("the received value's local"),
        });
    }

    /// A closure taking parameters named `names` of types `tys` (and, if `got` is given, what
    /// was received, after them), whose body `body` lowers. The parameters shadow the
    /// function's; it runs in the caller's frame, so it may capture nothing (which checking
    /// reports).
    fn part(
        &mut self,
        range: TextRange,
        names: &[String],
        tys: &[Ty],
        got: Option<(TextRange, &mut Option<LocalId>)>,
        body: impl FnOnce(&mut Self) -> ExprId,
    ) -> ExprId {
        let id = self.alloc(ExprKind::Missing, range);
        let parent = self.closure.replace(id);
        let mark = self.names.mark();
        let mut params = Vec::new();
        for (name, ty) in names.iter().zip(tys) {
            let local = self.out.body.locals.alloc(Local {
                name: name.clone(),
                mutable: false,
                range,
                closure: Some(id),
            });
            let pat = self.alloc_pat(PatKind::Bind { local, sub: None }, range);
            params.push((pat, Some(ty.clone())));
            self.names.bind(name.clone(), local);
        }
        if let Some((recv, got)) = got {
            let local = self.out.body.locals.alloc(Local {
                name: "{received}".to_string(),
                mutable: false,
                range,
                closure: Some(id),
            });
            let pat = self.alloc_pat(PatKind::Bind { local, sub: None }, range);
            params.push((pat, None));
            *got = Some(local);
            self.received = Some((recv, local));
        }
        let body = body(self);
        self.received = None;
        self.names.restore(mark);
        self.closure = parent;
        self.out.body.exprs[id].kind = ExprKind::Closure {
            params,
            ret: None,
            body,
            parent,
        };
        id
    }

    /// While the rest of a selectable function is lowered: the receive, by range, becomes
    /// what was received.
    pub(super) fn received_at(&mut self, node: &SyntaxNode) -> Option<ExprId> {
        let (recv, local) = self.received?;
        (node.text_range() == recv).then(|| self.alloc(ExprKind::Local(local), node.text_range()))
    }
}

/// The receive a selectable body starts with, and the place it receives from: the body is one
/// expression, and the receive is what it evaluates first.
pub(super) fn receive_first(block: &ast::Block) -> Option<(TextRange, ast::Expr)> {
    let node = block.syntax();
    if node
        .children()
        .any(|c| matches!(c.kind(), K::LET_STMT | K::EXPR_STMT))
    {
        return None;
    }
    let mut e = node.children().find_map(ast::Expr::cast)?;
    loop {
        match &e {
            ast::Expr::MethodCall(_)
                if ast::member_name(e.syntax()).is_some_and(|n| n.text() == "recv")
                    && e.syntax()
                        .child(K::ARG_LIST)
                        .and_then(ast::ArgList::cast)
                        .is_none_or(|l| l.args().next().is_none()) =>
            {
                let place = e.sub_exprs().into_iter().next()?;
                return is_place(&place).then(|| (e.syntax().text_range(), place));
            }
            ast::Expr::MethodCall(_)
            | ast::Expr::Field(_)
            | ast::Expr::Propagate(_)
            | ast::Expr::Unwrap(_)
            | ast::Expr::Match(_)
            | ast::Expr::Paren(_)
            | ast::Expr::Bin(_) => e = e.sub_exprs().into_iter().next()?,
            _ => return None,
        }
    }
}

/// Whether `e` is a parameter or a field of one, read without running anything.
fn is_place(e: &ast::Expr) -> bool {
    match e {
        ast::Expr::Path(_) => true,
        ast::Expr::Field(_) | ast::Expr::Paren(_) => e.sub_exprs().first().is_some_and(is_place),
        _ => false,
    }
}
