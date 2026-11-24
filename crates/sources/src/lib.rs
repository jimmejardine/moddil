//! The Moddil sources every implementation reads, as text: the prelude
//! (`reference/2027/prelude/`) and the built-in `core` mod's modules (`reference/2027/core/`).
//!
//! The compiler (`moddil-sema`) and the reference interpreter (`moddil-refinterp`) both take
//! them from here, so a file added to either directory is listed once. This crate holds data
//! only: the reference interpreter stays independent of the compiler's code.

/// The prelude's files, joined in this order.
macro_rules! prelude {
    () => {
        concat!(
            include_str!("../../../reference/2027/prelude/core.moddil"),
            include_str!("../../../reference/2027/prelude/int.moddil"),
            include_str!("../../../reference/2027/prelude/numbers.moddil"),
            include_str!("../../../reference/2027/prelude/convert.moddil"),
            include_str!("../../../reference/2027/prelude/float.moddil"),
            include_str!("../../../reference/2027/prelude/text.moddil"),
            include_str!("../../../reference/2027/prelude/fmt.moddil"),
            include_str!("../../../reference/2027/prelude/arrays.moddil"),
            include_str!("../../../reference/2027/prelude/iter.moddil"),
            include_str!("../../../reference/2027/prelude/pointers.moddil"),
            include_str!("../../../reference/2027/prelude/collections.moddil"),
            include_str!("../../../reference/2027/prelude/hash.moddil"),
            include_str!("../../../reference/2027/prelude/program.moddil"),
            include_str!("../../../reference/2027/prelude/concurrency.moddil"),
        )
    };
}

/// The `core` mod's modules: each one's name, and its file.
macro_rules! core_modules {
    ($m:ident) => {
        $m!(
            "collections" => "../../../reference/2027/core/collections.moddil",
            "ptr" => "../../../reference/2027/core/ptr.moddil",
            "cell" => "../../../reference/2027/core/cell.moddil",
            "atomic" => "../../../reference/2027/core/atomic.moddil",
            "os" => "../../../reference/2027/core/os.moddil",
            "gpu" => "../../../reference/2027/core/gpu.moddil",
            "ffi" => "../../../reference/2027/core/ffi.moddil",
        )
    };
}

macro_rules! as_list {
    ($($name:literal => $file:literal),* $(,)?) => {
        [$(($name, include_str!($file))),*]
    };
}

macro_rules! as_text {
    ($($name:literal => $file:literal),* $(,)?) => {
        concat!(prelude!(), $(include_str!($file)),*)
    };
}

/// The prelude's source: the files of `reference/2027/prelude/`, joined.
pub const PRELUDE: &str = prelude!();

/// The `core` mod's modules, in order: each one's name and source.
pub const CORE_MODULES: [(&str, &str); 7] = core_modules!(as_list);

/// The prelude's source followed by the `core` mod's modules, in [`CORE_MODULES`]' order: the
/// compiler's prelude text, in which the `core` items start at [`PRELUDE`]'s length.
pub const PRELUDE_AND_CORE: &str = core_modules!(as_text);

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
