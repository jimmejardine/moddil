//! The Moddil sources every implementation reads, as text: the prelude
//! (`reference/2027/prelude/`) and the built-in `core` mod's modules (`reference/2027/core/`).
//!
//! The compiler (`moddil-sema`) and the reference interpreter (`moddil-refinterp`) both take
//! them from here, so a file added to either directory is listed once. This crate holds data
//! only: the reference interpreter stays independent of the compiler's code.

/// The prelude's files, in the order they are joined: each one's path under `reference/2027/`,
/// and its file.
macro_rules! prelude_files {
    ($m:ident) => {
        $m!(
            "prelude/core.moddil" => "../../../reference/2027/prelude/core.moddil",
            "prelude/int.moddil" => "../../../reference/2027/prelude/int.moddil",
            "prelude/numbers.moddil" => "../../../reference/2027/prelude/numbers.moddil",
            "prelude/convert.moddil" => "../../../reference/2027/prelude/convert.moddil",
            "prelude/float.moddil" => "../../../reference/2027/prelude/float.moddil",
            "prelude/text.moddil" => "../../../reference/2027/prelude/text.moddil",
            "prelude/fmt.moddil" => "../../../reference/2027/prelude/fmt.moddil",
            "prelude/arrays.moddil" => "../../../reference/2027/prelude/arrays.moddil",
            "prelude/iter.moddil" => "../../../reference/2027/prelude/iter.moddil",
            "prelude/pointers.moddil" => "../../../reference/2027/prelude/pointers.moddil",
            "prelude/collections.moddil" => "../../../reference/2027/prelude/collections.moddil",
            "prelude/hash.moddil" => "../../../reference/2027/prelude/hash.moddil",
            "prelude/program.moddil" => "../../../reference/2027/prelude/program.moddil",
            "prelude/concurrency.moddil" => "../../../reference/2027/prelude/concurrency.moddil",
            "prelude/derives.moddil" => "../../../reference/2027/prelude/derives.moddil",
            "prelude/alloc.moddil" => "../../../reference/2027/prelude/alloc.moddil",
            "prelude/units.moddil" => "../../../reference/2027/prelude/units.moddil",
        )
    };
}

/// The `core` mod's modules: each one's name, its path under `reference/2027/`, and its file.
macro_rules! core_modules {
    ($m:ident) => {
        $m!(
            "collections", "core/collections.moddil" => "../../../reference/2027/core/collections.moddil",
            "ptr", "core/ptr.moddil" => "../../../reference/2027/core/ptr.moddil",
            "cell", "core/cell.moddil" => "../../../reference/2027/core/cell.moddil",
            "atomic", "core/atomic.moddil" => "../../../reference/2027/core/atomic.moddil",
            "os", "core/os.moddil" => "../../../reference/2027/core/os.moddil",
            "gpu", "core/gpu.moddil" => "../../../reference/2027/core/gpu.moddil",
            "ffi", "core/ffi.moddil" => "../../../reference/2027/core/ffi.moddil",
            "text", "core/text.moddil" => "../../../reference/2027/core/text.moddil",
        )
    };
}

macro_rules! paths_and_texts {
    ($($path:literal => $file:literal),* $(,)?) => {
        [$(($path, include_str!($file))),*]
    };
}

macro_rules! joined {
    ($($path:literal => $file:literal),* $(,)?) => {
        concat!($(include_str!($file)),*)
    };
}

macro_rules! names_and_texts {
    ($($name:literal, $path:literal => $file:literal),* $(,)?) => {
        [$(($name, include_str!($file))),*]
    };
}

macro_rules! core_paths_and_texts {
    ($($name:literal, $path:literal => $file:literal),* $(,)?) => {
        [$(($path, include_str!($file))),*]
    };
}

macro_rules! prelude_and_core {
    ($($name:literal, $path:literal => $file:literal),* $(,)?) => {
        concat!(prelude_files!(joined), $(include_str!($file)),*)
    };
}

/// The prelude's source: the files of `reference/2027/prelude/`, joined.
pub const PRELUDE: &str = prelude_files!(joined);

/// The prelude's files, in [`PRELUDE`]'s order: each one's path under `reference/2027/`
/// (`prelude/core.moddil`), and its text.
pub const PRELUDE_FILES: [(&str, &str); 17] = prelude_files!(paths_and_texts);

/// The `core` mod's modules, in order: each one's name and source.
pub const CORE_MODULES: [(&str, &str); 8] = core_modules!(names_and_texts);

/// The `core` mod's files, in [`CORE_MODULES`]' order: each one's path under `reference/2027/`
/// (`core/ptr.moddil`), and its text.
pub const CORE_FILES: [(&str, &str); 8] = core_modules!(core_paths_and_texts);

/// The prelude's source followed by the `core` mod's modules, in [`CORE_MODULES`]' order: the
/// compiler's prelude text, in which the `core` items start at [`PRELUDE`]'s length.
pub const PRELUDE_AND_CORE: &str = core_modules!(prelude_and_core);

/// The file byte `offset` of [`PRELUDE_AND_CORE`] is in: its path under `reference/2027/`, its
/// text, and the offset within it.
pub fn file_at(offset: usize) -> Option<(&'static str, &'static str, usize)> {
    let mut start = 0;
    for (path, text) in PRELUDE_FILES.iter().chain(&CORE_FILES) {
        if offset < start + text.len() {
            return Some((path, text, offset - start));
        }
        start += text.len();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `.moddil` files of `reference/2027/<dir>`, each one's text.
    fn files(dir: &str) -> Vec<(String, String)> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../reference/2027")
            .join(dir);
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&dir).expect("the directory") {
            let path = entry.expect("an entry").path();
            if path.extension().is_some_and(|e| e == "moddil") {
                let text = std::fs::read_to_string(&path).expect("a source file");
                out.push((path.display().to_string(), text));
            }
        }
        out
    }

    /// A file added to `reference/2027/prelude/` is not left out of [`PRELUDE`].
    #[test]
    fn the_prelude_is_every_file_of_its_directory() {
        let mut total = 0;
        for (path, text) in files("prelude") {
            assert!(PRELUDE.contains(&text), "{path} is not in the prelude");
            total += text.len();
        }
        assert_eq!(
            total,
            PRELUDE.len(),
            "the prelude is not its files, each once"
        );
    }

    /// Each file's bytes are found in it, at their offset within it.
    #[test]
    fn an_offset_is_found_in_its_file() {
        let mut start = 0;
        for (path, text) in PRELUDE_FILES.iter().chain(&CORE_FILES) {
            assert_eq!(file_at(start), Some((*path, *text, 0)));
            let last = start + text.len() - 1;
            let (p, t, at) = file_at(last).expect("in a file");
            assert_eq!((p, at), (*path, text.len() - 1));
            assert_eq!(&PRELUDE_AND_CORE[start..=last], t);
            start += text.len();
        }
        assert_eq!(start, PRELUDE_AND_CORE.len());
        assert_eq!(file_at(start), None);
    }

    /// A file added to `reference/2027/core/` is not left out of [`CORE_MODULES`].
    #[test]
    fn the_core_modules_are_every_file_of_their_directory() {
        let found = files("core");
        assert_eq!(
            found.len(),
            CORE_MODULES.len(),
            "a `core` file is not listed"
        );
        for (path, text) in found {
            assert!(
                CORE_MODULES.iter().any(|(_, t)| *t == text),
                "{path} is not a core module"
            );
        }
        let joined: String = CORE_MODULES.iter().map(|(_, t)| *t).collect();
        assert_eq!(PRELUDE_AND_CORE, format!("{PRELUDE}{joined}"));
    }
}
