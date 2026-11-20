use moddil_sema::{Database, Item, Owner, SourceFile};

use super::*;

/// The MIR of every function and test in `text`, one after another.
fn dump(text: &str) -> String {
    let db = Database::default();
    let file = SourceFile::new(&db, "t.mdl".to_string(), text.to_string());
    let diagnostics = moddil_sema::diagnostics(&db, file);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    moddil_sema::owners(&db, file)
        .into_iter()
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
        "t.mdl".to_string(),
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
