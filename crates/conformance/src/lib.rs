//! The conformance runner: runs `conformance/` tests on every implementation and reports any
//! disagreement with their expectations.
//!
//! A test is a `.moddil` file, or a directory with a `moddil.toml`: a mod, with its files and the
//! mods it depends on by path. Its files state its expectations in comments:
//!
//! | Comment | Expects |
//! |---|---|
//! | `// covers: expr.arith.overflow.1, …` | The reference paragraphs the test covers |
//! | `// => 3` | The expression statement or final expression it follows evaluates to `3` (in a `test` item) |
//! | `//~ E0201` | A diagnostic with this code starting on this line (several codes may follow); warnings have `W` codes |
//! | `//~ trap` | A trap on this line |
//! | `//~ error` | An error leaves a test on this line, failing it |
//! | `// disputed` | The test does not count until the reference is clarified |
//! | `// native` | It calls C: only native code and the MIR interpreter run it |
//!
//! In a manifest, `#~ E0705` expects a diagnostic on its line. A test with expected diagnostics
//! is only checked; otherwise every `test` item of every mod in it runs, every `// =>` line must
//! run and produce exactly its value, and traps happen exactly on `//~ trap` lines. See this
//! crate's `README.md`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use moddil_project::Project;
use moddil_sema::Program;
use moddil_syntax::{SyntaxKind, line_col};

/// The edition the suite currently tests.
pub const EDITION: &str = "2027";

/// A line of a test: its file (as the project shows it) and its number (1-based).
pub type Line = (String, u32);

/// What a test expects, by line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Expectations {
    /// Reference paragraph IDs the test covers.
    pub covers: Vec<String>,
    /// `// =>` values.
    pub values: BTreeMap<Line, String>,
    /// The other values a `// =>` line may have under the parallel runtime, whose tasks run in
    /// whatever order: `// => v (in parallel, also w; x)`, or, for `(in parallel, in any
    /// order)`, none but its parts in another order.
    pub parallel: BTreeMap<Line, Vec<String>>,
    /// The `// =>` lines whose value's parts may come in any order under the parallel runtime.
    pub any_order: BTreeSet<Line>,
    /// `//~ E…` codes.
    pub diagnostics: BTreeSet<(Line, String)>,
    /// `//~ trap` lines.
    pub traps: BTreeSet<Line>,
    /// `//~ error` lines: where an error leaves a test, failing it.
    pub errors: BTreeSet<Line>,
    /// Marked `// disputed`.
    pub disputed: bool,
    /// Marked `// native`: it calls C (`ffi.extern.1`), which runs only in native code and on
    /// the MIR interpreter, so WASM and the reference interpreter skip it.
    pub native: bool,
}

/// What an implementation did with a test, by line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// The diagnostics: line and code.
    pub diagnostics: BTreeSet<(Line, String)>,
    /// Observed values: the line the expression ends on, and the value as Moddil source.
    pub observed: Vec<(Line, String)>,
    /// The lines traps happened on (where the failing check starts), one per trapping test.
    pub traps: BTreeSet<Line>,
    /// The lines errors left tests on, one per test that failed with one.
    pub errors: BTreeSet<Line>,
}

/// A test read from disk: the program, or the problems reading it.
pub type Loaded = Result<Project, Vec<moddil_project::Problem>>;

/// An implementation of Moddil the suite can run.
pub trait Implementation: Sync {
    /// Its name, for reports.
    fn name(&self) -> &str;
    /// Checks the program and, if it has no errors, runs all its tests.
    fn run(&self, program: &Loaded) -> Outcome;
    /// As [`Implementation::run`], or `None` if it cannot run this program yet (the native
    /// backend, for code it does not compile yet): the file is then skipped for it.
    fn run_if(&self, program: &Loaded) -> Option<Outcome> {
        Some(self.run(program))
    }
    /// Whether it checks programs. One that does not (the reference interpreter) only runs tests
    /// that expect no diagnostics.
    fn checks(&self) -> bool {
        true
    }
    /// Whether it runs tasks in parallel, so that a `// =>` line may also have the values it
    /// lists `in parallel`.
    fn parallel(&self) -> bool {
        false
    }
}

