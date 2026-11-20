//! The conformance runner: runs `conformance/` tests on every implementation and reports any
//! disagreement with their expectations.
//!
//! A test file states its expectations in comments:
//!
//! | Comment | Expects |
//! |---|---|
//! | `// covers: expr.arith.overflow.1, …` | The reference paragraphs the file tests |
//! | `// => 3` | The expression statement or final expression it follows evaluates to `3` (in a `test` item) |
//! | `//~ E0201` | A diagnostic with this code starting on this line (several codes may follow) |
//! | `//~ trap` | A trap on this line |
//! | `// disputed` | The test does not count until the reference is clarified |
//!
//! A file with expected diagnostics is only checked; otherwise every `test` item in it runs, every
//! `// =>` line must run and produce exactly its value, and traps happen exactly on `//~ trap`
//! lines. See this crate's `README.md`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use moddil_sema::{Database, SourceFile};
use moddil_syntax::{SyntaxKind, line_col};

/// The edition the suite currently tests.
pub const EDITION: &str = "2027";

/// What a test file expects, by line (1-based).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Expectations {
    /// Reference paragraph IDs the file covers.
    pub covers: Vec<String>,
    /// `// =>` values.
    pub values: BTreeMap<u32, String>,
    /// `//~ E…` codes.
    pub diagnostics: BTreeSet<(u32, String)>,
    /// `//~ trap` lines.
    pub traps: BTreeSet<u32>,
    /// Marked `// disputed`.
    pub disputed: bool,
}

/// What an implementation did with a test file, by line (1-based).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// The diagnostics: line and code.
    pub diagnostics: BTreeSet<(u32, String)>,
    /// Observed values: the line the expression ends on, and the value as Moddil source.
    pub observed: Vec<(u32, String)>,
    /// The lines traps happened on (where the failing check starts), one per trapping test.
    pub traps: BTreeSet<u32>,
}

/// An implementation of Moddil the suite can run.
pub trait Implementation {
    /// Its name, for reports.
    fn name(&self) -> &str;
    /// Checks the file and, if it has no errors, runs all its tests.
    fn run(&self, path: &str, text: &str) -> Outcome;
}

/// The compiler's front end with the MIR interpreter.
pub struct MirInterpreter;

impl Implementation for MirInterpreter {
    fn name(&self) -> &str {
        "MIR interpreter"
    }

    fn run(&self, path: &str, text: &str) -> Outcome {
        let line = |offset: u32| line_col(text, offset).0;
        let db = Database::default();
        let file = SourceFile::new(&db, path.to_string(), text.to_string());
        let mut outcome = Outcome {
            diagnostics: moddil_sema::diagnostics(&db, file)
                .iter()
                .map(|d| (line(d.range.start), d.code.as_str().to_string()))
                .collect(),
            ..Outcome::default()
        };
        if moddil_sema::has_errors(&db, file) {
            return outcome;
        }
        let tests = moddil_sema::item_tree(&db, file).tests.len() as u32;
        for i in 0..tests {
            let mut observed = Vec::new();
            let result = moddil_interp::run_test(&db, file, i, &mut |range, value| {
                observed.push((line(range.end), value.to_string()));
            });
            outcome.observed.extend(observed);
            if let Err(trap) = result {
                outcome.traps.insert(line(trap.range.start));
            }
        }
        outcome
    }
}

/// Reads the expectations in a test file's comments.
///
/// # Errors
///
/// A message for a malformed expectation comment.
pub fn expectations(text: &str) -> Result<Expectations, String> {
    let mut e = Expectations::default();
    let mut offset = 0u32;
    for tok in moddil_syntax::lex(text).0 {
        let at = offset;
        offset += tok.len;
        if tok.kind != SyntaxKind::LINE_COMMENT {
            continue;
        }
        let comment = &text[at as usize..offset as usize];
        let line = line_col(text, at).0;
        if let Some(rest) = comment.strip_prefix("//~") {
            let words: Vec<&str> = rest.split_whitespace().collect();
            if words.is_empty() {
                return Err(format!("line {line}: `//~` without an expectation"));
            }
            for w in words {
                if w == "trap" {
                    e.traps.insert(line);
                } else if is_code(w) {
                    e.diagnostics.insert((line, w.to_string()));
                } else {
                    return Err(format!(
                        "line {line}: `{w}` is not `trap` or a code like E0201"
                    ));
                }
            }
        } else if let Some(rest) = comment.strip_prefix("// =>") {
            let value = rest.trim();
            if value.is_empty() {
                return Err(format!("line {line}: `// =>` without a value"));
            }
            e.values.insert(line, value.to_string());
        } else if let Some(rest) = comment.strip_prefix("// covers:") {
            e.covers.extend(
                rest.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from),
            );
        } else if comment.trim_end() == "// disputed" || comment.starts_with("// disputed:") {
            e.disputed = true;
        }
    }
    Ok(e)
}

