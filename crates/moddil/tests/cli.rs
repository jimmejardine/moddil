//! Runs the `moddil` binary on small files: `check`, `run` and `test`, with their exit codes.

use std::path::PathBuf;
use std::process::{Command, Output};

mod runtime;

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

/// A `// =>` line is an expected value (`testing.md`: Examples are tests): a test fails where the
/// value is another, or the line never runs, alike everywhere.
#[test]
fn a_test_checks_its_expected_values_everywhere() {
    let text = "test \"right\" { 1 + 2 // => 3\n}\n\
                test \"wrong\" { 1 + 2 // => 4\n}\n\
                test \"never\" { if false { 5 // => 5\n ; } }\n";
    let path = file("expected-values.moddil", text);
    for how in [&["test"][..], &["test", "--interp"], &["test", "--wasm"]] {
        let out = moddil(how, &path);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        if how.contains(&"--wasm") && stderr.contains("no Node") {
            continue;
        }
        assert_eq!(out.status.code(), Some(1), "{how:?}: {stdout}{stderr}");
        assert!(
            stdout.contains("test \"right\" ... ok"),
            "{how:?}: {stdout}"
        );
        assert!(
            stdout.contains("test \"wrong\" ... failed"),
            "{how:?}: {stdout}"
        );
        assert!(
            stdout.contains("test \"never\" ... failed"),
            "{how:?}: {stdout}"
        );
        assert!(stdout.contains("1 passed, 2 failed"), "{how:?}: {stdout}");
        assert!(stderr.contains("expected 4, found 3"), "{how:?}: {stderr}");
        assert!(
            stderr.contains("expected 5, but the line never ran"),
            "{how:?}: {stderr}"
        );
    }
}

