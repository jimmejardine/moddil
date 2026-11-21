//! Files, attributes and items.

use super::exprs::{block, expr, expr_r};
use super::patterns::pattern_no_alt;
use super::types::{at_path_type, bounds, const_arg, exponent, path_type, type_, type_ne};
use super::{
    R, RECOVERY, at_member_name, at_name_colon, comma_list, name, simple_path, token_tree,
};
use crate::SyntaxKind as K;
use crate::parser::{CompletedMarker, Marker, Parser, TokenSet};

/// `file ::= inner_attr* item*`
pub(crate) fn source_file(p: &mut Parser) {
    let m = p.start();
    inner_attrs(p);
    while !p.at_eof() {
        if p.at(T!["}"]) {
            p.err_and_bump("unmatched `}`");
            continue;
        }
        item(p);
    }
    m.complete(p, K::SOURCE_FILE);
}

/// `inner_attr ::= '#![' attr_list ']' | DOC_INNER`
pub(super) fn inner_attrs(p: &mut Parser) {
    loop {
        if p.at(K::DOC_INNER) {
            let m = p.start();
            p.bump(K::DOC_INNER);
            m.complete(p, K::INNER_ATTR);
        } else if p.at(T!["#!["]) {
            let m = p.start();
            p.bump(T!["#!["]);
            attr_list(p);
            p.expect(T!["]"]);
            m.complete(p, K::INNER_ATTR);
        } else {
            break;
        }
    }
}

pub(super) fn at_outer_attr(p: &mut Parser) -> bool {
    p.at(K::DOC_OUTER) || p.at(T!["#["])
}

/// `outer_attr ::= '#[' attr_list ']' | DOC_OUTER`
pub(super) fn outer_attrs(p: &mut Parser) {
    loop {
        if p.at(K::DOC_OUTER) {
            let m = p.start();
            p.bump(K::DOC_OUTER);
            m.complete(p, K::OUTER_ATTR);
        } else if p.at(T!["#["]) {
            let m = p.start();
            p.bump(T!["#["]);
            attr_list(p);
            p.expect(T!["]"]);
            m.complete(p, K::OUTER_ATTR);
        } else {
            break;
        }
    }
}

/// `attr_list ::= attr ( ',' attr )* ','?` and `attr ::= attr_path attr_input?`
fn attr_list(p: &mut Parser) {
    let mut first = true;
    while first || at_member_name(p) {
        first = false;
        let m = p.start();
        attr_path(p);
        if p.at(T!["("]) {
            let input = p.start();
            p.bump(T!["("]);
            while !p.at(T![")"]) && !p.at_eof() {
                if p.at(T!["]"]) || p.at(T!["}"]) {
                    p.err_and_bump("unbalanced delimiter");
                    continue;
                }
                token_tree(p);
            }
            p.expect(T![")"]);
            input.complete(p, K::ATTR_INPUT);
        } else if p.at(T!["="]) {
            let input = p.start();
            p.bump(T!["="]);
            super::exprs::literal(p);
            input.complete(p, K::ATTR_INPUT);
        }
        m.complete(p, K::ATTR);
        if !p.eat(T![","]) {
            break;
        }
    }
}

/// `attr_path ::= member_name ( '::' member_name )*`: an attribute's name may be a keyword
/// (`#[default]`).
fn attr_path(p: &mut Parser) {
    let m = p.start();
    let seg = p.start();
    super::member_name(p);
    seg.complete(p, K::PATH_SEGMENT);
    while p.at(T!["::"]) && (p.nth_at(1, K::IDENT) || p.nth(1).is_keyword()) {
        p.bump(T!["::"]);
        let seg = p.start();
        super::member_name(p);
        seg.complete(p, K::PATH_SEGMENT);
    }
    m.complete(p, K::PATH);
}

/// `note ::= 'note' ( STRING | BLOCK_STRING )`
fn notes(p: &mut Parser) {
    while p.at(T![note]) {
        let m = p.start();
        p.bump(T![note]);
        if !p.eat(K::STRING) && !p.eat(K::BLOCK_STRING) {
            p.error("expected the note's text: a string");
        }
        m.complete(p, K::NOTE);
    }
}