/// The line `offset` of a project file is on.
fn line_of(project: &Project, file: u32, offset: u32) -> Line {
    let f = &project.files[file as usize];
    (f.display.clone(), line_col(&f.text, offset).0)
}

/// The compiler's front end with the MIR interpreter.
pub struct MirInterpreter;

impl Implementation for MirInterpreter {
    fn name(&self) -> &str {
        "MIR interpreter"
    }

    fn run(&self, loaded: &Loaded) -> Outcome {
        let project = match loaded {
            Ok(p) => p,
            Err(problems) => {
                let mut outcome = Outcome::default();
                for p in problems {
                    let line = line_col(&p.text, p.range.start as u32).0;
                    outcome
                        .diagnostics
                        .insert(((p.display.clone(), line), p.code.to_string()));
                }
                return outcome;
            }
        };
        let db = moddil_interp::database();
        let program = Program::of_project(&db, project);
        let mut outcome = Outcome {
            diagnostics: moddil_interp::diagnostics(&db, program)
                .iter()
                .map(|d| {
                    (
                        line_of(project, d.file, d.range.start),
                        d.code.as_str().to_string(),
                    )
                })
                .collect(),
            ..Outcome::default()
        };
        if moddil_interp::has_errors(&db, program) {
            return outcome;
        }
        let items = moddil_sema::item_tree(&db, program);
        for i in 0..items.tests.len() as u32 {
            let mut observed = Vec::new();
            let result = moddil_interp::run_test(&db, program, i, &mut |file, range, value| {
                observed.push((line_of(project, file, range.end), value.render(items)));
            });
            outcome.observed.extend(observed);
            match result {
                Ok(()) => {}
                Err(moddil_interp::Stop::Trap(trap)) => {
                    outcome
                        .traps
                        .insert(line_of(project, trap.file, trap.range.start));
                }
                Err(moddil_interp::Stop::Failed { file, range, .. }) => {
                    outcome.errors.insert(line_of(project, file, range.start));
                }
            }
        }
        outcome
    }
}

/// The native backend (M1): the front end, then each test compiled by Cranelift and run in
/// process. A file whose tests reach code it does not compile yet is skipped.
pub struct Native;

/// Native code under the parallel runtime (`task.runtime.1`): its tasks run at once.
pub struct NativeParallel;

impl Implementation for Native {
    fn name(&self) -> &str {
        "native"
    }

    fn run(&self, loaded: &Loaded) -> Outcome {
        self.run_if(loaded).unwrap_or_default()
    }

    fn run_if(&self, loaded: &Loaded) -> Option<Outcome> {
        run_native(loaded, moddil_native::Options::default())
    }

    fn checks(&self) -> bool {
        false
    }
}

impl Implementation for NativeParallel {
    fn name(&self) -> &str {
        "native, parallel"
    }

    fn run(&self, loaded: &Loaded) -> Outcome {
        self.run_if(loaded).unwrap_or_default()
    }

    fn run_if(&self, loaded: &Loaded) -> Option<Outcome> {
        let options = moddil_native::Options {
            parallel: true,
            ..moddil_native::Options::default()
        };
        run_native(loaded, options)
    }

    fn checks(&self) -> bool {
        false
    }

    fn parallel(&self) -> bool {
        true
    }
}

