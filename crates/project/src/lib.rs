//! Reads a Moddil program from disk (`modules.md`): a single file, or a mod's directory with its
//! manifest (`moddil.toml`), its module tree and the mods it depends on by path. The result is
//! plain data that the compiler and the reference interpreter both take. See this crate's
//! `README.md`.
//!
//! - **The file tree is the module tree.** Every `.moddil` file under the mod's source directory
//!   is a module: the library root (`[lib] root`, by default `src/lib.moddil`) is the root module,
//!   and `src/book/levels.moddil` is `book::levels`. A directory with its own manifest is another
//!   mod, and is left out.
//! - **Dependencies are by path**, and form a DAG: a cycle is a problem (E0705).

use std::collections::HashMap;
use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};

mod manifest;

use manifest::{Table, Value};

/// The manifest's file name.
pub const MANIFEST: &str = "moddil.toml";

/// The only edition there is.
pub const EDITION: &str = "2027";

/// A program on disk: its files and its mods. The first mod is the one being built.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Project {
    /// The files, each a module of a mod.
    pub files: Vec<File>,
    /// The mods.
    pub mods: Vec<Mod>,
}

/// A source file of a project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    /// Its path on disk.
    pub path: PathBuf,
    /// Its path as shown in messages: relative to the root mod's directory, with `/`.
    pub display: String,
    /// Its text, with `\r\n` line ends made `\n`.
    pub text: String,
    /// The mod it belongs to, by index.
    pub mod_: u32,
    /// Its module path in the mod: empty for the root module.
    pub module: Vec<String>,
    /// Whether it is an executable's main module.
    pub exe: bool,
}

/// A mod of a project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mod {
    /// Its name.
    pub name: String,
    /// The mods it depends on: the name it calls each by, and the mod's index.
    pub deps: Vec<(String, u32)>,
    /// Its manifest, if it has one (a single file has none).
    pub manifest: Option<PathBuf>,
}

/// A problem reading a project: a manifest that is wrong, or a file that cannot be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    /// The code: `E0704` (an invalid manifest) or `E0705` (a dependency cycle).
    pub code: &'static str,
    /// The file it is in, as shown in messages.
    pub display: String,
    /// The file's path on disk, and its text, if it could be read.
    pub path: PathBuf,
    /// The file's text, if it could be read (to show the problem in it).
    pub text: String,
    /// Where in the text, as a byte range (empty if nowhere in particular).
    pub range: Range<usize>,
    /// What is wrong.
    pub message: String,
}

/// Reads the program at `path`: a `.moddil` file, which is a program of one module, or a
/// directory with a `moddil.toml`.
///
/// # Errors
///
/// The problems found, if the program cannot be read.
pub fn load(path: &Path) -> Result<Project, Vec<Problem>> {
    if path.is_dir() {
        let mut l = Loader {
            base: path.to_path_buf(),
            project: Project::default(),
            loaded: HashMap::new(),
            stack: Vec::new(),
            problems: Vec::new(),
        };
        l.load_mod(path, None);
        return if l.problems.is_empty() {
            Ok(l.project)
        } else {
            Err(l.problems)
        };
    }
    let display = path.display().to_string().replace('\\', "/");
    match fs::read_to_string(path) {
        Ok(text) => Ok(Project {
            files: vec![File {
                path: path.to_path_buf(),
                display,
                text: text.replace("\r\n", "\n"),
                mod_: 0,
                module: Vec::new(),
                exe: false,
            }],
            mods: vec![Mod {
                name: String::new(),
                deps: Vec::new(),
                manifest: None,
            }],
        }),
        Err(e) => Err(vec![Problem {
            code: "E0704",
            display,
            path: path.to_path_buf(),
            text: String::new(),
            range: 0..0,
            message: format!("cannot read the file: {e}"),
        }]),
    }
}

struct Loader {
    /// The root mod's directory, which paths are shown relative to.
    base: PathBuf,
    project: Project,
    /// The mods read so far, by their directory.
    loaded: HashMap<PathBuf, u32>,
    /// The mods being read, outermost first, to find cycles.
    stack: Vec<PathBuf>,
    problems: Vec<Problem>,
}

impl Loader {
    fn display(&self, p: &Path) -> String {
        let rel = p.strip_prefix(&self.base).unwrap_or(p);
        let s = rel.display().to_string().replace('\\', "/");
        s.trim_start_matches("./").to_string()
    }

    fn problem(
        &mut self,
        code: &'static str,
        path: &Path,
        text: &str,
        range: Range<usize>,
        message: String,
    ) {
        self.problems.push(Problem {
            code,
            display: self.display(path),
            path: path.to_path_buf(),
            text: text.to_string(),
            range,
            message,
        });
    }

