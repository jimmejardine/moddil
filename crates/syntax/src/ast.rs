//! The typed AST: views over `SyntaxNode`s with accessors that return `Option`, since a tree
//! with errors may lack any part.
//!
//! It covers what slice S0 needs (functions, `let`, blocks, `if`, calls, operators, literals,
//! paths) and grows slice by slice.

use crate::{SyntaxKind, SyntaxNode, SyntaxToken};

/// A typed view of a node of one kind (or a family of kinds).
pub trait AstNode: Sized {
    /// Whether a node of `kind` is this type.
    fn can_cast(kind: SyntaxKind) -> bool;
    /// The view, if `node` is this type.
    fn cast(node: SyntaxNode) -> Option<Self>;
    /// The node underneath.
    fn syntax(&self) -> &SyntaxNode;
}

macro_rules! node {
    ($(#[$doc:meta])* $name:ident = $kind:ident) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash)]
        pub struct $name(SyntaxNode);

        impl AstNode for $name {
            fn can_cast(kind: SyntaxKind) -> bool {
                kind == SyntaxKind::$kind
            }
            fn cast(node: SyntaxNode) -> Option<Self> {
                Self::can_cast(node.kind()).then(|| $name(node))
            }
            fn syntax(&self) -> &SyntaxNode {
                &self.0
            }
        }

        impl $name {
            /// The view, if `node` is this type.
            pub fn cast(node: SyntaxNode) -> Option<Self> {
                <Self as AstNode>::cast(node)
            }
        }
    };
}

fn child<N: AstNode>(parent: &SyntaxNode) -> Option<N> {
    parent.children().find_map(N::cast)
}

fn children<'a, N: AstNode + 'a>(parent: &'a SyntaxNode) -> impl Iterator<Item = N> + 'a {
    parent.children().filter_map(N::cast)
}

node!(
    /// A whole file.
    SourceFile = SOURCE_FILE
);
node!(
    /// A function.
    FnItem = FN_ITEM
);
node!(
    /// A defining name.
    Name = NAME
);
node!(
    /// `( … )` after a function's name.
    ParamList = PARAM_LIST
);
node!(
    /// A parameter.
    Param = PARAM
);
node!(
    /// `-> T`.
    RetType = RET_TYPE
);
node!(
    /// `{ … }`.
    Block = BLOCK
);
node!(
    /// `let p: T = e;`.
    LetStmt = LET_STMT
);
node!(
    /// An expression followed by `;` (or a block-like expression with attributes).
    ExprStmt = EXPR_STMT
);
node!(
    /// A path: `a::b::c`.
    Path = PATH
);
node!(
    /// `test "name" { … }`.
    TestItem = TEST_ITEM
);
node!(
    /// `( … )` after a callee.
    ArgList = ARG_LIST
);
node!(
    /// One argument of a call, maybe labelled: `limit: 32`.
    Arg = ARG
);

impl SourceFile {
    /// The functions at the top level.
    pub fn fns(&self) -> impl Iterator<Item = FnItem> + '_ {
        children(&self.0)
    }
}

impl FnItem {
    /// The function's name.
    pub fn name(&self) -> Option<Name> {
        child(&self.0)
    }
    /// The parameters.
    pub fn params(&self) -> Option<ParamList> {
        child(&self.0)
    }
    /// The return type.
    pub fn ret_type(&self) -> Option<RetType> {
        child(&self.0)
    }
    /// The body, if the function has one.
    pub fn body(&self) -> Option<Block> {
        child(&self.0)
    }
}

impl TestItem {
    /// The string literal naming the test, quotes included.
    pub fn name_token(&self) -> Option<SyntaxToken> {
        self.0.token(SyntaxKind::STRING)
    }
    /// The body.
    pub fn body(&self) -> Option<Block> {
        child(&self.0)
    }
}

impl ArgList {
    /// The arguments, in order.
    pub fn args(&self) -> impl Iterator<Item = Arg> + '_ {
        children(&self.0)
    }
}