/// A file's tests natively, with `options`; `None` if native code does not compile it yet.
fn run_native(loaded: &Loaded, options: moddil_native::Options) -> Option<Outcome> {
    let project = loaded.as_ref().ok()?;
    let db = moddil_interp::database();
    let program = Program::of_project(&db, project);
    if moddil_interp::has_errors(&db, program) {
        return None;
    }
    let items = moddil_sema::item_tree(&db, program);
    let mut outcome = Outcome::default();
    for i in 0..items.tests.len() as u32 {
        let (observed, result) = moddil_native::run_test_with(&db, program, i, options);
        outcome.observed.extend(
            observed
                .into_iter()
                .map(|(file, range, value)| (line_of(project, file, range.end), value)),
        );
        match result {
            Ok(()) => {}
            Err(moddil_native::Native::Trap(_, file, range, _)) => {
                outcome.traps.insert(line_of(project, file, range.start));
            }
            Err(moddil_native::Native::Failed(file, range, _)) => {
                outcome.errors.insert(line_of(project, file, range.start));
            }
            // A leak is a disagreement: no interpreter observes one.
            Err(moddil_native::Native::Leak(n)) => {
                outcome.observed.push((
                    line_of(project, 0, 0),
                    format!("native code leaked {n} allocations"),
                ));
            }
            Err(moddil_native::Native::Unsupported(why)) => {
                // `MODDIL_NATIVE_WHY=1` says what keeps each file from running natively.
                if std::env::var_os("MODDIL_NATIVE_WHY").is_some() {
                    let name = project.files.first().map_or("", |f| f.display.as_str());
                    eprintln!("native skips {name}: {why}");
                }
                return None;
            }
        }
    }
    Some(outcome)
}

/// The WASM backend (M1): the front end, then a file's tests compiled into one WebAssembly
/// module and run in Node. A file whose tests reach code it does not compile yet is skipped, and
/// so is every file where there is no Node.
pub struct Wasm;

impl Implementation for Wasm {
    fn name(&self) -> &str {
        "wasm"
    }

    fn run(&self, loaded: &Loaded) -> Outcome {
        self.run_if(loaded).unwrap_or_default()
    }

    fn run_if(&self, loaded: &Loaded) -> Option<Outcome> {
        let project = loaded.as_ref().ok()?;
        let db = moddil_interp::database();
        let program = Program::of_project(&db, project);
        if moddil_interp::has_errors(&db, program) {
            return None;
        }
        let items = moddil_sema::item_tree(&db, program);
        let tests: Vec<u32> = (0..items.tests.len() as u32).collect();
        let runs = match moddil_wasm::run_tests(&db, program, &tests, 0) {
            Ok(r) => r,
            Err(why) => {
                // `MODDIL_WASM_WHY=1` says what keeps each file from running as WASM.
                if std::env::var_os("MODDIL_WASM_WHY").is_some() {
                    let name = project.files.first().map_or("", |f| f.display.as_str());
                    eprintln!("wasm skips {name}: {why:?}");
                }
                return None;
            }
        };
        let mut outcome = Outcome::default();
        for (observed, result) in runs {
            outcome.observed.extend(
                observed
                    .into_iter()
                    .map(|(file, range, value)| (line_of(project, file, range.end), value)),
            );
            match result {
                Ok(()) => {}
                Err(moddil_wasm::Wasm::Trap(_, file, range, _)) => {
                    outcome.traps.insert(line_of(project, file, range.start));
                }
                Err(moddil_wasm::Wasm::Failed(file, range, _)) => {
                    outcome.errors.insert(line_of(project, file, range.start));
                }
                // A crash is a disagreement: no interpreter has one.
                Err(e) => outcome.observed.push((
                    line_of(project, 0, 0),
                    format!("the WASM run stopped: {e:?}"),
                )),
            }
        }
        Some(outcome)
    }

    fn checks(&self) -> bool {
        false
    }
}

/// The reference interpreter. It runs checked programs and does not check them itself.
pub struct ReferenceInterpreter;

impl Implementation for ReferenceInterpreter {
    fn name(&self) -> &str {
        "reference interpreter"
    }

    fn run(&self, loaded: &Loaded) -> Outcome {
        let mut outcome = Outcome::default();
        let Ok(project) = loaded else {
            return outcome;
        };
        let files = project
            .files
            .iter()
            .map(|f| moddil_refinterp::SourceSpec {
                path: f.display.clone(),
                text: f.text.clone(),
                mod_: f.mod_,
                module: f.module.clone(),
            })
            .collect();
        let mods = project
            .mods
            .iter()
            .map(|m| moddil_refinterp::ModSpec {
                deps: m.deps.clone(),
            })
            .collect();
        let Some(runs) = moddil_refinterp::run_program(files, mods) else {
            outcome
                .diagnostics
                .insert(((String::new(), 0), "(the interpreter panicked)".to_string()));
            return outcome;
        };
        for run in runs {
            outcome.observed.extend(
                run.observed
                    .into_iter()
                    .map(|(file, range, value)| (line_of(project, file, range.end), value)),
            );
            if let Some(trap) = run.trap {
                outcome
                    .traps
                    .insert(line_of(project, trap.file, trap.range.start));
            }
            if let Some((file, range)) = run.error {
                outcome.errors.insert(line_of(project, file, range.start));
            }
        }
        outcome
    }

