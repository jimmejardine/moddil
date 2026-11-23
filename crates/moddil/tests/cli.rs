//! Runs the `moddil` binary on small files: `check`, `run` and `test`, with their exit codes.

use std::path::PathBuf;
use std::process::{Command, Output};

/// Writes `text` to a file named `name` in a fresh directory under the target directory.
fn file(name: &str, text: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("cli");
    std::fs::create_dir_all(&dir).expect("a temporary directory");
    let path = dir.join(name);
    std::fs::write(&path, text).expect("written");
    path
}

fn moddil(args: &[&str], path: &PathBuf) -> Output {
    Command::new(env!("CARGO_BIN_EXE_moddil"))
        .args(args)
        .arg(path)
        .output()
        .expect("moddil runs")
}

const PROGRAM: &str = "\
fn fib(n: int) -> int { if n < 2 { n } else { fib(n - 1) + fib(n - 2) } }
fn main() -> int { fib(20) }
test \"fib\" { fib(10) }
";

#[test]
fn runs_main_and_prints_its_result() {
    let out = moddil(&["run"], &file("fib.moddil", PROGRAM));
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "6765");
}

#[test]
fn runs_tests() {
    let out = moddil(&["test"], &file("fib-test.moddil", PROGRAM));
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("test \"fib\" ... ok"), "{stdout}");
}

#[test]
fn reports_errors_with_codes() {
    let out = moddil(
        &["check"],
        &file("bad.moddil", "fn main() -> int { true }\n"),
    );
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("E0201"), "{stderr}");
}

#[test]
fn a_trap_exits_with_70() {
    let path = file(
        "trap.moddil",
        "fn main() -> int { let max = 9223372036854775807; max + 1 }\n",
    );
    let out = moddil(&["run"], &path);
    assert_eq!(out.status.code(), Some(70));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("trap: overflow"), "{stderr}");
}

#[test]
fn a_failed_assert_shows_its_parts() {
    let text = "fn fee(a: int) -> int { a / 10 }\n\
                test \"fees\" { let amount = 250; assert(fee(amount) + amount == 260); }\n";
    let out = moddil(&["test"], &file("assert.moddil", text));
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("trap: assertion failed"), "{stderr}");
    assert!(stderr.contains("fee(amount) + amount"), "{stderr}");
    assert!(stderr.contains("= 275"), "{stderr}");
}

#[test]
fn runs_as_wasm_when_asked() {
    let out = moddil(&["run", "--wasm"], &file("fib-wasm.moddil", PROGRAM));
    let stderr = String::from_utf8_lossy(&out.stderr);
    if stderr.contains("no Node") {
        return;
    }
    assert!(out.status.success(), "{stderr}");
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "6765");
    let out = moddil(&["test", "--wasm"], &file("fib-wasm-test.moddil", PROGRAM));
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("test \"fib\" ... ok"));
}

#[test]
fn runs_on_the_interpreter_when_asked() {
    let out = moddil(&["run", "--interp"], &file("fib-interp.moddil", PROGRAM));
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "6765");
}

#[test]
fn falls_back_to_the_interpreter_with_a_note() {
    // A `dyn` borrow as a supertrait's that is not its first is not compiled natively yet.
    let text = "trait A { fn a(&self) -> int; }  trait B { fn b(&self) -> int; }  trait C: A + B {}
        struct S;  impl A for S { fn a(&self) -> int { 1 } }  impl B for S { fn b(&self) -> int { 2 } }
        impl C for S {}
        fn second(x: &dyn B) -> int { x.b() }  fn via(c: &dyn C) -> int { second(c) }
        fn main() -> int { via(&S) }\n";
    let out = moddil(&["run"], &file("fallback.moddil", text));
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "2");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("running on the interpreter"), "{stderr}");
}

/// Builds `text` into an executable and runs it, if the runtime library is there to link with
/// (`cargo build -p moddil-rt-main`): its output, or `None`.
fn built(name: &str, text: &str) -> Option<Output> {
    let path = file(&format!("{name}.moddil"), text);
    let exe = path.with_extension(std::env::consts::EXE_EXTENSION);
    let out = Command::new(env!("CARGO_BIN_EXE_moddil"))
        .arg("build")
        .arg(&path)
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("moddil runs");
    let stderr = String::from_utf8_lossy(&out.stderr);
    if stderr.contains("the runtime library") || stderr.contains("link.exe") {
        eprintln!("skipped: {stderr}");
        return None;
    }
    assert!(out.status.success(), "{stderr}");
    Some(Command::new(&exe).output().expect("the built program runs"))
}