/// `vis ::= 'pub' ( '(' 'mod' ')' )?`
fn vis(p: &mut Parser) {
    if !p.at(T![pub]) {
        return;
    }
    let m = p.start();
    p.bump(T![pub]);
    if p.at(T!["("]) && p.nth_at(1, T![mod]) && p.nth_at(2, T![")"]) {
        p.bump(T!["("]);
        p.bump(T![mod]);
        p.expect(T![")"]);
    }
    m.complete(p, K::VIS);
}

const ITEM_START: TokenSet = TokenSet::new(&[
    T![fn],
    T![const],
    T![unsafe],
    T![gen],
    T![linear],
    T![struct],
    T![view],
    T![enum],
    T![trait],
    T![impl],
    T![type],
    T![use],
    T![mod],
    T![test],
    T![property],
    T![bench],
    T![group],
    T![unit],
    T![dimension],
]);

/// `item ::= outer_attr* note* vis? item_kind`
fn item(p: &mut Parser) {
    let m = p.start();
    outer_attrs(p);
    notes(p);
    vis(p);
    if p.at_set(ITEM_START) {
        item_kind(p, m);
    } else {
        let found = p.current();
        p.error(format!("expected an item, found {}", found.describe()));
        // Skip to something that can start an item.
        while !p.at_eof() && !p.at_set(ITEM_START) && !at_outer_attr(p) && !p.at(T![pub]) {
            p.bump_any();
        }
        m.complete(p, K::ERROR);
    }
}

/// `item_kind`: the item at the current token, completing `m`.
fn item_kind(p: &mut Parser, m: Marker) -> CompletedMarker {
    match p.current() {
        T![const] if p.nth_at(1, K::IDENT) => const_item(p, m),
        T![unsafe] if p.nth_at(1, T![trait]) => trait_item(p, m),
        T![unsafe] if p.nth_at(1, T![impl]) => impl_item(p, m),
        T![fn] | T![const] | T![unsafe] | T![gen] => fn_item(p, m),
        T![linear] | T![struct] => struct_item(p, m),
        T![view] => view_item(p, m),
        T![enum] => enum_item(p, m),
        T![trait] => trait_item(p, m),
        T![impl] => impl_item(p, m),
        T![type] => type_item(p, m),
        T![use] => use_item(p, m),
        T![mod] => mod_item(p, m),
        T![test] => test_item(p, m),
        T![property] => property_item(p, m),
        T![bench] => bench_item(p, m),
        T![group] => group_item(p, m),
        T![unit] => unit_item(p, m),
        T![dimension] => dimension_item(p, m),
        _ => unreachable!("item_kind is called at an item start"),
    }
}

/// `local_item`: the items a block may hold. At `const`, `unsafe` and `gen` the caller has
/// already ruled out the block-like forms.
pub(super) const LOCAL_ITEM_START: TokenSet = TokenSet::new(&[
    T![fn],
    T![const],
    T![unsafe],
    T![gen],
    T![linear],
    T![struct],
    T![enum],
    T![trait],
    T![impl],
    T![type],
    T![use],
]);

pub(super) fn local_item(p: &mut Parser, m: Marker) -> CompletedMarker {
    item_kind(p, m)
}

// ─── Functions ──────────────────────────────────────────────────────────────

/// `fn_item ::= fn_qualifier* 'fn' member_name generic_params? '(' params? ')' signature_tail
///              ( block | ';' )`
fn fn_item(p: &mut Parser, m: Marker) -> CompletedMarker {
    while p.at(T![const]) || p.at(T![unsafe]) || p.at(T![gen]) {
        p.bump_any();
    }
    p.expect(T![fn]);
    name(p, true);
    if p.at(T!["<"]) {
        generic_params(p);
    }
    param_list(p, T!["("], T![")"]);
    signature_tail(p);
    if p.at(T!["{"]) {
        block(p);
    } else if !p.eat(T![";"]) {
        p.err_recover("expected a body `{ … }` or `;`", RECOVERY);
    }
    m.complete(p, K::FN_ITEM)
}

