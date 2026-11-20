//! The `moddil` command. At M0 it checks files and runs them and their tests on the MIR
//! interpreter. See `crates/moddil/README.md`.

use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;

use ariadne::{Config, IndexType, Label, Report, ReportKind, Source};
use clap::{Parser, Subcommand};
use moddil_interp::{Trap, Value};
use moddil_sema::{Database, Diagnostic, ItemTree, Severity, SourceFile};

/// Moddil: check and run programs.
#[derive(Parser)]
#[command(name = "moddil", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Reports the errors in a file.
    Check {
        /// The file.
        file: PathBuf,
    },
    /// Runs a file's `fn main()` and prints its result.
    Run {
        /// The file.
        file: PathBuf,
    },
    /// Runs a file's tests.
    Test {
        /// The file.
        file: PathBuf,
    },
}

/// The exit code for a trap, as for a Rust panic.
const TRAPPED: u8 = 101;

fn main() -> ExitCode {
    let cli = Cli::parse();
    let path = match &cli.command {
        Command::Check { file } | Command::Run { file } | Command::Test { file } => file,
    };
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("moddil: cannot read {}: {e}", path.display());
            return ExitCode::from(2);
        }
    };
    let name = path.display().to_string();
    let db = Database::default();
    let file = SourceFile::new(&db, name.clone(), text.clone());
    let diagnostics = moddil_sema::diagnostics(&db, file);
    for d in diagnostics {
        report(&name, &text, d);
    }
    if moddil_sema::has_errors(&db, file) {
        let n = diagnostics.len();
        eprintln!("moddil: {n} error{}", if n == 1 { "" } else { "s" });
        return ExitCode::FAILURE;
    }
    match cli.command {
        Command::Check { .. } => ExitCode::SUCCESS,
        Command::Run { .. } => run(&db, file, &name, &text),
        Command::Test { .. } => test(&db, file, &name, &text),
    }
}

fn run(db: &Database, file: SourceFile, name: &str, text: &str) -> ExitCode {
    let items = moddil_sema::item_tree(db, file);
    let Some(main) = items.fn_named("main") else {
        eprintln!("moddil: {name} has no `fn main()`");
        return ExitCode::FAILURE;
    };
    if !items.fns[main as usize].params.is_empty() {
        eprintln!("moddil: `main` must not take parameters");
        return ExitCode::FAILURE;
    }
    match moddil_interp::call_fn(db, file, main, &[], &mut |_, _| {}) {
        Ok(Value::Unit) => ExitCode::SUCCESS,
        Ok(v) => {
            println!("{}", v.render(items));
            ExitCode::SUCCESS
        }
        Err(trap) => {
            report_trap(name, text, items, &trap);
            ExitCode::from(TRAPPED)
        }
    }
}

fn test(db: &Database, file: SourceFile, name: &str, text: &str) -> ExitCode {
    let items = moddil_sema::item_tree(db, file);
    let mut failed = 0;
    for (i, t) in items.tests.iter().enumerate() {
        match moddil_interp::run_test(db, file, i as u32, &mut |_, _| {}) {
            Ok(()) => println!("test \"{}\" ... ok", t.name),
            Err(trap) => {
                failed += 1;
                println!("test \"{}\" ... trapped", t.name);
                report_trap(name, text, items, &trap);
            }
        }
    }
    let passed = items.tests.len() - failed;
    println!("{passed} passed, {failed} failed");
    if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn config() -> Config {
    Config::default()
        .with_index_type(IndexType::Byte)
        .with_color(std::io::stderr().is_terminal())
}

fn report(name: &str, text: &str, d: &Diagnostic) {
    let kind = match d.severity {
        Severity::Error => ReportKind::Error,
        Severity::Warning => ReportKind::Warning,
    };
    let span = (name, d.range.to_range());
    let mut r = Report::build(kind, span.clone())
        .with_config(config())
        .with_code(d.code)
        .with_message(&d.message)
        .with_label(Label::new(span).with_message(d.code.description()));
    for (range, message) in &d.labels {
        r = r.with_label(Label::new((name, range.to_range())).with_message(message));
    }
    // Writing to stderr can only fail if stderr is gone, and then there is nobody to tell.
    let _ = r.finish().eprint((name, Source::from(text)));
}

fn report_trap(name: &str, text: &str, items: &ItemTree, trap: &Trap) {
    let span = (name, trap.range.to_range());
    let mut report = Report::build(ReportKind::Error, span.clone())
        .with_config(config())
        .with_message(format!("trap: {}", trap.class))
        .with_label(Label::new(span).with_message("here"));
    // A failed `assert` shows the value of each part of its condition.
    if !trap.parts.is_empty() {
        let width = trap
            .parts
            .iter()
            .map(|(r, _)| text[r.to_range()].len())
            .max()
            .unwrap_or(0);
        let lines: Vec<String> = trap
            .parts
            .iter()
            .map(|(r, v)| {
                let shown = v
                    .as_ref()
                    .map_or_else(|| "not evaluated".to_string(), |v| v.render(items));
                format!("{:width$} = {shown}", &text[r.to_range()])
            })
            .collect();
        report = report.with_note(lines.join(
            "
",
        ));
    }
    let _ = report.finish().eprint((name, Source::from(text)));
}
