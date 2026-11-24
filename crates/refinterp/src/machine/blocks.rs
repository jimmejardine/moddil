//! Blocks and statements: scopes, `defer` blocks, `let` and expression statements.

use moddil_syntax::SyntaxKind as K;
use moddil_syntax::ast::{self, AstNode};

use crate::value::Value;
use crate::{Eval, Flow};

use super::Machine;
use super::patterns::Src;

impl Machine<'_> {
    /// A block. Leaving it in any way but a trap runs its `defer` blocks and drops its
    /// variables, in the reverse of the order they were reached (`stmt.defer.1`,
    /// `own.drop.1`). Leaving it with an error also runs its `errdefer` blocks, whose own
    /// errors are suppressed (`stmt.errdefer.1`).
    pub(super) fn block(&mut self, b: &ast::Block) -> Eval {
        let range = b.syntax().text_range();
        let mark = self.memory.mark();
        self.defers.push(Vec::new());
        let mut result = self.block_inner(b);
        // A trap in an isolated scope unwinds the task (`task.isolate.1`); once it unwinds,
        // cancellation is not seen again.
        if let Err(f @ Flow::Trap(..)) = result {
            result = Err(self.isolate(f));
        }
        if matches!(result, Err(Flow::Unwind)) {
            self.unwinding();
        }
        let defers = self.defers.pop().expect("this block's defers");
        if !matches!(result, Err(Flow::Trap(..))) {
            for (d, at, err) in defers.iter().rev() {
                // The variables declared after the `defer` are dropped before it runs.
                result = self.exit(*at, result, range);
                if matches!(result, Err(Flow::Trap(..))) {
                    break;
                }
                if *err {
                    if !matches!(result, Err(Flow::Fail(..) | Flow::Unwind)) {
                        continue;
                    }
                    match self.block(d) {
                        Ok(_) | Err(Flow::Fail(..)) => {}
                        Err(f) => {
                            result = Err(f);
                            break;
                        }
                    }
                    continue;
                }
                if let Err(f) = self.block(d) {
                    result = Err(f);
                    break;
                }
            }
        }
        // A closure the block gives keeps what it captures of the block's variables
        // (`expr.closure.result.1`).
        if let Ok(v) = &mut result {
            self.keep_captures(v, mark);
        }
        self.exit(mark, result, range)
    }

    pub(super) fn block_inner(&mut self, b: &ast::Block) -> Eval {
        for s in b.stmts() {
            let mark = self.memory.mark();
            let range = match &s {
                ast::Stmt::Let(l) => l.syntax().text_range(),
                ast::Stmt::Expr(e) => e.syntax().text_range(),
                ast::Stmt::BlockLike(x) => x.syntax().text_range(),
                ast::Stmt::Item(n) => n.text_range(),
            };
            match s {
                ast::Stmt::Let(l) => self.let_stmt(&l)?,
                ast::Stmt::Expr(e) => {
                    let x = e.expr().expect("a checked statement");
                    self.expr_stmt(&x)?;
                }
                ast::Stmt::BlockLike(x) => self.expr_stmt(&x)?,
                ast::Stmt::Item(n) => {
                    if n.kind() == K::DEFER_STMT
                        && let Some(d) = n.child(K::BLOCK).and_then(ast::Block::cast)
                    {
                        let at = self.memory.mark();
                        let err = n.token(K::ERRDEFER_KW).is_some();
                        self.defers.last_mut().expect("a block").push((d, at, err));
                    }
                }
            }
            // Temporaries are dropped at the end of their statement.
            self.drop_temps(mark, range)?;
        }
        match b.tail_expr() {
            Some(t) => {
                let mark = self.memory.mark();
                let v = self.eval(&t)?;
                self.observe(t.syntax().text_range(), &v);
                self.drop_temps(mark, t.syntax().text_range())?;
                Ok(v)
            }
            None => Ok(Value::Unit),
        }
    }

    /// An expression statement: a place's value is read but not moved (`stmt.expr.1`); any
    /// other value is dropped.
    pub(super) fn expr_stmt(&mut self, x: &ast::Expr) -> Eval<()> {
        let range = x.syntax().text_range();
        if let Some((p, _)) = self.place_of(x)? {
            let v = self.memory.load(&p);
            self.observe(range, &v);
            return Ok(());
        }
        let v = self.eval(x)?;
        self.observe(range, &v);
        self.drop_value(v, true, range)
    }

    /// `let p = e;` and `let p = e else { … };` (`stmt.let.1`, `stmt.let.else.1`).
    pub(super) fn let_stmt(&mut self, l: &ast::LetStmt) -> Eval<()> {
        let Some(init) = l.init() else {
            // `let x;`: the name, with no value until it is assigned (`stmt.let.uninit.1`).
            let pat = l.pat().expect("a checked `let` has a pattern");
            let src = Src::Place(self.memory.temp(Value::Moved), false);
            let mut binds = Vec::new();
            self.matches(&pat, &src, &mut binds);
            self.bind(binds);
            return Ok(());
        };
        // A literal takes the number type written (`num.lit.1`).
        let src = match l.ty() {
            Some(t)
                if {
                    let n = t.syntax().text().to_string();
                    let n = n.trim();
                    crate::num::Kind::named(n).is_some() || n == "int" || n == "i64"
                } =>
            {
                let v = self.eval_as(&init, Some(&t))?;
                Src::Place(self.memory.temp(self.typed(v, Some(&t))), false)
            }
            _ => self.scrutinee(&init)?,
        };
        let pat = l.pat().expect("a checked `let` has a pattern");
        let mut binds = Vec::new();
        let matched = self.matches(&pat, &src, &mut binds);
        self.matched(l.syntax().text_range())?;
        if matched {
            self.bind(binds);
            return Ok(());
        }
        let else_ = l
            .syntax()
            .child(K::LET_ELSE)
            .and_then(|e| e.child(K::BLOCK))
            .and_then(ast::Block::cast)
            .expect("a refutable `let` has an `else`");
        // The `else` block never finishes normally, so this returns its `Flow`.
        self.block(&else_)?;
        unreachable!("the `else` block of `let … else` diverges")
    }

    /// What a pattern matches against the value of `e`: `e`'s own place if it is one, or else
    /// a nameless variable holding its value.
    pub(super) fn scrutinee(&mut self, e: &ast::Expr) -> Eval<Src> {
        if let Some((p, indirect)) = self.place_of(e)? {
            return Ok(Src::Place(p, indirect));
        }
        let v = self.eval(e)?;
        Ok(Src::Place(self.memory.temp(v), false))
    }
}