/// `'(' params? ')'`, with `params ::= param_entry ( ',' param_entry )* ','?`
fn param_list(p: &mut Parser, open: K, close: K) {
    let m = p.start();
    if !p.expect(open) {
        m.complete(p, K::PARAM_LIST);
        return;
    }
    comma_list(p, close, "a parameter", at_param, param_entry);
    p.expect(close);
    m.complete(p, K::PARAM_LIST);
}

fn at_param(p: &mut Parser) -> bool {
    at_outer_attr(p) || p.at(T![self]) || p.at(T!["*"]) || super::patterns::at_pattern(p)
}

/// `param_entry ::= outer_attr* ( self_param | '*' | param )`
fn param_entry(p: &mut Parser) {
    let m = p.start();
    outer_attrs(p);
    let is_self = p.at(T![self])
        || (p.at(T!["&"])
            && (p.nth_at(1, T![self]) || (p.nth_at(1, T![mut]) && p.nth_at(2, T![self]))))
        || (p.at(T![mut]) && p.nth_at(1, T![self]));
    if is_self {
        // self_param ::= '&' 'mut'? 'self' | 'mut'? 'self' ( ':' type )?
        if p.eat(T!["&"]) {
            p.eat(T![mut]);
            p.expect(T![self]);
        } else {
            p.eat(T![mut]);
            p.expect(T![self]);
            if p.eat(T![":"]) {
                type_(p);
            }
        }
        m.complete(p, K::SELF_PARAM);
    } else if p.eat(T!["*"]) {
        m.complete(p, K::NAMED_ONLY_MARKER);
    } else {
        // param ::= pattern_no_alt ( ':' type )? ( '=' expr )?
        pattern_no_alt(p);
        if p.eat(T![":"]) {
            type_(p);
        }
        if p.eat(T!["="]) {
            expr(p);
        }
        m.complete(p, K::PARAM);
    }
}

/// `generic_params ::= '<' generic_param ( ',' generic_param )* ','? '>'`
pub(super) fn generic_params(p: &mut Parser) {
    let m = p.start();
    p.bump(T!["<"]);
    let mut first = true;
    loop {
        if p.at_split(T![">"]) && !first {
            break;
        }
        first = false;
        let gp = p.start();
        outer_attrs(p);
        if p.eat(T![const]) {
            name(p, false);
            p.expect(T![":"]);
            type_(p);
            if p.eat(T!["="]) {
                const_arg(p);
            }
            gp.complete(p, K::CONST_PARAM);
        } else if p.at(K::IDENT) {
            name(p, false);
            if p.eat(T![":"]) {
                bounds(p);
            }
            if p.eat(T!["="]) {
                type_(p);
            }
            gp.complete(p, K::TYPE_PARAM);
        } else {
            gp.abandon(p);
            let found = p.current();
            p.err_recover(
                format!("expected a generic parameter, found {}", found.describe()),
                RECOVERY,
            );
            break;
        }
        if !p.eat(T![","]) {
            break;
        }
    }
    if p.at_split(T![">"]) {
        p.bump_split(T![">"]);
    } else {
        p.error("expected `>`");
    }
    m.complete(p, K::GENERIC_PARAMS);
}

/// `signature_tail ::= ( '->' type_ne )? resume_clause? from_clause? effect_clause?
///                     where_clause? contract*`
fn signature_tail(p: &mut Parser) {
    if p.at(T!["->"]) {
        let m = p.start();
        p.bump(T!["->"]);
        type_ne(p);
        m.complete(p, K::RET_TYPE);
    }
    if p.at(T![resume]) {
        let m = p.start();
        p.bump(T![resume]);
        name(p, false);
        p.expect(T![":"]);
        type_ne(p);
        m.complete(p, K::RESUME_CLAUSE);
    }
    if p.at(T![from]) {
        let m = p.start();
        p.bump(T![from]);
        p.expect(K::IDENT);
        while p.eat(T![","]) {
            p.expect(K::IDENT);
        }
        m.complete(p, K::FROM_CLAUSE);
    }
    if p.at(T![with]) {
        let m = p.start();
        p.bump(T![with]);
        loop {
            let e = p.start();
            if at_name_colon(p) {
                p.bump(K::IDENT);
                p.bump(T![":"]);
            }
            effect_atom(p);
            e.complete(p, K::SIG_EFFECT);
            if !p.eat(T![","]) {
                break;
            }
        }
        m.complete(p, K::EFFECT_CLAUSE);
    }
    if p.at(T![where]) {
        where_clause(p);
    }
    while p.at(T![requires]) || p.at(T![ensures]) {
        // contract ::= 'requires' 'always'? contract_body | 'ensures' contract_body
        // contract_body ::= ( IDENT ':' )? expr+ns
        let m = p.start();
        if p.eat(T![requires]) {
            p.eat(T![always]);
        } else {
            p.bump(T![ensures]);
        }
        if at_name_colon(p) {
            name(p, false);
            p.bump(T![":"]);
        }
        expr_r(p, R::NS);
        m.complete(p, K::CONTRACT);
    }
}

