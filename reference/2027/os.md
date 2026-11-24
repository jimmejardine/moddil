# The operating system

The `core` mod's `os` module ([source](core/os.moddil)): what the operating system gives a run.
The standard runtime mod builds the root capabilities over it
([`specs/lifecycle.md`](../../specs/lifecycle.md)), so an application reaches the OS only through
what `main` is given. Added in M2 slice M2d.

**os.1** `core::os` gives a run its arguments (`arg_count`, `arg_len`, `arg_copy`), standard error
(`write_stderr`), random numbers (`random`, greater than 0), the real time (`wall_clock`,
nanoseconds since 1970-01-01T00:00:00 UTC, **os.clock.1**), TCP connections on the local machine
(`listen`, `incoming`, `connect`, `arrivals`, `take`, `send`, `close`) and signals (`signals`: 1 for
an interrupt, Ctrl-C or a `stop` line on standard input; 2 for a request to end). Handles are
numbers, and what the OS brings arrives on channels, so `select` waits on it: a listener's
connections on `incoming`, how many bytes a connection has received on `arrivals`, which `take`
then copies into a buffer, and once it ends, `arrivals` has no sender. A run of `main` has the OS,
and its time is the wall clock's (**task.timer.1**): when no task is ready, the run waits for the
OS until the next timer.

**os.clock.1** `wall_clock()` is the system's real time, which the system can set back, so two
readings need not be in order; the root capability `Clock` reads it (`unix_nanos()`, and `now()`
as a `time::Timestamp`). It is not a task's time (**task.timer.1**), which timers measure.

**os.cap.1** A mod names `core::os` only if its manifest lists the capability,
`capabilities = ["os"]` under `[mod]` (E0220), and only such a mod (or a stub mod) declares root
capabilities (**main.root.1**).

**os.test.1** Under `moddil test` there is no OS: there are no arguments, `listen` and `connect` give
-1, `random` gives the same number each time, `wall_clock` gives 0 (the epoch), `incoming` and
`arrivals` have no sender, and no signal comes. A test's time is virtual (**task.timer.1**).

## Diagnostics

| Code | Meaning |
|---|---|
| E0220 | `core::os` (or `core::gpu`, **gpu.cap.1**) named in a mod without the `os` capability |
