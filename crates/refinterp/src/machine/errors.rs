//! Errors (`errors.md`): `fail`, `?`, `?!`, `try` and `catch`. An error travels as
//! [`Flow::Fail`] until a `try` block or a `catch` takes it, or it leaves the test; `?` on `None`
//! travels as [`Flow::Absent`] to the function or `try` block it ends.

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{SyntaxKind as K, SyntaxNode, TextRange};

use crate::value::Value;
use crate::{Eval, Flow, TrapClass, trap};

use super::patterns::{Bind, Src};
use super::{Machine, segments};

/// A prelude enum's variant, as a value.
fn variant(ty: &str, name: &str, fields: Vec<Value>) -> Value {
    Value::Adt {
        ty: ty.to_string(),
        variant: name.to_string(),
        fields,
    }
}

impl Machine<'_> {
    /// `x?` and `x ?! "reason"` (`err.question.1`, `err.bang.1`): the value of a call, an `Ok` or
    /// a `Some`; or else the error (or `None`) goes on, or, for `?!`, traps.
    pub(super) fn question(&mut self, node: &SyntaxNode, e: &ast::Expr, bang: bool) -> Eval {
        let range = node.text_range();
        let inner = &e.sub_exprs()[0];
        let unexpected = || trap(TrapClass::Unexpected, range);
        let v = match self.eval(inner) {
            Ok(v) => v,
            Err(Flow::Fail(_, _)) if bang => return Err(unexpected()),
            Err(Flow::Fail(err, _)) => {
                self.set_mode(false);
                return Err(Flow::Fail(err, range));
            }
            Err(f) => return Err(f),
        };
        match &v {
            Value::Adt {
                ty,
                variant,
                fields,
            } if ty == "Result" && self.is_prelude(ty) => {
                self.set_mode(false);
                match variant.as_str() {
                    "Ok" => Ok(fields[0].clone()),
                    _ if bang => Err(unexpected()),
                    _ => Err(Flow::Fail(fields[0].clone(), range)),
                }
            }
            Value::Adt {
                ty,
                variant,
                fields,
            } if ty == "Option" && self.is_prelude(ty) => {
                self.set_mode(true);
                match variant.as_str() {
                    "Some" => Ok(fields[0].clone()),
                    _ if bang => Err(unexpected()),
                    _ => Err(Flow::Absent),
                }
            }
            // The value of a call that can fail and did not.
            _ => Ok(v),
        }
    }

    fn is_prelude(&self, ty: &str) -> bool {
        self.program.adts.get(ty).is_some_and(|a| a.bare)
    }

    /// The innermost `try` block learns from a `?` whether it is over `Option`s.
    fn set_mode(&mut self, option: bool) {
        if let Some(m) = self.try_modes.last_mut()
            && m.is_none()
        {
            *m = Some(option);
        }
    }

    /// `fail(e)` (`err.fail.1`).
    pub(super) fn fail(&mut self, node: &SyntaxNode, e: &ast::Expr) -> Eval {
        let v = self.eval(&e.sub_exprs()[0])?;
        Err(Flow::Fail(v, node.text_range()))
    }

    /// `try { … }`, an `Ok`, `Err`, `Some` or `None`; and `try { … } catch { arms }`, whose arms
    /// handle the errors they match, letting the others go on (`err.try.1`, `err.catch.1`).
    pub(super) fn try_expr(&mut self, node: &SyntaxNode) -> Eval {
        let range = node.text_range();
        let block = node
            .child(K::BLOCK)
            .and_then(ast::Block::cast)
            .expect("a `try` block");
        self.try_modes.push(None);
        let result = self.block(&block);
        let option = self.try_modes.pop().flatten() == Some(true);
        let Some(arms) = node
            .child(K::CATCH)
            .and_then(|c| c.child(K::MATCH_ARM_LIST))
        else {
            return match result {
                Ok(v) if option => Ok(variant("Option", "Some", vec![v])),
                Ok(v) => Ok(variant("Result", "Ok", vec![v])),
                Err(Flow::Absent) => Ok(variant("Option", "None", Vec::new())),
                Err(Flow::Fail(err, _)) => Ok(variant("Result", "Err", vec![err])),
                Err(f) => Err(f),
            };
        };
        let err = match result {
            Err(Flow::Fail(err, _)) => err,
            other => return other,
        };
        let slot = self.memory.temp(err.clone());
        let src = Src::Place(slot.clone(), false);
        for arm in arms.children().filter(|c| c.kind() == K::MATCH_ARM) {
            let pat = arm.children().find_map(ast::Pat::cast).expect("a pattern");
            let mut binds = Vec::new();
            if !self.catches(&pat, &err, &src, &mut binds) {
                continue;
            }
            let mark = self.memory.mark();
            let guard = arm
                .child(K::MATCH_GUARD)
                .and_then(|g| g.children().find_map(ast::Expr::cast));
            if let Some(g) = guard {
                let copies = binds
                    .iter()
                    .map(|(n, v, _)| (n.clone(), v.clone(), None))
                    .collect();
                self.bind(copies);
                let go = self.eval(&g);
                self.memory.truncate(mark);
                if go? != Value::Bool(true) {
                    continue;
                }
            }
            self.bind(binds);
            let body = arm.children().filter_map(ast::Expr::cast).last();
            let result = self.eval(&body.expect("an arm's value"));
            return self.exit(mark, result, range);
        }
        // No arm matches: the error goes on.
        let err = self.memory.load(&slot);
        Err(Flow::Fail(err, range))
    }

    /// Whether a `catch` arm's pattern matches error `err` (held at `src`): `Type(p)` matches an
    /// error of that type that `p` matches; any other pattern is matched as usual.
    fn catches(&self, pat: &ast::Pat, err: &Value, src: &Src, binds: &mut Vec<Bind>) -> bool {
        let node = pat.syntax();
        if let ast::Pat::TupleStruct(_) = pat {
            let segs = segments(node);
            if let [one] = segs.as_slice()
                && let Some(crate::program::Def::Item(key)) =
                    self.program.lookup(self.memory.module(), one)
                && self.program.adts.contains_key(&key)
            {
                let subs: Vec<ast::Pat> = node.children().filter_map(ast::Pat::cast).collect();
                if let [sub] = subs.as_slice()
                    && !matches!(sub, ast::Pat::Rest(_))
                {
                    let of_type = matches!(err, Value::Adt { ty, .. } if *ty == key);
                    return of_type && self.matches(sub, src, binds);
                }
            }
        }
        // A variant or struct pattern applies only to errors of its type.
        if let Some((ty, _)) = self.ctor(&segments(node))
            && !matches!(err, Value::Adt { ty: t, .. } if *t == ty)
        {
            return false;
        }
        self.matches(pat, src, binds)
    }

    /// `discard(r, "reason")`: `r` is dropped on purpose.
    pub(super) fn discard(&mut self, args: &[ast::Expr], range: TextRange) -> Eval {
        let v = self.eval(&args[0])?;
        self.drop_value(v, true, range)?;
        Ok(Value::Unit)
    }
}
