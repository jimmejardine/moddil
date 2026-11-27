//! The frame stack: every frame's locals and where it is in its body. All memory lives here, so a
//! borrow is a [`Pointer`] into a frame, and only this module reads or writes through one.

use moddil_mir::{Body, Const, Local, Operand, Place, Projection, Statement, Terminator};
use moddil_sema::{ItemTree, Solver, Src, Ty};
use moddil_syntax::TextRange;

use crate::cmem::{CMEM, CMem};
use crate::value::{Pointer, Value};

/// Where a call's result goes.
pub(crate) enum Ret {
    /// Out of the interpreter: the function or test it was asked to run.
    Top,
    /// To a place of the caller, which then continues at a block; and, for a function that can
    /// fail, where its error goes and where the caller continues then.
    To(Pointer, usize, Option<(Pointer, usize)>),
    /// Nowhere: the call is a destructor, run while the caller drops a value.
    Drop,
    /// To the scheduler: the call is a task's closure (`task.spawn.1`).
    Task,
    /// Back to C: the call is a callback C made while a C call ran (`ffi.native.2`).
    Callback,
    /// To a buffer made or regrown where an allocator is installed (`alloc.with.1`): the
    /// buffer's place, where the caller continues, and the allocator and size the allocation
    /// it is given came from.
    Buf(Pointer, usize, Box<Value>, i64),
}

/// A destructor call waiting to run: the function, its type arguments, and the value.
pub(crate) type DropCall = (u32, Vec<Ty>, Value);

/// One call being run.
pub(crate) struct Frame<'a> {
    body: &'a Body,
    /// The type arguments it runs with (none for a function that is not generic).
    type_args: Vec<Ty>,
    locals: Vec<Value>,
    block: usize,
    stmt: usize,
    ret: Ret,
    /// While a `Drop` terminator runs: the destructors still to call, and where to go after.
    drops: Vec<DropCall>,
    after_drops: Option<usize>,
    /// For a closure called through a borrow: where the closure is, to which its captures go
    /// back when the call returns.
    pub(crate) write_back: Option<Pointer>,
}

impl<'a> Frame<'a> {
    pub(crate) fn new(body: &'a Body, args: Vec<Value>, type_args: Vec<Ty>, ret: Ret) -> Self {
        assert_eq!(args.len(), body.arg_count, "argument count");
        let mut locals = vec![Value::Empty; body.locals.len()];
        for (slot, a) in locals[1..].iter_mut().zip(args) {
            *slot = a;
        }
        Frame {
            body,
            type_args,
            locals,
            block: 0,
            stmt: 0,
            ret,
            drops: Vec::new(),
            after_drops: None,
            write_back: None,
        }
    }

    /// Whether a trap here is reported where its caller called it: in a `#[track_caller]`
    /// function, or in the prelude's or derived code.
    fn tracked(&self) -> bool {
        self.body.track_caller || !self.body.src.is_file()
    }

    fn terminator_span(&self) -> TextRange {
        self.body.blocks[self.block].terminator_span
    }
}

/// What the top frame's pending destructors ask for next.
pub(crate) enum PendingDrop {
    /// No `Drop` terminator is running.
    None,
    /// All of its destructors have run; the frame has moved on to the block after it.
    Finished,
    /// Call this destructor next.
    Call(DropCall),
}

/// The stacks of frames, one per task (`task.sched.1`). The running frame is the top one of the
/// running task's.
pub(crate) struct Stack<'a> {
    /// Each task's frames, by the task's index; a task that has ended has none.
    tasks: Vec<Vec<Frame<'a>>>,
    /// The running task.
    current: usize,
    /// The program's items, for C's layouts.
    items: &'a ItemTree,
    /// What the run has given C, and the places in C's memory it reads and writes
    /// (`ffi.native.2`).
    cmem: CMem,
    /// The values of the const items borrowed so far: static data, which a [`Pointer`] with
    /// [`STATIC`] for its frame points into.
    statics: std::collections::HashMap<u32, Value>,
    /// The values `core::ffi::into_raw` keeps alive behind a raw pointer (`ffi.core.1`), which a
    /// [`Pointer`] with [`HEAP`] for its frame points into, until `from_raw` takes them back.
    heap: std::collections::HashMap<u32, Value>,
    next_heap: u32,
    /// Where each text is, so a position's text is told from other text (`text.pos.1`).
    pub(crate) texts: crate::texts::Texts<Pointer>,
    /// Each `const { }` block's static number, by its item.
    blocks: std::collections::HashMap<moddil_sema::Item, u32>,
}

