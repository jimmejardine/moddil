//! Statements, blocks and expressions, lowest precedence first.

use super::items::{
    LOCAL_ITEM_START, at_outer_attr, impl_body, inner_attrs, local_item, outer_attrs, unit_suffix,
};
use super::patterns::{arm_pattern, pattern, pattern_no_alt};
use super::types::{generic_args, path_type, qualified_path, type_, type_path};
use super::{R, RECOVERY, at_member_name, at_record, comma_list, member_name};
use crate::SyntaxKind as K;
use crate::parser::{CompletedMarker, Parser, TokenSet};

// ─── Blocks and statements ──────────────────────────────────────────────────

/// `block ::= '{' inner_attr* statements '}'`
pub(crate) fn block(p: &mut Parser) -> CompletedMarker {
    let m = p.start();
    if p.expect(T!["{"]) {
        inner_attrs(p);
        statements(p);
        p.expect(T!["}"]);
    }
    m.complete(p, K::BLOCK)
}

fn at_local_item(p: &mut Parser) -> bool {
    match p.current() {
        // `const {`, `unsafe (` and `gen {` start block-like expressions instead.
        T![const] | T![gen] => !p.nth_at(1, T!["{"]),
        T![unsafe] => !p.nth_at(1, T!["("]),
        k => LOCAL_ITEM_START.contains(k),
    }
}

/// A block-like expression in statement position (`block_like`).
fn at_block_like(p: &mut Parser) -> bool {
    match p.current() {
        T!["{"] => !at_record(p),
        T![const] | T![gen] => p.nth_at(1, T!["{"]),
        T![unsafe] => p.nth_at(1, T!["("]),
        T![if]
        | T![match]
        | T![loop]
        | T![while]
        | T![for]
        | K::LABEL
        | T![try]
        | T![with]
        | T![select]
        | T![inline] => true,
        _ => false,
    }
}

/// `statements ::= ( statement statements | expr+stmt ( ';' statements )? )?`
fn statements(p: &mut Parser) {
    while !p.at(T!["}"]) && !p.at_eof() {
        let before = p.position();
        if at_outer_attr(p)
            || p.at(T![let])
            || p.at(T![defer])
            || p.at(T![errdefer])
            || at_local_item(p)
            || at_block_like(p)
        {
            statement(p);
        } else if can_start_expr(p, R::STMT) {
            let m = p.start();
            expr_r(p, R::STMT);
            if p.eat(T![";"]) {
                m.complete(p, K::EXPR_STMT);
            } else if p.at(T!["}"]) || p.at_eof() {
                m.abandon(p); // the block's value
            } else {
                let found = p.current();
                p.error(format!("expected `;` or `}}`, found {}", found.describe()));
                m.complete(p, K::EXPR_STMT);
            }
        } else {
            let found = p.current();
            p.err_and_bump(format!("expected a statement, found {}", found.describe()));
        }
        if p.position() == before {
            p.err_and_bump("expected a statement");
        }
    }
}

/// `statement ::= outer_attr* ( let_stmt | local_item | defer_stmt | block_like ';'? )`
fn statement(p: &mut Parser) {
    let m = p.start();
    let attrs = at_outer_attr(p);
    outer_attrs(p);
    match p.current() {
        T![let] => {
            // let_stmt ::= 'let' pattern ( ':' type )? ( '=' expr ( 'else' block )? )? ';'
            p.bump(T![let]);
            pattern(p);
            if p.eat(T![":"]) {
                type_(p);
            }
            if p.eat(T!["="]) {
                expr(p);
                if p.at(T![else]) {
                    let e = p.start();
                    p.bump(T![else]);
                    block(p);
                    e.complete(p, K::LET_ELSE);
                }
            }
            p.expect(T![";"]);
            m.complete(p, K::LET_STMT);
        }
        T![defer] | T![errdefer] => {
            // defer_stmt ::= ( 'defer' | 'errdefer' ) ( block | expr+stmt ';' )
            p.bump_any();
            if p.at(T!["{"]) && !at_record(p) {
                block(p);
            } else {
                expr_r(p, R::STMT);
                p.expect(T![";"]);
            }
            m.complete(p, K::DEFER_STMT);
        }
        _ if at_local_item(p) => {
            local_item(p, m);
        }
        _ if at_block_like(p) => {
            block_like(p);
            if p.eat(T![";"]) || attrs {
                m.complete(p, K::EXPR_STMT);
            } else {
                m.abandon(p);
            }
        }
        found => {
            p.error(format!(
                "expected `let`, an item or a block-like expression after attributes, found {}",
                found.describe()
            ));
            m.complete(p, K::ERROR);
        }
    }
}