/// `effect_atom ::= 'suspend' | 'spawn' | 'nondet' | 'fail' '(' error_types ')' | '&' type
///                | path_type`
pub(super) fn effect_atom(p: &mut Parser) {
    let m = p.start();
    match p.current() {
        T![suspend] | T![spawn] | T![nondet] => p.bump_any(),
        T![fail] => {
            p.bump(T![fail]);
            p.expect(T!["("]);
            let e = p.start();
            type_(p);
            while p.eat(T!["|"]) {
                type_(p);
            }
            e.complete(p, K::ERROR_TYPES);
            p.expect(T![")"]);
        }
        T!["&"] | T!["&&"] => {
            p.bump_split(T!["&"]);
            type_(p);
        }
        _ if at_path_type(p) => {
            path_type(p);
        }
        found => p.err_recover(
            format!("expected an effect, found {}", found.describe()),
            RECOVERY,
        ),
    }
    m.complete(p, K::EFFECT);
}

/// `where_clause ::= 'where' where_pred ( ',' where_pred )*` with `where_pred ::= type ':' bounds`
pub(super) fn where_clause(p: &mut Parser) {
    let m = p.start();
    p.bump(T![where]);
    loop {
        let w = p.start();
        type_(p);
        p.expect(T![":"]);
        bounds(p);
        w.complete(p, K::WHERE_PRED);
        if !p.eat(T![","]) {
            break;
        }
    }
    m.complete(p, K::WHERE_CLAUSE);
}

// ─── Data types ─────────────────────────────────────────────────────────────

/// `struct_item ::= 'linear'? 'struct' IDENT generic_params?
///     ( where_clause? ( struct_body | ';' ) | tuple_fields where_clause? ';' )`
fn struct_item(p: &mut Parser, m: Marker) -> CompletedMarker {
    p.eat(T![linear]);
    p.expect(T![struct]);
    name(p, false);
    if p.at(T!["<"]) {
        generic_params(p);
    }
    if p.at(T!["("]) {
        tuple_fields(p);
        if p.at(T![where]) {
            where_clause(p);
        }
        p.expect(T![";"]);
    } else {
        if p.at(T![where]) {
            where_clause(p);
        }
        if p.at(T!["{"]) {
            struct_body(p);
        } else if !p.eat(T![";"]) {
            p.err_recover("expected fields `{ … }`, `( … )` or `;`", RECOVERY);
        }
    }
    m.complete(p, K::STRUCT_ITEM)
}

/// `view_item ::= 'view' IDENT generic_params? where_clause? struct_body`
fn view_item(p: &mut Parser, m: Marker) -> CompletedMarker {
    p.bump(T![view]);
    name(p, false);
    if p.at(T!["<"]) {
        generic_params(p);
    }
    if p.at(T![where]) {
        where_clause(p);
    }
    struct_body(p);
    m.complete(p, K::VIEW_ITEM)
}

