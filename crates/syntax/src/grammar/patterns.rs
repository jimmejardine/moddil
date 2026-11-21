//! Patterns.

use super::exprs::expr_path_after_start;
use super::types::{generic_args, qualified_path, type_};
use super::{RECOVERY, at_member_name, comma_list, member_name};
use crate::SyntaxKind as K;
use crate::parser::{CompletedMarker, Parser, TokenSet};

const PATTERN_START: TokenSet = TokenSet::new(&[
    T![_],
    T![".."],
    T!["-"],
    K::INTEGER,
    K::FLOAT,
    K::STRING,
    K::CHAR,
    K::BYTE,
    K::BYTE_STRING,
    T![true],
    T![false],
    T![mut],
    K::IDENT,
    T![Self],
    T![super],
    T![mod],
    T!["<"],
    T!["&"],
    T!["&&"],
    T!["("],
    T!["["],
    T!["{"],
]);

pub(super) fn at_pattern(p: &mut Parser) -> bool {
    p.at_set(PATTERN_START)
}

/// `pattern ::= pattern_no_alt ( '|' pattern_no_alt )*`
pub(super) fn pattern(p: &mut Parser) {
    let Some(first) = pattern_no_alt(p) else {
        return;
    };
    if p.at(T!["|"]) {
        let m = first.precede(p);
        while p.eat(T!["|"]) {
            pattern_no_alt(p);
        }
        m.complete(p, K::OR_PAT);
    }
}

/// `arm_pattern ::= pattern ( ':' type )?`
pub(super) fn arm_pattern(p: &mut Parser) {
    let m = p.start();
    pattern(p);
    if p.eat(T![":"]) {
        type_(p);
        m.complete(p, K::TYPED_PAT);
    } else {
        m.abandon(p);
    }
}

fn at_literal_pat(p: &mut Parser) -> bool {
    match p.current() {
        T!["-"] => matches!(p.nth(1), K::INTEGER | K::FLOAT),
        K::INTEGER
        | K::FLOAT
        | K::STRING
        | K::CHAR
        | K::BYTE
        | K::BYTE_STRING
        | T![true]
        | T![false] => true,
        _ => false,
    }
}

/// `literal_pat ::= '-'? number | STRING | CHAR | BYTE | BYTE_STRING | 'true' | 'false'`
fn literal_pat(p: &mut Parser) -> CompletedMarker {
    let m = p.start();
    if p.eat(T!["-"]) {
        if !p.eat(K::INTEGER) && !p.eat(K::FLOAT) {
            p.error("expected a number");
        }
    } else {
        p.bump_any();
    }
    m.complete(p, K::LITERAL_PAT)
}

/// `pattern_no_alt`
pub(super) fn pattern_no_alt(p: &mut Parser) -> Option<CompletedMarker> {
    let m = p.start();
    let kind = match p.current() {
        T![_] => {
            p.bump(T![_]);
            K::WILDCARD_PAT
        }
        T![".."] => {
            p.bump(T![".."]);
            K::REST_PAT
        }
        _ if at_literal_pat(p) => {
            // literal_pat ( range_op literal_pat? )?
            m.abandon(p);
            let lit = literal_pat(p);
            if p.at(T![".."]) || p.at(T!["..="]) {
                let r = lit.precede(p);
                p.bump_any();
                if at_literal_pat(p) {
                    literal_pat(p);
                }
                return Some(r.complete(p, K::RANGE_PAT));
            }
            return Some(lit);
        }
        T![mut] => {
            // 'mut' IDENT ( '@' pattern_no_alt )?
            p.bump(T![mut]);
            super::name(p, false);
            if p.eat(T!["@"]) {
                pattern_no_alt(p);
            }
            K::IDENT_PAT
        }
        K::IDENT | T![Self] | T![super] | T![mod] | T!["<"] => {
            // pattern_path path_pat_tail?
            pattern_path(p);
            match p.current() {
                T!["("] => {
                    pattern_list(p, T!["("], T![")"]);
                    K::TUPLE_STRUCT_PAT
                }
                T!["{"] => {
                    field_pats(p);
                    K::STRUCT_PAT
                }
                T!["@"] => {
                    p.bump(T!["@"]);
                    pattern_no_alt(p);
                    K::AT_PAT
                }
                _ => K::PATH_PAT,
            }
        }
        T!["&"] | T!["&&"] => {
            // '&' 'mut'?? pattern_no_alt; `&&` is two borrows
            p.bump_split(T!["&"]);
            if !p.at_split(T!["&"]) {
                p.eat(T![mut]);
            }
            pattern_no_alt(p);
            K::REF_PAT
        }
        T!["("] => {
            pattern_list(p, T!["("], T![")"]);
            K::TUPLE_PAT
        }
        T!["["] => {
            pattern_list(p, T!["["], T!["]"]);
            K::SLICE_PAT
        }
        T!["{"] => {
            field_pats(p);
            K::RECORD_PAT
        }
        found => {
            m.abandon(p);
            p.err_recover(
                format!("expected a pattern, found {}", found.describe()),
                RECOVERY,
            );
            return None;
        }
    };
    Some(m.complete(p, kind))
}

/// `pattern_path ::= ( qualified_path | IDENT | 'Self' | 'super' | 'crate' )
///                   ( '::' ( member_name | generic_args ) )*`
fn pattern_path(p: &mut Parser) {
    let m = p.start();
    if p.at(T!["<"]) {
        qualified_path(p);
    } else {
        let seg = p.start();
        p.bump_any();
        seg.complete(p, K::PATH_SEGMENT);
    }
    expr_path_after_start(p, |p| {
        if p.at(T!["<"]) {
            generic_args(p);
        } else {
            let seg = p.start();
            member_name(p);
            seg.complete(p, K::PATH_SEGMENT);
        }
    });
    m.complete(p, K::PATH);
}

/// `'(' ( pattern ( ',' pattern )* ','? )? ')'`, and with brackets.
fn pattern_list(p: &mut Parser, open: K, close: K) {
    p.bump(open);
    comma_list(p, close, "a pattern", at_pattern, pattern);
    p.expect(close);
}

/// `'{' field_pats '}'` with `field_pats ::= ( field_pat ( ',' field_pat )* ','? )?` and
/// `field_pat ::= member_name ( ':' pattern )? | '..'`
fn field_pats(p: &mut Parser) {
    p.bump(T!["{"]);
    comma_list(
        p,
        T!["}"],
        "a field pattern",
        |p| p.at(T![".."]) || at_member_name(p),
        |p| {
            let f = p.start();
            if !p.eat(T![".."]) {
                member_name(p);
                if p.eat(T![":"]) {
                    pattern(p);
                }
            }
            f.complete(p, K::FIELD_PAT);
        },
    );
    p.expect(T!["}"]);
}
