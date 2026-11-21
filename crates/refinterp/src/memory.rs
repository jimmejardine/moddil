//! Memory: one frame per active call, each a list of variables, and [`Pointer`]s into them.
//!
//! Variables are kept in the order they were reached, so leaving a scope truncates its frame
//! back to a mark (`own.drop.1`). A borrow is a pointer to a variable of some frame and a path
//! of field positions in it.

use std::collections::HashMap;

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
}

pub(crate) struct Memory {
    /// One frame per active call (the test first).
    frames: Vec<Frame>,
}

impl Memory {
    /// The memory of a test: one frame, running the code of `file`, in `module`.
    pub(crate) fn new(file: u32, module: usize) -> Memory {
        Memory {
            frames: vec![Frame {
                vars: Vec::new(),
                types: HashMap::new(),
                file: Some(file),
                module,
            }],
        }
    }

    /// How many calls are active.
    pub(crate) fn depth(&self) -> usize {
        self.frames.len()
    }

    /// Enters a call, with the kinds of its type parameters, and the file (`None` for the
    /// prelude) and module of its code.
    pub(crate) fn push_frame(
        &mut self,
        types: HashMap<String, Head>,
        file: Option<u32>,
        module: usize,
    ) {
        self.frames.push(Frame {
            vars: Vec::new(),
            types,
            file,
            module,
        });
    }

    /// Leaves the innermost call; its variables must already be dropped.
    pub(crate) fn pop_frame(&mut self) {
        self.frames.pop();
    }

    fn frame(&self) -> &Frame {
        self.frames.last().expect("a frame")
    }

    fn frame_mut(&mut self) -> &mut Frame {
        self.frames.last_mut().expect("a frame")
    }

    /// Whether the innermost call runs the files' code (not the prelude's).
    pub(crate) fn user(&self) -> bool {
        self.frame().file.is_some()
    }

    /// The file of the innermost call's code, if it is one of the program's.
    pub(crate) fn file(&self) -> Option<u32> {
        self.frame().file
    }

    /// The module whose names the innermost call's code uses.
    pub(crate) fn module(&self) -> usize {
        self.frame().module
    }

    /// The kind of type parameter `name` in the innermost call, if it knows it.
    pub(crate) fn type_of(&self, name: &str) -> Option<&Head> {
        self.frame().types.get(name)
    }

    /// How many variables the innermost call has: a point to [`Memory::truncate`] back to.
    pub(crate) fn mark(&self) -> usize {
        self.frame().vars.len()
    }

    /// Adds variable `name` (empty for a temporary) holding `v` to the innermost call.
    pub(crate) fn push(&mut self, name: String, v: Value) {
        self.frame_mut().vars.push((name, v));
    }

    /// Removes the innermost call's last variable.
    pub(crate) fn pop(&mut self) -> Option<(String, Value)> {
        self.frame_mut().vars.pop()
    }

    /// Removes the innermost call's variables from `mark` on, without dropping them.
    pub(crate) fn truncate(&mut self, mark: usize) {
        self.frame_mut().vars.truncate(mark);
    }

    /// The value of the innermost call's variable `i` if it is a temporary (nameless).
    pub(crate) fn temp_value(&self, i: usize) -> Option<&Value> {
        let (name, v) = &self.frame().vars[i];
        name.is_empty().then_some(v)
    }

    /// Takes the value out of the innermost call's variable `i`, leaving it empty.
    pub(crate) fn take(&mut self, i: usize) -> Value {
        std::mem::replace(&mut self.frame_mut().vars[i].1, Value::Moved)
    }

    /// The innermost variable called `name` in the innermost call.
    pub(crate) fn slot(&self, name: &str) -> Option<usize> {
        self.frame().vars.iter().rposition(|(n, _)| n == name)
    }

    /// A pointer to the innermost call's variable `slot`.
    pub(crate) fn here(&self, slot: usize) -> Pointer {
        Pointer {
            frame: self.frames.len() - 1,
            slot,
            path: Vec::new(),
        }
    }

    /// A nameless variable holding `v`, so that it can be borrowed or dropped.
    pub(crate) fn temp(&mut self, v: Value) -> Pointer {
        self.push(String::new(), v);
        self.here(self.mark() - 1)
    }

    /// The value at `ptr`.
    pub(crate) fn load(&self, ptr: &Pointer) -> Value {
        let mut v = &self.frames[ptr.frame].vars[ptr.slot].1;
        for &i in &ptr.path {
            v = match v {
                Value::Tuple(xs) | Value::Adt { fields: xs, .. } => &xs[i],
                _ => unreachable!("a checked field"),
            };
        }
        v.clone()
    }

    /// Stores `value` at `ptr`.
    pub(crate) fn store(&mut self, ptr: &Pointer, value: Value) {
        let mut v = &mut self.frames[ptr.frame].vars[ptr.slot].1;
        for &i in &ptr.path {
            v = match v {
                Value::Tuple(xs) | Value::Adt { fields: xs, .. } => &mut xs[i],
                _ => unreachable!("a checked field"),
            };
        }
        *v = value;
    }
}