/// `struct_body ::= '{' ( struct_member ( ',' struct_member )* ','? )? '}'`
fn struct_body(p: &mut Parser) {
    let m = p.start();
    if !p.expect(T!["{"]) {
        m.complete(p, K::STRUCT_BODY);
        return;
    }
    comma_list(
        p,
        T!["}"],
        "a field",
        |p| at_outer_attr(p) || at_member_name(p),
        |p| {
            // struct_member ::= outer_attr* ( field | 'invariant' expr )
            // field ::= vis? member_name ':' type ( '=' expr )?
            let f = p.start();
            outer_attrs(p);
            if p.at(T![invariant]) && !p.nth_at(1, T![":"]) {
                p.bump(T![invariant]);
                expr(p);
                f.complete(p, K::INVARIANT);
                return;
            }
            if p.at(T![pub]) && !p.nth_at(1, T![":"]) {
                vis(p);
            }
            name(p, true);
            p.expect(T![":"]);
            type_(p);
            if p.eat(T!["="]) {
                expr(p);
            }
            f.complete(p, K::FIELD);
        },
    );
    p.expect(T!["}"]);
    m.complete(p, K::STRUCT_BODY);
}

/// `tuple_fields ::= '(' ( tuple_field ( ',' tuple_field )* ','? )? ')'`
fn tuple_fields(p: &mut Parser) {
    let m = p.start();
    p.bump(T!["("]);
    comma_list(
        p,
        T![")"],
        "a field type",
        |p| at_outer_attr(p) || p.at(T![pub]) || super::types::at_type(p),
        |p| {
            // tuple_field ::= outer_attr* vis? type
            let f = p.start();
            outer_attrs(p);
            vis(p);
            type_(p);
            f.complete(p, K::TUPLE_FIELD);
        },
    );
    p.expect(T![")"]);
    m.complete(p, K::TUPLE_FIELDS);
}

/// `enum_item ::= 'enum' IDENT generic_params? where_clause? '{' ( variant ( ',' variant )* ','? )? '}'`
fn enum_item(p: &mut Parser, m: Marker) -> CompletedMarker {
    p.bump(T![enum]);
    name(p, false);
    if p.at(T!["<"]) {
        generic_params(p);
    }
    if p.at(T![where]) {
        where_clause(p);
    }
    let list = p.start();
    if p.expect(T!["{"]) {
        comma_list(
            p,
            T!["}"],
            "a variant",
            |p| at_outer_attr(p) || p.at(K::IDENT),
            |p| {
                // variant ::= outer_attr* IDENT ( struct_body | tuple_fields )? ( '=' expr )?
                let v = p.start();
                outer_attrs(p);
                name(p, false);
                if p.at(T!["{"]) {
                    struct_body(p);
                } else if p.at(T!["("]) {
                    tuple_fields(p);
                }
                if p.eat(T!["="]) {
                    expr(p);
                }
                v.complete(p, K::VARIANT);
            },
        );
        p.expect(T!["}"]);
    }
    list.complete(p, K::VARIANT_LIST);
    m.complete(p, K::ENUM_ITEM)
}

/// `trait_item ::= 'unsafe'? 'trait' IDENT generic_params? ( ':' bounds )? where_clause?
///                 '{' trait_member* '}'`
fn trait_item(p: &mut Parser, m: Marker) -> CompletedMarker {
    p.eat(T![unsafe]);
    p.bump(T![trait]);
    name(p, false);
    if p.at(T!["<"]) {
        generic_params(p);
    }
    if p.eat(T![":"]) {
        bounds(p);
    }
    if p.at(T![where]) {
        where_clause(p);
    }
    let body = p.start();
    if p.expect(T!["{"]) {
        members(p, false);
        p.expect(T!["}"]);
    }
    body.complete(p, K::TRAIT_BODY);
    m.complete(p, K::TRAIT_ITEM)
}

/// `trait_member ::= outer_attr* note* ( fn_item | assoc_type | const_item )`, and with `impl`:
/// `impl_member ::= outer_attr* note* vis? 'default'? ( fn_item | assoc_type | const_item )`
fn members(p: &mut Parser, is_impl: bool) {
    while !p.at(T!["}"]) && !p.at_eof() {
        let m = p.start();
        outer_attrs(p);
        notes(p);
        if is_impl {
            vis(p);
            p.eat(T![default]);
        }
        match p.current() {
            T![type] => {
                assoc_type(p, m);
            }
            T![const] if p.nth_at(1, K::IDENT) => {
                const_item(p, m);
            }
            T![fn] | T![const] | T![unsafe] | T![gen] => {
                fn_item(p, m);
            }
            found => {
                p.error(format!(
                    "expected `fn`, `type` or `const`, found {}",
                    found.describe()
                ));
                if !p.at(T!["}"]) {
                    p.bump_any();
                }
                m.complete(p, K::ERROR);
            }
        }
    }
}

