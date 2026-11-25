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
    /// The node of each item of the open scopes, in the same order.
    ids: Vec<Vec<u32>>,
    /// Every node: an item, and the node before it, what was held when it was added. Node 0,
    /// first, holds nothing (and is its own parent).
    nodes: Vec<(Option<ScopeItem>, u32)>,
    /// The enclosing loops, innermost last.
    loops: Vec<LoopCx>,
    /// While a cleanup block is built: the node whose cleanup it goes on to, and how many
    /// scopes were open below the one opened for it. What is held there is that node, not the
    /// body's items.
    base: Option<(u32, usize)>,
}

impl Scopes {
    /// The body's own scope, open.
    pub(super) fn new() -> Scopes {
        Scopes {
            open: vec![Vec::new()],
            ids: vec![Vec::new()],
            nodes: vec![(None, 0)],
            loops: Vec::new(),
            base: None,
        }
    }

    /// Opens a scope inside the innermost one.
    pub(super) fn open(&mut self) {
        self.open.push(Vec::new());
        self.ids.push(Vec::new());
    }

    /// Closes the innermost scope, whose leaving code has already been emitted.
    pub(super) fn close(&mut self) {
        self.open.pop();
        self.ids.pop();
    }

    /// The node of what the open scopes hold now (`task.cancel.1`).
    pub(super) fn node(&self) -> u32 {
        let (base, below) = self.base.unwrap_or((0, 0));
        self.ids[below..]
            .iter()
            .rev()
            .find_map(|s| s.last().copied())
            .unwrap_or(base)
    }

    /// Node `n`: its item, and the node before it.
    pub(super) fn node_at(&self, n: usize) -> (Option<ScopeItem>, u32) {
        self.nodes[n]
    }

    /// How many nodes there are.
    pub(super) fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Opens a scope for a cleanup block that goes on to node `n`'s: what it holds, until
    /// [`Scopes::end_base`], is `n` and what the cleanup's own code adds.
    pub(super) fn set_base(&mut self, n: u32) {
        self.base = Some((n, self.ids.len()));
        self.open.push(Vec::new());
        self.ids.push(Vec::new());
    }

    /// Closes the scope [`Scopes::set_base`] opened.
    pub(super) fn end_base(&mut self) {
        self.base = None;
        self.open.pop();
        self.ids.pop();
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
        let parent = self.node();
        self.nodes.push((Some(item), parent));
        self.ids
            .last_mut()
            .expect("a scope")
            .push(self.nodes.len() as u32 - 1);
        self.open.last_mut().expect("a scope").push(item);
    }

    /// Removes the temporaries added to the innermost scope since it had `mark` items, and
    /// returns them in the order to drop them: the newest first. At the end of a statement.
    pub(super) fn take_temps(&mut self, mark: usize) -> Vec<Local> {
        let scope = self.open.last_mut().expect("a scope");
        let ids = self.ids.last_mut().expect("a scope");
        let from = mark.min(scope.len());
        let mut temps = Vec::new();
        let mut kept = Vec::new();
        for (item, id) in scope.drain(from..).zip(ids.drain(from..)) {
            match item {
                ScopeItem::Temp(t) => temps.push(t),
                other => kept.push((other, id)),
            }
        }
        // What is kept gets new nodes: the temporaries before it are gone.
        for (item, _) in kept {
            let parent = self
                .ids
                .iter()
                .rev()
                .find_map(|s| s.last().copied())
                .unwrap_or(0);
            self.nodes.push((Some(item), parent));
            self.ids
                .last_mut()
                .expect("a scope")
                .push(self.nodes.len() as u32 - 1);
            self.open.last_mut().expect("a scope").push(item);
        }
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
