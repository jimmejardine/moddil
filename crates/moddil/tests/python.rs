//! `moddil build --wheel` (`ffi.python.1`): the example `examples/python` built as a wheel,
//! installed with pip into a directory of its own, and checked from Python by
//! `examples/python/py/check.py`. Skipped where there is no Python 3.11 or later, but not on CI
//! (`CI` set, as GitHub sets it), where Python and numpy must be there.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A Python 3.11 or later: `MODDIL_PYTHON`, `python3` or `python`.
fn python() -> Option<String> {
    let candidates = std::env::var("MODDIL_PYTHON")
        .ok()
        .into_iter()
        .chain(["python3".to_string(), "python".to_string()]);
    for exe in candidates {
        let ok = Command::new(&exe)
            .args([
                "-c",
                "import sys; sys.exit(0 if sys.version_info >= (3, 11) else 1)",
            ])
            .status()
            .is_ok_and(|s| s.success());
        if ok {
            return Some(exe);
        }
    }
    None
}

#[test]
fn the_example_builds_installs_and_runs_as_a_python_module() {
    let ci = std::env::var_os("CI").is_some();
    let Some(py) = python() else {
        assert!(!ci, "CI has Python 3.11 or later (actions/setup-python)");
        eprintln!("skipped: no Python 3.11 or later");
        return;
    };
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("python");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a directory");
    let out = Command::new(env!("CARGO_BIN_EXE_moddil"))
        .args(["build", "--wheel"])
        .arg(repo.join("examples/python"))
        .arg("-o")
        .arg(&dir)
        .env("MODDIL_PYTHON", &py)
        .output()
        .expect("moddil runs");
    let stderr = String::from_utf8_lossy(&out.stderr);
    if stderr.contains("the runtime library") {
        eprintln!("skipped: {stderr}");
        return;
    }
    assert!(out.status.success(), "{stderr}");
    let wheel = std::fs::read_dir(&dir)
        .expect("the directory")
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "whl"))
        .expect("a wheel");
    assert!(
        wheel.to_string_lossy().contains("stats-0.1.0-cp311-abi3-"),
        "{}",
        wheel.display()
    );
    let site = dir.join("site");
    let pip = Command::new(&py)
        .args([
            "-m",
            "pip",
            "install",
            "--quiet",
            "--no-deps",
            "--no-index",
            "--target",
        ])
        .arg(&site)
        .arg(&wheel)
        .output()
        .expect("pip runs");
    if !ci && String::from_utf8_lossy(&pip.stderr).contains("No module named pip") {
        eprintln!("skipped: no pip");
        return;
    }
    assert!(
        pip.status.success(),
        "{}",
        String::from_utf8_lossy(&pip.stderr)
    );
    assert!(site.join("stats.pyi").is_file(), "the stubs are installed");
    let run = Command::new(&py)
        .arg(repo.join("examples/python/py/check.py"))
        .env("PYTHONPATH", &site)
        .output()
        .expect("python runs");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        run.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(stdout.contains("every check passed"), "{stdout}");
}
