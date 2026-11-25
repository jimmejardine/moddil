//! The walk over the tree: where the structure breaks lines and indents. Every token and every
//! piece of trivia goes to [`Out`] in source order; a node with no rule of its own is printed
//! as its tokens, on the line it starts on.

use moddil_syntax::{SyntaxElement, SyntaxKind as K, SyntaxNode, SyntaxToken};

use crate::out::Out;

/// `root`, a `SOURCE_FILE`, printed.
pub(crate) fn print(root: &SyntaxNode) -> String {
    let mut p = Printer {
        out: Out::new(),
        chain_dots: Vec::new(),
    };
    p.lines(root);
    p.out.finish()
}

struct Printer {
    out: Out,
    /// Where the `.` of a broken method chain's calls are: each starts a line.
    chain_dots: Vec<u32>,
}

impl Printer {
    fn token(&mut self, t: &SyntaxToken) {
        match t.kind() {
            K::EOF => {}
            k if k.is_trivia() => self.out.trivia(t),
            K::DOT if self.chain_dots.contains(&t.text_range().start) => {
                self.out.soft_line();
                self.out.token(t);
            }
            _ => self.out.token(t),
        }
    }

    fn element(&mut self, e: &SyntaxElement) {
        match e {
            SyntaxElement::Node(n) => self.node(n),
            SyntaxElement::Token(t) => self.token(t),
        }
    }

    fn node(&mut self, n: &SyntaxNode) {
        if chain_top(n) {
            let dots = chain_breaks(n);
            if !dots.is_empty() {
                self.chain_dots.extend(dots);
                self.out.indent_after();
                self.kind(n);
                self.out.dedent();
                return;
            }
        }
        self.kind(n);
    }

    /// `n`, by the rule of its kind.
    fn kind(&mut self, n: &SyntaxNode) {
        match n.kind() {
            // `unsafe("reason") {`: the code under a reason starts on a line of its own.
            K::BLOCK => self.block(n, under_reason(n) || is_body(n)),
            K::FN_ITEM => self.fn_item(n),
            K::STRUCT_BODY if owns_lines(n) => self.body(n, Commas::Between),
            K::VARIANT_LIST => self.body(n, Commas::Between),
            K::MATCH_ARM_LIST => self.body(n, Commas::Arms),
            K::TRAIT_BODY | K::IMPL_BODY | K::EXTERN_BLOCK | K::MOD_ITEM => {
                self.body(n, Commas::None);
            }
            K::MATCH_ARM => self.match_arm(n),
            k if is_list(k) => self.list(n),
            _ => self.plain(n),
        }
    }

    /// `n`'s elements, its outer attributes on lines of their own where `n` is on one.
    fn plain(&mut self, n: &SyntaxNode) {
        let own = attrs_on_lines(n.kind());
        for e in n.children_with_tokens() {
            self.element(&e);
            if own && e.kind() == K::OUTER_ATTR {
                self.out.line();
            }
        }
    }

    /// A node whose child nodes are each on a line of their own: the source file, a broken
    /// block's statements.
    fn lines(&mut self, n: &SyntaxNode) {
        for e in n.children_with_tokens() {
            match &e {
                SyntaxElement::Node(c) => {
                    self.out.line();
                    self.node(c);
                    self.out.line();
                }
                SyntaxElement::Token(t) => self.token(t),
            }
        }
    }

    /// A block: on one line if it has no statements and nothing in it breaks (and `broken`
    /// does not ask for lines); otherwise a line for each statement.
    fn block(&mut self, n: &SyntaxNode, broken: bool) {
        if !broken && flat_block(n) && !if_chain_breaks(n) {
            return self.plain(n);
        }
        self.braced(n, |p, e| {
            if let SyntaxElement::Node(c) = e {
                p.out.line();
                p.node(c);
                p.out.line();
            } else if let SyntaxElement::Token(t) = e {
                p.token(t);
            }
        });
    }

