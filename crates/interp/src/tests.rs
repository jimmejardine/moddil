use moddil_sema::{Database, Program};

use super::*;

/// Calls `main` in `text` with `args` (or `f`, given arguments).
fn eval(text: &str, args: &[Value]) -> Result<Value, Trap> {
    match run_main(text, args) {
        Ok(v) => Ok(v),
        Err(Stop::Trap(t)) => Err(t),
        Err(Stop::Failed { .. }) => panic!("an error escaped"),
    }
}

/// Calls `main` in `text` with `args` (or `f`, given arguments), which may stop with an error.
/// `main` takes only root capabilities (`main.root.1`), so a function with other parameters is
/// `f`.
fn run_main(text: &str, args: &[Value]) -> Result<Value, Stop> {
    let db = Database::default();
    let file = Program::single(&db, "t.moddil".to_string(), text.to_string());
    let diagnostics = moddil_mir::diagnostics(&db, file);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let items = moddil_sema::item_tree(&db, file);
    let main = items
        .fns
        .iter()
        .position(|g| !args.is_empty() && g.name == "f" && g.src.is_file())
        .map(|i| i as u32)
        .or_else(|| items.main_fn())
        .expect("a main");
    call_fn(&db, file, main, args, &mut |_, _, _| {})
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
        trap("fn main() -> int { { let max = 9223372036854775807; max + 1 } }"),
        TrapClass::Overflow
    );
    assert_eq!(
        trap("fn main() -> int { let a = -9223372036854775807; a - 2 }"),
        TrapClass::Overflow
    );
    assert_eq!(
        trap("fn main() -> int { let a = 4611686018427387904; a * 2 }"),
        TrapClass::Overflow
    );
    assert_eq!(
        trap("fn main() -> int { let m = -9223372036854775807 - 1; -m }"),
        TrapClass::Overflow
    );
    assert_eq!(
        trap("fn main() -> int { let a = -9223372036854775807 - 1; a / -1 }"),
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
    assert_eq!(
        trap("fn main() -> int { let a = 1; a << 64 }"),
        TrapClass::Overflow
    );
    assert_eq!(trap("fn main() -> int { 1 << -1 }"), TrapClass::Overflow);
    assert_eq!(
        trap("fn main() -> int { let a = 3; a << 62 }"),
        TrapClass::Overflow
    );
    assert_eq!(trap("fn main() -> int { 1 >> 64 }"), TrapClass::Overflow);
    assert_eq!(
        trap("fn main() -> int { f(0) } fn f(x: int) -> int { f(x + 1) }"),
        TrapClass::StackOverflow
    );
}

#[test]
fn control_flow_and_calls() {
    let fib = "fn f(n: int) -> int { if n < 2 { n } else { f(n - 1) + f(n - 2) } }";
    assert_eq!(eval(fib, &[Value::Int(20)]), Ok(Value::Int(6765)));
    let fact = "fn f(n: int) -> int { if n == 0 { return 1; } n * f(n - 1) }";
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
    let file = Program::single(&db, "t.moddil".to_string(), text.to_string());
    let mut seen = Vec::new();
    run_test(&db, file, 0, &mut |_, range, v| {
        seen.push((moddil_syntax::line_col(text, range.start).0, v));
    })
    .expect("no trap");
    assert_eq!(seen, vec![(3, Value::Int(3)), (4, Value::Int(2))]);
}

/// Runs `main` in `text` and renders its value.
fn shown(text: &str) -> String {
    let db = Database::default();
    let file = Program::single(&db, "t.moddil".to_string(), text.to_string());
    let diagnostics = moddil_mir::diagnostics(&db, file);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let items = moddil_sema::item_tree(&db, file);
    let main = items.main_fn().expect("a main");
    call_fn(&db, file, main, &[], &mut |_, _, _| {})
        .expect("no trap")
        .render(items)
}

const SHAPES: &str = "struct Point { x: int, y: int }\n\
                      enum Shape { Dot, Circle(int), Rect { w: int, h: int } }\n\
                      fn area(s: Shape) -> int {\n\
                          match s { Shape::Dot => 0, Shape::Circle(r) => 3 * r * r, Shape::Rect { w, h } => w * h }\n\
                      }\n";

#[test]
fn aggregates() {
    let p = |body: &str| shown(&format!("{SHAPES}{body}"));
    assert_eq!(
        p("fn main() -> Point { Point { y: 2, x: 1 } }"),
        "Point { x: 1, y: 2 }"
    );
    assert_eq!(
        p("fn main() -> (int, (bool,)) { (1, (true,)) }"),
        "(1, (true,))"
    );
    assert_eq!(
        p("fn main() -> Shape { Shape::Circle(4) }"),
        "Shape::Circle(4)"
    );
    assert_eq!(p("fn main() -> Shape { Shape::Dot }"), "Shape::Dot");
    assert_eq!(
        p(
            "fn main() -> int { area(Shape::Circle(2)) + area(Shape::Rect { w: 2, h: 5 }) + area(Shape::Dot) }"
        ),
        "22"
    );
    assert_eq!(
        p("fn main() -> Point { let mut q = Point { x: 1, y: 2 }; q.x = 10; q.y += q.x; q }"),
        "Point { x: 10, y: 12 }"
    );
    assert_eq!(
        p("fn main() -> int { let t = (1, (2, 3)); t.1.0 + t.1.1 }"),
        "5"
    );
}

