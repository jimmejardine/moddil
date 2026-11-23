use moddil_mir::TrapClass;
use moddil_sema::{Database, Item, Program};

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
    let compiled = compile(&db, file, &[Item::Fn(main)]).expect("compiled to WASM");
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(&compiled.bytes)
        .expect("a valid module");
    if !host_available() {
        eprintln!("skipped: no Node to run WASM in");
        return None;
    }
    Some(run_main(&db, file, main).map(|(shown, _)| shown))
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
fn wide_integers() {
    check(
        "fn main() -> i128 { let a: i128 = 9223372036854775807; a * a - a / 3 }",
        "85070591730234615844322450438614242647",
    );
    check(
        "fn main() -> u128 { let a: u128 = 1; (a << 100) + (a << 3) }",
        "1267650600228229401496703205384",
    );
    check_trap(
        "fn main() -> i128 { let a: i128 = 170141183460469231731687303715884105727; a + 1 }",
        TrapClass::Overflow,
    );
    check(
        "fn main() -> bool { let a: i128 = -5; let b: i128 = 3; a < b && b > a && a % b == -2 }",
        "true",
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
        "fn sum(xs: &[int]) -> int { let mut s = 0; for x in xs { s += *x; } s }
         fn main() -> int { let a = [1, 2, 3, 4]; sum(&a) }",
        "10",
    );
}

#[test]
fn the_heap_generics_and_traits() {
    check(
        "fn main() -> Vec<int> { let mut v = Vec::new(); for i in 0..5 { v.push(i * i); } v }",
        "[0, 1, 4, 9, 16]",
    );
    check(
        r#"fn main() -> Vec<String> { let mut v = Vec::new(); v.push("a".to_string()); v.push(String::from_str("a string long enough for the heap")); v }"#,
        r#"["a", "a string long enough for the heap"]"#,
    );
    check(
        "trait Shape { fn area(&self) -> int; }  struct Sq { s: int }  impl Shape for Sq { fn area(&self) -> int { self.s * self.s } }
         fn main() -> int { let b: Box<dyn Shape> = Box::new(Sq { s: 3 }); let mut v = Vec::new(); v.push(b); v[0].area() }",
        "9",
    );
    check(
        "fn first<T: Copy>(xs: &[T]) -> T { xs[0] }  fn main() -> (int, bool) { (first(&[4, 5]), first(&[true])) }",
        "(4, true)",
    );
    check(
        "trait Area { fn area(&self) -> int; }
         struct Sq { s: int }  impl Area for Sq { fn area(&self) -> int { self.s * self.s } }
         struct Re { w: int, h: int }  impl Area for Re { fn area(&self) -> int { self.w * self.h } }
         fn by_dyn(x: &dyn Area) -> int { x.area() }
         fn main() -> int { by_dyn(&Sq { s: 3 }) + by_dyn(&Re { w: 2, h: 5 }) }",
        "19",
    );
    check(
        "fn apply<F: Fn(int) -> int>(f: F, x: int) -> int { f(x) }  fn main() -> int { let k = 10; apply(|x| x * k, 4) }",
        "40",
    );
    check(
        "fn main() -> int { let f: Box<dyn Fn(int) -> int> = Box::new(|x| x * 3); let g: Box<dyn FnOnce() -> int> = Box::new(|| 7); f(4) + g() }",
        "19",
    );
}

#[test]
fn text_and_conversions() {
    check(
        "fn main() -> String { String::from_str((-17).text()) }",
        "\"-17\"",
    );
    check(
        "fn main() -> int { let x: f64 = 3.25; x.text().len() }",
        "4",
    );
    check(
        r#"fn main() -> String { String::from_str("  Hi  ".trim().upper_text()) }"#,
        "\"HI\"",
    );
    check(r#"fn main() -> bool { "abc" < "abd" }"#, "true");
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
        }
        test \"u\" {
            let v = Vec::from_array([1, 2]);
            v                                       // => [1, 2]
        }\n";
    let file = program(&db, text);
    if !host_available() {
        return;
    }
    let runs = run_tests(&db, file, &[0, 1]).expect("ran");
    let shown: Vec<Vec<&str>> = runs
        .iter()
        .map(|(observed, result)| {
            assert!(result.is_ok(), "{result:?}");
            observed.iter().map(|(_, _, v)| v.as_str()).collect()
        })
        .collect();
    assert_eq!(shown, [vec!["P { x: 4, y: true }", "5"], vec!["[1, 2]"]]);
}
