use std::fs;
use std::path::PathBuf;

use super::*;

/// A fresh directory holding `files` (path and text).
fn tree(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("moddil-project-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    for (p, text) in files {
        let path = dir.join(p);
        fs::create_dir_all(path.parent().expect("a parent")).expect("a directory");
        fs::write(path, text).expect("written");
    }
    dir
}

#[test]
fn the_file_tree_is_the_module_tree() {
    let dir = tree(
        "tree",
        &[
            (
                "moddil.toml",
                "[mod]\nname = \"a\"\n\n[[exe]]\nmain = \"src/main.moddil\"\n\n[dependencies]\nb = { path = \"b\" }\n",
            ),
            ("src/lib.moddil", ""),
            ("src/main.moddil", ""),
            ("src/book/levels.moddil", ""),
            ("b/moddil.toml", "[mod]\nname = \"b\"\n"),
            ("b/src/lib.moddil", ""),
        ],
    );
    let p = load(&dir).expect("a project");
    let modules: Vec<(u32, Vec<String>, bool)> = p
        .files
        .iter()
        .map(|f| (f.mod_, f.module.clone(), f.exe))
        .collect();
    assert_eq!(
        modules,
        vec![
            (0, vec!["book".to_string(), "levels".to_string()], false),
            (0, Vec::new(), false),
            (0, vec!["main".to_string()], true),
            (1, Vec::new(), false),
        ]
    );
    assert_eq!(p.mods[0].name, "a");
    assert_eq!(p.mods[0].deps, vec![("b".to_string(), 1)]);
    assert_eq!(p.files[0].display, "src/book/levels.moddil");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn cycles_and_bad_manifests_are_problems() {
    let dir = tree(
        "cycle",
        &[
            (
                "moddil.toml",
                "[mod]\nname = \"a\"\n[dependencies]\nb = { path = \"b\" }\n",
            ),
            (
                "b/moddil.toml",
                "[mod]\nname = \"b\"\n[dependencies]\na = { path = \"..\" }\n",
            ),
        ],
    );
    let problems = load(&dir).expect_err("a cycle");
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].code, "E0705");
    assert_eq!(problems[0].display, "b/moddil.toml");
    let _ = fs::remove_dir_all(&dir);

    let dir = tree(
        "bad",
        &[("moddil.toml", "[mod]\nname = \"a\"\nedition = \"2030\"\n")],
    );
    let problems = load(&dir).expect_err("an edition");
    assert_eq!(problems[0].code, "E0704");
    let _ = fs::remove_dir_all(&dir);
}
