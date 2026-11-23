//! Positions and paths as LSP has them: lines and UTF-16 columns, and `file://` URIs.

use std::path::{Path, PathBuf};

/// Where each line of a text starts, to turn byte offsets into LSP positions and back.
pub struct LineIndex<'t> {
    text: &'t str,
    starts: Vec<u32>,
}

impl<'t> LineIndex<'t> {
    /// The index of `text`.
    pub fn new(text: &'t str) -> Self {
        let mut starts = vec![0];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                starts.push(i as u32 + 1);
            }
        }
        LineIndex { text, starts }
    }

    /// Byte `offset` as a line and a UTF-16 column, both from 0.
    pub fn position(&self, offset: u32) -> (u32, u32) {
        let offset = offset.min(self.text.len() as u32);
        let line = self.starts.partition_point(|&s| s <= offset) - 1;
        let start = self.starts[line] as usize;
        let col: usize = self.text[start..offset as usize]
            .chars()
            .map(char::len_utf16)
            .sum();
        (line as u32, col as u32)
    }

    /// The byte offset of `line` and UTF-16 column `col`, clamped to the line's end.
    pub fn offset(&self, line: u32, col: u32) -> u32 {
        let Some(&start) = self.starts.get(line as usize) else {
            return self.text.len() as u32;
        };
        let rest = &self.text[start as usize..];
        let mut units = 0;
        for (i, c) in rest.char_indices() {
            if units >= col as usize || c == '\n' {
                return start + i as u32;
            }
            units += c.len_utf16();
        }
        self.text.len() as u32
    }
}

/// The path a `file://` URI names, percent-decoded (`file:///d%3A/x.moddil` is `d:/x.moddil`).
pub fn uri_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        if bytes[i] == b'%'
            && let Some(b) = hex
        {
            out.push(b);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    let path = String::from_utf8(out).ok()?;
    // `/d:/x` on Windows is the drive's path.
    let path = match path.as_bytes() {
        [b'/', d, b':', ..] if cfg!(windows) && d.is_ascii_alphabetic() => &path[1..],
        _ => path.as_str(),
    };
    Some(PathBuf::from(path))
}

/// The `file://` URI of `path`.
pub fn path_uri(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    // A canonical Windows path has the verbatim prefix.
    let s = s.strip_prefix("//?/").unwrap_or(&s);
    let mut out = String::from("file://");
    if !s.starts_with('/') {
        out.push('/');
    }
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            out.push(b as char);
        } else if b == b':' && out.len() <= "file:///x".len() {
            // A drive letter's colon, as editors write it.
            out.push_str("%3A");
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_count_utf16_units() {
        let t = "ab\nλ😀x\n";
        let ix = LineIndex::new(t);
        // `x` is after `λ` (1 unit, 2 bytes) and `😀` (2 units, 4 bytes).
        assert_eq!(ix.position(9), (1, 3));
        assert_eq!(ix.offset(1, 3), 9);
        assert_eq!(ix.offset(0, 99), 2);
        assert_eq!(ix.position(0), (0, 0));
    }

    #[test]
    fn uris_and_paths_round_trip() {
        let p = uri_path("file:///tmp/a%20b.moddil").expect("a path");
        assert!(p.to_string_lossy().ends_with("a b.moddil"));
        let u = path_uri(Path::new("/tmp/a b.moddil"));
        assert_eq!(u, "file:///tmp/a%20b.moddil");
    }
}