    /// Reads the mod in `dir` and those it depends on; its index. `from` is the dependency line
    /// that names it (the manifest, its text and the line's range), for a cycle.
    fn load_mod(&mut self, dir: &Path, from: Option<(&Path, &str, Range<usize>)>) -> Option<u32> {
        let key = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
        if self.stack.contains(&key) {
            if let Some((m, text, range)) = from {
                let names: Vec<String> = self
                    .stack
                    .iter()
                    .skip_while(|p| **p != key)
                    .filter_map(|p| self.loaded.get(p))
                    .map(|&i| self.project.mods[i as usize].name.clone())
                    .collect();
                self.problem(
                    "E0705",
                    m,
                    text,
                    range,
                    format!(
                        "mods depend on each other in a cycle ({} and back): mods form a DAG",
                        names.join(" → ")
                    ),
                );
            }
            return None;
        }
        if let Some(&i) = self.loaded.get(&key) {
            return Some(i);
        }
        let manifest = dir.join(MANIFEST);
        let text = match fs::read_to_string(&manifest) {
            Ok(t) => t.replace("\r\n", "\n"),
            Err(e) => {
                let (m, t, r) = from.map_or((manifest.as_path(), "", 0..0), |(m, t, r)| (m, t, r));
                let m = m.to_path_buf();
                let t = t.to_string();
                self.problem(
                    "E0704",
                    &m,
                    &t,
                    r,
                    format!("cannot read `{}`: {e}", self.display(&manifest)),
                );
                return None;
            }
        };
        let tables = match manifest::parse(&text) {
            Ok(t) => t,
            Err((range, message)) => {
                self.problem("E0704", &manifest, &text, range, message);
                return None;
            }
        };
        let index = self.project.mods.len() as u32;
        self.loaded.insert(key.clone(), index);
        self.stack.push(key);
        self.project.mods.push(Mod {
            name: String::new(),
            deps: Vec::new(),
            manifest: Some(manifest.clone()),
        });
        let m = Manifest {
            path: &manifest,
            text: &text,
            tables: &tables,
        };
        let name = self.mod_section(&m);
        self.project.mods[index as usize].name = name;
        let (src_dir, root) = self.lib_root(&m, dir);
        let exes = self.exes(&m, dir);
        self.module_tree(dir, &src_dir, root.as_deref(), &exes, index);
        let mut deps = Vec::new();
        for t in tables.iter().filter(|t| t.name == "dependencies") {
            for p in &t.pairs {
                let path = match &p.value {
                    Value::Table(kv) => {
                        kv.iter().find(|(k, _)| k == "path").map(|(_, v)| v.clone())
                    }
                    Value::Str(_) => None,
                };
                let Some(path) = path else {
                    self.problem(
                        "E0704",
                        &manifest,
                        &text,
                        p.range.clone(),
                        "a dependency names its directory: `name = { path = \"../name\" }`"
                            .to_string(),
                    );
                    continue;
                };
                if !is_name(&p.key) {
                    self.problem(
                        "E0704",
                        &manifest,
                        &text,
                        p.range.clone(),
                        format!("`{}` is not a name a path can start with", p.key),
                    );
                    continue;
                }
                if let Some(d) =
                    self.load_mod(&dir.join(&path), Some((&manifest, &text, p.range.clone())))
                {
                    deps.push((p.key.clone(), d));
                }
            }
        }
        self.project.mods[index as usize].deps = deps;
        self.stack.pop();
        Some(index)
    }

    /// `[mod]`: the mod's name, and its edition, which must be 2027.
    fn mod_section(&mut self, m: &Manifest) -> String {
        for t in m.tables {
            if !matches!(t.name.as_str(), "" | "mod" | "lib" | "exe" | "dependencies") {
                self.problem(
                    "E0704",
                    m.path,
                    m.text,
                    t.range.clone(),
                    format!("there is no table `{}` in a manifest yet", t.name),
                );
            }
        }
        let Some(t) = m.table("mod") else {
            self.problem(
                "E0704",
                m.path,
                m.text,
                0..0,
                "a manifest has a `[mod]` table".to_string(),
            );
            return String::new();
        };
        let mut name = String::new();
        for p in &t.pairs {
            match (p.key.as_str(), &p.value) {
                ("name", Value::Str(s)) => name = s.clone(),
                ("edition", Value::Str(s)) if s == EDITION => {}
                ("edition", _) => self.problem(
                    "E0704",
                    m.path,
                    m.text,
                    p.range.clone(),
                    format!("the edition is \"{EDITION}\", the only one there is"),
                ),
                _ => self.problem(
                    "E0704",
                    m.path,
                    m.text,
                    p.range.clone(),
                    format!("`[mod]` has no `{}`", p.key),
                ),
            }
        }
        if name.is_empty() {
            self.problem(
                "E0704",
                m.path,
                m.text,
                t.range.clone(),
                "a mod has a `name`".to_string(),
            );
        }
        name
    }

