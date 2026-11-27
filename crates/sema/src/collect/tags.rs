//! The fields of tagged literals (`comptime.tag.fields.1`): for each literal whose tag's value
//! implements `TagFields`, the hidden items that give it a `read` into a struct of its fields.
//! Their names are the tag's to say, so the tag runs while the program is collected
//! (`comptime.tag.fields.2`), through the database's [`Evaluator`](crate::Evaluator), on a
//! program of the tag's mod and its dependencies, collected without any literal's fields: so a
//! tag of the literal's own mod runs in that mod as it is before its literals have fields.

use moddil_syntax::ast::{self, AstNode};
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
        let tagged = moddil_syntax::Tagged::of(&text);
        let (tag, lit) = (tagged.tag, tagged.lit);
        let Some(Def::Fn(t)) = tree.lookup(module as u32, tag, false) else {
            continue;
        };
        let sig = &tree.fns[t as usize];
        let str_ref = |t: &Ty| matches!(t, Ty::Ref(false, s) if **s == Ty::Str);
        let takes_text = matches!(sig.params.as_slice(), [p] if str_ref(&p.ty));
        let takes_parts = matches!(sig.params.as_slice(),
            [p] if matches!(&p.ty, Ty::Ref(false, s) if matches!(&**s, Ty::Slice(e) if str_ref(e))));
        if !sig.tag || !sig.is_const || !(takes_text || takes_parts) {
            continue;
        }
        // A tag taking `&[&str]` is called with the literal's parts (`comptime.tag.2`): those of
        // a one-line literal, as written but for its braces.
        let parts = if takes_parts {
            if !tagged.raw && lit.starts_with("\"\"\"") {
                continue;
            }
            match split_parts(tagged.body().0) {
                Some(p) => Some(p),
                None => continue,
            }
        } else {
            None
        };
        if !matches!(
            Solver::new(tree, &[]).implements(&sig.ret, &fields_trait),
            Answer::Yes(_)
        ) {
            continue;
        }
        let Src::File(tf) = sig.src else { continue };
        let home = files[tf as usize].mod_;
        // A tag of the literal's own mod is evaluated in that mod, collected without the fields
        // of its literals (`comptime.tag.fields.2`).
        let own = home == files[f as usize].mod_;
        let value = if parts.is_some() {
            String::new()
        } else {
            match crate::lower::text::string_value(&format!("r{lit}")) {
                Ok(v) => v,
                Err(_) => continue,
            }
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
            parts,
            own,
        };
        // Where the text starts in the literal: after the tag and its opening quotes.
        let from = range.start + tagged.body().1 as u32;
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

/// The parts of a literal's text for a tag taking `&[&str]`: split at its holes, `{{` and `}}`
/// braces; `None` if a brace is unmatched (an error the literal reports when lowered).
fn split_parts(inner: &str) -> Option<Vec<String>> {
    let cs: Vec<char> = inner.chars().collect();
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut i = 0;
    while i < cs.len() {
        match cs[i] {
            '{' if cs.get(i + 1) == Some(&'{') => {
                cur.push('{');
                i += 2;
            }
            '}' if cs.get(i + 1) == Some(&'}') => {
                cur.push('}');
                i += 2;
            }
            '{' => {
                let mut depth = 1;
                i += 1;
                while i < cs.len() && depth > 0 {
                    match cs[i] {
                        '{' => depth += 1,
                        '}' => depth -= 1,
                        _ => {}
                    }
                    i += 1;
                }
                if depth != 0 {
                    return None;
                }
                parts.push(std::mem::take(&mut cur));
            }
            '}' => return None,
            c => {
                cur.push(c);
                i += 1;
            }
        }
    }
    parts.push(cur);
    Some(parts)
}

/// The types a tag may give its literal's fields (`comptime.tag.fields.3`): text, where it is,
/// and what the prelude's `FromStr` reads.
const FIELD_TYPES: &[&str] = &[
    "String", "Match", "int", "i8", "i16", "i32", "i128", "u8", "u16", "u32", "u64", "u128",
    "isize", "usize", "bool", "char",
];

/// Why the field names `fields` cannot be a struct's, or their types its fields', if they
/// cannot.
fn bad_names(fields: &[(String, bool, String)]) -> Option<String> {
    for (i, (name, _, ty)) in fields.iter().enumerate() {
        if !FIELD_TYPES.contains(&ty.as_str()) {
            return Some(format!(
                "this literal's field `{name}` is of type `{ty}`, which a field is not: \
                 `String`, `Match`, an integer type, `bool` or `char`"
            ));
        }
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
        if fields[..i].iter().any(|(n, _, _)| n == name) {
            return Some(format!("this literal has two fields named `{name}`"));
        }
    }
    None
}

