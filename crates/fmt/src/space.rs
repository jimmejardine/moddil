//! Spaces within a line: whether two tokens printed next to each other have a space between
//! them. The answer depends on the two tokens and the nodes they are in; where no rule speaks,
//! the source decides (one space if it has any whitespace there, none if not).

use moddil_syntax::{SyntaxKind as K, SyntaxNode, SyntaxToken};

/// Whether `cur`, printed after `prev` on the same line, has a space before it. `ws` is
/// whether the source has whitespace between them.
pub(crate) fn between(prev: &SyntaxToken, cur: &SyntaxToken, ws: bool) -> bool {
    let (p, c) = (prev.kind(), cur.kind());
    let (pp, cp) = (prev.parent(), cur.parent());
    if p == K::GT && c == K::GT {
        // `Vec<Vec<T>>`: the parser splits `>>` where generic arguments close.
        return false;
    }
    if joins(prev.text(), cur.text()) {
        return true;
    }
    if matches!(c, K::COMMA | K::SEMI) {
        return false;
    }
    if matches!(c, K::R_PAREN | K::R_BRACK) {
        return false;
    }
    // `use a::{b, c}`: a use list's braces hold no spaces.
    if (c == K::R_CURLY && cp.kind() == K::USE_LIST)
        || (p == K::L_CURLY && pp.kind() == K::USE_LIST)
    {
        return false;
    }
    if c == K::R_CURLY {
        return p != K::L_CURLY;
    }
    if matches!(p, K::COMMA | K::SEMI) {
        return true;
    }
    if matches!(
        p,
        K::L_PAREN | K::L_BRACK | K::POUND_L_BRACK | K::POUND_BANG_L_BRACK
    ) {
        return false;
    }
    if p == K::L_CURLY {
        return true;
    }
    if c == K::COLON_COLON || p == K::COLON_COLON {
        return false;
    }
    if c == K::DOT || p == K::DOT {
        return false;
    }
    if is_unit(&pp) || is_unit(&cp) {
        return ws;
    }
    // A prefix operator: `-x`, `!x`, `&mut x`, `*p`, `&self`, `\T.f`.
    if matches!(
        p,
        K::MINUS | K::BANG | K::STAR | K::AMP | K::AMP_AMP | K::BACKSLASH
    ) && prefixes(pp.kind())
        && first(&pp).is_some_and(|f| f == *prev)
    {
        return false;
    }
    // An attribute's input is tokens, not expressions: whether `&` there is a prefix
    // (`#[around(w(&table()))]`) or an operator, the source says.
    if matches!(pp.kind(), K::ATTR_INPUT | K::TOKEN_TREE)
        && matches!(p, K::MINUS | K::BANG | K::STAR | K::AMP)
    {
        return ws;
    }
    // Generic parameters and arguments: `Vec<T>`, `f::<T>()`, `<T as Tr>::X`.
    let angled = |n: &SyntaxNode| {
        matches!(
            n.kind(),
            K::GENERIC_PARAMS | K::GENERIC_ARGS | K::QUALIFIED_PATH
        )
    };
    if c == K::LT && angled(&cp) {
        return cp.kind() == K::QUALIFIED_PATH && ws;
    }
    if (p == K::LT && angled(&pp)) || (c == K::GT && angled(&cp)) {
        return false;
    }
    // A closure's bars: `|a, b| a + b`.
    if pp.kind() == K::CLOSURE_PARAMS && p == K::PIPE && first(&pp).is_some_and(|f| f == *prev) {
        return false;
    }
    if cp.kind() == K::CLOSURE_PARAMS
        && c == K::PIPE
        && last(&cp).is_some_and(|l| l == *cur)
        && p != K::PIPE
    {
        return false;
    }
    match c {
        K::COLON | K::QUESTION => return false,
        K::BANG if cp.kind() == K::UNWRAP_EXPR => return false,
        // A name, even a keyword used as one (`fn default()`), takes its parameters right after it.
        K::L_PAREN => return pp.kind() != K::NAME && !calls(p, &pp, &cp),
        K::L_BRACK => return !indexes(p, &pp),
        K::L_CURLY => return true,
        K::DOT_DOT | K::DOT_DOT_EQ => return !ends_value(p),
        _ => {}
    }
    match p {
        K::COLON => return true,
        K::DOT_DOT | K::DOT_DOT_EQ => return false,
        _ => {}
    }
    true
}