impl Arg {
    /// The label of a named argument (`limit` in `limit: 32`).
    pub fn label(&self) -> Option<SyntaxToken> {
        self.0
            .child_tokens()
            .find(|t| !t.kind().is_trivia() && t.kind() != SyntaxKind::COLON)
    }
    /// The argument's value.
    pub fn expr(&self) -> Option<Expr> {
        child(&self.0)
    }
}

impl Name {
    /// The name's text.
    pub fn text(&self) -> String {
        self.0
            .child_tokens()
            .find(|t| !t.kind().is_trivia())
            .map(|t| t.text().to_string())
            .unwrap_or_default()
    }
}

/// The defining name of an item, variant, field or parameter: its `NAME` child.
pub fn name(node: &SyntaxNode) -> Option<Name> {
    node.child(SyntaxKind::NAME).and_then(Name::cast)
}

/// The member a field expression, method call, struct-literal field or struct-pattern field
/// names: the first name-like token after its `.`, or its first one if it has no `.`. A member
/// is an identifier, a tuple index (`t.0`) or a keyword used as a name.
pub fn member_name(node: &SyntaxNode) -> Option<SyntaxToken> {
    let tokens: Vec<SyntaxToken> = node
        .child_tokens()
        .filter(|t| !t.kind().is_trivia())
        .collect();
    let start = tokens
        .iter()
        .position(|t| t.kind() == SyntaxKind::DOT)
        .map_or(0, |i| i + 1);
    tokens[start..]
        .iter()
        .find(|t| {
            matches!(t.kind(), SyntaxKind::IDENT | SyntaxKind::INTEGER) || t.kind().is_keyword()
        })
        .cloned()
}

/// The value of an integer literal's text: decimal, `0x`, `0o` or `0b`, with `_` separators
/// ignored. `None` if it has no digits or does not fit in an `i128`.
pub fn int_value(text: &str) -> Option<i128> {
    IntValue::parse(text)?.to_i128()
}

/// An integer literal's exact value: a sign and a magnitude up to `u128::MAX`, so a literal can
/// be any value of any integer type, `u128`'s greatest and `i128`'s least among them
/// (`num.lit.1`), before its type is known.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IntValue {
    neg: bool,
    mag: u128,
}

impl IntValue {
    /// The value of a literal's text (as [`int_value`] reads it), not negative; `None` if it has
    /// no digits or is past `u128::MAX`.
    pub fn parse(text: &str) -> Option<IntValue> {
        let digits: String = text.chars().filter(|&c| c != '_').collect();
        let (radix, digits) = match digits.get(..2) {
            Some("0x") => (16, &digits[2..]),
            Some("0o") => (8, &digits[2..]),
            Some("0b") => (2, &digits[2..]),
            _ => (10, digits.as_str()),
        };
        let mag = u128::from_str_radix(digits, radix).ok()?;
        Some(IntValue { neg: false, mag })
    }

    /// Its negation (zero stays zero).
    pub fn negated(self) -> IntValue {
        IntValue {
            neg: !self.neg && self.mag != 0,
            mag: self.mag,
        }
    }

    /// The value as an `i128`, if it is one.
    pub fn to_i128(self) -> Option<i128> {
        if self.neg {
            0i128.checked_sub_unsigned(self.mag)
        } else {
            i128::try_from(self.mag).ok()
        }
    }

    /// Its low 128 bits, two's complement: what it is as a value of a type it fits.
    pub fn bits(self) -> u128 {
        if self.neg {
            self.mag.wrapping_neg()
        } else {
            self.mag
        }
    }

    /// The nearest `f64` (its magnitude rounded once).
    pub fn as_f64(self) -> f64 {
        let m = self.mag as f64;
        if self.neg { -m } else { m }
    }

    /// Whether it is from `lo` to `hi`, inclusive.
    pub fn fits(self, lo: i128, hi: u128) -> bool {
        if self.neg {
            lo < 0 && self.mag <= lo.unsigned_abs()
        } else {
            self.mag <= hi && (lo <= 0 || self.mag >= lo.unsigned_abs())
        }
    }
}

