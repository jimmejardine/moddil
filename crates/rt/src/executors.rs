//! The parallel runtime's executors (`task.executor.1`): each a group of worker threads and the
//! queues its tasks with a turn wait in (`crossbeam-deque`). A worker takes from its own queue,
//! then from its executor's shared one, then steals from its executor's other workers; one of an
//! executor that lends (the default one and the pools) then takes from the other lenders, so an
//! idle pool's threads run another pool's tasks. A pinned executor's workers are each pinned to
//! one of its cores (`core_affinity`), never lend, and may poll for work rather than sleep; each
//! executor's workers have the OS's priority for its own (`thread-priority`). An executor that
//! names a NUMA node and no cores has that node's ([`crate::numa`]).

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use crossbeam_deque::{Injector, Steal, Stealer, Worker};
use moddil_sched::Executor;

use crate::sleepers::Sleepers;

/// What an executor is: 0 a pool, 1 pinned, 2 for blocking calls, 3 affine (`moddil/runtime`).
const POOL: i64 = 0;
const PINNED: i64 = 1;
const BLOCKING: i64 = 2;
const AFFINE: i64 = 3;

/// The queues of an executor's tasks with a turn, and its workers asleep.
pub(crate) struct Queues {
    /// The executor's number.
    id: i64,
    shared: Injector<usize>,
    stealers: Vec<Stealer<usize>>,
    asleep: Sleepers,
    /// Whether its workers poll for a task rather than sleep.
    busy_poll: bool,
    /// Whether its workers take other executors' tasks when idle, and its tasks may be taken.
    lends: bool,
    /// The queues of every executor that lends, its own among them if it does.
    lenders: Arc<RwLock<Vec<Arc<Queues>>>>,
    /// Whether the run has ended, and its workers with it.
    over: Arc<AtomicBool>,
}

thread_local! {
    /// On a worker, its executor's number and its own queue.
    static LOCAL: RefCell<Option<(i64, Worker<usize>)>> = const { RefCell::new(None) };
}

/// Queues task `t` on this thread's worker's queue if it is a worker of executor `id`: whether
/// it is.
#[inline(never)]
fn push_local(id: i64, t: usize) -> bool {
    LOCAL.with(|l| match &*l.borrow() {
        Some((e, w)) if *e == id => {
            w.push(t);
            true
        }
        _ => false,
    })
}

impl Queues {
    /// Queues task `t`: on this thread's worker's queue if it is one of this executor's, or on
    /// the shared one; and wakes a worker asleep, of this executor or one that lends to it.
    pub(crate) fn push(&self, t: usize) {
        if !push_local(self.id, t) {
            self.shared.push(t);
        }
        if !self.wake_one() && self.lends {
            for q in self
                .lenders
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .iter()
            {
                if q.wake_one() {
                    break;
                }
            }
        }
    }

    /// A task from this executor's queues, stolen into `local` or taken from it.
    fn take(&self, local: &Worker<usize>, own: bool) -> Steal<usize> {
        if own && let Some(t) = local.pop() {
            return Steal::Success(t);
        }
        let mut s = self.shared.steal_batch_and_pop(local);
        for w in &self.stealers {
            if s.is_success() {
                break;
            }
            s = s.or_else(|| w.steal());
        }
        s
    }

    /// The next task for the worker whose queue is `local`: its executor's, or, if it lends, a
    /// lender's.
    fn find(&self, local: &Worker<usize>) -> Option<usize> {
        loop {
            let mut s = self.take(local, true);
            if !s.is_success() && self.lends {
                for q in self
                    .lenders
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .iter()
                {
                    if q.id != self.id {
                        s = s.or_else(|| q.take(local, false));
                        if s.is_success() {
                            break;
                        }
                    }
                }
            }
            match s {
                Steal::Success(t) => return Some(t),
                Steal::Empty => return None,
                Steal::Retry => {}
            }
        }
    }

    /// Wakes one of its workers asleep: whether one was.
    fn wake_one(&self) -> bool {
        self.asleep.wake_one()
    }

    /// Wakes every worker asleep: the run has ended.
    pub(crate) fn wake_all(&self) {
        self.asleep.wake_all();
    }

