//! The Moddil language server (`moddil lsp`, `toolchain.md`: Editor support): live diagnostics,
//! hover, go to definition, inlay hints at suspension points, and completion.
//!
//! - [`Analysis`] answers by file and byte offset, over the same salsa queries `moddil check` uses,
//!   so an edit recomputes only what depends on it. It is the testable part.
//! - [`run`] speaks LSP over standard input and output (`lsp-server`), converting between its
//!   UTF-16 positions and byte offsets.

mod complete;
mod query;
mod server;
mod text;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use moddil_sema::{Database, Diagnostic, Program};
use moddil_syntax::TextRange;
use query::Place;
use salsa::Setter;

pub use complete::{Completion, CompletionKind};
pub use server::{run, serve};
pub use text::{Edit, LineIndex, apply_edits, path_uri, uri_path};

/// A problem found: where, how serious, its code and its message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    /// Where, in bytes.
    pub range: TextRange,
    /// Whether it is an error (or a warning).
    pub error: bool,
    /// Its code (`E0201`).
    pub code: String,
    /// What is wrong, with any secondary labels after it.
    pub message: String,
}

/// An inlay hint: text shown after byte `offset`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hint {
    /// Where it goes.
    pub offset: u32,
    /// What it says.
    pub label: String,
}

/// The programs being edited, each loaded from its mod (the nearest `moddil.toml` above a file)
/// or a single file, with the open files' texts in place of what is on disk.
#[derive(Default)]
pub struct Analysis {
    db: Database,
    roots: Vec<Root>,
    /// The prelude's and `core`'s files a definition has been found in, by path, with their
    /// text: not part of any program.
    library: HashMap<PathBuf, String>,
}

/// One program: a mod with its dependencies, or a single file.
struct Root {
    /// The manifest's directory, or the file.
    at: PathBuf,
    program: Program,
    /// Each file's path on disk, by its index in the program, canonical.
    paths: Vec<PathBuf>,
    /// Problems reading the project (a manifest, a cycle): its files are not checked.
    load_problems: Vec<(PathBuf, Problem)>,
}

/// The file of the prelude or `core` at `rel` under `reference/2027/`, holding `text`: under the
/// program's directory `at` or one above it (a checkout of Moddil's repository), or where this
/// server was built from. One whose text differs is not it: its offsets would not match.
fn library_file(at: &Path, rel: &str, text: &str) -> Option<PathBuf> {
    let built = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../reference/2027");
    at.ancestors()
        .map(|d| d.join("reference/2027"))
        .chain(std::iter::once(built))
        .map(|dir| dir.join(rel))
        .find(|p| std::fs::read_to_string(p).is_ok_and(|t| t == text))
        .map(|p| canonical(&p))
}

/// `p`, canonical where it exists (so paths compare whatever their spelling).
fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// Where the program containing `file` starts: the nearest directory above it with a
/// `moddil.toml`, or the file itself.
fn root_of(file: &Path) -> PathBuf {
    let mut dir = file.parent();
    while let Some(d) = dir {
        if d.join("moddil.toml").is_file() {
            return d.to_path_buf();
        }
        dir = d.parent();
    }
    file.to_path_buf()
}

impl Analysis {
    /// No programs yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// The root and file index of `path`, loading its program the first time.
    fn locate(&mut self, path: &Path) -> Option<(usize, u32)> {
        let path = canonical(path);
        if let Some(found) = self.find(&path) {
            return Some(found);
        }
        let at = canonical(&root_of(&path));
        // A root already loaded that does not have the file (a new one) is reloaded.
        if let Some(r) = self.roots.iter().position(|r| r.at == at) {
            self.roots.remove(r);
        }
        let root = self.load(&at);
        self.roots.push(root);
        self.find(&path)
    }

    fn find(&self, path: &Path) -> Option<(usize, u32)> {
        self.roots.iter().enumerate().find_map(|(r, root)| {
            root.paths
                .iter()
                .position(|p| p == path)
                .map(|f| (r, f as u32))
        })
    }

    fn load(&mut self, at: &Path) -> Root {
        let db = &self.db;
        match moddil_project::load(at) {
            Ok(project) => Root {
                at: at.to_path_buf(),
                program: Program::of_project(db, &project),
                paths: project.files.iter().map(|f| canonical(&f.path)).collect(),
                load_problems: Vec::new(),
            },
            Err(problems) => Root {
                at: at.to_path_buf(),
                program: Program::new(db, Vec::new(), Vec::new()),
                paths: Vec::new(),
                load_problems: problems
                    .into_iter()
                    .map(|p| {
                        let range = TextRange::new(p.range.start as u32, p.range.end as u32);
                        let problem = Problem {
                            range,
                            error: true,
                            code: p.code.to_string(),
                            message: p.message,
                        };
                        (canonical(&p.path), problem)
                    })
                    .collect(),
            },
        }
    }

