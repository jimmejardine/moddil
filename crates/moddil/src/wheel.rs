//! `moddil build --wheel` (`ffi.python.1`): finding Python, and writing a wheel.
//!
//! A wheel is a zip of the extension module, its stubs and its `dist-info` (`METADATA`, `WHEEL`
//! and `RECORD`, which lists each file with its SHA-256). Its files are stored, not compressed:
//! a wheel may be either, and stored needs no compressor.

use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

/// The Python the module is built for.
pub(crate) struct Python {
    /// Its headers' directory.
    pub(crate) include: PathBuf,
    /// The stable ABI's import library, on Windows (`python3.lib`).
    pub(crate) lib: Option<PathBuf>,
    /// The executable that answered.
    pub(crate) exe: String,
}

/// The first Python 3.11 or later among `MODDIL_PYTHON`, `python3` and `python`.
pub(crate) fn find_python() -> Result<Python, String> {
    let mut tried = Vec::new();
    let candidates: Vec<String> = std::env::var("MODDIL_PYTHON")
        .ok()
        .into_iter()
        .chain(["python3".to_string(), "python".to_string()])
        .collect();
    for exe in candidates {
        let out = Command::new(&exe)
            .args([
                "-c",
                "import sys, sysconfig; print(sysconfig.get_paths()['include']); \
                 print(sys.base_prefix); print(sys.version_info[0] * 100 + sys.version_info[1])",
            ])
            .output();
        let Ok(out) = out else {
            tried.push(exe);
            continue;
        };
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        let lines: Vec<&str> = text.lines().map(str::trim).collect();
        let [include, prefix, version] = lines[..] else {
            tried.push(exe);
            continue;
        };
        if version.parse::<u32>().unwrap_or(0) < 311 {
            tried.push(format!("{exe} (older than 3.11)"));
            continue;
        }
        let lib = cfg!(windows).then(|| Path::new(prefix).join("libs").join("python3.lib"));
        return Ok(Python {
            include: PathBuf::from(include),
            lib,
            exe,
        });
    }
    Err(format!(
        "no Python 3.11 or later (tried {}; say which with MODDIL_PYTHON)",
        tried.join(", ")
    ))
}

/// The platform tag of a wheel built here.
pub(crate) fn platform() -> String {
    let arch = std::env::consts::ARCH;
    if cfg!(windows) {
        if arch == "x86_64" {
            "win_amd64".to_string()
        } else {
            format!("win_{arch}")
        }
    } else if cfg!(target_os = "macos") {
        // macOS's tags say `arm64` where Rust says `aarch64`.
        let arch = if arch == "aarch64" { "arm64" } else { arch };
        format!("macosx_11_0_{arch}")
    } else {
        format!("linux_{arch}")
    }
}

/// CRC-32 (IEEE), as zip uses it.
fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in data {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 == 1 {
                (c >> 1) ^ 0xEDB8_8320
            } else {
                c >> 1
            };
        }
    }
    !c
}

/// A zip archive of `files` (each a name and its bytes), stored.
fn zip(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in files {
        let offset = out.len() as u32;
        let crc = crc32(data);
        let size = data.len() as u32;
        let n = name.len() as u16;
        let header = |sig: u32, central: bool| {
            let mut h = Vec::new();
            h.extend(sig.to_le_bytes());
            if central {
                h.extend(20u16.to_le_bytes()); // made by
            }
            h.extend(20u16.to_le_bytes()); // needed
            h.extend(0u16.to_le_bytes()); // flags
            h.extend(0u16.to_le_bytes()); // stored
            h.extend(0u16.to_le_bytes()); // time
            h.extend(0x21u16.to_le_bytes()); // date: 1980-01-01
            h.extend(crc.to_le_bytes());
            h.extend(size.to_le_bytes());
            h.extend(size.to_le_bytes());
            h.extend(n.to_le_bytes());
            h.extend(0u16.to_le_bytes()); // extra
            if central {
                h.extend(0u16.to_le_bytes()); // comment
                h.extend(0u16.to_le_bytes()); // disk
                h.extend(0u16.to_le_bytes()); // internal attributes
                h.extend(0u32.to_le_bytes()); // external attributes
                h.extend(offset.to_le_bytes());
            }
            h.extend(name.as_bytes());
            h
        };
        out.extend(header(0x0403_4b50, false));
        out.extend(data);
        central.extend(header(0x0201_4b50, true));
    }
    let at = out.len() as u32;
    let count = files.len() as u16;
    let size = central.len() as u32;
    out.extend(central);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0u8; 4]); // disks
    out.extend(count.to_le_bytes());
    out.extend(count.to_le_bytes());
    out.extend(size.to_le_bytes());
    out.extend(at.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out
}

/// URL-safe base64 without padding, as `RECORD`'s hashes are written.
fn base64_url(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        for k in 0..=c.len() {
            out.push(A[((n >> (18 - 6 * k)) & 63) as usize] as char);
        }
    }
    out
}

/// Writes the wheel of module `name` (version `version`) into `dir`: the extension module file
/// `module` (its file name in the wheel: `module_name`) and the stubs. Its path.
pub(crate) fn write(
    dir: &Path,
    name: &str,
    version: &str,
    module: &Path,
    module_name: &str,
    pyi: &str,
) -> Result<PathBuf, String> {
    let tag = format!("cp311-abi3-{}", platform());
    let info = format!("{name}-{version}.dist-info");
    let module_bytes =
        std::fs::read(module).map_err(|e| format!("reading {}: {e}", module.display()))?;
    let mut files: Vec<(String, Vec<u8>)> = vec![
        (module_name.to_string(), module_bytes),
        (format!("{name}.pyi"), pyi.as_bytes().to_vec()),
        (
            format!("{info}/METADATA"),
            format!(
                "Metadata-Version: 2.1\nName: {name}\nVersion: {version}\nRequires-Python: >=3.11\n"
            )
            .into_bytes(),
        ),
        (
            format!("{info}/WHEEL"),
            format!("Wheel-Version: 1.0\nGenerator: moddil\nRoot-Is-Purelib: false\nTag: {tag}\n")
                .into_bytes(),
        ),
    ];
    let mut record = String::new();
    for (f, data) in &files {
        let hash = base64_url(&Sha256::digest(data));
        record.push_str(&format!("{f},sha256={hash},{}\n", data.len()));
    }
    record.push_str(&format!("{info}/RECORD,,\n"));
    files.push((format!("{info}/RECORD"), record.into_bytes()));
    let path = dir.join(format!("{name}-{version}-{tag}.whl"));
    std::fs::write(&path, zip(&files)).map_err(|e| format!("writing {}: {e}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksums_and_encodings_are_the_standard_ones() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(base64_url(b"\xfb\xff"), "-_8");
        assert_eq!(base64_url(b"abc"), "YWJj");
    }
}
