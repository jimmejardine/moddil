# Concurrency

```rust
fn sum(xs: &[int]) -> int { … }

fn total(left: &[int], right: &[int]) -> int {      // with spawn, suspend: inferred
    scope(|s| {
        let a = s.spawn(|| sum(left));               // two tasks share the borrows
        let b = s.spawn(|| sum(right));
        a.join() + b.join()
    })                                               // both have ended here
}
```

A **task** runs a closure alongside the code that started it. Tasks are structured: each belongs
to a scope, which ends only once all of its tasks have
([`specs/concurrency.md`: Tasks and borrowing](../../specs/concurrency.md#tasks-and-borrowing)).

**task.scope.1** `scope(body)` runs `body` as the scope's first task, at once, with `&mut Scope`,
a handle that further tasks are spawned into; the code that called `scope` waits for every task of
the scope to end, then the call gives `body`'s value, or fails with the scope's failure
(**task.error.1**, **task.isolate.1**). `scope` needs `spawn` (**task.effects.1**).

**task.spawn.1** `s.spawn(f)` starts a task running closure `f` (taking nothing), queued to run,
and gives a `Task` for it. A call that passes a closure that can fail is marked, as any such call
is (E0501, [Closures](expressions/closures.md)): so the scope's errors are those of its body and
of every task spawned into it. `spawn` itself does not fail: a task's error goes where
**task.error.1** says.

**task.join.1** `t.join()` waits for task `t` to end and gives its result, or fails with its error.
It takes `t`, so a task is joined at most once. `join` needs `suspend`.

**task.sched.1** One task runs at a time. A task that is spawned, or woken because what it waited
for has happened, joins the back of a queue of ready tasks. The running task runs until it
suspends: at a `join` of a task that has not ended, at a scope's end while its tasks have not,
at a channel's `send` or `recv` that must wait, at a `select` or `stop_requested()` with nothing
ready, or at `yield_now()`, which needs `suspend` and lets the ready tasks run first. Then the task
at the front of the queue runs; with none ready, every task waits for another, and the program
traps (a deadlock). So a program's tasks run in the same order every time, for now:
other schedules, and tasks running in parallel, come with the runtimes.

**task.error.1** A task's error goes to its `join`. When a task whose `Task` has been dropped (so
it can no longer be joined) fails, or has failed, or the body fails, the scope **fails** with that
error and is cancelled (**task.cancel.1**). Only the first failure counts: the others' errors are
dropped, for now.

**task.cancel.1** A cancelled scope cancels its tasks, its body's included, and the scopes they
have opened, and so on in. A cancelled task runs on until it next suspends (**task.sched.1**); there
it **unwinds** instead: it leaves every block it is in as an error would, running their `defer`s,
`errdefer`s and drops ([Statements](statements.md)), and ends. A task waiting at a scope's end
waits for that scope's tasks first. A `join` of a cancelled task unwinds the joining task, whose
scope is cancelled too.

**task.cancel.2** While a task unwinds, its cleanup is not cancelled again: its suspension points
wait as they would. A trap while unwinding, or in a destructor, ends the program.

**task.isolate.1** `scope(body, isolate: true)` is **isolated**: a trap in one of its tasks, or in
the tasks of the scopes inside it that are not isolated themselves, does not end the program but
fails the scope, cancelling it, with the prelude's `Trapped { class, at }`: the trap's kind
(`overflow`) and its `file:line:column`, the file as messages show it. The task that trapped unwinds from where it trapped
(**task.cancel.1**). Only an isolated scope's call can fail with `Trapped`: one whose `isolate` is
left out, or known to be `false` when compiling, cannot. Known are `true` and `false`, `!`, `&&`
and `||` of known values, and `bool` const items of them; any other `isolate` is taken as one
that may be `true`.

**task.isolate.2** A trap outside any isolated scope ends the program, as before
([Traps](traps.md)).

**task.channel.1** `channel::<T>(n)` makes a channel holding up to `n` values (a negative `n`
traps), and gives its `Sender<T>` and `Receiver<T>`. `tx.send(v)` puts `v` in it, waiting while it
is full; with `n` of 0, `send` also waits until `v` has been received (a rendezvous). `rx.recv()`
gives the next value, in the order sent, waiting while there is none. Both need `suspend`.

**task.channel.2** A `Sender` can be cloned. Once every `Sender` of a channel is dropped, `recv`
gives `None` when the channel is empty; once its `Receiver` is dropped, `send` fails with the
prelude's `Closed`, and the values left in the channel are dropped, their destructors run.

**task.channel.3** The ends of a channel are values like any other: a task borrows them, or has
them moved into its closure (`let tx = tx;`), so that the channel closes when the task ends.

**task.timer.1** A run's time is **virtual**: it starts at zero and moves only when every task
waits, straight to the earliest timer, which fires then. `Duration` is a length of time
(`Duration::seconds(n)`, `Duration::millis(n)`, `d.nanos()`); `timer(d)` is a `Receiver<()>`
whose one value arrives once `d` has passed, after which it has no sender, so a `select` arm
`_ = timer(d).recv()` is a timeout; `sleep(d)` waits until `d` has passed. Timers that fire at the
same time fire in the order they were made.

**task.select.1** `select { p = op => body, … }` waits on several operations, each an arm's `op`:
`rx.recv()`, `tx.send(v)`, `t.join()` or `stop_requested()` (E0217), maybe marked with `?`. Every
arm's handle and value to send are evaluated first, in order; then the first arm, in written
order, whose operation is ready does it, its pattern (which matches every value, E0209) binds the
result, and its body is the `select`'s value. With none ready, the task waits until one is. The
other arms' operations are not done: a value an arm would have sent is dropped. An error of an
operation marked with `?` passes on as `?` passes it; a `send` in a `select` does not wait for its
value to be received.

**task.select.2** The arms' values have one type (E0201), as a `match`'s do.

**task.select.3** `#[selectable]` on a function that needs `suspend` (E0217 otherwise) makes a call
of it an operation a `select` arm may wait on, as a library's `listener.accept()` is. It is called
as any other function.

**task.select.4** A selectable function **receives first**: its body is one expression that
starts with `p.recv()`, where `p` is a parameter or a field of one, read without running anything;
the rest works with what was received (`match self.incoming.recv() { Some(c) => …, None => fail(…) }`)
(E0217 otherwise). A `select` arm on a call of it evaluates the call's arguments, then waits on
`p`'s channel as an arm `p.recv()` would; if the arm is chosen, the rest of the body runs with what
was received, and its value is the arm's result, its error passing on as the arm's `?` says. The
rest runs where the `select` is, so it may fail but uses no handler (E0217).

**task.stop.1** `s.stop()` asks scope `s` to stop. `stop_requested()`, which needs `suspend`,
waits until a stop reaches the running task: it is in `s` or in a scope inside it, and every scope
the tasks of its own scope have opened has ended, so the innermost scopes stop first. A stop
cancels nothing: the code that asks for it chooses what to do, typically to stop taking new work
and let the work in hand finish.

**task.borrow.1** A task's closure may borrow what outlives the scope: a shared borrow by any
number of tasks, a `&mut` borrow by one. The scope keeps what its tasks borrow until it ends, so
the code around them cannot use what a task borrows mutably, or change what it borrows, before
then (E0305, E0306). A task cannot borrow a local of the scope's body, which ends before the
tasks do (E0308).

**task.effects.1** `suspend` and `spawn` are the control effects ([Effects](effects.md)): a
function that may suspend its task needs `suspend`, and one that starts tasks needs `spawn`. A
task's other effects are met where its closure is written, so tasks use the handlers installed
around their scope ([Closures](expressions/closures.md)).

**task.with.1** A task may not outlive a handler it uses: a task whose closure uses a handler a
`with` installs must be spawned into a scope opened inside that `with`, which ends, and so ends
its tasks, before the handler does. Spawning it into a scope opened outside the `with` is an
error (E0610).