    fn checks(&self) -> bool {
        false
    }
}

/// Reads the expectations in a test file's comments into `e`, under the file's name `file`.
///
/// # Errors
///
/// A message for a malformed expectation comment.
pub fn read_expectations(file: &str, text: &str, e: &mut Expectations) -> Result<(), String> {
    // The `// =>` lines of the examples in its doc comments, each in its own source, whose lines
    // are the file's (`test.doc.1`).
    let parse = moddil_syntax::parse(text);
    for ex in moddil_syntax::doc_examples(&parse.syntax(), text) {
        let mut values = Expectations::default();
        read_comments(file, &ex.source, &mut values)?;
        e.values.extend(values.values);
    }
    read_comments(file, text, e)
}

/// Reads the expectations in the comments of `text` into `e`, as [`read_expectations`].
fn read_comments(file: &str, text: &str, e: &mut Expectations) -> Result<(), String> {
    let mut offset = 0u32;
    let at_line = |line: u32| (file.to_string(), line);
    for tok in moddil_syntax::lex(text).0 {
        let at = offset;
        offset += tok.len;
        if tok.kind != SyntaxKind::LINE_COMMENT {
            continue;
        }
        let comment = &text[at as usize..offset as usize];
        let line = line_col(text, at).0;
        if let Some(rest) = comment.strip_prefix("//~") {
            marks(rest, &at_line(line), e)?;
        } else if let Some(x) = moddil_syntax::expected(comment) {
            let x = x.map_err(|why| format!("line {line}: {why}"))?;
            let at = at_line(line);
            if x.any_order {
                e.any_order.insert(at.clone());
            }
            if !x.also.is_empty() {
                let also = x.also.iter().map(|a| a.to_string()).collect();
                e.parallel.insert(at.clone(), also);
            }
            e.values.insert(at, x.value.to_string());
        } else if let Some(rest) = comment.strip_prefix("// covers:") {
            e.covers.extend(
                rest.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from),
            );
        } else if comment.trim_end() == "// disputed" || comment.starts_with("// disputed:") {
            e.disputed = true;
        } else if comment.trim_end() == "// native" {
            e.native = true;
        }
    }
    Ok(())
}

/// The expectations of a single test file.
///
/// # Errors
///
/// A message for a malformed expectation comment.
pub fn expectations(text: &str) -> Result<Expectations, String> {
    let mut e = Expectations::default();
    read_expectations("", text, &mut e)?;
    Ok(e)
}

/// Reads a manifest's `#~ E…` expectations into `e`.
fn manifest_expectations(file: &str, text: &str, e: &mut Expectations) -> Result<(), String> {
    for (i, l) in text.lines().enumerate() {
        if let Some(at) = l.find("#~") {
            marks(&l[at + 2..], &(file.to_string(), i as u32 + 1), e)?;
        }
    }
    Ok(())
}

/// The marks after `//~` (or `#~`) on `line`.
fn marks(rest: &str, line: &Line, e: &mut Expectations) -> Result<(), String> {
    let words: Vec<&str> = rest.split_whitespace().collect();
    if words.is_empty() {
        return Err(format!("line {}: `//~` without an expectation", line.1));
    }
    for w in words {
        if w == "trap" {
            e.traps.insert(line.clone());
        } else if w == "error" {
            e.errors.insert(line.clone());
        } else if is_code(w) {
            e.diagnostics.insert((line.clone(), w.to_string()));
        } else {
            return Err(format!(
                "line {}: `{w}` is not `trap`, `error` or a code like E0201 or W0201",
                line.1
            ));
        }
    }
    Ok(())
}

