//! The `moddil` command: it checks programs, runs them and their tests (compiled to native code,
//! or on the MIR interpreter), and builds them into executables. See `crates/moddil/README.md`.

mod link;

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
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
    /// Runs a program's `fn main()`, compiled to native code, and prints its result.
    Run {
        /// A file, or a mod's directory (with a `moddil.toml`).
        file: PathBuf,
        /// Run it on the MIR interpreter instead.
        #[arg(long)]
        interp: bool,
        /// Compile it to WebAssembly instead, and run that in Node (M1, slice W0).
        #[arg(long, conflicts_with = "interp")]
        wasm: bool,
    },
    /// Runs a program's tests (a mod's, not its dependencies'), compiled to native code.
    Test {
        /// A file, or a mod's directory (with a `moddil.toml`).
        file: PathBuf,
        /// Run them on the MIR interpreter instead.
        #[arg(long)]
        interp: bool,
        /// Compile them to WebAssembly instead, and run them in Node (M1, slice W0).
        #[arg(long, conflicts_with = "interp")]
        wasm: bool,
    },
    /// Compiles a program's `fn main()` ahead of time into an executable.
    Build {
        /// A file, or a mod's directory (with a `moddil.toml`).
        file: PathBuf,
        /// The executable to write (by default, the file's name, here); with `--wasm`, the
        /// glue (`name.mjs`), the other files beside it.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Build for the web instead: core WebAssembly modules and the JS glue that runs them,
        /// in a browser or in Node (`name.mjs`, `name.wasm`, `moddil-rt.wasm`, `name.html`).
        #[arg(long)]
        wasm: bool,
        /// A release build: linked with `rust-lld` (or `mold`), with line tables split off
        /// into a debug file where the platform has one.
        #[arg(long, conflicts_with = "wasm")]
        release: bool,
    },
    /// Runs test `index` natively, for `moddil test`, which runs each in a process of its own.
    #[command(name = "__native-test", hide = true)]
    NativeTest { file: PathBuf, index: u32 },
}

/// The exit code for a trap: 70 (`EX_SOFTWARE`), as a built program's (`lifecycle.md`).
const TRAPPED: u8 = 70;

