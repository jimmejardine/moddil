//! The scheduler (`task.sched.1`): which tasks there are, in which scopes, which are ready, and
//! how each ended. One task runs at a time; the others are queued, first in, first out, and run
//! only when the running one suspends (`join`, a scope's end, `yield_now`).

use std::collections::VecDeque;

use crate::value::Value;

/// How a task ended: its value, or its error.
pub(crate) type Outcome = Result<Value, Value>;

/// What a suspended task waits for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Wait {
    /// A task to end.
    Join(usize),
    /// Every task of a scope to end.
    Scope(usize),
}

struct TaskState {
    /// How it ended, once it has.
    outcome: Option<Outcome>,
    /// When it ended, among the tasks that have.
    ended: Option<usize>,
    /// Whether `join` has taken its outcome.
    joined: bool,
    /// What it waits for, while it is suspended on that.
    waiting: Option<Wait>,
}

pub(crate) struct Scheduler {
    /// Each task, by the index the frame stack gives it: the first is the run's own.
    tasks: Vec<TaskState>,
    /// Each scope's tasks, in the order they were spawned.
    scopes: Vec<Vec<usize>>,
    /// The tasks ready to run, the next first.
    ready: VecDeque<usize>,
    /// How many tasks have ended.
    ended: usize,
}

impl Scheduler {
    pub(crate) fn new() -> Self {
        Scheduler {
            tasks: vec![TaskState::new()],
            scopes: Vec::new(),
            ready: VecDeque::new(),
            ended: 0,
        }
    }

    /// Opens a scope, and gives its index.
    pub(crate) fn open_scope(&mut self) -> usize {
        self.scopes.push(Vec::new());
        self.scopes.len() - 1
    }

    /// Records task `t`, just started in `scope`, as ready to run after those already ready.
    pub(crate) fn spawn(&mut self, t: usize, scope: usize) {
        debug_assert_eq!(t, self.tasks.len(), "tasks are numbered as they start");
        self.tasks.push(TaskState::new());
        self.scopes[scope].push(t);
        self.ready.push_back(t);
    }

    /// Records how task `t` ended, and makes ready the tasks that waited for that.
    pub(crate) fn end(&mut self, t: usize, outcome: Outcome) {
        let task = &mut self.tasks[t];
        task.outcome = Some(outcome);
        task.ended = Some(self.ended);
        self.ended += 1;
        for w in 0..self.tasks.len() {
            let wake = match self.tasks[w].waiting {
                Some(Wait::Join(j)) => j == t,
                Some(Wait::Scope(s)) => self.scope_done(s),
                None => false,
            };
            if wake {
                self.tasks[w].waiting = None;
                self.ready.push_back(w);
            }
        }
    }

    /// Task `t`'s outcome, taken by its `join`, if it has ended.
    pub(crate) fn take(&mut self, t: usize) -> Option<Outcome> {
        let task = &mut self.tasks[t];
        let outcome = task.outcome.take()?;
        task.joined = true;
        Some(outcome)
    }

    /// Whether every task of `scope` has ended.
    pub(crate) fn scope_done(&self, scope: usize) -> bool {
        self.scopes[scope]
            .iter()
            .all(|&t| self.tasks[t].ended.is_some())
    }

    /// The error a scope fails with once its tasks have ended: that of the first, by when it
    /// ended, to fail and never be joined (`task.error.1`).
    pub(crate) fn scope_error(&mut self, scope: usize) -> Option<Value> {
        let first = self.scopes[scope]
            .iter()
            .copied()
            .filter(|&t| {
                let task = &self.tasks[t];
                !task.joined && matches!(task.outcome, Some(Err(_)))
            })
            .min_by_key(|&t| self.tasks[t].ended)?;
        match self.tasks[first].outcome.take() {
            Some(Err(e)) => Some(e),
            _ => None,
        }
    }

    /// Suspends task `t` until `w` happens.
    pub(crate) fn block(&mut self, t: usize, w: Wait) {
        self.tasks[t].waiting = Some(w);
    }

    /// Queues task `t` again, after those already ready (`yield_now`).
    pub(crate) fn requeue(&mut self, t: usize) {
        self.ready.push_back(t);
    }

    /// The next task to run, if any is ready.
    pub(crate) fn next(&mut self) -> Option<usize> {
        self.ready.pop_front()
    }
}

impl TaskState {
    fn new() -> Self {
        TaskState {
            outcome: None,
            ended: None,
            joined: false,
            waiting: None,
        }
    }
}
