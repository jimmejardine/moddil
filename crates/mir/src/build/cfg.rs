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
}

impl Cfg {
    /// A graph for `body` (its blocks empty), continuing in a new entry block.
    pub(super) fn new(body: Body) -> Cfg {
        let mut cfg = Cfg {
            body,
            current: BlockId(0),
            span: TextRange::default(),
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

    /// Records `span` with what is built from now on, and returns the span it replaces.
    pub(super) fn set_span(&mut self, span: TextRange) -> TextRange {
        std::mem::replace(&mut self.span, span)
    }

    /// A new local of type `ty`, named for printing if `name` is given.
    pub(super) fn new_local(&mut self, ty: Ty, name: Option<String>) -> Local {
        self.body.locals.push(LocalDecl { ty, name });
        Local(self.body.locals.len() as u32 - 1)
    }

    /// The declared type of `local`.
    pub(super) fn local_ty(&self, local: Local) -> Ty {
        self.body.locals[local.index()].ty.clone()
    }

    pub(super) fn push(&mut self, s: Statement) {
        let block = &mut self.body.blocks[self.current.index()];
        block.statements.push(s);
        block.spans.push(self.span);
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
        let mut work = vec![BlockId(0)];
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
                | Terminator::Unreachable
                | Terminator::Fail(_)
                | Terminator::Trap(..) => {}
            }
            body.blocks.push(block);
        }
        self.body
    }
}
