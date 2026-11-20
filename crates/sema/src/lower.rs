//! Lowering a body from the syntax tree to HIR: the `body` query.

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{SyntaxKind as K, SyntaxNode, SyntaxToken, TextRange};

use crate::hir::{
    BinaryOp, Body, CompareOp, Expr, ExprId, ExprKind, Local, LocalId, LogicOp, Stmt, UnaryOp,
};
use crate::items::{binding, item_node, item_tree, parse, resolve_type};
use crate::{Code, Db, Diagnostic, Item, ItemTree, Owner};

/// A body in HIR, with the problems found while lowering it.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Lowered {
    /// The body.
    pub body: Body,
    /// Unresolved names, unsupported constructs and the like.
    pub diagnostics: Vec<Diagnostic>,
}

/// The HIR of a function's or a test's body.
#[salsa::tracked(returns(ref))]
pub fn body<'db>(db: &'db dyn Db, owner: Owner<'db>) -> Lowered {
    let file = owner.file(db);
    let root = parse(db, file).syntax();
    let items = item_tree(db, file);
    let mut cx = Lowerer {
        text: file.text(db),
        items,
        out: Lowered::default(),
        scopes: Vec::new(),
    };
    let block = match owner.item(db) {
        Item::Fn(i) => {
            let node = item_node(&root, K::FN_ITEM, i).and_then(ast::FnItem::cast);
            let sig = &items.fns[i as usize];
            for p in &sig.params {
                let local = cx.out.body.locals.alloc(Local {
                    name: p.name.clone().unwrap_or_else(|| "_".to_string()),
                    mutable: p.mutable,
                    range: p.range,
                });
                if let Some(name) = &p.name {
                    cx.scopes.push((name.clone(), local));
                }
                cx.out.body.params.push(local);
            }
            node.and_then(|f| f.body())
        }
        Item::Test(i) => item_node(&root, K::TEST_ITEM, i)
            .and_then(ast::TestItem::cast)
            .and_then(|t| t.body()),
    };
    if let Some(block) = block {
        let id = cx.block(&block);
        cx.out.body.root = Some(id);
    }
    cx.out
}

struct Lowerer<'a> {
    text: &'a str,
    items: &'a ItemTree,
    out: Lowered,
    /// Locals in scope, innermost last.
    scopes: Vec<(String, LocalId)>,
}

