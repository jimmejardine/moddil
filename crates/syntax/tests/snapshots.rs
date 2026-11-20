//! Snapshot tests: the trees the parser builds, printed without whitespace tokens, for the
//! hand-written samples and for focused cases (precedence, flags, splitting, recovery).
//! Review changes with `cargo insta review`.

use std::fmt::Write as _;

use moddil_syntax::{SyntaxElement, SyntaxKind, SyntaxNode};

fn tree(text: &str) -> String {
    let parse = moddil_syntax::parse(text);
    assert_eq!(parse.syntax().text(), text, "lossless");
    let mut out = String::new();
    fn walk(n: &SyntaxNode, depth: usize, out: &mut String) {
        let _ = writeln!(out, "{:indent$}{:?}", "", n.kind(), indent = depth * 2);
        for c in n.children_with_tokens() {
            match c {
                SyntaxElement::Node(n) => walk(&n, depth + 1, out),
                SyntaxElement::Token(t) if t.kind() != SyntaxKind::WHITESPACE => {
                    let _ = writeln!(
                        out,
                        "{:indent$}{:?} {:?}",
                        "",
                        t.kind(),
                        t.text(),
                        indent = depth * 2 + 2
                    );
                }
                SyntaxElement::Token(_) => {}
            }
        }
    }
    walk(&parse.syntax(), 0, &mut out);
    for e in parse.errors() {
        let _ = writeln!(out, "error {e}");
    }
    out
}

macro_rules! snap {
    ($name:ident, $text:expr) => {
        #[test]
        fn $name() {
            insta::assert_snapshot!(stringify!($name), tree($text), $text);
        }
    };
}

snap!(
    precedence,
    "fn f() { x = a || b && c == d | e ^ f & g << h + i * -j?.k(1)[2]; }"
);
snap!(
    chains_and_ranges,
    "fn f() { 0 <= i < n && i in 1..10; ..; a..=b; x = ..y; }"
);
snap!(
    heads_have_no_struct_literals,
    "fn f() { if a == b { c } else if d { S { e: 1 } } else { f } }"
);
snap!(
    let_chains,
    "fn f() { while let Some(x) = it.next() && x > 0 { } if let A(b) = c { } }"
);
snap!(
    splitting,
    "type V = Vec<Vec<T>>; fn f() { a >>= 1; let r = &&x; let g = |a| a || b; let h = || 0; }"
);
snap!(
    block_like_statements,
    "fn f() { if c { 1 } else { 2 } match x { _ => {} } { 3 } }"
);
snap!(
    postfix_forms,
    "fn f() { t.0.1; x.to::<u8>()?; load(p)? \"context\"; v ?! \"reason\"; xs[1, 2]; 1[m/s^2]; }"
);
snap!(
    records_and_structs,
    "fn f() { let r = { min: lo, max: hi }; let s = S { a, b: 1, ..base }; }"
);
snap!(
    key_paths_and_comptime,
    r#"fn f() { let k = \Order.customer.email; if const target.os == Os::Linux { } }"#
);
snap!(
    signature_clauses,
    "gen fn g<T: Ord, const N: int = 4>(x: &T, *, y: int = 2) -> T resume r: R from x with t: Time, fail(E | F) where T: Copy requires always n: x > 0 ensures true;"
);
snap!(
    effects_in_types,
    "type H = Box<dyn Fn(Event) with (suspend, fail(E))>; type R = (u32 where self >= 1);"
);
snap!(
    items,
    "#[derive(Copy)] pub struct P(pub u8, u8); linear struct T { id: u64 } enum E { A, B(int) = 2 } unit km = 1000 * m; dimension V = L / T; group g \"title\";"
);
snap!(
    comments_attach_to_items,
    "// about f\nfn f() {}\n\n// loose\n\n/// doc\nfn g() {}"
);
snap!(recovery_missing_parts, "fn f( { let = ; } fn g() {}");
snap!(recovery_bad_arguments, "fn f() { foo(1, ; bar(); }");
snap!(recovery_bad_field, "struct S { a: , b: int } fn g() {}");
snap!(recovery_garbage_between_items, "fn f() {} ) ] 42 fn g() {}");

#[test]
fn samples() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference/grammar/tests/samples.mdl");
    let text = std::fs::read_to_string(path)
        .expect("samples")
        .replace("\r\n", "\n");
    let cases: Vec<&str> = text.split("\n// ---\n").skip(1).collect();
    assert_eq!(cases.len(), 2);
    insta::assert_snapshot!("sample_tour", tree(cases[0]));
    insta::assert_snapshot!("sample_edge_cases", tree(cases[1]));
}