/// The range of the tagged literal const `node` holds, if its value is one written as it is
/// (`const DATE: Regex = re"…";`).
pub(crate) fn const_literal(node: &SyntaxNode) -> Option<TextRange> {
    if node.kind() != K::CONST_ITEM {
        return None;
    }
    let init = node.children().find(|c| ast::Expr::can_cast(c.kind()))?;
    let tokens = init.tokens();
    let [tok] = tokens.as_slice() else {
        return None;
    };
    (tok.kind() == K::TAGGED_STRING && !tok.text().starts_with("fmt\"")).then(|| tok.text_range())
}

/// The name of the struct a literal at `range` reads its fields into (`DATE::Fields`).
pub(crate) fn fields_name(range: TextRange) -> String {
    format!("__TagFields_{}_{}", range.start, range.end)
}

/// The hidden items of the literal at `range`, whose fields are `fields`: a wrapper of its
/// value that dereferences to it and reads a text into a struct of the fields, each of its type
/// (`comptime.tag.fields.3`): a `String` as it is, a `Match` from where it is, and another type
/// with `FromStr`, a text it does not read making `read` give `None`.
fn items(range: TextRange, fields: &[(String, bool, String)]) -> Vec<String> {
    let (wrapper, make) = wrapper_names(range);
    let read = fields_name(range);
    let every_has = "unreachable(\"a field every reading has\")";
    let decls: Vec<String> = fields
        .iter()
        .map(|(n, every, ty)| {
            let ty = if ty == "Match" {
                "core::text::Match"
            } else {
                ty.as_str()
            };
            if *every {
                format!("pub {n}: {ty}")
            } else {
                format!("pub {n}: Option<{ty}>")
            }
        })
        .collect();
    let inits: Vec<String> = fields
        .iter()
        .enumerate()
        .map(|(i, (n, every, ty))| match (ty.as_str(), *every) {
            ("String", true) => {
                format!("{n}: match parts[{i}].clone() {{ Some(t) => t, None => {every_has} }}")
            }
            ("String", false) => format!("{n}: parts[{i}].clone()"),
            ("Match", true) => format!(
                "{n}: match spans[{i}] {{ Some((s, e)) => core::text::Match::new(s, e), None => \
                 {every_has} }}"
            ),
            ("Match", false) => format!(
                "{n}: match spans[{i}] {{ Some((s, e)) => Some(core::text::Match::new(s, e)), \
                 None => None }}"
            ),
            (ty, true) => format!(
                "{n}: match parts[{i}].clone() {{ Some(t) => {{ let r: Option<{ty}> = \
                 FromStr::from_str(t.as_str()); match r {{ Some(v) => v, None => return None }} }}, \
                 None => {every_has} }}"
            ),
            (ty, false) => format!(
                "{n}: match parts[{i}].clone() {{ Some(t) => {{ let r: Option<{ty}> = \
                 FromStr::from_str(t.as_str()); match r {{ Some(v) => Some(v), None => return \
                 None }} }}, None => None }}"
            ),
        })
        .collect();
    // Where the fields are, for a `Match` field.
    let spans = if fields.iter().any(|(_, _, t)| t == "Match") {
        "let spans = match self.value.field_spans(text) { Some(s) => s, None => return None }; "
    } else {
        ""
    };
    vec![
        format!("pub struct {wrapper}<V> {{ value: V }}"),
        format!("fn {make}<V>(value: V) -> {wrapper}<V> {{ {wrapper} {{ value: value }} }}"),
        format!(
            "impl<V> Deref for {wrapper}<V> {{ type Output = V; fn deref(&self) -> &V {{ \
             &self.value }} }}"
        ),
        format!(
            "impl<V: TagFields> {wrapper}<V> {{ pub fn read(&self, text: &str) -> \
             Option<{read}> {{ match self.value.field_values(text) {{ Some(parts) => {{ \
             {spans}Some({read} {{ {} }}) }}, None => None }} }} }}",
            inits.join(", ")
        ),
        format!("pub struct {read} {{ {} }}", decls.join(", ")),
        format!("impl Clone for {read} {{ }}"),
        format!(
            "impl Debug for {read} {{ fn fmt(&self, out: &mut dyn TextWrite) {{ \
             out.write_str(\"{{ \"); {} out.write_str(\" }}\"); }} }}",
            fields
                .iter()
                .enumerate()
                .map(|(i, (n, _, _))| {
                    let sep = if i == 0 { "" } else { ", " };
                    format!("out.write_str(\"{sep}{n}: \"); Debug::fmt(&self.{n}, out);")
                })
                .collect::<Vec<_>>()
                .join(" ")
        ),
        format!("impl PartialEq for {read} {{ }}"),
    ]
}