#[test]
fn a_built_program_exits_with_its_exit_code() {
    let text = "fn fib(n: int) -> int { if n < 2 { n } else { fib(n - 1) + fib(n - 2) } }
        fn main() -> ExitCode { if fib(20) == 6765 { ExitCode::Config } else { ExitCode::Ok } }\n";
    if let Some(out) = built("exit", text) {
        assert_eq!(out.status.code(), Some(78));
    }
}

#[test]
fn a_built_program_reports_a_trap_and_exits_with_70() {
    let text = "fn main() { let v = Vec::from_array([1, 2]); let i = 5; let x = v[i]; }\n";
    if let Some(out) = built("boom", text) {
        assert_eq!(out.status.code(), Some(70));
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("trap: index out of bounds"), "{stderr}");
        assert!(stderr.contains("boom.moddil:1:"), "{stderr}");
    }
}

#[test]
fn run_exits_with_mains_exit_code() {
    let path = file(
        "code.moddil",
        "fn main() -> ExitCode { ExitCode::new(42) }\n",
    );
    assert_eq!(moddil(&["run"], &path).status.code(), Some(42));
    assert_eq!(moddil(&["run", "--interp"], &path).status.code(), Some(42));
}

#[test]
fn a_main_with_root_capabilities_runs_and_builds() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../conformance/2027/main/roots");
    let out = moddil(&["run"], &dir);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let exe = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("cli")
        .join("roots")
        .with_extension(std::env::consts::EXE_EXTENSION);
    let built = Command::new(env!("CARGO_BIN_EXE_moddil"))
        .arg("build")
        .arg(&dir)
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("moddil runs");
    let stderr = String::from_utf8_lossy(&built.stderr);
    if stderr.contains("the runtime library") || stderr.contains("link.exe") {
        return;
    }
    assert!(built.status.success(), "{stderr}");
    assert_eq!(
        Command::new(&exe).status().expect("it runs").code(),
        Some(0)
    );
}

#[test]
fn a_release_build_runs_and_splits_its_line_tables() {
    let path = file(
        "release.moddil",
        "fn fib(n: int) -> int { if n < 2 { n } else { fib(n - 1) + fib(n - 2) } }
        fn main() -> ExitCode { if fib(20) == 6765 { ExitCode::Config } else { ExitCode::Ok } }\n",
    );
    let exe = path.with_extension(std::env::consts::EXE_EXTENSION);
    let out = Command::new(env!("CARGO_BIN_EXE_moddil"))
        .args(["build", "--release"])
        .arg(&path)
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("moddil runs");
    let stderr = String::from_utf8_lossy(&out.stderr);
    if stderr.contains("the runtime library")
        || stderr.contains("link.exe")
        || stderr.contains("rust-lld")
    {
        eprintln!("skipped: {stderr}");
        return;
    }
    assert!(out.status.success(), "{stderr}");
    assert_eq!(
        Command::new(&exe).status().expect("it runs").code(),
        Some(78)
    );
    if cfg!(target_os = "linux") {
        assert!(
            exe.with_extension("debug").exists(),
            "the line tables split off"
        );
    }
}

/// Runs the tests of the project at `dir` (from the repository's root) natively, on the MIR
/// interpreter and, where Node has JSPI, as WASM: each run passes all `n`, natively compiled
/// (none falls back to the interpreter).
fn passes_everywhere(dir: &str, n: usize) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(dir);
    let wasm = Command::new(std::env::var("MODDIL_NODE").unwrap_or_else(|_| "node".to_string()))
        .args([
            "-e",
            "process.exit(typeof WebAssembly.Suspending === 'function' ? 0 : 1)",
        ])
        .status()
        .is_ok_and(|s| s.success());
    for how in [&["test"][..], &["test", "--interp"], &["test", "--wasm"]] {
        if how.contains(&"--wasm") && !wasm {
            continue;
        }
        let out = moddil(how, &dir);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{how:?}: {stdout}{stderr}");
        assert!(
            stdout.contains(&format!("{n} passed, 0 failed")),
            "{how:?}: {stdout}"
        );
        assert!(
            !stderr.contains("runs on the interpreter"),
            "{how:?}: {stderr}"
        );
    }
}

#[test]
fn the_standard_mods_pass_their_tests_everywhere() {
    for (dir, n) in [
        ("mods/collections/tests", 1),
        ("mods/sync/tests", 2),
        ("mods/rand/tests", 1),
        ("mods/serde/tests", 4),
        ("mods/net/tests", 4),
        ("mods/sim/tests", 3),
    ] {
        passes_everywhere(dir, n);
    }
}

#[test]
fn the_tour_passes_its_tests_everywhere() {
    passes_everywhere("tour/shorty", 3);
}
