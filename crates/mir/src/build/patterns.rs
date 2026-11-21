//! Patterns: testing a place against a pattern and binding its names.

use moddil_sema::hir::{PatId, PatKind};
use moddil_sema::{Ty, tuple_positions};

use super::Builder;
use crate::{BinOp, BlockId, Const, Operand, Place, Rvalue, Terminator};

impl Builder<'_> {
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
                self.cfg.assign(l, value);
            }
            PatKind::Ref { pat: inner, .. } => self.test_pat(inner, &place.deref(), fail),
            PatKind::Int(v) => self.test_cmp(BinOp::Eq, place, Const::Int(v as i64), fail),
            PatKind::Bool(v) => self.test_cmp(BinOp::Eq, place, Const::Bool(v), fail),
            PatKind::Range { lo, hi, inclusive } => {
                self.test_cmp(BinOp::Ge, place, Const::Int(lo as i64), fail);
                if let Some(hi) = hi {
                    let op = if inclusive { BinOp::Le } else { BinOp::Lt };
                    self.test_cmp(op, place, Const::Int(hi as i64), fail);
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
                    self.test_pat(alt, place, next);
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
