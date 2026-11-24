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
    /// The locals of the effects a function's signature names (`with time: Time`), with the
    /// effect's index in the signature's effects.
    pub effect_locals: Vec<(LocalId, usize)>,
    /// The function's preconditions (`contract.requires.1`), checked on entry.
    pub requires: Vec<ExprId>,
    /// Its postconditions (`contract.ensures.1`), checked at each return.
    pub ensures: Vec<ExprId>,
    /// The values `old(e)` in its postconditions names: a local for each, and `e`, evaluated on
    /// entry.
    pub olds: Vec<(LocalId, ExprId)>,
    /// The local `result` names in its postconditions: the value returned.
    pub result: Option<LocalId>,
    /// The arguments passed by name (`port: 8443`): each one's label, and where it is.
    pub labels: std::collections::BTreeMap<ExprId, (String, TextRange)>,
    /// The `*e` written inside an `unsafe` block, where a raw pointer may be dereferenced
    /// (`expr.unsafe.1`).
    pub unsafe_derefs: std::collections::HashSet<ExprId>,
    /// Every expression written inside an `unsafe` block: where a C function may be called
    /// (`ffi.extern.1`).
    pub in_unsafe: std::collections::HashSet<ExprId>,
    /// For a `#[selectable]` function, its two parts (`task.select.4`).
    pub select: Option<SelectParts>,
}

/// A `#[selectable]` function's two parts, each a closure in its body taking its parameters
/// (`task.select.4`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectParts {
    /// Gives the receiver the body receives from, borrowed.
    pub chan: ExprId,
    /// The rest of the body, taking what was received after the parameters.
    pub done: ExprId,
    /// The local `done` binds what was received to.
    pub received: LocalId,
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
    /// The innermost closure whose parameters or body declare it, if any.
    pub closure: Option<ExprId>,
}

/// An expression and where it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expr {
    /// What it is.
    pub kind: ExprKind,
    /// Where it is.
    pub range: TextRange,
}

/// What a [`ExprKind::Halt`] says: why the program stops there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Halt {
    /// `unreachable("reason")` (`expr.unreachable.1`): a broken invariant.
    Unreachable,
    /// `todo("what")` (`expr.todo.1`): code not written yet.
    Todo,
}

