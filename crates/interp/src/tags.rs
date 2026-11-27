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
        (&request.files, &request.mods, request.own).hash(&mut hasher);
        let key = hasher.finish();
        let mut programs = self.programs.lock().expect("programs");
        let probe = programs
            .entry(key)
            .or_insert_with(|| build(&request.files, &request.mods));
        let source = probe_source(
            &request.module,
            &request.tag,
            &request.text,
            request.parts.as_deref(),
        );
        // The probe is in a module of its own, or, for a tag of the literal's own mod, which may
        // be private to its module, at the end of the tag's file: only the probe's errors count
        // there, for the mod's other code may use the fields not yet found. A tag that does not
        // compile fails when it runs.
        let mut at = ((probe.program.files(&probe.db).len() - 1) as u32, 0);
        if request.own
            && let Some(i) = request
                .files
                .iter()
                .position(|(_, _, m, module)| *m == 0 && *module == request.module)
        {
            let text = format!(
                "{}
{source}",
                request.files[i].1
            );
            let file = probe.program.files(&probe.db)[i].source;
            file.set_text(&mut probe.db).to(text);
            at = (i as u32, request.files[i].1.len() as u32);
        } else {
            probe.file.set_text(&mut probe.db).to(source);
        }
        let db = &probe.db;
        let (last, offset) = at;
        if let Some(d) = moddil_mir::diagnostics(db, probe.program).iter().find(|d| {
            d.severity == moddil_sema::Severity::Error
                && (!request.own || (d.file == last && d.range.start >= offset))
        }) {
            return TagAnswer::Failed(format!("its mod does not compile here: {}", d.message));
        }
        let tree = moddil_sema::item_tree(db, probe.program);
        let Some(f) = tree
            .fns
            .iter()
            .position(|f| f.name == PROBE && f.src == Src::File(last))
        else {
            return TagAnswer::Failed("the tag could not be called".to_string());
        };
        let go = || {
            crate::machine::run(
                db,
                probe.program,
                Item::Fn(f as u32),
                &[],
                &mut |_, _, _| {},
                Some(FUEL),
                None,
            )
        };
        let run = if request.own {
            crate::machine::with_sound_bodies(go)
        } else {
            go()
        };
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
/// it, or `?`, then its type's text, if the value gives one (`comptime.tag.fields.3`).
fn probe_source(module: &[String], tag: &str, text: &str, parts: Option<&[String]>) -> String {
    let mut path = String::from("mod::");
    for m in module {
        path.push_str(m);
        path.push_str("::");
    }
    let mut hashes = String::from("#");
    while std::iter::once(text)
        .chain(parts.unwrap_or_default().iter().map(String::as_str))
        .any(|t| t.contains(&format!("\"{hashes}")))
    {
        hashes.push('#');
    }
    // The tag's argument: the text, or its parts (`comptime.tag.2`).
    let arg = match parts {
        Some(ps) => {
            let ps: Vec<String> = ps
                .iter()
                .map(|p| format!("r{hashes}\"{p}\"{hashes}"))
                .collect();
            format!("&[{}]", ps.join(", "))
        }
        None => format!("r{hashes}\"{text}\"{hashes}"),
    };
    format!(
        "fn {PROBE}() {{\n    let value = {path}{tag}({arg});\n    \
         let mut out = String::new();\n    let types = value.field_types();\n    let mut i = 0;\n    \
         for (name, every) in value.field_names() {{\n        \
         out.push_str(name.as_str());\n        out.push_str(if every {{ \"!\" }} else {{ \"?\" }});\n        \
         if i < types.len() {{ out.push_str(types[i].as_str()); }}\n        i += 1;\n        \
         out.push_str(\"\\n\");\n    }}\n    compile_error(out.as_str(), {}, {})\n}}\n",
        PROBE_AT.0, PROBE_AT.1
    )
}

/// The fields a probe's message names: each one's name, whether every reading has it, and its
/// type's text (`String` where it gives none).
fn fields(message: &str) -> Vec<(String, bool, String)> {
    message
        .lines()
        .filter_map(|l| {
            let at = l.find(['!', '?'])?;
            let every = l[at..].starts_with('!');
            let ty = l[at + 1..].trim();
            let ty = if ty.is_empty() { "String" } else { ty };
            Some((l[..at].to_string(), every, ty.to_string()))
        })
        .collect()
}
