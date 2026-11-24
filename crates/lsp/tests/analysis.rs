//! The language server's answers (`moddil-lsp`'s `Analysis`), over a small program written to a
//! file of its own.

use std::path::PathBuf;

use moddil_lsp::{Analysis, CompletionKind};

const PROGRAM: &str = "\
pub struct Point {
    pub x: int,
    pub y: int,
}

impl Point {
    pub fn sum(&self) -> int {
        self.x + self.y
    }
}

pub enum Shape {
    Circle,
    Square,
}

fn twice(n: int) -> int {
    n * 2
}

fn wait(r: Receiver<int>) -> int with suspend {
    r.recv().unwrap_or(0)
}

test \"points\" {
    let p = Point { x: 1, y: 2 };
    let s = p.sum();
    assert(twice(s) == 6);
}
";

/// `text` written as `name.moddil` in a directory of its own, and an analysis of it.
fn open(name: &str, text: &str) -> (Analysis, PathBuf) {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("lsp")
        .join(name);
    std::fs::create_dir_all(&dir).expect("a directory");
    let path = dir.join(format!("{name}.moddil"));
    std::fs::write(&path, text).expect("written");
    let mut a = Analysis::new();
    assert!(a.set_text(&path, text), "the file is a program");
    (a, path)
}

/// The byte offset of the `n`th (from 0) `needle` in `text`, plus `plus`.
fn at(text: &str, needle: &str, n: usize, plus: usize) -> u32 {
    let (i, _) = text
        .match_indices(needle)
        .nth(n)
        .expect("the needle is there");
    (i + plus) as u32
}

#[test]
fn a_program_without_errors_has_no_problems_and_one_with_has_its_code() {
    let (mut a, path) = open("clean", PROGRAM);
    let problems = a.problems(&path);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].1.is_empty(), "{problems:?}");
    // An edit brings a type error; the next one fixes it.
    let broken = PROGRAM.replace("n * 2", "true");
    a.set_text(&path, &broken);
    let problems = a.problems(&path);
    let p = &problems[0].1;
    assert_eq!(p.len(), 1, "{p:?}");
    assert!(p[0].error && p[0].code.starts_with('E'), "{p:?}");
    assert_eq!(p[0].range.start, at(&broken, "true", 0, 0));
    a.set_text(&path, PROGRAM);
    assert!(a.problems(&path)[0].1.is_empty());
}

#[test]
fn hover_shows_types_signatures_and_effects() {
    let (mut a, path) = open("hover", PROGRAM);
    let hover = |a: &mut Analysis, off| a.hover(&path, off).unwrap_or_default();
    // A local, where it is used and where it is bound.
    let h = hover(&mut a, at(PROGRAM, "p.sum", 0, 0));
    assert!(h.contains("p: Point"), "{h}");
    let h = hover(&mut a, at(PROGRAM, "let p", 0, 4));
    assert!(h.contains("p: Point"), "{h}");
    // A method call, and a function called.
    let h = hover(&mut a, at(PROGRAM, "p.sum", 0, 3));
    assert!(h.contains("fn Point::sum(&self) -> int"), "{h}");
    let h = hover(&mut a, at(PROGRAM, "twice(s)", 0, 1));
    assert!(h.contains("fn twice(n: int) -> int"), "{h}");
    // A field, and a function's own name with what it needs.
    let h = hover(&mut a, at(PROGRAM, "self.x", 0, 5));
    assert!(h.contains("x: int"), "{h}");
    let h = hover(&mut a, at(PROGRAM, "fn wait", 0, 4));
    assert!(h.contains("with suspend"), "{h}");
    // A type named in a signature.
    let h = hover(&mut a, at(PROGRAM, "Receiver<int>", 0, 2));
    assert!(h.contains("struct Receiver"), "{h}");
}

#[test]
fn definitions_of_locals_functions_fields_and_methods() {
    let (mut a, path) = open("definition", PROGRAM);
    let def = |a: &mut Analysis, off| a.definition(&path, off).map(|(_, r)| r.start);
    assert_eq!(
        def(&mut a, at(PROGRAM, "twice(s)", 0, 2)),
        Some(at(PROGRAM, "twice", 0, 0))
    );
    assert_eq!(
        def(&mut a, at(PROGRAM, "p.sum", 0, 0)),
        Some(at(PROGRAM, "p = Point", 0, 0))
    );
    assert_eq!(
        def(&mut a, at(PROGRAM, "p.sum", 0, 3)),
        Some(at(PROGRAM, "sum", 0, 0))
    );
    assert_eq!(
        def(&mut a, at(PROGRAM, "self.y", 0, 5)),
        Some(at(PROGRAM, "y: int", 0, 0))
    );
    // A type in a struct literal, through the name under the cursor.
    assert_eq!(
        def(&mut a, at(PROGRAM, "Point { x: 1", 0, 1)),
        Some(at(PROGRAM, "Point", 0, 0))
    );
}

#[test]
fn definitions_in_the_prelude_are_in_its_files() {
    let (mut a, path) = open("library", PROGRAM);
    let mut def = |off| {
        let (file, r) = a.definition(&path, off).expect("a definition");
        let text = a.text(&file).expect("its text");
        (file, text[r.start as usize..r.end as usize].to_string())
    };
    let (file, name) = def(at(PROGRAM, "Receiver", 0, 1));
    assert!(
        file.ends_with("reference/2027/prelude/concurrency.moddil"),
        "{}",
        file.display()
    );
    assert_eq!(name, "Receiver");
    let (_, name) = def(at(PROGRAM, "unwrap_or", 0, 1));
    assert_eq!(name, "unwrap_or");
}

