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
fn a_trap_exits_with_101() {
    let path = file(
        "trap.moddil",
        "fn main() -> int { 9223372036854775807 + 1 }\n",
    );
    let out = moddil(&["run"], &path);
    assert_eq!(out.status.code(), Some(101));
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
