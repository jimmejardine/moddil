//! The runtime library built programs are linked with (`moddil-rt-main`), built before a test
//! builds a program, a C library or a Python module. `cargo test` builds what the tests depend
//! on, and the library is no dependency of `moddil` (it is beside it, `link.rs` finds it), so it
//! could be older than the `moddil` under test: its link would then fail on what the runtime
//! gained since.

use std::process::Command;

/// Builds `moddil-rt-main` once per test binary, in the profile `moddil` was built in.
pub(crate) fn ensure() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let mut cmd = Command::new(cargo);
        cmd.args(["build", "-q", "-p", "moddil-rt-main"])
            .current_dir(env!("CARGO_MANIFEST_DIR"));
        let exe = std::path::Path::new(env!("CARGO_BIN_EXE_moddil"));
        if exe.parent().and_then(|d| d.file_name()) == Some("release".as_ref()) {
            cmd.arg("--release");
        }
        let status = cmd.status().expect("cargo runs");
        assert!(status.success(), "building the runtime library: {status}");
    });
}
