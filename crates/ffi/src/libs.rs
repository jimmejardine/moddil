//! The C libraries a run has loaded, and symbol lookup in them and in the C runtime.

use std::path::Path;
use std::sync::Mutex;

use libloading::Library;

/// The libraries loaded so far, newest last. They stay loaded for the process's life: code may
/// hold pointers into them.
static LIBS: Mutex<Vec<Library>> = Mutex::new(Vec::new());

/// The C runtime's libraries, opened the first time a symbol is looked for.
static SYSTEM: Mutex<Option<Vec<Library>>> = Mutex::new(None);

/// Loads the shared library at `path`, so [`lookup`] finds its symbols.
///
/// # Errors
///
/// Why the library could not be loaded.
pub fn load(path: &Path) -> Result<(), String> {
    // SAFETY: a library's initialisers run as it loads; the libraries loaded are those the
    // program's own C sources make.
    let lib =
        unsafe { Library::new(path) }.map_err(|e| format!("loading {}: {e}", path.display()))?;
    LIBS.lock().map_err(|e| e.to_string())?.push(lib);
    Ok(())
}

/// The C runtime's libraries: the process itself on Unix (libc is in it); the CRT's DLLs on
/// Windows.
fn system() -> Vec<Library> {
    #[cfg(unix)]
    {
        vec![libloading::os::unix::Library::this().into()]
    }
    #[cfg(windows)]
    {
        let mut v: Vec<Library> = libloading::os::windows::Library::this()
            .map(Into::into)
            .into_iter()
            .collect();
        for name in [
            "ucrtbase.dll",
            "vcruntime140.dll",
            "msvcrt.dll",
            "kernel32.dll",
            "ntdll.dll",
        ] {
            // SAFETY: the system's own libraries.
            if let Ok(l) = unsafe { Library::new(name) } {
                v.push(l);
            }
        }
        v
    }
}

/// The address of C function `name`: in the libraries loaded (the newest first), then in the C
/// runtime's.
pub fn lookup(name: &str) -> Option<*const u8> {
    let find = |libs: &[Library]| {
        libs.iter().rev().find_map(|l| {
            // SAFETY: only the address is taken; it is called with the signature its `extern`
            // declaration gives.
            unsafe { l.get::<*const u8>(name.as_bytes()) }
                .ok()
                .map(|s| *s)
        })
    };
    if let Some(p) = LIBS.lock().ok().and_then(|l| find(&l)) {
        return Some(p);
    }
    let mut sys = SYSTEM.lock().ok()?;
    find(sys.get_or_insert_with(system))
}