/// `assoc_type ::= 'type' IDENT ( ':' bounds )? ( '=' type )? ';'`
fn assoc_type(p: &mut Parser, m: Marker) -> CompletedMarker {
    p.bump(T![type]);
    name(p, false);
    if p.eat(T![":"]) {
        bounds(p);
    }
    if p.eat(T!["="]) {
        type_(p);
    }
    p.expect(T![";"]);
    m.complete(p, K::ASSOC_TYPE)
}

/// `impl_item ::= 'unsafe'? 'impl' generic_params? type ( 'for' type )? where_clause? impl_body`
fn impl_item(p: &mut Parser, m: Marker) -> CompletedMarker {
    p.eat(T![unsafe]);
    p.bump(T![impl]);
    if at_generic_params_after_impl(p) {
        generic_params(p);
    }
    type_(p);
    if p.eat(T![for]) {
        type_(p);
    }
    if p.at(T![where]) {
        where_clause(p);
    }
    impl_body(p);
    m.complete(p, K::IMPL_ITEM)
}

/// After `impl`, `<` opens generic parameters unless it starts a qualified path type
/// (`impl <A as B>::C { … }`).
fn at_generic_params_after_impl(p: &mut Parser) -> bool {
    if !p.at(T!["<"]) {
        return false;
    }
    match p.nth(1) {
        T![const] | T!["#["] | K::DOC_OUTER => true,
        K::IDENT => matches!(
            p.nth(2),
            T![">"] | T![","] | T![":"] | T!["="] | T![">>"] | T![">="]
        ),
        _ => false,
    }
}

/// `impl_body ::= '{' impl_member* '}'`
pub(super) fn impl_body(p: &mut Parser) {
    let m = p.start();
    if p.expect(T!["{"]) {
        members(p, true);
        p.expect(T!["}"]);
    }
    m.complete(p, K::IMPL_BODY);
}

/// `type_item ::= 'type' IDENT generic_params? '=' type ( 'where' expr )? ';'`
fn type_item(p: &mut Parser, m: Marker) -> CompletedMarker {
    p.bump(T![type]);
    name(p, false);
    if p.at(T!["<"]) {
        generic_params(p);
    }
    p.expect(T!["="]);
    type_(p);
    if p.eat(T![where]) {
        expr(p);
    }
    p.expect(T![";"]);
    m.complete(p, K::TYPE_ITEM)
}

/// `const_item ::= 'const' IDENT ':' type '=' expr ';'`
fn const_item(p: &mut Parser, m: Marker) -> CompletedMarker {
    p.bump(T![const]);
    name(p, false);
    p.expect(T![":"]);
    type_(p);
    p.expect(T!["="]);
    expr(p);
    p.expect(T![";"]);
    m.complete(p, K::CONST_ITEM)
}

// ─── Modules ────────────────────────────────────────────────────────────────

/// `use_item ::= 'use' use_tree ';'`
fn use_item(p: &mut Parser, m: Marker) -> CompletedMarker {
    p.bump(T![use]);
    use_tree(p);
    p.expect(T![";"]);
    m.complete(p, K::USE_ITEM)
}

fn at_use_segment(p: &mut Parser) -> bool {
    matches!(p.current(), K::IDENT | T![self] | T![super] | T![mod])
}

/// `use_tree ::= use_path ( '::' ( '*' | '{' use_list '}' ) )? ( 'as' IDENT )? | '{' use_list '}'`
fn use_tree(p: &mut Parser) {
    let m = p.start();
    if p.at(T!["{"]) {
        use_list(p);
    } else if at_use_segment(p) {
        // use_path ::= use_segment ( '::' use_segment )*
        let path = p.start();
        let seg = p.start();
        p.bump_any();
        seg.complete(p, K::PATH_SEGMENT);
        while p.at(T!["::"]) && matches!(p.nth(1), K::IDENT | T![self] | T![super] | T![mod]) {
            p.bump(T!["::"]);
            let seg = p.start();
            p.bump_any();
            seg.complete(p, K::PATH_SEGMENT);
        }
        path.complete(p, K::PATH);
        if p.eat(T!["::"]) && !p.eat(T!["*"]) {
            if p.at(T!["{"]) {
                use_list(p);
            } else {
                p.err_recover("expected a name, `*` or `{`", RECOVERY);
            }
        }
        if p.eat(T![as]) {
            name(p, false);
        }
    } else {
        p.err_recover("expected a path to import", RECOVERY);
    }
    m.complete(p, K::USE_TREE);
}