impl Lowerer<'_> {
    fn alloc(&mut self, kind: ExprKind, range: TextRange) -> ExprId {
        self.out.body.exprs.alloc(Expr { kind, range })
    }

    fn error(&mut self, d: Diagnostic) {
        self.out.diagnostics.push(d);
    }

    fn unsupported(&mut self, range: TextRange, what: &str) -> ExprId {
        self.error(Diagnostic::unsupported(range, what));
        self.alloc(ExprKind::Missing, range)
    }

    /// Whether a `// =>` expectation follows `end` on the same line (after an optional `;`).
    fn observed(&self, end: u32) -> bool {
        let rest = self.text[end as usize..].trim_start_matches([' ', '\t']);
        let rest = rest
            .strip_prefix(';')
            .unwrap_or(rest)
            .trim_start_matches([' ', '\t']);
        rest.starts_with("// =>")
    }

    fn block(&mut self, block: &ast::Block) -> ExprId {
        let node = block.syntax();
        if let Some(attr) = node.child(K::INNER_ATTR) {
            self.error(Diagnostic::unsupported(attr.text_range(), "an attribute"));
        }
        let mark = self.scopes.len();
        let mut stmts = Vec::new();
        for s in block.stmts() {
            match s {
                ast::Stmt::Let(l) => {
                    if let Some(stmt) = self.let_stmt(&l) {
                        stmts.push(stmt);
                    }
                }
                ast::Stmt::Expr(e) => {
                    if let Some(attr) = e.syntax().child(K::OUTER_ATTR) {
                        self.error(Diagnostic::unsupported(attr.text_range(), "an attribute"));
                    }
                    let semi = e.syntax().token(K::SEMI).is_some();
                    let Some(inner) = e.expr() else { continue };
                    let observe = self.observed(inner.syntax().text_range().end);
                    let expr = self.expr(&inner);
                    stmts.push(Stmt::Expr {
                        expr,
                        semi,
                        observe,
                    });
                }
                ast::Stmt::BlockLike(e) => {
                    let observe = self.observed(e.syntax().text_range().end);
                    let expr = self.expr(&e);
                    stmts.push(Stmt::Expr {
                        expr,
                        semi: false,
                        observe,
                    });
                }
                ast::Stmt::Item(n) => {
                    let what = if n.kind() == K::DEFER_STMT {
                        "`defer`"
                    } else {
                        "a local item"
                    };
                    self.error(Diagnostic::unsupported(n.text_range(), what));
                }
            }
        }
        let (tail, observe_tail) = match block.tail_expr() {
            Some(t) => {
                let observe = self.observed(t.syntax().text_range().end);
                (Some(self.expr(&t)), observe)
            }
            None => (None, false),
        };
        self.scopes.truncate(mark);
        self.alloc(
            ExprKind::Block {
                stmts,
                tail,
                observe_tail,
            },
            node.text_range(),
        )
    }

    fn let_stmt(&mut self, l: &ast::LetStmt) -> Option<Stmt> {
        let node = l.syntax();
        if let Some(e) = node.child(K::LET_ELSE) {
            self.error(Diagnostic::unsupported(e.text_range(), "`let … else`"));
        }
        if let Some(attr) = node.child(K::OUTER_ATTR) {
            self.error(Diagnostic::unsupported(attr.text_range(), "an attribute"));
        }
        let ty = l.ty().map(|t| resolve_type(&t, &mut self.out.diagnostics));
        let init = match l.init() {
            Some(e) => self.expr(&e),
            None => self.unsupported(node.text_range(), "`let` without an initialiser"),
        };
        let pat = l.pat()?;
        let (name, mutable, range) = binding(&pat, &mut self.out.diagnostics);
        let local = name.map(|name| {
            let id = self.out.body.locals.alloc(Local {
                name: name.clone(),
                mutable,
                range,
            });
            self.scopes.push((name, id));
            id
        });
        Some(Stmt::Let { local, ty, init })
    }

    fn lookup(&self, name: &str) -> Option<LocalId> {
        self.scopes
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|&(_, id)| id)
    }

    /// The single-segment name of a path expression, or `None` after reporting why not.
    fn path_name(&mut self, node: &SyntaxNode) -> Option<String> {
        let path = node.child(K::PATH).and_then(ast::Path::cast);
        let generic = node
            .descendants()
            .iter()
            .any(|d| matches!(d.kind(), K::GENERIC_ARGS | K::QUALIFIED_PATH));
        match path.map(|p| p.segments()) {
            Some(segments) if segments.len() == 1 && !generic => segments.into_iter().next(),
            _ => {
                self.error(Diagnostic::unsupported(node.text_range(), "this path"));
                None
            }
        }
    }

    fn expr(&mut self, e: &ast::Expr) -> ExprId {
        let node = e.syntax().clone();
        let range = node.text_range();
        match e {
            ast::Expr::Literal(_) => self.literal(&node, false),
            ast::Expr::Path(_) => {
                let Some(name) = self.path_name(&node) else {
                    return self.alloc(ExprKind::Missing, range);
                };
                if let Some(local) = self.lookup(&name) {
                    self.alloc(ExprKind::Local(local), range)
                } else if let Some(f) = self.items.fn_named(&name) {
                    self.alloc(ExprKind::Fn(f), range)
                } else {
                    self.error(Diagnostic::error(
                        Code::E0101,
                        range,
                        format!("there is nothing called `{name}` here"),
                    ));
                    self.alloc(ExprKind::Missing, range)
                }
            }
            ast::Expr::Paren(_) => match e.sub_exprs().first() {
                Some(inner) => self.expr(inner),
                None => self.alloc(ExprKind::Missing, range),
            },
            ast::Expr::Tuple(_) if e.sub_exprs().is_empty() => self.alloc(ExprKind::Unit, range),
            ast::Expr::Block(_) => match ast::Block::cast(node) {
                Some(b) => self.block(&b),
                None => self.alloc(ExprKind::Missing, range),
            },
            ast::Expr::If(_) => self.if_expr(&node, e),
            ast::Expr::Return(_) => {
                let value = e.sub_exprs().first().map(|v| self.expr(v));
                self.alloc(ExprKind::Return(value), range)
            }
            ast::Expr::Prefix(_) => self.prefix(&node, e),
            ast::Expr::Bin(_) => self.binary(&node, e),
            ast::Expr::CompareChain(_) => self.compare(&node, e),
            ast::Expr::Assign(_) => self.assign(&node, e),
            ast::Expr::Call(_) => self.call(&node, e),
            _ => self.unsupported(range, "this expression"),
        }
    }

    fn literal(&mut self, node: &SyntaxNode, negate: bool) -> ExprId {
        let range = node.text_range();
        if node.child(K::UNIT_SUFFIX).is_some() {
            return self.unsupported(range, "a unit of measure");
        }
        let Some(tok) = node.child_tokens().find(|t| !t.kind().is_trivia()) else {
            return self.alloc(ExprKind::Missing, range);
        };
        match tok.kind() {
            K::TRUE_KW => self.alloc(ExprKind::Bool(true), range),
            K::FALSE_KW => self.alloc(ExprKind::Bool(false), range),
            K::INTEGER => match integer_value(tok.text()) {
                Some(v) => self.alloc(ExprKind::Int(if negate { -v } else { v }), range),
                None => {
                    self.error(Diagnostic::error(
                        Code::E0204,
                        range,
                        format!("`{}` is too large for any integer type", tok.text()),
                    ));
                    self.alloc(ExprKind::Missing, range)
                }
            },
            _ => self.unsupported(range, "this kind of literal"),
        }
    }

    fn if_expr(&mut self, node: &SyntaxNode, e: &ast::Expr) -> ExprId {
        let range = node.text_range();
        if node.token(K::CONST_KW).is_some() {
            return self.unsupported(range, "`if const`");
        }
        let subs = e.sub_exprs();
        let cond = match subs.first() {
            Some(c) => self.expr(c),
            None => self.alloc(ExprKind::Missing, range),
        };
        let then = match subs.get(1) {
            Some(t) => self.expr(t),
            None => self.alloc(ExprKind::Missing, range),
        };
        let else_ = subs.get(2).map(|x| self.expr(x));
        self.alloc(ExprKind::If { cond, then, else_ }, range)
    }

    fn prefix(&mut self, node: &SyntaxNode, e: &ast::Expr) -> ExprId {
        let range = node.text_range();
        let Some(op) = ast::operator(node) else {
            return self.alloc(ExprKind::Missing, range);
        };
        let operand = e.sub_exprs().into_iter().next();
        let op = match op.kind() {
            K::MINUS => UnaryOp::Neg,
            K::BANG => UnaryOp::Not,
            _ => return self.unsupported(range, "this operator"),
        };
        match operand {
            Some(ast::Expr::Literal(lit))
                if op == UnaryOp::Neg && lit.token(K::INTEGER).is_some() =>
            {
                // `-9223372036854775808` is one literal: constant arithmetic on literals is exact.
                let id = self.literal(&lit, true);
                self.out.body.exprs[id].range = range;
                id
            }
            Some(x) => {
                let x = self.expr(&x);
                self.alloc(ExprKind::Unary(op, x), range)
            }
            None => self.alloc(ExprKind::Missing, range),
        }
    }

    fn operands(&mut self, e: &ast::Expr, range: TextRange) -> (ExprId, ExprId) {
        let (l, r) = e.operands();
        let l = match l {
            Some(l) => self.expr(&l),
            None => self.alloc(ExprKind::Missing, range),
        };
        let r = match r {
            Some(r) => self.expr(&r),
            None => self.alloc(ExprKind::Missing, range),
        };
        (l, r)
    }

    fn binary(&mut self, node: &SyntaxNode, e: &ast::Expr) -> ExprId {
        let range = node.text_range();
        let Some(op) = ast::operator(node) else {
            return self.alloc(ExprKind::Missing, range);
        };
        if let Some(logic) = match op.kind() {
            K::AMP_AMP => Some(LogicOp::And),
            K::PIPE_PIPE => Some(LogicOp::Or),
            _ => None,
        } {
            let (l, r) = self.operands(e, range);
            return self.alloc(ExprKind::Logic(logic, l, r), range);
        }
        if let Some(cmp) = compare_op(&op) {
            let (first, x) = self.operands(e, range);
            let rest = vec![(cmp, x)];
            return self.alloc(ExprKind::Compare { first, rest }, range);
        }
        let Some(op) = binary_op(&op) else {
            return self.unsupported(op.text_range(), &format!("`{}`", op.text()));
        };
        let (l, r) = self.operands(e, range);
        self.alloc(ExprKind::Binary(op, l, r), range)
    }

    fn compare(&mut self, node: &SyntaxNode, e: &ast::Expr) -> ExprId {
        let range = node.text_range();
        let ops: Vec<CompareOp> = node.child_tokens().filter_map(|t| compare_op(&t)).collect();
        let ascending = ops
            .iter()
            .all(|o| matches!(o, CompareOp::Lt | CompareOp::Le));
        let descending = ops
            .iter()
            .all(|o| matches!(o, CompareOp::Gt | CompareOp::Ge));
        if ops.len() > 1 && !ascending && !descending {
            self.error(Diagnostic::error(
                Code::E0002,
                range,
                "a comparison chain must run in one direction (all `<` and `<=`, or all `>` and \
                 `>=`), and `==` and `!=` do not chain",
            ));
        }
        let mut operands = e.sub_exprs().into_iter();
        let first = match operands.next() {
            Some(x) => self.expr(&x),
            None => self.alloc(ExprKind::Missing, range),
        };
        let mut rest = Vec::new();
        for op in ops {
            let x = match operands.next() {
                Some(x) => self.expr(&x),
                None => self.alloc(ExprKind::Missing, range),
            };
            rest.push((op, x));
        }
        self.alloc(ExprKind::Compare { first, rest }, range)
    }

    fn assign(&mut self, node: &SyntaxNode, e: &ast::Expr) -> ExprId {
        let range = node.text_range();
        let Some(op_tok) = ast::operator(node) else {
            return self.alloc(ExprKind::Missing, range);
        };
        let op = match op_tok.kind() {
            K::EQ => None,
            K::PLUS_EQ => Some(BinaryOp::Add),
            K::MINUS_EQ => Some(BinaryOp::Sub),
            K::STAR_EQ => Some(BinaryOp::Mul),
            K::SLASH_EQ => Some(BinaryOp::Div),
            K::PERCENT_EQ => Some(BinaryOp::Rem),
            K::AMP_EQ => Some(BinaryOp::BitAnd),
            K::PIPE_EQ => Some(BinaryOp::BitOr),
            K::CARET_EQ => Some(BinaryOp::BitXor),
            K::LT_LT_EQ => Some(BinaryOp::Shl),
            K::GT_GT_EQ => Some(BinaryOp::Shr),
            _ => {
                return self.unsupported(op_tok.text_range(), &format!("`{}`", op_tok.text()));
            }
        };
        let (target, value) = e.operands();
        let value = match value {
            Some(v) => self.expr(&v),
            None => self.alloc(ExprKind::Missing, range),
        };
        let Some(target) = target else {
            return self.alloc(ExprKind::Missing, range);
        };
        let target_range = target.syntax().text_range();
        let local = match &target {
            ast::Expr::Path(_) => {
                let Some(name) = self.path_name(target.syntax()) else {
                    return self.alloc(ExprKind::Missing, range);
                };
                match self.lookup(&name) {
                    Some(l) => l,
                    None if self.items.fn_named(&name).is_some() => {
                        self.error(Diagnostic::error(
                            Code::E0206,
                            target_range,
                            format!("cannot assign to the function `{name}`"),
                        ));
                        return self.alloc(ExprKind::Missing, range);
                    }
                    None => {
                        self.error(Diagnostic::error(
                            Code::E0101,
                            target_range,
                            format!("there is nothing called `{name}` here"),
                        ));
                        return self.alloc(ExprKind::Missing, range);
                    }
                }
            }
            ast::Expr::Field(_) | ast::Expr::Index(_) | ast::Expr::Prefix(_) => {
                return self.unsupported(target_range, "assigning to this place");
            }
            _ => {
                self.error(Diagnostic::error(
                    Code::E0206,
                    target_range,
                    "only a variable can be assigned to",
                ));
                return self.alloc(ExprKind::Missing, range);
            }
        };
        let data = &self.out.body.locals[local];
        if !data.mutable {
            let d = Diagnostic::error(
                Code::E0301,
                target_range,
                format!(
                    "`{}` is not declared `mut`, so it cannot be assigned to",
                    data.name
                ),
            )
            .with_label(data.range, "declared here");
            self.error(d);
        }
        self.alloc(ExprKind::Assign { local, op, value }, range)
    }

    fn call(&mut self, node: &SyntaxNode, e: &ast::Expr) -> ExprId {
        let range = node.text_range();
        let callee = match e.sub_exprs().first() {
            Some(c) => self.expr(c),
            None => self.alloc(ExprKind::Missing, range),
        };
        let mut args = Vec::new();
        if let Some(list) = node.child(K::ARG_LIST).and_then(ast::ArgList::cast) {
            for a in list.args() {
                if let Some(label) = a.label() {
                    self.error(Diagnostic::unsupported(
                        label.text_range(),
                        "a named argument",
                    ));
                }
                match a.expr() {
                    Some(x) => args.push(self.expr(&x)),
                    None => args.push(self.alloc(ExprKind::Missing, a.syntax().text_range())),
                }
            }
        }
        self.alloc(ExprKind::Call { callee, args }, range)
    }
}

