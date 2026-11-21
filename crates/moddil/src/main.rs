//! The `moddil` command. At M0 it checks files and runs them and their tests on the MIR
//! interpreter. See `crates/moddil/README.md`.

use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;

use ariadne::{Config, IndexType, Label, Report, ReportKind, Source};
use clap::{Parser, Subcommand};
use moddil_interp::{Stop, Trap, Value};
use moddil_sema::{
    Database, Diagnostic, ItemTree, ModInfo, Program, ProgramFile, Severity, SourceFile,
};
use moddil_syntax::TextRange;

/// Moddil: check and run programs.
#[derive(Parser)]
#[command(name = "moddil", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Reports the errors in a program.
    Check {
        /// A file, or a mod's directory (with a `moddil.toml`).
        file: PathBuf,
    },
    /// Runs a program's `fn main()` and prints its result.
    Run {
        /// A file, or a mod's directory (with a `moddil.toml`).
        file: PathBuf,
    },
    /// Runs a program's tests (a mod's, not its dependencies').
    Test {
        /// A file, or a mod's directory (with a `moddil.toml`).
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
    let project = match moddil_project::load(path) {
        Ok(p) => p,
        Err(problems) => {
            for p in &problems {
                report_problem(p);
            }
            let n = problems.len();
            eprintln!("moddil: {n} error{}", if n == 1 { "" } else { "s" });
            return ExitCode::FAILURE;
        }
    };
    let files: Vec<(String, String)> = project
        .files
        .iter()
        .map(|f| (f.display.clone(), f.text.clone()))
        .collect();
    let db = Database::default();
    let program = program(&db, &project);
    let diagnostics = moddil_interp::diagnostics(&db, program);
    for d in &diagnostics {
        let (name, text) = &files[d.file as usize];
        report(name, text, d);
    }
    if moddil_interp::has_errors(&db, program) {
        let n = diagnostics.len();
        eprintln!("moddil: {n} error{}", if n == 1 { "" } else { "s" });
        return ExitCode::FAILURE;
    }
    match cli.command {
        Command::Check { .. } => ExitCode::SUCCESS,
        Command::Run { .. } => run(&db, program, &files, &path.display().to_string()),
        Command::Test { .. } => test(&db, program, &files),
    }
}

/// The program `project` describes, as the compiler's input.
fn program(db: &Database, project: &moddil_project::Project) -> Program {
    let files = project
        .files
        .iter()
        .map(|f| ProgramFile {
            source: SourceFile::new(db, f.display.clone(), f.text.clone()),
            mod_: f.mod_,
            module: f.module.clone(),
            exe: f.exe,
        })
        .collect();
    let mods = project
        .mods
        .iter()
        .map(|m| ModInfo {
            name: m.name.clone(),
            deps: m.deps.clone(),
        })
        .collect();
    Program::new(db, files, mods)
}

fn run(db: &Database, program: Program, files: &[(String, String)], name: &str) -> ExitCode {
    let items = moddil_sema::item_tree(db, program);
    let Some(main) = items.main_fn() else {
        eprintln!("moddil: {name} has no `fn main()`");
        return ExitCode::FAILURE;
    };
    if !items.fns[main as usize].params.is_empty() {
        eprintln!("moddil: `main` must not take parameters");
        return ExitCode::FAILURE;
    }
    match moddil_interp::call_fn(db, program, main, &[], &mut |_, _, _| {}) {
        Ok(Value::Unit) => ExitCode::SUCCESS,
        Ok(v) => {
            println!("{}", v.render(items));
            ExitCode::SUCCESS
        }
        Err(Stop::Trap(trap)) => {
            let (name, text) = &files[trap.file as usize];
            report_trap(name, text, items, &trap);
            ExitCode::from(TRAPPED)
        }
        // `main` cannot fail (R3), so no error can escape it.
        Err(Stop::Failed { .. }) => unreachable!("a checked `main` cannot fail"),
    }
}

fn test(db: &Database, program: Program, files: &[(String, String)]) -> ExitCode {
    let items = moddil_sema::item_tree(db, program);
    let mut failed = 0;
    let mut run = 0;
    for (i, t) in items.tests.iter().enumerate() {
        // The tests of the mod being built, not of its dependencies.
        if items.mod_of(t.module) != 0 {
            continue;
        }
        run += 1;
        match moddil_interp::run_test(db, program, i as u32, &mut |_, _, _| {}) {
            Ok(()) => println!("test \"{}\" ... ok", t.name),
            Err(Stop::Trap(trap)) => {
                failed += 1;
                println!("test \"{}\" ... trapped", t.name);
                let (name, text) = &files[trap.file as usize];
                report_trap(name, text, items, &trap);
            }
            Err(Stop::Failed { file, range, error }) => {
                failed += 1;
                println!("test \"{}\" ... failed", t.name);
                let (name, text) = &files[file as usize];
                report_error(name, text, items, range, &error);
            }
        }
    }
    let passed = run - failed;
    println!("{passed} passed, {failed} failed");
    if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// A problem reading the program (a manifest, a missing file).
fn report_problem(p: &moddil_project::Problem) {
    let name = p.display.as_str();
    if p.text.is_empty() {
        eprintln!("[{}] Error: {name}: {}", p.code, p.message);
        return;
    }
    let span = (name, p.range.clone());
    let r = Report::build(ReportKind::Error, span.clone())
        .with_config(config())
        .with_code(p.code)
        .with_message(&p.message)
        .with_label(Label::new(span).with_message("here"));
    let _ = r.finish().eprint((name, Source::from(p.text.as_str())));
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

/// An error that escaped a test: where it left the test, and the error.
fn report_error(name: &str, text: &str, items: &ItemTree, range: TextRange, error: &Value) {
    let span = (name, range.to_range());
    let report = Report::build(ReportKind::Error, span.clone())
        .with_config(config())
        .with_message(format!("error: {}", error.render(items)))
        .with_label(Label::new(span).with_message("the error leaves the test here"));
    let _ = report.finish().eprint((name, Source::from(text)));
}

fn report_trap(name: &str, text: &str, items: &ItemTree, trap: &Trap) {
    let span = (name, trap.range.to_range());
    let message = match &trap.reason {
        Some(r) => format!("trap: {}: {r}", trap.class),
        None => format!("trap: {}", trap.class),
    };
    let mut report = Report::build(ReportKind::Error, span.clone())
        .with_config(config())
        .with_message(message)
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
