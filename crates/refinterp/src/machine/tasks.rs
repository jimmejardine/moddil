//! Tasks (`task.scope.1`): `scope`, `spawn`, `join` and `yield_now`, independently of the MIR
//! interpreter's scheduler.
//!
//! Each task runs on a thread of its own, with a machine of its own and its own parse of the
//! program (a syntax node is not shared between threads), and the memory shared
//! ([`crate::memory`]). One runs at a time: the running task hands the turn on, first in, first
//! out, only where it suspends (`task.sched.1`), and waits for it to come back.

use std::collections::VecDeque;
use std::sync::Arc;

use moddil_syntax::TextRange;

use crate::memory::{Memory, Shared};
use crate::program::Program;
use crate::value::{Head, Value};
use crate::{Eval, Flow, ModSpec, SourceSpec, Trap};

use super::Machine;

/// How a task ended: its value, or its error.
type Outcome = Result<Value, Value>;

/// What a suspended task waits for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Wait {
    Join(usize),
    Scope(usize),
}

#[derive(Default)]
struct TaskState {
    outcome: Option<Outcome>,
    ended: Option<usize>,
    joined: bool,
    waiting: Option<Wait>,
}

/// The tasks and scopes of a test, which every task's thread shares.
pub(crate) struct Tasks {
    states: Vec<TaskState>,
    scopes: Vec<Vec<usize>>,
    ready: VecDeque<usize>,
    ended: usize,
    /// The task whose turn it is.
    running: usize,
    /// The trap that ended the test, once one has, whichever task it was in: every task stops.
    trapped: Option<(Trap, bool)>,
    /// What the tasks other than the test's observed.
    observed: Vec<(u32, TextRange, String)>,
    threads: Vec<std::thread::JoinHandle<()>>,
    /// Whether a task's thread panicked (a bug in the interpreter): every task stops.
    panicked: bool,
}

impl Tasks {
    pub(crate) fn new() -> Self {
        Tasks {
            states: vec![TaskState::default()],
            scopes: Vec::new(),
            ready: VecDeque::new(),
            ended: 0,
            running: 0,
            trapped: None,
            observed: Vec::new(),
            threads: Vec::new(),
            panicked: false,
        }
    }

    fn end(&mut self, t: usize, outcome: Outcome) {
        let task = &mut self.states[t];
        task.outcome = Some(outcome);
        task.ended = Some(self.ended);
        self.ended += 1;
        for w in 0..self.states.len() {
            let wake = match self.states[w].waiting {
                Some(Wait::Join(j)) => j == t,
                Some(Wait::Scope(s)) => self.scope_done(s),
                None => false,
            };
            if wake {
                self.states[w].waiting = None;
                self.ready.push_back(w);
            }
        }
    }

    fn take(&mut self, t: usize) -> Option<Outcome> {
        let task = &mut self.states[t];
        let outcome = task.outcome.take()?;
        task.joined = true;
        Some(outcome)
    }

    fn scope_done(&self, s: usize) -> bool {
        self.scopes[s]
            .iter()
            .all(|&t| self.states[t].ended.is_some())
    }

    /// The error of the first of the scope's tasks to fail, by when it ended, of those never
    /// joined (`task.error.1`).
    fn scope_error(&mut self, s: usize) -> Option<Value> {
        let first = self.scopes[s]
            .iter()
            .copied()
            .filter(|&t| {
                let task = &self.states[t];
                !task.joined && matches!(task.outcome, Some(Err(_)))
            })
            .min_by_key(|&t| self.states[t].ended)?;
        match self.states[first].outcome.take() {
            Some(Err(e)) => Some(e),
            _ => None,
        }
    }

    /// Gives the turn to the next ready task.
    fn hand_on(&mut self) {
        self.running = self
            .ready
            .pop_front()
            .expect("a task is ready while the test has not ended");
    }
}

/// The program's sources, which each task parses again.
pub(crate) type Sources = Arc<(Vec<SourceSpec>, Vec<ModSpec>)>;

