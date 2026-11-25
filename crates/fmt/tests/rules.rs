//! Each layout rule of the formatter (this crate's `README.md`: The layout), on a small input
//! written against it: the snapshot is the canonical form, and formatting that again changes
//! nothing.

fn fmt(source: &str) -> String {
    let out = moddil_fmt::format(source).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        moddil_fmt::format(&out).expect("formats again"),
        out,
        "formatting is not stable"
    );
    out
}

#[test]
fn lines_break_between_items_members_fields_variants_and_statements() {
    insta::assert_snapshot!(fmt(
        "struct P { x: int, y: int } enum E { A, B(int) } impl P { fn a(&self) -> int { self.x } \
         fn b(&self) -> int { let s = self.x; s + self.y } }"
    ));
}

#[test]
fn bodies_start_a_new_line_and_an_empty_one_stays_on_its_line() {
    insta::assert_snapshot!(fmt(
        "fn f() -> int { 1 } fn g() {} test \"t\" { assert(true) }"
    ));
}

#[test]
fn where_requires_and_ensures_go_on_lines_of_their_own() {
    insta::assert_snapshot!(fmt(
        "fn max<T>(a: T, b: T) -> T where T: Ord requires true ensures true { a }"
    ));
}

#[test]
fn expression_blocks_stay_on_one_line_when_they_hold_only_a_value() {
    insta::assert_snapshot!(fmt(
        "fn f(c: bool) -> int { let a = if c { 1 } else { 2 }; let b = if c { let x = 1; x } \
         else { 2 }; a + b }"
    ));
}

#[test]
fn unsafe_blocks_end_their_line() {
    insta::assert_snapshot!(fmt(
        "fn f(p: *mut int) { unsafe(\"p is live\") { *p = 1; } }"
    ));
}

#[test]
fn method_chains_go_on_lines_when_two_calls_take_arguments() {
    insta::assert_snapshot!(fmt(
        "fn f(v: Vec<int>) -> int { let n = v.len(); v.iter().map(|x| x * 2).filter(|x| *x > 1)\
         .count() + n }"
    ));
}

#[test]
fn match_arms_take_commas_unless_blocks_and_their_arrows_line_up() {
    insta::assert_snapshot!(fmt(
        "fn f(o: Option<int>) -> int { match o { Some(n) => n, None => { 0 } } }"
    ));
}

#[test]
fn a_line_comment_breaks_a_list() {
    insta::assert_snapshot!(fmt(
        "fn f() -> int { g(1, // one\n2, 3) }\nfn g(a: int, b: int, c: int) -> int { a + b + c }"
    ));
}

#[test]
fn trailing_comments_line_up_and_own_line_comments_stay() {
    insta::assert_snapshot!(fmt(
        "fn f() -> int {\n// own line\nlet a = 1; // one\nlet bbbb = 2; // two\na + bbbb\n}"
    ));
}

#[test]
fn one_blank_line_is_kept_but_none_at_a_block_s_ends() {
    insta::assert_snapshot!(fmt(
        "fn f() -> int {\n\n\nlet a = 1;\n\n\n\nlet b = 2;\n\na + b\n\n}\n\n\n\nfn g() {}"
    ));
}

#[test]
fn spacing_within_a_line_comes_from_the_token_table() {
    insta::assert_snapshot!(fmt("fn f(a:int,b:int)->int{let x=a+b*2;let r=&x;-a+*r}"));
}