/// The frame of a pointer to static data: the const item is its local.
pub(crate) const STATIC: usize = usize::MAX;

/// The frame of a raw pointer to a value on the heap: its slot is its local.
pub(crate) const HEAP: usize = usize::MAX - 1;

impl<'a> Stack<'a> {
    pub(crate) fn new(first: Frame<'a>, items: &'a ItemTree) -> Self {
        Stack {
            tasks: vec![vec![first]],
            current: 0,
            items,
            cmem: CMem::default(),
            statics: std::collections::HashMap::new(),
            heap: std::collections::HashMap::new(),
            next_heap: 0,
            texts: crate::texts::Texts::default(),
            blocks: std::collections::HashMap::new(),
        }
    }

    /// Keeps `v` on the heap: a raw pointer to it (`ffi.core.1`).
    pub(crate) fn alloc(&mut self, v: Value) -> Value {
        let slot = self.next_heap;
        self.next_heap += 1;
        self.heap.insert(slot, v);
        Value::Ref(
            Pointer {
                task: 0,
                frame: HEAP,
                local: slot,
                path: Vec::new(),
                span: None,
            },
            true,
        )
    }

    /// The value at the heap pointer `p`, taken off the heap.
    pub(crate) fn free(&mut self, p: &Pointer) -> Value {
        assert!(
            p.frame == HEAP && p.path.is_empty(),
            "`from_raw` of a pointer `into_raw` did not give"
        );
        self.heap
            .remove(&p.local)
            .expect("`from_raw` of a pointer already given back")
    }

