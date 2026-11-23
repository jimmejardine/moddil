//! Builds the runtime, `moddil-rt-wasm`, for `wasm32-unknown-unknown` (a target
//! `rust-toolchain.toml` pins), into this build's own directory, and gives its bytes to the
//! crate as `runtime.wasm` in `OUT_DIR`. It imports its memory, so the program shares it, and
//! exports `__heap_base`, where the program's own memory starts.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets it"));
    let root = manifest.join("../..");
    for dir in ["crates/rt", "crates/rt-wasm"] {
        println!(
            "cargo::rerun-if-changed={}",
            root.join(dir).join("src").display()
        );
        println!(
            "cargo::rerun-if-changed={}",
            root.join(dir).join("Cargo.toml").display()
        );
    }
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets it"));
    let target = out.join("rt-target");
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut cmd = Command::new(cargo);
    cmd.arg("build")
        .arg("--release")
        .arg("--offline")
        .arg("--locked")
        .arg("--quiet")
        .arg("-p")
        .arg("moddil-rt-wasm")
        .arg("--target")
        .arg("wasm32-unknown-unknown")
        .arg("--manifest-path")
        .arg(root.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(&target)
        // What the outer build sets for itself is not for this one.
        .env_remove("RUSTFLAGS")
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("CARGO_BUILD_TARGET")
        .env_remove("CARGO_MAKEFLAGS")
        .env_remove("MAKEFLAGS")
        .env_remove("MFLAGS")
        .env(
            "CARGO_ENCODED_RUSTFLAGS",
            [
                "-Clink-arg=--import-memory",
                "-Clink-arg=--export=__heap_base",
            ]
            .join("\u{1f}"),
        );
    let status = cmd
        .status()
        .expect("cargo, to build the runtime for wasm32");
    assert!(
        status.success(),
        "building moddil-rt-wasm for wasm32-unknown-unknown failed (is the target installed? \
         `rustup target add wasm32-unknown-unknown`)"
    );
    let wasm = target.join("wasm32-unknown-unknown/release/moddil_rt_wasm.wasm");
    std::fs::copy(&wasm, out.join("runtime.wasm")).expect("the runtime module just built");
}
