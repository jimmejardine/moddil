//! `moddil-grammar`: checks Moddil's grammar (or another file in its dialect) and reports every
//! decision that is not LL(k).
//!
//! ```text
//! moddil-grammar [PATH] [-k N]          check; default reference/grammar/moddil.ebnf, k = 3
//! moddil-grammar generate [--check]     write the generated files (or check they are current)
//! ```
//!
//! Run it from the repository root.

use std::process::ExitCode;

const DEFAULT_PATH: &str = "reference/grammar/moddil.ebnf";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("generate") {
        return generate(args.iter().any(|a| a == "--check"));
    }
    let mut path = DEFAULT_PATH.to_string();
    let mut k = moddil_grammar::DEFAULT_K;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-k" | "--k" => match args.next().and_then(|v| v.parse::<usize>().ok()) {
                Some(n) if n > 0 => k = n,
                _ => {
                    eprintln!("-k needs a positive number");
                    return ExitCode::from(2);
                }
            },
            "-h" | "--help" => {
                println!("usage: moddil-grammar [PATH] [-k N]   (default {DEFAULT_PATH}, k = {k})");
                return ExitCode::SUCCESS;
            }
            _ => path = arg,
        }
    }
    let source = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            return ExitCode::from(2);
        }
    };
    let report = match moddil_grammar::check(&source, k) {
        Ok(r) => r,
        Err(errors) => {
            for e in &errors {
                eprintln!("{path}:{e}");
            }
            return ExitCode::FAILURE;
        }
    };
    println!(
        "{path}: {} syntax rules, {} token rules; {} copies after flags; {} decisions checked at k = {}",
        report.syntax_rules, report.token_rules, report.instances, report.decisions, report.k
    );
    for name in &report.unreachable {
        println!("unreachable from `file`: {name}");
    }
    for c in &report.conflicts {
        print!("not LL({}): {c}", report.k);
    }
    if report.is_ll() {
        println!("LL({}): yes", report.k);
        ExitCode::SUCCESS
    } else {
        println!(
            "LL({}): no ({} conflicts, {} unreachable rules)",
            report.k,
            report.conflicts.len(),
            report.unreachable.len()
        );
        ExitCode::FAILURE
    }
}

/// Writes every generated file, or (with `--check`) fails if any is out of date.
fn generate(check: bool) -> ExitCode {
    let source = match std::fs::read_to_string(DEFAULT_PATH) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cannot read {DEFAULT_PATH} (run from the repository root): {e}");
            return ExitCode::from(2);
        }
    };
    let generated = match moddil_grammar::generate(&source) {
        Ok(g) => g,
        Err(errors) => {
            for e in &errors {
                eprintln!("{DEFAULT_PATH}:{e}");
            }
            return ExitCode::FAILURE;
        }
    };
    let (covered, total) = generated.coverage;
    println!("corpus: {covered} of {total} productions covered");
    for name in &generated.uncovered {
        println!("  not reached by a shortest-context sentence: {name}");
    }
    let mut stale = Vec::new();
    for file in &generated.files {
        let current = std::fs::read_to_string(&file.path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        if current == file.contents {
            continue;
        }
        if check {
            stale.push(file.path.clone());
            continue;
        }
        if let Some(dir) = std::path::Path::new(&file.path).parent()
            && let Err(e) = std::fs::create_dir_all(dir)
        {
            eprintln!("cannot create {}: {e}", dir.display());
            return ExitCode::FAILURE;
        }
        if let Err(e) = std::fs::write(&file.path, &file.contents) {
            eprintln!("cannot write {}: {e}", file.path);
            return ExitCode::FAILURE;
        }
        println!("wrote {}", file.path);
    }
    if stale.is_empty() {
        ExitCode::SUCCESS
    } else {
        for path in &stale {
            eprintln!("out of date: {path} (run `cargo run -p moddil-grammar -- generate`)");
        }
        ExitCode::FAILURE
    }
}
