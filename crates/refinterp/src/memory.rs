//! Memory: one frame per active call of each task, each a list of variables, and [`Pointer`]s
//! into them.
//!
//! Variables are kept in the order they were reached, so leaving a scope truncates its frame
//! back to a mark (`own.drop.1`). A borrow is a pointer to a variable of some frame of some task
//! and a path of field positions in it. Tasks run on threads of their own, one at a time
//! (`task.sched.1`), so the memory they share is behind a lock, which each access takes for no
//! longer than it lasts.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use crate::machine::tasks::Tasks;
use crate::value::{Head, Pointer, Value};

/// One call's variables and the kinds of its type parameters.
struct Frame {
    /// Its variables, innermost last. Temporaries that are borrowed or dropped get a nameless
    /// variable.
    vars: Vec<(String, Value)>,
    /// The kind of each type parameter it knows (and of `Self`).
    types: HashMap<String, Head>,
    /// The file of the code it runs, or `None` for the prelude's.
    file: Option<u32>,
    /// The module whose names its code uses.
    module: usize,
    /// Whether its function is `#[track_caller]`.
    tracked: bool,
}

/// What the tasks of a test share: their frames, the static data, and the scheduler.
pub(crate) struct Store {
    /// Each task's frames, by the task's index (the test's first).
    frames: Vec<Vec<Frame>>,
    /// The values of the const items used so far, by key: static data, which a [`Pointer`] with
    /// [`STATIC`] for its frame points into (`borrow.static.1`).
    statics: Vec<(String, Value)>,
    /// The values `core::ffi::into_raw` keeps alive behind a raw pointer (`ffi.core.1`), which a
    /// [`Pointer`] with [`HEAP`] for its frame points into, until `from_raw` takes them back.
    heap: HashMap<usize, Value>,
    next_heap: usize,
    /// The tasks and scopes.
    pub(crate) tasks: Tasks,
}

/// The shared store, and the condition a task waits on for its turn.
pub(crate) type Shared = Arc<(Mutex<Store>, Condvar)>;

/// One task's view of the memory.
pub(crate) struct Memory {
    shared: Shared,
    task: usize,
}

/// The frame of a pointer to static data: its slot is the const's place among the statics.
pub(crate) const STATIC: usize = usize::MAX;

/// The frame of a raw pointer to a value on the heap: its slot is the heap's.
pub(crate) const HEAP: usize = usize::MAX - 1;

impl Memory {
    /// The memory of a test: one task, with one frame, running the code of `file`, in `module`.
    pub(crate) fn new(file: u32, module: usize) -> Memory {
        let store = Store {
            frames: vec![vec![Frame {
                vars: Vec::new(),
                types: HashMap::new(),
                file: Some(file),
                module,
                tracked: false,
            }]],
            statics: Vec::new(),
            heap: HashMap::new(),
            next_heap: 0,
            tasks: Tasks::new(),
        };
        Memory {
            shared: Arc::new((Mutex::new(store), Condvar::new())),
            task: 0,
        }
    }

    /// The memory as task `task` sees it, once the task has been added with
    /// [`Memory::add_task`].
    pub(crate) fn of_task(shared: Shared, task: usize) -> Memory {
        Memory { shared, task }
    }

    /// Adds a task, with no frames yet, and gives its index.
    pub(crate) fn add_task(&self) -> usize {
        let mut s = self.lock();
        s.frames.push(Vec::new());
        s.frames.len() - 1
    }

    /// What the tasks share.
    pub(crate) fn shared(&self) -> Shared {
        self.shared.clone()
    }

    /// The task this is the memory of.
    pub(crate) fn task(&self) -> usize {
        self.task
    }

