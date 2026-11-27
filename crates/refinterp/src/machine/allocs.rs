//! The allocator in scope (`alloc.with.1`): `with Alloc = a` installs `a` among the handlers,
//! and a buffer made while one is installed asks it for its storage, and gives the storage back
//! to it when it is dropped or given up (`alloc.keep.1`), wherever that happens.
//!
//! Values are kept, not bytes: a buffer's elements stay in its value, and the allocator is
//! called so that what it is asked for and given back is what it counts, `8` bytes an element,
//! aligned to `8`. Its own calls run with the default allocator in scope (`alloc.self.1`): an
//! entry of [`Value::Moved`] among the handlers.

use std::collections::HashMap;

use moddil_syntax::TextRange;

use crate::Eval;
use crate::program::{Def, PRELUDE_MODULE};
use crate::value::{Cap, Value};

use super::Machine;

/// The bytes an allocator is asked for, for each element of a buffer.
const ELEM: i64 = 8;

impl Machine<'_> {
    /// The key the prelude's `Alloc` is installed under.
    fn alloc_key(&self) -> String {
        match self.program.resolve(PRELUDE_MODULE, &["Alloc".to_string()]) {
            Some((Def::Item(k), _)) => k,
            _ => "Alloc".to_string(),
        }
    }

    /// The innermost allocator installed, unless it is the default.
    pub(super) fn alloc_top(&self) -> Option<Value> {
        let key = self.alloc_key();
        match self.handlers.iter().rev().find(|(k, _)| *k == key) {
            Some((_, Value::Moved)) | None => None,
            Some((_, v)) => Some(v.clone()),
        }
    }

    /// The prelude's allocator intrinsic `name` on `args`, if it is one.
    pub(super) fn alloc_intrinsic(&mut self, name: &str, args: &[Value]) -> Option<Value> {
        Some(match name {
            "__alloc_push" => {
                let key = self.alloc_key();
                self.handlers.push((key, args[0].clone()));
                Value::Unit
            }
            "__alloc_push_none" => {
                let key = self.alloc_key();
                self.handlers.push((key, Value::Moved));
                Value::Unit
            }
            "__alloc_pop" => {
                let key = self.alloc_key();
                if let Some(i) = self.handlers.iter().rposition(|(k, _)| *k == key) {
                    self.handlers.remove(i);
                }
                Value::Unit
            }
            "__alloc_top" => super::tasks::option(self.alloc_top()),
            // Memory of its own, which nothing reads here.
            "__raw_alloc" => Value::Ref(self.memory.alloc(Value::Unit), true),
            "__raw_free" | "__raw_move" => Value::Unit,
            _ => return None,
        })
    }

    /// Calls the prelude's function `name` with `args`.
    fn call_prelude(&mut self, name: &str, args: Vec<Value>, range: TextRange) -> Eval {
        let Some((Def::Item(k), _)) = self.program.resolve(PRELUDE_MODULE, &[name.to_string()])
        else {
            unreachable!("the prelude's `{name}`")
        };
        let f = self.program.fns[&k].clone();
        self.invoke(&f, args, HashMap::new(), range)
    }

    /// A buffer of capacity `n` made (`alloc.make.1`): from the allocator in scope, if one is
    /// installed.
    pub(super) fn buf_cap(&mut self, n: usize, range: TextRange) -> Eval<Cap> {
        let Some(a) = self.alloc_top() else {
            return Ok(Cap::new(n));
        };
        let size = n as i64 * ELEM;
        let args = vec![a.clone(), Value::Int(size), Value::Int(ELEM)];
        let p = self.call_prelude("__alloc_by", args, range)?;
        Ok(Cap {
            n,
            owner: Some(Box::new(record(a, p, size))),
        })
    }

    /// A buffer's capacity `cap` changed to `n` (`alloc.keep.1`): by the allocator it came from.
    pub(super) fn buf_recap(&mut self, cap: Cap, n: usize, range: TextRange) -> Eval<Cap> {
        let Some(rec) = cap.owner else {
            return Ok(Cap::new(n));
        };
        let Value::Tuple(r) = &*rec else {
            unreachable!("an allocation")
        };
        let a = r[0].clone();
        let size = n as i64 * ELEM;
        let p = self.call_prelude("__realloc_by", vec![*rec, Value::Int(size)], range)?;
        Ok(Cap {
            n,
            owner: Some(Box::new(record(a, p, size))),
        })
    }

    /// A buffer of capacity `cap` dropped or given up: its storage goes back.
    pub(super) fn buf_free(&mut self, cap: Cap, range: TextRange) -> Eval<()> {
        if let Some(rec) = cap.owner {
            self.call_prelude("__free_by", vec![*rec], range)?;
        }
        Ok(())
    }
}

/// What `__free_by` takes: the allocator, the allocation, its size and its alignment.
fn record(a: Value, p: Value, size: i64) -> Value {
    Value::Tuple(vec![a, p, Value::Int(size), Value::Int(ELEM)])
}
