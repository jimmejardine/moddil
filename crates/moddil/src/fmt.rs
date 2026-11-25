//! `moddil fmt`: Moddil source rewritten in canonical format ([`moddil_fmt`]), or, with
//! `--check`, the files that are not.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Formats the `.moddil` files `paths` name (files, or directories walked for them; `.` when
/// none is named). With `check`, rewrites nothing: lists the files that would change, and
/// fails if there are any.
pub(crate) fn run(paths: &[PathBuf], check: bool) -> ExitCode {
    let here = [PathBuf::from(".")];
    let paths = if paths.is_empty() { &here[..] } else { paths };
    let mut files = Vec::new();
    let mut ok = true;
    for path in paths {
        if path.is_dir() {
            walk(path, &mut files);
        } else if path.is_file() {
            files.push((path.clone(), true));
        } else {
            eprintln!("moddil: {}: no such file or directory", path.display());
            ok = false;
        }
    }
    let mut unformatted = 0;
    for (path, named) in &files {
        match file(path, *named, check) {
            Outcome::Same | Outcome::Skipped => {}
            Outcome::Changed => unformatted += 1,
            Outcome::Failed => ok = false,
        }
    }
    if check && unformatted > 0 {
        let s = if unformatted == 1 { "" } else { "s" };
        eprintln!("moddil: {unformatted} file{s} not formatted; `moddil fmt` formats them");
        ok = false;
    }
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Every `.moddil` file under `dir`, in order, but in `target/` and hidden directories.
fn walk(dir: &Path, out: &mut Vec<(PathBuf, bool)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for path in entries {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if path.is_dir() {
            if name != "target" && !name.starts_with('.') {
                walk(&path, out);
            }
        } else if path.extension().is_some_and(|x| x == "moddil") {
            out.push((path, false));
        }
    }
}

enum Outcome {
    /// Already in canonical format.
    Same,
    /// Rewritten, or (with `--check`) would be.
    Changed,
    /// Found by a directory walk, and not parsed: left alone.
    Skipped,
    /// Unreadable, unwritable, or (named on the command line) not parsed.
    Failed,
}

/// Formats one file. One named on the command line must parse; one a directory walk found is
/// skipped if it does not (the conformance suite's syntax-error tests, say).
fn file(path: &Path, named: bool, check: bool) -> Outcome {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("moddil: reading {}: {e}", path.display());
            return Outcome::Failed;
        }
    };
    let formatted = match moddil_fmt::format(&text) {
        Ok(f) => f,
        Err(moddil_fmt::Error::Syntax(errors)) if named => {
            let lf = text.replace("\r\n", "\n");
            for e in &errors {
                let (line, col) = moddil_syntax::line_col(&lf, e.range.start);
                eprintln!("{}:{line}:{col}: {}", path.display(), e.message);
            }
            eprintln!(
                "moddil: {} is not formatted: it does not parse",
                path.display()
            );
            return Outcome::Failed;
        }
        Err(moddil_fmt::Error::Syntax(_)) => {
            eprintln!("moddil: skipped {}: it does not parse", path.display());
            return Outcome::Skipped;
        }
        Err(e) => {
            eprintln!("moddil: {}: {e}", path.display());
            return Outcome::Failed;
        }
    };
    // Line endings are the checkout's business: a file is unformatted only if its text is.
    let crlf = text.contains("\r\n");
    if formatted == text.replace("\r\n", "\n") {
        return Outcome::Same;
    }
    if check {
        println!("{}", path.display());
        return Outcome::Changed;
    }
    let out = if crlf {
        formatted.replace('\n', "\r\n")
    } else {
        formatted
    };
    if let Err(e) = std::fs::write(path, out) {
        eprintln!("moddil: writing {}: {e}", path.display());
        return Outcome::Failed;
    }
    eprintln!("moddil: formatted {}", path.display());
    Outcome::Changed
}