fn is_code(w: &str) -> bool {
    w.len() == 5
        && (w.starts_with('E') || w.starts_with('W'))
        && w[1..].chars().all(|c| c.is_ascii_digit())
}

/// Why an outcome does not meet its expectations; empty if it does.
pub fn compare(expected: &Expectations, outcome: &Outcome) -> Vec<String> {
    compare_in(expected, outcome, false)
}

/// As [`compare`], for an implementation that runs tasks in parallel if `parallel`: a value may
/// then also be one its line lists `in parallel`.
pub fn compare_in(expected: &Expectations, outcome: &Outcome, parallel: bool) -> Vec<String> {
    let mut problems = Vec::new();
    // A file that expects diagnostics is only checked, so what it says of running would never
    // be checked.
    let runs =
        !expected.values.is_empty() || !expected.traps.is_empty() || !expected.errors.is_empty();
    if !expected.diagnostics.is_empty() && runs {
        problems.push(
            "this file expects diagnostics, so it is only checked, never run: its `// =>`, \
             `//~ trap` and `//~ error` lines would never be checked; move them to a file of \
             their own"
                .to_string(),
        );
    }
    for (line, code) in outcome.diagnostics.difference(&expected.diagnostics) {
        let line = show(line);
        problems.push(format!("{line}: unexpected diagnostic {code}"));
    }
    for (line, code) in expected.diagnostics.difference(&outcome.diagnostics) {
        let line = show(line);
        problems.push(format!("{line}: expected diagnostic {code}, none reported"));
    }
    if !outcome.diagnostics.is_empty() || !expected.diagnostics.is_empty() {
        return problems;
    }
    for (line, value) in &expected.values {
        let seen: Vec<&String> = outcome
            .observed
            .iter()
            .filter(|(l, _)| l == line)
            .map(|(_, v)| v)
            .collect();
        let shown = show(line);
        if seen.is_empty() {
            problems.push(format!("{shown}: expected {value}, but the line never ran"));
        }
        let x = moddil_syntax::Expected {
            value,
            also: expected
                .parallel
                .get(line)
                .map(|a| a.iter().map(String::as_str).collect())
                .unwrap_or_default(),
            any_order: expected.any_order.contains(line),
        };
        for v in seen {
            if !x.accepts(v, parallel) {
                problems.push(format!("{shown}: expected {value}, found {v}"));
            }
        }
    }
    for line in outcome.traps.difference(&expected.traps) {
        problems.push(format!("{}: unexpected trap", show(line)));
    }
    for line in expected.traps.difference(&outcome.traps) {
        problems.push(format!("{}: expected a trap, none happened", show(line)));
    }
    for line in outcome.errors.difference(&expected.errors) {
        problems.push(format!("{}: an error unexpectedly left a test", show(line)));
    }
    for line in expected.errors.difference(&outcome.errors) {
        problems.push(format!(
            "{}: expected an error to leave a test, none did",
            show(line)
        ));
    }
    problems
}

/// A line, for messages: `line 3`, or `src/a.moddil line 3` in a test of several files.
fn show(line: &Line) -> String {
    if line.0.is_empty() {
        format!("line {}", line.1)
    } else {
        format!("{} line {}", line.0, line.1)
    }
}

/// The result of running the suite.
#[derive(Debug, Default)]
pub struct Report {
    /// Test files run.
    pub files: usize,
    /// Disputed files, not counted.
    pub disputed: Vec<String>,
    /// Failures: file, implementation and problem.
    pub failures: Vec<String>,
    /// Paragraph IDs in the reference that no test cites.
    pub uncovered: Vec<String>,
    /// Paragraph IDs tests cite that the reference does not have.
    pub unknown: Vec<String>,
    /// Paragraph IDs in the reference.
    pub paragraphs: usize,
    /// For each implementation that skips files it cannot run yet: how many it ran.
    pub ran: BTreeMap<String, usize>,
}

impl Report {
    /// Whether everything passed and every paragraph is covered.
    pub fn is_clean(&self) -> bool {
        self.failures.is_empty() && self.uncovered.is_empty() && self.unknown.is_empty()
    }
}

