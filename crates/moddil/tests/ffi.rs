//! C calling Moddil (`ffi.export.1`): `moddil build --lib examples/cabi` makes a shared library
//! (or, with `--static`, a static one) and its header, and `examples/cabi/c/main.c`, compiled by
//! the system's C compiler against them, calls every export and checks its result.

use std::path::{Path, PathBuf};
use std::process::Command;

mod runtime;

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
    c_calls_the_library(false);
}

#[test]
fn c_calls_the_exports_in_a_static_library() {
    c_calls_the_library(true);
}

/// Builds `examples/cabi` as a library (shared, or static), and `main.c` against it, and runs it.
fn c_calls_the_library(static_lib: bool) {
    runtime::ensure();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(if static_lib {
        "cabi-static"
    } else {
        "cabi"
    });
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a directory");
    let mut build = Command::new(env!("CARGO_BIN_EXE_moddil"));
    build.args(["build", "--lib"]);
    if static_lib {
        build.arg("--static");
    }
    let out = build
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
    // A static library's program links the system libraries it names too.
    let system: Vec<String> = if static_lib {
        std::fs::read_to_string(dir.join("cabi.link"))
            .expect("the link line")
            .split_whitespace()
            .map(str::to_string)
            .collect()
    } else {
        Vec::new()
    };
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
            .arg(dir.join("cabi.lib"))
            .args(&system);
    } else {
        c.arg(&main_c)
            .arg(format!("-I{}", dir.display()))
            .arg("-o")
            .arg(&exe);
        if static_lib {
            c.arg(dir.join("libcabi.a")).args(&system);
        } else {
            c.arg(format!("-L{}", dir.display()))
                .arg("-lcabi")
                .arg(format!("-Wl,-rpath,{}", dir.display()));
        }
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
    if static_lib {
        // Nothing to find at run time: the program holds the library.
        assert!(!dir.join("cabi.dll").exists() && !dir.join("libcabi.so").exists());
    }
}
