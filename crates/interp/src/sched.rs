//! The scheduler (`task.sched.1`): `moddil-sched`'s, native code's runtime's too, with values as
//! the interpreter's and traps as [`Trap`]. The interpreter runs destructors itself, so it gives
//! tasks no drop glue and gets no values back to drop.

use crate::Trap;
use crate::value::Value;

pub(crate) use moddil_sched::{Op, OsSource, Wait};

/// The scheduler of a run.
pub(crate) type Scheduler = moddil_sched::Scheduler<Value, Trap, (), RunOs>;
/// How a task ended.
pub(crate) type Outcome = moddil_sched::Outcome<Value>;
/// Why a scope failed.
pub(crate) type Failure = moddil_sched::Failure<Value, Trap>;

impl moddil_sched::Item for Value {
    fn unit() -> Self {
        Value::Unit
    }

    fn int(i: i64) -> Self {
        Value::Int(i)
    }
}

/// The OS of a run of `main` (`os.1`), and when the run began.
pub(crate) struct RunOs {
    os: moddil_os::Os,
    start: std::time::Instant,
}

impl RunOs {
    /// The OS, with the program's arguments.
    pub(crate) fn new(args: Vec<String>) -> RunOs {
        RunOs {
            os: moddil_os::Os::new(args),
            start: std::time::Instant::now(),
        }
    }
}

impl std::ops::Deref for RunOs {
    type Target = moddil_os::Os;
    fn deref(&self) -> &moddil_os::Os {
        &self.os
    }
}

impl std::ops::DerefMut for RunOs {
    fn deref_mut(&mut self) -> &mut moddil_os::Os {
        &mut self.os
    }
}

impl moddil_sched::Host for RunOs {
    fn watch(&mut self, id: usize) {
        self.os.watch(id);
    }

    fn watch_signals(&mut self) {
        self.os.watch_signals();
    }

    fn waiting(&self) -> bool {
        self.os.waiting()
    }

    fn poll(&mut self, wait: Option<std::time::Duration>) -> Vec<moddil_sched::Event> {
        use moddil_sched::Event;
        self.os
            .poll(wait)
            .into_iter()
            .map(|e| match e {
                moddil_os::Event::Accepted(l, c) => Event::Accepted(l, c),
                moddil_os::Event::Data(c, n) => Event::Data(c, n),
                moddil_os::Event::Ended(c) => Event::Ended(c),
                moddil_os::Event::Signal(s) => Event::Signal(s),
            })
            .collect()
    }

    fn elapsed(&self) -> i64 {
        i64::try_from(self.start.elapsed().as_nanos()).unwrap_or(i64::MAX)
    }
}
