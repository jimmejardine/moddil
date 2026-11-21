//! The HIR of a body: expressions, patterns and statements in arenas, with names resolved and the
//! surface sugar removed (`else if` is an `if` in an `else` block, `x += e` an assignment with an
//! operator, a tuple-struct constructor call a struct expression, parentheses are gone).

use la_arena::{Arena, Idx};
use moddil_syntax::TextRange;

/// An expression's ID within its body.
pub type ExprId = Idx<Expr>;
/// A local's ID within its body.
pub type LocalId = Idx<Local>;
/// A pattern's ID within its body.
pub type PatId = Idx<Pat>;

/// A function's or a test's body.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Body {
    /// The parameters' locals, in order (none for a test).
    pub params: Vec<LocalId>,
    /// Every local: parameters and bindings.
    pub locals: Arena<Local>,
    /// Every expression.
    pub exprs: Arena<Expr>,
    /// Every pattern.
    pub pats: Arena<Pat>,
    /// Parameters written as patterns (`(a, b): (int, int)`): the parameter's local and the
    /// pattern its value is destructured with on entry.
    pub param_pats: Vec<(LocalId, PatId)>,
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
    /// A function named by a path, by its index in the item tree, with the type arguments for
    /// its own type parameters ([`Ty::HOLE`](crate::Ty::HOLE) where they are inferred).
    Fn {
        /// The function.
        func: u32,
        /// Its type arguments.
        args: Vec<crate::Ty>,
    },
    /// `Type::name`: a function of an inherent impl of the type, or a method of a trait it
    /// implements.
    AssocFn {
        /// The type (with holes for arguments to infer).
        ty: crate::Ty,
        /// The function's name.
        name: String,
        /// Where the name is.
        name_range: TextRange,
        /// Type arguments written for the function's own parameters, if any.
        args: Option<Vec<crate::Ty>>,
    },
    /// `Trait::name`: a trait's method, for a `Self` to infer.
    TraitFn {
        /// The trait.
        trait_: u32,
        /// The method's declaration in the trait.
        func: u32,
        /// The trait's type arguments (holes when not written).
        trait_args: Vec<crate::Ty>,
        /// Type arguments written for the method's own parameters, if any.
        args: Option<Vec<crate::Ty>>,
    },
    /// `receiver.name(args)`.
    MethodCall {
        /// The value the method is called on.
        receiver: ExprId,
        /// The method's name.
        name: String,
        /// Where the name is.
        name_range: TextRange,
        /// Type arguments written for the method's own parameters (`x.f::<int>()`), if any.
        generic_args: Option<Vec<crate::Ty>>,
        /// The other arguments.
        args: Vec<ExprId>,
    },
    /// `(a, b)`: a tuple of one or more elements.
    Tuple(Vec<ExprId>),
    /// A struct or variant value: `Point { x: 1, y: 2 }`, `Pair(1, 2)`, `Colour::Red`.
    Adt {
        /// The struct or enum, by its index in the item tree.
        adt: u32,
        /// The variant (0 for a struct).
        variant: u32,
        /// Each field's index and value, in source order (the order they are evaluated in).
        fields: Vec<(u32, ExprId)>,
        /// The struct's or enum's type arguments (holes where they are inferred).
        args: Vec<crate::Ty>,
    },
    /// `base.name` or `base.0`.
    Field {
        /// The value whose field is read.
        base: ExprId,
        /// The field's name, or its position for a tuple or tuple struct.
        name: String,
        /// Where the name is.
        name_range: TextRange,
    },
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
        /// The condition, which may contain `let` conditions joined by `&&`.
        cond: ExprId,
        /// The block taken when it holds.
        then: ExprId,
        /// The `else` block (an `else if` is a block holding an `if`).
        else_: Option<ExprId>,
    },
    /// `let p = e` in the head of an `if` or `while`: whether `e` matches `p`, binding its names
    /// if it does.
    Let {
        /// The pattern.
        pat: PatId,
        /// The value matched.
        init: ExprId,
    },
    /// `match scrutinee { arms }`.
    Match {
        /// The value matched.
        scrutinee: ExprId,
        /// The arms, tried in order.
        arms: Vec<Arm>,
    },
    /// `loop { … }`: its value is that of the `break` that leaves it.
    Loop {
        /// The body.
        body: ExprId,
    },
    /// `while cond { … }`.
    While {
        /// The condition, which may contain `let` conditions joined by `&&`.
        cond: ExprId,
        /// The body.
        body: ExprId,
    },
    /// `break` out of loop `target`, with a value for a `loop`.
    Break {
        /// The `loop` or `while` expression left.
        target: ExprId,
        /// The value.
        value: Option<ExprId>,
    },
    /// `continue` with the next iteration of loop `target`.
    Continue {
        /// The `loop` or `while` expression continued.
        target: ExprId,
    },
    /// `return e`.
    Return(Option<ExprId>),
    /// `assert(cond)`: traps unless `cond` holds.
    Assert(ExprId),
    /// `&e` or `&mut e`: a borrow of the place `e` (or of a temporary holding its value).
    Ref {
        /// Whether it is `&mut`.
        mutable: bool,
        /// What is borrowed.
        expr: ExprId,
    },
    /// `*e`: the place a borrow points to.
    Deref(ExprId),
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
    /// `place = e` or `place op= e`, where the place is a local, a dereferenced borrow, or fields
    /// of one (`p.x.0`, `*r`, `r.x`).
    Assign {
        /// The place assigned: `Local`, `Deref` and `Field` expressions.
        target: ExprId,
        /// The operator of a compound assignment.
        op: Option<BinaryOp>,
        /// The value.
        value: ExprId,
    },
}

