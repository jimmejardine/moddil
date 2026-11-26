//! The fields of tagged literals (`comptime.tag.fields.1`): for each literal whose tag's value
//! implements `TagFields`, the hidden items that give it a `read` into a struct of its fields.
//! Their names are the tag's to say, so the tag runs while the program is collected
//! (`comptime.tag.fields.2`), through the database's [`Evaluator`](crate::Evaluator), on a
//! program of the tag's mod and its dependencies: never the literal's own mod, which is still
//! being collected.

use moddil_syntax::ast;
use moddil_syntax::{SyntaxKind as K, SyntaxNode, TextRange};

use super::{item_tree_base, parse};
use crate::db::{Program, TagAnswer, TagRequest};
use crate::items::{Def, Src};
use crate::traits::{Answer, Solver};
use crate::{Code, Db, Diagnostic, Ty};

/// The hidden items of a program's tagged literals with fields: each line, and the file and
/// range of the literal it is for; and the errors in finding their fields.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub(crate) struct TagItems {
    pub(crate) lines: Vec<(String, u32, TextRange)>,
    pub(crate) diagnostics: Vec<Diagnostic>,
}

/// The name of the wrapper type a tagged literal at `range` of its file is
/// (`comptime.tag.fields.1`), and of the function that makes one.
pub(crate) fn wrapper_names(range: TextRange) -> (String, String) {
    (
        format!("__Tag_{}_{}", range.start, range.end),
        format!("__tag_{}_{}", range.start, range.end),
    )
}

/// Whether derived impl `node` is a tagged literal's wrapper's (`impl Deref for __Tag_…`, or its
/// own methods): kept although its trait is not one a derive makes.
pub(crate) fn is_wrapper_impl(node: &SyntaxNode) -> bool {
    node.children()
        .any(|t| ast::Type::cast(t.clone()).is_some() && t.text().to_string().starts_with("__Tag_"))
}

/// The hidden items of `program`'s tagged literals with fields.
#[salsa::tracked(returns(ref))]
pub(crate) fn tag_items(db: &dyn Db, program: Program) -> TagItems {
    let mut out = TagItems::default();
    let files = program.files(db);
    // The literals, found before anything is collected twice: most programs have none.
    let mut literals = Vec::new();
    for (f, file) in files.iter().enumerate() {
        for tok in parse(db, file.source).syntax().tokens() {
            if tok.kind() == K::TAGGED_STRING && !tok.text().starts_with("fmt\"") {
                literals.push((f as u32, tok.text().to_string(), tok.text_range()));
            }
        }
    }
    if literals.is_empty() || db.evaluator().is_none() {
        return out;
    }
    let Some(evaluator) = db.evaluator() else {
        return out;
    };
    let tree = item_tree_base(db, program);
    let Some(fields_trait) = tree.lang_ref("TagFields") else {
        return out;
    };
    let mods = program.mods(db);
    for (f, text, range) in literals {
        let Some(module) = tree.modules.iter().position(|m| m.file == Some(f)) else {
            continue;
        };
        let at = text.find('"').unwrap_or(0);
        let (tag, lit) = text.split_at(at);
        let Some(Def::Fn(t)) = tree.lookup(module as u32, tag, false) else {
            continue;
        };
        let sig = &tree.fns[t as usize];
        let takes_text = matches!(sig.params.as_slice(),
            [p] if matches!(&p.ty, Ty::Ref(false, s) if **s == Ty::Str));
        if !sig.tag || !sig.is_const || !takes_text {
            continue;
        }
        if !matches!(
            Solver::new(tree, &[]).implements(&sig.ret, &fields_trait),
            Answer::Yes(_)
        ) {
            continue;
        }
        let Src::File(tf) = sig.src else { continue };
        let home = files[tf as usize].mod_;
        if home == files[f as usize].mod_ {
            out.diagnostics.push(Diagnostic::error(
                Code::E1012,
                range,
                format!(
                    "`{tag}`'s value has fields, which are found while this mod is collected, so \
                     the tag must come from another mod"
                ),
            ));
            continue;
        }
        let Ok(value) = crate::lower::text::string_value(&format!("r{lit}")) else {
            continue;
        };
        // The tag's mod first, then what it depends on, directly or not.
        let mut order = vec![home];
        let mut i = 0;
        while i < order.len() {
            for &(_, d) in &mods[order[i] as usize].deps {
                if !order.contains(&d) {
                    order.push(d);
                }
            }
            i += 1;
        }
        let new = |m: u32| order.iter().position(|&o| o == m).unwrap_or(0) as u32;
        let request = TagRequest {
            files: files
                .iter()
                .filter(|file| order.contains(&file.mod_))
                .map(|file| {
                    (
                        file.source.path(db).clone(),
                        file.source.text(db).clone(),
                        new(file.mod_),
                        file.module.clone(),
                    )
                })
                .collect(),
            mods: order
                .iter()
                .map(|&m| {
                    let mut info = mods[m as usize].clone();
                    info.deps = info
                        .deps
                        .iter()
                        .map(|(n, d)| (n.clone(), new(*d)))
                        .collect();
                    info
                })
                .collect(),
            module: tree.modules[sig.module as usize].path.clone(),
            tag: tag.to_string(),
            text: value,
        };
        // Where the text starts in the literal: after the tag and its opening quotes.
        let quotes = if lit.starts_with("\"\"\"") { 3 } else { 1 };
        let from = range.start + (tag.len() + quotes) as u32;
        let fields = match evaluator.tag_fields(&request) {
            TagAnswer::Fields(fields) => fields,
            TagAnswer::CompileError(message, s, e) => {
                let clamp = |v: i64| {
                    u32::try_from(v.max(0))
                        .unwrap_or(0)
                        .saturating_add(from)
                        .min(range.end)
                };
                let at = TextRange::new(clamp(s), clamp(e.max(s)));
                out.diagnostics
                    .push(Diagnostic::error(Code::E1010, at, message));
                continue;
            }
            TagAnswer::Failed(why) => {
                out.diagnostics.push(Diagnostic::error(
                    Code::E1004,
                    range,
                    format!("evaluating `{tag}` for this literal's fields failed: {why}"),
                ));
                continue;
            }
        };
        if let Some(bad) = bad_names(&fields) {
            out.diagnostics
                .push(Diagnostic::error(Code::E1012, range, bad));
            continue;
        }
        if fields.is_empty() {
            continue;
        }
        for line in items(range, &fields) {
            out.lines.push((line, f, range));
        }
    }
    out
}