/// `block_like ::= block | 'const' block | block_like_keyword`
fn block_like(p: &mut Parser) -> CompletedMarker {
    match p.current() {
        T!["{"] => block(p),
        T![const] => const_block(p),
        _ => block_like_keyword(p),
    }
}

fn const_block(p: &mut Parser) -> CompletedMarker {
    let m = p.start();
    p.bump(T![const]);
    block(p);
    m.complete(p, K::CONST_BLOCK)
}

// ─── Expressions ────────────────────────────────────────────────────────────

const JUMP: TokenSet = TokenSet::new(&[T![escape], T![yield], T![return], T![break], T![continue]]);
const ASSIGN_OPS: TokenSet = TokenSet::new(&[
    T!["="],
    T!["+="],
    T!["-="],
    T!["*="],
    T!["/="],
    T!["%="],
    T!["^="],
    T!["&="],
    T!["|="],
    T!["<<="],
    T![">>="],
    T!["+%="],
    T!["-%="],
    T!["*%="],
]);
const COMPARE_OPS: TokenSet =
    TokenSet::new(&[T!["=="], T!["!="], T!["<"], T![">"], T!["<="], T![">="]]);
const RANGE_OPS: TokenSet = TokenSet::new(&[T![".."], T!["..="]]);
const LITERALS: TokenSet = super::LITERAL_TOKENS.union(TokenSet::new(&[T![true], T![false]]));
const BLOCK_LIKE_KEYWORDS: TokenSet = TokenSet::new(&[
    T![if],
    T![match],
    T![loop],
    T![while],
    T![for],
    K::LABEL,
    T![try],
    T![with],
    T![select],
    T![unsafe],
    T![gen],
    T![inline],
]);

/// Whether an expression (`expr`) can start here under flags `r`.
pub(super) fn can_start_expr(p: &mut Parser, r: R) -> bool {
    p.at_set(JUMP) || p.at(T!["|"]) || p.at(T!["||"]) || p.at_set(RANGE_OPS) || can_start_or(p, r)
}

/// `or` (and every level down to `and_operand`): a `let` condition under `cond`.
fn can_start_or(p: &mut Parser, r: R) -> bool {
    (r.cond && p.at(T![let])) || can_start_unary(p, r)
}

fn can_start_unary(p: &mut Parser, r: R) -> bool {
    matches!(
        p.current(),
        T!["-"] | T!["!"] | T!["*"] | T!["&"] | T!["&&"] | T!["\\"]
    ) || can_start_primary(p, r)
}

fn can_start_primary(p: &mut Parser, r: R) -> bool {
    match p.current() {
        k if LITERALS.contains(k) => true,
        K::IDENT
        | T![self]
        | T![Self]
        | T![super]
        | T![mod]
        | T!["<"]
        | T!["("]
        | T!["["]
        | T![fail] => true,
        T!["{"] => !r.ns && (at_record(p) || !r.stmt),
        T![const] => !r.ns && !r.stmt && p.nth_at(1, T!["{"]),
        T![impl] => !r.stmt,
        k => !r.stmt && BLOCK_LIKE_KEYWORDS.contains(k),
    }
}

/// `expr`, with no flags.
pub(crate) fn expr(p: &mut Parser) {
    expr_r(p, R::NONE);
}

/// `expr ::= jump@ns | closure@ns | assign@`
pub(super) fn expr_r(p: &mut Parser, r: R) -> Option<CompletedMarker> {
    if p.at_set(JUMP) {
        Some(jump(p, r.operand()))
    } else if p.at(T!["|"]) || p.at(T!["||"]) {
        Some(closure(p, r.operand()))
    } else {
        assign(p, r)
    }
}

