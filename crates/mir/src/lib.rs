//! Moddil's mid-level IR (MIR): places and a control-flow graph per item, its phases, and the
//! ownership checking done on it (`specs/mir.md`).
//!
//! A [`Body`] is a list of locals (`_0` is the return value, then the arguments) and basic blocks
//! of statements ending in a terminator. It is not SSA: locals are assigned any number of times.
//! Every check that can trap is an explicit [`Statement::Assert`] with a [`TrapClass`] and a source
//! range, so backends cannot forget one. The [`mir`] query builds a body from the typed HIR.
//!
//! Places are a local and a path of projections into it (a field, a variant's view of an enum,
//! or what a borrow points to). Operands copy or move; borrows are `Rvalue::Ref`. The
//! [`borrowck`] query checks ownership and borrows on the MIR; see this crate's `README.md`.

use std::fmt;

use moddil_sema::{Callee, Src, Ty};
use moddil_syntax::TextRange;

mod build;
mod convert;
mod ownership;
mod pretty;

pub use build::mir;
pub use ownership::{borrowck, diagnostics, has_errors};

/// A local: `_0` is the return value, `_1` to `_n` the arguments, then named locals and
/// temporaries.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Local(pub u32);

/// A basic block.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockId(pub u32);

impl Local {
    /// The return value.
    pub const RETURN: Local = Local(0);

    /// The local's index.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

impl BlockId {
    /// The block's index.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Debug for Local {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "_{}", self.0)
    }
}

impl fmt::Debug for BlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "bb{}", self.0)
    }
}

/// The MIR of one function or test.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Body {
    /// The function's name, or the test's name in quotes.
    pub name: String,
    /// Which source its code is in: a trap in the prelude's or derived code is reported where
    /// the file's code called it.
    pub src: Src,
    /// Whether it is `#[track_caller]`: a trap in it is reported where its caller called it.
    pub track_caller: bool,
    /// Every local; the first is the return value, the next `arg_count` the arguments.
    pub locals: Vec<LocalDecl>,
    /// How many arguments there are.
    pub arg_count: usize,
    /// The blocks; `bb0` is the entry.
    pub blocks: Vec<BasicBlock>,
    /// The text of its string literals ([`Const::Str`]).
    pub strs: Vec<String>,
    /// For each scope node (what the scopes hold at a point: see [`BasicBlock::nodes`]): the
    /// cleanup block that unwinding from there runs (`task.cancel.1`). Node 0 holds nothing.
    pub cleanups: Vec<BlockId>,
    /// How many of the last locals only cleanup code uses (not shown when printed).
    pub cleanup_locals: usize,
    /// The locals declared without a value (`let x;`, `stmt.let.uninit.1`), and whether each is
    /// `mut`: one that is not is assigned at most once.
    pub late: Vec<(Local, bool)>,
}

impl Body {
    /// An empty body of `arg_count` arguments, giving a `ret`: its return local, and nothing
    /// else yet.
    pub(crate) fn new(
        name: String,
        src: Src,
        track_caller: bool,
        ret: Ty,
        arg_count: usize,
    ) -> Body {
        Body {
            name,
            src,
            track_caller,
            locals: vec![LocalDecl {
                ty: ret,
                name: None,
            }],
            arg_count,
            blocks: Vec::new(),
            strs: Vec::new(),
            cleanups: Vec::new(),
            cleanup_locals: 0,
            late: Vec::new(),
        }
    }

    /// The cleanup block that unwinding from statement `stmt` of `block` (its terminator, if
    /// `stmt` is past the statements) runs.
    pub fn cleanup_at(&self, block: usize, stmt: usize) -> BlockId {
        let b = &self.blocks[block];
        let node = b.nodes.get(stmt).copied().unwrap_or(b.terminator_node);
        self.cleanups[node as usize]
    }
}

/// A local's declaration.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LocalDecl {
    /// Its type.
    pub ty: Ty,
    /// The source name, for a parameter or a `let` binding.
    pub name: Option<String>,
}

/// Statements run in order, then the terminator.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BasicBlock {
    /// The statements.
    pub statements: Vec<Statement>,
    /// The source each statement comes from, for diagnostics.
    pub spans: Vec<TextRange>,
    /// Where control goes next.
    pub terminator: Terminator,
    /// The source the terminator comes from.
    pub terminator_span: TextRange,
    /// For each statement: the scope node at it, which says what unwinding from it cleans up.
    pub nodes: Vec<u32>,
    /// The scope node at the terminator.
    pub terminator_node: u32,
    /// Whether it is cleanup code, run only while unwinding.
    pub cleanup: bool,
}

