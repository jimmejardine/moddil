//! Every Moddil project with tests (each standard mod's `tests/` project, the examples and the
//! tour) passes them natively, on the MIR interpreter and as WASM: one test for each project and
//! runtime, so the runner spreads them over every core and a failure names both. Re-run one
//! project with `-E 'test(regex::)'`, or one runtime everywhere with `-E 'test(::wasm)'`.
//! `every_project_has_a_test` catches a project missing from the list.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Whether Node can run WASM tests: it has JSPI.
fn node_has_jspi() -> bool {
    Command::new(std::env::var("MODDIL_NODE").unwrap_or_else(|_| "node".to_string()))
        .args([
            "-e",
            "process.exit(typeof WebAssembly.Suspending === 'function' ? 0 : 1)",
        ])
        .status()
        .is_ok_and(|s| s.success())
}

/// Runs the tests of the project at `dir` (from the repository's root) the way `how` says (no
/// flag: natively): they all pass, at least one of them, and none falls back to the interpreter.
fn passes(dir: &str, how: Option<&str>) {
    if how == Some("--wasm") && !node_has_jspi() {
        eprintln!("skipped: no Node with JSPI");
        return;
    }
    if how == Some("--runtime=parallel") && ONE_TASK_AT_A_TIME.contains(&dir) {
        eprintln!("skipped: it simulates with `moddil/sim`, under the deterministic runtime only");
        return;
    }
    let out = Command::new(env!("CARGO_BIN_EXE_moddil"))
        .arg("test")
        .args(how)
        .arg(repo().join(dir))
        .output()
        .expect("moddil runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stdout}{stderr}");
    let passed = stdout
        .lines()
        .filter_map(|l| l.strip_suffix(" passed, 0 failed"))
        .find_map(|n| n.trim().parse::<usize>().ok());
    assert!(passed.is_some_and(|n| n > 0), "no tests ran: {stdout}");
    assert!(!stderr.contains("runs on the interpreter"), "{stderr}");
}

/// The projects that use `moddil/sim`, whose cells assume one task at a time: the parallel
/// runtime refuses them.
const ONE_TASK_AT_A_TIME: &[&str] = &["mods/sim/tests", "tour/shorty"];

/// For each project, a module of four tests: natively, natively under the parallel runtime, on
/// the interpreter and as WASM.
macro_rules! projects {
    ($($name:ident => $dir:literal),* $(,)?) => {
        const PROJECTS: &[&str] = &[$($dir),*];
        $(
            mod $name {
                #[test]
                fn native() {
                    super::passes($dir, None);
                }

                #[test]
                fn parallel() {
                    super::passes($dir, Some("--runtime=parallel"));
                }

                #[test]
                fn interp() {
                    super::passes($dir, Some("--interp"));
                }

                #[test]
                fn wasm() {
                    super::passes($dir, Some("--wasm"));
                }
            }
        )*
    };
}

/// Projects that call C (`ffi.extern.1`): natively and on the MIR interpreter, and refused as
/// WASM, which C does not run in yet.
macro_rules! c_projects {
    ($($name:ident => $dir:literal),* $(,)?) => {
        const C_PROJECTS: &[&str] = &[$($dir),*];
        $(
            mod $name {
                #[test]
                fn native() {
                    super::passes($dir, None);
                }

                #[test]
                fn interp() {
                    super::passes($dir, Some("--interp"));
                }

                #[test]
                fn wasm_is_refused() {
                    let out = std::process::Command::new(env!("CARGO_BIN_EXE_moddil"))
                        .args(["test", "--wasm"])
                        .arg(super::repo().join($dir))
                        .output()
                        .expect("moddil runs");
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    assert!(!out.status.success(), "{stderr}");
                    assert!(stderr.contains("runs only in native code"), "{stderr}");
                }
            }
        )*
    };
}

c_projects! {
    cabi => "examples/cabi",
    zlib => "examples/zlib",
}

projects! {
    alloc => "mods/alloc/tests",
    collections => "mods/collections/tests",
    crypto => "mods/crypto/tests",
    decimal => "mods/decimal/tests",
    gpu => "mods/gpu/tests",
    net => "mods/net/tests",
    num => "mods/num/tests",
    rand => "mods/rand/tests",
    regex => "mods/regex/tests",
    runtime => "mods/runtime/tests",
    serde => "mods/serde/tests",
    sim => "mods/sim/tests",
    sync => "mods/sync/tests",
    time => "mods/time/tests",
    mandelbulb => "examples/mandelbulb",
    mandelbulb_web => "examples/mandelbulb/web",
    python => "examples/python",
    tour => "tour/shorty",
}

/// The projects under `mods/`, `examples/` and `tour/` with tests of their own: a `moddil.toml`
/// and a `test` block in its `src/`.
fn found_projects() -> Vec<String> {
    fn has_test(dir: &Path) -> bool {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return false;
        };
        entries.flatten().any(|e| {
            let p = e.path();
            if p.is_dir() {
                has_test(&p)
            } else {
                p.extension().is_some_and(|x| x == "moddil")
                    && std::fs::read_to_string(&p)
                        .is_ok_and(|t| t.lines().any(|l| l.trim_start().starts_with("test \"")))
            }
        })
    }
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        if dir.join("moddil.toml").is_file() && has_test(&dir.join("src")) {
            let rel = dir.strip_prefix(root).expect("under the root");
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            let name = e.file_name();
            if p.is_dir() && name != "src" && name != "target" {
                walk(root, &p, out);
            }
        }
    }
    let root = repo();
    let mut out = Vec::new();
    for top in ["mods", "examples", "tour"] {
        walk(&root, &root.join(top), &mut out);
    }
    out.sort();
    out
}

#[test]
fn every_project_has_a_test() {
    let found = found_projects();
    let listed: Vec<String> = PROJECTS
        .iter()
        .chain(C_PROJECTS)
        .map(|p| p.to_string())
        .collect();
    let missing: Vec<&String> = found.iter().filter(|p| !listed.contains(p)).collect();
    let gone: Vec<&String> = listed.iter().filter(|p| !found.contains(p)).collect();
    assert!(
        missing.is_empty() && gone.is_empty(),
        "add {missing:?} to, and remove {gone:?} from, the `projects!` list in projects.rs"
    );
}
