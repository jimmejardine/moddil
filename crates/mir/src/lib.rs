//! Moddil's mid-level IR (MIR): places and a control-flow graph per item, its phases, and the
//! ownership checking done on it (`specs/mir.md`).
//!
//! A [`Body`] is a list of locals (`_0` is the return value, then the arguments) and basic blocks
//! of statements ending in a terminator. It is not SSA: locals are assigned any number of times.
//! Every check that can trap is an explicit [`Statement::Assert`] with a [`TrapClass`] and a source
//! range, so backends cannot forget one. The [`mir`] query builds a body from the typed HIR.
//!
//! In slice S0 there are no projections, drops or ownership checks yet; see this crate's
//! `README.md`.

use std::fmt;

use moddil_sema::Ty;
use moddil_syntax::TextRange;

mod build;
mod pretty;

pub use build::mir;

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
    /// Where control goes next.
    pub terminator: Terminator,
}

/// A statement.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Statement {
    /// `local = rvalue`.
    Assign(Local, Rvalue),
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
    },
    /// Reports a value to the test harness: an expression followed by a `// =>` expectation.
    Observe {
        /// The expression observed.
        range: TextRange,
        /// Its value.
        value: Operand,
    },
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
}

/// A local's value or a constant.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Operand {
    /// The value of a local.
    Copy(Local),
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
    /// Calls function `func` of the same file; its result goes to `dest`, then control to
    /// `target`.
    Call {
        /// The function, by its index in the file's item tree.
        func: u32,
        /// The arguments.
        args: Vec<Operand>,
        /// Where the result goes.
        dest: Local,
        /// Where control goes after the call returns.
        target: BlockId,
        /// The call's source.
        range: TextRange,
    },
    /// Returns `_0`.
    Return,
    /// Control never reaches here.
    Unreachable,
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
}

impl TrapClass {
    /// How the trap is named in messages and conformance tests: `overflow`.
    pub fn name(self) -> &'static str {
        match self {
            TrapClass::Overflow => "overflow",
            TrapClass::DivisionByZero => "division by zero",
            TrapClass::StackOverflow => "stack overflow",
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