/// `jump`
fn jump(p: &mut Parser, r: R) -> CompletedMarker {
    let m = p.start();
    let kind = match p.current() {
        T![escape] => K::ESCAPE_EXPR,
        T![yield] => K::YIELD_EXPR,
        T![return] => K::RETURN_EXPR,
        T![break] => K::BREAK_EXPR,
        _ => K::CONTINUE_EXPR,
    };
    p.bump_any();
    if matches!(kind, K::BREAK_EXPR | K::CONTINUE_EXPR) && p.at(K::LABEL) && !p.nth_at(1, T![":"]) {
        p.bump(K::LABEL);
    }
    if kind != K::CONTINUE_EXPR && can_start_expr(p, r) {
        expr_r(p, r);
    }
    m.complete(p, kind)
}

/// `closure ::= ( '||' | '|' ( closure_param ( ',' closure_param )* ','? )? '|' )
///              ( '->' type block | expr@ns )`
fn closure(p: &mut Parser, r: R) -> CompletedMarker {
    let m = p.start();
    let params = p.start();
    if !p.eat(T!["||"]) {
        p.bump(T!["|"]);
        comma_list(
            p,
            T!["|"],
            "a closure parameter",
            super::patterns::at_pattern,
            |p| {
                // closure_param ::= pattern_no_alt ( ':' type )?
                let c = p.start();
                pattern_no_alt(p);
                if p.eat(T![":"]) {
                    type_(p);
                }
                c.complete(p, K::CLOSURE_PARAM);
            },
        );
        p.expect(T!["|"]);
    }
    params.complete(p, K::CLOSURE_PARAMS);
    if p.at(T!["->"]) {
        let ret = p.start();
        p.bump(T!["->"]);
        type_(p);
        ret.complete(p, K::RET_TYPE);
        block(p);
    } else if can_start_expr(p, r) {
        expr_r(p, r);
    } else {
        let found = p.current();
        p.err_recover(
            format!("expected the closure's body, found {}", found.describe()),
            RECOVERY,
        );
    }
    m.complete(p, K::CLOSURE_EXPR)
}

/// `assign ::= range@ ( assign_op expr@ns )?`
fn assign(p: &mut Parser, r: R) -> Option<CompletedMarker> {
    let lhs = range(p, r)?;
    if p.at_set(ASSIGN_OPS) {
        let m = lhs.precede(p);
        p.bump_any();
        expr_r(p, r.operand());
        return Some(m.complete(p, K::ASSIGN_EXPR));
    }
    Some(lhs)
}

/// `range ::= or@ ( range_op or@ns? )? | range_op or@ns?`
fn range(p: &mut Parser, r: R) -> Option<CompletedMarker> {
    range_of(p, r, or, can_start_or)
}

/// A range over `operand`: `range` (over `or`) and `in_operand` (over `bit_or`).
fn range_of(
    p: &mut Parser,
    r: R,
    operand: fn(&mut Parser, R) -> Option<CompletedMarker>,
    can_start: fn(&mut Parser, R) -> bool,
) -> Option<CompletedMarker> {
    if p.at_set(RANGE_OPS) {
        let m = p.start();
        p.bump_any();
        if can_start(p, r.operand()) {
            operand(p, r.operand());
        }
        return Some(m.complete(p, K::RANGE_EXPR));
    }
    let lhs = operand(p, r)?;
    if p.at_set(RANGE_OPS) {
        let m = lhs.precede(p);
        p.bump_any();
        if can_start(p, r.operand()) {
            operand(p, r.operand());
        }
        return Some(m.complete(p, K::RANGE_EXPR));
    }
    Some(lhs)
}

/// A left-associative binary level: `next@ ( op next@ns )*`.
fn binary(
    p: &mut Parser,
    r: R,
    ops: TokenSet,
    next: fn(&mut Parser, R) -> Option<CompletedMarker>,
) -> Option<CompletedMarker> {
    let mut lhs = next(p, r)?;
    while p.at_set(ops) {
        let m = lhs.precede(p);
        p.bump_any();
        next(p, r.operand());
        lhs = m.complete(p, K::BIN_EXPR);
    }
    Some(lhs)
}

