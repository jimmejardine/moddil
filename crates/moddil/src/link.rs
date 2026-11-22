//! Linking a built program: its object file and `moddil-rt-main`, the runtime's static library,
//! into an executable, with the system's linker (`compiler.md`: Builds).

use std::path::{Path, PathBuf};
use std::process::Command;

/// The runtime library's file name on this platform.
fn library_name() -> &'static str {
    if cfg!(windows) {
        "moddil_rt_main.lib"
    } else {
        "libmoddil_rt_main.a"
    }
}

/// Where the runtime library is: where `MODDIL_RT_LIB` says, or beside the `moddil` executable
/// (where the workspace's build puts it).
fn library() -> Result<PathBuf, String> {
    if let Some(p) = std::env::var_os("MODDIL_RT_LIB") {
        return Ok(PathBuf::from(p));
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("the moddil executable's directory")?;
    let at = dir.join(library_name());
    if at.exists() {
        return Ok(at);
    }
    Err(format!(
        "the runtime library {} is not beside {} (build it with `cargo build -p moddil-rt-main`, \
         or say where it is with MODDIL_RT_LIB)",
        library_name(),
        exe.display()
    ))
}

/// Links `object` (the object file's bytes) with the runtime into executable `out`.
pub(crate) fn link(object: &[u8], out: &Path) -> Result<(), String> {
    let lib = library()?;
    let obj = out.with_extension(if cfg!(windows) { "obj" } else { "o" });
    std::fs::write(&obj, object).map_err(|e| format!("writing {}: {e}", obj.display()))?;
    let status = linker(&obj, &lib, out)?
        .status()
        .map_err(|e| format!("running the linker: {e}"));
    let _ = std::fs::remove_file(&obj);
    match status? {
        s if s.success() => Ok(()),
        s => Err(format!("the linker failed ({s})")),
    }
}

/// The linker's command for `obj` and `lib` into `out`: MSVC's `link.exe` on Windows, with the
/// system libraries Rust's standard library needs; the system's `cc` elsewhere.
fn linker(obj: &Path, lib: &Path, out: &Path) -> Result<Command, String> {
    if cfg!(windows) {
        let target = format!("{}-pc-windows-msvc", std::env::consts::ARCH);
        let tool = cc::windows_registry::find_tool(&target, "link.exe")
            .ok_or("MSVC's link.exe (install Visual Studio's C++ build tools)")?;
        let mut cmd = tool.to_command();
        cmd.arg("/NOLOGO")
            .arg(format!("/OUT:{}", out.display()))
            .arg("/SUBSYSTEM:CONSOLE")
            .arg(obj)
            .arg(lib);
        for l in [
            "kernel32.lib",
            "ntdll.lib",
            "userenv.lib",
            "ws2_32.lib",
            "dbghelp.lib",
            "synchronization.lib",
            "msvcrt.lib",
        ] {
            cmd.arg(l);
        }
        return Ok(cmd);
    }
    let mut cmd = Command::new(std::env::var_os("CC").unwrap_or_else(|| "cc".into()));
    cmd.arg(obj).arg(lib).arg("-o").arg(out);
    if cfg!(target_os = "macos") {
        cmd.args(["-lSystem", "-lc", "-lm"]);
    } else {
        cmd.args([
            "-lgcc_s",
            "-lutil",
            "-lrt",
            "-lpthread",
            "-lm",
            "-ldl",
            "-lc",
        ]);
    }
    Ok(cmd)
}
