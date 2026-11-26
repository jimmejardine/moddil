//! Patterns: testing a place against a pattern and binding its names.

use moddil_sema::hir::{PatId, PatKind};
use moddil_sema::{Ty, tuple_positions};

use super::Builder;
use crate::{BinOp, BlockId, Const, Local, Operand, Place, Rvalue, Terminator};

/// A binding a pattern makes once it has matched: the local, its value, and the place it moves
/// out of, if it does.
pub(super) type Binding = (Local, Rvalue, Option<Place>);

impl Builder<'_> {
    /// Tests whether the value at `place` matches `pat`, and only then binds its names, so a
    /// pattern that fails part way moves nothing; continues in the current block if it matches
    /// and goes to `fail` if not. Returns the locals bound by moving, and where from, so that a
    /// guard that fails can move them back.
    pub(super) fn match_pat(
        &mut self,
        pat: PatId,
        place: &Place,
        fail: BlockId,
    ) -> Vec<(Local, Place)> {
        let outer = self.binds.replace(Vec::new());
        self.test_pat(pat, place, fail);
        let found = std::mem::replace(&mut self.binds, outer).unwrap_or_default();
        let mut moved = Vec::new();
        for (l, value, from) in found {
            self.cfg.assign(l, value);
            if let Some(p) = from {
                moved.push((l, p));
            }
        }
        moved
    }

    /// Continues if `cond` (a guard) holds; if not, moves the values `moved` back where they
    /// came from and goes to `fail`.
    pub(super) fn guard_branch(&mut self, cond: Operand, moved: &[(Local, Place)], fail: BlockId) {
        if moved.is_empty() {
            self.cfg.branch(cond, fail);
            return;
        }
        let back = self.cfg.new_block();
        self.cfg.branch(cond, back);
        let matched = self.cfg.current();
        self.cfg.continue_in(back);
        for (l, p) in moved {
            self.cfg
                .assign(p.clone(), Rvalue::Use(Operand::Move(Place::local(*l))));
        }
        self.cfg.terminate_into(Terminator::Goto(fail), matched);
    }

    /// Tests whether the value at `place` matches `pat`, binding its names as it goes; continues
    /// in the current block if it does and goes to `fail` if not.
    pub(super) fn test_pat(&mut self, pat: PatId, place: &Place, fail: BlockId) {
        let span = self.hir.pats[pat].range;
        let outer = self.cfg.set_span(span);
        self.test_pat_kind(pat, place, fail);
        self.cfg.set_span(outer);
    }

    pub(super) fn test_pat_kind(&mut self, pat: PatId, place: &Place, fail: BlockId) {
        // Matching through a borrow tests what it points to.
        let place = &if self.types.pat_through_borrow(pat) {
            place.deref()
        } else {
            place.clone()
        };
        match self.hir.pats[pat].kind.clone() {
            PatKind::Missing | PatKind::Wild | PatKind::Unit => {}
            // The error's type is tested before (`try_catch`); here the pattern matches it.
            PatKind::OfType { pat: inner, .. } => self.test_pat(inner, place, fail),
            // `p: X`: `X::holds` on (a copy of) the value, then `p`. A value that is not `Copy`
            // is moved into `holds` and put back from what it gives back.
            PatKind::Refined {
                refinement,
                pat: inner,
            } => {
                let r = self.items.types[refinement as usize]
                    .refinement
                    .as_ref()
                    .expect("a refinement");
                let (holds, holds_ref) = (r.holds, r.holds_ref);
                let base = self.items.types[refinement as usize].ty.clone();
                let copy = self.is_copy(&base);
                // Matched through a borrow, a value that is not `Copy` is tested where it is
                // (`refine.pat.2`).
                if !copy
                    && self.types.pat_through_borrow(pat)
                    && let Some(f) = holds_ref
                {
                    let r = self.temp(Ty::reference(false, base));
                    self.cfg.assign(r, Rvalue::Ref(false, place.clone()));
                    let ok = self.temp(Ty::Bool);
                    let range = self.hir.pats[pat].range;
                    self.call_into(
                        None,
                        moddil_sema::Callee::Fn {
                            func: f,
                            args: Vec::new(),
                        },
                        Place::local(ok),
                        vec![Operand::Move(Place::local(r))],
                        range,
                    );
                    self.cfg.branch(Operand::Copy(Place::local(ok)), fail);
                    self.test_pat(inner, place, fail);
                    return;
                }
                let ok = self.temp(Ty::tuple(vec![Ty::Bool, base]));
                let range = self.hir.pats[pat].range;
                let value = if copy {
                    Operand::Copy(place.clone())
                } else {
                    Operand::Move(place.clone())
                };
                self.call_into(
                    None,
                    moddil_sema::Callee::Fn {
                        func: holds,
                        args: Vec::new(),
                    },
                    Place::local(ok),
                    vec![value],
                    range,
                );
                if !copy {
                    self.cfg.assign(
                        place.clone(),
                        Rvalue::Use(Operand::Move(Place::local(ok).field(1))),
                    );
                }
                self.cfg
                    .branch(Operand::Copy(Place::local(ok).field(0)), fail);
                self.test_pat(inner, place, fail);
            }
            PatKind::Bind { local, sub } => {
                if let Some(s) = sub {
                    self.test_pat(s, place, fail);
                }
                let l = self.local(local);
                let value = match self.types.binds_borrow(pat) {
                    Some(m) => Rvalue::Ref(m, place.clone()),
                    None => {
                        let ty = self.types.pat(pat);
                        Rvalue::Use(self.use_place(place.clone(), &ty))
                    }
                };
                match &mut self.binds {
                    Some(found) => {
                        let from = match &value {
                            Rvalue::Use(Operand::Move(p)) if !p.is_indirect() => Some(p.clone()),
                            _ => None,
                        };
                        found.push((l, value, from));
                    }
                    None => self.cfg.assign(l, value),
                }
            }
            PatKind::Ref { pat: inner, .. } => self.test_pat(inner, &place.deref(), fail),
            // At the scrutinee's integer type (`pat.lit.1`).
            PatKind::Int(v) => {
                let t = self.items.base(self.types.pat(pat).strip_ref());
                let c = crate::int_const(&t, v);
                self.test_cmp(BinOp::Eq, place, c, fail);
            }
            PatKind::Bool(v) => self.test_cmp(BinOp::Eq, place, Const::Bool(v), fail),
            PatKind::Char(c) => self.test_cmp(BinOp::Eq, place, Const::Char(c), fail),
            PatKind::Str(ref s) => {
                let c = self.cfg.str_const(s);
                self.test_cmp(BinOp::Eq, place, c, fail);
            }
            PatKind::Range {
                lo,
                hi,
                inclusive,
                chars,
            } => {
                let t = self.items.base(self.types.pat(pat).strip_ref());
                let bound = |v: moddil_syntax::ast::IntValue| {
                    if chars {
                        Const::Char(char::from_u32(v.bits() as u32).expect("a checked `char`"))
                    } else {
                        crate::int_const(&t, v)
                    }
                };
                self.test_cmp(BinOp::Ge, place, bound(lo), fail);
                if let Some(hi) = hi {
                    let op = if inclusive { BinOp::Le } else { BinOp::Lt };
                    self.test_cmp(op, place, bound(hi), fail);
                }
            }
            // `[a, .., z]` (`pat.slice.1`): the length, then each element from the front or the
            // back, then what `..` stands for, as a part.
            PatKind::Slice {
                pats,
                rest,
                rest_pat,
            } => {
                let len = self.temp(Ty::Int);
                self.cfg.assign(len, Rvalue::Len(place.clone()));
                let n = pats.len() as i64;
                let op = if rest.is_some() { BinOp::Ge } else { BinOp::Eq };
                self.test_cmp(op, &Place::local(len), Const::Int(n), fail);
                let before = rest.unwrap_or(pats.len());
                for (i, &s) in pats.iter().enumerate() {
                    let at = self.temp(Ty::Int);
                    if i < before {
                        self.cfg
                            .assign(at, Rvalue::Use(Operand::Const(Const::Int(i as i64))));
                    } else {
                        // Counted from the end.
                        self.cfg.assign(
                            at,
                            Rvalue::Binary(
                                BinOp::Sub,
                                Operand::Copy(Place::local(len)),
                                Operand::Const(Const::Int(n - i as i64)),
                            ),
                        );
                    }
                    self.test_pat(s, &place.index(at), fail);
                }
                if let Some(r) = rest_pat {
                    let from = self.temp(Ty::Int);
                    self.cfg
                        .assign(from, Rvalue::Use(Operand::Const(Const::Int(before as i64))));
                    let to = self.temp(Ty::Int);
                    self.cfg.assign(
                        to,
                        Rvalue::Binary(
                            BinOp::Sub,
                            Operand::Copy(Place::local(len)),
                            Operand::Const(Const::Int(n - before as i64)),
                        ),
                    );
                    self.test_pat(r, &place.subslice(from, to), fail);
                }
            }
            PatKind::Tuple { pats, rest } => {
                let arity = match self.types.pat(pat).strip_ref() {
                    Ty::Tuple(ts) => ts.len(),
                    _ => pats.len(),
                };
                for (&s, i) in pats.iter().zip(tuple_positions(pats.len(), rest, arity)) {
                    self.test_pat(s, &place.field(i as u32), fail);
                }
            }
            PatKind::Adt {
                adt,
                variant,
                fields,
            } => {
                let inner = if self.items.adts[adt as usize].is_enum {
                    let d = self.temp(Ty::Int);
                    self.cfg.assign(d, Rvalue::Discriminant(place.clone()));
                    self.test_cmp(
                        BinOp::Eq,
                        &Place::local(d),
                        Const::Int(i64::from(variant)),
                        fail,
                    );
                    place.downcast(variant)
                } else {
                    place.clone()
                };
                for (i, s) in fields {
                    self.test_pat(s, &inner.field(i), fail);
                }
            }
            PatKind::Or(alts) => {
                let matched = self.cfg.new_block();
                for (k, &alt) in alts.iter().enumerate() {
                    let next = if k + 1 == alts.len() {
                        fail
                    } else {
                        self.cfg.new_block()
                    };
                    // Each alternative binds on its own path to the join.
                    let outer = self.binds.take();
                    self.test_pat(alt, place, next);
                    self.binds = outer;
                    self.cfg.terminate_into(Terminator::Goto(matched), next);
                }
                self.cfg.continue_in(matched);
            }
        }
    }

    /// Continues if `place op c` holds, else goes to `fail`.
    pub(super) fn test_cmp(&mut self, op: BinOp, place: &Place, c: Const, fail: BlockId) {
        let t = self.temp(Ty::Bool);
        self.cfg.assign(
            t,
            Rvalue::Binary(op, Operand::Copy(place.clone()), Operand::Const(c)),
        );
        self.cfg.branch(Operand::Copy(Place::local(t)), fail);
    }
}
