//! The allocator in scope (`alloc.with.1`) on the MIR interpreter: each task's stack of the
//! allocators `with Alloc` installs, and the calls of an allocator for the buffers made where
//! one is installed.
//!
//! The interpreter keeps values, not bytes, so a buffer's elements stay in its value; the
//! allocator is called so that what it is asked for and given back is what it counts. A
//! buffer's allocation is `8` bytes an element, aligned to `8` (compiled code asks for its
//! elements' real sizes).

use moddil_sema::{Container, Item, Ty};

use super::Machine;
use crate::stack::{Frame, Ret};
use crate::value::{Pointer, Value};

/// The bytes the interpreter asks an allocator for, for each element of a buffer.
const ELEM: i64 = 8;

impl Machine<'_> {
    /// The scheduler's allocator operation `name` on `values`, or `None` if `name` is not one.
    pub(super) fn alloc_op(&mut self, name: &str, values: &[Value]) -> Option<Value> {
        let me = self.stack.current();
        Some(match name {
            "alloc_push" => {
                self.allocs
                    .entry(me)
                    .or_default()
                    .push(Some(values[0].clone()));
                Value::Unit
            }
            "alloc_push_none" => {
                self.allocs.entry(me).or_default().push(None);
                Value::Unit
            }
            "alloc_pop" => {
                if let Some(v) = self.allocs.get_mut(&me) {
                    v.pop();
                }
                Value::Unit
            }
            "alloc_top" => {
                let top = self.alloc_top(me);
                self.natives.option_of(top)
            }
            "raw_alloc" => Value::Foreign(crate::cmem::raw_alloc(
                values[0].as_int(),
                values[1].as_int(),
            )),
            "raw_free" => {
                if let Value::Foreign(p) = values[0] {
                    crate::cmem::raw_free(p, values[1].as_int(), values[2].as_int());
                }
                Value::Unit
            }
            "raw_move" => {
                if let (Value::Foreign(d), Value::Foreign(s)) = (&values[0], &values[1]) {
                    crate::cmem::raw_move(*d, *s, values[2].as_int());
                }
                Value::Unit
            }
            _ => return None,
        })
    }

    /// The innermost allocator task `t` has installed, unless it is the default.
    fn alloc_top(&self, t: usize) -> Option<Value> {
        self.allocs
            .get(&t)
            .and_then(|v| v.last())
            .cloned()
            .flatten()
    }

    /// Task `t`, started by the running task, starts with its allocator in scope
    /// (`alloc.task.1`).
    pub(super) fn inherit_alloc(&mut self, t: usize) {
        let me = self.stack.current();
        match self.allocs.get(&me).and_then(|v| v.last()).cloned() {
            Some(top) => {
                self.allocs.insert(t, vec![top]);
            }
            None => {
                self.allocs.remove(&t);
            }
        }
    }

    /// After built-in function `f` of `Buf` gave `result` for `values`: the allocator calls it
    /// needs (`alloc.make.1`, `alloc.keep.1`), with `result` stored at `dest`, and the caller
    /// continuing at `target` once they are made. `false` if it needs none.
    pub(super) fn buf_allocs(
        &mut self,
        f: u32,
        values: &[Value],
        result: &Value,
        dest: &Pointer,
        target: usize,
    ) -> bool {
        let items = self.dispatch.items();
        let sig = &items.fns[f as usize];
        let Container::Impl(k) = sig.container else {
            return false;
        };
        if !matches!(items.impls[k as usize].self_ty, Ty::Buf(_)) {
            return false;
        }
        match sig.name.as_str() {
            // New storage, from the allocator in scope.
            "with_capacity" | "from_array" => {
                let Some(owner) = self.alloc_top(self.stack.current()) else {
                    return false;
                };
                let Value::Buf(cap, _) = result else {
                    return false;
                };
                let size = cap.n as i64 * ELEM;
                self.stack.store(dest, result.clone());
                let args = vec![owner.clone(), Value::Int(size), Value::Int(ELEM)];
                let ret = Ret::Buf(dest.clone(), target, Box::new(owner), size);
                self.call_prelude("__alloc_by", args, ret);
                true
            }
            // Storage grown or shrunk, by the allocator it came from.
            "set_capacity" => {
                let Value::Ref(at, _) = &values[0] else {
                    return false;
                };
                let Value::Buf(cap, _) = self.stack.load(at) else {
                    return false;
                };
                let Some(rec) = cap.owner else {
                    return false;
                };
                let Value::Tuple(r) = &*rec else {
                    return false;
                };
                let owner = r[0].clone();
                let size = cap.n as i64 * ELEM;
                self.stack.store(dest, result.clone());
                let ret = Ret::Buf(at.clone(), target, Box::new(owner), size);
                self.call_prelude("__realloc_by", vec![*rec, Value::Int(size)], ret);
                true
            }
            // The buffer given up: its storage goes back.
            "into_array" => {
                let Value::Buf(cap, _) = &values[0] else {
                    return false;
                };
                let Some(rec) = cap.owner.clone() else {
                    return false;
                };
                let free = items
                    .prelude_fn("__free_by")
                    .expect("the prelude's `__free_by`");
                self.stack.store(dest, result.clone());
                self.stack
                    .start_drops(vec![(free, Vec::new(), *rec)], target);
                true
            }
            _ => false,
        }
    }

    /// Calls the prelude's function `name` with `args`, its result going where `ret` says.
    fn call_prelude(&mut self, name: &str, args: Vec<Value>, ret: Ret) {
        let f = self
            .dispatch
            .items()
            .prelude_fn(name)
            .unwrap_or_else(|| panic!("the prelude's `{name}`"));
        let code = self.body(Item::Fn(f));
        self.stack.push(Frame::new(code, args, Vec::new(), ret));
    }

    /// `__alloc_by` or `__realloc_by` gave `p` for the buffer at `at`: what it came from, kept
    /// in the buffer for when it goes back.
    pub(super) fn buf_allocated(&mut self, at: &Pointer, owner: Value, size: i64, p: Value) {
        let mut b = self.stack.load(at);
        if let Value::Buf(cap, _) = &mut b {
            cap.owner = Some(Box::new(Value::Tuple(vec![
                owner,
                p,
                Value::Int(size),
                Value::Int(ELEM),
            ])));
        }
        self.stack.store(at, b);
    }
}