/// A place: a local, or a part of one reached through projections (`_1.0`, `(_2 as Circle).0`).
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Place {
    /// The local.
    pub local: Local,
    /// The projections, outermost first.
    pub projection: Vec<Projection>,
}

/// One step into a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Projection {
    /// Field `n` of a tuple, struct or (after a downcast) variant.
    Field(u32),
    /// The enum viewed as variant `n`; only valid when it holds that variant.
    Downcast(u32),
    /// What the borrow at this place points to.
    Deref,
    /// The element of an array or slice at the index in a local (an `int`, checked to be in
    /// bounds before).
    Index(Local),
    /// The part of an array or slice from the index in the first local up to, not including,
    /// the one in the second (checked before, `expr.slice.1`): a `[T]`.
    Subslice(Local, Local),
}

impl Place {
    /// The element of this array or slice at the index in `i`.
    #[must_use]
    pub fn index(&self, i: Local) -> Place {
        let mut p = self.clone();
        p.projection.push(Projection::Index(i));
        p
    }

    /// The part of this array or slice from the index in `from` up to the one in `to`.
    #[must_use]
    pub fn subslice(&self, from: Local, to: Local) -> Place {
        let mut p = self.clone();
        p.projection.push(Projection::Subslice(from, to));
        p
    }

    /// The place `local` itself.
    pub fn local(local: Local) -> Place {
        Place {
            local,
            projection: Vec::new(),
        }
    }

    /// Field `n` of this place.
    #[must_use]
    pub fn field(&self, n: u32) -> Place {
        let mut p = self.clone();
        p.projection.push(Projection::Field(n));
        p
    }

    /// What the borrow at this place points to.
    #[must_use]
    pub fn deref(&self) -> Place {
        let mut p = self.clone();
        p.projection.push(Projection::Deref);
        p
    }

    /// Whether the place is reached through a borrow.
    pub fn is_indirect(&self) -> bool {
        self.projection.contains(&Projection::Deref)
    }

    /// This place viewed as variant `n`.
    #[must_use]
    pub fn downcast(&self, n: u32) -> Place {
        let mut p = self.clone();
        p.projection.push(Projection::Downcast(n));
        p
    }
}

impl Body {
    /// The type of `place` in this body (in terms of its type parameters).
    pub fn place_ty(&self, items: &moddil_sema::ItemTree, place: &Place) -> Ty {
        let mut ty = self.locals[place.local.index()].ty.clone();
        let mut variant = 0;
        for proj in &place.projection {
            ty = match (proj, &ty) {
                (Projection::Deref, Ty::Ref(_, t) | Ty::Ptr(_, t)) => (**t).clone(),
                (Projection::Downcast(v), t) => {
                    variant = *v;
                    t.clone()
                }
                (Projection::Field(i), Ty::Tuple(ts)) => ts[*i as usize].clone(),
                (Projection::Field(i), t @ Ty::Adt(..)) => {
                    let f = items.field_types(t, variant);
                    variant = 0;
                    f.get(*i as usize).cloned().unwrap_or(Ty::Error)
                }
                _ => Ty::Error,
            };
        }
        ty
    }
}

impl From<Local> for Place {
    fn from(l: Local) -> Place {
        Place::local(l)
    }
}

/// A statement.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Statement {
    /// `place = rvalue`.
    Assign(Place, Rvalue),
    /// Traps with `trap` unless `cond` is `expected`.
    Assert {
        /// The condition checked.
        cond: Operand,
        /// The value it must have.
        expected: bool,
        /// What kind of trap it is.
        trap: TrapClass,
        /// The source of the check.
        range: TextRange,
        /// For an `assert`: the parts of its condition, whose values the trap shows.
        parts: Vec<AssertPart>,
    },
    /// Reads a place without using its value: a discarded place expression, or keeping a borrow
    /// in use (the borrow that stops `match` guards changing what is matched).
    Read(Place),
    /// The local's storage ends: it leaves scope, so nothing may borrow it any more.
    StorageDead(Local),
    /// Reports a value to the test harness: an expression followed by a `// =>` expectation.
    Observe {
        /// The expression observed.
        range: TextRange,
        /// Its value.
        value: Operand,
    },
}

/// A part of an `assert`'s condition (a local, a call, an operator): where it is, the place its
/// value is in, and a `bool` local that says whether it was evaluated (`&&`, `||` and
/// comparison chains may skip parts). The place is a local of its own for a value computed, or
/// where the value lives for one borrowed where it is (a method's receiver), which a failure
/// shows only if it still holds a value there.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AssertPart {
    /// The part's source.
    pub range: TextRange,
    /// Its value, if it was evaluated.
    pub value: Place,
    /// Whether it was evaluated.
    pub evaluated: Local,
}