    /// `n`'s elements, those between its `{` and `}` indented and given to `inner`.
    fn braced(&mut self, n: &SyntaxNode, mut inner: impl FnMut(&mut Self, &SyntaxElement)) {
        let elements: Vec<SyntaxElement> = n.children_with_tokens().collect();
        let open = elements.iter().position(|e| e.kind() == K::L_CURLY);
        let close = elements.iter().rposition(|e| e.kind() == K::R_CURLY);
        let (Some(open), Some(close)) = (open, close) else {
            return self.plain(n);
        };
        let own = attrs_on_lines(n.kind());
        for e in &elements[..open] {
            self.element(e);
            if own && e.kind() == K::OUTER_ATTR {
                self.out.line();
            }
        }
        self.element(&elements[open]);
        let empty = elements[open + 1..close]
            .iter()
            .all(|e| matches!(e, SyntaxElement::Token(t) if t.kind() == K::WHITESPACE));
        if empty {
            self.element(&elements[close]);
        } else {
            self.out.indent();
            self.out.line();
            for e in &elements[open + 1..close] {
                inner(self, e);
            }
            self.out.dedent();
            self.out.line();
            self.element(&elements[close]);
        }
        for e in &elements[close + 1..] {
            self.element(e);
        }
    }

    /// A body whose members are each on a line of their own, with `commas` between them.
    fn body(&mut self, n: &SyntaxNode, commas: Commas) {
        let members: Vec<SyntaxNode> = n
            .children()
            .filter(|c| !matches!(c.kind(), K::OUTER_ATTR | K::INNER_ATTR))
            .collect();
        let last = members.last().cloned();
        let mut comma_seen = false;
        self.braced(n, |p, e| match e {
            SyntaxElement::Node(c) => {
                p.out.line();
                p.node(c);
                comma_seen = false;
                if commas == Commas::Between && last.as_ref() == Some(c) && !followed_by_comma(c) {
                    p.out.comma();
                    comma_seen = true;
                }
                p.out.line();
            }
            SyntaxElement::Token(t) if t.kind() == K::COMMA && comma_seen => {}
            SyntaxElement::Token(t) => p.token(t),
        });
    }

    /// A match arm: a block body takes no comma after it, any other one does.
    fn match_arm(&mut self, n: &SyntaxNode) {
        let body_is_block = n.children().last().is_some_and(|b| b.kind() == K::BLOCK);
        let has_comma = n.token(K::COMMA).is_some();
        for e in n.children_with_tokens() {
            if e.kind() == K::COMMA && body_is_block {
                continue;
            }
            if e.kind() == K::EQ_GT
                && let Some(list) = n.parent()
            {
                self.out.arrow(list.text_range().start);
            }
            self.element(&e);
            if e.kind() == K::OUTER_ATTR {
                self.out.line();
            }
        }
        if !body_is_block && !has_comma {
            self.out.comma();
        }
    }

    /// A function: its signature on one line, its `where` and contract clauses each on a line
    /// of their own, indented, and then its body on lines of its own.
    fn fn_item(&mut self, n: &SyntaxNode) {
        let clauses = n
            .children()
            .any(|c| matches!(c.kind(), K::WHERE_CLAUSE | K::CONTRACT));
        let mut indented = false;
        for e in n.children_with_tokens() {
            match &e {
                SyntaxElement::Node(c) if matches!(c.kind(), K::WHERE_CLAUSE | K::CONTRACT) => {
                    if !indented {
                        self.out.indent();
                        indented = true;
                    }
                    self.out.soft_line();
                    self.node(c);
                }
                SyntaxElement::Node(c) if c.kind() == K::BLOCK => {
                    if indented {
                        self.out.dedent();
                        indented = false;
                        self.out.soft_line();
                    }
                    self.block(c, clauses || is_body(c));
                }
                SyntaxElement::Node(c) => {
                    self.node(c);
                    if c.kind() == K::OUTER_ATTR {
                        self.out.line();
                    }
                }
                SyntaxElement::Token(t) => self.token(t),
            }
        }
        if indented {
            self.out.dedent();
        }
    }