/// The kinds of expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExprKind {
    /// Something already reported as an error.
    Missing,
    /// An integer literal, with its exact value (a leading `-` folded in).
    Int(moddil_syntax::ast::IntValue),
    /// A fractional literal (`1.5`, `2e-3`), as written without `_`: an exact number until its
    /// type, a float, is known (`num.lit.1`).
    Float(String),
    /// `true` or `false`.
    Bool(bool),
    /// `()`.
    Unit,
    /// A `char` literal.
    Char(char),
    /// A string literal (plain, raw or a block), with its text: a `&str`.
    Str(String),
    /// `fmt"…"`: a `String` of its pieces.
    Fmt(Vec<FmtPiece>),
    /// An array literal, `[a, b, c]`.
    Array(Vec<ExprId>),
    /// `[x; n]`: `n` copies of `x`, with `n` a length known at compile time ([`Ty::Len`] or a
    /// const parameter).
    Repeat(ExprId, crate::Ty),
    /// `[x; n]` where `n` is not known when compiling: only for a type made by
    /// `FromArrayLiteral::from_repeat`.
    RepeatN(ExprId, ExprId),
    /// `base[index]`.
    Index {
        /// What is indexed.
        base: ExprId,
        /// The index.
        index: ExprId,
    },
    /// `base[start..end]`, `base[start..=end]`, `base[start..]`, `base[..end]` or `base[..]`
    /// (`expr.slice.1`): part of an array, a slice or a `Vec` (or, by positions, of text).
    Slice {
        /// What is sliced.
        base: ExprId,
        /// Where the part starts, if written.
        start: Option<ExprId>,
        /// Where it ends, if written.
        end: Option<ExprId>,
        /// Whether `end` is in the part (`..=`).
        inclusive: bool,
    },
    /// A const parameter used as a value: its index in the item's generics.
    ConstParam(u32),
    /// `const { e }`: compile-time code (`comptime.block.1`), which names no local, type
    /// parameter or const parameter of the body around it.
    ConstBlock(ExprId),
    /// A value of a refinement's base taken as the refined type (`X::new`'s body only): its
    /// index among the `type` items.
    Refine(ExprId, u32),
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
    /// `unreachable("reason")` or `todo("what")`: traps, with the reason.
    Halt(Halt, String),
    /// `e?`: the error of a fallible call, or of an `Err`, goes to the enclosing `try` block or
    /// out of the function; `None` likewise. Otherwise it is the value.
    Question(ExprId),
    /// `select { p = op => body, … }` (`task.select.1`): the arm of the first operation that
    /// is ready.
    Select(Vec<SelectArm>),
    /// In a wrapped function's body (`attr.around.1`), a wrapper's call: the errors it can fail
    /// with pass on, as through `?`, but its value is not taken apart.
    Pass(ExprId),
    /// `e ?! "reason"`: as `e?`, but an error traps, with the reason.
    Bang(ExprId, String),
    /// `fail(e)`: fails with the error `e`.
    Fail(ExprId),
    /// `try { body }`, a `Result` (or an `Option`), or `try { body } catch { arms }`, which
    /// handles the errors the arms match.
    Try {
        /// The block.
        body: ExprId,
        /// The `catch` arms, if any.
        catch: Option<Vec<Arm>>,
    },
    /// `discard(r, "reason")`: drops the `Result` `r` on purpose.
    Discard(ExprId),
    /// `impl Trait { fn … }`: a value of a type of its own implementing the trait.
    AnonImpl {
        /// The trait, with its arguments.
        trait_ref: crate::TraitRef,
        /// Its methods: each the trait's method it defines, and the closure that is its body
        /// (taking the method's parameters but `self`).
        methods: Vec<(u32, ExprId)>,
    },
    /// `|p, …| body`: a closure.
    Closure {
        /// Its parameters: a pattern each, and the type written, if any.
        params: Vec<(PatId, Option<crate::Ty>)>,
        /// The result type written, if any.
        ret: Option<crate::Ty>,
        /// Its body.
        body: ExprId,
        /// The closure it is written in, if any.
        parent: Option<ExprId>,
    },
    /// `with E = value, … { body }`: runs the block with handlers installed for its effects.
    With {
        /// The handlers, in order: their values are evaluated first, outside the block.
        handlers: Vec<Handler>,
        /// The block.
        body: ExprId,
    },
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

/// A piece of a `fmt"…"` literal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FmtPiece {
    /// Text as it is.
    Lit(String),
    /// A hole: an expression whose `Display` (or, with `?`, `Debug`) goes here, as its spec
    /// says.
    Hole(ExprId, Spec),
}

/// A hole's format spec, `{e:[[fill]align][0][width][.precision][?]}` (`expr.fmt.spec.1`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Spec {
    /// `?`: `Debug`, not `Display`.
    pub debug: bool,
    /// The character padding fills with, if written (a space otherwise).
    pub fill: Option<char>,
    /// The alignment, if written (numbers to the right, others to the left otherwise).
    pub align: Option<Align>,
    /// `0`: a number padded with zeros after its sign.
    pub zero: bool,
    /// The fewest characters written.
    pub width: Option<u32>,
    /// A float's digits after the point, or the most characters of text written.
    pub precision: Option<u32>,
}

impl Spec {
    /// Whether the piece is written as it is, with nothing to pad or cut.
    pub fn is_plain(&self) -> bool {
        self.width.is_none() && self.precision.is_none()
    }
}

/// Where a hole's text goes in its width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    /// `<`.
    Left,
    /// `^`: the extra fill after it when it does not split evenly.
    Center,
    /// `>`.
    Right,
}

