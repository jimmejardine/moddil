use super::*;

/// The codes reported for `text`, with the 1-based line each starts on.
fn codes(text: &str) -> Vec<(String, u32)> {
    let db = Database::default();
    let file = SourceFile::new(&db, "test.moddil".to_string(), text.to_string());
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
    err("trait T {}", &[("I0001", 1)]);
    err("fn f<T>() {}", &[("I0001", 1)]);
    err("fn f() { let s = \"x\"; }", &[("I0001", 1)]);
    err("fn f() { for x in y {} }", &[("I0001", 1)]);
    err("fn f() { let x = [1]; }", &[("I0001", 1)]);
}

#[test]
fn tests_observe_annotated_lines() {
    let text = "test \"t\" {\n  1 + 2; // => 3\n  if true { 4 } else { 5 } // => 4\n  6 // => 6\n}";
    let db = Database::default();
    let file = SourceFile::new(&db, "t.moddil".to_string(), text.to_string());
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
    let file = SourceFile::new(
        &db,
        "t.moddil".to_string(),
        "fn f() -> int { 1 }".to_string(),
    );
    assert!(diagnostics(&db, file).is_empty());
    file.set_text(&mut db)
        .to("fn f() -> int { true }".to_string());
    assert_eq!(diagnostics(&db, file).len(), 1);
}

const SHAPES: &str = "struct Point { x: int, y: int }\n\
                      struct Pair(int, bool);\n\
                      struct Unit;\n\
                      enum Shape { Dot, Circle(int), Rect { w: int, h: int } }\n";

fn ok1(text: &str) {
    ok(&format!("{SHAPES}{text}"));
}

fn err1(text: &str, expected: &[(&str, u32)]) {
    let shifted: Vec<(&str, u32)> = expected.iter().map(|&(c, l)| (c, l + 4)).collect();
    err(&format!("{SHAPES}{text}"), &shifted);
}

#[test]
fn structs_enums_and_tuples() {
    ok1("fn f() -> int { let p = Point { y: 2, x: 1 }; p.x + p.y }");
    ok1("fn f(x: int) -> Point { Point { x, y: x } }");
    ok1("fn f() -> bool { let p = Pair(1, true); p.1 }");
    ok1("fn f() -> Unit { Unit }");
    ok1("fn f() -> Shape { if true { Shape::Circle(1) } else { Shape::Rect { w: 1, h: 2 } } }");
    ok1("fn f() -> int { let t = (1, (true, 2)); t.1.1 }");
    ok1("fn f() -> int { let mut p = Point { x: 1, y: 2 }; p.x = 5; p.y += 1; p.x }");
    ok1("fn f() -> (int,) { (1,) }");
    err1("fn f() -> int { Point { x: 1 }.x }", &[("E0207", 1)]);
    err1(
        "fn f() -> int { Point { x: 1, y: 2, z: 3 }.x }",
        &[("E0107", 1)],
    );
    err1(
        "fn f() -> int { Point { x: 1, x: 2, y: 3 }.x }",
        &[("E0108", 1)],
    );
    err1("fn f() -> int { Pair(1).0 }", &[("E0202", 1)]);
    err1("fn f() -> int { (1, 2).2 }", &[("E0107", 1)]);
    err1("fn f() -> int { Shape::Dot.x }", &[("E0107", 1)]);
    err1("fn f() -> Shape { Shape::Square }", &[("E0101", 1)]);
    err1("fn f() -> Point { Point }", &[("E0111", 1)]);
    err1(
        "fn f() { let p = Point { x: 1, y: 2 }; p.x = 3; }",
        &[("E0301", 1)],
    );
    err1("fn f() -> bool { Unit == Unit }", &[("E0205", 1)]);
    err(
        "struct A { b: B }\nstruct B { a: A }",
        &[("E0211", 1), ("E0211", 2)],
    );
    err("enum E { A, A }", &[("E0103", 1)]);
    err("struct S;\nfn S() {}", &[("E0103", 2)]);
}

