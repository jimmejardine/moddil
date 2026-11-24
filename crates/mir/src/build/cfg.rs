//! The control-flow graph under construction: the body's blocks and locals, the block being
//! filled, and the source range recorded with each statement and terminator.
//!
//! Nothing else touches the blocks: lowering appends statements and ends blocks through this
//! API, and [`Cfg::finish`] drops the blocks control never reaches.

use moddil_sema::Ty;
use moddil_syntax::TextRange;

use crate::{
    BasicBlock, BlockId, Body, Const, Local, LocalDecl, Operand, Place, Rvalue, Statement,
    Terminator,
};

pub(super) struct Cfg {
    body: Body,
    current: BlockId,
    span: TextRange,
    /// The scope node statements and terminators are recorded with from now on.
    pub(super) node: u32,
    /// Whether the blocks made from now on are cleanup code.
    pub(super) cleanup: bool,
}

impl Cfg {
    /// A graph for `body` (its blocks empty), continuing in a new entry block.
    pub(super) fn new(body: Body) -> Cfg {
        let mut cfg = Cfg {
            body,
            current: BlockId(0),
            span: TextRange::default(),
            node: 0,
            cleanup: false,
        };
        cfg.current = cfg.new_block();
        cfg
    }

    /// A new, empty block, which ends as unreachable until it is terminated.
    pub(super) fn new_block(&mut self) -> BlockId {
        self.body.blocks.push(BasicBlock {
            statements: Vec::new(),
            spans: Vec::new(),
            terminator: Terminator::Unreachable,
            terminator_span: self.span,
            nodes: Vec::new(),
            terminator_node: 0,
            cleanup: self.cleanup,
        });
        BlockId(self.body.blocks.len() as u32 - 1)
    }

    /// The block being filled.
    pub(super) fn current(&self) -> BlockId {
        self.current
    }

    /// Continues filling `block` instead of the current block, which is left as it is.
    pub(super) fn continue_in(&mut self, block: BlockId) {
        self.current = block;
    }

    /// Where each block's statements end now, and whether it is terminated: a point to look
    /// back to with [`Cfg::moved_since`].
    pub(super) fn mark(&self) -> Vec<(usize, bool)> {
        self.body
            .blocks
            .iter()
            .map(|b| {
                (
                    b.statements.len(),
                    !matches!(b.terminator, Terminator::Unreachable),
                )
            })
            .collect()
    }

    /// Whether anything built since `mark` moves out of `local` (or a part of it) or drops it.
    pub(super) fn moved_since(&self, mark: &[(usize, bool)], local: Local) -> bool {
        let moves = |o: &Operand| matches!(o, Operand::Move(p) if p.local == local);
        let in_rvalue = |r: &Rvalue| match r {
            Rvalue::Use(a)
            | Rvalue::Unsize(a, _)
            | Rvalue::Widen(a, _)
            | Rvalue::Unary(_, a)
            | Rvalue::NegOverflows(a)
            | Rvalue::Repeat(a, _)
            | Rvalue::InBounds(a, _) => moves(a),
            Rvalue::Binary(_, a, b) | Rvalue::Overflows(_, a, b) | Rvalue::SpanOk(a, b, _) => {
                moves(a) || moves(b)
            }
            Rvalue::Aggregate(_, ops) => ops.iter().any(moves),
            Rvalue::Discriminant(_)
            | Rvalue::ErrorIs(..)
            | Rvalue::Ref(..)
            | Rvalue::Len(_)
            | Rvalue::Length(_)
            | Rvalue::StaticRef(_)
            | Rvalue::FnPtr(_) => false,
        };
        self.body.blocks.iter().enumerate().any(|(i, b)| {
            let (from, terminated) = mark.get(i).copied().unwrap_or((0, false));
            let statements = b.statements[from..].iter().any(|s| match s {
                Statement::Assign(_, r) => in_rvalue(r),
                Statement::Assert { cond: o, .. } | Statement::Observe { value: o, .. } => moves(o),
                _ => false,
            });
            let terminator = !terminated
                && match &b.terminator {
                    Terminator::Call { args, .. } => args.iter().any(moves),
                    Terminator::SwitchBool { cond: o, .. } | Terminator::Fail(o) => moves(o),
                    Terminator::Drop { place, .. } => place.local == local,
                    _ => false,
                };
            statements || terminator
        })
    }

    /// Records `span` with what is built from now on, and returns the span it replaces.
    pub(super) fn set_span(&mut self, span: TextRange) -> TextRange {
        std::mem::replace(&mut self.span, span)
    }