    /// `[lib] root`: the source directory and the root module's file, if there is a library.
    fn lib_root(&mut self, m: &Manifest, dir: &Path) -> (PathBuf, Option<PathBuf>) {
        let written = m.table("lib").and_then(|t| {
            t.pairs
                .iter()
                .find_map(|p| match (p.key.as_str(), &p.value) {
                    ("root", Value::Str(s)) => Some((s.clone(), p.range.clone())),
                    _ => None,
                })
        });
        match written {
            Some((root, range)) => {
                let path = dir.join(&root);
                if !path.is_file() {
                    self.problem(
                        "E0704",
                        m.path,
                        m.text,
                        range,
                        format!("there is no file `{root}`"),
                    );
                    return (dir.join("src"), None);
                }
                let src = path
                    .parent()
                    .map_or_else(|| dir.to_path_buf(), Path::to_path_buf);
                (src, Some(path))
            }
            None => {
                let path = dir.join("src").join("lib.moddil");
                let root = path.is_file().then_some(path);
                (dir.join("src"), root)
            }
        }
    }

    /// `[[exe]] main`: each executable's main file.
    fn exes(&mut self, m: &Manifest, dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for t in m.tables.iter().filter(|t| t.name == "exe") {
            let main = t
                .pairs
                .iter()
                .find_map(|p| match (p.key.as_str(), &p.value) {
                    ("main", Value::Str(s)) => Some((s.clone(), p.range.clone())),
                    _ => None,
                });
            match main {
                Some((main, range)) => {
                    let path = dir.join(&main);
                    if path.is_file() {
                        out.push(path);
                    } else {
                        self.problem(
                            "E0704",
                            m.path,
                            m.text,
                            range,
                            format!("there is no file `{main}`"),
                        );
                    }
                }
                None => self.problem(
                    "E0704",
                    m.path,
                    m.text,
                    t.range.clone(),
                    "an executable names its main file: `main = \"src/main.moddil\"`".to_string(),
                ),
            }
        }
        out
    }

    /// Every `.moddil` file under `src_dir` (but in directories with a manifest of their own),
    /// as a module of mod `index`.
    fn module_tree(
        &mut self,
        dir: &Path,
        src_dir: &Path,
        root: Option<&Path>,
        exes: &[PathBuf],
        index: u32,
    ) {
        let mut found = Vec::new();
        moddil_files(src_dir, true, &mut found);
        for path in found {
            let rel = path.strip_prefix(src_dir).unwrap_or(&path);
            let is_root = root.is_some_and(|r| same(r, &path));
            let module: Vec<String> = if is_root {
                Vec::new()
            } else {
                let mut parts: Vec<String> = rel
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect();
                if let Some(last) = parts.last_mut() {
                    *last = last.trim_end_matches(".moddil").to_string();
                }
                parts
            };
            let display = self.display(&path);
            let text = match fs::read_to_string(&path) {
                Ok(t) => t.replace("\r\n", "\n"),
                Err(e) => {
                    self.problem(
                        "E0704",
                        &path,
                        "",
                        0..0,
                        format!("cannot read the file: {e}"),
                    );
                    continue;
                }
            };
            if let Some(bad) = module.iter().find(|p| !is_name(p)) {
                self.problem(
                    "E0704",
                    &path,
                    &text,
                    0..0,
                    format!("`{bad}` is not a name, so this file cannot be a module"),
                );
                continue;
            }
            let exe = exes.iter().any(|e| same(e, &path));
            self.project.files.push(File {
                path,
                display,
                text,
                mod_: index,
                module,
                exe,
            });
        }
        for e in exes {
            if !e.starts_with(src_dir) {
                let manifest = dir.join(MANIFEST);
                self.problem(
                    "E0704",
                    &manifest,
                    "",
                    0..0,
                    format!(
                        "`{}` is not under the mod's source directory",
                        self.display(e)
                    ),
                );
            }
        }
    }
}

/// A manifest being read.
struct Manifest<'a> {
    path: &'a Path,
    text: &'a str,
    tables: &'a [Table],
}

impl Manifest<'_> {
    fn table(&self, name: &str) -> Option<&Table> {
        self.tables.iter().find(|t| t.name == name)
    }
}

/// Whether `a` and `b` are the same file.
fn same(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// Whether `s` is a name: a letter or `_`, then letters, digits and `_`.
fn is_name(s: &str) -> bool {
    let mut cs = s.chars();
    cs.next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && cs.all(|c| c.is_alphanumeric() || c == '_')
}

/// The `.moddil` files under `dir`, sorted, leaving out directories with a manifest of their own
/// (but `dir` itself, if `top`).
fn moddil_files(dir: &Path, top: bool, out: &mut Vec<PathBuf>) {
    if !top && dir.join(MANIFEST).is_file() {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            moddil_files(&p, false, out);
        } else if p.extension().is_some_and(|e| e == "moddil") {
            out.push(p);
        }
    }
}

/// This crate's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests;
