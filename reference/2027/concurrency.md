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

**task.scope.1** `scope(body)` calls `body` with `&mut Scope`, a handle that tasks are spawned
into, then waits for every task spawned into it to end, and gives `body`'s value. If `body` fails,
the scope still waits for its tasks, then fails with `body`'s error. `scope` needs `spawn`
(**task.effects.1**).

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
or at `yield_now()`, which needs `suspend` and lets the ready tasks run first. Then the task at
the front of the queue runs. So a program's tasks run in the same order every time, for now:
other schedules, and tasks running in parallel, come with the runtimes.

**task.error.1** A task's error goes to its `join`. Once the scope's body has ended, if a task
that was never joined has failed, the scope fails with the error of the first such task to end;
the others' errors are dropped, for now. The scope's other tasks still run to their end: a failed
task does not cancel them yet.

**task.borrow.1** A task's closure may borrow what outlives the scope: a shared borrow by any
number of tasks, a `&mut` borrow by one. The scope keeps what its tasks borrow until it ends, so
the code around them cannot use what a task borrows mutably, or change what it borrows, before
then (E0305, E0306). A task cannot borrow a local of the scope's body, which ends before the
tasks do (E0308).

**task.effects.1** `suspend` and `spawn` are the control effects ([Effects](effects.md)): a
function that may suspend its task needs `suspend`, and one that starts tasks needs `spawn`. A
task's other effects are met where its closure is written, so tasks use the handlers installed
around their scope ([Closures](expressions/closures.md)).
