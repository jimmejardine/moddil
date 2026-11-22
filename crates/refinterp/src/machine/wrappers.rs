//! Function wrappers (`attr.around.1`): the body of a function with `#[around(w)]` is a call of
//! `w`, given the function's description, its arguments as one `f::Args` value, and a closure
//! taking them back that runs the next wrapper, or the body.

use std::sync::Arc;

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{SyntaxKind as K, TextRange};

use crate::Eval;

use crate::program::{Def, FnDef, Wrapping};
use crate::value::{Closure, Pointer, Value};

use super::{Machine, segments};

impl Machine<'_> {
    /// The body of `f` from its wrapper `k` on: that wrapper's call, or, past the last, the
    /// body itself.
    pub(super) fn wrapped_body(&mut self, f: &FnDef, k: usize, range: TextRange) -> Eval {
        let body = f.node.body().expect("a body");
        let Some(w) = f.wrapping.clone().filter(|w| k < w.wrappers.len()) else {
            return self.block(&body);
        };
        let e = &w.wrappers[k];
        let (callee, consts) = if e.syntax().kind() == K::CALL_EXPR {
            let args: Vec<ast::Expr> = e
                .syntax()
                .child(K::ARG_LIST)
                .and_then(ast::ArgList::cast)
                .map(|l| l.args().filter_map(|a| a.expr()).collect())
                .unwrap_or_default();
            (e.sub_exprs()[0].clone(), args)
        } else {
            (e.clone(), Vec::new())
        };
        let Some((Def::Item(key), _)) = self.resolve(&segments(callee.syntax())) else {
            unreachable!("a checked wrapper")
        };
        let wrapper = self.program.fns[&key].clone();
        let mut values = self.eval_all(&consts)?;
        values.push(Value::Ref(self.fn_info(&w), false));
        // The arguments, moved into `f::Args`.
        let mut fields = Vec::new();
        for (name, _) in &w.params {
            let slot = self.memory.slot(name).expect("a parameter");
            let p = self.memory.here(slot);
            fields.push(self.take(&p, false));
        }
        let name = self.program.adts[&w.args].name.clone();
        values.push(Value::Adt {
            ty: w.args.clone(),
            variant: name,
            fields,
        });
        values.push(Value::Closure(Arc::new(Closure {
            node: self.program.node_ref(f.node.syntax()),
            vars: self.memory.visible(),
            handlers: self.handlers.clone(),
            types: self.memory.types(),
            file: self.memory.file(),
            module: self.memory.module(),
            escaped: false,
            wrapped: Some((w.key.clone(), k + 1)),
        })));
        let types = self.turbofish(None, &wrapper.params);
        self.invoke(&wrapper, values, types, range)
    }

    /// Where the wrapped function's `FnInfo` is: static data, made the first time.
    fn fn_info(&mut self, w: &Wrapping) -> Pointer {
        let key = format!("{}::INFO", w.key);
        if let Some(p) = self.memory.static_of(&key) {
            return p;
        }
        let string = |s: &str| crate::value::make_string(s);
        let params: Vec<Value> = w
            .params
            .iter()
            .map(|(n, t)| Value::Adt {
                ty: "ParamInfo".to_string(),
                variant: "ParamInfo".to_string(),
                fields: vec![string(n), string(t)],
            })
            .collect();
        let vec = Value::Adt {
            ty: "Vec".to_string(),
            variant: "Vec".to_string(),
            fields: vec![Value::Buf(params.len(), params)],
        };
        let info = Value::Adt {
            ty: "FnInfo".to_string(),
            variant: "FnInfo".to_string(),
            fields: vec![string(&w.path), vec],
        };
        self.memory.add_static(key, info)
    }
}