/// Every file with extension `ext` under `dir`, sorted.
fn files_with_extension(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            files_with_extension(&p, ext, out);
        } else if p.extension().is_some_and(|e| e == ext) {
            out.push(p);
        }
    }
}

/// The paragraph IDs in the reference for `edition`: every paragraph starting with a bold ID such
/// as `**expr.arith.overflow.1**`.
pub fn paragraph_ids(repo: &Path, edition: &str) -> BTreeSet<String> {
    let mut files = Vec::new();
    files_with_extension(&repo.join("reference").join(edition), "md", &mut files);
    let mut ids = BTreeSet::new();
    for f in files {
        let Ok(text) = fs::read_to_string(&f) else {
            continue;
        };
        for line in text.lines() {
            if let Some(id) = paragraph_id(line) {
                ids.insert(id.to_string());
            }
        }
    }
    ids
}

/// The ID a reference paragraph starts with, if the line starts one.
fn paragraph_id(line: &str) -> Option<&str> {
    let rest = line.trim_start_matches("- ").strip_prefix("**")?;
    let id = &rest[..rest.find("**")?];
    let valid = id.contains('.')
        && id.split('.').all(|p| {
            !p.is_empty()
                && p.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        })
        && id
            .rsplit('.')
            .next()
            .is_some_and(|n| n.chars().all(|c| c.is_ascii_digit()));
    valid.then_some(id)
}

/// The tests under `dir`, sorted: each directory with a `moddil.toml` (whose contents are its
/// own), and each other `.moddil` file.
fn tests_in(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() && p.join(moddil_project::MANIFEST).is_file() {
            out.push(p);
        } else if p.is_dir() {
            tests_in(&p, out);
        } else if p.extension().is_some_and(|e| e == "moddil") {
            out.push(p);
        }
    }
}

/// A test's expectations, from all its files and manifests.
fn test_expectations(path: &Path, loaded: &Loaded) -> Result<Expectations, String> {
    let mut e = Expectations::default();
    if path.is_file() {
        let text = fs::read_to_string(path)
            .map_err(|x| x.to_string())?
            .replace("\r\n", "\n");
        let display = match loaded {
            Ok(p) => p.files[0].display.clone(),
            Err(_) => String::new(),
        };
        read_expectations(&display, &text, &mut e)?;
        return Ok(e);
    }
    let mut files = Vec::new();
    files_with_extension(path, "moddil", &mut files);
    for f in files {
        let text = fs::read_to_string(&f)
            .map_err(|x| x.to_string())?
            .replace("\r\n", "\n");
        read_expectations(&shown_path(path, &f), &text, &mut e)
            .map_err(|m| format!("{}: {m}", shown_path(path, &f)))?;
    }
    let mut manifests = Vec::new();
    files_with_extension(path, "toml", &mut manifests);
    for f in manifests {
        let text = fs::read_to_string(&f)
            .map_err(|x| x.to_string())?
            .replace("\r\n", "\n");
        manifest_expectations(&shown_path(path, &f), &text, &mut e)?;
    }
    Ok(e)
}

/// `f` as a project rooted at `dir` shows it: relative, with `/`.
fn shown_path(dir: &Path, f: &Path) -> String {
    f.strip_prefix(dir)
        .unwrap_or(f)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Runs every test of `edition` under `repo/conformance/` on each implementation, and checks the
/// paragraph IDs the tests cite against `repo/reference/`.
pub fn run_suite(repo: &Path, edition: &str, implementations: &[&dyn Implementation]) -> Report {
    run_tests(repo, edition, None, implementations)
}

/// Runs only the tests of one area, `repo/conformance/<edition>/<area>/`, so that areas can run
/// in parallel. Its coverage lists describe only that area's tests; [`coverage`] checks the
/// whole suite's.
pub fn run_area(
    repo: &Path,
    edition: &str,
    area: &str,
    implementations: &[&dyn Implementation],
) -> Report {
    run_tests(repo, edition, Some(area), implementations)
}

/// Reads every test's expectations without running anything, and checks the paragraph IDs they
/// cite against `repo/reference/`: the suite's coverage, cheaply.
pub fn coverage(repo: &Path, edition: &str) -> Report {
    run_tests(repo, edition, None, &[])
}

/// The suite's areas: the directories directly under `repo/conformance/<edition>/`, sorted. A
/// directory with a manifest is a test, not an area, and is not listed.
pub fn areas(repo: &Path, edition: &str) -> Vec<String> {
    let Ok(entries) = fs::read_dir(repo.join("conformance").join(edition)) else {
        return Vec::new();
    };
    let mut areas: Vec<String> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir() && !p.join(moddil_project::MANIFEST).is_file())
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    areas.sort();
    areas
}

