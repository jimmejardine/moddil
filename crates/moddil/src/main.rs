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
        /// Compile it to native code and run that (M1: scalars and calls so far), rather than
        /// interpret it.
        #[arg(long)]
        native: bool,
    },
    /// Runs a program's tests (a mod's, not its dependencies').
    Test {
        /// A file, or a mod's directory (with a `moddil.toml`).
        file: PathBuf,
        /// Compile each test to native code and run that, rather than interpret it.
        #[arg(long)]
        native: bool,
    },
}

/// The exit code for a trap, as for a Rust panic.
const TRAPPED: u8 = 101;

fn main() -> ExitCode {
    let cli = Cli::parse();
    let path = match &cli.command {
        Command::Check { file } | Command::Run { file, .. } | Command::Test { file, .. } => file,
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
        Command::Run { native: true, .. } => {
            run_native(&db, program, &files, &path.display().to_string())
        }
        Command::Run { .. } => run(&db, program, &files, &path.display().to_string()),
        Command::Test { native: true, .. } => test_native(&db, program, &files),
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
            stub: m.stub,
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
    // The root capabilities `main` takes (`main.root.1`): checked, so each is a struct marked
    // `#[root_capability]`, made here with its fields (its handle) zero.
    let roots: Vec<Value> = items.fns[main as usize]
        .params
        .iter()
        .map(|p| match p.ty {
            moddil_sema::Ty::Adt(adt, ..) => Value::Adt {
                adt,
                variant: 0,
                fields: items.adts[adt as usize].variants[0]
                    .fields
                    .iter()
                    .map(|_| Value::Int(0))
                    .collect(),
            },
            _ => unreachable!("a checked `main` takes root capabilities"),
        })
        .collect();
    match moddil_interp::call_fn(db, program, main, &roots, &mut |_, _, _| {}) {
        Ok(Value::Unit) => ExitCode::SUCCESS,
        // An `ExitCode` (`main.exit.1`): the number it stands for.
        Ok(Value::Adt {
            adt,
            variant,
            fields,
        }) if items.adts[adt as usize].src == moddil_sema::Src::Prelude
            && items.adts[adt as usize].name == "ExitCode" =>
        {
            const CODES: [u8; 16] = [
                0, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78,
            ];
            match (CODES.get(variant as usize), fields.first()) {
                (Some(&c), _) => ExitCode::from(c),
                (None, Some(Value::Int(n))) => ExitCode::from((*n).clamp(0, 255) as u8),
                _ => ExitCode::FAILURE,
            }
        }
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

/// `moddil run --native`: `main` compiled by the native backend and run.
fn run_native(db: &Database, program: Program, files: &[(String, String)], name: &str) -> ExitCode {
    let items = moddil_sema::item_tree(db, program);
    let Some(main) = items.main_fn() else {
        eprintln!("moddil: {name} has no `fn main()`");
        return ExitCode::FAILURE;
    };
    match moddil_native::run_main(db, program, main) {
        Ok(v) if v == "()" => ExitCode::SUCCESS,
        Ok(v) => {
            println!("{v}");
            ExitCode::SUCCESS
        }
        Err(moddil_native::Native::Trap(class, file, range)) => {
            let (name, text) = &files[file as usize];
            report_native_trap(name, text, class, range);
            ExitCode::from(TRAPPED)
        }
        Err(moddil_native::Native::Unsupported(what)) => {
            eprintln!("moddil: not supported natively yet: {what}");
            ExitCode::FAILURE
        }
        Err(moddil_native::Native::Leak(n)) => {
            eprintln!("moddil: native code leaked {n} allocations (a bug in the backend)");
            ExitCode::FAILURE
        }
    }
}

/// `moddil test --native`: each test compiled by the native backend and run.
fn test_native(db: &Database, program: Program, files: &[(String, String)]) -> ExitCode {
    let items = moddil_sema::item_tree(db, program);
    let mut failed = 0;
    let mut run = 0;
    let mut unsupported = 0;
    for (i, t) in items.tests.iter().enumerate() {
        if items.mod_of(t.module) != 0 {
            continue;
        }
        run += 1;
        match moddil_native::run_test(db, program, i as u32).1 {
            Ok(()) => println!("test \"{}\" ... ok", t.name),
            Err(moddil_native::Native::Trap(class, file, range)) => {
                failed += 1;
                println!("test \"{}\" ... trapped", t.name);
                let (name, text) = &files[file as usize];
                report_native_trap(name, text, class, range);
            }
            Err(moddil_native::Native::Unsupported(what)) => {
                unsupported += 1;
                println!("test \"{}\" ... not supported natively yet: {what}", t.name);
            }
            Err(moddil_native::Native::Leak(n)) => {
                failed += 1;
                println!("test \"{}\" ... leaked {n} allocations natively", t.name);
            }
        }
    }
    let passed = run - failed - unsupported;
    println!("{passed} passed, {failed} failed, {unsupported} not supported natively yet");
    if failed == 0 && unsupported == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn report_native_trap(name: &str, text: &str, class: moddil_mir::TrapClass, range: TextRange) {
    let span = (name, range.to_range());
    let report = Report::build(ReportKind::Error, span.clone())
        .with_config(config())
        .with_message(format!("trap: {}", class.name()))
        .with_label(Label::new(span).with_message("here"));
    let _ = report.finish().eprint((name, Source::from(text)));
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
