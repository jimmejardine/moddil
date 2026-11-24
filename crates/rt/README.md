# moddil-rt

The runtime that native Moddil code calls into ([`specs/roadmap.md`: M1](../../specs/roadmap.md)).
Slice C0 has the functions JIT code calls when a check fails (`trap`) and when a test observes a
value (`observe`), and the float remainder Cranelift has no instruction for. Each is a plain Rust
function whose address `moddil-native` gives the JIT; what they record is kept per thread, and
the host takes it once a run ends.

- **Tasks** ([`src/run.rs`](src/run.rs), [`src/sched.rs`](src/sched.rs)): one task runs at a
  time, holding the baton, in the order of the scheduler it shares with the MIR interpreter
  ([`moddil-sched`](../sched/src/lib.rs)). What differs between targets is the baton ([`src/baton.rs`](src/baton.rs)):
  - natively, a thread per task, waiting on a condition variable;
  - built for `wasm32` ([`moddil-rt-wasm`](../rt-wasm/README.md)), a JSPI call per task, which
    suspends in the host's `switch` until its turn comes again.
- **Site tables** ([`src/sites.rs`](src/sites.rs)): the text of a trap for a program run without
  the compiler, shared by `moddil-rt-main` and `moddil-rt-wasm`.
