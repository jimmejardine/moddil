use moddil_sema::{Database, SourceFile};

use super::*;

/// Calls `main` in `text` with `args`.
fn eval(text: &str, args: &[Value]) -> Result<Value, Trap> {
    let db = Database::default();
    let file = SourceFile::new(&db, "t.mdl".to_string(), text.to_string());
    let diagnostics = moddil_sema::diagnostics(&db, file);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let main = moddil_sema::item_tree(&db, file)
        .fn_named("main")
        .expect("a main");
    call_fn(&db, file, main, args, &mut |_, _| {})
}

fn value(text: &str) -> Value {
    eval(text, &[]).expect("no trap")
}

fn trap(text: &str) -> TrapClass {
    eval(text, &[]).expect_err("a trap").class
}

#[test]
fn version_has_three_parts() {
    assert_eq!(VERSION.split('.').count(), 3);
}

#[test]
fn arithmetic() {
    assert_eq!(value("fn main() -> int { 1 + 2 * 3 - 4 }"), Value::Int(3));
    assert_eq!(value("fn main() -> int { -7 / 2 }"), Value::Int(-3));
    assert_eq!(value("fn main() -> int { -7 % 2 }"), Value::Int(-1));
    assert_eq!(value("fn main() -> int { 1 << 62 }"), Value::Int(1 << 62));
    assert_eq!(value("fn main() -> int { -8 >> 1 }"), Value::Int(-4));
    assert_eq!(value("fn main() -> int { 6 & 3 | 8 ^ 1 }"), Value::Int(11));
    assert_eq!(value("fn main() -> int { !0 }"), Value::Int(-1));
    assert_eq!(
        value("fn main() -> int { -9223372036854775808 }"),
        Value::Int(i64::MIN)
    );
    assert_eq!(
        value("fn main() -> int { (-9223372036854775807 - 1) % -1 }"),
        Value::Int(0)
    );
}

#[test]
fn traps() {
    assert_eq!(
        trap("fn main() -> int { 9223372036854775807 + 1 }"),
        TrapClass::Overflow
    );
    assert_eq!(
        trap("fn main() -> int { -9223372036854775807 - 2 }"),
        TrapClass::Overflow
    );
    assert_eq!(
        trap("fn main() -> int { 4611686018427387904 * 2 }"),
        TrapClass::Overflow
    );
    assert_eq!(
        trap("fn main() -> int { let m = -9223372036854775807 - 1; -m }"),
        TrapClass::Overflow
    );
    assert_eq!(
        trap("fn main() -> int { (-9223372036854775807 - 1) / -1 }"),
        TrapClass::Overflow
    );
    assert_eq!(
        trap("fn main() -> int { 1 / 0 }"),
        TrapClass::DivisionByZero
    );
    assert_eq!(
        trap("fn main() -> int { 1 % 0 }"),
        TrapClass::DivisionByZero
    );
    assert_eq!(trap("fn main() -> int { 1 << 64 }"), TrapClass::Overflow);
    assert_eq!(trap("fn main() -> int { 1 << -1 }"), TrapClass::Overflow);
    assert_eq!(trap("fn main() -> int { 3 << 62 }"), TrapClass::Overflow);
    assert_eq!(trap("fn main() -> int { 1 >> 64 }"), TrapClass::Overflow);
    assert_eq!(
        trap("fn main() -> int { f(0) } fn f(x: int) -> int { f(x + 1) }"),
        TrapClass::StackOverflow
    );
}

#[test]
fn control_flow_and_calls() {
    let fib = "fn main(n: int) -> int { if n < 2 { n } else { main(n - 1) + main(n - 2) } }";
    assert_eq!(eval(fib, &[Value::Int(20)]), Ok(Value::Int(6765)));
    let fact = "fn main(n: int) -> int { if n == 0 { return 1; } n * main(n - 1) }";
    assert_eq!(
        eval(fact, &[Value::Int(20)]),
        Ok(Value::Int(2_432_902_008_176_640_000))
    );
    assert_eq!(
        eval(fact, &[Value::Int(21)]).map_err(|t| t.class),
        Err(TrapClass::Overflow)
    );
    assert_eq!(
        value("fn main() -> bool { let x = 5; 0 <= x < 10 && !(x == 3) }"),
        Value::Bool(true)
    );
    assert_eq!(value("fn main() -> bool { 3 > 2 > 2 }"), Value::Bool(false));
    assert_eq!(
        value("fn main() -> int { let mut x = 1; x + { x = 5; x } }"),
        Value::Int(6)
    );
    assert_eq!(
        value("fn main() -> int { let mut x = 1; x <<= 3; x -= 1; x }"),
        Value::Int(7)
    );
    assert_eq!(
        value("fn main() -> int { let x = 1; let x = x + 1; x }"),
        Value::Int(2)
    );
}

#[test]
fn logic_short_circuits() {
    // The right operand would trap if it ran.
    assert_eq!(
        value("fn main() -> bool { false && 1 / 0 == 1 }"),
        Value::Bool(false)
    );
    assert_eq!(
        value("fn main() -> bool { true || 1 / 0 == 1 }"),
        Value::Bool(true)
    );
    assert_eq!(
        value("fn main() -> bool { 1 > 2 > 1 / 0 }"),
        Value::Bool(false)
    );
}

#[test]
fn tests_report_observed_values() {
    let text = "test \"t\" {\n  let x = 2;\n  x + 1; // => 3\n  x // => 2\n}";
    let db = Database::default();
    let file = SourceFile::new(&db, "t.mdl".to_string(), text.to_string());
    let mut seen = Vec::new();
    run_test(&db, file, 0, &mut |range, v| {
        seen.push((moddil_syntax::line_col(text, range.start).0, v));
    })
    .expect("no trap");
    assert_eq!(seen, vec![(3, Value::Int(3)), (4, Value::Int(2))]);
}
