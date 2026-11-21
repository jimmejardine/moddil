//! Closures (`expressions/closures.md`): a closure value remembers the variables in scope where
//! it was written, and its body sees them as its own (aliases of them, so it reads and changes
//! the variables themselves, and a move out of one leaves it empty), and the handlers installed
//! there, which its calls use wherever it is called.

use std::sync::Arc;

use moddil_syntax::ast;
use moddil_syntax::{SyntaxKind as K, SyntaxNode, TextRange};

use crate::value::{Closure, Value};
use crate::{Eval, Flow, MAX_DEPTH, TrapClass, trap};

use super::Machine;
use super::patterns::Src;

impl Machine<'_> {
    /// `|p, …| body`: a closure value.
    pub(super) fn closure_value(&mut self, node: &SyntaxNode) -> Value {
        Value::Closure(Arc::new(Closure {
            node: node.clone(),
            vars: self.memory.visible(),
            handlers: self.handlers.clone(),
            types: self.memory.types(),
            file: self.memory.file(),
            module: self.memory.module(),
        }))
    }

    /// `impl Trait { fn … }`: each method a closure over where it is written.
    pub(super) fn anon_value(&mut self, node: &SyntaxNode) -> Value {
        let path = node
            .child(K::PATH_TYPE)
            .map(|t| crate::program::type_path(&t))
            .unwrap_or_default();
        let key = match self.program.resolve(self.memory.module(), &path) {
            Some((crate::program::Def::Item(k), _)) => k,
            _ => path.join("::"),
        };
        let methods = node
            .child(K::IMPL_BODY)
            .into_iter()
            .flat_map(|b| b.children().collect::<Vec<_>>())
            .filter(|c| c.kind() == K::FN_ITEM)
            .map(|f| {
                let name = f
                    .child(K::NAME)
                    .and_then(ast::Name::cast)
                    .map(|n| n.text())
                    .unwrap_or_default();
                let Value::Closure(c) = self.closure_value(&f) else {
                    unreachable!("a closure")
                };
                (name, c)
            })
            .collect();
        Value::Anon(key, methods)
    }

    /// Calls closure `c` with `args`: its parameters are matched against them, and its body runs
    /// with the variables and handlers of where it was written. A closure whose node is a
    /// method (of an anonymous impl) takes the method's parameters but `self`.
    pub(super) fn call_closure(&mut self, c: &Closure, args: Vec<Value>, range: TextRange) -> Eval {
        if self.memory.depth() >= MAX_DEPTH {
            return Err(trap(TrapClass::StackOverflow, range));
        }
        let caller_file = self.memory.file().unwrap_or(0);
        self.memory
            .push_frame(c.types.clone(), c.file, c.module, false);
        for (name, p) in &c.vars {
            self.memory.push(name.clone(), Value::Alias(p.clone()));
        }
        let method = c.node.kind() == K::FN_ITEM;
        let params: Vec<SyntaxNode> = if method {
            c.node
                .child(K::PARAM_LIST)
                .map(|ps| ps.children().filter(|p| p.kind() == K::PARAM).collect())
                .unwrap_or_default()
        } else {
            c.node
                .child(K::CLOSURE_PARAMS)
                .map(|ps| {
                    ps.children()
                        .filter(|p| p.kind() == K::CLOSURE_PARAM)
                        .collect()
                })
                .unwrap_or_default()
        };
        for (p, v) in params.iter().zip(args) {
            let pat = p
                .children()
                .find_map(ast::Pat::cast)
                .expect("a parameter pattern");
            let slot = self.memory.temp(v);
            let mut binds = Vec::new();
            let matched = self.matches(&pat, &Src::Place(slot, false), &mut binds);
            debug_assert!(matched, "parameter patterns are irrefutable");
            self.bind(binds);
        }
        let saved = std::mem::replace(&mut self.handlers, c.handlers.clone());
        let saved_modes = std::mem::take(&mut self.try_modes);
        let body = if method {
            c.node
                .child(K::BLOCK)
                .and_then(ast::Expr::cast)
                .expect("a method's body")
        } else {
            c.node
                .children()
                .filter_map(ast::Expr::cast)
                .last()
                .expect("a closure's body")
        };
        let result = match self.eval(&body) {
            Ok(v) | Err(Flow::Return(v)) => Ok(v),
            Err(Flow::Absent) => Ok(Value::Adt {
                ty: "Option".to_string(),
                variant: "None".to_string(),
                fields: Vec::new(),
            }),
            Err(f) => Err(f),
        };
        self.handlers = saved;
        self.try_modes = saved_modes;
        let result = if matches!(result, Err(Flow::Trap(..))) {
            result
        } else {
            let mut result = result;
            while let Some((_, v)) = self.memory.pop() {
                if let Err(e) = self.drop_value(v, true, range) {
                    result = Err(e);
                    break;
                }
            }
            result
        };
        self.memory.pop_frame();
        match result {
            Err(Flow::Trap(mut t, false)) => {
                t.file = c.file.unwrap_or(caller_file);
                Err(Flow::Trap(t, true))
            }
            r => r,
        }
    }
}
