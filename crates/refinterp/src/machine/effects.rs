//! Effects (`effects.md`): `with` installs handlers for a block, and a function that names an
//! effect in its signature (`with time: Time`) is given the innermost handler for exactly that
//! effect. Handlers are kept on a stack in dynamic scope, rather than passed from caller to
//! callee as the compiler does, so the two cross-check.

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{SyntaxKind as K, SyntaxNode};

use crate::Eval;
use crate::program::{Def, type_path};
use crate::value::{Head, Value};

use super::Machine;

/// The name a handler that is a value is kept under while its `with` block runs: a keyword, so
/// no variable has it.
const HELD: &str = "with";

impl Machine<'_> {
    /// `with E = value, … { body }` (`eff.with.1`): the handlers are evaluated in order, then
    /// installed for the block. A handler that is a value lives until the block ends, and is then
    /// dropped; one that is a borrow is installed as it is.
    pub(super) fn with_expr(&mut self, node: &SyntaxNode) -> Eval {
        let range = node.text_range();
        let mark = self.memory.mark();
        let mut installed = Vec::new();
        for h in node.children().filter(|c| c.kind() == K::HANDLER) {
            let key = h
                .children()
                .find_map(ast::Type::cast)
                .map(|t| self.effect_key(t.syntax()))
                .unwrap_or_default();
            let value = h
                .children()
                .find_map(ast::Expr::cast)
                .expect("a checked handler");
            let v = match self.eval(&value) {
                Ok(v) => v,
                Err(f) => return self.exit(mark, Err(f), range),
            };
            let handler = match v {
                v @ Value::Ref(..) => v,
                v => {
                    self.memory.push(HELD.to_string(), v);
                    Value::Ref(self.memory.here(self.memory.mark() - 1), false)
                }
            };
            installed.push((key, handler));
        }
        let depth = self.handlers.len();
        self.handlers.extend(installed);
        let body = node
            .child(K::BLOCK)
            .and_then(ast::Block::cast)
            .expect("a checked `with`");
        let result = self.block(&body);
        self.handlers.truncate(depth);
        self.exit(mark, result, range)
    }

    /// Binds the effects the signature `f` names (`with time: Time`, `config: &AppConfig`) to the
    /// innermost handlers installed for them, in the call's frame (`eff.name.1`).
    pub(super) fn bind_effects(&mut self, f: &ast::FnItem) {
        let Some(clause) = f.syntax().child(K::EFFECT_CLAUSE) else {
            return;
        };
        for e in clause.children().filter(|c| c.kind() == K::SIG_EFFECT) {
            let (Some(name), Some(effect)) = (e.token(K::IDENT), e.child(K::EFFECT)) else {
                continue;
            };
            let Some(t) = effect.children().find_map(ast::Type::cast) else {
                continue;
            };
            let key = self.effect_key(t.syntax());
            let handler = self
                .handlers
                .iter()
                .rev()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.clone())
                .unwrap_or(Value::Moved);
            self.memory.push(name.text().to_string(), handler);
        }
    }

    /// How an effect written as the type `t` is looked up: the item each path names (so two
    /// modules naming one trait differently agree), with its type arguments, and the running
    /// call's type parameters replaced by the types they stand for (`Store<T>` as `Store<User>`).
    fn effect_key(&self, t: &SyntaxNode) -> String {
        match t.kind() {
            K::REF_TYPE | K::PAREN_TYPE => t
                .children()
                .find(|c| ast::Type::can_cast(c.kind()))
                .map(|c| self.effect_key(&c))
                .unwrap_or_default(),
            K::PATH_TYPE => {
                let path = type_path(t);
                let base = match path.as_slice() {
                    [one] if self.memory.type_of(one).is_some() => {
                        match self.memory.type_of(one).expect("known") {
                            Head::Adt(k) => k.clone(),
                            Head::Int => "int".to_string(),
                            Head::Num(k) => format!("{k:?}").to_lowercase(),
                            Head::Bool => "bool".to_string(),
                            Head::Char => "char".to_string(),
                            Head::Str => "str".to_string(),
                            other => format!("{other:?}"),
                        }
                    }
                    _ => match self.program.resolve(self.memory.module(), &path) {
                        Some((Def::Item(k), _)) => k,
                        _ => path.join("::"),
                    },
                };
                let args: Vec<String> = t
                    .child(K::PATH)
                    .and_then(|p| {
                        p.children()
                            .filter(|c| c.kind() == K::PATH_SEGMENT)
                            .last()
                            .and_then(|s| s.child(K::GENERIC_ARGS))
                            .or_else(|| p.child(K::GENERIC_ARGS))
                    })
                    .map(|g| {
                        g.children()
                            .filter(|c| c.kind() == K::TYPE_ARG)
                            .filter_map(|a| a.children().find(|c| ast::Type::can_cast(c.kind())))
                            .map(|a| self.effect_key(&a))
                            .collect()
                    })
                    .unwrap_or_default();
                if args.is_empty() {
                    base
                } else {
                    format!("{base}<{}>", args.join(","))
                }
            }
            _ => t
                .tokens()
                .iter()
                .filter(|x| !x.kind().is_trivia())
                .map(|x| x.text().to_string())
                .collect(),
        }
    }
}