/// `or ::= and@ ( '||' and@ns )*`
fn or(p: &mut Parser, r: R) -> Option<CompletedMarker> {
    binary(p, r, TokenSet::new(&[T!["||"]]), and)
}

/// `and ::= and_operand@ ( '&&' and_operand@ns )*`
fn and(p: &mut Parser, r: R) -> Option<CompletedMarker> {
    binary(p, r, TokenSet::new(&[T!["&&"]]), and_operand)
}

/// `and_operand ::= ?cond 'let' pattern '=' compare@ns | compare@`
fn and_operand(p: &mut Parser, r: R) -> Option<CompletedMarker> {
    if r.cond && p.at(T![let]) {
        let m = p.start();
        p.bump(T![let]);
        pattern(p);
        p.expect(T!["="]);
        compare(p, r.operand());
        return Some(m.complete(p, K::LET_COND));
    }
    compare(p, r)
}

/// `compare ::= bit_or@ ( ( compare_op bit_or@ns )+ | 'in' in_operand@ns )?`
fn compare(p: &mut Parser, r: R) -> Option<CompletedMarker> {
    let lhs = bit_or(p, r)?;
    if p.at_set(COMPARE_OPS) {
        let m = lhs.precede(p);
        let mut ops = 0;
        while p.at_set(COMPARE_OPS) {
            p.bump_any();
            bit_or(p, r.operand());
            ops += 1;
        }
        return Some(m.complete(
            p,
            if ops == 1 {
                K::BIN_EXPR
            } else {
                K::COMPARE_CHAIN
            },
        ));
    }
    if p.at(T![in]) {
        let m = lhs.precede(p);
        p.bump(T![in]);
        // in_operand ::= bit_or@ ( range_op bit_or@ns? )?? | range_op bit_or@ns?
        range_of(p, r.operand(), bit_or, can_start_unary);
        return Some(m.complete(p, K::IN_EXPR));
    }
    Some(lhs)
}

fn bit_or(p: &mut Parser, r: R) -> Option<CompletedMarker> {
    binary(p, r, TokenSet::new(&[T!["|"]]), bit_xor)
}

fn bit_xor(p: &mut Parser, r: R) -> Option<CompletedMarker> {
    binary(p, r, TokenSet::new(&[T!["^"]]), bit_and)
}

fn bit_and(p: &mut Parser, r: R) -> Option<CompletedMarker> {
    binary(p, r, TokenSet::new(&[T!["&"]]), shift)
}

fn shift(p: &mut Parser, r: R) -> Option<CompletedMarker> {
    binary(p, r, TokenSet::new(&[T!["<<"], T![">>"]]), additive)
}

fn additive(p: &mut Parser, r: R) -> Option<CompletedMarker> {
    binary(
        p,
        r,
        TokenSet::new(&[T!["+"], T!["-"], T!["+%"], T!["-%"]]),
        multiplicative,
    )
}

fn multiplicative(p: &mut Parser, r: R) -> Option<CompletedMarker> {
    binary(
        p,
        r,
        TokenSet::new(&[T!["*"], T!["/"], T!["%"], T!["*%"], T!["@"]]),
        unary,
    )
}

/// `unary ::= ( '-' | '!' | '*' | '&' 'mut'? ) unary@ns | '\' type_path ( '.' member_name )+ | postfix@`
fn unary(p: &mut Parser, r: R) -> Option<CompletedMarker> {
    match p.current() {
        T!["-"] | T!["!"] | T!["*"] => {
            let m = p.start();
            p.bump_any();
            unary(p, r.operand());
            Some(m.complete(p, K::PREFIX_EXPR))
        }
        T!["&"] | T!["&&"] => {
            // `&&` in prefix position is two borrows.
            let m = p.start();
            p.bump_split(T!["&"]);
            if !p.at_split(T!["&"]) {
                p.eat(T![mut]);
            }
            unary(p, r.operand());
            Some(m.complete(p, K::REF_EXPR))
        }
        T!["\\"] => {
            let m = p.start();
            p.bump(T!["\\"]);
            type_path(p);
            if !p.at(T!["."]) {
                p.error("expected `.` and a field name after the key path's type");
            }
            while p.eat(T!["."]) {
                member_name(p);
            }
            Some(m.complete(p, K::KEY_PATH))
        }
        _ => postfix(p, r),
    }
}