/// What running one test found, to merge into the [`Report`].
#[derive(Default)]
struct TestResult {
    failures: Vec<String>,
    covers: Vec<String>,
    /// The test's path, if it is disputed (and so not run).
    disputed: Option<String>,
    /// Whether it counts as a file run.
    counted: bool,
    /// The implementations that ran it.
    ran: Vec<String>,
}

/// Reads test `t`'s expectations and runs it on each implementation, each on a thread of its own.
fn run_test(repo: &Path, t: &Path, implementations: &[&dyn Implementation]) -> TestResult {
    let mut r = TestResult::default();
    let rel = t
        .strip_prefix(repo)
        .unwrap_or(t)
        .to_string_lossy()
        .replace('\\', "/");
    let loaded = if t.is_file() {
        // A single file is named by its path from the repository, as the suite shows it.
        moddil_project::load(t).map(|mut p| {
            p.files[0].display = rel.clone();
            p
        })
    } else {
        moddil_project::load(t)
    };
    let expected = match test_expectations(t, &loaded) {
        Ok(e) => e,
        Err(e) => {
            r.failures.push(format!("{rel}: {e}"));
            return r;
        }
    };
    if expected.covers.is_empty() {
        r.failures.push(format!("{rel}: no `// covers:` line"));
    }
    r.covers.clone_from(&expected.covers);
    if expected.disputed {
        r.disputed = Some(rel);
        return r;
    }
    r.counted = true;
    let outcomes: Vec<Option<Outcome>> = std::thread::scope(|s| {
        let running: Vec<_> = implementations
            .iter()
            .map(|imp| {
                let skip = (!imp.checks() && !expected.diagnostics.is_empty())
                    || (expected.native && matches!(imp.name(), "wasm" | "reference interpreter"));
                (!skip).then(|| s.spawn(|| imp.run_if(&loaded)))
            })
            .collect();
        running
            .into_iter()
            .map(|h| h.and_then(|h| h.join().expect("an implementation's thread")))
            .collect()
    });
    for (imp, outcome) in implementations.iter().zip(outcomes) {
        let Some(outcome) = outcome else {
            continue;
        };
        r.ran.push(imp.name().to_string());
        for p in compare_in(&expected, &outcome, imp.parallel()) {
            r.failures.push(format!("{rel} ({}): {p}", imp.name()));
        }
    }
    r
}

fn run_tests(
    repo: &Path,
    edition: &str,
    area: Option<&str>,
    implementations: &[&dyn Implementation],
) -> Report {
    let mut root = repo.join("conformance").join(edition);
    if let Some(area) = area {
        root.push(area);
    }
    let mut tests = Vec::new();
    tests_in(&root, &mut tests);
    let ids = paragraph_ids(repo, edition);
    let mut report = Report {
        paragraphs: ids.len(),
        ..Report::default()
    };
    // Every test, and every implementation's run of it, on a thread of its own: they share
    // nothing, and an area then takes as long as its slowest run, not the sum of them all. The
    // results are merged in the tests' order, so the report is the same as run one by one.
    let results: Vec<TestResult> = std::thread::scope(|s| {
        let running: Vec<_> = tests
            .iter()
            .map(|t| s.spawn(|| run_test(repo, t, implementations)))
            .collect();
        running
            .into_iter()
            .map(|h| h.join().expect("a conformance test's thread"))
            .collect()
    });
    let mut cited = BTreeSet::new();
    for r in results {
        report.failures.extend(r.failures);
        cited.extend(r.covers);
        match r.disputed {
            Some(rel) => report.disputed.push(rel),
            None if r.counted => report.files += 1,
            None => {}
        }
        for name in r.ran {
            *report.ran.entry(name).or_default() += 1;
        }
    }
    report.uncovered = ids.difference(&cited).cloned().collect();
    report.unknown = cited.difference(&ids).cloned().collect();
    report
}

