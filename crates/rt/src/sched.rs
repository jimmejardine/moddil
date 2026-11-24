//! The scheduler (`task.sched.1`): `moddil-sched`'s, the MIR interpreter's too, with values as
//! their bytes, traps as [`Trapped`], and drop glue for the values nobody takes.

use crate::Trapped;
use crate::baton::DropFn;

pub(crate) use moddil_sched::{Op, OsSource, Wait};

/// The scheduler of a run.
pub(crate) type Scheduler = moddil_sched::Scheduler<Bytes, Trapped, DropFn, RunOs>;
/// How a task ended.
pub(crate) type Outcome = moddil_sched::Outcome<Bytes>;
/// Why a scope failed.
pub(crate) type Failure = moddil_sched::Failure<Bytes, Trapped>;

/// A value, as its bytes.
pub(crate) type Bytes = Vec<u8>;

/// The OS of a run of `main`, natively or the host's in WebAssembly, and when the run began.
pub(crate) struct RunOs {
    os: crate::os::Os,
    start: crate::os::Start,
}

impl RunOs {
    pub(crate) fn new(args: Vec<String>) -> RunOs {
        RunOs {
            os: crate::os::Os::new(args),
            start: crate::os::Start::now(),
        }
    }
}

impl std::ops::Deref for RunOs {
    type Target = crate::os::Os;
    fn deref(&self) -> &crate::os::Os {
        &self.os
    }
}

impl std::ops::DerefMut for RunOs {
    fn deref_mut(&mut self) -> &mut crate::os::Os {
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
        self.os.poll(wait)
    }

    fn elapsed(&self) -> i64 {
        self.start.elapsed()
    }
}
