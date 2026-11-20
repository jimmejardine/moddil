//! `moddil-grammar`: checks Moddil's grammar (or another file in its dialect) and reports every
//! decision that is not LL(k).
//!
//! ```text
//! moddil-grammar [PATH] [-k N]      default: reference/grammar/moddil.ebnf, k = 3
//! ```

use std::process::ExitCode;

const DEFAULT_PATH: &str = "reference/grammar/moddil.ebnf";

fn main() -> ExitCode {
    let mut path = DEFAULT_PATH.to_string();
    let mut k = moddil_grammar::DEFAULT_K;
    let mut args = std::env::args().skip(1);
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
