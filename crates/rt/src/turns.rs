//! Which tasks run now (`task.runtime.1`): up to `slots` at once, each with a turn. The
//! deterministic runtime has one slot, so a task runs only while every other waits for its turn
//! (`task.sched.1`); the parallel runtime has one a core, and its tasks run at once, each on its
//! own thread.
//!
//! A task gives up its turn where it suspends or ends; the turns free are then given to the
//! ready tasks, in the order the scheduler picks; and when no task runs, time moves (a timer
//! fires, or the OS's events come in) until one is ready. With none, every task waits: a
//! deadlock.

use crate::sched::Scheduler;

pub(crate) struct Turns {
    /// How many tasks may run at once.
    slots: usize,
    /// Those that do, in the order they were given their turns.
    running: Vec<usize>,
}

impl Turns {
    /// The run's first task's turn, with one slot.
    pub(crate) fn new() -> Turns {
        Turns {
            slots: 1,
            running: vec![0],
        }
    }

    /// Lets `n` tasks run at once from now on.
    pub(crate) fn set_slots(&mut self, n: usize) {
        self.slots = n.max(1);
    }

    /// How many tasks may run at once.
    pub(crate) fn slots(&self) -> usize {
        self.slots
    }

    /// The tasks with a turn.
    pub(crate) fn holders(&self) -> impl Iterator<Item = usize> + '_ {
        self.running.iter().copied()
    }

    /// Whether task `t` has a turn.
    pub(crate) fn has(&self, t: usize) -> bool {
        self.running.contains(&t)
    }

    /// The task with the first turn given, if any: with one slot, the one that runs.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub(crate) fn first(&self) -> Option<usize> {
        self.running.first().copied()
    }

    /// Whether no task runs.
    pub(crate) fn idle(&self) -> bool {
        self.running.is_empty()
    }

    /// Task `t` gives up its turn.
    pub(crate) fn give_up(&mut self, t: usize) {
        self.running.retain(|&r| r != t);
    }

    /// Task `t` takes a turn: a scope's body, given its owner's.
    pub(crate) fn take(&mut self, t: usize) {
        if !self.has(t) {
            self.running.push(t);
        }
    }

    /// Gives the turns free to the ready tasks; with none running, lets time move until one is
    /// ready. Whether a turn was given.
    pub(crate) fn hand_out(&mut self, sched: &mut Scheduler) -> bool {
        let mut given = false;
        while self.running.len() < self.slots {
            let next = if self.running.is_empty() {
                sched.next_ready()
            } else {
                sched.pop_ready()
            };
            let Some(t) = next else { break };
            self.running.push(t);
            given = true;
        }
        given
    }

    /// Whether a turn is free and a task ready for it.
    pub(crate) fn can_hand_out(&self, sched: &Scheduler) -> bool {
        self.running.len() < self.slots && sched.has_ready()
    }
}
