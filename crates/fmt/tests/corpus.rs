//! The formatter over every Moddil source in the repository: each case of the grammar's corpus
//! (which together use every production) and its samples, and every `.moddil` file of the
//! prelude, the mods, the tour, the examples and the conformance suite that parses. Each one
//! formats (which checks that its tokens and comments are kept and that the result parses),
//! and formatting it again changes nothing.

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The cases of a corpus file: each whole file after a `// ---` line.
fn cases(file: &str) -> Vec<String> {
    let path = root().join("reference/grammar/tests").join(file);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .replace("\r\n", "\n");
    let mut cases: Vec<Vec<&str>> = Vec::new();
    for line in text.lines() {
        if line == "// ---" {
            cases.push(Vec::new());
        } else if let Some(case) = cases.last_mut() {
            case.push(line);
        }
    }
    cases.into_iter().map(|lines| lines.join("\n")).collect()
}

/// Every `.moddil` file under `dir`.
fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n != "target") {
                files(&path, out);
            }
        } else if path.extension().is_some_and(|x| x == "moddil") {
            out.push(path);
        }
    }
}

/// What is wrong with formatting `text`, if anything: it fails, or a second pass changes it.
fn check(text: &str) -> Option<String> {
    let once = match moddil_fmt::format(text) {
        Ok(s) => s,
        Err(moddil_fmt::Error::Syntax(_)) => return None,
        Err(e) => return Some(format!("{e}")),
    };
    match moddil_fmt::format(&once) {
        Ok(twice) if twice == once => None,
        Ok(twice) => Some(format!("not idempotent:\n{once}\n=== becomes ===\n{twice}")),
        Err(e) => Some(format!("the second pass fails: {e}\n{once}")),
    }
}

fn report(failures: &[String], total: usize) -> String {
    let shown: Vec<&str> = failures.iter().take(10).map(String::as_str).collect();
    format!(
        "{} of {total} fail:\n\n{}",
        failures.len(),
        shown.join("\n---\n")
    )
}

#[test]
fn every_case_of_the_grammar_formats() {
    let cases: Vec<String> = cases("valid.moddil")
        .into_iter()
        .chain(cases("samples.moddil"))
        .collect();
    let mut failures = Vec::new();
    for (i, case) in cases.iter().enumerate() {
        if let Some(why) = check(case) {
            failures.push(format!("case {i}:\n{case}\n: {why}"));
        }
    }
    assert!(failures.is_empty(), "{}", report(&failures, cases.len()));
}

#[test]
fn every_file_in_the_repository_formats() {
    let mut paths = Vec::new();
    for dir in [
        "reference/2027/prelude",
        "mods",
        "tour",
        "examples",
        "conformance",
    ] {
        files(&root().join(dir), &mut paths);
    }
    assert!(paths.len() > 300, "found only {} files", paths.len());
    let mut failures = Vec::new();
    for path in &paths {
        let text =
            std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        if let Some(why) = check(&text) {
            failures.push(format!("{}: {why}", path.display()));
        }
    }
    assert!(failures.is_empty(), "{}", report(&failures, paths.len()));
}
