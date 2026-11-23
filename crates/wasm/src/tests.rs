use moddil_mir::TrapClass;
use moddil_sema::{Database, Program};

use super::*;

fn program(db: &Database, text: &str) -> Program {
    let file = Program::single(db, "t.moddil".to_string(), text.to_string());
    let diagnostics = moddil_mir::diagnostics(db, file);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    file
}

/// `main` compiled, its module validated, and run in Node if there is one: its result.
fn run(text: &str) -> Option<Result<String, Wasm>> {
    let db = Database::default();
    let file = program(&db, text);
    let items = moddil_sema::item_tree(&db, file);
    let main = items.main_fn().expect("a main");
    let compiled = compile(&db, file, &[], Some(main)).expect("compiled to WASM");
    wasmparser::validate(&compiled.bytes).expect("a valid module");
    if !host_available() {
        eprintln!("skipped: no Node to run WASM in");
        return None;
    }
    Some(run_main(&db, file, main))
}

fn check(text: &str, want: &str) {
    if let Some(r) = run(text) {
        assert_eq!(r.expect("a value"), want, "{text}");
    }
}

fn check_trap(text: &str, want: TrapClass) {
    if let Some(r) = run(text) {
        match r {
            Err(Wasm::Trap(class, ..)) => assert_eq!(class, want, "{text}"),
            other => panic!("expected a trap, got {other:?}"),
        }
    }
}

#[test]
fn version_has_three_parts() {
    assert_eq!(VERSION.split('.').count(), 3);
}

#[test]
fn scalars_and_calls() {
    check(
        "fn fib(n: int) -> int { if n < 2 { n } else { fib(n - 1) + fib(n - 2) } }  fn main() -> int { fib(20) }",
        "6765",
    );
    check("fn main() -> bool { 3 < 4 && !(2 == 3) }", "true");
    check("fn main() -> f64 { 1.5 * 4.0 - 0.5 }", "5.5");
    check("fn main() -> f64 { 7.5 % 2.0 }", "1.5");
    check("fn main() -> char { 'x' }", "'x'");
    check(
        "fn main() -> u8 { let a: u8 = 200; let b: u8 = 50; a + b }",
        "250",
    );
    check(
        "fn main() -> i8 { let a: i8 = -100; let b: i8 = 28; a - b }",
        "-128",
    );
    check(
        "fn main() -> int { let mut s = 0; let mut i = 0; while i < 10 { if i % 2 == 0 { s += i; } i += 1; } s }",
        "20",
    );
}

#[test]
fn checks_trap() {
    check_trap(
        "fn main() -> int { let max = 9223372036854775807; max + 1 }",
        TrapClass::Overflow,
    );
    check_trap(
        "fn main() -> u8 { let a: u8 = 200; let b: u8 = 100; a + b }",
        TrapClass::Overflow,
    );
    check_trap(
        "fn main() -> int { let z = 0; 10 / z }",
        TrapClass::DivisionByZero,
    );
    check_trap(
        "fn down(n: int) -> int { down(n + 1) }  fn main() -> int { down(0) }",
        TrapClass::StackOverflow,
    );
}

#[test]
fn values_in_memory_and_borrows() {
    check(
        "struct P { x: u8, y: int, z: u8 }  fn main() -> P { P { x: 1, y: 2, z: 3 } }",
        "P { x: 1, y: 2, z: 3 }",
    );
    check(
        "fn main() -> int { let mut a = [1, 2, 3]; a[1] = 20; a[0] + a[1] + a[2] }",
        "24",
    );
    check_trap(
        "fn main() -> int { let a = [1, 2, 3]; let i = 3; a[i] }",
        TrapClass::IndexOutOfBounds,
    );
    let shape = "enum Shape { Circle(int), Rect(int, int), Empty }
        fn area(s: Shape) -> int { match s { Shape::Circle(r) => 3 * r * r, Shape::Rect(w, h) => w * h, Shape::Empty => 0 } }";
    check(
        &format!("{shape} fn main() -> int {{ area(Shape::Rect(2, 5)) + area(Shape::Circle(1)) }}"),
        "13",
    );
    check(
        "fn f(x: Option<Option<bool>>) -> int { match x { None => 0, Some(None) => 1, Some(Some(b)) => if b { 3 } else { 2 } } }
         fn main() -> int { f(None) * 100 + f(Some(None)) * 10 + f(Some(Some(true))) }",
        "13",
    );
    check(
        "fn bump(x: &mut int) { *x += 1; }  fn main() -> int { let mut n = 1; bump(&mut n); bump(&mut n); n }",
        "3",
    );
    check(
        "struct Big { a: int, b: int, c: int, d: int }
         fn make(n: int) -> Big { Big { a: n, b: n + 1, c: n + 2, d: n + 3 } }
         fn total(b: Big) -> int { b.a + b.b + b.c + b.d }
         fn main() -> int { total(make(10)) }",
        "46",
    );
}

#[test]
fn tests_observe_values() {
    let db = Database::default();
    let text = "struct P { x: int, y: bool }
        test \"t\" {
            let p = P { x: 4, y: true };
            p                                       // => P { x: 4, y: true }
            ;
            p.x + 1                                 // => 5
        }\n";
    let file = program(&db, text);
    if !host_available() {
        return;
    }
    let runs = run_tests(&db, file, &[0]).expect("ran");
    let (observed, result) = &runs[0];
    assert!(result.is_ok(), "{result:?}");
    let shown: Vec<&str> = observed.iter().map(|(_, _, v)| v.as_str()).collect();
    assert_eq!(shown, ["P { x: 4, y: true }", "5"]);
}
