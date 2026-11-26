# moddil-rt

The runtime that native Moddil code calls into ([`specs/roadmap.md`: M1](../../specs/roadmap.md)).
Slice C0 has the functions JIT code calls when a check fails (`trap`) and when a test observes a
value (`observe`), and the float remainder Cranelift has no instruction for. Each is a plain Rust
function whose address `moddil-native` gives the JIT; what they record is kept per thread, and
the host takes it once a run ends.

- **Tasks** ([`src/run.rs`](src/run.rs), [`src/sched.rs`](src/sched.rs)): the tasks with a turn
  run ([`src/turns.rs`](src/turns.rs)), given out in the order of the scheduler it shares with
  the MIR interpreter ([`moddil-sched`](../sched/src/lib.rs)): one turn under the deterministic
  runtime, so one task runs at a time; one a core under the parallel runtime (`set_workers`,
  `with Runtime = runtime::cpus::parallel(&cpus)`), so tasks run at once. A trap then stops the
  running ones at their next safe point. A run of `main` under it has a reactor
  ([`src/reactor.rs`](src/reactor.rs)), a thread waiting in the OS's wait for readiness without
  the run's lock, which brings the OS's events and fires the timers while tasks run. A task
  that has run its 10 ms slice gives its turn
  to a ready one there: a thread sets the safe-point word for the last 200 µs of each slice. What differs between targets is the baton
  ([`src/baton.rs`](src/baton.rs)):
  - natively, a coroutine per task (`corosensei`), each with a 256 MiB stack reserved and
    committed as it is used, resumed by a worker thread (one a turn) once it has a turn: each executor has its workers and queues ([`src/executors.rs`](src/executors.rs)): a worker takes from its own queue, then its executor's shared one, then steals, then, in a pool, from the other pools, and sleeps when there is nothing; a pinned executor's workers are pinned to its cores; under the deterministic runtime there is the default executor's one worker; the
    run's own task runs on the thread that began the run, waiting on a condition variable. What
    a task keeps of its thread's locals (its call sites, its `const` depth) goes with it, and the
    runtime reads its thread's locals afresh after a switch;
  - built for `wasm32` ([`moddil-rt-wasm`](../rt-wasm/README.md)), a JSPI call per task, which
    suspends in the host's `switch` until its turn comes again.
- **Site tables** ([`src/sites.rs`](src/sites.rs)): the text of a trap for a program run without
  the compiler, shared by `moddil-rt-main` and `moddil-rt-wasm`.