    /// Records the cleanup block of each scope node.
    pub(super) fn set_cleanups(&mut self, cleanups: Vec<BlockId>, locals: usize) {
        self.body.cleanups = cleanups;
        self.body.cleanup_locals = locals;
    }

    /// How many locals there are.
    pub(super) fn local_count(&self) -> usize {
        self.body.locals.len()
    }

    /// A new local of type `ty`, named for printing if `name` is given.
    pub(super) fn new_local(&mut self, ty: Ty, name: Option<String>) -> Local {
        self.body.locals.push(LocalDecl { ty, name });
        Local(self.body.locals.len() as u32 - 1)
    }

    /// Records that `local` is declared without a value (`let x;`), and whether it is `mut`.
    pub(super) fn late(&mut self, local: Local, mutable: bool) {
        self.body.late.push((local, mutable));
    }

    /// The declared type of `local`.
    pub(super) fn local_ty(&self, local: Local) -> Ty {
        self.body.locals[local.index()].ty.clone()
    }

    pub(super) fn push(&mut self, s: Statement) {
        let block = &mut self.body.blocks[self.current.index()];
        block.statements.push(s);
        block.spans.push(self.span);
        block.nodes.push(self.node);
    }

    pub(super) fn assign(&mut self, dest: impl Into<Place>, r: Rvalue) {
        self.push(Statement::Assign(dest.into(), r));
    }

    /// Ends the current block with `t` and continues in a new block (unreachable unless
    /// something jumps to it).
    pub(super) fn terminate(&mut self, t: Terminator) {
        let next = self.new_block();
        self.terminate_into(t, next);
    }

    /// Ends the current block with `t` and continues in `next`.
    pub(super) fn terminate_into(&mut self, t: Terminator, next: BlockId) {
        let block = &mut self.body.blocks[self.current.index()];
        block.terminator = t;
        block.terminator_span = self.span;
        block.terminator_node = self.node;
        self.current = next;
    }

    /// Continues in a new block if `cond` holds, and goes to `fail` if not.
    pub(super) fn branch(&mut self, cond: Operand, fail: BlockId) {
        let next = self.new_block();
        self.terminate_into(
            Terminator::SwitchBool {
                cond,
                then: next,
                else_: fail,
            },
            next,
        );
    }

    /// The finished body, without the blocks control never reaches (those after a `return`,
    /// say), the rest renumbered in order.
    /// The constant for the string literal `s`.
    pub(super) fn str_const(&mut self, s: &str) -> Const {
        let strs = &mut self.body.strs;
        let i = strs.iter().position(|x| x == s).unwrap_or_else(|| {
            strs.push(s.to_string());
            strs.len() - 1
        });
        Const::Str(i as u32)
    }

    pub(super) fn finish(mut self) -> Body {
        let body = &mut self.body;
        let mut reachable = vec![false; body.blocks.len()];
        // Cleanup blocks are reached by unwinding, not by edges.
        let mut work = vec![BlockId(0)];
        work.extend(body.cleanups.iter().copied());
        while let Some(b) = work.pop() {
            if std::mem::replace(&mut reachable[b.index()], true) {
                continue;
            }
            work.extend(body.blocks[b.index()].terminator.successors());
        }
        let mut renumber = vec![BlockId(0); body.blocks.len()];
        let mut next = 0;
        for (i, &r) in reachable.iter().enumerate() {
            if r {
                renumber[i] = BlockId(next);
                next += 1;
            }
        }
        let blocks = std::mem::take(&mut body.blocks);
        for (i, mut block) in blocks.into_iter().enumerate() {
            if !reachable[i] {
                continue;
            }
            match &mut block.terminator {
                Terminator::Call {
                    target, on_error, ..
                } => {
                    *target = renumber[target.index()];
                    if let Some((_, e)) = on_error {
                        *e = renumber[e.index()];
                    }
                }
                Terminator::Goto(t) | Terminator::Drop { target: t, .. } => {
                    *t = renumber[t.index()];
                }
                Terminator::SwitchBool { then, else_, .. } => {
                    *then = renumber[then.index()];
                    *else_ = renumber[else_.index()];
                }
                Terminator::Return
                | Terminator::Resume
                | Terminator::Unreachable
                | Terminator::Fail(_)
                | Terminator::Trap(..) => {}
            }
            body.blocks.push(block);
        }
        for c in &mut body.cleanups {
            *c = renumber[c.index()];
        }
        self.body
    }
}