    /// A comma-separated list in delimiters: on one line, without a trailing comma; or, when a
    /// line comment is in it, broken after its opener and before its closer, each run of
    /// elements up to a comment on a line, with a trailing comma.
    fn list(&mut self, n: &SyntaxNode) {
        let elements: Vec<SyntaxElement> = n.children_with_tokens().collect();
        let open = elements.iter().position(|e| is_opener(e.kind()));
        let close = elements.iter().rposition(|e| is_closer(e.kind()));
        let (Some(open), Some(close)) = (open, close) else {
            return self.plain(n);
        };
        if close <= open {
            return self.plain(n);
        }
        let items: Vec<usize> = (open + 1..close)
            .filter(|&i| matches!(elements[i], SyntaxElement::Node(_)))
            .collect();
        let trailing = (open + 1..close)
            .rev()
            .find(|&i| !elements[i].kind().is_trivia() && elements[i].kind() != K::WHITESPACE);
        let trailing_comma = trailing.filter(|&i| elements[i].kind() == K::COMMA);
        let commas = takes_commas(n, items.len());
        let broken = commented(n);
        for e in &elements[..=open] {
            self.element(e);
        }
        if broken {
            self.out.indent();
            self.out.soft_line();
        }
        for (i, e) in elements.iter().enumerate().take(close).skip(open + 1) {
            if Some(i) == trailing_comma && commas && !broken {
                continue;
            }
            self.element(e);
            if broken && commas && trailing_comma.is_none() && items.last() == Some(&i) {
                self.out.comma();
            }
        }
        if broken {
            self.out.dedent();
            self.out.soft_line();
        }
        for e in &elements[close..] {
            self.element(e);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Commas {
    /// No commas: items.
    None,
    /// Between members, and after the last.
    Between,
    /// Match arms: after each arm whose body is not a block.
    Arms,
}

/// Whether node `c` is followed by a comma among its siblings, trivia aside.
fn followed_by_comma(c: &SyntaxNode) -> bool {
    let Some(parent) = c.parent() else {
        return false;
    };
    let mut after = false;
    for e in parent.children_with_tokens() {
        if after {
            if e.kind().is_trivia() || e.kind() == K::WHITESPACE {
                continue;
            }
            return e.kind() == K::COMMA;
        }
        if let SyntaxElement::Node(n) = &e
            && n == c
        {
            after = true;
        }
    }
    false
}

/// Whether a `STRUCT_BODY` is a struct's or a view's, whose fields are each on a line; a
/// variant's fields stay on its line.
fn owns_lines(n: &SyntaxNode) -> bool {
    n.parent()
        .is_some_and(|p| matches!(p.kind(), K::STRUCT_ITEM | K::VIEW_ITEM))
}

/// The nodes on a line of their own, whose outer attributes go on lines before them.
fn attrs_on_lines(k: K) -> bool {
    is_item(k)
        || matches!(
            k,
            K::FIELD
                | K::VARIANT
                | K::MATCH_ARM
                | K::LET_STMT
                | K::EXPR_STMT
                | K::ASSOC_TYPE
                | K::SELECT_ARM
                | K::DEFER_STMT
        )
}

fn is_item(k: K) -> bool {
    matches!(
        k,
        K::FN_ITEM
            | K::STRUCT_ITEM
            | K::VIEW_ITEM
            | K::ENUM_ITEM
            | K::TRAIT_ITEM
            | K::IMPL_ITEM
            | K::TYPE_ITEM
            | K::CONST_ITEM
            | K::USE_ITEM
            | K::MOD_ITEM
            | K::EXTERN_BLOCK
            | K::TEST_ITEM
            | K::PROPERTY_ITEM
            | K::BENCH_ITEM
            | K::GROUP_ITEM
            | K::UNIT_ITEM
            | K::DIMENSION_ITEM
    )
}

/// The comma-separated lists in delimiters, which a line comment breaks.
fn is_list(k: K) -> bool {
    matches!(
        k,
        K::PARAM_LIST
            | K::ARG_LIST
            | K::GENERIC_PARAMS
            | K::GENERIC_ARGS
            | K::TUPLE_FIELDS
            | K::STRUCT_BODY
            | K::USE_LIST
            | K::TUPLE_TYPE
            | K::RECORD_TYPE
            | K::TUPLE_PAT
            | K::SLICE_PAT
            | K::TUPLE_STRUCT_PAT
            | K::STRUCT_PAT
            | K::RECORD_PAT
            | K::TUPLE_EXPR
            | K::ARRAY_EXPR
            | K::RECORD_EXPR
            | K::STRUCT_LIT
            | K::INDEX_EXPR
    )
}

/// Whether list `n`, of `items` elements, takes a trailing comma when broken and drops one
/// when on one line: not a one-element tuple, whose comma makes it one, nor `[x; n]`.
fn takes_commas(n: &SyntaxNode, items: usize) -> bool {
    match n.kind() {
        K::TUPLE_EXPR | K::TUPLE_TYPE | K::TUPLE_PAT => items > 1,
        K::ARRAY_EXPR => n.token(K::SEMI).is_none(),
        K::INDEX_EXPR => false,
        _ => true,
    }
}

fn is_opener(k: K) -> bool {
    matches!(k, K::L_PAREN | K::L_BRACK | K::L_CURLY | K::LT)
}

fn is_closer(k: K) -> bool {
    matches!(k, K::R_PAREN | K::R_BRACK | K::R_CURLY | K::GT)
}

/// Whether a line comment is in list `n` itself, not in a list, block or body inside it.
fn commented(n: &SyntaxNode) -> bool {
    fn walk(n: &SyntaxNode, top: bool) -> bool {
        if !top && (is_list(n.kind()) || is_lined(n)) {
            return false;
        }
        n.children_with_tokens().any(|e| match e {
            SyntaxElement::Token(t) => {
                matches!(t.kind(), K::LINE_COMMENT | K::DOC_OUTER | K::DOC_INNER)
            }
            SyntaxElement::Node(c) => walk(&c, false),
        })
    }
    walk(n, true)
}

/// Whether node `n` is always printed on lines of its own, whatever surrounds it.
fn is_lined(n: &SyntaxNode) -> bool {
    if chain_top(n) && !chain_breaks(n).is_empty() {
        return true;
    }
    match n.kind() {
        K::BLOCK => !flat_shape(n) || under_reason(n) || is_body(n),
        K::STRUCT_BODY => owns_lines(n) && n.children().next().is_some(),
        K::VARIANT_LIST
        | K::MATCH_ARM_LIST
        | K::TRAIT_BODY
        | K::IMPL_BODY
        | K::EXTERN_BLOCK
        | K::MOD_ITEM => n.children().any(|c| c.kind() != K::OUTER_ATTR),
        K::FN_ITEM => n
            .children()
            .any(|c| matches!(c.kind(), K::WHERE_CLAUSE | K::CONTRACT)),
        k => is_item(k) && n.parent().is_some_and(|p| p.kind() != K::SOURCE_FILE),
    }
}

/// Whether block `n`'s shape lets it be on one line: no statements and no inner attributes,
/// only a value (or nothing).
fn flat_shape(n: &SyntaxNode) -> bool {
    let children: Vec<SyntaxNode> = n.children().collect();
    match children.as_slice() {
        [] => true,
        [c] => !is_statement(c.kind()),
        _ => false,
    }
}

fn is_statement(k: K) -> bool {
    is_item(k)
        || matches!(
            k,
            K::LET_STMT | K::EXPR_STMT | K::DEFER_STMT | K::INNER_ATTR | K::OUTER_ATTR | K::ERROR
        )
}

/// Whether block `n` is printed on one line: its shape allows it, and nothing inside it
/// breaks a line (a comment, a token over several lines, something always on lines).
fn flat_block(n: &SyntaxNode) -> bool {
    if !flat_shape(n) {
        return false;
    }
    let tokens_break = n.tokens().iter().any(|t| {
        matches!(t.kind(), K::LINE_COMMENT | K::DOC_OUTER | K::DOC_INNER)
            || (t.kind() != K::WHITESPACE && t.text().contains('\n'))
    });
    !tokens_break && !n.descendants().iter().skip(1).any(is_lined)
}

/// Whether block `n` is a branch of an `if … else` chain another branch of which is on lines:
/// then every branch is.
fn if_chain_breaks(n: &SyntaxNode) -> bool {
    let Some(mut top) = n.parent().filter(|p| p.kind() == K::IF_EXPR) else {
        return false;
    };
    while let Some(p) = top.parent().filter(|p| p.kind() == K::IF_EXPR) {
        top = p;
    }
    let mut at = Some(top);
    while let Some(i) = at {
        if i.children()
            .any(|c| c.kind() == K::BLOCK && !flat_block(&c))
        {
            return true;
        }
        at = i.children().find(|c| c.kind() == K::IF_EXPR);
    }
    false
}

/// Whether block `n` is an `unsafe` block's, after its reason.
fn under_reason(n: &SyntaxNode) -> bool {
    n.parent().is_some_and(|p| p.kind() == K::UNSAFE_BLOCK)
}

/// Whether block `n` is the body of a function, a test, a property or a bench, and not empty:
/// such a body always starts a new line at its `{`.
fn is_body(n: &SyntaxNode) -> bool {
    let named = n.parent().is_some_and(|p| {
        matches!(
            p.kind(),
            K::FN_ITEM | K::TEST_ITEM | K::PROPERTY_ITEM | K::BENCH_ITEM
        )
    });
    named
        && n.children_with_tokens()
            .any(|e| !matches!(e.kind(), K::L_CURLY | K::R_CURLY | K::WHITESPACE))
}

/// The nodes a method chain is made of, each with its receiver as its first child.
fn is_link(k: K) -> bool {
    matches!(
        k,
        K::METHOD_CALL_EXPR
            | K::FIELD_EXPR
            | K::PROPAGATE_EXPR
            | K::ERROR_CONTEXT
            | K::INDEX_EXPR
            | K::UNWRAP_EXPR
    )
}

/// Whether `n` is the outermost link of a chain: not the receiver of another link.
fn chain_top(n: &SyntaxNode) -> bool {
    is_link(n.kind())
        && !n
            .parent()
            .is_some_and(|p| is_link(p.kind()) && p.children().next().as_ref() == Some(n))
}

/// The offsets of the `.`s that start lines in the chain `n` ends: none unless two or more of
/// its method calls take arguments; then every method call from the first that takes
/// arguments on.
fn chain_breaks(n: &SyntaxNode) -> Vec<u32> {
    let mut calls = Vec::new();
    let mut at = Some(n.clone());
    while let Some(link) = at.filter(|l| is_link(l.kind())) {
        if link.kind() == K::METHOD_CALL_EXPR {
            calls.push(link.clone());
        }
        at = link.children().next();
    }
    calls.reverse();
    let takes_args = |c: &SyntaxNode| {
        c.child(K::ARG_LIST)
            .is_some_and(|a| a.children().next().is_some())
    };
    if calls.iter().filter(|c| takes_args(c)).count() < 2 {
        return Vec::new();
    }
    let first = calls.iter().position(takes_args).unwrap_or(0);
    calls[first..]
        .iter()
        .filter_map(|c| c.token(K::DOT).map(|d| d.text_range().start))
        .collect()
}