    fn lock(&self) -> MutexGuard<'_, Store> {
        self.shared.0.lock().expect("the memory's lock")
    }

    /// Runs `f` on this task's frames.
    fn frames<R>(&self, f: impl FnOnce(&Vec<Frame>) -> R) -> R {
        f(&self.lock().frames[self.task])
    }

    /// Runs `f` on this task's innermost frame.
    fn frame<R>(&self, f: impl FnOnce(&Frame) -> R) -> R {
        self.frames(|fs| f(fs.last().expect("a frame")))
    }

    fn frame_mut<R>(&mut self, f: impl FnOnce(&mut Frame) -> R) -> R {
        let task = self.task;
        f(self.lock().frames[task].last_mut().expect("a frame"))
    }

    /// Where const item `key`'s value is, if it has been computed.
    pub(crate) fn static_of(&self, key: &str) -> Option<Pointer> {
        self.lock()
            .statics
            .iter()
            .position(|(k, _)| k == key)
            .map(|slot| Pointer {
                task: 0,
                frame: STATIC,
                slot,
                path: Vec::new(),
                span: None,
            })
    }

    /// Keeps const item `key`'s value, and gives where it is.
    pub(crate) fn add_static(&mut self, key: String, v: Value) -> Pointer {
        let mut s = self.lock();
        s.statics.push((key, v));
        Pointer {
            task: 0,
            frame: STATIC,
            slot: s.statics.len() - 1,
            path: Vec::new(),
            span: None,
        }
    }

    /// Keeps `v` on the heap, and gives where it is (`ffi.core.1`).
    pub(crate) fn alloc(&mut self, v: Value) -> Pointer {
        let mut s = self.lock();
        let slot = s.next_heap;
        s.next_heap += 1;
        s.heap.insert(slot, v);
        Pointer {
            task: 0,
            frame: HEAP,
            slot,
            path: Vec::new(),
            span: None,
        }
    }

    /// The value at heap pointer `p`, taken off the heap.
    pub(crate) fn free(&mut self, p: &Pointer) -> Value {
        assert!(
            p.frame == HEAP && p.path.is_empty(),
            "`from_raw` of a pointer `into_raw` did not give"
        );
        self.lock()
            .heap
            .remove(&p.slot)
            .expect("`from_raw` of a pointer already given back")
    }

    /// How many calls of this task are active.
    pub(crate) fn depth(&self) -> usize {
        self.frames(Vec::len)
    }

    /// Enters a call, with the kinds of its type parameters, and the file (`None` for the
    /// prelude) and module of its code.
    pub(crate) fn push_frame(
        &mut self,
        types: HashMap<String, Head>,
        file: Option<u32>,
        module: usize,
        tracked: bool,
    ) {
        let task = self.task;
        self.lock().frames[task].push(Frame {
            vars: Vec::new(),
            types,
            file,
            module,
            tracked,
        });
    }

    /// Whether the innermost call's function is `#[track_caller]`.
    pub(crate) fn tracked(&self) -> bool {
        self.frame(|f| f.tracked)
    }

    /// Leaves the innermost call; its variables must already be dropped.
    pub(crate) fn pop_frame(&mut self) {
        let task = self.task;
        self.lock().frames[task].pop();
    }

    /// Whether the innermost call runs the files' code (not the prelude's).
    pub(crate) fn user(&self) -> bool {
        self.frame(|f| f.file.is_some())
    }

    /// The file of the innermost call's code, if it is one of the program's.
    pub(crate) fn file(&self) -> Option<u32> {
        self.frame(|f| f.file)
    }

    /// The module whose names the innermost call's code uses.
    pub(crate) fn module(&self) -> usize {
        self.frame(|f| f.module)
    }

    /// The kind of type parameter `name` in the innermost call, if it knows it.
    pub(crate) fn type_of(&self, name: &str) -> Option<Head> {
        self.frame(|f| f.types.get(name).cloned())
    }

    /// How many variables the innermost call has: a point to [`Memory::truncate`] back to.
    pub(crate) fn mark(&self) -> usize {
        self.frame(|f| f.vars.len())
    }

    /// Adds variable `name` (empty for a temporary) holding `v` to the innermost call.
    pub(crate) fn push(&mut self, name: String, v: Value) {
        self.frame_mut(|f| f.vars.push((name, v)));
    }

    /// Removes the innermost call's last variable.
    pub(crate) fn pop(&mut self) -> Option<(String, Value)> {
        self.frame_mut(|f| f.vars.pop())
    }

    /// Removes the innermost call's variables from `mark` on, without dropping them.
    pub(crate) fn truncate(&mut self, mark: usize) {
        self.frame_mut(|f| f.vars.truncate(mark));
    }

    /// The value of the innermost call's variable `i` if it is a temporary (nameless).
    pub(crate) fn temp_value(&self, i: usize) -> Option<Value> {
        self.frame(|f| {
            let (name, v) = &f.vars[i];
            name.is_empty().then(|| v.clone())
        })
    }

    /// Takes the value out of the innermost call's variable `i`, leaving it empty.
    pub(crate) fn take(&mut self, i: usize) -> Value {
        self.frame_mut(|f| std::mem::replace(&mut f.vars[i].1, Value::Moved))
    }

    /// The innermost variable called `name` in the innermost call.
    pub(crate) fn slot(&self, name: &str) -> Option<usize> {
        self.frame(|f| f.vars.iter().rposition(|(n, _)| n == name))
    }

    /// A pointer to the innermost call's variable `slot`.
    /// The running task and its innermost frame.
    pub(crate) fn current_frame(&self) -> (usize, usize) {
        let task = self.task;
        self.frames(|fs| (task, fs.len() - 1))
    }

    pub(crate) fn here(&self, slot: usize) -> Pointer {
        let task = self.task;
        self.frames(|fs| {
            // A closure's body sees the variables it captures as aliases of them.
            if let Value::Alias(p) = &fs.last().expect("a frame").vars[slot].1 {
                return p.clone();
            }
            Pointer {
                task,
                frame: fs.len() - 1,
                slot,
                path: Vec::new(),
                span: None,
            }
        })
    }

    /// The innermost call's named variables, each once (the innermost of a name), as pointers.
    pub(crate) fn visible(&self) -> Vec<(String, Pointer)> {
        let names: Vec<String> = self.frame(|f| f.vars.iter().map(|(n, _)| n.clone()).collect());
        let mut out: Vec<(String, Pointer)> = Vec::new();
        for (slot, name) in names.into_iter().enumerate() {
            if name.is_empty() {
                continue;
            }
            let p = self.here(slot);
            match out.iter_mut().find(|(n, _)| *n == name) {
                Some(e) => e.1 = p,
                None => out.push((name, p)),
            }
        }
        out
    }

    /// The kinds of the innermost call's type parameters.
    pub(crate) fn types(&self) -> HashMap<String, Head> {
        self.frame(|f| f.types.clone())
    }

    /// A nameless variable holding `v`, so that it can be borrowed or dropped.
    pub(crate) fn temp(&mut self, v: Value) -> Pointer {
        self.push(String::new(), v);
        self.here(self.mark() - 1)
    }

    /// The value at `ptr`.
    pub(crate) fn load(&self, ptr: &Pointer) -> Value {
        let s = self.lock();
        let mut v = if ptr.frame == STATIC {
            &s.statics[ptr.slot].1
        } else if ptr.frame == HEAP {
            &s.heap[&ptr.slot]
        } else {
            &s.frames[ptr.task][ptr.frame].vars[ptr.slot].1
        };
        for &i in &ptr.path {
            v = match v {
                Value::Tuple(xs)
                | Value::Array(xs)
                | Value::Buf(_, xs)
                | Value::Adt { fields: xs, .. } => &xs[i],
                _ => unreachable!("a checked field"),
            };
        }
        // Part of an array (`expr.slice.1`).
        if let Some((start, n)) = ptr.span {
            return match v {
                Value::Array(xs) | Value::Buf(_, xs) => Value::Array(xs[start..start + n].to_vec()),
                _ => unreachable!("a checked slice"),
            };
        }
        v.clone()
    }

    /// Stores `value` at `ptr`.
    pub(crate) fn store(&mut self, ptr: &Pointer, value: Value) {
        let mut s = self.lock();
        // A static slot: a const's, or what a closure that outlives its scope captured.
        let mut v = if ptr.frame == STATIC {
            &mut s.statics[ptr.slot].1
        } else if ptr.frame == HEAP {
            s.heap.get_mut(&ptr.slot).expect("a live heap value")
        } else {
            &mut s.frames[ptr.task][ptr.frame].vars[ptr.slot].1
        };
        for &i in &ptr.path {
            v = match v {
                Value::Tuple(xs)
                | Value::Array(xs)
                | Value::Buf(_, xs)
                | Value::Adt { fields: xs, .. } => &mut xs[i],
                _ => unreachable!("a checked field"),
            };
        }
        // Part of an array: its elements are replaced.
        if let Some((start, n)) = ptr.span {
            if let (Value::Array(xs) | Value::Buf(_, xs), Value::Array(ys)) = (v, value) {
                xs.splice(start..start + n, ys);
            }
            return;
        }
        *v = value;
    }
}