    /// The next task for this thread's worker, waiting (asleep, or polling) while there is none:
    /// `None` once the run has ended.
    pub(crate) fn next(&self) -> Option<usize> {
        let find = || {
            LOCAL.with(|l| {
                let l = l.borrow();
                let (_, w) = l.as_ref().expect("a worker's queue");
                self.find(w)
            })
        };
        loop {
            if self.over.load(Ordering::Acquire) {
                return None;
            }
            if let Some(t) = find() {
                return Some(t);
            }
            if self.busy_poll {
                std::hint::spin_loop();
                std::thread::yield_now();
                continue;
            }
            // Asleep: a task queued, or the run's end, after this looks again is seen by that
            // look, or wakes this worker.
            let seen = self.asleep.sleep(|| {
                if self.over.load(Ordering::Acquire) {
                    Some(None)
                } else {
                    find().map(Some)
                }
            });
            if let Some(t) = seen {
                return t;
            }
        }
    }
}

/// An executor's workers and queues, for a run.
pub(crate) struct Group {
    pub(crate) queues: Arc<Queues>,
    pub(crate) workers: Vec<std::thread::JoinHandle<()>>,
}

/// How many workers executor `e` has: one a core it names, or `n` for the default one (no
/// cores named); one for an affine one; its thread count for one for blocking calls.
pub(crate) fn workers(e: &Executor, n: usize) -> usize {
    match e.kind {
        AFFINE => 1,
        BLOCKING => usize::try_from(e.threads).unwrap_or(1).max(1),
        _ => cores(e).map_or(n, |c| c.len().max(1)),
    }
}

/// The cores executor `e` names: its range, or, with none, those of its NUMA node, if it names
/// one (`crate::numa`).
fn cores(e: &Executor) -> Option<Vec<usize>> {
    if e.cores.1 > e.cores.0 {
        let (a, b) = (
            usize::try_from(e.cores.0).ok()?,
            usize::try_from(e.cores.1).ok()?,
        );
        return Some((a..b).collect());
    }
    let node = usize::try_from(e.node).ok()?;
    crate::numa::cores_of(node).map(<[usize]>::to_vec)
}

/// Starts executor `id` (`e`): its workers, `n` of them, each running `work` with the queues;
/// `lenders` are the run's lending executors' queues, which a lending one joins.
pub(crate) fn start(
    id: i64,
    e: &Executor,
    n: usize,
    lenders: &Arc<RwLock<Vec<Arc<Queues>>>>,
    over: &Arc<AtomicBool>,
    work: impl Fn(&Arc<Queues>) + Clone + Send + 'static,
) -> Group {
    let locals: Vec<Worker<usize>> = (0..n).map(|_| Worker::new_fifo()).collect();
    let lends = matches!(e.kind, POOL);
    let queues = Arc::new(Queues {
        id,
        shared: Injector::new(),
        stealers: locals.iter().map(Worker::stealer).collect(),
        asleep: Sleepers::default(),
        busy_poll: e.kind == PINNED && e.busy_poll,
        lends,
        lenders: lenders.clone(),
        over: over.clone(),
    });
    if lends {
        lenders
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(queues.clone());
    }
    let ids = core_affinity::get_core_ids().unwrap_or_default();
    // A pinned or pool executor naming cores, or a NUMA node: one of them a worker.
    let named = matches!(e.kind, PINNED | POOL).then(|| cores(e)).flatten();
    let mut workers = Vec::new();
    for (i, local) in locals.into_iter().enumerate() {
        let q = queues.clone();
        let work = work.clone();
        let core = named
            .as_ref()
            .and_then(|c| c.get(i))
            .and_then(|&c| ids.iter().find(|id| id.id == c).copied());
        let priority = e.priority;
        let w = std::thread::Builder::new()
            .name(format!("moddil-executor-{id}"))
            .spawn(move || {
                if let Some(core) = core {
                    core_affinity::set_for_current(core);
                }
                set_priority(priority);
                LOCAL.with(|l| *l.borrow_mut() = Some((id, local)));
                work(&q);
            })
            .expect("a worker thread");
        workers.push(w);
    }
    Group { queues, workers }
}

/// Gives this thread the OS's priority for executor priority `p` (0 low to 3 real-time): what
/// the OS refuses (a real-time priority without the right to it) is left as it was.
fn set_priority(p: i64) {
    use thread_priority::{ThreadPriority, ThreadPriorityValue, set_current_thread_priority};
    let p = match p {
        0 => ThreadPriority::Min,
        2 => ThreadPriority::Crossplatform(ThreadPriorityValue::try_from(75u8).unwrap_or_default()),
        3 => ThreadPriority::Max,
        _ => return,
    };
    let _ = set_current_thread_priority(p);
}
