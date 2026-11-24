//! A mod's C sources, compiled by the system's C compiler (the `cc` crate) into a static library
//! and a shared one, cached under a directory keyed by everything that went into them.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A mod's C code, as its manifest's `[c]` table gives it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CSpec {
    /// The source files.
    pub sources: Vec<PathBuf>,
    /// The directories headers are found in.
    pub include: Vec<PathBuf>,
    /// The preprocessor definitions: a name, and its value if it has one.
    pub defines: Vec<(String, Option<String>)>,
    /// The system libraries the code needs, linked by name (`m`, `pthread`).
    pub libs: Vec<String>,
    /// Whether to make the shared library too (the JIT's and the interpreter's), not only the
    /// static one.
    pub shared: bool,
}

/// What [`build`] makes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Built {
    /// The static library, for `moddil build` to link.
    pub static_lib: PathBuf,
    /// The shared library, for the JIT and the MIR interpreter to load.
    pub shared_lib: PathBuf,
    /// The system libraries to link with it.
    pub libs: Vec<String>,
}

const TARGET: &str = env!("MODDIL_FFI_TARGET");

/// FNV-1a, 64 bits: stable across runs and Rust versions, for the cache's key.
fn fnv(h: &mut u64, bytes: &[u8]) {
    for b in bytes {
        *h ^= u64::from(*b);
        *h = h.wrapping_mul(0x0100_0000_01b3);
    }
}

/// Builds `spec` as the library `name` under `out` (a cache: built again only when the sources,
/// the flags or the exported symbols change). `exports` are the symbols the shared library must
/// export (Windows exports only those named).
///
/// # Errors
///
/// A source that cannot be read, or what the C compiler or the linker reported.
pub fn build(spec: &CSpec, name: &str, out: &Path, exports: &[String]) -> Result<Built, String> {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    // The cache's own version: a change to how libraries are made rebuilds them.
    fnv(&mut h, b"moddil-ffi 2");
    fnv(&mut h, TARGET.as_bytes());
    for s in &spec.sources {
        let text = std::fs::read(s).map_err(|e| format!("reading {}: {e}", s.display()))?;
        fnv(&mut h, s.to_string_lossy().as_bytes());
        fnv(&mut h, &text);
    }
    for i in &spec.include {
        fnv(&mut h, i.to_string_lossy().as_bytes());
        // A header's change rebuilds too.
        if let Ok(rd) = std::fs::read_dir(i) {
            let mut hs: Vec<PathBuf> = rd
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "h"))
                .collect();
            hs.sort();
            for p in hs {
                fnv(&mut h, &std::fs::read(&p).unwrap_or_default());
            }
        }
    }
    for (k, v) in &spec.defines {
        fnv(&mut h, k.as_bytes());
        fnv(&mut h, v.as_deref().unwrap_or("").as_bytes());
    }
    for e in exports {
        fnv(&mut h, e.as_bytes());
    }
    fnv(&mut h, &[u8::from(spec.shared)]);
    let done = out.join(format!("{name}-{h:016x}"));
    // Built in a directory of this process's own, then moved into place: two runs building the
    // same code at once (tests do) do not write over each other.
    let dir = out.join(format!("{name}-{h:016x}.{}", std::process::id()));
    let windows = TARGET.contains("windows");
    let (static_name, shared_name) = if windows {
        (format!("{name}.lib"), format!("{name}.dll"))
    } else if TARGET.contains("apple") {
        (format!("lib{name}.a"), format!("lib{name}.dylib"))
    } else {
        (format!("lib{name}.a"), format!("lib{name}.so"))
    };
    let built = Built {
        static_lib: done.join(&static_name),
        shared_lib: done.join(&shared_name),
        libs: spec.libs.clone(),
    };
    if built.static_lib.exists() && (built.shared_lib.exists() || !spec.shared) {
        return Ok(built);
    }
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let mut cc = cc::Build::new();
    cc.target(TARGET)
        .host(TARGET)
        .opt_level(2)
        .debug(false)
        .out_dir(&dir)
        .cargo_metadata(false)
        .warnings(false)
        .pic(true)
        .files(&spec.sources)
        .includes(&spec.include);
    for (k, v) in &spec.defines {
        cc.define(k, v.as_deref());
    }
    let objects = cc
        .try_compile_intermediates()
        .map_err(|e| format!("compiling the C sources: {e}"))?;
    let tool = cc.try_get_compiler().map_err(|e| e.to_string())?;
    let run = |mut c: Command, what: &str| -> Result<(), String> {
        let o = c.output().map_err(|e| format!("{what}: {e}"))?;
        if o.status.success() {
            Ok(())
        } else {
            Err(format!(
                "{what} failed:\n{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            ))
        }
    };
    if windows {
        let lib = cc::windows_registry::find_tool(TARGET, "lib.exe")
            .ok_or("MSVC's lib.exe (install Visual Studio's C++ build tools)")?;
        let mut c = lib.to_command();
        c.arg("/NOLOGO")
            .arg(format!("/OUT:{}", dir.join(&static_name).display()))
            .args(&objects);
        run(c, "lib.exe")?;
    } else {
        let mut c = Command::new("ar");
        c.arg("crs").arg(dir.join(&static_name)).args(&objects);
        run(c, "ar")?;
    }
    if spec.shared && windows {
        let link = cc::windows_registry::find_tool(TARGET, "link.exe")
            .ok_or("MSVC's link.exe (install Visual Studio's C++ build tools)")?;
        let mut c = link.to_command();
        c.arg("/NOLOGO")
            .arg("/DLL")
            .arg(format!("/OUT:{}", dir.join(&shared_name).display()))
            // Its import library apart from the static one, which has the same name otherwise.
            .arg(format!(
                "/IMPLIB:{}",
                dir.join(format!("{name}-dll.lib")).display()
            ))
            .args(&objects);
        for e in exports {
            c.arg(format!("/EXPORT:{e}"));
        }
        for l in &spec.libs {
            c.arg(format!("{l}.lib"));
        }
        run(c, "link.exe")?;
    } else if spec.shared {
        let mut c = tool.to_command();
        if TARGET.contains("apple") {
            c.args(["-dynamiclib", "-undefined", "dynamic_lookup"]);
        } else {
            c.arg("-shared");
        }
        c.args(&objects).arg("-o").arg(dir.join(&shared_name));
        for l in &spec.libs {
            c.arg(format!("-l{l}"));
        }
        run(c, "linking the shared library")?;
    }
    // Into place; if another run got there first, its copy is the same, and is kept.
    if std::fs::rename(&dir, &done).is_err() {
        let _ = std::fs::remove_dir_all(&dir);
        if !(built.static_lib.exists() && (built.shared_lib.exists() || !spec.shared)) {
            return Err(format!("moving the built C code to {}", done.display()));
        }
    }
    Ok(built)
}
