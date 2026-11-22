use super::*;

/// The codes reported for `text`, with the 1-based line each starts on.
fn codes(text: &str) -> Vec<(String, u32)> {
    let db = Database::default();
    let file = Program::single(&db, "test.moddil".to_string(), text.to_string());
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
fn well_typed_programs_1() {
    ok("fn main() {}");
    ok("fn add(a: int, b: int) -> int { a + b }");
    ok("fn f(x: int) -> bool { let y = x * 2; y > 3 && !(y == 10) }");
    ok("fn f(x: int) -> int { if x < 0 { return -x; } x }");
    ok("fn f(x: int) -> int { if x < 0 { -x } else if x == 0 { 1 } else { x } }");
    ok("fn f() -> int { let mut n = 1; n += 2; n <<= 1; n }");
}

#[test]
fn well_typed_programs_2() {
    ok("fn f(a: int, b: int, c: int) -> bool { a <= b < c }");
    ok("fn f() -> int { g(1, true) } fn g(a: int, b: bool) -> int { if b { a } else { 0 } }");
    ok("fn f() -> int { -9223372036854775808 }");
    ok("fn f() -> () { () }");
    ok("fn f() -> int { return 1 }");
    ok("test \"adds\" { 1 + 2 }");
}

#[test]
fn well_typed_programs_3() {
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
fn types_1() {
    err("fn f() -> int { true }", &[("E0201", 1)]);
    err("fn f() -> int { }", &[("E0201", 1)]);
    err("fn f() -> int {\n  return;\n}", &[("E0201", 2)]);
    err("fn f() { if 1 { } }", &[("E0201", 1)]);
    err(
        "fn f() -> int { if true { 1 } else { false } }",
        &[("E0201", 1)],
    );
    err("fn f() { if true { 1 } }", &[("E0201", 1)]);
}

#[test]
fn types_2() {
    err("fn f() -> int { f(1) }", &[("E0202", 1)]);
    err("fn f(x: int) -> int { x(1) }", &[("E0203", 1)]);
    err("fn f() -> int { 9223372036854775808 }", &[("E0204", 1)]);
    err("fn f() -> bool { -true }", &[("E0205", 1)]);
    err("fn f() -> bool { (1, 2) < (3, 4) }", &[("E0205", 1)]);
    err("fn f() -> bool { 1 == true }", &[("E0201", 1)]);
}

#[test]
fn types_3() {
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
fn the_prelude_and_its_derives_check_cleanly() {
    let db = Database::default();
    let file = Program::single(&db, "t.moddil".to_string(), String::new());
    let items = item_tree(&db, file);
    assert_eq!(items.hidden, vec![], "the prelude's items");
    assert_eq!(items.diagnostics, vec![]);
    for owner in owners(&db, file) {
        let Item::Fn(i) = owner.item(&db) else {
            continue;
        };
        let ds: Vec<&Diagnostic> = body(&db, owner)
            .diagnostics
            .iter()
            .chain(&infer(&db, owner).diagnostics)
            .collect();
        let sig = &items.fns[i as usize];
        assert!(ds.is_empty(), "`{}` ({:?}): {ds:#?}", sig.name, sig.src);
    }
}

#[test]
fn beyond_s0_is_not_supported_yet() {
    err("fn f() -> (impl Clone, int) { (1, 2) }", &[("I0001", 1)]);
    // A function without a body, outside a stub mod (`mod.stub.1`).
    err("fn f() -> int;", &[("I0001", 1)]);
    // `#[root_capability]` outside a stub mod.
    err("#[root_capability] struct S { h: int }", &[("I0001", 1)]);
    err("trait T { const N: int = 1; }", &[("I0001", 1)]);
    err("fn f() { let s = b\"x\"; }", &[("I0001", 1)]);
    err("fn f() { let [a, b] = [1, 2]; }", &[("I0001", 1)]);
}

#[test]
fn tests_observe_annotated_lines() {
    let text = "test \"t\" {\n  1 + 2; // => 3\n  if true { 4 } else { 5 } // => 4\n  6 // => 6\n}";
    let db = Database::default();
    let file = Program::single(&db, "t.moddil".to_string(), text.to_string());
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
    let file = Program::single(
        &db,
        "t.moddil".to_string(),
        "fn f() -> int { 1 }".to_string(),
    );
    assert!(diagnostics(&db, file).is_empty());
    file.files(&db)[0]
        .source
        .set_text(&mut db)
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
fn structs_enums_and_tuples_1() {
    ok1("fn f() -> int { let p = Point { y: 2, x: 1 }; p.x + p.y }");
    ok1("fn f(x: int) -> Point { Point { x, y: x } }");
    ok1("fn f() -> bool { let p = Pair(1, true); p.1 }");
    ok1("fn f() -> Unit { Unit }");
    ok1("fn f() -> Shape { if true { Shape::Circle(1) } else { Shape::Rect { w: 1, h: 2 } } }");
    ok1("fn f() -> int { let t = (1, (true, 2)); t.1.1 }");
}

#[test]
fn structs_enums_and_tuples_2() {
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
}

#[test]
fn structs_enums_and_tuples_3() {
    err1("fn f() -> int { (1, 2).2 }", &[("E0107", 1)]);
    err1("fn f() -> int { Shape::Dot.x }", &[("E0107", 1)]);
    err1("fn f() -> Shape { Shape::Square }", &[("E0101", 1)]);
    err1("fn f() -> Point { Point }", &[("E0111", 1)]);
    err1(
        "fn f() { let p = Point { x: 1, y: 2 }; p.x = 3; }",
        &[("E0301", 1)],
    );
    err1("fn f() -> bool { Unit == Unit }", &[("E0205", 1)]);
}

#[test]
fn structs_enums_and_tuples_4() {
    err(
        "struct A { b: B }\nstruct B { a: A }",
        &[("E0211", 1), ("E0211", 2)],
    );
    err("enum E { A, A }", &[("E0103", 1)]);
    err("struct S;\nfn S() {}", &[("E0103", 2)]);
}

#[test]
fn matching_1() {
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
}

#[test]
fn matching_2() {
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
}

#[test]
fn matching_3() {
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
}

#[test]
fn matching_4() {
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
fn loops_1() {
    ok("fn f() -> int { let mut i = 0; while i < 10 { i += 1; } i }");
    ok("fn f() -> int { let mut i = 0; loop { i += 1; if i == 5 { break i * 2; } } }");
    ok("fn f() -> int { 'outer: loop { loop { break 'outer 1; } } }");
    ok("fn f() -> int { let mut n = 0; while n < 3 { n += 1; continue; } n }");
    ok("fn f() -> ! { loop {} }");
    err("fn f() { break; }", &[("E0109", 1)]);
}

#[test]
fn loops_2() {
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
fn borrows_and_copy_1() {
    ok("fn f(a: &int, b: &mut int) { *b = *a + 1; }");
    ok("struct P { x: int }\nfn f(p: &mut P) -> int { p.x += 1; p.x }");
    ok("fn f() -> int { let mut x = 1; let r = &mut x; *r += 1; x }");
    ok("fn first(a: &int, b: &int) -> &int from a { a }");
    ok("fn f(a: &mut int) -> &int { a }");
    ok("#[derive(Copy, Clone)]\nstruct P { x: int, y: bool }");
}

#[test]
fn borrows_and_copy_2() {
    ok("enum S { A(int), B }\nfn f(s: &S) -> int { match s { S::A(n) => *n, S::B => 0 } }");
    ok("fn f(t: &(int, bool)) -> int { let &(n, _) = t; n }");
    ok("fn f() { let mut n = 0; defer { n += 1; } }");
    err("fn f(a: &int) { *a = 1; }", &[("E0302", 1)]);
    err(
        "struct P { x: int }\nfn f(p: &P) { p.x = 1; }",
        &[("E0302", 2)],
    );
    err("fn f() { let x = 1; let r = &mut x; }", &[("E0301", 1)]);
}

#[test]
fn borrows_and_copy_3() {
    err("fn f(a: int) -> int { *a }", &[("E0205", 1)]);
    err("fn f(a: &int, b: &int) -> bool { a == b }", &[("E0205", 1)]);
    err("struct P { r: &int }", &[("E0309", 1)]);
    err("fn f(a: &&int) {}", &[("E0309", 1)]);
    err("fn f() -> &int { 1 }", &[("E0201", 1)]);
    err(
        "fn f(a: int, b: &int) -> &int from a { b }",
        &[("E0310", 1)],
    );
}

#[test]
fn borrows_and_copy_4() {
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

#[test]
fn generics_and_traits_1() {
    ok("fn id<T>(x: T) -> T { x }\nfn f() -> int { id(1) + id::<int>(2) }");
    ok("struct Pair<A, B> { a: A, b: B }\nfn f() -> bool { let p = Pair { a: 1, b: true }; p.b }");
    ok("fn f() -> int { let x = Some(3); x.unwrap_or(0) }");
    ok("fn f() -> bool { let x: Option<int> = None; x.is_none() }");
    ok("fn f() -> int { match Some(1) { Some(n) => n, None => 0 } }");
    ok(
        "trait Area { fn area(&self) -> int; }\nstruct Sq { s: int }\nimpl Area for Sq { fn area(&self) -> int { self.s * self.s } }\nfn f(x: Sq) -> int { x.area() }",
    );
}

#[test]
fn generics_and_traits_2() {
    ok(
        "trait Area { fn area(&self) -> int; }\nfn total<T: Area>(a: &T, b: &T) -> int { a.area() + b.area() }",
    );
    ok(
        "fn max<T: PartialOrd>(a: T, b: T) -> T { if a > b { a } else { b } }\nfn f() -> int { max(1, 2) }",
    );
    ok(
        "#[derive(Clone, PartialEq)]\nstruct P { x: int }\nfn f(a: P, b: &P) -> bool { a == b.clone() }",
    );
    ok(
        "struct V { x: int }\nimpl Add for V { type Output = V; fn add(self, rhs: V) -> V { V { x: self.x + rhs.x } } }\nfn f(a: V, b: V) -> int { (a + b).x }",
    );
    ok("fn sum<T: Add<Output = T>>(a: T, b: T) -> T { a + b }\nfn f() -> int { sum(1, 2) }");
    ok(
        "struct P { x: int }\nimpl P { fn new(x: int) -> P { P { x } } fn get(&self) -> int { self.x } }\nfn f() -> int { P::new(3).get() }",
    );
}

#[test]
fn generics_and_traits_3() {
    ok("fn f() -> bool { 1.cmp(&2) == Ordering::Less }");
    ok(
        "trait Zero { fn zero() -> Self; }\nimpl Zero for int { fn zero() -> int { 0 } }\nfn f() -> int { int::zero() + Zero::zero() }",
    );
    err("fn f<T>(a: T, b: T) -> bool { a == b }", &[("E0205", 1)]);
    err("struct P;\nfn f() -> P { P.clone() }", &[("E0403", 2)]);
    err("fn f() { let x = None; }", &[("E0405", 1)]);
    err(
        "trait T { fn m(&self); }\nstruct S;\nimpl T for S {}",
        &[("E0407", 3)],
    );
}

#[test]
fn generics_and_traits_4() {
    err(
        "struct S;\nimpl Clone for S { fn clone(&self) -> S { S } }\nimpl Clone for S { fn clone(&self) -> S { S } }",
        &[("E0406", 3)],
    );
    err(
        "fn f<T: Clone>(x: T) {}\nstruct S;\nfn g() { f(S) }",
        &[("E0402", 3)],
    );
    // The prelude's names are shadowed, not taken.
    ok("struct Option;");
}

#[test]
fn errors_1() {
    let e = "pub struct A { n: int }\npub enum B { X, Y(int) }\n";
    let ok2 = |t: &str| ok(&format!("{e}{t}"));
    ok2("fn f(x: int) -> int with fail(A) { if x < 0 { fail(A { n: x }); } x }");
    ok2("fn f() -> int with fail(A) { 1 }\nfn g() -> int with fail(A | B) { f()? + 1 }");
    ok2("fn f() -> int with fail(A) { 1 }\nfn g() -> int { f() ?! \"never\" }");
    ok2("fn f() -> int with fail(A) { 1 }\nfn g() -> int { try { f()? } catch { A(a) => a.n } }");
    ok2(
        "fn f() -> int with fail(A | B) { 1 }\nfn g() -> int with fail(B) { try { f()? } catch { A(_) => 0 } }",
    );
    ok2(
        "fn f() -> int with fail(A | B) { 1 }\nfn g() -> int { try { f()? } catch { A(_) => 0, B::X => 1, B::Y(n) => n } }",
    );
}

#[test]
fn errors_2() {
    let e = "pub struct A { n: int }\npub enum B { X, Y(int) }\n";
    let ok2 = |t: &str| ok(&format!("{e}{t}"));
    let err2 = |t: &str, x: &[(&str, u32)]| {
        let shifted: Vec<(&str, u32)> = x.iter().map(|&(c, l)| (c, l + 2)).collect();
        err(&format!("{e}{t}"), &shifted);
    };
    ok2("fn f() -> int with fail(A) { 1 }\nfn g() -> bool { let r = try { f()? }; r.is_ok() }");
    ok2("fn f(o: Option<int>) -> Option<int> { let x = o?; Some(x + 1) }");
    ok2("fn f(r: Result<int, A>) -> int with fail(A) { r? }");
    ok2("fn f(r: Result<int, A>) { discard(r, \"why\"); }");
    ok2("fn f() -> int with fail(A) { 1 }\ntest \"t\" { f()?; }");
    err2(
        "fn f() -> int with fail(A) { 1 }\nfn g() -> int { f() }",
        &[("E0501", 2)],
    );
}

#[test]
fn errors_3() {
    let e = "pub struct A { n: int }\npub enum B { X, Y(int) }\n";
    let err2 = |t: &str, x: &[(&str, u32)]| {
        let shifted: Vec<(&str, u32)> = x.iter().map(|&(c, l)| (c, l + 2)).collect();
        err(&format!("{e}{t}"), &shifted);
    };
    err2(
        "fn f() -> int with fail(A) { 1 }\npub fn g() -> int { f()? }",
        &[("E0501", 2)],
    );
    err2(
        "fn f() -> int with fail(A | B) { 1 }\npub fn g() -> int with fail(A) { f()? }",
        &[("E0502", 2)],
    );
    err2("fn f() -> int { 1? }", &[("E0504", 1)]);
    err2(
        "fn f() -> int with fail(A) { 1 }\npub fn g() -> int { try { f()? } catch { B::X => 0 } }",
        &[("E0501", 2), ("E0505", 2)],
    );
    err2("fn f(o: Option<int>) -> int { o? }", &[("E0506", 1)]);
    err2("fn main() with fail(A) {}", &[("E0507", 1)]);
}

#[test]
fn errors_4() {
    let e = "pub struct A { n: int }\npub enum B { X, Y(int) }\n";
    let ok2 = |t: &str| ok(&format!("{e}{t}"));
    let err2 = |t: &str, x: &[(&str, u32)]| {
        let shifted: Vec<(&str, u32)> = x.iter().map(|&(c, l)| (c, l + 2)).collect();
        err(&format!("{e}{t}"), &shifted);
    };
    err2("fn f() with fail(int) {}", &[("E0508", 1)]);
    err2("fn f(r: Result<int, A>) { r; }", &[("E0503", 1)]);
    err2(
        "fn f() -> int with fail(A) { defer { fail(A { n: 1 }); } 1 }",
        &[("E0501", 1)],
    );
    // A private function's errors are inferred.
    ok2(
        "fn f() -> int with fail(A) { 1 }\nfn g() -> int { f()? }\npub fn h() -> int with fail(A) { g()? }",
    );
    // A bound on a closure's errors takes them: they cannot leave the function.
    err(
        "trait A { fn a(self) -> int; }\n\
         fn f<F: Fn(int) -> int>(g: F) -> int where F::Fails: A { g(1)? }",
        &[("E0502", 2)],
    );
    // A closure's call names no arguments.
    err(
        "fn f() -> int {\nlet g = |x: int| x;\ng(x: 1)\n}",
        &[("E0215", 3)],
    );
    // Effects that grow without end are E0607, at the function that grows them only.
    err(
        "pub trait S<T> { fn n(&self) -> int; }\nstruct W<T> { x: T }\nfn need<T>() -> int with s: S<T> { s.n() }\nfn grow<T>() -> int { need::<T>() + grow::<W<T>>() }\nfn calm() -> int { 1 }",
        &[("E0607", 4)],
    );
}

#[test]
fn effects_1() {
    let e = "pub trait Time { fn now(&self) -> int; }\nstruct Clock { t: int }\n\
             impl Time for Clock { fn now(&self) -> int { self.t } }\n\
             fn uses() -> int with time: Time { time.now() }\n";
    let ok2 = |t: &str| ok(&format!("{e}{t}"));
    let err2 = |t: &str, x: &[(&str, u32)]| {
        let shifted: Vec<(&str, u32)> = x.iter().map(|&(c, l)| (c, l + 4)).collect();
        err(&format!("{e}{t}"), &shifted);
    };
    ok2("fn f() -> int { uses() }\nfn g(c: Clock) -> int { with Time = c { f() } }");
    ok2("fn f() -> int { uses() }\npub fn g() -> int with Time { f() }");
    ok2("fn f(c: &Clock) -> int { let t: &dyn Time = c; t.now() }");
    ok2("fn f(n: int) -> int { if n == 0 { uses() } else { f(n - 1) } }");
    err2("pub fn f() -> int { uses() }", &[("E0603", 1)]);
    err2("fn main() -> int { uses() }", &[("E0602", 1)]);
}

#[test]
fn effects_2() {
    let e = "pub trait Time { fn now(&self) -> int; }\nstruct Clock { t: int }\n\
             impl Time for Clock { fn now(&self) -> int { self.t } }\n\
             fn uses() -> int with time: Time { time.now() }\n";
    let err2 = |t: &str, x: &[(&str, u32)]| {
        let shifted: Vec<(&str, u32)> = x.iter().map(|&(c, l)| (c, l + 4)).collect();
        err(&format!("{e}{t}"), &shifted);
    };
    err2("test \"t\" { uses(); }", &[("E0602", 1)]);
    err2("fn f() with int {}", &[("E0601", 1)]);
    err2("trait T { fn f(&self) with Time; }", &[("E0604", 1)]);
    err2("fn f(t: dyn Time) {}", &[("E0605", 1)]);
    err2("fn f() with Time, Time {}", &[("E0606", 1)]);
    err2(
        "fn f(c: &Clock) -> int { with Time = c, Time = c { uses() } }",
        &[("E0606", 1)],
    );
}

/// The codes reported for a program of files (module path and text) in mods (name and
/// dependencies), with each code's file and 1-based line.
fn program_codes(
    files: &[(u32, &[&str], &str)],
    mods: &[(&str, &[(&str, u32)])],
) -> Vec<(String, u32, u32)> {
    let db = Database::default();
    let pfiles = files
        .iter()
        .enumerate()
        .map(|(i, (m, path, text))| ProgramFile {
            source: SourceFile::new(&db, format!("f{i}.moddil"), (*text).to_string()),
            mod_: *m,
            module: path.iter().map(|s| (*s).to_string()).collect(),
            exe: false,
        })
        .collect();
    let pmods = mods
        .iter()
        .map(|(n, deps)| ModInfo {
            name: (*n).to_string(),
            deps: deps.iter().map(|(d, i)| ((*d).to_string(), *i)).collect(),
            stub: false,
        })
        .collect();
    let program = Program::new(&db, pfiles, pmods);
    diagnostics(&db, program)
        .iter()
        .map(|d| {
            let text = files[d.file as usize].2;
            (
                d.code.as_str().to_string(),
                d.file,
                moddil_syntax::line_col(text, d.range.start).0,
            )
        })
        .collect()
}

#[test]
fn modules() {
    let root = "use mod::a::Point;\npub(mod) fn f() -> int { a::norm(Point { x: 1 }) + b::g() }\n";
    let a = "pub(mod) struct Point { x: int }\npub(mod) fn norm(p: Point) -> int { p.x }\n";
    let b = "pub(mod) fn g() -> int { mod::f() }\n";
    let one: &[(&str, &[(&str, u32)])] = &[("m", &[])];
    assert_eq!(
        program_codes(&[(0, &[], root), (0, &["a"], a), (0, &["b"], b)], one),
        vec![]
    );
    // Private items are their module's own.
    let a = "fn hidden() -> int { 1 }\n";
    assert_eq!(
        program_codes(
            &[(0, &[], "fn f() -> int { a::hidden() }\n"), (0, &["a"], a)],
            one
        ),
        vec![("E0701".to_string(), 0, 1)]
    );
    // Another mod sees only `pub` items.
    let dep = "pub(mod) fn inner() -> int { 1 }\npub fn outer() -> int { inner() }\n";
    let two: &[(&str, &[(&str, u32)])] = &[("app", &[("dep", 1)]), ("dep", &[])];
    assert_eq!(
        program_codes(
            &[
                (0, &[], "fn f() -> int { dep::outer() + dep::inner() }\n"),
                (1, &[], dep)
            ],
            two
        ),
        vec![("E0701".to_string(), 0, 1)]
    );
    // The prelude's names can be shadowed; imports and items cannot clash.
    assert_eq!(
        program_codes(&[(0, &[], "struct Option;\nfn drop() {}\n")], one),
        vec![]
    );
    assert_eq!(
        program_codes(
            &[
                (0, &[], "use mod::a::X;\nstruct X;\n"),
                (0, &["a"], "pub(mod) struct X;\n")
            ],
            one
        ),
        vec![("E0103".to_string(), 0, 2)]
    );
}

#[test]
fn closures() {
    ok("fn apply(f: impl Fn(int) -> int, x: int) -> int { f(x) }\n\
        fn g() -> int { let k = 2; apply(|x| x * k, 3) }");
    ok(
        "fn each(f: impl FnMut(int)) { f(1); }\nfn g() -> int { let mut n = 0; each(|x| n += x); n }",
    );
    ok("fn g() -> int { let f = |a: int, b: int| a + b; f(1, 2) }");
    err(
        "fn apply(f: impl Fn(int) -> int, x: int) -> int { f(x) }\n\
         fn g() -> int { let mut n = 0; apply(|x| { n += x; x }, 1) }",
        &[("E0801", 2)],
    );
    // A tuple holding a closure is a view.
    ok("fn g() { let f = |x: int| x; let t = (f, 1); }");
    err(
        "fn g() -> int { let f = |x: int| x; f(1, 2) }",
        &[("E0202", 1)],
    );
    // A call passing a closure that can fail is marked.
    err(
        "struct E;\nfn c() -> int with fail(E) { 1 }\n\
         fn apply(f: impl Fn(int) -> int) -> int { f(1) }\n\
         pub fn g() -> int { apply(|x| c()?) }",
        &[("E0501", 4)],
    );
}

#[test]
fn wrappers_not_supported_yet() {
    let w = "fn w<A, R>(f: &FnInfo, a: A, inner: impl FnOnce(A) -> R) -> R { inner(a) }\n";
    ok(&format!("{w}#[around(w)] fn f(x: int) -> int {{ x }}"));
    err(
        &format!("{w}struct S;\nimpl S {{\n#[around(w)] fn m(&self) -> int {{ 1 }}\n}}"),
        &[("I0001", 4)],
    );
    err(
        &format!("{w}#[around(w)] const fn c() -> int {{ 1 }}"),
        &[("I0001", 2)],
    );
    err(
        &format!("{w}#[around(w)] fn d((a, b): (int, int)) -> int {{ a + b }}"),
        &[("I0001", 2)],
    );
}