/// This crate's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_has_three_parts() {
        assert_eq!(VERSION.split('.').count(), 3);
    }

    #[test]
    fn dependencies_are_wired() {
        assert_eq!(moddil_refinterp::VERSION, VERSION);
    }

    #[test]
    fn reads_expectations() {
        let text = "// covers: a.b.1, c.2\n// disputed\ntest \"t\" {\n    1 + 2 // => 3\n}\nfn f() { x } //~ E0101 E0201\n//~ trap\n";
        let e = expectations(text).expect("valid");
        assert_eq!(e.covers, vec!["a.b.1", "c.2"]);
        assert!(e.disputed);
        assert_eq!(
            e.values.get(&(String::new(), 4)).map(String::as_str),
            Some("3")
        );
        assert_eq!(e.diagnostics.len(), 2);
        assert!(e.traps.contains(&(String::new(), 7)));
        assert!(expectations("//~ E99\n").is_err());
        assert!(expectations("// =>\n").is_err());
    }

    #[test]
    fn a_checked_file_expects_nothing_of_running() {
        let e = expectations("test \"t\" {\n    1 // => 1\n}\nfn f() { x } //~ E0101\n")
            .expect("valid");
        assert_eq!(compare(&e, &Outcome::default()).len(), 2);
        let e = expectations("fn f() { x } //~ E0101\n").expect("valid");
        assert_eq!(compare(&e, &Outcome::default()).len(), 1);
    }

    #[test]
    fn paragraph_ids_are_bold_at_the_start() {
        assert_eq!(
            paragraph_id("**expr.arith.overflow.1** Text"),
            Some("expr.arith.overflow.1")
        );
        assert_eq!(paragraph_id("- **fn.decl.2** Text"), Some("fn.decl.2"));
        assert_eq!(paragraph_id("**Note** text"), None);
        assert_eq!(paragraph_id("**expr.arith** text"), None);
    }

    fn outcome(text: &str) -> Vec<String> {
        let project = Project {
            files: vec![moddil_project::File {
                path: PathBuf::new(),
                display: String::new(),
                text: text.to_string(),
                mod_: 0,
                module: Vec::new(),
                exe: false,
            }],
            mods: vec![moddil_project::Mod {
                name: String::new(),
                deps: Vec::new(),
                manifest: None,
                stub: false,
                unsafe_ok: false,
                os_ok: false,
                c: None,
                version: None,
                runtime: None,
                threads: None,
            }],
        };
        compare(
            &expectations(text).expect("valid"),
            &MirInterpreter.run(&Ok(project)),
        )
    }

    #[test]
    fn values_traps_and_diagnostics() {
        assert!(outcome("test \"t\" {\n    1 + 2 // => 3\n}").is_empty());
        assert_eq!(
            outcome("test \"t\" {\n    1 + 2 // => 4\n}"),
            vec!["line 2: expected 4, found 3"]
        );
        assert_eq!(
            outcome("test \"t\" {\n    return;\n    1 // => 1\n}"),
            vec!["line 3: expected 1, but the line never ran"]
        );
        assert!(
            outcome("test \"t\" {\n    { let max = 9223372036854775807; max + 1 }; //~ trap\n}")
                .is_empty()
        );
        assert_eq!(
            outcome("test \"t\" {\n    { let max = 9223372036854775807; max + 1 };\n}"),
            vec!["line 2: unexpected trap"]
        );
        assert!(outcome("fn f() -> int { true } //~ E0201\n").is_empty());
        assert_eq!(
            outcome("fn f() -> int { 1 } //~ E0201\n"),
            vec!["line 1: expected diagnostic E0201, none reported"]
        );
    }
}