/// The punctuation the parser reads as one token when its characters are next to each other.
const COMPOUNDS: &[&str] = &[
    "<<=", ">>=", "+%=", "-%=", "*%=", "..=", "&&", "||", "<<", ">>", "+%", "-%", "*%", "?!", "+=",
    "-=", "*=", "/=", "%=", "^=", "&=", "|=", "==", "!=", "<=", ">=", "..", "::", "->", "=>",
];

/// Whether `a` and `b` next to each other would read as other punctuation: `| |` is not `||`,
/// `.. =>` is not `..= >`, and a `/` before `/` or `*` would start a comment.
fn joins(a: &str, b: &str) -> bool {
    let punct = |s: &str| !s.is_empty() && s.chars().all(|ch| ch.is_ascii_punctuation());
    if !punct(a) || !punct(b) {
        return false;
    }
    if a.ends_with('/') && (b.starts_with('/') || b.starts_with('*')) {
        return true;
    }
    let both = format!("{a}{b}");
    COMPOUNDS
        .iter()
        .any(|x| x.len() > a.len() && both.starts_with(x))
}

/// Whether `n` is part of a unit of measure, whose spacing the source keeps.
fn is_unit(n: &SyntaxNode) -> bool {
    matches!(
        n.kind(),
        K::UNIT_SUFFIX | K::UNIT_BIN | K::UNIT_POWER | K::UNIT_PAREN | K::EXPONENT
    )
}

/// The nodes whose first token is a prefix operator.
fn prefixes(k: K) -> bool {
    matches!(
        k,
        K::PREFIX_EXPR
            | K::REF_EXPR
            | K::REF_TYPE
            | K::PTR_TYPE
            | K::REF_PAT
            | K::SELF_PARAM
            | K::LITERAL_PAT
            | K::RANGE_PAT
            | K::EFFECT
            | K::KEY_PATH
            | K::SIG_EFFECT
    )
}

/// Whether a `(` after `p` opens arguments or parameters right after what they belong to:
/// `f(x)`, `fn f(x)`, `Some(x)`, `fail(E)`, `pub(mod)`, `fn(int) -> int`.
fn calls(p: K, pp: &SyntaxNode, cp: &SyntaxNode) -> bool {
    match p {
        K::IDENT
        | K::SELF_KW
        | K::SELF_TYPE_KW
        | K::SUPER_KW
        | K::R_PAREN
        | K::R_BRACK
        | K::QUESTION
        | K::QUESTION_BANG => true,
        K::GT => matches!(
            pp.kind(),
            K::GENERIC_ARGS | K::GENERIC_PARAMS | K::QUALIFIED_PATH
        ),
        K::FAIL_KW
        | K::PUB_KW
        | K::UNSAFE_KW
        | K::FN_KW
        | K::DYN_KW
        | K::IMPL_KW
        | K::ESCAPE_KW => !matches!(cp.kind(), K::TUPLE_EXPR | K::PAREN_EXPR | K::BLOCK),
        K::R_CURLY => matches!(cp.kind(), K::ARG_LIST),
        _ => false,
    }
}

/// Whether a `[` after `p` indexes what is before it: `a[i]`, `f()[0]`.
fn indexes(p: K, pp: &SyntaxNode) -> bool {
    match p {
        K::IDENT
        | K::SELF_KW
        | K::R_PAREN
        | K::R_BRACK
        | K::QUESTION
        | K::QUESTION_BANG
        | K::STRING
        | K::INTEGER => true,
        K::R_CURLY => pp.kind() != K::BLOCK,
        _ => false,
    }
}

/// Whether `p` ends a value, so that a range operator after it binds to it: `a..b`.
fn ends_value(p: K) -> bool {
    matches!(
        p,
        K::IDENT
            | K::INTEGER
            | K::FLOAT
            | K::STRING
            | K::CHAR
            | K::BYTE
            | K::SELF_KW
            | K::TRUE_KW
            | K::FALSE_KW
            | K::R_PAREN
            | K::R_BRACK
            | K::R_CURLY
            | K::QUESTION
            | K::QUESTION_BANG
    )
}

/// The first token of `n`, trivia aside.
pub(crate) fn first(n: &SyntaxNode) -> Option<SyntaxToken> {
    n.tokens().into_iter().find(|t| !t.kind().is_trivia())
}

/// The last token of `n`, trivia aside.
fn last(n: &SyntaxNode) -> Option<SyntaxToken> {
    n.tokens().into_iter().rev().find(|t| !t.kind().is_trivia())
}