fn main() -> ExitCode {
    let cli = Cli::parse();
    let path = match &cli.command {
        Command::Check { file }
        | Command::Run { file, .. }
        | Command::Test { file, .. }
        | Command::Build { file, .. }
        | Command::NativeTest { file, .. } => file,
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
    let name = path.display().to_string();
    match &cli.command {
        Command::Check { .. } => ExitCode::SUCCESS,
        Command::Run { interp: true, .. } => run(&db, program, &files, &name),
        Command::Run { wasm: true, .. } => run_wasm(&db, program, &files, &name),
        Command::Test { wasm: true, .. } => test_wasm(&db, program, &files),
        Command::Run { .. } => run_native(&db, program, &files, &name),
        Command::Test { interp, .. } => test(&db, program, &files, *interp, path),
        Command::Build {
            output, wasm: true, ..
        } => build_wasm(&db, program, path, output.as_deref()),
        Command::Build {
            output, release, ..
        } => build(&db, program, path, output.as_deref(), *release),
        Command::NativeTest { index, .. } => native_test(&db, program, &files, *index),
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
            unsafe_ok: m.unsafe_ok,
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

/// `moddil run`: `main` compiled to native code and run; on the MIR interpreter instead, with a
/// note, if it reaches something not compiled natively yet.
fn run_native(db: &Database, program: Program, files: &[(String, String)], name: &str) -> ExitCode {
    let items = moddil_sema::item_tree(db, program);
    let Some(main) = items.main_fn() else {
        eprintln!("moddil: {name} has no `fn main()`");
        return ExitCode::FAILURE;
    };
    match moddil_native::run_main_exit(db, program, main) {
        // An `ExitCode` (`main.exit.1`): the number it stands for.
        Ok((_, Some(code))) => ExitCode::from(code),
        Ok((v, None)) if v == "()" => ExitCode::SUCCESS,
        Ok((v, None)) => {
            println!("{v}");
            ExitCode::SUCCESS
        }
        Err(moddil_native::Native::Trap(class, file, range, parts)) => {
            let (name, text) = &files[file as usize];
            report_native_trap(name, text, class, range, &parts);
            ExitCode::from(TRAPPED)
        }
        Err(moddil_native::Native::Unsupported(what)) => {
            eprintln!("moddil: running on the interpreter: {what} is not compiled natively yet");
            run(db, program, files, name)
        }
        Err(moddil_native::Native::Leak(n)) => {
            eprintln!("moddil: native code leaked {n} allocations (a bug in the backend)");
            ExitCode::FAILURE
        }
        Err(moddil_native::Native::Failed(file, range, error)) => {
            let (name, text) = &files[file as usize];
            report_native_error(name, text, range, &error);
            ExitCode::FAILURE
        }
    }
}

/// `moddil run --wasm`: `main` compiled to WebAssembly and run in Node.
fn run_wasm(db: &Database, program: Program, files: &[(String, String)], name: &str) -> ExitCode {
    let items = moddil_sema::item_tree(db, program);
    let Some(main) = items.main_fn() else {
        eprintln!("moddil: {name} has no `fn main()`");
        return ExitCode::FAILURE;
    };
    match moddil_wasm::run_main(db, program, main) {
        // An `ExitCode` (`main.exit.1`): the number it stands for.
        Ok((_, Some(code))) => ExitCode::from(code),
        Ok((v, None)) if v == "()" => ExitCode::SUCCESS,
        Ok((v, None)) => {
            println!("{v}");
            ExitCode::SUCCESS
        }
        Err(moddil_wasm::Wasm::Trap(class, file, range, parts)) => {
            let (name, text) = &files[file as usize];
            report_native_trap(name, text, class, range, &parts);
            ExitCode::from(TRAPPED)
        }
        Err(moddil_wasm::Wasm::Failed(file, range, error)) => {
            let (name, text) = &files[file as usize];
            report_native_error(name, text, range, &error);
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("moddil: {}", wasm_problem(&e));
            ExitCode::FAILURE
        }
    }
}

/// What stopped a WASM run, for a person.
fn wasm_problem(e: &moddil_wasm::Wasm) -> String {
    match e {
        moddil_wasm::Wasm::Unsupported(what) => format!("not compiled to WASM yet: {what}"),
        moddil_wasm::Wasm::NoHost(why) => format!("no Node to run WASM in: {why}"),
        moddil_wasm::Wasm::Crash(how) => format!("the WASM run stopped (a backend bug): {how}"),
        moddil_wasm::Wasm::Leak(n) => {
            format!("the WASM run leaked {n} allocations (a backend bug)")
        }
        moddil_wasm::Wasm::Trap(..) => "a trap".to_string(),
        moddil_wasm::Wasm::Failed(..) => "an error".to_string(),
    }
}

/// `moddil test --wasm`: the mod's tests compiled into one WebAssembly module, run in Node.
fn test_wasm(db: &Database, program: Program, files: &[(String, String)]) -> ExitCode {
    let items = moddil_sema::item_tree(db, program);
    let tests: Vec<u32> = (0..items.tests.len() as u32)
        .filter(|&i| items.mod_of(items.tests[i as usize].module) == 0)
        .collect();
    let runs = match moddil_wasm::run_tests(db, program, &tests) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("moddil: {}", wasm_problem(&e));
            return ExitCode::FAILURE;
        }
    };
    let mut failed = 0;
    for (&i, (_, result)) in tests.iter().zip(&runs) {
        let name = &items.tests[i as usize].name;
        match result {
            Ok(()) => println!("test \"{name}\" ... ok"),
            Err(moddil_wasm::Wasm::Trap(class, file, range, parts)) => {
                failed += 1;
                println!("test \"{name}\" ... trapped");
                let (fname, text) = &files[*file as usize];
                report_native_trap(fname, text, *class, *range, parts);
            }
            Err(moddil_wasm::Wasm::Failed(file, range, error)) => {
                failed += 1;
                println!("test \"{name}\" ... failed");
                let (fname, text) = &files[*file as usize];
                report_native_error(fname, text, *range, error);
            }
            Err(e) => {
                failed += 1;
                println!("test \"{name}\" ... {}", wasm_problem(e));
            }
        }
    }
    println!("{} passed, {failed} failed", tests.len() - failed);
    if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// `moddil __native-test`: test `index` run natively, in a process of `moddil test`'s. Its first
/// line of output says how it went (`ok`, `trapped`, `failed`, `leaked`, or `unsupported` and
/// why); reports go to standard error.
fn native_test(
    db: &Database,
    program: Program,
    files: &[(String, String)],
    index: u32,
) -> ExitCode {
    match moddil_native::run_test(db, program, index).1 {
        Ok(()) => println!("ok"),
        Err(moddil_native::Native::Trap(class, file, range, parts)) => {
            println!("trapped");
            let (name, text) = &files[file as usize];
            report_native_trap(name, text, class, range, &parts);
        }
        Err(moddil_native::Native::Unsupported(what)) => println!("unsupported {what}"),
        Err(moddil_native::Native::Leak(n)) => {
            println!("leaked");
            eprintln!("moddil: native code leaked {n} allocations (a bug in the backend)");
        }
        Err(moddil_native::Native::Failed(file, range, error)) => {
            println!("failed");
            let (name, text) = &files[file as usize];
            report_native_error(name, text, range, &error);
        }
    }
    ExitCode::SUCCESS
}

/// How one native test, run in a process of its own, went.
enum Outcome {
    Passed,
    Failed,
    /// It reaches something not compiled natively yet: what.
    Unsupported(String),
}

/// Runs test `index` natively in a child process (so that a crash in compiled code fails it,
/// not the run), printing its line and reports as `moddil test` does.
fn native_child(path: &Path, index: u32, name: &str) -> Outcome {
    let out = std::env::current_exe().and_then(|exe| {
        std::process::Command::new(exe)
            .arg("__native-test")
            .arg(path)
            .arg(index.to_string())
            .output()
    });
    let Ok(out) = out else {
        println!("test \"{name}\" ... could not start a process to run it in");
        return Outcome::Failed;
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    let status = stdout.lines().next().unwrap_or("");
    let outcome = match status.split_once(' ').map_or(status, |(s, _)| s) {
        "ok" => {
            println!("test \"{name}\" ... ok");
            return Outcome::Passed;
        }
        "unsupported" => {
            return Outcome::Unsupported(status["unsupported ".len()..].to_string());
        }
        "trapped" | "failed" | "leaked" => {
            println!("test \"{name}\" ... {status}");
            Outcome::Failed
        }
        _ => {
            println!("test \"{name}\" ... crashed (a bug in the native backend)");
            Outcome::Failed
        }
    };
    eprint!("{}", String::from_utf8_lossy(&out.stderr));
    outcome
}

/// `moddil build --wasm`: `main` compiled to WebAssembly, written as the files that run it in a
/// browser or in Node.
fn build_wasm(db: &Database, program: Program, path: &Path, output: Option<&Path>) -> ExitCode {
    let name = path.display();
    let items = moddil_sema::item_tree(db, program);
    let Some(main) = items.main_fn() else {
        eprintln!("moddil: {name} has no `fn main()`");
        return ExitCode::FAILURE;
    };
    let glue = output.map_or_else(
        || {
            let stem = path
                .file_stem()
                .map_or_else(|| "main".into(), |s| s.to_os_string());
            PathBuf::from(stem).with_extension("mjs")
        },
        Path::to_path_buf,
    );
    let stem = glue
        .file_stem()
        .map_or_else(|| "main".to_string(), |s| s.to_string_lossy().into_owned());
    let dir = glue.parent().unwrap_or(Path::new("")).to_path_buf();
    let p = match moddil_wasm::package(db, program, main, &stem) {
        Ok(p) => p,
        Err(what) => {
            eprintln!("moddil: {name} cannot be built for WASM yet: {what}");
            return ExitCode::FAILURE;
        }
    };
    let files: [(&str, &[u8]); 4] = [
        (&p.glue.0, p.glue.1.as_bytes()),
        (&p.program.0, &p.program.1),
        (&p.runtime.0, p.runtime.1),
        (&p.html.0, p.html.1.as_bytes()),
    ];
    for (file, bytes) in files {
        let at = dir.join(file);
        if let Err(e) = std::fs::write(&at, bytes) {
            eprintln!("moddil: writing {}: {e}", at.display());
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}

/// `moddil build`: `main` compiled ahead of time, linked into an executable.
fn build(
    db: &Database,
    program: Program,
    path: &Path,
    output: Option<&Path>,
    release: bool,
) -> ExitCode {
    let name = path.display();
    let items = moddil_sema::item_tree(db, program);
    let Some(main) = items.main_fn() else {
        eprintln!("moddil: {name} has no `fn main()`");
        return ExitCode::FAILURE;
    };
    let built = if release {
        moddil_native::build_release_object(db, program, main)
    } else {
        moddil_native::build_object(db, program, main)
    };
    let object = match built {
        Ok(o) => o,
        Err(moddil_native::Native::Unsupported(what)) => {
            eprintln!("moddil: {name} cannot be built yet: {what}");
            return ExitCode::FAILURE;
        }
        Err(_) => return ExitCode::FAILURE,
    };
    let out = output.map_or_else(
        || {
            let stem = path
                .file_stem()
                .map_or_else(|| "main".into(), |s| s.to_os_string());
            PathBuf::from(stem).with_extension(std::env::consts::EXE_EXTENSION)
        },
        Path::to_path_buf,
    );
    match link::link(&object, &out, release) {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            eprintln!("moddil: linking {}: {why}", out.display());
            ExitCode::FAILURE
        }
    }
}

fn report_native_trap(
    name: &str,
    text: &str,
    class: moddil_mir::TrapClass,
    range: TextRange,
    parts: &[(TextRange, Option<String>)],
) {
    let span = (name, range.to_range());
    let mut report = Report::build(ReportKind::Error, span.clone())
        .with_config(config())
        .with_message(format!("trap: {class}"))
        .with_label(Label::new(span).with_message("here"));
    // A failed `assert` shows the value of each part of its condition.
    if !parts.is_empty() {
        report = report.with_note(parts_note(text, parts));
    }
    let _ = report.finish().eprint((name, Source::from(text)));
}

/// Each part of a failed `assert`'s condition, its source and its value, a line each.
fn parts_note(text: &str, parts: &[(TextRange, Option<String>)]) -> String {
    let width = parts
        .iter()
        .map(|(r, _)| text[r.to_range()].len())
        .max()
        .unwrap_or(0);
    parts
        .iter()
        .map(|(r, v)| {
            let shown = v.as_deref().unwrap_or("not evaluated");
            format!("{:width$} = {shown}", &text[r.to_range()])
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `moddil test`: each test of the mod being built, compiled to native code (each in a process
/// of its own) or, with `--interp`, on the MIR interpreter. A test native code does not compile
/// yet runs on the interpreter, with a note.
fn test(
    db: &Database,
    program: Program,
    files: &[(String, String)],
    interp: bool,
    path: &Path,
) -> ExitCode {
    let items = moddil_sema::item_tree(db, program);
    let mut failed = 0;
    let mut run = 0;
    for (i, t) in items.tests.iter().enumerate() {
        // The tests of the mod being built, not of its dependencies.
        if items.mod_of(t.module) != 0 {
            continue;
        }
        run += 1;
        let passed = if interp {
            interp_test(db, program, files, i as u32)
        } else {
            match native_child(path, i as u32, &t.name) {
                Outcome::Passed => true,
                Outcome::Failed => false,
                Outcome::Unsupported(what) => {
                    eprintln!(
                        "moddil: test \"{}\" runs on the interpreter: {what} is not compiled \
                         natively yet",
                        t.name
                    );
                    interp_test(db, program, files, i as u32)
                }
            }
        };
        if !passed {
            failed += 1;
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

/// Test `index` on the MIR interpreter, printed as `moddil test` prints it: whether it passed.
fn interp_test(db: &Database, program: Program, files: &[(String, String)], index: u32) -> bool {
    let items = moddil_sema::item_tree(db, program);
    let t = &items.tests[index as usize];
    match moddil_interp::run_test(db, program, index, &mut |_, _, _| {}) {
        Ok(()) => {
            println!("test \"{}\" ... ok", t.name);
            true
        }
        Err(Stop::Trap(trap)) => {
            println!("test \"{}\" ... trapped", t.name);
            let (name, text) = &files[trap.file as usize];
            report_trap(name, text, items, &trap);
            false
        }
        Err(Stop::Failed { file, range, error }) => {
            println!("test \"{}\" ... failed", t.name);
            let (name, text) = &files[file as usize];
            report_error(name, text, items, range, &error);
            false
        }
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
/// An error that left a natively run test or `main`, as [`report_error`] reports one.
fn report_native_error(name: &str, text: &str, range: TextRange, error: &str) {
    let span = (name, range.to_range());
    let report = Report::build(ReportKind::Error, span.clone())
        .with_config(config())
        .with_message(format!("error: {error}"))
        .with_label(Label::new(span).with_message("the error leaves the test here"));
    let _ = report.finish().eprint((name, Source::from(text)));
}

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