impl Machine<'_> {
    /// The prelude's tasks and scopes: `name` (of a value of kind `head`) on `args`, if it is one
    /// of them.
    pub(super) fn task_intrinsic(
        &mut self,
        name: &str,
        head: &Head,
        args: &[Value],
        range: TextRange,
    ) -> Option<Eval> {
        let id = |m: &Self, v: &Value| match m.target(v.clone()) {
            Value::Int(i) => i as usize,
            Value::Adt { fields, .. } => match fields[0] {
                Value::Int(i) => i as usize,
                _ => unreachable!("an id"),
            },
            _ => unreachable!("a task or scope"),
        };
        Some(match (head, name) {
            (Head::Unit, "__scope_open") => {
                let s = self.with_tasks(|t| {
                    t.scopes.push(Vec::new());
                    t.scopes.len() - 1
                });
                Ok(Value::Int(s as i64))
            }
            (Head::Adt(k), "spawn") if k == "Scope" => {
                let scope = id(self, &args[0]);
                Ok(self.spawn(scope, args[1].clone(), range))
            }
            (Head::Adt(k), "join") if k == "Task" => {
                let t = id(self, &args[0]);
                let outcome = match self.with_tasks(|ts| ts.take(t)) {
                    Some(o) => o,
                    None => {
                        if let Err(f) =
                            self.suspend(|ts, me| ts.states[me].waiting = Some(Wait::Join(t)))
                        {
                            return Some(Err(f));
                        }
                        self.with_tasks(|ts| ts.take(t)).expect("an ended task")
                    }
                };
                outcome.map_err(|e| Flow::Fail(e, range))
            }
            (Head::Unit, "yield_now") => self
                .suspend(|ts, me| ts.ready.push_back(me))
                .map(|()| Value::Unit),
            (Head::Unit, "__scope_wait" | "__scope_close") => {
                let s = id(self, &args[0]);
                if !self.with_tasks(|ts| ts.scope_done(s))
                    && let Err(f) =
                        self.suspend(|ts, me| ts.states[me].waiting = Some(Wait::Scope(s)))
                {
                    return Some(Err(f));
                }
                match self.with_tasks(|ts| ts.scope_error(s)) {
                    Some(e) if name == "__scope_close" => Err(Flow::Fail(e, range)),
                    _ => Ok(Value::Unit),
                }
            }
            _ => return None,
        })
    }

    /// Runs `f` on the shared tasks.
    fn with_tasks<R>(&self, f: impl FnOnce(&mut Tasks) -> R) -> R {
        let shared = self.memory.shared();
        let mut s = shared.0.lock().expect("the memory's lock");
        f(&mut s.tasks)
    }

    /// Suspends the running task: `park` records why (or queues it again), the next ready task
    /// gets the turn, and this one waits for it to come back. A trap in another task ends it.
    fn suspend(&mut self, park: impl FnOnce(&mut Tasks, usize)) -> Eval<()> {
        let shared = self.memory.shared();
        let me = self.memory.task();
        let (lock, turn) = &*shared;
        let mut s = lock.lock().expect("the memory's lock");
        park(&mut s.tasks, me);
        s.tasks.hand_on();
        turn.notify_all();
        while s.tasks.running != me && s.tasks.trapped.is_none() && !s.tasks.panicked {
            s = turn.wait(s).expect("the memory's lock");
        }
        assert!(!s.tasks.panicked, "a task's thread panicked");
        match s.tasks.trapped {
            Some((t, placed)) => Err(Flow::Trap(t, placed)),
            None => Ok(()),
        }
    }

    /// Starts a task running closure `f` in `scope`, queued; its value (`task.spawn.1`).
    fn spawn(&mut self, scope: usize, f: Value, range: TextRange) -> Value {
        let t = self.memory.add_task();
        self.with_tasks(|ts| {
            ts.states.push(TaskState::default());
            ts.scopes[scope].push(t);
            ts.ready.push_back(t);
        });
        let shared = self.memory.shared();
        let sources = self.program.sources();
        let thread = std::thread::Builder::new()
            .stack_size(1 << 30)
            .spawn(move || run_task(&shared, t, &sources, f, range))
            .expect("a thread for the task");
        self.with_tasks(|ts| ts.threads.push(thread));
        Value::Adt {
            ty: "Task".to_string(),
            variant: "Task".to_string(),
            fields: vec![Value::Int(t as i64)],
        }
    }

    /// At the end of the test: what its other tasks observed, once their threads have ended.
    pub(super) fn end_tasks(&mut self) {
        let (threads, observed) = self.with_tasks(|ts| {
            (
                std::mem::take(&mut ts.threads),
                std::mem::take(&mut ts.observed),
            )
        });
        for t in threads {
            let _ = t.join();
        }
        self.observed.extend(observed);
    }
}

/// Task `t`'s thread: once it has the turn, it runs closure `f`, records how it ended, and hands
/// the turn on.
fn run_task(shared: &Shared, t: usize, sources: &Sources, f: Value, range: TextRange) {
    let program = Program::new(&sources.0, &sources.1);
    let (lock, turn) = &**shared;
    {
        let mut s = lock.lock().expect("the memory's lock");
        while s.tasks.running != t && s.tasks.trapped.is_none() && !s.tasks.panicked {
            s = turn.wait(s).expect("the memory's lock");
        }
        if s.tasks.trapped.is_some() || s.tasks.panicked {
            return;
        }
    }
    let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut m = Machine::new(&program, Memory::of_task(shared.clone(), t));
        let Value::Closure(c) = f else {
            unreachable!("a task's closure")
        };
        // The task's first frame is where its closure was written.
        m.memory
            .push_frame(std::collections::HashMap::new(), c.file, c.module, false);
        let result = m.call_closure(&c, Vec::new(), range);
        (result, std::mem::take(&mut m.observed))
    }));
    // A bug in the interpreter: the other tasks are woken, and find the lock poisoned.
    let Ok((result, mut observed)) = run else {
        let mut s = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        s.tasks.panicked = true;
        turn.notify_all();
        return;
    };
    let mut s = lock.lock().expect("the memory's lock");
    s.tasks.observed.append(&mut observed);
    match result {
        Ok(v) => s.tasks.end(t, Ok(v)),
        Err(Flow::Fail(e, _)) => s.tasks.end(t, Err(e)),
        Err(Flow::Trap(trap, placed)) => {
            s.tasks.trapped.get_or_insert((trap, placed));
            turn.notify_all();
            return;
        }
        Err(_) => unreachable!("a closure's body leaves only by its value, an error or a trap"),
    }
    s.tasks.hand_on();
    turn.notify_all();
}
