//! The names in scope while a body is lowered: locals, the enclosing loops and their labels, and
//! whether the code is inside a `defer` block.
//!
//! The rules for what a `break` or `continue` may reach live here; the lowerer reports the
//! problems.

use crate::hir::{ExprId, LocalId};

#[derive(Default)]
pub(super) struct Names {
    /// Locals in scope, innermost last.
    locals: Vec<(String, LocalId)>,
    /// Enclosing loops, innermost last: the label and the loop expression.
    loops: Vec<(Option<String>, ExprId)>,
    /// Inside a `defer` block: how many loops enclose the block, so jumps out of it are errors.
    defer_floor: Option<usize>,
}

/// Why a `break` or `continue` has no target.
pub(super) enum JumpError {
    /// No enclosing loop has the label.
    NoSuchLabel,
    /// It is not inside a loop.
    NotInLoop,
    /// Its loop encloses the `defer` block it is in.
    OutOfDefer,
}

impl Names {
    /// The point to come back to with [`Names::restore`] when a scope ends.
    pub(super) fn mark(&self) -> usize {
        self.locals.len()
    }

    /// Ends the scopes opened since `mark`: their names go out of scope.
    pub(super) fn restore(&mut self, mark: usize) {
        self.locals.truncate(mark);
    }

    /// Brings `name` into scope as `local`, shadowing an earlier one.
    pub(super) fn bind(&mut self, name: String, local: LocalId) {
        self.locals.push((name, local));
    }

    /// Brings each of `names` into scope, in order.
    pub(super) fn bind_all(&mut self, names: impl IntoIterator<Item = (String, LocalId)>) {
        self.locals.extend(names);
    }

    /// The local `name` refers to, the innermost first.
    pub(super) fn lookup(&self, name: &str) -> Option<LocalId> {
        self.locals
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|&(_, id)| id)
    }

    /// Enters the loop `id`, maybe labelled.
    pub(super) fn enter_loop(&mut self, label: Option<String>, id: ExprId) {
        self.loops.push((label, id));
    }

    /// Leaves the innermost loop.
    pub(super) fn exit_loop(&mut self) {
        self.loops.pop();
    }

    /// Enters a `defer` block, returning what [`Names::exit_defer`] restores.
    pub(super) fn enter_defer(&mut self) -> Option<usize> {
        self.defer_floor.replace(self.loops.len())
    }

    /// Leaves a `defer` block entered with [`Names::enter_defer`].
    pub(super) fn exit_defer(&mut self, outer: Option<usize>) {
        self.defer_floor = outer;
    }

    /// Enters a closure's body: no loop or `defer` block outside it can be jumped out of from
    /// it. Returns what [`Names::exit_closure`] restores.
    pub(super) fn enter_closure(&mut self) -> (Vec<(Option<String>, ExprId)>, Option<usize>) {
        (std::mem::take(&mut self.loops), self.defer_floor.take())
    }

    /// Leaves a closure's body entered with [`Names::enter_closure`].
    pub(super) fn exit_closure(&mut self, outer: (Vec<(Option<String>, ExprId)>, Option<usize>)) {
        self.loops = outer.0;
        self.defer_floor = outer.1;
    }

    /// Whether the code is inside a `defer` block.
    pub(super) fn in_defer(&self) -> bool {
        self.defer_floor.is_some()
    }

    /// The loop a `break` or `continue` with `label` goes to.
    pub(super) fn jump_target(&self, label: Option<&str>) -> Result<ExprId, JumpError> {
        let target = match label {
            Some(l) => self
                .loops
                .iter()
                .rev()
                .find(|(name, _)| name.as_deref() == Some(l))
                .map(|&(_, id)| id)
                .ok_or(JumpError::NoSuchLabel)?,
            None => self
                .loops
                .last()
                .map(|&(_, id)| id)
                .ok_or(JumpError::NotInLoop)?,
        };
        if let Some(floor) = self.defer_floor
            && self.loops.iter().position(|&(_, id)| id == target) < Some(floor)
        {
            return Err(JumpError::OutOfDefer);
        }
        Ok(target)
    }
}