/// `'{' use_list '}'` with `use_list ::= ( use_tree ( ',' use_tree )* ','? )?`
fn use_list(p: &mut Parser) {
    let m = p.start();
    p.bump(T!["{"]);
    comma_list(
        p,
        T!["}"],
        "an import",
        |p| p.at(T!["{"]) || at_use_segment(p),
        use_tree,
    );
    p.expect(T!["}"]);
    m.complete(p, K::USE_LIST);
}

/// `mod_item ::= 'mod' IDENT ';'`
fn mod_item(p: &mut Parser, m: Marker) -> CompletedMarker {
    p.bump(T![mod]);
    name(p, false);
    p.expect(T![";"]);
    m.complete(p, K::MOD_ITEM)
}

// ─── Tests and documentation groups ─────────────────────────────────────────

/// `test_item ::= 'test' STRING ( 'for' test_subject ( 'with' IDENT 'in' '[' type ( ',' type )* ','? ']' )? )?
///                ( 'with' path_type ( ',' path_type )* )? block`
fn test_item(p: &mut Parser, m: Marker) -> CompletedMarker {
    p.bump(T![test]);
    p.expect(K::STRING);
    if p.eat(T![for]) {
        test_subject(p);
        if p.at(T![with]) && p.nth_at(1, K::IDENT) && p.nth_at(2, T![in]) {
            let w = p.start();
            p.bump(T![with]);
            name(p, false);
            p.bump(T![in]);
            p.expect(T!["["]);
            comma_list(p, T!["]"], "a type", super::types::at_type, type_);
            p.expect(T!["]"]);
            w.complete(p, K::TEST_INSTANCES);
        }
    }
    if p.at(T![with]) {
        let w = p.start();
        p.bump(T![with]);
        path_type(p);
        while p.eat(T![","]) {
            path_type(p);
        }
        w.complete(p, K::TEST_CAPABILITIES);
    }
    block(p);
    m.complete(p, K::TEST_ITEM)
}

/// `test_subject ::= expr_path`
fn test_subject(p: &mut Parser) {
    if super::exprs::at_expr_path(p) {
        super::exprs::expr_path(p);
    } else {
        p.err_recover("expected what the test is for: a path", RECOVERY);
    }
}

/// `property_item ::= 'property' STRING 'for' test_subject '(' params? ')' block`
fn property_item(p: &mut Parser, m: Marker) -> CompletedMarker {
    p.bump(T![property]);
    p.expect(K::STRING);
    p.expect(T![for]);
    test_subject(p);
    param_list(p, T!["("], T![")"]);
    block(p);
    m.complete(p, K::PROPERTY_ITEM)
}

/// `bench_item ::= 'bench' STRING ( 'for' test_subject )? block`
fn bench_item(p: &mut Parser, m: Marker) -> CompletedMarker {
    p.bump(T![bench]);
    p.expect(K::STRING);
    if p.eat(T![for]) {
        test_subject(p);
    }
    block(p);
    m.complete(p, K::BENCH_ITEM)
}

/// `group_item ::= 'group' IDENT STRING ';'`
fn group_item(p: &mut Parser, m: Marker) -> CompletedMarker {
    p.bump(T![group]);
    name(p, false);
    p.expect(K::STRING);
    p.expect(T![";"]);
    m.complete(p, K::GROUP_ITEM)
}

// ─── Units and dimensions ───────────────────────────────────────────────────