/// One arm of a `match`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arm {
    /// The pattern.
    pub pat: PatId,
    /// The guard: `if cond`.
    pub guard: Option<ExprId>,
    /// The arm's value.
    pub body: ExprId,
}

/// A statement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stmt {
    /// `let p = init;` or `let p = init else { … };`.
    Let {
        /// The pattern bound.
        pat: PatId,
        /// The declared type, if written.
        ty: Option<crate::Ty>,
        /// The initialiser.
        init: ExprId,
        /// The `else` block, which must not finish normally.
        else_: Option<ExprId>,
    },
    /// `defer block`: the block runs when the enclosing block is left, in last-in, first-out
    /// order with the other deferred blocks.
    Defer(ExprId),
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

/// A pattern and where it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pat {
    /// What it is.
    pub kind: PatKind,
    /// Where it is.
    pub range: TextRange,
}

/// The kinds of pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatKind {
    /// Something already reported as an error; matches anything.
    Missing,
    /// `_`.
    Wild,
    /// `x`, `mut x` or `x @ p`.
    Bind {
        /// The local bound.
        local: LocalId,
        /// The pattern after `@`.
        sub: Option<PatId>,
    },
    /// An integer literal (a leading `-` folded in).
    Int(i128),
    /// `true` or `false`.
    Bool(bool),
    /// `lo..hi`, `lo..=hi` or `lo..`: the integers from `lo` to `hi`, with `hi` included as
    /// written (`None` for no upper bound).
    Range {
        /// The lower bound, included.
        lo: i128,
        /// The upper bound, if any.
        hi: Option<i128>,
        /// Whether the upper bound is included (`..=`).
        inclusive: bool,
    },
    /// `()`.
    Unit,
    /// `(a, b)` or `(a, .., z)`.
    Tuple {
        /// The element patterns, without the `..`.
        pats: Vec<PatId>,
        /// Where the `..` is among them, if anywhere.
        rest: Option<usize>,
    },
    /// A struct or variant: `Point { x, .. }`, `Pair(a, b)`, `Colour::Red`.
    Adt {
        /// The struct or enum.
        adt: u32,
        /// The variant (0 for a struct).
        variant: u32,
        /// The fields matched, by index; the others match anything.
        fields: Vec<(u32, PatId)>,
    },
    /// `a | b`.
    Or(Vec<PatId>),
    /// `&p` or `&mut p`: matches through a borrow, binding by copying.
    Ref {
        /// Whether it is `&mut p`.
        mutable: bool,
        /// The pattern for the value the borrow points to.
        pat: PatId,
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