    /// The running task's frames.
    fn frames(&self) -> &Vec<Frame<'a>> {
        &self.tasks[self.current]
    }

    fn frames_mut(&mut self) -> &mut Vec<Frame<'a>> {
        &mut self.tasks[self.current]
    }

    /// Starts a task whose first frame is `first`, and gives its index.
    pub(crate) fn add_task(&mut self, first: Frame<'a>) -> usize {
        self.tasks.push(vec![first]);
        self.tasks.len() - 1
    }

    /// The running task.
    pub(crate) fn current(&self) -> usize {
        self.current
    }

    /// Runs task `t` from now on.
    pub(crate) fn switch(&mut self, t: usize) {
        self.current = t;
    }

    /// A borrow of const item `f`'s value, if it has been computed.
    pub(crate) fn static_ref(&self, f: u32) -> Option<Value> {
        self.statics.contains_key(&f).then(|| {
            Value::Ref(
                Pointer {
                    task: 0,
                    frame: STATIC,
                    local: f,
                    path: Vec::new(),
                    span: None,
                },
                false,
            )
        })
    }

    /// The static number of a `const { }` block's value (`comptime.block.2`), by its item:
    /// numbered after the functions, so it never meets a const item's.
    pub(crate) fn block_static(&mut self, item: moddil_sema::Item) -> u32 {
        let next = u32::MAX / 2 + self.blocks.len() as u32;
        *self.blocks.entry(item).or_insert(next)
    }

    /// Keeps const item `f`'s value as static data.
    pub(crate) fn set_static(&mut self, f: u32, v: Value) {
        self.statics.insert(f, v);
    }

    /// What the run has given C (`ffi.native.2`).
    pub(crate) fn cmem(&mut self) -> &mut CMem {
        &mut self.cmem
    }

    /// The place a pointer C gave, `addr`, points to: `place`'s projections up to `k` are the
    /// pointer, whose type says what is there.
    fn c_place(&self, place: &Place, k: usize, addr: u64) -> Pointer {
        let prefix = Place {
            local: place.local,
            projection: place.projection[..k].to_vec(),
        };
        let ty = self.body().place_ty(self.items, &prefix);
        let ty = Solver::new(self.items, &[]).deep(&ty.subst(self.type_args()));
        let (Ty::Ptr(_, inner) | Ty::Ref(_, inner)) = ty else {
            unreachable!("dereferencing a `{ty}`")
        };
        self.cmem.place(addr, (*inner).clone())
    }

    pub(crate) fn depth(&self) -> usize {
        self.frames().len()
    }

    pub(crate) fn push(&mut self, frame: Frame<'a>) {
        self.frames_mut().push(frame);
    }

    /// Ends the top frame's call, giving where its result goes.
    pub(crate) fn pop(&mut self) -> Ret {
        let frame = self.frames_mut().pop().expect("a frame");
        // A closure's captures, which it may have changed, go back to it.
        if let Some(ptr) = frame.write_back {
            let captures = match &frame.locals[1] {
                Value::Unit => Some(Vec::new()),
                Value::Tuple(xs) => Some(xs.clone()),
                _ => None,
            };
            if let Some(xs) = captures
                && !xs.iter().any(|x| matches!(x, Value::Empty))
                && matches!(self.load(&ptr), Value::Closure(_))
            {
                self.store(&ptr, Value::Closure(xs));
            }
        }
        frame.ret
    }

    fn top_index(&self) -> usize {
        self.frames().len() - 1
    }

    fn top(&self) -> &Frame<'a> {
        self.frames().last().expect("a frame")
    }

    fn top_mut(&mut self) -> &mut Frame<'a> {
        self.frames_mut().last_mut().expect("a frame")
    }

    /// The body the top frame runs.
    pub(crate) fn body(&self) -> &'a Body {
        self.top().body
    }

    /// The type arguments the top frame runs with.
    pub(crate) fn type_args(&self) -> &[Ty] {
        &self.top().type_args
    }

    /// Whether the top frame is the call the interpreter was asked to run.
    pub(crate) fn at_root(&self) -> bool {
        matches!(self.top().ret, Ret::Top)
    }

    /// Whether the top frame is a callback C made (`ffi.native.2`).
    pub(crate) fn at_callback(&self) -> bool {
        matches!(self.top().ret, Ret::Callback)
    }

    /// Ends the running task's calls above the first `depth`, as a trap in a callback does.
    pub(crate) fn unwind_to(&mut self, depth: usize) {
        self.frames_mut().truncate(depth);
    }

    /// A local of the top frame.
    pub(crate) fn local(&self, l: Local) -> &Value {
        &self.top().locals[l.index()]
    }

    /// The span of the top frame's current terminator.
    pub(crate) fn terminator_span(&self) -> TextRange {
        self.top().terminator_span()
    }

    /// The top frame's next statement, moving past it; `None` when only its block's terminator
    /// is left.
    pub(crate) fn next_statement(&mut self) -> Option<&'a Statement> {
        let frame = self.top_mut();
        let body: &'a Body = frame.body;
        let s = body.blocks[frame.block].statements.get(frame.stmt)?;
        frame.stmt += 1;
        Some(s)
    }

    /// The top frame's current terminator.
    pub(crate) fn terminator(&self) -> &'a Terminator {
        let frame = self.top();
        let body: &'a Body = frame.body;
        &body.blocks[frame.block].terminator
    }

    /// Whether the running task is running a destructor, where a trap always ends the run.
    pub(crate) fn in_drop(&self) -> bool {
        self.frames().iter().any(|f| matches!(f.ret, Ret::Drop))
    }

    /// Starts unwinding the top frame from where it is (`task.cancel.1`): at its terminator, or
    /// at the statement it has just run; it goes on in the cleanup block for that point.
    pub(crate) fn unwind_top(&mut self, at_terminator: bool) {
        let frame = self.top_mut();
        let stmt = if at_terminator {
            frame.body.blocks[frame.block].statements.len()
        } else {
            frame.stmt.saturating_sub(1)
        };
        frame.block = frame.body.cleanup_at(frame.block, stmt).index();
        frame.stmt = 0;
        frame.drops.clear();
        frame.after_drops = None;
    }

    /// Continues the top frame at the start of `block`.
    pub(crate) fn goto(&mut self, block: usize) {
        let frame = self.top_mut();
        frame.block = block;
        frame.stmt = 0;
    }

    /// Starts running destructors for the top frame's `Drop` terminator, then continues at
    /// `after`.
    pub(crate) fn start_drops(&mut self, calls: Vec<DropCall>, after: usize) {
        let frame = self.top_mut();
        frame.drops = calls;
        frame.after_drops = Some(after);
    }

    /// Runs `calls`, then the top frame's cleanup, where [`Stack::unwind_top`] has put it.
    pub(crate) fn drops_before_cleanup(&mut self, calls: Vec<DropCall>) {
        if calls.is_empty() {
            return;
        }
        let frame = self.top_mut();
        frame.drops = calls;
        frame.after_drops = Some(frame.block);
    }

    /// Runs `calls` before the rest of the running `Drop` terminator's destructors.
    pub(crate) fn drops_first(&mut self, calls: Vec<DropCall>) {
        let frame = self.top_mut();
        frame.drops.splice(0..0, calls);
    }

    /// The next destructor of a running `Drop` terminator, if one is running.
    pub(crate) fn pending_drop(&mut self) -> PendingDrop {
        let frame = self.top_mut();
        let Some(after) = frame.after_drops else {
            return PendingDrop::None;
        };
        if frame.drops.is_empty() {
            frame.after_drops = None;
            self.goto(after);
            PendingDrop::Finished
        } else {
            PendingDrop::Call(frame.drops.remove(0))
        }
    }

    /// Where a trap is reported: the file and range of the top frame's code, or, when that code
    /// is tracked (`#[track_caller]`, the prelude's or derived code), the call of the nearest
    /// untracked caller. The flag says whether the trap is in the top frame's own code, where
    /// the details of a failed `assert` belong.
    /// If `at_call`, it is the call of the top frame's nearest untracked caller whatever the top
    /// frame is (a broken precondition is the caller's).
    pub(crate) fn trap_site_from(
        &self,
        range: Option<TextRange>,
        at_call: bool,
    ) -> (u32, TextRange, bool) {
        let top = self.top();
        if at_call && self.frames().len() > 1 {
            let caller = self
                .frames()
                .iter()
                .rev()
                .skip(1)
                .find(|f| !f.tracked())
                .unwrap_or(top);
            return (
                caller.body.src.file().unwrap_or(0),
                caller.terminator_span(),
                false,
            );
        }
        if !top.tracked()
            && let Src::File(file) = top.body.src
        {
            return (file, range.unwrap_or_else(|| top.terminator_span()), true);
        }
        let caller = self
            .frames()
            .iter()
            .rev()
            .find(|f| !f.tracked())
            .unwrap_or(top);
        (
            caller.body.src.file().unwrap_or(0),
            caller.terminator_span(),
            false,
        )
    }

    /// Where `place` of the top frame is, following borrows.
    pub(crate) fn resolve(&self, place: &Place) -> Pointer {
        let mut ptr = Pointer {
            task: self.current,
            frame: self.top_index(),
            local: place.local.0,
            path: Vec::new(),
            span: None,
        };
        for (k, p) in place.projection.iter().enumerate() {
            match p {
                // C's memory (`ffi.native.2`).
                Projection::Field(i) if ptr.frame == CMEM => {
                    ptr = self.cmem.field_of(self.items, &ptr, *i);
                }
                Projection::Index(l) if ptr.frame == CMEM => {
                    let i = self.local(*l).as_int();
                    ptr = self.cmem.element_of(self.items, &ptr, i);
                }
                Projection::Deref if ptr.frame == CMEM => {
                    match self.cmem.load_at(self.items, &ptr) {
                        Value::Foreign(addr) => ptr = self.c_place(place, k, addr),
                        Value::Ref(target, _) => ptr = target,
                        other => unreachable!("dereferencing {other:?}"),
                    }
                }
                Projection::Field(i) => ptr.path.push(*i),
                Projection::Downcast(_) => {}
                Projection::Deref => match self.load(&ptr) {
                    Value::Ref(target, _) | Value::Dyn(target, _, _) => ptr = target,
                    Value::Foreign(addr) => ptr = self.c_place(place, k, addr),
                    other => unreachable!("dereferencing {other:?}"),
                },
                // An index into part of an array is one into the array.
                Projection::Index(l) => match self.local(*l) {
                    Value::Int(i) => {
                        let start = ptr.span.take().map_or(0, |(s, _)| s);
                        ptr.path.push(start + *i as u32);
                    }
                    other => unreachable!("an index: {other:?}"),
                },
                Projection::Subslice(a, b) => {
                    let (a, b) = (
                        self.local(*a).as_int() as u32,
                        self.local(*b).as_int() as u32,
                    );
                    let start = ptr.span.map_or(0, |(s, _)| s);
                    ptr.span = Some((start + a, b - a));
                }
            }
        }
        ptr
    }

    /// The value `ptr` points to, in place: not C's memory, and its span (if any) not applied.
    fn at(&self, ptr: &Pointer) -> &Value {
        let mut v = if ptr.frame == STATIC {
            &self.statics[&ptr.local]
        } else if ptr.frame == HEAP {
            &self.heap[&ptr.local]
        } else {
            &self.tasks[ptr.task][ptr.frame].locals[ptr.local as usize]
        };
        for &i in &ptr.path {
            v = match v {
                Value::Tuple(xs)
                | Value::Array(xs)
                | Value::Buf(_, xs)
                | Value::Adt { fields: xs, .. } => &xs[i as usize],
                Value::Owned(o) => &o.0,
                other => unreachable!("a field of {other:?}"),
            };
        }
        v
    }

    /// How many elements the array, `Buf` or slice at `place` has, without copying them.
    pub(crate) fn len_of(&self, place: &Place) -> usize {
        let ptr = self.resolve(place);
        if let Some((_, n)) = ptr.span {
            return n as usize;
        }
        let len = |v: &Value| match v {
            Value::Array(xs) | Value::Buf(_, xs) => xs.len(),
            other => unreachable!("the length of {other:?}"),
        };
        if ptr.frame == CMEM {
            return len(&self.load(&ptr));
        }
        len(self.at(&ptr))
    }

    /// The struct or enum and the variant of the value at `place`, if it is one of those,
    /// without copying it.
    pub(crate) fn tag_of(&self, place: &Place) -> Option<(u32, u32)> {
        let ptr = self.resolve(place);
        let tag = |v: &Value| match v {
            Value::Adt { adt, variant, .. } => Some((*adt, *variant)),
            _ => None,
        };
        if ptr.frame == CMEM {
            return tag(&self.load(&ptr));
        }
        tag(self.at(&ptr))
    }

    pub(crate) fn load(&self, ptr: &Pointer) -> Value {
        if ptr.frame == CMEM {
            return self.cmem.load_at(self.items, ptr);
        }
        let v = self.at(ptr);
        // Part of an array (`expr.slice.1`).
        if let Some((s, n)) = ptr.span {
            return match v {
                Value::Array(xs) | Value::Buf(_, xs) => {
                    Value::Array(xs[s as usize..(s + n) as usize].to_vec())
                }
                other => unreachable!("part of {other:?}"),
            };
        }
        v.clone()
    }

    pub(crate) fn store(&mut self, ptr: &Pointer, value: Value) {
        if ptr.frame == CMEM {
            self.cmem.store_at(self.items, ptr, &value);
            return;
        }
        let mut v = if ptr.frame == HEAP {
            self.heap.get_mut(&ptr.local).expect("a live heap value")
        } else {
            &mut self.tasks[ptr.task][ptr.frame].locals[ptr.local as usize]
        };
        for &i in &ptr.path {
            v = match v {
                Value::Tuple(xs)
                | Value::Array(xs)
                | Value::Buf(_, xs)
                | Value::Adt { fields: xs, .. } => &mut xs[i as usize],
                Value::Owned(o) => &mut o.0,
                other => unreachable!("a field of {other:?}"),
            };
        }
        // Part of an array: its elements are replaced (`expr.slice.1`).
        if let Some((s, n)) = ptr.span {
            match (v, value) {
                (Value::Array(xs) | Value::Buf(_, xs), Value::Array(ys)) => {
                    let (s, n) = (s as usize, n as usize);
                    xs.splice(s..s + n, ys);
                }
                (other, _) => unreachable!("part of {other:?}"),
            }
            return;
        }
        *v = value;
    }

    pub(crate) fn read(&self, place: &Place) -> Value {
        self.load(&self.resolve(place))
    }

    pub(crate) fn operand(&self, o: &Operand) -> Value {
        match o {
            Operand::Copy(p) | Operand::Move(p) => self.read(p),
            Operand::Const(Const::Int(v)) => Value::Int(*v),
            Operand::Const(Const::Num(n, bits)) => Value::Num(*n, *bits),
            Operand::Const(Const::Bool(v)) => Value::Bool(*v),
            Operand::Const(Const::Unit) => Value::Unit,
            Operand::Const(Const::Char(c)) => Value::Char(*c),
            Operand::Const(Const::Str(i)) => {
                Value::Str(self.body().strs[*i as usize].as_str().into())
            }
        }
    }

    /// Like [`Stack::operand`], but a moved value leaves its place empty.
    pub(crate) fn take(&mut self, o: &Operand) -> Value {
        let v = self.operand(o);
        if let Operand::Move(p) = o
            && !p.is_indirect()
        {
            let ptr = self.resolve(p);
            self.store(&ptr, Value::Empty);
        }
        v
    }

    /// What a borrow points to, or the value itself if it is not a borrow.
    pub(crate) fn deref(&self, v: &Value) -> Value {
        match v {
            Value::Ref(ptr, _) => self.load(ptr),
            v => v.clone(),
        }
    }

    /// `v` with every borrow replaced by what it points to, for reporting outside the program.
    pub(crate) fn shown(&self, v: Value) -> Value {
        match v {
            Value::Ref(ptr, m) | Value::Dyn(ptr, m, _) => {
                Value::Borrowed(Box::new(self.shown(self.load(&ptr))), m)
            }
            Value::Tuple(xs) => Value::Tuple(xs.into_iter().map(|x| self.shown(x)).collect()),
            Value::Adt {
                adt,
                variant,
                fields,
            } => Value::Adt {
                adt,
                variant,
                fields: fields.into_iter().map(|x| self.shown(x)).collect(),
            },
            v => v,
        }
    }
}
