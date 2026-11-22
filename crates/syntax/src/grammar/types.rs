//! Types, paths in types, generic arguments and bounds.

use super::exprs::{block, expr};
use super::items::{effect_atom, unit_suffix};
use super::{RECOVERY, at_member_name, at_record, comma_list, member_name};
use crate::SyntaxKind as K;
use crate::parser::{Parser, TokenSet};

const TYPE_START: TokenSet = TokenSet::new(&[
    T![impl],
    T![dyn],
    K::IDENT,
    T![Self],
    T![self],
    T![super],
    T![mod],
    T!["<"],
    T!["&"],
    T!["&&"],
    T!["*"],
    T!["["],
    T!["("],
    T!["{"],
    T![fn],
    T!["!"],
    T![_],
]);

pub(super) fn at_type(p: &mut Parser) -> bool {
    p.at_set(TYPE_START)
}

/// `type ::= type_ne type_effects?`
pub(crate) fn type_(p: &mut Parser) {
    let Some(t) = type_ne_inner(p, true) else {
        return;
    };
    if p.at(T![with]) {
        let m = t.precede(p);
        type_effects(p);
        m.complete(p, K::EFFECT_TYPE);
    }
}

/// `type_effects ::= 'with' ( effect_atom | '(' effect_atom ( ',' effect_atom )* ','? ')' )`
fn type_effects(p: &mut Parser) {
    let e = p.start();
    p.bump(T![with]);
    if p.eat(T!["("]) {
        comma_list(
            p,
            T![")"],
            "an effect",
            |p| {
                matches!(
                    p.current(),
                    T![suspend] | T![spawn] | T![nondet] | T![fail] | T!["&"] | T!["&&"]
                ) || at_path_type(p)
            },
            effect_atom,
        );
        p.expect(T![")"]);
    } else {
        effect_atom(p);
    }
    e.complete(p, K::TYPE_EFFECTS);
}

/// `type_ne ::= 'impl' bounds | 'dyn' bounds | type_core`: no trailing effects.
pub(super) fn type_ne(p: &mut Parser) {
    type_ne_inner(p, true);
}

/// `type_nb ::= 'impl' bound | 'dyn' bound | type_core`: no bounds list, no effects.
fn type_nb(p: &mut Parser) {
    type_ne_inner(p, false);
}

fn type_ne_inner(p: &mut Parser, bounds_list: bool) -> Option<crate::parser::CompletedMarker> {
    if p.at(T![impl]) || p.at(T![dyn]) {
        let kind = if p.at(T![impl]) {
            K::IMPL_TYPE
        } else {
            K::DYN_TYPE
        };
        let m = p.start();
        p.bump_any();
        if bounds_list {
            bounds(p);
        } else {
            bound(p, false);
        }
        return Some(m.complete(p, kind));
    }
    type_core(p)
}

/// `type_core`
fn type_core(p: &mut Parser) -> Option<crate::parser::CompletedMarker> {
    let m = p.start();
    let kind = match p.current() {
        K::IDENT | T![Self] | T![self] | T![super] | T![mod] | T!["<"] => {
            m.abandon(p);
            return Some(path_type(p));
        }
        T!["&"] | T!["&&"] => {
            // '&' 'mut'? type_nb; `&&` is two borrows
            p.bump_split(T!["&"]);
            if !p.at_split(T!["&"]) {
                p.eat(T![mut]);
            }
            type_nb(p);
            K::REF_TYPE
        }
        T!["*"] => {
            // '*' ( 'const' | 'mut' ) type_nb
            p.bump(T!["*"]);
            if !p.eat(T![const]) && !p.eat(T![mut]) {
                p.error("expected `const` or `mut` after `*`");
            }
            type_nb(p);
            K::PTR_TYPE
        }
        T!["["] => {
            // '[' type ( ';' expr )? ']'
            p.bump(T!["["]);
            type_(p);
            let kind = if p.eat(T![";"]) {
                expr(p);
                K::ARRAY_TYPE
            } else {
                K::SLICE_TYPE
            };
            p.expect(T!["]"]);
            kind
        }
        T!["("] => {
            // paren_type_rest ::= ')' | type ( ')' | ',' type_list? ')' | 'where' expr ')' )
            p.bump(T!["("]);
            if p.eat(T![")"]) {
                K::TUPLE_TYPE
            } else {
                type_(p);
                let kind = if p.eat(T![","]) {
                    comma_list(p, T![")"], "a type", at_type, type_);
                    K::TUPLE_TYPE
                } else if p.eat(T![where]) {
                    expr(p);
                    K::REFINED_TYPE
                } else {
                    K::PAREN_TYPE
                };
                p.expect(T![")"]);
                kind
            }
        }
        T!["{"] => {
            // '{' member_name ':' type ( ',' member_name ':' type )* ','? '}'
            p.bump(T!["{"]);
            let mut first = true;
            while first || (!p.at(T!["}"]) && at_member_name(p)) {
                first = false;
                let f = p.start();
                member_name(p);
                p.expect(T![":"]);
                type_(p);
                f.complete(p, K::RECORD_TYPE_FIELD);
                if !p.eat(T![","]) {
                    break;
                }
            }
            p.expect(T!["}"]);
            K::RECORD_TYPE
        }
        T![fn] => {
            // 'fn' '(' type_list? ')' ( '->' type_nb )?
            p.bump(T![fn]);
            type_list_parens(p);
            if p.eat(T!["->"]) {
                type_nb(p);
            }
            K::FN_TYPE
        }
        T!["!"] => {
            p.bump(T!["!"]);
            K::NEVER_TYPE
        }
        T![_] => {
            p.bump(T![_]);
            K::INFER_TYPE
        }
        found => {
            m.abandon(p);
            p.err_recover(
                format!("expected a type, found {}", found.describe()),
                RECOVERY,
            );
            return None;
        }
    };
    Some(m.complete(p, kind))
}

