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

/// The line (from 0) of `text` a trap is at.
fn trap_line(text: &str) -> usize {
    match run(text) {
        Err(Native::Trap(_, _, range)) => text[..range.start as usize].matches('\n').count(),
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
        run("fn main() -> Option<i8> { 300.checked_to::<i8>() }"),
        Err(Native::Unsupported(_))
    ));
}

#[test]
fn structs_tuples_and_arrays() {
    assert_eq!(
        value(
            "struct P { x: int, y: bool }  fn main() -> int { let p = P { x: 4, y: true }; if p.y { p.x } else { 0 } }"
        ),
        "4"
    );
    assert_eq!(
        value("fn main() -> (int, bool) { (3, false) }"),
        "(3, false)"
    );
    assert_eq!(
        value("struct P { x: u8, y: int, z: u8 }  fn main() -> P { P { x: 1, y: 2, z: 3 } }"),
        "P { x: 1, y: 2, z: 3 }"
    );
    assert_eq!(
        value("fn main() -> int { let mut a = [1, 2, 3]; a[1] = 20; a[0] + a[1] + a[2] }"),
        "24"
    );
    assert_eq!(value("fn main() -> [int; 3] { [7; 3] }"), "[7, 7, 7]");
    assert_eq!(
        trap("fn main() -> int { let a = [1, 2, 3]; let i = 3; a[i] }"),
        TrapClass::IndexOutOfBounds
    );
}