/// `unit_item ::= 'unit' IDENT ( ':' path_type | '=' unit_def ) ';'`
fn unit_item(p: &mut Parser, m: Marker) -> CompletedMarker {
    p.bump(T![unit]);
    name(p, false);
    if p.eat(T![":"]) {
        path_type(p);
    } else if p.eat(T!["="]) {
        // unit_def ::= scale_expr ( ( '+' | '-' ) number )?
        let d = p.start();
        scale_expr(p);
        if p.at(T!["+"]) || p.at(T!["-"]) {
            p.bump_any();
            if !p.eat(K::INTEGER) && !p.eat(K::FLOAT) {
                p.error("expected a number");
            }
        }
        d.complete(p, K::UNIT_DEF);
    } else {
        p.err_recover("expected `:` or `=`", RECOVERY);
    }
    p.expect(T![";"]);
    m.complete(p, K::UNIT_ITEM)
}

/// `scale_expr ::= scale_power ( ( '*' | '/' ) scale_power )*`
fn scale_expr(p: &mut Parser) {
    let mut lhs = scale_power(p);
    while p.at(T!["*"]) || p.at(T!["/"]) {
        let m = lhs.precede(p);
        p.bump_any();
        scale_power(p);
        lhs = m.complete(p, K::UNIT_BIN);
    }
}

/// `scale_power ::= scale_atom ( '^' exponent )?` and
/// `scale_atom ::= simple_path | number | '(' scale_expr ')'`
fn scale_power(p: &mut Parser) -> CompletedMarker {
    let m = p.start();
    let atom = match p.current() {
        K::IDENT => {
            simple_path(p);
            m.complete(p, K::PATH_TYPE)
        }
        K::INTEGER | K::FLOAT => {
            p.bump_any();
            m.complete(p, K::LITERAL)
        }
        T!["("] => {
            p.bump(T!["("]);
            scale_expr(p);
            p.expect(T![")"]);
            m.complete(p, K::UNIT_PAREN)
        }
        found => {
            p.err_recover(
                format!("expected a unit or a number, found {}", found.describe()),
                RECOVERY,
            );
            m.complete(p, K::ERROR)
        }
    };
    power(p, atom)
}

fn power(p: &mut Parser, atom: CompletedMarker) -> CompletedMarker {
    if p.at(T!["^"]) {
        let m = atom.precede(p);
        p.bump(T!["^"]);
        exponent(p);
        m.complete(p, K::UNIT_POWER)
    } else {
        atom
    }
}

/// `dimension_item ::= 'dimension' IDENT ( '=' unit_expr )? ';'`
fn dimension_item(p: &mut Parser, m: Marker) -> CompletedMarker {
    p.bump(T![dimension]);
    name(p, false);
    if p.eat(T!["="]) {
        unit_expr(p);
    }
    p.expect(T![";"]);
    m.complete(p, K::DIMENSION_ITEM)
}

/// `unit_expr ::= unit_power ( ( '*' | '/' ) unit_power )*`
pub(super) fn unit_expr(p: &mut Parser) {
    let mut lhs = unit_power(p);
    while p.at(T!["*"]) || p.at(T!["/"]) {
        let m = lhs.precede(p);
        p.bump_any();
        unit_power(p);
        lhs = m.complete(p, K::UNIT_BIN);
    }
}

/// `unit_power ::= unit_atom ( '^' exponent )?` and `unit_atom ::= simple_path | '(' unit_expr ')'`
fn unit_power(p: &mut Parser) -> CompletedMarker {
    let m = p.start();
    let atom = match p.current() {
        K::IDENT => {
            simple_path(p);
            m.complete(p, K::PATH_TYPE)
        }
        T!["("] => {
            p.bump(T!["("]);
            unit_expr(p);
            p.expect(T![")"]);
            m.complete(p, K::UNIT_PAREN)
        }
        found => {
            p.err_recover(
                format!("expected a unit, found {}", found.describe()),
                RECOVERY,
            );
            m.complete(p, K::ERROR)
        }
    };
    power(p, atom)
}

/// `unit_suffix ::= '[' unit_expr ']'`
pub(super) fn unit_suffix(p: &mut Parser) {
    let m = p.start();
    p.bump(T!["["]);
    unit_expr(p);
    p.expect(T!["]"]);
    m.complete(p, K::UNIT_SUFFIX);
}