/// `'(' type_list? ')'`
fn type_list_parens(p: &mut Parser) {
    let m = p.start();
    if p.expect(T!["("]) {
        comma_list(p, T![")"], "a type", at_type, type_);
        p.expect(T![")"]);
    }
    m.complete(p, K::TYPE_LIST);
}

pub(super) fn at_path_type(p: &mut Parser) -> bool {
    matches!(
        p.current(),
        K::IDENT | T![Self] | T![self] | T![super] | T![mod] | T!["<"]
    )
}

/// `path_type ::= qualified_path ( '::' member_name )* | type_path`
pub(super) fn path_type(p: &mut Parser) -> crate::parser::CompletedMarker {
    let m = p.start();
    if p.at(T!["<"]) {
        let path = p.start();
        qualified_path(p);
        while p.eat(T!["::"]) {
            let seg = p.start();
            member_name(p);
            seg.complete(p, K::PATH_SEGMENT);
        }
        path.complete(p, K::PATH);
    } else {
        type_path_inner(p);
    }
    m.complete(p, K::PATH_TYPE)
}

/// `type_path ::= ( 'mod' '::' )? type_segment ( '::' type_segment )* unit_suffix?`, as a `PATH`
/// (plus the suffix).
pub(super) fn type_path(p: &mut Parser) {
    type_path_inner(p);
}

fn type_path_inner(p: &mut Parser) {
    let path = p.start();
    // `mod::`: the root module of the mod.
    if p.at(T![mod]) {
        let seg = p.start();
        p.bump(T![mod]);
        seg.complete(p, K::PATH_SEGMENT);
        p.expect(T!["::"]);
    }
    type_segment(p);
    while p.at(T!["::"]) {
        p.bump(T!["::"]);
        type_segment(p);
    }
    path.complete(p, K::PATH);
    if p.at(T!["["]) {
        unit_suffix(p);
    }
}

/// `type_segment ::= IDENT generic_args? | 'Self' | 'self' | 'super' | 'crate'`
fn type_segment(p: &mut Parser) {
    let m = p.start();
    match p.current() {
        K::IDENT => {
            p.bump(K::IDENT);
            if p.at(T!["<"]) {
                generic_args(p);
            }
        }
        T![Self] | T![self] | T![super] => p.bump_any(),
        found => p.err_recover(
            format!("expected a type name, found {}", found.describe()),
            RECOVERY,
        ),
    }
    m.complete(p, K::PATH_SEGMENT);
}

/// `qualified_path ::= '<' type 'as' type_path '>'`
pub(super) fn qualified_path(p: &mut Parser) {
    let m = p.start();
    p.bump(T!["<"]);
    type_(p);
    p.expect(T![as]);
    type_path(p);
    close_angle(p);
    m.complete(p, K::QUALIFIED_PATH);
}