/// A part borrowed where it lives (a method's receiver, and what it is a field of) is shown from
/// there, and one the condition moves away later is not shown, alike everywhere.
#[test]
fn a_failed_assert_shows_borrowed_parts_everywhere() {
    let text = "fn has(v: Vec<int>, n: int) -> bool { v.len() == n }\n\
                struct Bag { items: Vec<int> }\n\
                test \"borrowed\" { let b = Bag { items: [7] }; assert(b.items.len() == 2); }\n\
                test \"moved\" { let v: Vec<int> = [1, 2]; assert(v.len() == 2 && has(v, 9)); }\n";
    let path = file("assert-borrowed.moddil", text);
    for how in [&["test"][..], &["test", "--interp"], &["test", "--wasm"]] {
        let out = moddil(how, &path);
        let stderr = String::from_utf8_lossy(&out.stderr);
        if how.contains(&"--wasm") && stderr.contains("no Node") {
            continue;
        }
        assert_eq!(out.status.code(), Some(1), "{how:?}: {stderr}");
        assert!(!stderr.contains("not evaluated"), "{how:?}: {stderr}");
        assert!(
            stderr.contains("b.items            = [7]"),
            "{how:?}: {stderr}"
        );
        assert!(
            stderr.contains("b                  = Bag { items: [7] }"),
            "{how:?}: {stderr}"
        );
        // `v` was moved to `has` after `v.len()` borrowed it: not there to show.
        assert!(!stderr.contains("v      "), "{how:?}: {stderr}");
        assert!(
            stderr.contains("has(v, 9)                 = false"),
            "{how:?}: {stderr}"
        );
    }
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

/// Builds `text` into an executable and runs it, if the runtime library is there to link with
/// (`cargo build -p moddil-rt-main`): its output, or `None`.
fn built(name: &str, text: &str) -> Option<Output> {
    runtime::ensure();
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

/// A built program's `main` may give any type: it prints it as `moddil run` does, and exits 0.
#[test]
fn a_built_program_prints_its_result_as_run_does() {
    let text = "struct Node { value: int, next: Option<Box<Node>> }
enum Shape { Dot, Circle(f64), Box { w: int } }

fn main() -> (Vec<Shape>, Node, Option<&str>, u128, String) {
    let shapes: Vec<Shape> = [Shape::Dot, Shape::Circle(1.5), Shape::Box { w: 3 }];
    let list = Node { value: 1, next: Some(Box::new(Node { value: 2, next: None })) };
    let big: u128 = 340282366920938463463374607431768211455;
    (shapes, list, Some(\"s\"), big, \"text longer than twenty-three bytes\".to_string())
}
";
    let Some(out) = built("shown", text) else {
        return;
    };
    assert_eq!(out.status.code(), Some(0));
    let run = moddil(&["run"], &file("shown.moddil", text));
    let (built, ran) = (
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&run.stdout),
    );
    assert_eq!(built, ran);
    assert!(
        built.contains("Node { value: 1, next: Some(Box { "),
        "{built}"
    );
    // `()` is not printed.
    if let Some(out) = built_unit() {
        assert!(out.stdout.is_empty() && out.status.success());
    }
}

/// A built `main` giving `()`.
fn built_unit() -> Option<Output> {
    built(
        "unit",
        "fn main() { let x = 1; }
",
    )
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

/// Its `main` reads the real time through `Clock`, so exiting with 0 says the clock is the
/// system's, in each runner (`os.clock.1`).
#[test]
fn a_main_with_root_capabilities_runs_and_builds() {
    runtime::ensure();
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../conformance/2027/main/roots");
    for how in [&["run"][..], &["run", "--interp"], &["run", "--wasm"]] {
        let out = moddil(how, &dir);
        assert!(
            out.status.success(),
            "{how:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
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
    runtime::ensure();
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

/// Sends `request` to `port` on this machine, and gives the whole response.
fn exchange(port: u16, request: &str) -> String {
    use std::io::{Read, Write};
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).expect("the server answers");
    s.set_read_timeout(Some(std::time::Duration::from_secs(30)))
        .expect("a timeout");
    s.write_all(request.as_bytes()).expect("the request goes");
    let mut out = String::new();
    s.read_to_string(&mut out).expect("a response");
    out
}

/// A port nothing listens on, for now.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .expect("a free port")
        .port()
}

/// The tour serves HTTP for real (`os.1`, M2d's gate): it runs, creates a link, follows it,
/// and stops at a `stop` line on standard input, exiting with 0.
fn the_tour_serves(how: &[&str]) {
    use std::io::Write;
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tour/shorty");
    let port = free_port();
    let mut child = Command::new(env!("CARGO_BIN_EXE_moddil"))
        .arg("run")
        .args(how)
        .arg(&dir)
        .args(["--", "--port", &port.to_string()])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("moddil runs");
    // Ready once it accepts a connection.
    let start = std::time::Instant::now();
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(
            start.elapsed() < std::time::Duration::from_secs(120),
            "{how:?}: the server did not start"
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let body = r#"{"url": "https://moddil.com"}"#;
    let created = exchange(
        port,
        &format!(
            "POST /links HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        ),
    );
    assert!(created.starts_with("HTTP/1.1 201"), "{how:?}: {created}");
    let code = created
        .split("\"code\":\"")
        .nth(1)
        .and_then(|r| r.split('"').next())
        .expect("a code");
    let followed = exchange(
        port,
        &format!("GET /{code} HTTP/1.1\r\nHost: localhost\r\n\r\n"),
    );
    assert!(followed.starts_with("HTTP/1.1 308"), "{how:?}: {followed}");
    assert!(
        followed.contains("Location: https://moddil.com"),
        "{how:?}: {followed}"
    );
    let plain = r#"{"url": "http://moddil.com"}"#;
    let refused = exchange(
        port,
        &format!(
            "POST /links HTTP/1.1\r\nContent-Length: {}\r\n\r\n{plain}",
            plain.len()
        ),
    );
    assert!(refused.starts_with("HTTP/1.1 400"), "{how:?}: {refused}");
    let mut stdin = child.stdin.take().expect("a pipe");
    writeln!(stdin, "stop").expect("the stop goes");
    let out = child.wait_with_output().expect("it stops");
    assert!(
        out.status.success(),
        "{how:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !String::from_utf8_lossy(&out.stderr).contains("on the interpreter"),
        "{how:?}: it fell back to the interpreter"
    );
}

#[test]
fn the_tour_serves_natively() {
    the_tour_serves(&[]);
}

#[test]
fn the_tour_serves_on_the_interpreter() {
    the_tour_serves(&["--interp"]);
}

#[test]
fn the_tour_serves_as_wasm() {
    let jspi = Command::new(std::env::var("MODDIL_NODE").unwrap_or_else(|_| "node".to_string()))
        .args([
            "-e",
            "process.exit(typeof WebAssembly.Suspending === 'function' ? 0 : 1)",
        ])
        .status()
        .is_ok_and(|s| s.success());
    if jspi {
        the_tour_serves(&["--wasm"]);
    }
}

#[test]
fn fmt_check_passes_formatted_source_and_fails_the_rest() {
    let formatted = file(
        "fmt-ok.moddil",
        "fn add(a: int, b: int) -> int {\n    a + b\n}\n",
    );
    let out = moddil(&["fmt", "--check"], &formatted);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let messy = file("fmt-messy.moddil", "fn add(a:int,b:int)->int{a+b}\n");
    let out = moddil(&["fmt", "--check"], &messy);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stdout).contains("fmt-messy.moddil"));
    // --check rewrites nothing.
    assert_eq!(
        std::fs::read_to_string(&messy).unwrap(),
        "fn add(a:int,b:int)->int{a+b}\n"
    );
}

#[test]
fn fmt_rewrites_a_file_in_canonical_format() {
    let path = file("fmt-rewrite.moddil", "fn add(a:int,b:int)->int{a+b}\n");
    let out = moddil(&["fmt"], &path);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "fn add(a: int, b: int) -> int {\n    a + b\n}\n"
    );
    let again = moddil(&["fmt", "--check"], &path);
    assert!(again.status.success());
}

#[test]
fn fmt_refuses_a_named_file_that_does_not_parse() {
    let path = file("fmt-broken.moddil", "fn broken( {\n");
    let out = moddil(&["fmt"], &path);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("does not parse"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "fn broken( {\n");
}

/// Tasks whose values arrive in the order they run.
const ORDER: &str = "\
test \"in turn\" {
    let (tx, rx) = channel::<int>(3);
    scope(|s| {
        s.spawn(|| tx.send(1) ?! \"open\");
        s.spawn(|| tx.send(2) ?! \"open\");
        s.spawn(|| tx.send(3) ?! \"open\");
    });
    (rx.recv(), rx.recv(), rx.recv())  // => (Some(1), Some(2), Some(3))
}
";

#[test]
fn seeds_reorder_tasks_and_a_failure_names_its_seed() {
    let path = file("order.moddil", ORDER);
    for runner in [&[][..], &["--interp"][..]] {
        let args = [&["test"][..], runner].concat();
        assert!(moddil(&args, &path).status.success());
        let args = [&["test", "--seeds", "10"][..], runner].concat();
        let out = moddil(&args, &path);
        assert!(!out.status.success());
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("failed with seed 1 (`--seed 1`)"),
            "{stdout}"
        );
    }
}

#[test]
fn the_parallel_runtime_is_refused_until_it_is_built() {
    let out = moddil(
        &["run", "--runtime", "parallel"],
        &file("par.moddil", PROGRAM),
    );
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("not built yet"), "{stderr}");
}
