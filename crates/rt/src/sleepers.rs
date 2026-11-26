//! Workers asleep for want of a task, and waking one (P38's stage 3): a worker with nothing to
//! do adds itself, looks for a task once more, and only then sleeps; a task queued wakes one. So
//! a task queued after the worker last looked either is found by that look or wakes it: none
//! waits with a worker asleep.
//!
//! Its locks and its threads' parking are `std`'s, or, built with `--cfg moddil_loom` or
//! `--cfg moddil_shuttle`, `loom`'s or `shuttle`'s, which check the protocol in every order its
//! threads may run in, or in many random ones (`ci.py loom`).

#[cfg(moddil_loom)]
use loom::{sync::Mutex, thread};
#[cfg(moddil_shuttle)]
use shuttle::{sync::Mutex, thread};
#[cfg(not(any(moddil_loom, moddil_shuttle)))]
use std::{sync::Mutex, thread};

/// The workers asleep.
#[derive(Default)]
pub(crate) struct Sleepers {
    asleep: Mutex<Vec<thread::Thread>>,
}

impl Sleepers {
    /// Sleeps this thread unless `found` finds a task once it is among the sleepers: what it
    /// found, if it did. It may wake with nothing to do; the caller looks again.
    pub(crate) fn sleep<T>(&self, found: impl Fn() -> Option<T>) -> Option<T> {
        self.asleep
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(thread::current());
        if let Some(t) = found() {
            return Some(t);
        }
        thread::park();
        None
    }

    /// Wakes one worker asleep: whether one was.
    pub(crate) fn wake_one(&self) -> bool {
        let w = self
            .asleep
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop();
        w.map(|w| w.unpark()).is_some()
    }

    /// Wakes every worker asleep.
    pub(crate) fn wake_all(&self) {
        for w in self
            .asleep
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .drain(..)
        {
            w.unpark();
        }
    }
}

/// The protocol under `loom` or `shuttle`: workers taking tasks that others queue, each
/// sleeping when it finds none. A lost wake-up would leave a worker asleep with a task queued,
/// which they report as a deadlock.
#[cfg(all(test, any(moddil_loom, moddil_shuttle)))]
mod tests {
    use std::collections::VecDeque;

    #[cfg(moddil_loom)]
    use loom::{sync::Arc, sync::Mutex, thread};
    #[cfg(moddil_shuttle)]
    use shuttle::{sync::Arc, sync::Mutex, thread};

    use super::Sleepers;

    /// Two workers take two tasks, queued by a third thread, each queued then a worker woken.
    fn two_workers_take_two_tasks() {
        let queue = Arc::new(Mutex::new(VecDeque::new()));
        let sleepers = Arc::new(Sleepers::default());
        let workers: Vec<_> = (0..2)
            .map(|_| {
                let (queue, sleepers) = (queue.clone(), sleepers.clone());
                thread::spawn(move || {
                    let take = || queue.lock().unwrap().pop_front();
                    loop {
                        if let Some(t) = take().or_else(|| sleepers.sleep(take)) {
                            return t;
                        }
                    }
                })
            })
            .collect();
        for t in 0..2 {
            queue.lock().unwrap().push_back(t);
            sleepers.wake_one();
        }
        let mut got: Vec<i32> = workers.into_iter().map(|w| w.join().unwrap()).collect();
        got.sort_unstable();
        assert_eq!(got, [0, 1]);
    }

    #[cfg(moddil_loom)]
    #[test]
    fn no_task_waits_with_a_worker_asleep_in_any_order() {
        let mut model = loom::model::Builder::new();
        model.preemption_bound = Some(3);
        model.check(two_workers_take_two_tasks);
    }

    #[cfg(moddil_shuttle)]
    #[test]
    fn no_task_waits_with_a_worker_asleep_in_random_orders() {
        shuttle::check_random(two_workers_take_two_tasks, 10_000);
    }
}
