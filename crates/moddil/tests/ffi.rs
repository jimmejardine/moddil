//! C calling Moddil (`ffi.export.1`): `moddil build --lib examples/cabi` makes a shared library
//! and its header, and `examples/cabi/c/main.c`, compiled by the system's C compiler against
//! them, calls every export and checks its result.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The target triple of the C compiler, as the `cc` crate names it.
fn target() -> String {
    let arch = std::env::consts::ARCH;
    if cfg!(windows) {
        format!("{arch}-pc-windows-msvc")
    } else if cfg!(target_os = "macos") {
        format!("{arch}-apple-darwin")
    } else {
        format!("{arch}-unknown-linux-gnu")
    }
}

#[test]
fn c_calls_the_exports_through_the_generated_header() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("cabi");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a directory");
    let out = Command::new(env!("CARGO_BIN_EXE_moddil"))
        .args(["build", "--lib"])
        .arg(repo.join("examples/cabi"))
        .arg("-o")
        .arg(&dir)
        .output()
        .expect("moddil runs");
    let stderr = String::from_utf8_lossy(&out.stderr);
    if stderr.contains("the runtime library") {
        eprintln!("skipped: {stderr}");
        return;
    }
    assert!(out.status.success(), "{stderr}");
    let header = std::fs::read_to_string(dir.join("cabi.h")).expect("the header");
    assert!(
        header.contains("double moddil_distance(const Point *a, const Point *b);"),
        "{header}"
    );
    // The C program, compiled and linked against the library.
    let t = target();
    let cc = cc::Build::new()
        .target(&t)
        .host(&t)
        .opt_level(0)
        .cargo_metadata(false)
        .try_get_compiler()
        .expect("a C compiler");
    let main_c = repo.join("examples/cabi/c/main.c");
    let exe = dir.join(if cfg!(windows) { "main.exe" } else { "main" });
    let mut c = cc.to_command();
    if cfg!(windows) {
        c.arg(&main_c)
            .arg(format!("/I{}", dir.display()))
            .arg(format!("/Fe{}", exe.display()))
            .arg(format!("/Fo{}", dir.join("main.obj").display()))
            .arg("/link")
            .arg(dir.join("cabi.lib"));
    } else {
        c.arg(&main_c)
            .arg(format!("-I{}", dir.display()))
            .arg(format!("-L{}", dir.display()))
            .arg("-lcabi")
            .arg(format!("-Wl,-rpath,{}", dir.display()))
            .arg("-o")
            .arg(&exe);
    }
    let built = c.output().expect("the C compiler runs");
    assert!(
        built.status.success(),
        "{}{}",
        String::from_utf8_lossy(&built.stdout),
        String::from_utf8_lossy(&built.stderr)
    );
    // Beside its library, which Windows finds there.
    let run = Command::new(&exe)
        .current_dir(&dir)
        .output()
        .expect("the program runs");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert_eq!(run.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("every result is right"), "{stdout}");
}
