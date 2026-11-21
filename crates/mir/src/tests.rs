use moddil_sema::{Database, Item, Owner, SourceFile, Src};

use super::*;

/// The MIR of every function and test in `text`, one after another.
fn dump(text: &str) -> String {
    let db = Database::default();
    let file = SourceFile::new(&db, "t.moddil".to_string(), text.to_string());
    let diagnostics = moddil_sema::diagnostics(&db, file);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let items = moddil_sema::item_tree(&db, file);
    moddil_sema::owners(&db, file)
        .into_iter()
        .filter(|o| match o.item(&db) {
            Item::Fn(i) => items.fns[i as usize].src == Src::File,
            Item::Test(_) => true,
        })
        .map(|o| format!("{:?}", mir(&db, o).as_ref().expect("no errors")))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn version_has_three_parts() {
    assert_eq!(VERSION.split('.').count(), 3);
}

#[test]
fn no_mir_for_files_with_errors() {
    let db = Database::default();
    let file = SourceFile::new(
        &db,
        "t.moddil".to_string(),
        "fn f() -> int { true }".to_string(),
    );
    assert!(mir(&db, Owner::new(&db, file, Item::Fn(0))).is_none());
}

#[test]
fn arithmetic_is_checked() {
    insta::assert_snapshot!(dump(
        "fn f(a: int, b: int) -> int { -(a + b) * 2 / b % 3 << 1 >> a & 1 | 2 ^ 3 }"
    ));
}

#[test]
fn control_flow() {
    insta::assert_snapshot!(dump(
        "fn abs(x: int) -> int { if x < 0 { return -x; } x }\n\
         fn sign(x: int) -> int { if x < 0 { -1 } else if x == 0 { 0 } else { 1 } }\n\
         fn both(a: bool, b: bool) -> bool { a && !b || b }"
    ));
}

#[test]
fn chains_and_calls() {
    insta::assert_snapshot!(dump(
        "fn within(lo: int, x: int, hi: int) -> bool { lo <= x < hi }\n\
         fn twice(x: int) -> int { add(x, x) }\n\
         fn add(a: int, b: int) -> int { a + b }"
    ));
}

#[test]
fn locals_and_assignment() {
    insta::assert_snapshot!(dump(
        "fn f(x: int) -> int { let mut y = x; y += 1; let mut y = y * 2; y = y - 1; y }\n\
         fn g() -> int { let mut x = 1; x + { x = 5; x } }"
    ));
}

#[test]
fn tests_observe_values() {
    insta::assert_snapshot!(dump(
        "test \"t\" {\n    let x = 2;\n    x + 1; // => 3\n    x // => 2\n}"
    ));
}

#[test]
fn aggregates_and_fields() {
    insta::assert_snapshot!(dump(
        "struct Point { x: int, y: int }\n\
         fn f(p: Point) -> (int, Point) { let mut q = Point { y: p.x, x: 1 }; q.y += 1; (q.x, q) }"
    ));
}

#[test]
fn matching() {
    insta::assert_snapshot!(dump(
        "enum Shape { Dot, Circle(int) }\n\
         fn f(s: Shape) -> int { match s { Shape::Circle(r @ 1..=9) if r > 2 => r, Shape::Circle(_) | Shape::Dot => 0 } }"
    ));
}

#[test]
fn loops() {
    insta::assert_snapshot!(dump(
        "fn f() -> int { let mut i = 0; while i < 3 { i += 1; } loop { if i > 5 { break i; } i += 1; } }"
    ));
}

/// The ownership and borrow codes reported for `text`, with the 1-based line each starts on.
fn ownership(text: &str) -> Vec<(String, u32)> {
    let db = Database::default();
    let file = SourceFile::new(&db, "t.moddil".to_string(), text.to_string());
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

fn accepted(text: &str) {
    assert_eq!(ownership(text), vec![], "for:\n{text}");
}

fn rejected(text: &str, expected: &[(&str, u32)]) {
    let expected: Vec<(String, u32)> = expected.iter().map(|&(c, l)| (c.to_string(), l)).collect();
    assert_eq!(ownership(text), expected, "for:\n{text}");
}

const S: &str =
    "struct S { a: P, b: P }\nstruct P { x: int }\n#[derive(Copy, Clone)]\nstruct C { x: int }\n";

#[test]
fn moves() {
    accepted(&format!("{S}fn f(p: P) -> P {{ let q = p; q }}"));
    accepted(&format!("{S}fn f(c: C) -> (C, C) {{ let d = c; (c, d) }}"));
    accepted(&format!(
        "{S}fn f(s: S) -> P {{ let a = s.a; let b = s.b; a }}"
    ));
    accepted(&format!(
        "{S}fn f(p: P) -> P {{ let mut q = p; let r = q; q = P {{ x: 1 }}; q }}"
    ));
    accepted(&format!("{S}fn f(p: P) -> int {{ p; p.x }}"));
    rejected(
        &format!("{S}fn f(p: P) -> P {{\nlet q = p;\np\n}}"),
        &[("E0303", 7)],
    );
    rejected(
        &format!("{S}fn f(s: S) -> S {{\nlet a = s.a;\ns\n}}"),
        &[("E0303", 7)],
    );
    rejected(
        &format!("{S}fn f(b: bool, p: P) -> P {{\nif b {{ let q = p; }}\np\n}}"),
        &[("E0303", 7)],
    );
    rejected(
        &format!("{S}fn f(p: P) {{\nloop {{ let q = p; }}\n}}"),
        &[("E0303", 6)],
    );
    rejected(&format!("{S}fn f(r: &P) -> P {{\n*r\n}}"), &[("E0304", 6)]);
    rejected(&format!("{S}fn f(r: &S) -> P {{\nr.a\n}}"), &[("E0304", 6)]);
}

#[test]
fn borrows() {
    accepted("fn f() -> int { let mut x = 1; let r = &mut x; *r += 1; x }");
    accepted("fn f() -> int { let x = 1; let a = &x; let b = &x; *a + *b }");
    accepted(
        "fn g(r: &mut int) { *r += 1; }\nfn f() -> int { let mut x = 1; g(&mut x); g(&mut x); x }",
    );
    accepted("fn g(r: &mut int) { *r += 1; }\nfn f(r: &mut int) { g(r); g(r); *r += 1; }");
    accepted(
        "fn first(a: &int, b: &int) -> &int from a { a }\nfn f() -> int { let x = 1; let y = 2; *first(&x, &y) }",
    );
    rejected(
        "fn f() -> int {\nlet mut x = 1;\nlet r = &mut x;\nlet y = x;\n*r += 1;\ny\n}",
        &[("E0307", 4)],
    );
    rejected(
        "fn f() {\nlet mut x = 1;\nlet r = &x;\nx = 2;\nlet y = *r;\n}",
        &[("E0306", 4)],
    );
    rejected(
        "fn f() {\nlet mut x = 1;\nlet a = &mut x;\nlet b = &mut x;\n*a += 1;\n}",
        &[("E0305", 4)],
    );
    rejected(
        "fn f() {\nlet mut x = 1;\nlet a = &x;\nlet b = &mut x;\nlet y = *a;\n}",
        &[("E0305", 4)],
    );
    rejected(
        "fn f() -> int {\nlet mut r = &0;\n{ let x = 1; r = &x; }\n*r\n}",
        &[("E0308", 3)],
    );
    rejected("fn f() -> &int {\nlet x = 1;\n&x\n}", &[("E0310", 1)]);
    rejected("fn f(a: int) -> &int from a {\n&a\n}", &[("E0310", 1)]);
    rejected(
        "fn f(a: &int) -> &int {\nlet x = 1;\n&x\n}",
        &[("E0308", 3)],
    );
    rejected(
        "fn f(a: &int, b: &int) -> &int from a {\nb\n}",
        &[("E0310", 2)],
    );
    rejected(
        "fn first(a: &int, b: &int) -> &int from a { a }\nfn f() -> int {\nlet mut x = 1;\nlet r = first(&x, &x);\nx = 2;\n*r\n}",
        &[("E0306", 5)],
    );
}

#[test]
fn matching_borrows() {
    accepted(
        "enum E { A(int), B }\nfn f(e: &mut E) { match e { E::A(n) => *n += 1, E::B => {} } }",
    );
    rejected(
        "fn f(n: int) -> int {\nlet mut x = n;\nmatch x { y if { x = 0; y > 0 } => y, _ => 0 }\n}",
        &[("E0306", 3)],
    );
}

#[test]
fn defer_runs_on_every_exit() {
    insta::assert_snapshot!(dump(
        "fn f(b: bool) -> int { let mut n = 0; { defer { n += 1; } if b { return n; } } n }"
    ));
}

#[test]
fn traits_and_drops() {
    insta::assert_snapshot!(dump(
        "struct G { n: int }\n\
         impl Drop for G { fn drop(self) {} }\n\
         fn eq<T: PartialEq>(a: &T, b: &T) -> bool { *a == *b }\n\
         fn first<T>(x: T, y: T) -> T { x }\n\
         fn f(g: G) -> int { let h = G { n: 1 }; let mut k = g; k = G { n: 2 }; h.n.cmp(&k.n); 0 }"
    ));
}

#[test]
fn destructors() {
    let g = "struct G { n: int }\nimpl Drop for G { fn drop(self) { let G { n } = self; } }\n";
    accepted(&format!("{g}fn f(g: G) -> int {{ g.n }}"));
    accepted(&format!("{g}fn f(g: G) {{ let h = g; drop(h); }}"));
    rejected(
        &format!(
            "{g}struct H {{ g: G }}\nimpl Drop for H {{ fn drop(self) {{}} }}\nfn f(h: H) -> G {{ h.g }}"
        ),
        &[("E0314", 5)],
    );
    rejected(
        &format!("{g}fn f() -> int {{ let r = &G {{ n: 1 }}; r.n }}"),
        &[("E0308", 3)],
    );
}

#[test]
fn errors() {
    insta::assert_snapshot!(dump(
        "struct E { n: int }\n\
         fn check(n: int) -> int with fail(E) { if n > 9 { fail(E { n }); } n }\n\
         fn twice(n: int) -> int with fail(E) { errdefer { assert(true); } check(n)? * 2 }\n\
         fn safe(n: int) -> int { try { check(n)? } catch { E(e) => e.n } }"
    ));
}