fn compare_op(tok: &SyntaxToken) -> Option<CompareOp> {
    Some(match tok.kind() {
        K::EQ_EQ => CompareOp::Eq,
        K::BANG_EQ => CompareOp::Ne,
        K::LT => CompareOp::Lt,
        K::LT_EQ => CompareOp::Le,
        K::GT => CompareOp::Gt,
        K::GT_EQ => CompareOp::Ge,
        _ => return None,
    })
}

fn binary_op(tok: &SyntaxToken) -> Option<BinaryOp> {
    Some(match tok.kind() {
        K::PLUS => BinaryOp::Add,
        K::MINUS => BinaryOp::Sub,
        K::STAR => BinaryOp::Mul,
        K::SLASH => BinaryOp::Div,
        K::PERCENT => BinaryOp::Rem,
        K::AMP => BinaryOp::BitAnd,
        K::PIPE => BinaryOp::BitOr,
        K::CARET => BinaryOp::BitXor,
        K::LT_LT => BinaryOp::Shl,
        K::GT_GT => BinaryOp::Shr,
        _ => return None,
    })
}

/// The value of an `INTEGER` token, if it fits in an `i128`.
fn integer_value(text: &str) -> Option<i128> {
    let digits: String = text.chars().filter(|&c| c != '_').collect();
    let (radix, digits) = match digits.get(..2) {
        Some("0x") => (16, &digits[2..]),
        Some("0o") => (8, &digits[2..]),
        Some("0b") => (2, &digits[2..]),
        _ => (10, digits.as_str()),
    };
    i128::from_str_radix(digits, radix).ok()
}

#[cfg(test)]
mod tests {
    use super::integer_value;

    #[test]
    fn integer_literals() {
        assert_eq!(integer_value("1_000"), Some(1000));
        assert_eq!(integer_value("0xff"), Some(255));
        assert_eq!(integer_value("0o17"), Some(15));
        assert_eq!(integer_value("0b1010"), Some(10));
        assert_eq!(integer_value("0x_"), None);
        assert_eq!(integer_value(&"9".repeat(50)), None);
    }
}