#[test]
fn matching() {
    ok1(
        "fn area(s: Shape) -> int { match s { Shape::Dot => 0, Shape::Circle(r) => 3 * r * r, Shape::Rect { w, h } => w * h } }",
    );
    ok1(
        "fn f(b: bool, c: bool) -> int { match (b, c) { (true, _) => 1, (false, true) => 2, (false, false) => 3 } }",
    );
    ok1("fn f(n: int) -> int { match n { 0 => 0, 1..=9 => 1, 10.. => 2, _ => -1 } }");
    ok1("fn f(n: int) -> int { match n { ..0 => 0, _ => 1 } }"
        .replace("..0", "-9223372036854775808..0")
        .as_str());
    ok1(
        "fn f(s: Shape) -> int { match s { Shape::Dot | Shape::Circle(_) => 0, Shape::Rect { w: x @ 1..=5, .. } if x > 2 => x, Shape::Rect { .. } => 1 } }",
    );
    ok1(
        "fn f(s: Shape) -> int { match s { Shape::Circle(r) | Shape::Rect { w: r, .. } => r, Shape::Dot => 0 } }",
    );
    ok1("fn f(p: Pair) -> int { let Pair(n, _) = p; n }");
    ok1("fn f((a, b): (int, int)) -> int { a + b }");
    ok1("fn f(s: Shape) -> int { let Shape::Circle(r) = s else { return 0; }; r }");
    ok1("fn f(s: Shape) -> int { if let Shape::Circle(r) = s && r > 1 { r } else { 0 } }");
    err1(
        "fn f(s: Shape) -> int { match s { Shape::Dot => 0, Shape::Circle(_) => 1 } }",
        &[("E0208", 1)],
    );
    err1(
        "fn f(n: int) -> int { match n { 0..=5 => 0, 7.. => 1 } }",
        &[("E0208", 1)],
    );
    err1(
        "fn f(n: int) -> int { match n { _ => 0, 1 => 1 } }",
        &[("W0201", 1)],
    );
    err1(
        "fn f(n: int) -> int { match n { x if x > 0 => 0 } }",
        &[("E0208", 1)],
    );
    err1(
        "fn f(s: Shape) -> int { let Shape::Circle(r) = s; r }",
        &[("E0209", 1)],
    );
    err1(
        "fn f(s: Shape) -> int { let Shape::Circle(r) = s else { 0 }; r }",
        &[("E0210", 1)],
    );
    err1(
        "fn f(s: Shape) -> int { match s { Shape::Circle(r) | Shape::Dot => 0, _ => 1 } }",
        &[("E0105", 1)],
    );
    err1(
        "fn f(p: (int, int)) -> int { let (a, a) = p; a }",
        &[("E0106", 1)],
    );
    err1(
        "fn f(p: Pair) -> int { let Pair(a) = p; a }",
        &[("E0213", 1)],
    );
    err1(
        "fn f(p: Point) -> int { let Point { x } = p; x }",
        &[("E0207", 1)],
    );
    err1(
        "fn f(n: int) -> int { match n { 5..=1 => 0, _ => 1 } }",
        &[("E0214", 1)],
    );
    err1(
        "fn f(n: int) -> int { match n { true => 0, _ => 1 } }",
        &[("E0201", 1)],
    );
    ok("enum Never {}\nfn f(n: Never) -> int { match n {} }");
}

#[test]
fn loops() {
    ok("fn f() -> int { let mut i = 0; while i < 10 { i += 1; } i }");
    ok("fn f() -> int { let mut i = 0; loop { i += 1; if i == 5 { break i * 2; } } }");
    ok("fn f() -> int { 'outer: loop { loop { break 'outer 1; } } }");
    ok("fn f() -> int { let mut n = 0; while n < 3 { n += 1; continue; } n }");
    ok("fn f() -> ! { loop {} }");
    err("fn f() { break; }", &[("E0109", 1)]);
    err("fn f() { loop { break 'a; } }", &[("E0110", 1)]);
    err("fn f() { while true { break 1; } }", &[("E0212", 1)]);
    err(
        "fn f() -> int { loop { if true { break 1; } break true; } }",
        &[("E0201", 1)],
    );
}

#[test]
fn assert_is_a_prelude_built_in() {
    ok("test \"t\" { assert(1 + 1 == 2); }");
    ok("fn f(x: int) { assert(x > 0 && x < 10) }");
    ok("fn assert(x: int) -> int { x }\nfn f() -> int { assert(1) }");
    ok("fn f() -> int { let assert = 3; assert }");
    err("fn f() { assert(1); }", &[("E0201", 1)]);
    err("fn f() { assert(true, false); }", &[("E0202", 1)]);
    err("fn f() { let a = assert; }", &[("E0112", 1)]);
}

#[test]
fn borrows_and_copy() {
    ok("fn f(a: &int, b: &mut int) { *b = *a + 1; }");
    ok("struct P { x: int }\nfn f(p: &mut P) -> int { p.x += 1; p.x }");
    ok("fn f() -> int { let mut x = 1; let r = &mut x; *r += 1; x }");
    ok("fn first(a: &int, b: &int) -> &int from a { a }");
    ok("fn f(a: &mut int) -> &int { a }");
    ok("#[derive(Copy, Clone)]\nstruct P { x: int, y: bool }");
    ok("enum S { A(int), B }\nfn f(s: &S) -> int { match s { S::A(n) => *n, S::B => 0 } }");
    ok("fn f(t: &(int, bool)) -> int { let &(n, _) = t; n }");
    ok("fn f() { let mut n = 0; defer { n += 1; } }");
    err("fn f(a: &int) { *a = 1; }", &[("E0302", 1)]);
    err(
        "struct P { x: int }\nfn f(p: &P) { p.x = 1; }",
        &[("E0302", 2)],
    );
    err("fn f() { let x = 1; let r = &mut x; }", &[("E0301", 1)]);
    err("fn f(a: int) -> int { *a }", &[("E0205", 1)]);
    err("fn f(a: &int, b: &int) -> bool { a == b }", &[("E0205", 1)]);
    err("struct P { r: &int }", &[("E0309", 1)]);
    err("fn f(a: &int) { let t = (a, 1); }", &[("E0309", 1)]);
    err("fn f(a: &&int) {}", &[("E0309", 1)]);
    err("fn f() -> &int { 1 }", &[("E0310", 1), ("E0201", 1)]);
    err(
        "fn f(a: int, b: &int) -> &int from a { b }",
        &[("E0310", 1)],
    );
    err(
        "struct S { v: P }\nstruct P { x: int }\n#[derive(Copy, Clone)]\nenum E { A(S) }",
        &[("E0311", 4)],
    );
    err("#[derive(Copy)]\nstruct P;", &[("E0311", 1)]);
    err(
        "#[derive(Copy, Clone)]\nstruct Big { a: (int, int, int, int), b: (int, int, int, int, int) }",
        &[("E0312", 2)],
    );
    err(
        "struct S { x: P }\nstruct P { y: int }\nfn f(s: &S) { let &S { x } = s; }",
        &[("E0304", 3)],
    );
    err("fn f() { loop { defer { break; } } }", &[("E0313", 1)]);
    err("fn f() { defer { return; } }", &[("E0313", 1)]);
}