/// A handler a `with` installs: `Time = clock`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Handler {
    /// The effect, as a type (`dyn Time`, or context data's type).
    pub effect: crate::Ty,
    /// The handler: a value whose type implements the trait (or is the context data's type), or
    /// a borrow of one.
    pub value: ExprId,
    /// Where it is written.
    pub range: TextRange,
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

/// An arm of a `select`: the pattern its operation's result is matched with, the operation
/// (`rx.recv()`, `tx.send(v)`, `t.join()` or `stop_requested()`, maybe with `?`), and its value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectArm {
    /// The pattern.
    pub pat: PatId,
    /// The operation.
    pub op: ExprId,
    /// The arm's value.
    pub body: ExprId,
}

/// A statement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stmt {
    /// `let p = init;`, `let p = init else { … };`, or `let x;`, whose local is assigned later.
    Let {
        /// The pattern bound.
        pat: PatId,
        /// The declared type, if written.
        ty: Option<crate::Ty>,
        /// The initialiser, if there is one (`stmt.let.uninit.1`).
        init: Option<ExprId>,
        /// The `else` block, which must not finish normally.
        else_: Option<ExprId>,
    },
    /// `defer block`: the block runs when the enclosing block is left, in last-in, first-out
    /// order with the other deferred blocks.
    Defer(ExprId),
    /// `errdefer block`: as `defer`, but only when the block is left because of an error.
    ErrDefer(ExprId),
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
    /// A `char` literal.
    Char(char),
    /// A string literal, matched against a `&str`.
    Str(String),
    /// `true` or `false`.
    Bool(bool),
    /// `lo..hi`, `lo..=hi` or `lo..`: the integers from `lo` to `hi`, with `hi` included as
    /// written (`None` for no upper bound).
    Range {
        /// The lower bound, included.
        lo: i128,
        /// The upper bound, if any.
        hi: Option<i128>,
        /// Whether the bounds are `char`s (as code points).
        chars: bool,
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
    /// `p: X` at the top of a `match` arm (`refine.pat.1`): matches if refinement `X`'s
    /// predicate holds, and then `p`, as an `X`.
    Refined {
        /// The refinement, by its index among the `type` items.
        refinement: u32,
        /// The pattern for the refined value.
        pat: PatId,
    },
    /// `[a, b]`, `[a, .., z]` or `[a, rest @ ..]` (`pat.slice.1`): an array's or a slice's
    /// elements.
    Slice {
        /// The element patterns, without the `..`.
        pats: Vec<PatId>,
        /// Where the `..` is among them, if anywhere.
        rest: Option<usize>,
        /// The binding of the elements `..` stands for (`rest @ ..`), a pattern matched against
        /// that part.
        rest_pat: Option<PatId>,
    },
    /// `a | b`.
    Or(Vec<PatId>),
    /// In a `catch` arm, `Type(p)`: an error of the struct or enum `adt`, matched by `p`.
    OfType {
        /// The error's type.
        adt: u32,
        /// The pattern for the error.
        pat: PatId,
    },
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
    /// `+%`: addition that wraps around the integer's width (`num.wrap.1`).
    WrapAdd,
    /// `-%`.
    WrapSub,
    /// `*%`.
    WrapMul,
}

impl BinaryOp {
    /// Whether it is `&`, `|`, `^`, `<<` or `>>`.
    pub fn is_bitwise(self) -> bool {
        matches!(
            self,
            BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor | BinaryOp::Shl | BinaryOp::Shr
        )
    }

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
            BinaryOp::WrapAdd => "+%",
            BinaryOp::WrapSub => "-%",
            BinaryOp::WrapMul => "*%",
        }
    }

    /// Whether it wraps rather than trapping on overflow: `+%`, `-%`, `*%`.
    pub fn wraps(self) -> bool {
        matches!(
            self,
            BinaryOp::WrapAdd | BinaryOp::WrapSub | BinaryOp::WrapMul
        )
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