/// `postfix ::= primary@ postfix_op*`
fn postfix(p: &mut Parser, r: R) -> Option<CompletedMarker> {
    let mut lhs = primary(p, r)?;
    loop {
        lhs = match p.current() {
            T!["?"] => {
                // '?' error_context?
                let m = lhs.precede(p);
                p.bump(T!["?"]);
                if p.at(K::STRING) || p.at(K::TAGGED_STRING) {
                    let c = p.start();
                    p.bump_any();
                    c.complete(p, K::ERROR_CONTEXT);
                }
                m.complete(p, K::PROPAGATE_EXPR)
            }
            T!["?!"] => {
                let m = lhs.precede(p);
                p.bump(T!["?!"]);
                if !p.eat(K::STRING) {
                    p.error("expected the reason it cannot fail: a string");
                }
                m.complete(p, K::UNWRAP_EXPR)
            }
            T!["("] => {
                let m = lhs.precede(p);
                call_args(p);
                m.complete(p, K::CALL_EXPR)
            }
            T!["."] if p.nth_at(1, T!["["]) => {
                // '.' '[' expr ']': a reflected field
                let m = lhs.precede(p);
                p.bump(T!["."]);
                p.bump(T!["["]);
                expr(p);
                p.expect(T!["]"]);
                m.complete(p, K::REFLECTED_FIELD_EXPR)
            }
            T!["."] => {
                let m = lhs.precede(p);
                p.bump(T!["."]);
                if p.eat(K::INTEGER) {
                    m.complete(p, K::FIELD_EXPR)
                } else {
                    member_name(p);
                    if p.at(T!["::"]) {
                        p.bump(T!["::"]);
                        if p.at(T!["<"]) {
                            generic_args(p);
                        } else {
                            p.error("expected `<` after `::`");
                        }
                    }
                    if p.at(T!["("]) {
                        call_args(p);
                        m.complete(p, K::METHOD_CALL_EXPR)
                    } else {
                        m.complete(p, K::FIELD_EXPR)
                    }
                }
            }
            T!["["] => {
                let m = lhs.precede(p);
                p.bump(T!["["]);
                if p.at(T!["]"]) {
                    p.error("expected an index");
                }
                comma_list(p, T!["]"], "an index", |p| can_start_expr(p, R::NONE), expr);
                p.expect(T!["]"]);
                m.complete(p, K::INDEX_EXPR)
            }
            _ => return Some(lhs),
        };
    }
}

/// `call_args ::= '(' ( argument ( ',' argument )* ','? )? ')'` with
/// `argument ::= ( member_name ':' )? expr`
fn call_args(p: &mut Parser) {
    let m = p.start();
    p.bump(T!["("]);
    comma_list(
        p,
        T![")"],
        "an argument",
        |p| can_start_expr(p, R::NONE) || (at_member_name(p) && p.nth_at(1, T![":"])),
        |p| {
            let a = p.start();
            if at_member_name(p) && p.nth_at(1, T![":"]) {
                member_name(p);
                p.bump(T![":"]);
            }
            expr(p);
            a.complete(p, K::ARG);
        },
    );
    p.expect(T![")"]);
    m.complete(p, K::ARG_LIST);
}

// ─── Primaries ──────────────────────────────────────────────────────────────

/// `literal ::= number unit_suffix?? | STRING | RAW_STRING | BYTE_STRING | BLOCK_STRING
///            | TAGGED_STRING | CHAR | BYTE | 'true' | 'false'`
pub(super) fn literal(p: &mut Parser) -> Option<CompletedMarker> {
    let m = p.start();
    let k = p.current();
    if !LITERALS.contains(k) {
        m.abandon(p);
        p.err_recover(
            format!("expected a literal, found {}", k.describe()),
            RECOVERY,
        );
        return None;
    }
    p.bump_any();
    if matches!(k, K::INTEGER | K::FLOAT) && at_unit_suffix(p) {
        unit_suffix(p);
    }
    Some(m.complete(p, K::LITERAL))
}