#[test]
fn hints_mark_the_calls_that_suspend() {
    let (mut a, path) = open("hints", PROGRAM);
    let hints = a.hints(&path);
    let end = at(PROGRAM, "r.recv()", 0, "r.recv()".len());
    assert!(
        hints
            .iter()
            .any(|h| h.offset == end && h.label == "suspends"),
        "{hints:?}"
    );
    // Only calls that may suspend.
    assert!(hints.iter().all(|h| h.label == "suspends"), "{hints:?}");
}

#[test]
fn completion_after_a_dot_a_path_and_in_scope() {
    let typed = PROGRAM.replace("let s = p.sum();", "let s = p.su");
    let (mut a, path) = open("complete", &typed);
    let labels = |a: &mut Analysis, off| -> Vec<(String, CompletionKind)> {
        a.complete(&path, off)
            .into_iter()
            .map(|c| (c.label, c.kind))
            .collect()
    };
    // A method, by the name being typed.
    let got = labels(&mut a, at(&typed, "p.su", 0, 4));
    assert_eq!(got, vec![("sum".to_string(), CompletionKind::Method)]);
    // Everything after the dot: fields and methods.
    let got = labels(&mut a, at(&typed, "p.su", 0, 2));
    for want in ["x", "y", "sum"] {
        assert!(got.iter().any(|(l, _)| l == want), "{want} in {got:?}");
    }
    // The text is as it was.
    assert_eq!(a.text(&path).as_deref(), Some(typed.as_str()));

    // After `::`: an enum's variants.
    let path_typed = PROGRAM.replace("assert(twice(s) == 6);", "let k = Shape::");
    a.set_text(&path, &path_typed);
    let got = labels(&mut a, at(&path_typed, "Shape::", 0, 7));
    assert!(
        got.contains(&("Circle".to_string(), CompletionKind::Variant)),
        "{got:?}"
    );
    assert!(
        got.contains(&("Square".to_string(), CompletionKind::Variant)),
        "{got:?}"
    );

    // A name in scope: a local, a function of the module, the prelude's.
    let scope_typed = PROGRAM.replace("assert(twice(s) == 6);", "let t = tw");
    a.set_text(&path, &scope_typed);
    let got = labels(&mut a, at(&scope_typed, "= tw", 0, 4));
    assert_eq!(got, vec![("twice".to_string(), CompletionKind::Function)]);
    let got = labels(&mut a, at(&scope_typed, "= tw", 0, 2));
    assert!(
        got.contains(&("p".to_string(), CompletionKind::Local)),
        "{got:?}"
    );
    assert!(
        got.contains(&("s".to_string(), CompletionKind::Local)),
        "{got:?}"
    );
    assert!(got.iter().any(|(l, _)| l == "Vec"), "the prelude's names");
}

/// Completion offers the locals in scope where the name is typed: not those of a block that has
/// ended, a match arm's or a loop's after them, and of two of one name the later one.
#[test]
fn completion_offers_only_the_locals_in_scope() {
    let typed = "\
fn f() -> int {
    let a = 1;
    {
        let inner = 2;
    }
    let b = match a { n => n + 1 };
    for k in 0..3 {}
    let a = \"shadowed\";
    let c = 
    0
}
";
    let (mut a, path) = open("scoped", typed);
    let got: Vec<(String, String)> = a
        .complete(&path, at(typed, "let c = ", 0, 8))
        .into_iter()
        .filter(|c| c.kind == CompletionKind::Local)
        .map(|c| (c.label, c.detail))
        .collect();
    let names: Vec<&str> = got.iter().map(|(l, _)| l.as_str()).collect();
    assert_eq!(names, vec!["a", "b"], "{got:?}");
    assert_eq!(got[0].1, "&str", "the later `a`: {got:?}");
}

/// A name in a type is resolved by its whole path, as the compiler does: `HashMap` in
/// `core::collections::HashMap` is `core`'s, not the file's own `HashMap`.
#[test]
fn a_name_in_a_type_goes_by_its_path() {
    let text = "\
struct HashMap {
    x: int,
}

fn size(m: &core::collections::HashMap<int, int>, own: HashMap) -> int {
    m.len() + own.x
}
";
    let (mut a, path) = open("paths", text);
    let theirs = a
        .definition(&path, at(text, "collections::HashMap", 0, 14))
        .expect("core's HashMap");
    let canon = |p: &PathBuf| std::fs::canonicalize(p).expect("a file");
    assert_ne!(
        canon(&theirs.0),
        canon(&path),
        "core's, in its own file: {theirs:?}"
    );
    let ours = a
        .definition(&path, at(text, "own: HashMap", 0, 6))
        .expect("the file's HashMap");
    assert_eq!(
        (canon(&ours.0), ours.1.start),
        (canon(&path), at(text, "HashMap {", 0, 0))
    );
    let hover = a
        .hover(&path, at(text, "collections::HashMap", 0, 14))
        .unwrap_or_default();
    assert!(!hover.contains("x: int"), "{hover}");
}