impl From<i128> for IntValue {
    fn from(v: i128) -> IntValue {
        IntValue {
            neg: v < 0,
            mag: v.unsigned_abs(),
        }
    }
}

impl std::fmt::Display for IntValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.neg {
            write!(f, "-")?;
        }
        write!(f, "{}", self.mag)
    }
}

impl ParamList {
    /// The parameters, in order.
    pub fn params(&self) -> impl Iterator<Item = Param> + '_ {
        children(&self.0)
    }
}

impl Param {
    /// The pattern being bound.
    pub fn pat(&self) -> Option<Pat> {
        child(&self.0)
    }
    /// The declared type.
    pub fn ty(&self) -> Option<Type> {
        child(&self.0)
    }
}

impl RetType {
    /// The type after `->`.
    pub fn ty(&self) -> Option<Type> {
        child(&self.0)
    }
}

/// A statement or the block's final value.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Stmt {
    /// `let`.
    Let(LetStmt),
    /// An expression with `;`.
    Expr(ExprStmt),
    /// A block-like expression without `;` (not the last in the block).
    BlockLike(Expr),
    /// A local item.
    Item(SyntaxNode),
}

impl Block {
    /// The statements, not counting the final value.
    pub fn stmts(&self) -> Vec<Stmt> {
        let mut out: Vec<Stmt> = Vec::new();
        let nodes: Vec<SyntaxNode> = self
            .0
            .children()
            .filter(|n| n.kind() != SyntaxKind::INNER_ATTR)
            .collect();
        let last = nodes.len().checked_sub(1);
        for (i, n) in nodes.into_iter().enumerate() {
            if let Some(l) = LetStmt::cast(n.clone()) {
                out.push(Stmt::Let(l));
            } else if let Some(e) = ExprStmt::cast(n.clone()) {
                out.push(Stmt::Expr(e));
            } else if let Some(e) = Expr::cast(n.clone()) {
                if Some(i) != last {
                    out.push(Stmt::BlockLike(e));
                }
            } else {
                out.push(Stmt::Item(n));
            }
        }
        out
    }

    /// The final expression without `;`: the block's value.
    pub fn tail_expr(&self) -> Option<Expr> {
        self.0.children().last().and_then(Expr::cast)
    }
}

impl LetStmt {
    /// The pattern.
    pub fn pat(&self) -> Option<Pat> {
        child(&self.0)
    }
    /// The declared type.
    pub fn ty(&self) -> Option<Type> {
        child(&self.0)
    }
    /// The initialiser.
    pub fn init(&self) -> Option<Expr> {
        child(&self.0)
    }
}

impl ExprStmt {
    /// The expression.
    pub fn expr(&self) -> Option<Expr> {
        child(&self.0)
    }
}

impl Path {
    /// The segments' names, in order: `["a", "b"]` for `a::b`.
    pub fn segments(&self) -> Vec<String> {
        self.0
            .children()
            .filter(|n| n.kind() == SyntaxKind::PATH_SEGMENT)
            .map(|n| n.text().trim().to_string())
            .collect()
    }
}

