//! The grammar functions: one per rule (or family of rules) of `reference/grammar/moddil.ebnf`,
//! in the same order and with the same names where possible. Each decision looks at most three
//! grammar tokens ahead, as the grammar is LL(3).

mod exprs;
mod items;
mod patterns;
mod types;

use crate::SyntaxKind as K;
use crate::parser::{Parser, TokenSet};

pub(crate) use items::source_file;

/// The grammar's flags: Rust's restrictions, passed down the expression rules.
#[derive(Clone, Copy, Default)]
pub(crate) struct R {
    /// No struct or record literal and no bare block: in heads that a block follows.
    ns: bool,
    /// The expression starts a statement: it may not begin with a block-like expression.
    stmt: bool,
    /// A `let` condition may appear (`if` and `while` heads).
    cond: bool,
}

impl R {
    const NONE: R = R {
        ns: false,
        stmt: false,
        cond: false,
    };
    const NS: R = R {
        ns: true,
        stmt: false,
        cond: false,
    };
    const NS_COND: R = R {
        ns: true,
        stmt: false,
        cond: true,
    };
    const STMT: R = R {
        ns: false,
        stmt: true,
        cond: false,
    };

    /// `name@ns`: keep only `ns` and `cond`.
    fn operand(self) -> R {
        R {
            stmt: false,
            ..self
        }
    }
}

/// Literal tokens (`literal_token`).
const LITERAL_TOKENS: TokenSet = TokenSet::new(&[
    K::INTEGER,
    K::FLOAT,
    K::STRING,
    K::RAW_STRING,
    K::BYTE_STRING,
    K::BLOCK_STRING,
    K::TAGGED_STRING,
    K::CHAR,
    K::BYTE,
]);

/// Where list and block parsing can resume after an error.
const RECOVERY: TokenSet = TokenSet::new(&[
    T![";"],
    T!["}"],
    T![")"],
    T!["]"],
    T![","],
    T![fn],
    T![struct],
    T![enum],
    T![trait],
    T![impl],
    T![use],
    T![let],
]);

fn at_member_name(p: &mut Parser) -> bool {
    let k = p.current();
    k == K::IDENT || k.is_keyword()
}

/// `member_name ::= IDENT | KEYWORD`
fn member_name(p: &mut Parser) {
    if at_member_name(p) {
        let k = p.current();
        p.bump(k);
    } else {
        p.err_recover("expected a name", RECOVERY);
    }
}

/// A defining name: `IDENT` (or `member_name` where `keywords`), wrapped in `NAME`.
fn name(p: &mut Parser, keywords: bool) {
    let m = p.start();
    if keywords {
        member_name(p);
    } else if !p.eat(K::IDENT) {
        let found = p.current();
        p.err_recover(
            format!("expected a name, found {}", found.describe()),
            RECOVERY,
        );
    }
    m.complete(p, K::NAME);
}

/// `IDENT ':'` ahead (a name and a colon, not `::`).
fn at_name_colon(p: &mut Parser) -> bool {
    p.at(K::IDENT) && p.nth_at(1, T![":"])
}

/// `{ member_name ':'` ahead: a record (literal, type or pattern), not a block.
fn at_record(p: &mut Parser) -> bool {
    if !p.at(T!["{"]) {
        return false;
    }
    let k = p.nth(1);
    (k == K::IDENT || k.is_keyword()) && p.nth_at(2, T![":"])
}

/// A comma-separated list up to `close` (not consumed), with an optional trailing comma.
/// `item` parses one element; `can_start` says whether one starts here.
fn comma_list(
    p: &mut Parser,
    close: K,
    what: &str,
    can_start: impl Fn(&mut Parser) -> bool,
    mut item: impl FnMut(&mut Parser),
) {
    while !p.at(close) && !p.at_eof() {
        if can_start(p) {
            item(p);
        } else {
            let found = p.current();
            p.err_recover(
                format!("expected {what}, found {}", found.describe()),
                RECOVERY,
            );
            if p.at_set(RECOVERY) && !p.at(T![","]) {
                break;
            }
        }
        if !p.at(close) && !p.expect(T![","]) {
            // A missing comma: carry on if another element starts, else stop.
            if !can_start(p) {
                break;
            }
        }
    }
}

/// `'(' token_tree* ')'` and friends: anything balanced.
fn token_tree(p: &mut Parser) {
    let close = match p.current() {
        T!["("] => T![")"],
        T!["["] => T!["]"],
        T!["{"] => T!["}"],
        K::DOC_OUTER | K::DOC_INNER | T![_] | K::ERROR => {
            p.err_and_bump("expected a token tree");
            return;
        }
        _ => {
            p.bump_any();
            return;
        }
    };
    let m = p.start();
    p.bump_any();
    while !p.at(close) && !p.at_eof() {
        if p.at(T![")"]) || p.at(T!["]"]) || p.at(T!["}"]) {
            p.err_and_bump("unbalanced delimiter");
            continue;
        }
        token_tree(p);
    }
    p.expect(close);
    m.complete(p, K::TOKEN_TREE);
}

/// `simple_path ::= IDENT ( '::' IDENT )*`
fn simple_path(p: &mut Parser) {
    let m = p.start();
    let seg = p.start();
    p.expect(K::IDENT);
    seg.complete(p, K::PATH_SEGMENT);
    while p.at(T!["::"]) && p.nth_at(1, K::IDENT) {
        p.bump(T!["::"]);
        let seg = p.start();
        p.bump(K::IDENT);
        seg.complete(p, K::PATH_SEGMENT);
    }
    m.complete(p, K::PATH);
}
