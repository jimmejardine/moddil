//! The parallel runtime's reactor, for a run of `main` (`os.1`): a thread that waits in the
//! OS's wait for readiness (`moddil-os`, on `mio`) without the run's lock, until a socket is
//! ready, a signal comes or the next timer is due, and then, with the lock, delivers what came,
//! fires the timers due and gives the turns that frees to the ready tasks. So the OS's events
//! come in while tasks run; the scheduler never waits for them itself ([`moddil_sched::Scheduler::
//! set_reactor`]). Under the deterministic runtime a task that has nothing to run waits in the
//! OS's wait itself, as before.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::run::{Ctx, State};

/// A run's reactor: its thread, what wakes it, and whether the run has ended.
pub(crate) struct Reactor {
    thread: std::thread::JoinHandle<()>,
    wake: moddil_os::Wake,
    over: Arc<AtomicBool>,
}

impl Reactor {
    /// Starts run `c`'s reactor, if its tasks run in parallel and it has the OS, and it has none.
    pub(crate) fn start(c: &Arc<Ctx>, s: &mut State) {
        if s.reactor.is_some() || s.turns.slots() < 2 {
            return;
        }
        let Some((mut waiter, wake)) = s.sched.os().and_then(|o| o.take_waiter()) else {
            return;
        };
        s.sched.set_reactor();
        let over = Arc::new(AtomicBool::new(false));
        let (c, done) = (c.clone(), over.clone());
        let thread = std::thread::Builder::new()
            .name("moddil-reactor".to_string())
            .spawn(move || {
                crate::run::adopt(&c);
                loop {
                    let wait = c.lock().sched.until_timer();
                    if done.load(Ordering::Acquire) {
                        return;
                    }
                    waiter.wait(wait);
                    if done.load(Ordering::Acquire) {
                        return;
                    }
                    let mut s = c.lock();
                    let st = &mut *s;
                    st.sched.tick();
                    if st.turns.hand_out(&mut st.sched) {
                        crate::baton::pass_on(&c, st);
                    }
                }
            })
            .expect("the reactor's thread");
        s.reactor = Some(Reactor { thread, wake, over });
    }

    /// Wakes the reactor to look at what has come: a socket the program has just begun to watch
    /// may have been ready before.
    pub(crate) fn nudge(&self) {
        self.wake.wake();
    }

    /// Stops the reactor, once the run has ended.
    pub(crate) fn stop(self) {
        self.over.store(true, Ordering::Release);
        self.wake.wake();
        let _ = self.thread.join();
    }
}