/// A value computed from operands.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Rvalue {
    /// The operand itself.
    Use(Operand),
    /// `-a` (wrapping; an `Assert` checks first) or `!a`.
    Unary(UnOp, Operand),
    /// `a op b`. Arithmetic wraps here; an `Assert` before it rules out overflow.
    Binary(BinOp, Operand, Operand),
    /// Whether `a op b` overflows (`+`, `-`, `*`, `/`; and a shift whose amount is out of range,
    /// or a left shift that loses bits).
    Overflows(BinOp, Operand, Operand),
    /// Whether `-a` overflows.
    NegOverflows(Operand),
    /// A tuple, struct or variant built from its fields' values, in field order.
    Aggregate(AggregateKind, Vec<Operand>),
    /// Which variant an enum holds, as an `int` (the variant's index).
    Discriminant(Place),
    /// Whether the error at a place (of an error set) is a value of struct or enum `n`.
    ErrorIs(Place, u32),
    /// A borrow of a place: `&mut` if the flag is set.
    Ref(bool, Place),
    /// A borrow of a value of the given type (in terms of the body's type parameters) as a
    /// `dyn` borrow of a trait it implements: `&clock` as `&dyn Time`. If the borrow is already
    /// a `dyn` borrow, of a subtrait, it stays as it is.
    Unsize(Operand, Ty),
    /// A number as a wider number type, which holds every value of its own (`num.widen.1`):
    /// the operand's value, as the given type.
    Widen(Operand, Ty),
    /// `[x; n]`: an array of `n` copies of the operand, `n` a length type (in terms of the body's
    /// type parameters).
    Repeat(Operand, Ty),
    /// The value of a length type (a const parameter's argument), as an `int`.
    Length(Ty),
    /// Whether the index is in bounds for the array or slice at the place.
    InBounds(Operand, Place),
    /// The length of the array or slice at the place, as an `int`.
    Len(Place),
    /// Whether `from..to` is a part of the array or slice at the place: `0 <= from <= to <= len`.
    SpanOk(Operand, Operand, Place),
    /// A borrow of the value of const item `n` (by its function's index): static data, which
    /// borrows nothing (`borrow.static.1`).
    StaticRef(u32),
    /// A borrow of the value of a `const { }` block computed when compiling, by its item (a
    /// closure item of the function or test it is in, and the block's expression): static data
    /// (`comptime.block.2`).
    StaticBlock(moddil_sema::Item),
    /// The address of function `n` (by its index) as a C function pointer (`type.fnptr.1`):
    /// a C function's own, or that of the C-ABI wrapper of a Moddil one.
    FnPtr(u32),
}

/// What an [`Rvalue::Aggregate`] builds.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AggregateKind {
    /// A tuple.
    Tuple,
    /// An array.
    Array,
    /// A struct (variant 0) or an enum's variant.
    Adt {
        /// The struct or enum, by its index in the file's item tree.
        adt: u32,
        /// The variant.
        variant: u32,
        /// How it is written (`Point`, `Shape::Circle`), for printing.
        path: String,
    },
    /// A closure, of this type, from its captures in order.
    Closure(Ty),
}

/// A place's value or a constant.
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum Operand {
    /// A copy of the value of a place, which stays usable.
    Copy(Place),
    /// The value of a place, moved out: the place cannot be used again until it is assigned.
    Move(Place),
    /// A constant.
    Const(Const),
}

/// A constant.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Const {
    /// An `int`.
    Int(i64),
    /// A sized integer or a float (`num.types.1`): its type, and its bits (two's complement for
    /// a signed integer, IEEE 754 for a float), in the low bits.
    Num(moddil_sema::Num, u128),
    /// A `bool`.
    Bool(bool),
    /// `()`.
    Unit,
    /// A `char`.
    Char(char),
    /// A `&str` literal: its text, by its index in [`Body::strs`].
    Str(u32),
}

/// Prefix operators.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnOp {
    /// Negation.
    Neg,
    /// Logical not on `bool`, bitwise not on `int`.
    Not,
}

