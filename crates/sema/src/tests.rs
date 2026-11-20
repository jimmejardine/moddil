use super::*;

/// The codes reported for `text`, with the 1-based line each starts on.
fn codes(text: &str) -> Vec<(String, u32)> {
    let db = Database::default();
    let file = SourceFile::new(&db, "test.mdl".to_string(), text.to_string());
    diagnostics(&db, file)
        .iter()
        .map(|d| {
            (
                d.code.as_str().to_string(),
                moddil_syntax::line_col(text, d.range.start).0,
            )
        })
        .collect()
}

fn ok(text: &str) {
    assert_eq!(codes(text), vec![], "expected no diagnostics for:\n{text}");
}

fn err(text: &str, expected: &[(&str, u32)]) {
    let expected: Vec<(String, u32)> = expected.iter().map(|&(c, l)| (c.to_string(), l)).collect();
    assert_eq!(codes(text), expected, "for:\n{text}");
}

#[test]
fn version_has_three_parts() {
    assert_eq!(VERSION.split('.').count(), 3);
}

#[test]
fn well_typed_programs() {
    ok("fn main() {}");
    ok("fn add(a: int, b: int) -> int { a + b }");
    ok("fn f(x: int) -> bool { let y = x * 2; y > 3 && !(y == 10) }");
    ok("fn f(x: int) -> int { if x < 0 { return -x; } x }");
    ok("fn f(x: int) -> int { if x < 0 { -x } else if x == 0 { 1 } else { x } }");
    ok("fn f() -> int { let mut n = 1; n += 2; n <<= 1; n }");
    ok("fn f(a: int, b: int, c: int) -> bool { a <= b < c }");
    ok("fn f() -> int { g(1, true) } fn g(a: int, b: bool) -> int { if b { a } else { 0 } }");
    ok("fn f() -> int { -9223372036854775808 }");
    ok("fn f() -> () { () }");
    ok("fn f() -> int { return 1 }");
    ok("test \"adds\" { 1 + 2 }");
    ok("fn f(_: int, mut x: int) -> int { x = 3; x }");
    ok("fn f() -> bool { true & false ^ true }");
}

#[test]
fn names() {
    err("fn f() -> int { y }", &[("E0101", 1)]);
    err("fn f() {}\nfn f() {}", &[("E0103", 2)]);
    err("fn f(a: int, a: int) {}", &[("E0104", 1)]);
    err("fn f(a: Foo) {}", &[("E0102", 1)]);
    err("test \"t\" {}\ntest \"t\" {}", &[("E0103", 2)]);
    // A `let` binding is not visible in its own initialiser or outside its block.
    err("fn f() -> int { let x = x; x }", &[("E0101", 1)]);
    err("fn f() -> int { { let x = 1; } x }", &[("E0101", 1)]);
}

#[test]
fn types() {
    err("fn f() -> int { true }", &[("E0201", 1)]);
    err("fn f() -> int { }", &[("E0201", 1)]);
    err("fn f() -> int {\n  return;\n}", &[("E0201", 2)]);
    err("fn f() { if 1 { } }", &[("E0201", 1)]);
    err(
        "fn f() -> int { if true { 1 } else { false } }",
        &[("E0201", 1)],
    );
    err("fn f() { if true { 1 } }", &[("E0201", 1)]);
    err("fn f() -> int { f(1) }", &[("E0202", 1)]);
    err("fn f(x: int) -> int { x(1) }", &[("E0203", 1)]);
    err("fn f() -> int { 9223372036854775808 }", &[("E0204", 1)]);
    err("fn f() -> bool { -true }", &[("E0205", 1)]);
    err(
        "fn f() -> bool { true < false }",
        &[("E0205", 1), ("E0205", 1)],
    );
    err("fn f() -> bool { 1 == true }", &[("E0201", 1)]);
    err("fn f() -> int { true + 1 }", &[("E0205", 1)]);
    err("fn f() { let x: bool = 1; }", &[("E0201", 1)]);
    err(
        "fn f() { if true { 1 } else { 2 } let x = 1; }",
        &[("E0201", 1)],
    );
}

#[test]
fn chains_run_one_way() {
    err(
        "fn f(a: int, b: int, c: int) -> bool { a < b > c }",
        &[("E0002", 1)],
    );
    err(
        "fn f(a: int, b: int, c: int) -> bool { a == b == c }",
        &[("E0002", 1)],
    );
    ok("fn f(a: int, b: int, c: int) -> bool { a > b >= c }");
}

#[test]
fn mutability_and_places() {
    err("fn f() { let x = 1; x = 2; }", &[("E0301", 1)]);
    err("fn f(x: int) { x += 1; }", &[("E0301", 1)]);
    err("fn f() { 1 = 2; }", &[("E0206", 1)]);
    err("fn f() { f = 2; }", &[("E0206", 1)]);
}

#[test]
fn syntax_errors_hide_the_rest() {
    err("fn f() -> int { y + }", &[("E0001", 1)]);
}

#[test]
fn beyond_s0_is_not_supported_yet() {
    err("struct S {}", &[("I0001", 1)]);
    err("fn f<T>() {}", &[("I0001", 1)]);
    err("fn f() { let s = \"x\"; }", &[("I0001", 1)]);
    err("fn f() -> int { loop {} }", &[("I0001", 1)]);
    err("fn f() { let (a, b) = (); }", &[("I0001", 1)]);
}

#[test]
fn tests_observe_annotated_lines() {
    let text = "test \"t\" {\n  1 + 2; // => 3\n  if true { 4 } else { 5 } // => 4\n  6 // => 6\n}";
    let db = Database::default();
    let file = SourceFile::new(&db, "t.mdl".to_string(), text.to_string());
    assert!(diagnostics(&db, file).is_empty());
    let owner = Owner::new(&db, file, Item::Test(0));
    let lowered = body(&db, owner);
    let root = lowered.body.root.expect("a body");
    let hir::ExprKind::Block {
        stmts,
        observe_tail,
        ..
    } = &lowered.body.exprs[root].kind
    else {
        panic!("a block");
    };
    let observed: Vec<bool> = stmts
        .iter()
        .map(|s| matches!(s, hir::Stmt::Expr { observe: true, .. }))
        .collect();
    assert_eq!(observed, vec![true, true]);
    assert!(observe_tail);
}

#[test]
fn queries_are_reused_after_an_edit() {
    use salsa::Setter;
    let mut db = Database::default();
    let file = SourceFile::new(&db, "t.mdl".to_string(), "fn f() -> int { 1 }".to_string());
    assert!(diagnostics(&db, file).is_empty());
    file.set_text(&mut db)
        .to("fn f() -> int { true }".to_string());
    assert_eq!(diagnostics(&db, file).len(), 1);
}