/// Why the field names `fields` cannot be a struct's, if they cannot.
fn bad_names(fields: &[(String, bool)]) -> Option<String> {
    for (i, (name, _)) in fields.iter().enumerate() {
        let (tokens, errors) = moddil_syntax::lex(name);
        let one_name = errors.is_empty()
            && tokens.len() == 1
            && tokens[0].kind == K::IDENT
            && !name.starts_with("__");
        if !one_name {
            return Some(format!(
                "this literal's field `{name}` is not a name a field can have"
            ));
        }
        if fields[..i].iter().any(|(n, _)| n == name) {
            return Some(format!("this literal has two fields named `{name}`"));
        }
    }
    None
}

/// The hidden items of the literal at `range`, whose fields are `fields`: a wrapper of its
/// value that dereferences to it and reads a text into a struct of the fields.
fn items(range: TextRange, fields: &[(String, bool)]) -> Vec<String> {
    let (wrapper, make) = wrapper_names(range);
    let read = format!("__TagFields_{}_{}", range.start, range.end);
    let decls: Vec<String> = fields
        .iter()
        .map(|(n, every)| {
            if *every {
                format!("pub {n}: String")
            } else {
                format!("pub {n}: Option<String>")
            }
        })
        .collect();
    let inits: Vec<String> = fields
        .iter()
        .enumerate()
        .map(|(i, (n, every))| {
            if *every {
                format!(
                    "{n}: match parts[{i}].clone() {{ Some(t) => t, None => unreachable(\"a field \
                     every reading has\") }}"
                )
            } else {
                format!("{n}: parts[{i}].clone()")
            }
        })
        .collect();
    vec![
        format!("struct {wrapper}<V> {{ value: V }}"),
        format!("fn {make}<V>(value: V) -> {wrapper}<V> {{ {wrapper} {{ value: value }} }}"),
        format!(
            "impl<V> Deref for {wrapper}<V> {{ type Output = V; fn deref(&self) -> &V {{ \
             &self.value }} }}"
        ),
        format!(
            "impl<V: TagFields> {wrapper}<V> {{ pub fn read(&self, text: &str) -> \
             Option<{read}> {{ match self.value.field_values(text) {{ Some(parts) => \
             Some({read} {{ {} }}), None => None }} }} }}",
            inits.join(", ")
        ),
        format!("struct {read} {{ {} }}", decls.join(", ")),
        format!("impl Clone for {read} {{ }}"),
        format!("impl Debug for {read} {{ }}"),
        format!("impl PartialEq for {read} {{ }}"),
    ]
}