/// A closing `>`, which may be the first character of `>>`, `>=` or `>>=`.
fn close_angle(p: &mut Parser) {
    if p.at_split(T![">"]) {
        p.bump_split(T![">"]);
    } else {
        let found = p.current();
        p.error(format!("expected `>`, found {}", found.describe()));
    }
}

/// `generic_args ::= '<' ( generic_arg ( ',' generic_arg )* ','? )? '>'`
pub(crate) fn generic_args(p: &mut Parser) {
    let m = p.start();
    p.bump(T!["<"]);
    while !p.at_split(T![">"]) && !p.at_eof() {
        // generic_arg ::= type | const_arg | IDENT '=' type
        if p.at(K::IDENT) && p.nth_at(1, T!["="]) {
            let a = p.start();
            p.bump(K::IDENT);
            p.bump(T!["="]);
            type_(p);
            a.complete(p, K::ASSOC_ARG);
        } else if matches!(
            p.current(),
            T!["-"] | K::INTEGER | K::STRING | T![true] | T![false]
        ) || (p.at(T!["{"]) && !at_record(p))
        {
            const_arg(p);
        } else if at_type(p) {
            let a = p.start();
            type_(p);
            a.complete(p, K::TYPE_ARG);
        } else {
            let found = p.current();
            p.err_recover(
                format!("expected a generic argument, found {}", found.describe()),
                RECOVERY,
            );
            break;
        }
        if !p.eat(T![","]) {
            break;
        }
    }
    close_angle(p);
    m.complete(p, K::GENERIC_ARGS);
}

/// `const_arg ::= '-'? INTEGER | STRING | 'true' | 'false' | block`
pub(crate) fn const_arg(p: &mut Parser) {
    let m = p.start();
    match p.current() {
        T!["-"] => {
            p.bump(T!["-"]);
            p.expect(K::INTEGER);
        }
        K::INTEGER | K::STRING | T![true] | T![false] => p.bump_any(),
        T!["{"] => {
            block(p);
        }
        found => p.err_recover(
            format!("expected a constant, found {}", found.describe()),
            RECOVERY,
        ),
    }
    m.complete(p, K::CONST_ARG);
}

/// `bounds ::= bound ( '+' bound )*`
pub(crate) fn bounds(p: &mut Parser) {
    bound(p, false);
    while p.eat(T!["+"]) {
        bound(p, false);
    }
}

/// `pred_bounds ::= pred_bound ( '+' pred_bound )*`: a parameter's or a `where` clause's, whose
/// closure bounds may name their effects.
pub(crate) fn pred_bounds(p: &mut Parser) {
    bound(p, true);
    while p.eat(T!["+"]) {
        bound(p, true);
    }
}

/// `bound ::= type_path fn_sugar?` with `fn_sugar ::= '(' type_list? ')' ( '->' type_nb )?`, and
/// `pred_bound ::= type_path ( fn_sugar type_effects? )?` when `effects`.
fn bound(p: &mut Parser, effects: bool) {
    let m = p.start();
    if matches!(
        p.current(),
        K::IDENT | T![Self] | T![self] | T![super] | T![mod]
    ) {
        type_path(p);
    } else {
        let found = p.current();
        p.err_recover(
            format!("expected a trait bound, found {}", found.describe()),
            RECOVERY,
        );
    }
    if p.at(T!["("]) {
        let s = p.start();
        type_list_parens(p);
        if p.eat(T!["->"]) {
            type_nb(p);
        }
        // pred_effects: as type_effects, but a bare atom has no `&T`, whose type would take
        // the next `+`.
        if effects && p.at(T![with]) {
            if matches!(p.nth(1), T!["&"] | T!["&&"]) {
                p.error("context data in a bound's effects is parenthesised: `with (&T)`");
            }
            type_effects(p);
        }
        s.complete(p, K::FN_SUGAR);
    }
    m.complete(p, K::BOUND);
}

/// `exponent ::= '-'? INTEGER | '(' '-'? INTEGER ( '/' INTEGER )? ')'`
pub(crate) fn exponent(p: &mut Parser) {
    let m = p.start();
    if p.eat(T!["("]) {
        p.eat(T!["-"]);
        p.expect(K::INTEGER);
        if p.eat(T!["/"]) {
            p.expect(K::INTEGER);
        }
        p.expect(T![")"]);
    } else {
        p.eat(T!["-"]);
        p.expect(K::INTEGER);
    }
    m.complete(p, K::EXPONENT);
}
