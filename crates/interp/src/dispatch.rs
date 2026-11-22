//! Types as a frame runs them, and which function a call runs: generic code runs once for every
//! type, and a call of a trait's method is resolved through a **witness table** built the first
//! time a type is asked for that trait.

use std::collections::HashMap;

use moddil_sema::{Callee, ItemTree, Solver, Ty};

use crate::stack::DropCall;
use crate::value::Value;

/// A function to run, with its type arguments.
pub(crate) type Target = (u32, Vec<Ty>);

/// The program's items, and the witness tables built so far.
pub(crate) struct Dispatch<'a> {
    items: &'a ItemTree,
    /// For a trait's method and the type arguments of a call of it: the function that runs and
    /// its type arguments.
    witnesses: HashMap<(u32, Vec<Ty>), Option<Target>>,
}

impl<'a> Dispatch<'a> {
    pub(crate) fn new(items: &'a ItemTree) -> Self {
        Dispatch {
            items,
            witnesses: HashMap::new(),
        }
    }

    pub(crate) fn items(&self) -> &'a ItemTree {
        self.items
    }

    /// `ty` as a frame with `type_args` runs it: its type parameters replaced, associated types
    /// resolved.
    pub(crate) fn concrete(&self, ty: &Ty, type_args: &[Ty]) -> Ty {
        Solver::new(self.items, &[]).deep(&ty.subst(type_args))
    }

    /// The length a length type stands for in a frame with `type_args`.
    pub(crate) fn length(&self, n: &Ty, type_args: &[Ty]) -> usize {
        match self.concrete(n, type_args) {
            Ty::Len(k) => usize::try_from(k).unwrap_or(0),
            other => unreachable!("a length: {other:?}"),
        }
    }

    /// The function a call runs and its type arguments: for a trait's method, the impl's method
    /// (or the trait's default) for the type it is called for. `None` for the built-in `Clone`
    /// of a borrow or tuple, which copies.
    pub(crate) fn target(&mut self, callee: &Callee, type_args: &[Ty]) -> Option<Target> {
        let args: Vec<Ty> = callee
            .args()
            .iter()
            .map(|t| self.concrete(t, type_args))
            .collect();
        match callee {
            Callee::Fn { func, .. } => Some((*func, args)),
            Callee::Trait { func, .. } => {
                let key = (*func, args.clone());
                if let Some(w) = self.witnesses.get(&key) {
                    return w.clone();
                }
                let items = self.items;
                let sig = &items.fns[*func as usize];
                let t = items.trait_of(*func).expect("a trait's method");
                let def = &items.traits[t as usize];
                let n = def.generics.params.len();
                let trait_ref = moddil_sema::TraitRef {
                    trait_: t,
                    name: def.name.as_str().into(),
                    args: args[1..n].to_vec().into(),
                };
                if let Ty::Anon(_) = &args[0] {
                    let found = Some((*func, args.clone()));
                    self.witnesses.insert(key, found.clone());
                    return found;
                }
                let found = Solver::new(items, &[])
                    .impl_method(*func, &args[0], &trait_ref)
                    .map(|(f, mut parent)| {
                        parent.extend(args[sig.generics.parent as usize..].iter().cloned());
                        (f, parent)
                    });
                self.witnesses.insert(key, found.clone());
                found
            }
        }
    }

    /// The destructors to run, in order, to drop `v` of type `ty`: its own (if `own` and its type
    /// has one; the destructor then drops the fields), or else its fields', last first.
    pub(crate) fn drop_glue(&self, v: Value, ty: &Ty, own: bool) -> Vec<DropCall> {
        let mut out = Vec::new();
        self.glue(v, ty, own, &mut out);
        out
    }

    fn glue(&self, v: Value, ty: &Ty, own: bool, out: &mut Vec<DropCall>) {
        match (v, ty) {
            // A `dyn` value is dropped as the type it is (`type.box.dyn.1`).
            (Value::Owned(o), _) => {
                let (v, t) = *o;
                self.glue(v, &t, own, out);
            }
            (v @ Value::Adt { .. }, Ty::Adt(a, ..)) if own && self.items.has_drop_impl(*a) => {
                let drop = self.items.lang_ref("Drop").expect("the prelude's `Drop`");
                let decl = self.items.traits[drop.trait_ as usize].methods[0];
                let (f, args) = Solver::new(self.items, &[])
                    .impl_method(decl, ty, &drop)
                    .expect("a destructor");
                out.push((f, args, v));
            }
            (
                Value::Adt {
                    variant, fields, ..
                },
                t @ Ty::Adt(..),
            ) => {
                let types = self.items.field_types(t, variant);
                for (f, ft) in fields.into_iter().zip(types).rev() {
                    self.glue(f, &ft, true, out);
                }
            }
            (Value::Tuple(xs), Ty::Tuple(ts)) => {
                for (x, t) in xs.into_iter().zip(ts.iter()).rev() {
                    self.glue(x, t, true, out);
                }
            }
            // Elements are dropped first to last.
            (Value::Array(xs) | Value::Buf(_, xs), Ty::Array(t, _) | Ty::Buf(t)) => {
                for x in xs {
                    self.glue(x, t, true, out);
                }
            }
            (Value::Closure(xs), Ty::Closure(c)) => {
                for (x, t) in xs.into_iter().zip(c.captures.iter()).rev() {
                    self.glue(x, t, true, out);
                }
            }
            (Value::Tuple(xs), Ty::Anon(a)) => {
                for (x, (_, t)) in xs.into_iter().zip(a.methods.iter()).rev() {
                    self.glue(x, t, true, out);
                }
            }
            _ => {}
        }
    }
}