/// Binary operators.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BinOp {
    /// `+`.
    Add,
    /// `-`.
    Sub,
    /// `*`.
    Mul,
    /// `/`, rounding towards zero.
    Div,
    /// `%`, with the sign of the left operand.
    Rem,
    /// `&`.
    BitAnd,
    /// `|`.
    BitOr,
    /// `^`.
    BitXor,
    /// `<<`.
    Shl,
    /// `>>`, arithmetic.
    Shr,
    /// `==`.
    Eq,
    /// `!=`.
    Ne,
    /// `<`.
    Lt,
    /// `<=`.
    Le,
    /// `>`.
    Gt,
    /// `>=`.
    Ge,
}

/// Where control goes at the end of a block.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Terminator {
    /// To `target`.
    Goto(BlockId),
    /// To `then` if `cond` is true, else to `else_`.
    SwitchBool {
        /// The condition.
        cond: Operand,
        /// The block taken when it is true.
        then: BlockId,
        /// The block taken when it is false.
        else_: BlockId,
    },
    /// Calls a function; its result goes to `dest`, then control to `target`.
    Call {
        /// The function, with its type arguments (which may name the body's own type
        /// parameters). A trait's method is found from the type arguments when the call runs.
        callee: Callee,
        /// How the function is named, for printing: `add`, `PartialEq::eq`.
        name: String,
        /// The arguments.
        args: Vec<Operand>,
        /// Where the result goes.
        dest: Place,
        /// Where control goes after the call returns.
        target: BlockId,
        /// For a function that can fail: where its error goes, and where control goes then.
        on_error: Option<(Place, BlockId)>,
        /// The call's source.
        range: TextRange,
    },
    /// Leaves the function with an error: its caller continues where the call's error goes.
    Fail(Operand),
    /// Traps: for `?!`, with its reason, when the error it says cannot happen does.
    Trap(TrapClass, String),
    /// Drops the value at `place`, if it holds one (it may have been moved out of): runs its
    /// destructor if its type has one, then drops its fields, last first. Then control goes to
    /// `target`.
    Drop {
        /// The place dropped.
        place: Place,
        /// Where control goes next.
        target: BlockId,
        /// Drop only the fields, not the value itself: for `self` at the end of its own
        /// destructor.
        glue_only: bool,
    },
    /// Returns `_0`.
    Return,
    /// The end of a cleanup chain: the frame's unwinding goes on in its caller.
    Resume,
    /// Control never reaches here.
    Unreachable,
}

impl Terminator {
    /// The blocks this terminator can go to.
    pub fn successors(&self) -> Vec<BlockId> {
        match self {
            Terminator::Call {
                target,
                on_error: Some((_, e)),
                ..
            } => vec![*target, *e],
            Terminator::Goto(t)
            | Terminator::Call { target: t, .. }
            | Terminator::Drop { target: t, .. } => vec![*t],
            Terminator::SwitchBool { then, else_, .. } => vec![*then, *else_],
            Terminator::Return
            | Terminator::Resume
            | Terminator::Unreachable
            | Terminator::Fail(_)
            | Terminator::Trap(..) => vec![],
        }
    }
}

/// Why a program traps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TrapClass {
    /// Integer overflow: arithmetic, negation or a shift.
    Overflow,
    /// Integer division or remainder by zero.
    DivisionByZero,
    /// Calls nested too deeply.
    StackOverflow,
    /// A failed `assert`.
    Assertion,
    /// An index out of bounds.
    IndexOutOfBounds,
    /// More elements pushed into a `Buf` than its capacity.
    CapacityExceeded,
    /// Compile-time evaluation that ran out of its steps.
    OutOfFuel,
    /// Every task waits, and nothing can wake one (`task.sched.1`).
    Deadlock,
    /// A `requires` that does not hold: the caller's bug, reported at the call.
    Precondition,
    /// An `ensures` that does not hold: the function's bug.
    Postcondition,
    /// A struct's `invariant` that does not hold, after a literal made it or a method taking
    /// `&mut self` changed it (`contract.invariant.1`).
    Invariant,
    /// An error that `?!` said cannot happen.
    Unexpected,
    /// `unreachable("reason")` reached: a broken invariant (`expr.unreachable.1`).
    Unreachable,
    /// What waits for a runtime: a `select` on an operation other than the prelude's
    /// (`task.select.3`).
    Unimplemented,
}

impl TrapClass {
    /// How the trap is named in messages and conformance tests: `overflow`.
    pub fn name(self) -> &'static str {
        match self {
            TrapClass::Overflow => "overflow",
            TrapClass::DivisionByZero => "division by zero",
            TrapClass::StackOverflow => "stack overflow",
            TrapClass::Assertion => "assertion failed",
            TrapClass::IndexOutOfBounds => "index out of bounds",
            TrapClass::CapacityExceeded => "capacity exceeded",
            TrapClass::OutOfFuel => "out of fuel",
            TrapClass::Deadlock => "deadlock",
            TrapClass::Precondition => "precondition broken",
            TrapClass::Postcondition => "postcondition broken",
            TrapClass::Invariant => "invariant broken",
            TrapClass::Unexpected => "unexpected error",
            TrapClass::Unreachable => "unreachable",
            TrapClass::Unimplemented => "not implemented yet",
        }
    }
}

