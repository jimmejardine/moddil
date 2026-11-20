//! The HIR of a body: expressions and statements in arenas, with names resolved and the surface
//! sugar removed (`else if` is an `if` in an `else` block, `x += e` an assignment with an operator,
//! parentheses are gone).

use la_arena::{Arena, Idx};
use moddil_syntax::TextRange;

/// An expression's ID within its body.
pub type ExprId = Idx<Expr>;
/// A local's ID within its body.
pub type LocalId = Idx<Local>;

/// A function's or a test's body.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Body {
    /// The parameters' locals, in order (none for a test).
    pub params: Vec<LocalId>,
    /// Every local: parameters and `let` bindings.
    pub locals: Arena<Local>,
    /// Every expression.
    pub exprs: Arena<Expr>,
    /// The body's block; `None` if the body is missing.
    pub root: Option<ExprId>,
}

/// A local variable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Local {
    /// The name.
    pub name: String,
    /// Whether it is declared `mut`.
    pub mutable: bool,
    /// Where the binding is.
    pub range: TextRange,
}

/// An expression and where it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expr {
    /// What it is.
    pub kind: ExprKind,
    /// Where it is.
    pub range: TextRange,
}

/// The kinds of expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExprKind {
    /// Something already reported as an error.
    Missing,
    /// An integer literal, with its exact value (a leading `-` folded in).
    Int(i128),
    /// `true` or `false`.
    Bool(bool),
    /// `()`.
    Unit,
    /// A local.
    Local(LocalId),
    /// A function of this file, by its index in the item tree.
    Fn(u32),
    /// `callee(args)`.
    Call {
        /// The function called.
        callee: ExprId,
        /// The arguments.
        args: Vec<ExprId>,
    },
    /// `{ stmts; tail }`.
    Block {
        /// The statements.
        stmts: Vec<Stmt>,
        /// The final expression, without `;`.
        tail: Option<ExprId>,
        /// Whether the tail is followed by a `// =>` expectation on its line.
        observe_tail: bool,
    },
    /// `if cond { … } else …`.
    If {
        /// The condition.
        cond: ExprId,
        /// The block taken when it holds.
        then: ExprId,
        /// The `else` block (an `else if` is a block holding an `if`).
        else_: Option<ExprId>,
    },
    /// `return e`.
    Return(Option<ExprId>),
    /// `-e` or `!e`.
    Unary(UnaryOp, ExprId),
    /// `a op b`, other than `&&` and `||`.
    Binary(BinaryOp, ExprId, ExprId),
    /// `a && b` or `a || b`.
    Logic(LogicOp, ExprId, ExprId),
    /// `a < b <= c`: each operand evaluated once, left to right, stopping at the first false.
    Compare {
        /// The first operand.
        first: ExprId,
        /// Each operator and the operand after it.
        rest: Vec<(CompareOp, ExprId)>,
    },
    /// `local = e` or `local op= e`.
    Assign {
        /// The local assigned.
        local: LocalId,
        /// The operator of a compound assignment.
        op: Option<BinaryOp>,
        /// The value.
        value: ExprId,
    },
}

/// A statement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stmt {
    /// `let p = init;`.
    Let {
        /// The local bound (`None` for `_`).
        local: Option<LocalId>,
        /// The declared type, if written.
        ty: Option<crate::Ty>,
        /// The initialiser.
        init: ExprId,
    },
    /// An expression statement.
    Expr {
        /// The expression.
        expr: ExprId,
        /// Whether it ends with `;` (a block-like expression may not).
        semi: bool,
        /// Whether it is followed by a `// =>` expectation on its line.
        observe: bool,
    },
}

/// Prefix operators.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnaryOp {
    /// `-`.
    Neg,
    /// `!`: logical not on `bool`, bitwise not on `int`.
    Not,
}

/// Binary operators other than the logical and comparison ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BinaryOp {
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
}

impl BinaryOp {
    /// The operator's symbol.
    pub fn symbol(self) -> &'static str {
        match self {
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Mul => "*",
            BinaryOp::Div => "/",
            BinaryOp::Rem => "%",
            BinaryOp::BitAnd => "&",
            BinaryOp::BitOr => "|",
            BinaryOp::BitXor => "^",
            BinaryOp::Shl => "<<",
            BinaryOp::Shr => ">>",
        }
    }

    /// Whether the operator also applies to `bool` (`&`, `|`, `^`).
    pub fn applies_to_bool(self) -> bool {
        matches!(self, BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor)
    }
}

/// `&&` and `||`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LogicOp {
    /// `&&`.
    And,
    /// `||`.
    Or,
}

/// Comparison operators.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CompareOp {
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

impl CompareOp {
    /// The operator's symbol.
    pub fn symbol(self) -> &'static str {
        match self {
            CompareOp::Eq => "==",
            CompareOp::Ne => "!=",
            CompareOp::Lt => "<",
            CompareOp::Le => "<=",
            CompareOp::Gt => ">",
            CompareOp::Ge => ">=",
        }
    }

    /// Whether the operator orders its operands (`<`, `<=`, `>`, `>=`), so applies only to `int`.
    pub fn is_ordering(self) -> bool {
        !matches!(self, CompareOp::Eq | CompareOp::Ne)
    }
}
