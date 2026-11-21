use super::*;

/// The values observed by each test in `text`, and where each trapped.
fn runs(text: &str) -> Vec<(Vec<String>, Option<TrapClass>)> {
    run_tests(text)
        .expect("a checked program")
        .into_iter()
        .map(|r| {
            (
                r.observed.into_iter().map(|(_, _, v)| v).collect(),
                r.trap.map(|t| t.class),
            )
        })
        .collect()
}

#[test]
fn version_has_three_parts() {
    assert_eq!(VERSION.split('.').count(), 3);
}

#[test]
fn dependencies_are_wired() {
    assert_eq!(moddil_syntax::VERSION, VERSION);
}

#[test]
fn scalars_and_calls() {
    let text = "fn fib(n: int) -> int { if n < 2 { n } else { fib(n - 1) + fib(n - 2) } }\n\
                test \"t\" {\n\
                    fib(15); // => 610\n\
                    -9223372036854775808 // => -9223372036854775808\n\
                    ;\n\
                    1 < 2 <= 2 // => true\n\
                }\n";
    assert_eq!(
        runs(text),
        vec![(
            vec![
                "610".to_string(),
                "-9223372036854775808".to_string(),
                "true".to_string()
            ],
            None
        )]
    );
}

#[test]
fn traps() {
    let text = "fn down(n: int) -> int { down(n + 1) }\n\
                test \"a\" { 9223372036854775807 + 1; }\n\
                test \"b\" { 1 / 0; }\n\
                test \"c\" { 1 << 64; }\n\
                test \"d\" { down(0); }\n\
                test \"e\" { -9223372036854775807 - 1 % -1; }\n";
    let classes: Vec<Option<TrapClass>> = runs(text).into_iter().map(|(_, t)| t).collect();
    assert_eq!(
        classes,
        vec![
            Some(TrapClass::Overflow),
            Some(TrapClass::DivisionByZero),
            Some(TrapClass::Overflow),
            Some(TrapClass::StackOverflow),
            None,
        ]
    );
}

#[test]
fn data_and_patterns() {
    let text = "struct Point { x: int, y: int }\n\
                enum Shape { Dot, Circle(int), Rect { w: int, h: int } }\n\
                fn area(s: Shape) -> int {\n\
                    match s { Shape::Dot => 0, Shape::Circle(r) => 3 * r * r, Shape::Rect { w, h } => w * h }\n\
                }\n\
                test \"t\" {\n\
                    Point { y: 2, x: 1 } // => Point { x: 1, y: 2 }\n\
                    ;\n\
                    area(Shape::Circle(2)) + area(Shape::Rect { w: 2, h: 5 }) // => 22\n\
                    ;\n\
                    let mut p = Point { x: 1, y: 2 };\n\
                    p.x += 10;\n\
                    p // => Point { x: 11, y: 2 }\n\
                    ;\n\
                    let (a, .., z) = (1, 2, 3);\n\
                    (z, a, (true,)) // => (3, 1, (true,))\n\
                    ;\n\
                    match 7 { 0 => 0, n @ 1..=9 if n > 5 => n, _ => -1 } // => 7\n\
                    ;\n\
                    let mut i = 0;\n\
                    'out: loop { while true { i += 1; if i == 3 { break 'out; } } }\n\
                    i // => 3\n\
                }\n";
    assert_eq!(
        runs(text),
        vec![(
            vec![
                "Point { x: 1, y: 2 }".to_string(),
                "22".to_string(),
                "Point { x: 11, y: 2 }".to_string(),
                "(3, 1, (true,))".to_string(),
                "7".to_string(),
                "3".to_string(),
            ],
            None
        )]
    );
}

#[test]
fn traits_derives_and_drops() {
    let text = "#[derive(Clone, PartialEq, PartialOrd)]\n\
                enum E { A(int), B }\n\
                struct G { armed: bool }\n\
                impl Drop for G { fn drop(self) { assert(!self.armed); } }\n\
                fn big<T: PartialOrd>(a: T, b: T) -> T { if a < b { b } else { a } }\n\
                test \"t\" {\n\
                    big(E::A(3), E::B) // => E::B\n\
                    ;\n\
                    E::A(1) == E::A(1).clone() // => true\n\
                    ;\n\
                    Some(2).unwrap_or(0) // => 2\n\
                }\n\
                test \"u\" { let g = G { armed: true }; }\n";
    assert_eq!(
        runs(text),
        vec![
            (
                vec!["E::B".to_string(), "true".to_string(), "2".to_string()],
                None
            ),
            (vec![], Some(TrapClass::Assertion)),
        ]
    );
}

#[test]
fn errors() {
    let text = "struct E { n: int }\n\
                fn check(n: int) -> int with fail(E) { if n > 9 { fail(E { n }); } n }\n\
                test \"t\" {\n\
                    try { check(12)? } catch { E(e) => e.n } // => 12\n\
                    ;\n\
                    try { check(1)? } // => Ok(1)\n\
                    ;\n\
                    check(3) ?! \"small\"; check(20) ?! \"small\";\n\
                }\n";
    assert_eq!(
        runs(text),
        vec![(
            vec!["12".to_string(), "Ok(1)".to_string()],
            Some(TrapClass::Unexpected)
        )]
    );
}

#[test]
fn effects() {
    let text = "trait Time { fn now(&self) -> int; }\n\
                struct Clock { t: int }\n\
                impl Time for Clock { fn now(&self) -> int { self.t } }\n\
                fn uses(n: int) -> int with time: Time { time.now() + n }\n\
                fn passes() -> int { uses(1) }\n\
                test \"t\" {\n\
                    let a = Clock { t: 1 };\n\
                    let b = Clock { t: 10 };\n\
                    with Time = &a { passes() // => 2\n\
                    ; with Time = &b { passes() } // => 11\n\
                    }\n\
                }\n";
    assert_eq!(
        runs(text),
        vec![(vec!["2".to_string(), "11".to_string()], None)]
    );
}

#[test]
fn modules_and_mods() {
    let file = |text: &str, mod_: u32, module: &[&str]| SourceSpec {
        text: text.to_string(),
        mod_,
        module: module.iter().map(|s| (*s).to_string()).collect(),
    };
    let runs = run_program(
        vec![
            file(
                "use crate::a::Point;\ntest \"t\" {\n    a::norm(Point { x: 2 }) + dep::three() // => 5\n}\n",
                0,
                &[],
            ),
            file("pub(mod) struct Point { x: int }\npub(mod) fn norm(p: Point) -> int { p.x }\n", 0, &["a"]),
            file("pub fn three() -> int { 3 }\n", 1, &[]),
        ],
        vec![
            ModSpec {
                deps: vec![("dep".to_string(), 1)],
            },
            ModSpec::default(),
        ],
    )
    .expect("a checked program");
    assert_eq!(runs.len(), 1);
    assert_eq!(
        runs[0]
            .observed
            .iter()
            .map(|(f, _, v)| (*f, v.as_str()))
            .collect::<Vec<_>>(),
        vec![(0, "5")]
    );
}
