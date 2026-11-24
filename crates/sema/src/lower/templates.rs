//! Lowering a derive's template (`comptime.template.1`): in a method instantiated from a derive
//! function, `inline for f in fields(t) { … }` is the block once per field of the struct, and
//! `f` names that field there: `f.name` is its name, `x.[f]` and `[f]: e` the field itself,
//! `f.is_refined()` whether its type is a refinement and `f.new(e)` that refinement's `new`.
//! `if const c { … } else { … }` is the branch `c` chooses, `c` being such reflection. What
//! comes out is ordinary HIR, checked like hand-written code.

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{SyntaxKind as K, SyntaxNode, TextRange};

use super::Lowerer;
use crate::hir::{ExprId, ExprKind, Stmt};
use crate::{Code, Ty};

impl Lowerer<'_> {
    /// The field a reflected name `node` (a path of one name) stands for, if it is one.
    pub(super) fn reflected(&self, node: &SyntaxNode) -> Option<u32> {
        if node.kind() != K::PATH_EXPR {
            return None;
        }
        let path = node.child(K::PATH)?;
        let segs = ast::Path::cast(path)?.segments();
        let [name] = segs.as_slice() else {
            return None;
        };
        self.reflected
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, k)| *k)
    }

    /// The struct's field `k`: its name and its type.
    fn field_of(&self, k: u32) -> (String, Ty) {
        let Some((t, adt)) = self.template.and_then(|t| t.adt.map(|a| (t, a))) else {
            return (String::new(), Ty::Error);
        };
        let name = self
            .items
            .variant(adt, 0)
            .fields
            .get(k as usize)
            .map(|f| f.name.clone())
            .unwrap_or_default();
        let ty = self
            .items
            .field_types(&t.ty, 0)
            .get(k as usize)
            .cloned()
            .unwrap_or(Ty::Error);
        (name, ty)
    }

    /// The fields `inline for`'s head ranges over, and the name it binds: `inline for f in
    /// fields(t)` with `t` the template's parameter. E1007 otherwise.
    fn inline_head(&mut self, node: &SyntaxNode) -> Option<(String, u32)> {
        let range = node.text_range();
        let Some(t) = self.template else {
            self.report(
                Code::E1007,
                range,
                "`inline for` unrolls only in a derive's template (`comptime.template.1`)",
            );
            return None;
        };
        let name = node
            .children()
            .find_map(ast::Pat::cast)
            .filter(|p| matches!(p.syntax().kind(), K::IDENT_PAT | K::PATH_PAT))
            .map(|p| p.syntax().text().trim().to_string());
        let iter = node.children().find_map(ast::Expr::cast);
        let fields_of_t = iter.as_ref().is_some_and(|e| {
            let text: String = e
                .syntax()
                .text()
                .to_string()
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            text == format!("fields({})", t.var)
        });
        match name {
            Some(n) if fields_of_t => {
                let Some(adt) = t.adt else {
                    self.report(
                        Code::E1007,
                        range,
                        "a template for a built-in type has no fields to unroll over                          (`comptime.derive.for.1`)",
                    );
                    return None;
                };
                let count = self.items.variant(adt, 0).fields.len() as u32;
                Some((n, count))
            }
            _ => {
                self.report(
                    Code::E1007,
                    range,
                    format!(
                        "`inline for` is written `inline for f in fields({})`: once for each \
                         field, `f` naming it",
                        t.var
                    ),
                );
                None
            }
        }
    }

    /// `inline for f in fields(t) { … }`: the block once for each field, in order, as a block
    /// of those blocks.
    pub(super) fn inline_for(&mut self, node: &SyntaxNode) -> ExprId {
        let range = node.text_range();
        let Some((name, count)) = self.inline_head(node) else {
            return self.missing(range);
        };
        let Some(block) = node.child(K::BLOCK).and_then(ast::Block::cast) else {
            return self.missing(range);
        };
        let mut stmts = Vec::new();
        for k in 0..count {
            self.reflected.push((name.clone(), k));
            let expr = self.block(&block);
            self.reflected.pop();
            stmts.push(Stmt::Expr {
                expr,
                semi: true,
                observe: false,
            });
        }
        self.alloc(
            ExprKind::Block {
                stmts,
                tail: None,
                observe_tail: false,
            },
            range,
        )
    }

    /// A struct literal's fields, as names, where each name is, and values: `name: e` and
    /// `name`, and in a template `[f]: e` and `inline for f in fields(t) { … }`.
    pub(super) fn struct_inits(&mut self, node: &SyntaxNode) -> Vec<(String, TextRange, ExprId)> {
        let mut out = Vec::new();
        for init in node.children() {
            let init_range = init.text_range();
            match init.kind() {
                K::FIELD_INIT => {
                    if init.token(K::DOT_DOT).is_some() {
                        self.error(crate::Diagnostic::unsupported(
                            init_range,
                            "struct update syntax",
                        ));
                        continue;
                    }
                    let Some(name_tok) = ast::member_name(&init) else {
                        continue;
                    };
                    let name = name_tok.text().to_string();
                    let value = match init.children().find_map(ast::Expr::cast) {
                        Some(v) => self.expr(&v),
                        None => match self.lookup(&name) {
                            Some(l) => self.alloc(ExprKind::Local(l), name_tok.text_range()),
                            None => {
                                self.report(
                                    Code::E0101,
                                    name_tok.text_range(),
                                    format!("there is nothing called `{name}` here"),
                                );
                                self.missing(init_range)
                            }
                        },
                    };
                    out.push((name, name_tok.text_range(), value));
                }
                K::REFLECTED_FIELD_INIT => {
                    let exprs: Vec<ast::Expr> =
                        init.children().filter_map(ast::Expr::cast).collect();
                    let field = exprs.first().and_then(|e| self.reflected(e.syntax()));
                    let value = match exprs.get(1) {
                        Some(v) => self.expr(v),
                        None => self.missing(init_range),
                    };
                    match field {
                        Some(k) => {
                            let (name, _) = self.field_of(k);
                            out.push((name, init_range, value));
                        }
                        None => self.report(
                            Code::E1007,
                            init_range,
                            "`[f]: e` gives a reflected field `f`: one an `inline for` names",
                        ),
                    }
                }
                K::INLINE_FOR_FIELDS => {
                    let Some((name, count)) = self.inline_head(&init) else {
                        continue;
                    };
                    for k in 0..count {
                        self.reflected.push((name.clone(), k));
                        let inner = self.struct_inits(&init);
                        self.reflected.pop();
                        out.extend(inner);
                    }
                }
                K::SPREAD => {
                    self.error(crate::Diagnostic::unsupported(
                        init_range,
                        "struct update syntax",
                    ));
                }
                _ => {}
            }
        }
        out
    }

    /// `x.[f]`: field `f` of `x`.
    pub(super) fn reflected_field(&mut self, node: &SyntaxNode) -> ExprId {
        let range = node.text_range();
        let exprs: Vec<ast::Expr> = node.children().filter_map(ast::Expr::cast).collect();
        let base = match exprs.first() {
            Some(b) => self.expr(b),
            None => self.missing(range),
        };
        match exprs.get(1).and_then(|f| self.reflected(f.syntax())) {
            Some(k) => {
                let (name, _) = self.field_of(k);
                self.alloc(
                    ExprKind::Field {
                        base,
                        name,
                        name_range: range,
                    },
                    range,
                )
            }
            None => {
                self.report(
                    Code::E1007,
                    range,
                    "`x.[f]` names a reflected field `f`: one an `inline for` names",
                );
                self.missing(range)
            }
        }
    }

    /// `f.name` of reflected field `k`: its name, as text.
    pub(super) fn reflected_member(&mut self, k: u32, member: &str, range: TextRange) -> ExprId {
        if member == "name" {
            let (name, _) = self.field_of(k);
            return self.alloc(ExprKind::Str(name), range);
        }
        self.report(
            Code::E1007,
            range,
            format!("a reflected field has `name`, `is_refined()` and `new(e)`, not `{member}`"),
        );
        self.missing(range)
    }

    /// `f.is_refined()` and `f.new(e)` of reflected field `k`.
    pub(super) fn reflected_call(
        &mut self,
        k: u32,
        method: &str,
        args: &[ast::Expr],
        range: TextRange,
    ) -> ExprId {
        let (_, ty) = self.field_of(k);
        let refined = match ty {
            Ty::Refined(t, _) => self.items.types[t as usize]
                .refinement
                .as_ref()
                .map(|r| r.new),
            _ => None,
        };
        match (method, args) {
            ("is_refined", []) => self.alloc(ExprKind::Bool(refined.is_some()), range),
            ("new", [arg]) => match refined {
                Some(new) => {
                    let callee = self.alloc(
                        ExprKind::Fn {
                            func: new,
                            args: Vec::new(),
                        },
                        range,
                    );
                    let arg = self.expr(arg);
                    self.alloc(
                        ExprKind::Call {
                            callee,
                            args: vec![arg],
                        },
                        range,
                    )
                }
                None => {
                    self.report(
                        Code::E1007,
                        range,
                        "`f.new(e)` makes a refined field's value; this field's type is no \
                         refinement (ask `f.is_refined()` with `if const`)",
                    );
                    self.missing(range)
                }
            },
            _ => {
                self.report(
                    Code::E1007,
                    range,
                    format!(
                        "a reflected field has `name`, `is_refined()` and `new(e)`, not \
                         `{method}`"
                    ),
                );
                self.missing(range)
            }
        }
    }

    /// The value of `if const`'s condition: reflection (`f.is_refined()`, `f.name == "x"`),
    /// `true`, `false`, `!`, `&&` and `||`. E1007 otherwise.
    pub(super) fn comptime_bool(&mut self, e: &ast::Expr) -> Option<bool> {
        let node = e.syntax();
        let subs = e.sub_exprs();
        let op = node
            .child_tokens()
            .find(|t| !t.kind().is_trivia())
            .map(|t| t.kind());
        let value = match e {
            ast::Expr::Paren(_) => subs.first().and_then(|s| self.comptime_bool(s)),
            ast::Expr::Literal(_) => match node.tokens().first().map(|t| t.kind()) {
                Some(K::TRUE_KW) => Some(true),
                Some(K::FALSE_KW) => Some(false),
                _ => None,
            },
            ast::Expr::Prefix(_) if op == Some(K::BANG) => {
                subs.first().and_then(|s| self.comptime_bool(s)).map(|b| !b)
            }
            ast::Expr::Bin(_) | ast::Expr::CompareChain(_)
                if matches!(op, Some(K::EQ_EQ | K::BANG_EQ)) =>
            {
                self.compare_names(node, &subs, op == Some(K::EQ_EQ))
            }
            ast::Expr::Bin(_) if matches!(op, Some(K::AMP_AMP | K::PIPE_PIPE)) => {
                let a = subs.first().and_then(|s| self.comptime_bool(s))?;
                let b = subs.get(1).and_then(|s| self.comptime_bool(s))?;
                Some(if op == Some(K::AMP_AMP) {
                    a && b
                } else {
                    a || b
                })
            }
            ast::Expr::MethodCall(_) => {
                let k = subs.first().and_then(|r| self.reflected(r.syntax()))?;
                let method = ast::member_name(node)?.text().to_string();
                let (_, ty) = self.field_of(k);
                (method == "is_refined").then_some(matches!(ty, Ty::Refined(..)))
            }
            _ => None,
        };
        if value.is_none() {
            self.report(
                Code::E1007,
                node.text_range(),
                "`if const` decides by reflection: `f.is_refined()`, `f.name == \"…\"`, \
                 `true` and `false`, with `!`, `&&` and `||`",
            );
        }
        value
    }

    /// `f.name == "x"` (or `!=`, either way round): whether the names are equal (or not).
    fn compare_names(&mut self, node: &SyntaxNode, subs: &[ast::Expr], eq: bool) -> Option<bool> {
        let _ = node;
        let [a, b] = subs else {
            return None;
        };
        let name = |this: &Self, x: &ast::Expr| -> Option<String> {
            if let ast::Expr::Field(_) = x {
                let base = x.sub_exprs().into_iter().next()?;
                let k = this.reflected(base.syntax())?;
                let m = ast::member_name(x.syntax())?;
                if m.text() == "name" {
                    return Some(this.field_of(k).0);
                }
            }
            None
        };
        let text = |x: &ast::Expr| -> Option<String> {
            let t = x.syntax().text();
            let t = t.trim();
            t.strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .map(str::to_string)
        };
        let (l, r) = match (name(self, a), name(self, b)) {
            (Some(l), _) => (l, text(b)?),
            (_, Some(r)) => (text(a)?, r),
            _ => return None,
        };
        Some((l == r) == eq)
    }

    /// `if const c { … } else { … }`: the branch `c` chooses, in a template.
    pub(super) fn if_const(&mut self, node: &SyntaxNode, e: &ast::Expr) -> ExprId {
        let range = node.text_range();
        if self.template.is_none() {
            return self.unsupported(range, "`if const` outside a derive's template");
        }
        let subs = e.sub_exprs();
        let Some(chosen) = subs.first().and_then(|c| self.comptime_bool(c)) else {
            return self.missing(range);
        };
        match (chosen, subs.get(1), subs.get(2)) {
            (true, Some(then), _) => self.expr(then),
            (false, _, Some(other)) => self.expr(other),
            (false, _, None) => self.alloc(ExprKind::Unit, range),
            _ => self.missing(range),
        }
    }
}