#[test]
fn patterns() {
    let p = |body: &str| shown(&format!("{SHAPES}{body}"));
    assert_eq!(
        p(
            "fn f(n: int) -> int { match n { 0 => 0, 1..=9 => 1, 10..100 => 2, _ => 3 } }\n\
           fn main() -> (int, int, int, int, int) { (f(0), f(9), f(10), f(100), f(-5)) }"
        ),
        "(0, 1, 2, 3, 3)"
    );
    assert_eq!(
        p(
            "fn f(s: Shape) -> int { match s { Shape::Rect { w: n @ 1..=3, .. } if n > 1 => n, Shape::Rect { .. } | Shape::Dot => -1, Shape::Circle(r) => r } }\n\
           fn main() -> (int, int, int, int) { (f(Shape::Rect { w: 2, h: 0 }), f(Shape::Rect { w: 1, h: 0 }), f(Shape::Dot), f(Shape::Circle(7))) }"
        ),
        "(2, -1, -1, 7)"
    );
    assert_eq!(
        p(
            "fn main() -> int { let (a, .., z) = (1, 2, 3, 4); let Point { x, .. } = Point { x: 5, y: 6 }; a + z + x }"
        ),
        "10"
    );
    assert_eq!(
        p(
            "fn f(s: Shape) -> int { let Shape::Circle(r) = s else { return -1; }; r }\n\
           fn main() -> (int, int) { (f(Shape::Circle(3)), f(Shape::Dot)) }"
        ),
        "(3, -1)"
    );
    assert_eq!(
        p(
            "fn f(s: Shape) -> int { if let Shape::Circle(r) = s && r > 2 { r } else { 0 } }\n\
           fn main() -> (int, int, int) { (f(Shape::Circle(3)), f(Shape::Circle(1)), f(Shape::Dot)) }"
        ),
        "(3, 0, 0)"
    );
    assert_eq!(
        p(
            "fn main() -> int { let x = 5; match (x, true) { (y, false) => y, (y, true) => y * 2 } }"
        ),
        "10"
    );
}

#[test]
fn loops() {
    assert_eq!(
        shown(
            "fn main() -> int { let mut i = 0; let mut sum = 0; while i < 10 { i += 1; if i % 2 == 0 { continue; } sum += i; } sum }"
        ),
        "25"
    );
    assert_eq!(
        shown("fn main() -> int { let mut i = 1; loop { i *= 2; if i > 100 { break i; } } }"),
        "128"
    );
    assert_eq!(
        shown(
            "fn main() -> int { let mut n = 0; 'outer: loop { loop { n += 1; if n == 3 { break 'outer; } } } n }"
        ),
        "3"
    );
    assert_eq!(
        shown(
            "enum L { Nil, One(int) }\nfn main() -> int { let mut l = L::One(4); let mut n = 0; while let L::One(k) = l { n += k; l = L::Nil; } n }"
        ),
        "4"
    );
}

