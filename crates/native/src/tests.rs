use moddil_mir::TrapClass;
use moddil_sema::{Database, Program};

use super::*;

/// Runs `main` in `text` natively: its result, or why there is none.
fn run(text: &str) -> Result<String, Native> {
    let db = Database::default();
    let file = Program::single(&db, "t.moddil".to_string(), text.to_string());
    let diagnostics = moddil_mir::diagnostics(&db, file);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let items = moddil_sema::item_tree(&db, file);
    let main = items.main_fn().expect("a main");
    run_main(&db, file, main)
}

fn value(text: &str) -> String {
    run(text).expect("a value")
}

fn trap(text: &str) -> TrapClass {
    match run(text) {
        Err(Native::Trap(class, ..)) => class,
        other => panic!("expected a trap, got {other:?}"),
    }
}

#[test]
fn version_has_three_parts() {
    assert_eq!(VERSION.split('.').count(), 3);
}

#[test]
fn arithmetic() {
    assert_eq!(value("fn main() -> int { 1 + 2 * 3 - 4 }"), "3");
    assert_eq!(value("fn main() -> int { let a = -7; a / 2 }"), "-3");
    assert_eq!(value("fn main() -> int { let a = -7; a % 2 }"), "-1");
    assert_eq!(value("fn main() -> int { let a = -8; a >> 1 }"), "-4");
    assert_eq!(value("fn main() -> int { let a = 6; a & 3 | 8 ^ 1 }"), "11");
    assert_eq!(
        value("fn main() -> bool { let a = 3; a < 4 && !(a == 5) }"),
        "true"
    );
}

#[test]
fn overflow_and_division_trap() {
    assert_eq!(
        trap("fn main() -> int { let a = 9223372036854775807; a + 1 }"),
        TrapClass::Overflow
    );
    assert_eq!(
        trap("fn main() -> int { let a = 1; let b = 0; a / b }"),
        TrapClass::DivisionByZero
    );
    assert_eq!(
        trap("fn main() -> int { let a = 1; let b = 64; a << b }"),
        TrapClass::Overflow
    );
}

#[test]
fn sized_integers_and_floats() {
    assert_eq!(value("fn main() -> u8 { let a: u8 = 250; a +% 10 }"), "4");
    assert_eq!(
        trap("fn main() -> u8 { let a: u8 = 250; a + 10 }"),
        TrapClass::Overflow
    );
    assert_eq!(value("fn main() -> i8 { let a: i8 = -128; a / 3 }"), "-42");
    assert_eq!(
        value("fn main() -> u128 { let a: u128 = 1; a << 100 }"),
        "1267650600228229401496703205376"
    );
    assert_eq!(
        value("fn main() -> f64 { let a = 0.1; a * 3.0 }"),
        "0.30000000000000004"
    );
    assert_eq!(
        value("fn main() -> f32 { let a: f32 = 7.5; a % 2.0 }"),
        "1.5"
    );
    assert_eq!(
        value("fn main() -> int { let a: u8 = 200; let b: int = a; b + 1 }"),
        "201"
    );
}

#[test]
fn calls_and_recursion() {
    let fib = "fn fib(n: int) -> int { if n < 2 { n } else { fib(n - 1) + fib(n - 2) } }
               fn main() -> int { fib(20) }";
    assert_eq!(value(fib), "6765");
    assert_eq!(
        trap("fn down(n: int) -> int { down(n + 1) }  fn main() -> int { down(0) }"),
        TrapClass::StackOverflow
    );
    assert_eq!(
        trap("fn f(a: int) -> int { a * a }  fn main() -> int { f(4294967296) }"),
        TrapClass::Overflow
    );
}

#[test]
fn loops() {
    assert_eq!(
        value(
            "fn main() -> int { let mut s = 0; let mut i = 0; while i < 10 { s += i; i += 1; } s }"
        ),
        "45"
    );
}

#[test]
fn what_is_not_compiled_yet_says_so() {
    assert!(matches!(
        run("struct P { x: int }  fn main() -> int { let p = P { x: 1 }; p.x }"),
        Err(Native::Unsupported(_))
    ));
}
