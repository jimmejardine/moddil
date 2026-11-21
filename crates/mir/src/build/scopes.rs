//! The scopes and loops open while a body is built, and the order in which leaving them undoes
//! what they hold.
//!
//! This module decides *what* to undo; the builder emits the code for it, since running a
//! `defer` block means building it.

use moddil_sema::hir::ExprId;

use crate::{BlockId, Local};

/// What a scope holds, in the order it was declared: leaving the scope undoes them in reverse.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ScopeItem {
    /// A local, which is dropped and whose storage ends.
    Local(Local),
    /// A temporary that may need dropping, which is dropped at the end of its statement.
    Temp(Local),
    /// A `defer` block, which runs.
    Defer(ExprId),
    /// An `errdefer` block, which runs only when the scope is left with an error.
    ErrDefer(ExprId),
}

/// An enclosing loop while its body is built.
#[derive(Clone, Copy)]
pub(super) struct LoopCx {
    /// The HIR `loop` or `while`.
    pub(super) id: ExprId,
    /// Where `break` goes.
    pub(super) exit: BlockId,
    /// Where `continue` goes.
    pub(super) header: BlockId,
    /// Where a `break` value goes.
    pub(super) dest: Local,
    /// How many scopes were open outside the loop.
    pub(super) depth: usize,
}

pub(super) struct Scopes {
    /// The open scopes, outermost (the body, holding the parameters) first.
    open: Vec<Vec<ScopeItem>>,
    /// The enclosing loops, innermost last.
    loops: Vec<LoopCx>,
}

impl Scopes {
    /// The body's own scope, open.
    pub(super) fn new() -> Scopes {
        Scopes {
            open: vec![Vec::new()],
            loops: Vec::new(),
        }
    }

    /// Opens a scope inside the innermost one.
    pub(super) fn open(&mut self) {
        self.open.push(Vec::new());
    }

    /// Closes the innermost scope, whose leaving code has already been emitted.
    pub(super) fn close(&mut self) {
        self.open.pop();
    }

    /// How many scopes are open.
    pub(super) fn depth(&self) -> usize {
        self.open.len()
    }

    /// How many items the innermost scope holds, for [`Scopes::take_temps`].
    pub(super) fn mark(&self) -> usize {
        self.open.last().map_or(0, Vec::len)
    }

    /// Adds `item` to the innermost scope.
    pub(super) fn push(&mut self, item: ScopeItem) {
        self.open.last_mut().expect("a scope").push(item);
    }

    /// Removes the temporaries added to the innermost scope since it had `mark` items, and
    /// returns them in the order to drop them: the newest first. At the end of a statement.
    pub(super) fn take_temps(&mut self, mark: usize) -> Vec<Local> {
        let scope = self.open.last_mut().expect("a scope");
        let mut temps = Vec::new();
        let mut kept = Vec::new();
        for item in scope.drain(mark.min(scope.len())..) {
            match item {
                ScopeItem::Temp(t) => temps.push(t),
                other => kept.push(other),
            }
        }
        scope.extend(kept);
        temps.reverse();
        temps
    }

    /// What leaving every scope deeper than `depth` undoes, in order: the innermost scope first,
    /// each in reverse order of declaration. The scopes stay open.
    pub(super) fn unwind(&self, depth: usize) -> Vec<ScopeItem> {
        self.open[depth..]
            .iter()
            .rev()
            .flat_map(|s| s.iter().rev().copied())
            .collect()
    }

    /// Enters a loop whose body is built next.
    pub(super) fn enter_loop(&mut self, cx: LoopCx) {
        self.loops.push(cx);
    }

    /// Leaves the innermost loop.
    pub(super) fn exit_loop(&mut self) {
        self.loops.pop();
    }

    /// The enclosing loop `id`, which a `break` or `continue` names.
    pub(super) fn loop_cx(&self, id: ExprId) -> LoopCx {
        *self
            .loops
            .iter()
            .rev()
            .find(|l| l.id == id)
            .expect("a resolved loop")
    }
}
