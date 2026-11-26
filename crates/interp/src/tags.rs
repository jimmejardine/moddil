//! Evaluating a tag while a program is collected, for its literal's fields
//! (`comptime.tag.fields.2`): sema asks through the [`Evaluator`] it is given.
//!
//! The tag runs in a program of its own mod and that mod's dependencies, in a database of its
//! own, never the one being collected. One more module there calls the tag on the literal's text
//! and its value's `TagFields::field_names`, and hands the names back as a `compile_error`'s
//! message, which is how compile-time code's text reaches the compiler. Each such program is
//! kept, by its sources, and each answer, by its request.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

use moddil_sema::{
    Database, Evaluator, Item, ModInfo, Program, ProgramFile, SourceFile, Src, TagAnswer,
    TagRequest,
};
use salsa::Setter;

use crate::{FUEL, Stop};

/// The function the extra module defines.
const PROBE: &str = "__tag_fields";
/// The extra module's path in the tag's mod.
const PROBE_MODULE: &str = "__tag_fields";
/// The bytes the probe's `compile_error` names, which no tag's text has: its message is the
/// fields.
const PROBE_AT: (i64, i64) = (-1, -2);

/// A database for compiling programs whose tags give their literals fields: it evaluates them
/// while collecting (`comptime.tag.fields.2`).
pub fn database() -> Database {
    Database::with_evaluator(Arc::new(TagEvaluator::default()))
}

/// Evaluates tags for their literals' fields, keeping the programs it builds to.
#[derive(Default)]
pub struct TagEvaluator {
    programs: Mutex<HashMap<u64, Probe>>,
    answers: Mutex<HashMap<TagRequest, TagAnswer>>,
}

/// A program of a tag's mod and its dependencies, and the extra module's file.
struct Probe {
    db: Database,
    program: Program,
    file: SourceFile,
}

impl Evaluator for TagEvaluator {
    fn tag_fields(&self, request: &TagRequest) -> TagAnswer {
        if let Some(a) = self.answers.lock().expect("answers").get(request) {
            return a.clone();
        }
        // On a thread of its own: Salsa allows one database per thread while a query runs, and
        // this runs inside one of the database being collected.
        let answer = std::thread::scope(|s| {
            std::thread::Builder::new()
                .stack_size(256 << 20)
                .spawn_scoped(s, || self.evaluate(request))
                .expect("a thread")
                .join()
        })
        .unwrap_or_else(|_| TagAnswer::Failed("evaluating it panicked".to_string()));
        self.answers
            .lock()
            .expect("answers")
            .insert(request.clone(), answer.clone());
        answer
    }
}

impl TagEvaluator {
    fn evaluate(&self, request: &TagRequest) -> TagAnswer {
        let mut hasher = DefaultHasher::new();
        (&request.files, &request.mods).hash(&mut hasher);
        let key = hasher.finish();
        let mut programs = self.programs.lock().expect("programs");
        let probe = programs
            .entry(key)
            .or_insert_with(|| build(&request.files, &request.mods));
        probe.file.set_text(&mut probe.db).to(probe_source(
            &request.module,
            &request.tag,
            &request.text,
        ));
        let db = &probe.db;
        if let Some(d) = moddil_mir::diagnostics(db, probe.program)
            .iter()
            .find(|d| d.severity == moddil_sema::Severity::Error)
        {
            return TagAnswer::Failed(format!("its mod does not compile here: {}", d.message));
        }
        let last = (probe.program.files(db).len() - 1) as u32;
        let tree = moddil_sema::item_tree(db, probe.program);
        let Some(f) = tree
            .fns
            .iter()
            .position(|f| f.name == PROBE && f.src == Src::File(last))
        else {
            return TagAnswer::Failed("the tag could not be called".to_string());
        };
        let run = crate::machine::run(
            db,
            probe.program,
            Item::Fn(f as u32),
            &[],
            &mut |_, _, _| {},
            Some(FUEL),
            None,
        );
        match run {
            Err(Stop::Trap(t)) if t.at == Some(PROBE_AT) => {
                TagAnswer::Fields(fields(&t.reason.unwrap_or_default()))
            }
            Err(Stop::Trap(t)) if t.at.is_some() => {
                let (s, e) = t.at.unwrap_or_default();
                TagAnswer::CompileError(t.reason.unwrap_or_default(), s, e)
            }
            Err(Stop::Trap(t)) => TagAnswer::Failed(match t.reason {
                Some(r) => format!("{:?}: {r}", t.class),
                None => format!("{:?}", t.class),
            }),
            Err(Stop::Failed { .. }) => TagAnswer::Failed("an error left it".to_string()),
            Ok(_) => TagAnswer::Failed("it gave no fields".to_string()),
        }
    }
}

/// The program of `files` and `mods`, and an empty file for the extra module, in mod 0.
fn build(files: &[(String, String, u32, Vec<String>)], mods: &[ModInfo]) -> Probe {
    let db = Database::default();
    let mut list: Vec<ProgramFile> = files
        .iter()
        .map(|(path, text, mod_, module)| ProgramFile {
            source: SourceFile::new(&db, path.clone(), text.clone()),
            mod_: *mod_,
            module: module.clone(),
            exe: false,
        })
        .collect();
    let file = SourceFile::new(&db, format!("<{PROBE_MODULE}>"), String::new());
    list.push(ProgramFile {
        source: file,
        mod_: 0,
        module: vec![PROBE_MODULE.to_string()],
        exe: false,
    });
    let program = Program::new(&db, list, mods.to_vec());
    Probe { db, program, file }
}

/// The extra module: the tag in module `module` called on `text`, and its value's fields given
/// back as a `compile_error`'s message, a line each: the name, then `!` if every reading has
/// it, or `?`.
fn probe_source(module: &[String], tag: &str, text: &str) -> String {
    let mut path = String::from("mod::");
    for m in module {
        path.push_str(m);
        path.push_str("::");
    }
    let mut hashes = String::from("#");
    while text.contains(&format!("\"{hashes}")) {
        hashes.push('#');
    }
    format!(
        "fn {PROBE}() {{\n    let value = {path}{tag}(r{hashes}\"{text}\"{hashes});\n    \
         let mut out = String::new();\n    for (name, every) in value.field_names() {{\n        \
         out.push_str(name.as_str());\n        out.push_str(if every {{ \"!\" }} else {{ \"?\" }});\n        \
         out.push_str(\"\\n\");\n    }}\n    compile_error(out.as_str(), {}, {})\n}}\n",
        PROBE_AT.0, PROBE_AT.1
    )
}

/// The fields a probe's message names.
fn fields(message: &str) -> Vec<(String, bool)> {
    message
        .lines()
        .filter_map(|l| {
            let every = l.ends_with('!');
            let name = l.strip_suffix('!').or_else(|| l.strip_suffix('?'))?;
            Some((name.to_string(), every))
        })
        .collect()
}
