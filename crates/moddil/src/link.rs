//! Linking a built program: its object file and `moddil-rt-main`, the runtime's static library,
//! into an executable (`compiler.md`: Builds).
//!
//! - **A build** links with the system's linker: MSVC's `link.exe` on Windows, `cc` elsewhere.
//! - **A release build** links with `lld`, the one the Rust toolchain ships as `rust-lld`, or
//!   with `mold` where it is installed. On Linux, its line tables are then split off into
//!   `name.debug`, which the executable names (`.gnu_debuglink`); on macOS, `dsymutil` gathers
//!   them into `name.dSYM` where it is installed.

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
pub(crate) fn link(
    object: &[u8],
    out: &Path,
    release: bool,
    c: &[moddil_ffi::Built],
) -> Result<(), String> {
    let lib = library()?;
    let obj = out.with_extension(if cfg!(windows) { "obj" } else { "o" });
    std::fs::write(&obj, object).map_err(|e| format!("writing {}: {e}", obj.display()))?;
    let cmd = if release {
        release_linker(&obj, &lib, out)
    } else {
        linker(&obj, &lib, out)
    };
    // The mods' C code (`ffi.c.1`): its static libraries, and the system libraries it needs.
    let cmd = cmd.map(|mut cmd| {
        for b in c {
            cmd.arg(&b.static_lib);
            for l in &b.libs {
                cmd.arg(if cfg!(windows) {
                    format!("{l}.lib")
                } else {
                    format!("-l{l}")
                });
            }
        }
        cmd
    });
    let status = cmd.and_then(|mut c| c.status().map_err(|e| format!("running the linker: {e}")));
    let linked = match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("the linker failed ({s})")),
        Err(e) => Err(e),
    };
    let split = if release && linked.is_ok() {
        split_debug(out)
    } else {
        Ok(())
    };
    let _ = std::fs::remove_file(&obj);
    linked.and(split)
}