macro_rules! family {
    ($(#[$doc:meta])* $name:ident { $($variant:ident = $kind:ident,)* }) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash)]
        #[allow(missing_docs)]
        pub enum $name { $($variant(SyntaxNode),)* }

        impl AstNode for $name {
            fn can_cast(kind: SyntaxKind) -> bool {
                matches!(kind, $(SyntaxKind::$kind)|*)
            }
            fn cast(node: SyntaxNode) -> Option<Self> {
                Some(match node.kind() {
                    $(SyntaxKind::$kind => $name::$variant(node),)*
                    _ => return None,
                })
            }
            fn syntax(&self) -> &SyntaxNode {
                match self { $($name::$variant(n) => n,)* }
            }
        }

        impl $name {
            /// The view, if `node` is this type.
            pub fn cast(node: SyntaxNode) -> Option<Self> {
                <Self as AstNode>::cast(node)
            }
        }
    };
}

family!(/// An expression.
    Expr {
        Literal = LITERAL,
        Path = PATH_EXPR,
        StructLit = STRUCT_LIT,
        Paren = PAREN_EXPR,
        Tuple = TUPLE_EXPR,
        Array = ARRAY_EXPR,
        Record = RECORD_EXPR,
        Block = BLOCK,
        ConstBlock = CONST_BLOCK,
        Impl = IMPL_EXPR,
        Fail = FAIL_EXPR,
        If = IF_EXPR,
        Match = MATCH_EXPR,
        Loop = LOOP_EXPR,
        While = WHILE_EXPR,
        For = FOR_EXPR,
        Labelled = LABELLED_EXPR,
        Try = TRY_EXPR,
        With = WITH_EXPR,
        Select = SELECT_EXPR,
        Unsafe = UNSAFE_BLOCK,
        Gen = GEN_BLOCK,
        Return = RETURN_EXPR,
        Break = BREAK_EXPR,
        Continue = CONTINUE_EXPR,
        Yield = YIELD_EXPR,
        Escape = ESCAPE_EXPR,
        Closure = CLOSURE_EXPR,
        Assign = ASSIGN_EXPR,
        Range = RANGE_EXPR,
        Bin = BIN_EXPR,
        LetCond = LET_COND,
        CompareChain = COMPARE_CHAIN,
        In = IN_EXPR,
        Prefix = PREFIX_EXPR,
        Ref = REF_EXPR,
        KeyPath = KEY_PATH,
        Propagate = PROPAGATE_EXPR,
        Unwrap = UNWRAP_EXPR,
        Call = CALL_EXPR,
        MethodCall = METHOD_CALL_EXPR,
        Field = FIELD_EXPR,
        Index = INDEX_EXPR,
        InlineFor = INLINE_FOR,
        ReflectedField = REFLECTED_FIELD_EXPR,
    }
);

family!(/// A pattern.
    Pat {
        Or = OR_PAT,
        Wildcard = WILDCARD_PAT,
        Rest = REST_PAT,
        Literal = LITERAL_PAT,
        Range = RANGE_PAT,
        Ident = IDENT_PAT,
        Path = PATH_PAT,
        TupleStruct = TUPLE_STRUCT_PAT,
        Struct = STRUCT_PAT,
        At = AT_PAT,
        Ref = REF_PAT,
        Tuple = TUPLE_PAT,
        Slice = SLICE_PAT,
        Record = RECORD_PAT,
        Typed = TYPED_PAT,
    }
);

family!(/// A type.
    Type {
        Path = PATH_TYPE,
        Ref = REF_TYPE,
        Ptr = PTR_TYPE,
        Array = ARRAY_TYPE,
        Slice = SLICE_TYPE,
        Paren = PAREN_TYPE,
        Tuple = TUPLE_TYPE,
        Refined = REFINED_TYPE,
        Record = RECORD_TYPE,
        Fn = FN_TYPE,
        Never = NEVER_TYPE,
        Infer = INFER_TYPE,
        Impl = IMPL_TYPE,
        Dyn = DYN_TYPE,
        Effect = EFFECT_TYPE,
        TypeType = TYPE_TYPE,
    }
);

/// The operator token of a binary, assignment, range or prefix expression.
pub fn operator(node: &SyntaxNode) -> Option<SyntaxToken> {
    node.child_tokens().find(|t| !t.kind().is_trivia())
}

impl Expr {
    /// For a binary expression: the left and right operands.
    pub fn operands(&self) -> (Option<Expr>, Option<Expr>) {
        let mut it = children::<Expr>(self.syntax());
        (it.next(), it.next())
    }

    /// The sub-expressions, in order.
    pub fn sub_exprs(&self) -> Vec<Expr> {
        children(self.syntax()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::int_value;

    #[test]
    fn integer_literals() {
        assert_eq!(int_value("1_000"), Some(1000));
        assert_eq!(int_value("0xff"), Some(255));
        assert_eq!(int_value("0o17"), Some(15));
        assert_eq!(int_value("0b1010"), Some(10));
        assert_eq!(int_value("0x_"), None);
        assert_eq!(int_value(&"9".repeat(50)), None);
    }
}