#[test]
fn enums_and_match() {
    let shape = "enum Shape { Circle(int), Rect(int, int), Empty }
        fn area(s: Shape) -> int { match s { Shape::Circle(r) => 3 * r * r, Shape::Rect(w, h) => w * h, Shape::Empty => 0 } }";
    assert_eq!(
        value(&format!(
            "{shape} fn main() -> int {{ area(Shape::Rect(2, 5)) + area(Shape::Circle(1)) }}"
        )),
        "13"
    );
    assert_eq!(
        value(&format!(
            "{shape} fn main() -> Shape {{ Shape::Rect(2, 5) }}"
        )),
        "Shape::Rect(2, 5)"
    );
    assert_eq!(
        value("fn main() -> Option<bool> { Some(false) }"),
        "Some(false)"
    );
    assert_eq!(value("fn main() -> Option<bool> { None }"), "None");
    assert_eq!(
        value("fn f(x: Option<Option<bool>>) -> int { match x { None => 0, Some(None) => 1, Some(Some(b)) => if b { 3 } else { 2 } } }
               fn main() -> int { f(None) * 100 + f(Some(None)) * 10 + f(Some(Some(true))) }"),
        "13"
    );
}

#[test]
fn borrows() {
    assert_eq!(
        value(
            "fn bump(x: &mut int) { *x += 1; }  fn main() -> int { let mut n = 1; bump(&mut n); bump(&mut n); n }"
        ),
        "3"
    );
    assert_eq!(
        value(
            "struct P { x: int, y: int }  fn sum(p: &P) -> int { p.x + p.y }  fn main() -> int { let p = P { x: 2, y: 3 }; sum(&p) }"
        ),
        "5"
    );
    assert_eq!(
        value(
            "fn first(o: &Option<int>) -> int { match o { Some(x) => *x, None => -1 } }  fn main() -> int { let o = Some(9); first(&o) }"
        ),
        "9"
    );
}

#[test]
fn large_values_cross_calls() {
    let big = "struct Big { a: int, b: int, c: int, d: int }
        fn make(n: int) -> Big { Big { a: n, b: n + 1, c: n + 2, d: n + 3 } }
        fn total(b: Big) -> int { b.a + b.b + b.c + b.d }";
    assert_eq!(
        value(&format!("{big} fn main() -> int {{ total(make(10)) }}")),
        "46"
    );
    assert_eq!(
        value(&format!("{big} fn main() -> Big {{ make(1) }}")),
        "Big { a: 1, b: 2, c: 3, d: 4 }"
    );
}

/// The layouts of the types `main`'s locals have, by type name.
fn sizes(text: &str, tys: &[&str]) -> Vec<(u32, u32)> {
    let db = Database::default();
    let file = Program::single(&db, "t.moddil".to_string(), text.to_string());
    let items = moddil_sema::item_tree(&db, file);
    let main = items.main_fn().expect("a main");
    let owner = moddil_sema::Owner::new(&db, file, Item::Fn(main));
    let body = moddil_mir::mir(&db, owner).as_ref().expect("checked");
    let mut layouts = layout::Layouts::new(items);
    tys.iter()
        .map(|name| {
            let ty = body
                .locals
                .iter()
                .map(|l| &l.ty)
                .find(|t| t.to_string() == *name)
                .unwrap_or_else(|| panic!("no local of type {name}"));
            let l = layouts.of(ty).expect("a layout");
            (l.size, l.align)
        })
        .collect()
}

#[test]
fn layouts_reorder_fields_and_use_niches() {
    let text = "struct P { a: u8, b: int, c: u8 }
        enum E { A, B(int), C }
        fn main() {
            let p = P { a: 1, b: 2, c: 3 };
            let o: Option<bool> = None;
            let oo: Option<Option<bool>> = None;
            let r = 5;
            let or: Option<&int> = Some(&r);
            let e = E::A;
            let c: Option<char> = None;
        }";
    let got = sizes(
        text,
        &[
            "P",
            "Option<bool>",
            "Option<Option<bool>>",
            "Option<&int>",
            "E",
            "Option<char>",
        ],
    );
    assert_eq!(
        got,
        vec![(16, 8), (1, 1), (1, 1), (8, 8), (16, 8), (4, 4)],
        "fields largest first, niches in bool, borrows and char, a tag for E"
    );
}

#[test]
fn generic_functions() {
    assert_eq!(
        value("fn id<T>(x: T) -> T { x }  fn main() -> (int, bool) { (id(3), id(true)) }"),
        "(3, true)"
    );
}

#[test]
fn traits_static_and_dyn() {
    let shapes = "trait Shape { fn area(&self) -> int; fn twice(&self) -> int { self.area() * 2 } }
        trait Named: Shape { fn id(&self) -> int; }
        struct Sq { s: int }
        impl Shape for Sq { fn area(&self) -> int { self.s * self.s } }
        impl Named for Sq { fn id(&self) -> int { 7 } }
        fn total<T: Shape>(x: &T) -> int { x.twice() }
        fn by_dyn(x: &dyn Shape) -> int { x.area() }
        fn named(n: &dyn Named) -> int { n.id() * 100 + by_dyn(n) }";
    assert_eq!(
        value(&format!(
            "{shapes} fn main() -> int {{ total(&Sq {{ s: 3 }}) }}"
        )),
        "18"
    );
    assert_eq!(
        value(&format!(
            "{shapes} fn main() -> int {{ by_dyn(&Sq {{ s: 4 }}) }}"
        )),
        "16"
    );
    assert_eq!(
        value(&format!(
            "{shapes} fn main() -> int {{ named(&Sq {{ s: 2 }}) }}"
        )),
        "704"
    );
}

#[test]
fn slices_and_text() {
    assert_eq!(
        value("fn sum(xs: &[int]) -> int { let mut t = 0; let mut i = 0; while i < xs.len() { t += xs[i]; i += 1; } t }
               fn main() -> int { let a = [1, 2, 3, 4]; sum(&a) + sum(&a[1..3]) }"),
        "15"
    );
    assert_eq!(
        value("fn eq(x: &[int], y: &[int]) -> bool { x == y }
               fn main() -> (bool, bool) { let a = [1, 2, 3]; let b = [1, 2]; (eq(&a, &b), eq(&a[0..2], &b)) }"),
        "(false, true)"
    );
    assert_eq!(value(r#"fn main() -> int { "naïve".len() }"#), "6");
    assert_eq!(value(r#"fn main() -> &str { "hi" }"#), "\"hi\"");
    assert_eq!(
        value(
            r#"fn yes(s: &str) -> bool { match s { "y" | "yes" => true, _ => false } }  fn main() -> bool { yes("yes") && !yes("no") }"#
        ),
        "true"
    );
    assert_eq!(
        trap("fn main() -> int { let a = [1, 2, 3]; let s = &a[0..2]; let i = 2; s[i] }"),
        TrapClass::IndexOutOfBounds
    );
}

#[test]
fn float_functions() {
    assert_eq!(value("fn main() -> f64 { 9.0.sqrt() }"), "3.0");
    assert_eq!(value("fn main() -> bool { (0.0 / 0.0).is_nan() }"), "true");
}

#[test]
fn vectors_strings_and_boxes() {
    assert_eq!(
        value(
            "fn main() -> Vec<int> { let mut v = Vec::new(); let mut i = 0; while i < 50 { v.push(i); i += 1; } v.pop(); v.remove(0); v.insert(0, 100); let mut w = Vec::new(); w.push(v.len()); w.push(v[0]); w }"
        ),
        "[49, 100]"
    );
    assert_eq!(
        value(
            r#"fn main() -> Vec<String> { let mut v = Vec::new(); v.push("a".to_string()); v.push(String::from_str("a string long enough for the heap")); v }"#
        ),
        r#"["a", "a string long enough for the heap"]"#
    );
    assert_eq!(
        value("fn main() -> int { let b = Box::new(41); *b + 1 }"),
        "42"
    );
    assert_eq!(
        value("trait Shape { fn area(&self) -> int; }  struct Sq { s: int }  impl Shape for Sq { fn area(&self) -> int { self.s * self.s } }
               fn main() -> int { let b: Box<dyn Shape> = Box::new(Sq { s: 3 }); let mut v = Vec::new(); v.push(b); v[0].area() }"),
        "9"
    );
    assert_eq!(
        trap("fn main() -> int { let mut v = Vec::from_array([1]); v.remove(1) }"),
        TrapClass::IndexOutOfBounds
    );
}

#[test]
fn drops_run_in_order_and_once() {
    // Each destructor traps when armed: which line traps says which value was dropped first.
    let guards =
        "struct A { armed: bool }  impl Drop for A { fn drop(self) { assert(!self.armed); } }
        struct B { armed: bool }  impl Drop for B { fn drop(self) { assert(!self.armed); } }";
    // The last declared first.
    assert_eq!(
        trap_line(&format!(
            "{guards}
fn main() {{ let a = A {{ armed: true }}; let b = B {{ armed: true }}; }}"
        )),
        1
    );
    // A moved value is dropped where it goes, once.
    assert_eq!(
        value(&format!("{guards}
fn keep(a: A) -> A {{ a }}  fn main() -> int {{ let a = A {{ armed: false }}; let b = keep(a); let c = b; 3 }}")),
        "3"
    );
    // A part moved out of is not dropped with the rest.
    assert_eq!(
        trap_line(&format!("{guards}
struct P {{ a: A, b: B }}  fn main() {{ let p = P {{ a: A {{ armed: false }}, b: B {{ armed: true }} }}; let a = p.a; }}")),
        1
    );
}

#[test]
fn closures_called_every_way() {
    assert_eq!(
        value(
            "fn apply<F: Fn(int) -> int>(f: F, x: int) -> int { f(x) }  fn main() -> int { let k = 10; apply(|x| x + k, 5) }"
        ),
        "15"
    );
    assert_eq!(
        value(
            "fn main() -> int { let mut n = 0; let mut inc = |by: int| { n += by; }; inc(2); inc(3); n }"
        ),
        "5"
    );
    assert_eq!(
        value(
            "fn main() -> int { let v = Vec::from_array([1, 2, 3]); let f = || { let w = v; w.len() }; f() }"
        ),
        "3"
    );
    assert_eq!(
        value(
            "fn counter() -> impl FnMut() -> int { let mut n = 0; || { n += 1; n } }  fn main() -> int { let mut c = counter(); c(); c() }"
        ),
        "2"
    );
    assert_eq!(
        value(
            "fn main() -> int { let f: Box<dyn Fn(int) -> int> = Box::new(|x| x * 3); let g: Box<dyn FnOnce() -> int> = Box::new(|| 7); f(4) + g() }"
        ),
        "19"
    );
}

#[test]
fn everything_allocated_is_freed() {
    // Values that own memory, moved about and dropped: a leak would be `Native::Leak`.
    assert!(run("fn main() -> int {
            let mut v = Vec::new();
            let mut i = 0;
            while i < 20 { v.push(String::from_str(\"a string long enough for the heap\")); i += 1; }
            let w = v;
            let b = Box::new(w);
            let f = || { let x = b; x.len() };
            f()
        }")
    .is_ok());
}