/// `number unit_suffix??`: the greedy suffix is taken when a unit can follow `[` within three
/// tokens (`1[m]`, `2[m/s]`, `3[(m)]`); otherwise `[` starts an index (`1[x, y]`).
fn at_unit_suffix(p: &mut Parser) -> bool {
    p.at(T!["["])
        && match p.nth(1) {
            K::IDENT => matches!(p.nth(2), T!["]"] | T!["*"] | T!["/"] | T!["^"] | T!["::"]),
            T!["("] => matches!(p.nth(2), K::IDENT | T!["("]),
            _ => false,
        }
}

pub(super) fn at_expr_path(p: &mut Parser) -> bool {
    matches!(
        p.current(),
        K::IDENT | T![self] | T![Self] | T![super] | T![mod] | T!["<"]
    )
}

/// `expr_path ::= ( qualified_path | path_start ) ( '::' ( member_name | generic_args ) )*`
pub(super) fn expr_path(p: &mut Parser) {
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

/// `( '::' segment )*`
pub(super) fn expr_path_after_start(p: &mut Parser, mut segment: impl FnMut(&mut Parser)) {
    while p.at(T!["::"]) {
        p.bump(T!["::"]);
        segment(p);
    }
}

/// `primary`
fn primary(p: &mut Parser, r: R) -> Option<CompletedMarker> {
    let k = p.current();
    if LITERALS.contains(k) {
        return literal(p);
    }
    match k {
        K::IDENT | T![self] | T![Self] | T![super] | T![mod] | T!["<"] => {
            // expr_path struct_tail@?
            let m = p.start();
            expr_path(p);
            if !r.ns && p.at(T!["{"]) {
                struct_tail(p);
                Some(m.complete(p, K::STRUCT_LIT))
            } else {
                Some(m.complete(p, K::PATH_EXPR))
            }
        }
        T!["("] => {
            // '(' ( expr ( ',' ( expr ( ',' expr )* ','? )? )? )? ')'
            let m = p.start();
            p.bump(T!["("]);
            let kind = if p.at(T![")"]) {
                K::TUPLE_EXPR
            } else {
                expr(p);
                if p.eat(T![","]) {
                    comma_list(
                        p,
                        T![")"],
                        "an expression",
                        |p| can_start_expr(p, R::NONE),
                        expr,
                    );
                    K::TUPLE_EXPR
                } else {
                    K::PAREN_EXPR
                }
            };
            p.expect(T![")"]);
            Some(m.complete(p, kind))
        }
        T!["["] => {
            // '[' ( expr ( ( ',' expr )* ','? | ';' expr ) )? ']'
            let m = p.start();
            p.bump(T!["["]);
            if !p.at(T!["]"]) {
                expr(p);
                if p.eat(T![";"]) {
                    expr(p);
                } else if p.eat(T![","]) {
                    comma_list(
                        p,
                        T!["]"],
                        "an expression",
                        |p| can_start_expr(p, R::NONE),
                        expr,
                    );
                }
            }
            p.expect(T!["]"]);
            Some(m.complete(p, K::ARRAY_EXPR))
        }
        T!["{"] if !r.ns && at_record(p) => {
            // '{' member_name ':' expr ( ',' member_name ':' expr )* ','? '}'
            let m = p.start();
            p.bump(T!["{"]);
            let mut first = true;
            while first || at_member_name(p) {
                first = false;
                let f = p.start();
                member_name(p);
                p.expect(T![":"]);
                expr(p);
                f.complete(p, K::RECORD_FIELD_INIT);
                if !p.eat(T![","]) {
                    break;
                }
            }
            p.expect(T!["}"]);
            Some(m.complete(p, K::RECORD_EXPR))
        }
        T!["{"] if !r.ns && !r.stmt => Some(block(p)),
        T![const] if !r.ns && !r.stmt && p.nth_at(1, T!["{"]) => Some(const_block(p)),
        T![impl] if !r.stmt => {
            // 'impl' type impl_body: an anonymous impl
            let m = p.start();
            p.bump(T![impl]);
            type_(p);
            impl_body(p);
            Some(m.complete(p, K::IMPL_EXPR))
        }
        T![fail] => {
            let m = p.start();
            p.bump(T![fail]);
            p.expect(T!["("]);
            expr(p);
            p.expect(T![")"]);
            Some(m.complete(p, K::FAIL_EXPR))
        }
        k if !r.stmt && BLOCK_LIKE_KEYWORDS.contains(k) => Some(block_like_keyword(p)),
        found => {
            p.err_recover(
                format!("expected an expression, found {}", found.describe()),
                RECOVERY,
            );
            None
        }
    }
}

/// `struct_tail ::= '{' struct_fields '}'`
fn struct_tail(p: &mut Parser) {
    p.bump(T!["{"]);
    struct_fields(p);
    p.expect(T!["}"]);
}

/// `struct_fields ::= ( field_init ( ',' field_init )* ','? )?` with
/// `field_init ::= member_name ( ':' expr )? | '..' expr | '[' expr ']' ':' expr
///               | 'inline' 'for' pattern 'in' expr+ns '{' struct_fields '}'`
fn struct_fields(p: &mut Parser) {
    comma_list(
        p,
        T!["}"],
        "a field",
        |p| p.at(T![".."]) || p.at(T!["["]) || p.at(T![inline]) || at_member_name(p),
        |p| {
            let f = p.start();
            if p.eat(T![".."]) {
                expr(p);
                f.complete(p, K::SPREAD);
            } else if p.eat(T!["["]) {
                expr(p);
                p.expect(T!["]"]);
                p.expect(T![":"]);
                expr(p);
                f.complete(p, K::REFLECTED_FIELD_INIT);
            } else if p.at(T![inline]) && p.nth(1) == T![for] {
                // `inline` is a field's name too (`member_name`), unless `for` follows.
                p.bump(T![inline]);
                p.bump(T![for]);
                pattern(p);
                p.expect(T![in]);
                expr_r(p, R::NS);
                p.expect(T!["{"]);
                struct_fields(p);
                p.expect(T!["}"]);
                f.complete(p, K::INLINE_FOR_FIELDS);
            } else {
                member_name(p);
                if p.eat(T![":"]) {
                    expr(p);
                }
                f.complete(p, K::FIELD_INIT);
            }
        },
    );
}

// ─── Block-like expressions ─────────────────────────────────────────────────

/// `block_like_keyword`
fn block_like_keyword(p: &mut Parser) -> CompletedMarker {
    match p.current() {
        T![if] => if_expr(p),
        T![match] => {
            // match_expr ::= 'match' expr+ns '{' inner_attr* arms '}'
            let m = p.start();
            p.bump(T![match]);
            expr_r(p, R::NS);
            arm_list(p, false);
            m.complete(p, K::MATCH_EXPR)
        }
        T![loop] => {
            let m = p.start();
            p.bump(T![loop]);
            block(p);
            m.complete(p, K::LOOP_EXPR)
        }
        T![while] => {
            let m = p.start();
            p.bump(T![while]);
            expr_r(p, R::NS_COND);
            block(p);
            m.complete(p, K::WHILE_EXPR)
        }
        T![for] => {
            // for_expr ::= 'for' pattern 'in' expr+ns block
            let m = p.start();
            p.bump(T![for]);
            pattern(p);
            p.expect(T![in]);
            expr_r(p, R::NS);
            block(p);
            m.complete(p, K::FOR_EXPR)
        }
        T![inline] => {
            // inline_for ::= 'inline' 'for' pattern 'in' expr+ns block
            let m = p.start();
            p.bump(T![inline]);
            p.expect(T![for]);
            pattern(p);
            p.expect(T![in]);
            expr_r(p, R::NS);
            block(p);
            m.complete(p, K::INLINE_FOR)
        }
        K::LABEL => {
            // labelled ::= LABEL ':' ( loop_expr | while_expr | for_expr | block )
            let m = p.start();
            p.bump(K::LABEL);
            p.expect(T![":"]);
            match p.current() {
                T![loop] | T![while] | T![for] => {
                    block_like_keyword(p);
                }
                T!["{"] => {
                    block(p);
                }
                found => p.err_recover(
                    format!(
                        "expected `loop`, `while`, `for` or a block after a label, found {}",
                        found.describe()
                    ),
                    RECOVERY,
                ),
            }
            m.complete(p, K::LABELLED_EXPR)
        }
        T![try] => {
            // try_expr ::= 'try' block ( 'catch' '{' arms '}' )?
            let m = p.start();
            p.bump(T![try]);
            block(p);
            if p.at(T![catch]) {
                let c = p.start();
                p.bump(T![catch]);
                arm_list(p, false);
                c.complete(p, K::CATCH);
            }
            m.complete(p, K::TRY_EXPR)
        }
        T![with] => {
            // with_expr ::= 'with' handler ( ',' handler )* block
            let m = p.start();
            p.bump(T![with]);
            loop {
                // handler ::= path_type '=' expr+ns
                let h = p.start();
                path_type(p);
                p.expect(T!["="]);
                expr_r(p, R::NS);
                h.complete(p, K::HANDLER);
                if !p.eat(T![","]) {
                    break;
                }
            }
            block(p);
            m.complete(p, K::WITH_EXPR)
        }
        T![select] => {
            let m = p.start();
            p.bump(T![select]);
            arm_list(p, true);
            m.complete(p, K::SELECT_EXPR)
        }
        T![unsafe] => {
            // unsafe_block ::= 'unsafe' '(' STRING ')' block
            let m = p.start();
            p.bump(T![unsafe]);
            p.expect(T!["("]);
            if !p.eat(K::STRING) {
                p.error("expected the justification: a string");
            }
            p.expect(T![")"]);
            block(p);
            m.complete(p, K::UNSAFE_BLOCK)
        }
        T![gen] => {
            let m = p.start();
            p.bump(T![gen]);
            block(p);
            m.complete(p, K::GEN_BLOCK)
        }
        _ => unreachable!("block_like_keyword is called at a block-like keyword"),
    }
}

/// `if_expr ::= 'if' ( 'const' expr+ns | expr+ns+cond ) block ( 'else' ( if_expr | block ) )??`
fn if_expr(p: &mut Parser) -> CompletedMarker {
    let m = p.start();
    p.bump(T![if]);
    if p.eat(T![const]) {
        expr_r(p, R::NS);
    } else {
        expr_r(p, R::NS_COND);
    }
    block(p);
    if p.eat(T![else]) {
        if p.at(T![if]) {
            if_expr(p);
        } else {
            block(p);
        }
    }
    m.complete(p, K::IF_EXPR)
}

/// `'{' inner_attr* arms '}'` (match and catch) or `'{' select_arms '}'`, with
/// `arms ::= ( outer_attr* arm_pattern ( 'if' expr )? '=>' arm_rest )?`,
/// `arm_rest ::= block_like ','? arms | expr+stmt ( ',' arms )?`, and
/// `select_arms ::= ( pattern '=' expr '=>' arm_select_rest )?`.
fn arm_list(p: &mut Parser, select: bool) {
    let list = p.start();
    if !p.expect(T!["{"]) {
        list.complete(p, K::MATCH_ARM_LIST);
        return;
    }
    if !select {
        inner_attrs(p);
    }
    while !p.at(T!["}"]) && !p.at_eof() {
        let before = p.position();
        let m = p.start();
        if select {
            pattern(p);
            p.expect(T!["="]);
            expr(p);
        } else {
            outer_attrs(p);
            arm_pattern(p);
            if p.at(T![if]) {
                let g = p.start();
                p.bump(T![if]);
                expr(p);
                g.complete(p, K::MATCH_GUARD);
            }
        }
        p.expect(T!["=>"]);
        if at_block_like(p) {
            block_like(p);
            p.eat(T![","]);
        } else if can_start_expr(p, R::STMT) {
            expr_r(p, R::STMT);
            if !p.eat(T![","]) && !p.at(T!["}"]) {
                let found = p.current();
                p.error(format!(
                    "expected `,` or `}}` after the arm, found {}",
                    found.describe()
                ));
            }
        } else {
            let found = p.current();
            p.err_recover(
                format!("expected the arm's value, found {}", found.describe()),
                RECOVERY,
            );
        }
        m.complete(p, if select { K::SELECT_ARM } else { K::MATCH_ARM });
        if p.position() == before {
            p.err_and_bump("expected an arm");
        }
    }
    p.expect(T!["}"]);
    list.complete(p, K::MATCH_ARM_LIST);
}