/// Links `object` (a library's, `moddil build --lib`) with the runtime and the mods' C code
/// into a shared library in `dir`, exporting `symbols`: `name.dll` (and its import library,
/// `name.lib`) on Windows, `libname.dylib` on macOS, `libname.so` elsewhere. Its path.
pub(crate) fn link_library(
    object: &[u8],
    dir: &Path,
    name: &str,
    symbols: &[String],
    c: &[moddil_ffi::Built],
) -> Result<PathBuf, String> {
    let lib = library()?;
    let obj = dir.join(format!(
        "{name}.{}",
        if cfg!(windows) { "obj" } else { "o" }
    ));
    std::fs::write(&obj, object).map_err(|e| format!("writing {}: {e}", obj.display()))?;
    let mut cmd;
    let out;
    if cfg!(windows) {
        out = dir.join(format!("{name}.dll"));
        let target = format!("{}-pc-windows-msvc", std::env::consts::ARCH);
        let tool = cc::windows_registry::find_tool(&target, "link.exe")
            .ok_or("MSVC's link.exe (install Visual Studio's C++ build tools)")?;
        cmd = tool.to_command();
        cmd.arg("/NOLOGO")
            .arg("/DLL")
            .arg(format!("/OUT:{}", out.display()))
            .arg(format!(
                "/IMPLIB:{}",
                dir.join(format!("{name}.lib")).display()
            ))
            .arg(&obj)
            .arg(&lib);
        for s in symbols {
            cmd.arg(format!("/EXPORT:{s}"));
        }
        for l in WINDOWS_LIBS {
            cmd.arg(l);
        }
    } else {
        let mac = cfg!(target_os = "macos");
        out = dir.join(if mac {
            format!("lib{name}.dylib")
        } else {
            format!("lib{name}.so")
        });
        cmd = Command::new(std::env::var_os("CC").unwrap_or_else(|| "cc".into()));
        cmd.arg(if mac { "-dynamiclib" } else { "-shared" })
            .arg(&obj)
            .arg(&lib)
            .arg("-o")
            .arg(&out);
        if mac {
            cmd.args(["-lSystem", "-lobjc", "-lc", "-lm"]);
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
    }
    for b in c {
        cmd.arg(&b.static_lib);
        for l in &b.libs {
            cmd.arg(if cfg!(windows) {
                format!("{l}.lib")
            } else {
                format!("-l{l}")
            });
        }
    }
    let o = cmd
        .output()
        .map_err(|e| format!("running the linker: {e}"))?;
    let _ = std::fs::remove_file(&obj);
    if !o.status.success() {
        return Err(format!(
            "the linker failed ({}):\n{}{}",
            o.status,
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        ));
    }
    Ok(out)
}

/// Links `object` (the Python shims' library) with the runtime, the C glue `glue` (a static
/// library), the mods' C code and Python's stable ABI into the extension module `out`, which
/// exports `PyInit_name` (`ffi.python.1`).
pub(crate) fn link_extension(
    object: &[u8],
    out: &Path,
    name: &str,
    glue: &Path,
    python_lib: Option<&Path>,
    c: &[moddil_ffi::Built],
) -> Result<(), String> {
    let lib = library()?;
    let dir = out.parent().ok_or("the module's directory")?;
    let obj = dir.join(format!(
        "{name}.{}",
        if cfg!(windows) { "obj" } else { "o" }
    ));
    std::fs::write(&obj, object).map_err(|e| format!("writing {}: {e}", obj.display()))?;
    let mut cmd;
    if cfg!(windows) {
        let target = format!("{}-pc-windows-msvc", std::env::consts::ARCH);
        let tool = cc::windows_registry::find_tool(&target, "link.exe")
            .ok_or("MSVC's link.exe (install Visual Studio's C++ build tools)")?;
        cmd = tool.to_command();
        cmd.arg("/NOLOGO")
            .arg("/DLL")
            .arg(format!("/OUT:{}", out.display()))
            .arg(format!(
                "/IMPLIB:{}",
                dir.join(format!("{name}-module.lib")).display()
            ))
            .arg(format!("/EXPORT:PyInit_{name}"))
            .arg(&obj)
            .arg(glue)
            .arg(&lib);
        if let Some(p) = python_lib {
            cmd.arg(p);
        }
        for l in WINDOWS_LIBS {
            cmd.arg(l);
        }
    } else {
        let mac = cfg!(target_os = "macos");
        cmd = Command::new(std::env::var_os("CC").unwrap_or_else(|| "cc".into()));
        cmd.arg(if mac { "-dynamiclib" } else { "-shared" });
        if mac {
            // Python's symbols are the interpreter's, found when it loads the module.
            cmd.args(["-undefined", "dynamic_lookup"]);
        }
        // The glue is a static library and nothing refers to its `PyInit_name`, so without
        // `-u` the linker would leave it out (Windows has `/EXPORT` for this).
        let init = format!("{}PyInit_{name}", if mac { "_" } else { "" });
        cmd.arg(format!("-Wl,-u,{init}"));
        cmd.arg(&obj).arg(glue).arg(&lib).arg("-o").arg(out);
        if mac {
            cmd.args(["-lSystem", "-lobjc", "-lc", "-lm"]);
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
    }
    for b in c {
        cmd.arg(&b.static_lib);
        for l in &b.libs {
            cmd.arg(if cfg!(windows) {
                format!("{l}.lib")
            } else {
                format!("-l{l}")
            });
        }
    }
    let o = cmd
        .output()
        .map_err(|e| format!("running the linker: {e}"))?;
    let _ = std::fs::remove_file(&obj);
    if !o.status.success() {
        return Err(format!(
            "the linker failed ({}):\n{}{}",
            o.status,
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        ));
    }
    Ok(())
}

/// Where the Rust toolchain's own tools are (`rust-lld`, `rust-objcopy`, the `gcc-ld` wrappers):
/// its sysroot's `lib/rustlib/<host>/bin`.
fn rust_tools() -> Result<PathBuf, String> {
    let out = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
        .args(["--print", "sysroot"])
        .output()
        .map_err(|e| {
            format!("rustc, to find rust-lld (release builds link with Rust's lld): {e}")
        })?;
    let sysroot = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let host = if cfg!(windows) {
        format!("{}-pc-windows-msvc", std::env::consts::ARCH)
    } else if cfg!(target_os = "macos") {
        format!("{}-apple-darwin", std::env::consts::ARCH)
    } else {
        format!("{}-unknown-linux-gnu", std::env::consts::ARCH)
    };
    let dir = PathBuf::from(sysroot)
        .join("lib/rustlib")
        .join(host)
        .join("bin");
    if dir.join(exe("rust-lld")).exists() {
        Ok(dir)
    } else {
        Err(format!("rust-lld is not in {}", dir.display()))
    }
}

/// `name` with this platform's executable extension.
fn exe(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

/// Whether `name` runs from the path.
fn on_path(name: &str) -> bool {
    Command::new(name)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// A release build's linker command: `rust-lld` as `lld-link` on Windows, with the MSVC
/// libraries' paths from `link.exe`'s environment; elsewhere `cc` with `mold` if it is on the
/// path, else with Rust's `lld` (through its `gcc-ld` wrappers).
fn release_linker(obj: &Path, lib: &Path, out: &Path) -> Result<Command, String> {
    if cfg!(windows) {
        let tools = rust_tools()?;
        let target = format!("{}-pc-windows-msvc", std::env::consts::ARCH);
        let msvc = cc::windows_registry::find_tool(&target, "link.exe")
            .ok_or("MSVC's libraries (install Visual Studio's C++ build tools)")?;
        let mut cmd = Command::new(tools.join(exe("rust-lld")));
        cmd.args(["-flavor", "link"]);
        for (k, v) in msvc.get_envs() {
            cmd.env(k, v);
        }
        cmd.arg("/NOLOGO")
            .arg(format!("/OUT:{}", out.display()))
            .arg("/SUBSYSTEM:CONSOLE")
            .arg("/OPT:REF,ICF")
            .arg(obj)
            .arg(lib);
        for l in WINDOWS_LIBS {
            cmd.arg(l);
        }
        return Ok(cmd);
    }
    let mut cmd = linker(obj, lib, out)?;
    if on_path("mold") {
        cmd.arg("-fuse-ld=mold");
    } else {
        let tools = rust_tools()?;
        cmd.arg("-B").arg(tools.join("gcc-ld")).arg("-fuse-ld=lld");
    }
    Ok(cmd)
}

/// Splits a release build's line tables off executable `out`, where the platform has a place
/// for them: on Linux into `name.debug` (`rust-objcopy`, or the system's `objcopy`), which the
/// executable then names; on macOS into `name.dSYM`, if `dsymutil` is installed (it reads them
/// from the object, which is removed after). Windows has none yet.
fn split_debug(out: &Path) -> Result<(), String> {
    if cfg!(target_os = "macos") {
        if on_path("dsymutil") {
            let _ = Command::new("dsymutil").arg(out).status();
        }
        return Ok(());
    }
    if cfg!(windows) {
        return Ok(());
    }
    let objcopy = rust_tools()
        .map(|d| d.join(exe("rust-objcopy")))
        .ok()
        .filter(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from("objcopy"));
    let debug = out.with_extension("debug");
    let run = |args: &[&std::ffi::OsStr]| -> Result<(), String> {
        let s = Command::new(&objcopy)
            .args(args)
            .status()
            .map_err(|e| format!("running {}: {e}", objcopy.display()))?;
        if s.success() {
            Ok(())
        } else {
            Err(format!("{} failed ({s})", objcopy.display()))
        }
    };
    run(&[
        "--only-keep-debug".as_ref(),
        out.as_os_str(),
        debug.as_os_str(),
    ])?;
    let link = format!("--add-gnu-debuglink={}", debug.display());
    run(&["--strip-debug".as_ref(), link.as_ref(), out.as_os_str()])
}

/// The system libraries a Windows program needs beside the runtime (Rust's standard library's).
const WINDOWS_LIBS: [&str; 7] = [
    "kernel32.lib",
    "ntdll.lib",
    "userenv.lib",
    "ws2_32.lib",
    "dbghelp.lib",
    "synchronization.lib",
    "msvcrt.lib",
];

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
        for l in WINDOWS_LIBS {
            cmd.arg(l);
        }
        return Ok(cmd);
    }
    let mut cmd = Command::new(std::env::var_os("CC").unwrap_or_else(|| "cc".into()));
    cmd.arg(obj).arg(lib).arg("-o").arg(out);
    if cfg!(target_os = "macos") {
        // `-lobjc`: the runtime's signal handling (`ctrlc`, through `dispatch2` and `objc2`)
        // uses Apple's Objective-C runtime; libdispatch is part of libSystem. These are the
        // libraries `rustc --print native-static-libs` lists for `moddil-rt-main` there.
        cmd.args(["-lSystem", "-lobjc", "-lc", "-lm"]);
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