#[test]
fn a_failed_assert_shows_its_parts() {
    let db = Database::default();
    let text = "fn fee(a: int) -> int { a / 10 }\n\
                fn main() { let amount = 250; assert(fee(amount) + amount == 260 || amount < 0 && fee(1) > 0); }";
    let file = Program::single(&db, "t.moddil".to_string(), text.to_string());
    let items = moddil_sema::item_tree(&db, file);
    let main = items.main_fn().expect("a main");
    let Err(Stop::Trap(t)) = call_fn(&db, file, main, &[], &mut |_, _, _| {}) else {
        panic!("a trap");
    };
    assert_eq!(t.class, TrapClass::Assertion);
    let parts: Vec<(String, String)> = t
        .parts
        .iter()
        .map(|(r, v)| {
            (
                text[r.to_range()].to_string(),
                v.as_ref()
                    .map_or("not evaluated".to_string(), |v| v.render(items)),
            )
        })
        .collect();
    let find = |s: &str| {
        parts
            .iter()
            .find(|(p, _)| p == s)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    assert_eq!(find("fee(amount) + amount"), "275");
    assert_eq!(find("amount < 0"), "false");
    assert_eq!(find("fee(1) > 0"), "not evaluated");
    assert!(value("fn main() { assert(true); }") == Value::Unit);
}

#[test]
fn generics_and_traits() {
    let t = "trait Area { fn area(&self) -> int; }\n\
             struct Sq { s: int }\n\
             impl Area for Sq { fn area(&self) -> int { self.s * self.s } }\n\
             fn total<T: Area>(a: &T, b: &T) -> int { a.area() + b.area() }\n\
             fn main() -> int { total(&Sq { s: 2 }, &Sq { s: 3 }) }";
    assert_eq!(value(t), Value::Int(13));
    assert_eq!(
        value(
            "fn sum<T: Add<Output = T>>(a: T, b: T) -> T { a + b }\nfn main() -> int { sum(2, 5) }"
        ),
        Value::Int(7)
    );
    // An overflow in the prelude's `add` is reported where the file's code called it.
    let t = "fn sum<T: Add<Output = T>>(a: T, b: T) -> T { a + b }\n\
             fn main() -> int { sum(9223372036854775807, 1) }";
    let e = eval(t, &[]).expect_err("a trap");
    assert_eq!(e.class, TrapClass::Overflow);
    assert_eq!(&t[e.range.start as usize..e.range.end as usize], "a + b");
}

#[test]
fn destructors_run() {
    let t = "struct G { armed: bool }\n\
             impl Drop for G { fn drop(self) { assert(!self.armed); } }\n\
             fn main() -> int { let g = G { armed: true }; 1 }";
    assert_eq!(trap(t), TrapClass::Assertion);
    let t = "struct G { armed: bool }\n\
             impl Drop for G { fn drop(self) { assert(!self.armed); } }\n\
             fn keep(g: G) -> G { g }\n\
             fn main() -> bool { let g = keep(G { armed: false }); g.armed }";
    assert_eq!(value(t), Value::Bool(false));
}

#[test]
fn errors_return_through_their_edge() {
    let t = "struct E { n: int }\n\
             fn check(n: int) -> int with fail(E) { if n > 9 { fail(E { n }); } n }\n\
             fn main() -> int { try { check(12)? } catch { E(e) => e.n + 1 } }";
    assert_eq!(value(t), Value::Int(13));
    let t = "struct E { n: int }\n\
             fn check(n: int) -> int with fail(E) { if n > 9 { fail(E { n }); } n }\n\
             fn main() -> int { check(12) ?! \"small\" }";
    assert_eq!(trap(t), TrapClass::Unexpected);
}

#[test]
fn unreachable_and_todo_trap_with_their_classes() {
    let t = "fn main() -> int { if 1 < 2 { unreachable(\"1 is not less than 2\") } else { 3 } }";
    assert_eq!(trap(t), TrapClass::Unreachable);
    let t = "fn main() -> int { todo(\"the answer\") }";
    assert_eq!(trap(t), TrapClass::Unimplemented);
}

#[test]
fn a_method_on_an_error_set_runs_the_error_s_own_impl() {
    // `e` is `Missing | Bad`: the method is the one of the error's own type.
    let t = "trait Answer { fn answer(self) -> int; }\n\
             struct Missing;\n\
             impl Answer for Missing { fn answer(self) -> int { 404 } }\n\
             struct Bad;\n\
             impl Answer for Bad { fn answer(self) -> int { 400 } }\n\
             fn serve<F: Fn(int) -> int>(f: F, x: int) -> int where F::Fails: Answer {\n\
                 try { f(x)? } catch { e => e.answer() }\n\
             }\n\
             fn lookup(x: int) -> int with fail(Missing | Bad) {\n\
                 if x == 0 { fail(Missing) }\n\
                 if x < 0 { fail(Bad) }\n\
                 x\n\
             }\n\
             fn main() -> int { serve(lookup, 0) * 10000 + serve(lookup, -1) * 10 + serve(lookup, 3) }";
    assert_eq!(value(t), Value::Int(4044003));
}

#[test]
fn handlers_are_passed_and_called_through_dyn() {
    let t = "trait Time { fn now(&self) -> int; }\n\
             trait Steady: Time {}\n\
             struct Clock { t: int }\n\
             impl Time for Clock { fn now(&self) -> int { self.t } }\n\
             impl Steady for Clock {}\n\
             fn uses(n: int) -> int with time: Time { time.now() + n }\n\
             fn passes() -> int { uses(1) }\n\
             fn up(s: &dyn Steady) -> int { with Time = s { passes() } }\n\
             fn main() -> int { let c = Clock { t: 4 }; with Time = &c { passes() * 10 + up(&c) } }";
    assert_eq!(value(t), Value::Int(55));
}

#[test]
fn closures_capture_and_run() {
    let t = "fn each(f: impl FnMut(int), a: int, b: int) { f(a); f(b); }\n\
             fn main() -> int { let k = 10; let mut n = 0; each(|x| n += x * k, 1, 2); n }";
    assert_eq!(value(t), Value::Int(30));
}