impl fmt::Display for TrapClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// This crate's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests;

pub use convert::{convert, exp_text, fixed_text, num_code, num_text};
pub use moddil_rt::num::{float_f32, float_f64};

/// The runtime's operation a built-in function of the prelude stands for, as the runtimes
/// written in Rust name it (the MIR interpreter's tasks and `moddil-rt`'s): the `core` mod's `os`
/// module (`os.1`), and the tasks, scopes and channels (`task.sched.1`).
pub fn runtime_op(items: &moddil_sema::ItemTree, f: u32) -> Option<&'static str> {
    let sig = &items.fns[f as usize];
    if sig.src != Src::Prelude || !sig.intrinsic {
        return None;
    }
    if items.modules[sig.module as usize].path == ["os"] {
        return Some(match sig.name.as_str() {
            "arg_count" => "os_arg_count",
            "arg_len" => "os_arg_len",
            "arg_copy" => "os_arg_copy",
            "write_stderr" => "os_write_stderr",
            "task_tree_len" => "os_task_tree_len",
            "task_tree_copy" => "os_task_tree_copy",
            "random" => "os_random",
            "wall_clock" => "os_wall_clock",
            "listen" => "os_listen",
            "incoming" => "os_incoming",
            "connect" => "os_connect",
            "arrivals" => "os_arrivals",
            "take" => "os_take",
            "send" => "os_send",
            "close" => "os_close",
            "signals" => "os_signals",
            _ => return None,
        });
    }
    let owner = match sig.container {
        moddil_sema::Container::Impl(k) => match &items.impls[k as usize].self_ty {
            Ty::Adt(a, ..) => items.adts[*a as usize].name.as_str(),
            _ => return None,
        },
        _ => "",
    };
    Some(match (owner, sig.name.as_str()) {
        ("", "yield_now") => "yield_now",
        ("", "__task_id") => "task_id",
        ("", "__park") => "park",
        ("", "__install_runtime") => "install_runtime",
        ("", "__parallel") => "parallel",
        ("", "__hash_seed") => "hash_seed",
        ("", "__executor_of_task") => "executor_of_task",
        ("", "__executor_new") => "executor_new",
        ("", "__unpark") => "unpark",
        ("", "stop_requested") => "stop_requested",
        ("", "__scope_open") => "__scope_open",
        ("", "__scope_run") => "__scope_run",
        ("", "channel") => "channel",
        ("", "__timer") => "timer",
        ("", "__select") => "__select",
        ("Scope", "spawn") => "spawn",
        ("Scope", "stop") => "stop",
        ("Task", "join") => "join",
        ("Sender", "send") => "send",
        ("Sender", "clone") => "clone",
        ("Receiver", "recv") => "recv",
        _ => return None,
    })
}

/// The constant of number type `t` (`int` or a [`Ty::Num`]) for the exact integer `v`, which the
/// checker has made sure fits; a float gets the nearest value.
pub fn int_const(t: &Ty, v: moddil_syntax::ast::IntValue) -> Const {
    use moddil_sema::Num;
    match t {
        Ty::Num(Num::F32) => Const::Num(Num::F32, u128::from((v.as_f64() as f32).to_bits())),
        Ty::Num(Num::F64) => Const::Num(Num::F64, u128::from(v.as_f64().to_bits())),
        Ty::Num(n) => {
            let mask = if n.bits() == 128 {
                u128::MAX
            } else {
                (1u128 << n.bits()) - 1
            };
            Const::Num(*n, v.bits() & mask)
        }
        _ => Const::Int(v.bits() as i64),
    }
}

/// The constant of float type `t` for the literal `text`, rounded to the nearest value.
pub fn float_const(t: &Ty, text: &str) -> Const {
    use moddil_sema::Num;
    match t {
        Ty::Num(Num::F32) => {
            let v: f32 = text.parse().unwrap_or(f32::NAN);
            Const::Num(Num::F32, u128::from(v.to_bits()))
        }
        _ => {
            let v: f64 = text.parse().unwrap_or(f64::NAN);
            Const::Num(Num::F64, u128::from(v.to_bits()))
        }
    }
}
