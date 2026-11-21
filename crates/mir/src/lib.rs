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
    /// Every local; the first is the return value, the next `arg_count` the arguments.
    pub locals: Vec<LocalDecl>,
    /// How many arguments there are.
    pub arg_count: usize,
    /// The blocks; `bb0` is the entry.
    pub blocks: Vec<BasicBlock>,
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
}

impl Place {
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
                (Projection::Deref, Ty::Ref(_, t)) => (**t).clone(),
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

/// A part of an `assert`'s condition (a local, a call, an operator): where it is, the local its
/// value is stored in, and a `bool` local that says whether it was evaluated (`&&`, `||` and
/// comparison chains may skip parts).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AssertPart {
    /// The part's source.
    pub range: TextRange,
    /// Its value, if it was evaluated.
    pub value: Local,
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
}

/// What an [`Rvalue::Aggregate`] builds.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AggregateKind {
    /// A tuple.
    Tuple,
    /// A struct (variant 0) or an enum's variant.
    Adt {
        /// The struct or enum, by its index in the file's item tree.
        adt: u32,
        /// The variant.
        variant: u32,
        /// How it is written (`Point`, `Shape::Circle`), for printing.
        path: String,
    },
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
    /// A `bool`.
    Bool(bool),
    /// `()`.
    Unit,
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
    /// An error that `?!` said cannot happen.
    Unexpected,
}

impl TrapClass {
    /// How the trap is named in messages and conformance tests: `overflow`.
    pub fn name(self) -> &'static str {
        match self {
            TrapClass::Overflow => "overflow",
            TrapClass::DivisionByZero => "division by zero",
            TrapClass::StackOverflow => "stack overflow",
            TrapClass::Assertion => "assertion failed",
            TrapClass::Unexpected => "unexpected error",
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