    /// Sets the text of `path` (an open file, as its editor has it). Its program is loaded the
    /// first time; a file new to its mod reloads it. Whether the file is part of a program.
    pub fn set_text(&mut self, path: &Path, text: &str) -> bool {
        let text = text.replace("\r\n", "\n");
        let Some((r, f)) = self.locate(path) else {
            return false;
        };
        let source = self.roots[r].program.files(&self.db)[f as usize].source;
        if source.text(&self.db) != &text {
            source.set_text(&mut self.db).to(text);
        }
        true
    }

    /// Forgets the programs, so each is read from disk again when next needed (after a manifest
    /// changes).
    pub fn reload(&mut self) {
        self.roots.clear();
    }

    /// The text of `path`, as the program has it (an open file's editor text).
    pub fn text(&mut self, path: &Path) -> Option<String> {
        if let Some(text) = self.library.get(path) {
            return Some(text.clone());
        }
        let (r, f) = self.locate(path)?;
        let source = self.roots[r].program.files(&self.db)[f as usize].source;
        Some(source.text(&self.db).clone())
    }

    /// The problems of every file of the mod `path` belongs to (not its dependencies), by path:
    /// each file appears, so a fixed one is cleared. Problems reading the project come first.
    pub fn problems(&mut self, path: &Path) -> Vec<(PathBuf, Vec<Problem>)> {
        let path = canonical(path);
        let at = canonical(&root_of(&path));
        if self.locate(&path).is_none() {
            let Some(root) = self.roots.iter().find(|r| r.at == at) else {
                return Vec::new();
            };
            let mut out: Vec<(PathBuf, Vec<Problem>)> = Vec::new();
            for (p, problem) in &root.load_problems {
                match out.iter_mut().find(|(q, _)| q == p) {
                    Some((_, ps)) => ps.push(problem.clone()),
                    None => out.push((p.clone(), vec![problem.clone()])),
                }
            }
            return out;
        }
        let Some((r, _)) = self.find(&path) else {
            return Vec::new();
        };
        let root = &self.roots[r];
        let files = root.program.files(&self.db);
        let mut out: Vec<(PathBuf, Vec<Problem>)> = files
            .iter()
            .enumerate()
            .filter(|(_, f)| f.mod_ == 0)
            .map(|(i, _)| (root.paths[i].clone(), Vec::new()))
            .collect();
        for d in moddil_interp::diagnostics(&self.db, root.program) {
            if files.get(d.file as usize).is_none_or(|f| f.mod_ != 0) {
                continue;
            }
            let p = &root.paths[d.file as usize];
            if let Some((_, ps)) = out.iter_mut().find(|(q, _)| q == p) {
                ps.push(problem(&d));
            }
        }
        out
    }

    /// What to show on hovering over byte `offset` of `path`.
    pub fn hover(&mut self, path: &Path, offset: u32) -> Option<String> {
        let (r, f) = self.locate(path)?;
        query::Query::new(&self.db, self.roots[r].program, f).hover(offset)
    }

    /// Where what is at byte `offset` of `path` is defined: a file and the range of its name.
    /// One in the prelude or `core` is in its file under `reference/2027/`, when that file is
    /// found and is the text built in.
    pub fn definition(&mut self, path: &Path, offset: u32) -> Option<(PathBuf, TextRange)> {
        let (r, f) = self.locate(path)?;
        match query::Query::new(&self.db, self.roots[r].program, f).definition(offset)? {
            Place::File(file, range) => {
                Some((self.roots[r].paths.get(file as usize)?.clone(), range))
            }
            Place::Library(range) => {
                let (rel, text, at) = moddil_sources::file_at(range.start as usize)?;
                let file = library_file(&self.roots[r].at, rel, text)?;
                self.library.insert(file.clone(), text.to_string());
                let start = at as u32;
                Some((file, TextRange::new(start, start + range.end - range.start)))
            }
        }
    }

    /// The inlay hints of `path`: after each call that may suspend or spawn.
    pub fn hints(&mut self, path: &Path) -> Vec<Hint> {
        let Some((r, f)) = self.locate(path) else {
            return Vec::new();
        };
        query::Query::new(&self.db, self.roots[r].program, f).hints()
    }

    /// The completions at byte `offset` of `path`.
    pub fn complete(&mut self, path: &Path, offset: u32) -> Vec<Completion> {
        let Some((r, f)) = self.locate(path) else {
            return Vec::new();
        };
        let program = self.roots[r].program;
        complete::complete(&mut self.db, program, f, offset)
    }
}

/// `d` as a [`Problem`], its secondary labels in its message.
fn problem(d: &Diagnostic) -> Problem {
    let mut message = d.message.clone();
    for (_, l) in &d.labels {
        if !l.is_empty() {
            message.push_str("\n- ");
            message.push_str(l);
        }
    }
    Problem {
        range: d.range,
        error: d.severity == moddil_sema::Severity::Error,
        code: d.code.as_str().to_string(),
        message,
    }
}