fn is_code(w: &str) -> bool {
    w.len() == 5 && w.starts_with('E') && w[1..].chars().all(|c| c.is_ascii_digit())
}

/// Why an outcome does not meet its expectations; empty if it does.
pub fn compare(expected: &Expectations, outcome: &Outcome) -> Vec<String> {
    let mut problems = Vec::new();
    for (line, code) in outcome.diagnostics.difference(&expected.diagnostics) {
        problems.push(format!("line {line}: unexpected diagnostic {code}"));
    }
    for (line, code) in expected.diagnostics.difference(&outcome.diagnostics) {
        problems.push(format!(
            "line {line}: expected diagnostic {code}, none reported"
        ));
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
        if seen.is_empty() {
            problems.push(format!(
                "line {line}: expected {value}, but the line never ran"
            ));
        }
        for v in seen {
            if v != value {
                problems.push(format!("line {line}: expected {value}, found {v}"));
            }
        }
    }
    for line in outcome.traps.difference(&expected.traps) {
        problems.push(format!("line {line}: unexpected trap"));
    }
    for line in expected.traps.difference(&outcome.traps) {
        problems.push(format!("line {line}: expected a trap, none happened"));
    }
    problems
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
}

impl Report {
    /// Whether everything passed and every paragraph is covered.
    pub fn is_clean(&self) -> bool {
        self.failures.is_empty() && self.uncovered.is_empty() && self.unknown.is_empty()
    }
}

/// Every `.mdl` file under `dir`, sorted.
fn mdl_files(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            mdl_files(&p, ext, out);
        } else if p.extension().is_some_and(|e| e == ext) {
            out.push(p);
        }
    }
}

/// The paragraph IDs in the reference for `edition`: every paragraph starting with a bold ID such
/// as `**expr.arith.overflow.1**`.
pub fn paragraph_ids(repo: &Path, edition: &str) -> BTreeSet<String> {
    let mut files = Vec::new();
    mdl_files(&repo.join("reference").join(edition), "md", &mut files);
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

/// Runs every test of `edition` under `repo/conformance/` on each implementation, and checks the
/// paragraph IDs the tests cite against `repo/reference/`.
pub fn run_suite(repo: &Path, edition: &str, implementations: &[&dyn Implementation]) -> Report {
    let mut files = Vec::new();
    mdl_files(&repo.join("conformance").join(edition), "mdl", &mut files);
    let ids = paragraph_ids(repo, edition);
    let mut report = Report {
        paragraphs: ids.len(),
        ..Report::default()
    };
    let mut cited = BTreeSet::new();
    for f in files {
        let rel = f
            .strip_prefix(repo)
            .unwrap_or(&f)
            .to_string_lossy()
            .replace('\\', "/");
        let text = match fs::read_to_string(&f) {
            Ok(t) => t.replace("\r\n", "\n"),
            Err(e) => {
                report.failures.push(format!("{rel}: {e}"));
                continue;
            }
        };
        let expected = match expectations(&text) {
            Ok(e) => e,
            Err(e) => {
                report.failures.push(format!("{rel}: {e}"));
                continue;
            }
        };
        if expected.covers.is_empty() {
            report.failures.push(format!("{rel}: no `// covers:` line"));
        }
        cited.extend(expected.covers.iter().cloned());
        if expected.disputed {
            report.disputed.push(rel);
            continue;
        }
        report.files += 1;
        for imp in implementations {
            let outcome = imp.run(&rel, &text);
            for p in compare(&expected, &outcome) {
                report.failures.push(format!("{rel} ({}): {p}", imp.name()));
            }
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
        assert_eq!(e.values.get(&4).map(String::as_str), Some("3"));
        assert_eq!(e.diagnostics.len(), 2);
        assert!(e.traps.contains(&7));
        assert!(expectations("//~ E99\n").is_err());
        assert!(expectations("// =>\n").is_err());
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
        compare(
            &expectations(text).expect("valid"),
            &MirInterpreter.run("t.mdl", text),
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
        assert!(outcome("test \"t\" {\n    9223372036854775807 + 1; //~ trap\n}").is_empty());
        assert_eq!(
            outcome("test \"t\" {\n    9223372036854775807 + 1;\n}"),
            vec!["line 2: unexpected trap"]
        );
        assert!(outcome("fn f() -> int { true } //~ E0201\n").is_empty());
        assert_eq!(
            outcome("fn f() -> int { 1 } //~ E0201\n"),
            vec!["line 1: expected diagnostic E0201, none reported"]
        );
    }
}
