//! Completion: after `.`, a value's fields and methods; after `::`, a module's, an enum's or a
//! type's members; otherwise the names in scope. For a value's members the file is analysed with
//! a placeholder name at the cursor (so `x.` parses as a field access), then restored.

use moddil_sema::hir::ExprKind;
use moddil_sema::{Database, Def, ItemTree, Owner, PRELUDE_MODULE, Program, Src, Ty};
use moddil_syntax::{SyntaxKind as K, SyntaxNode, TextRange};
use salsa::Setter;

use crate::query::{signature, strip};

/// What a completion is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionKind {
    /// A struct's field.
    Field,
    /// A method (it takes `self`).
    Method,
    /// A free or associated function.
    Function,
    /// A struct or enum.
    Type,
    /// An enum's variant.
    Variant,
    /// A trait.
    Trait,
    /// A module or mod.
    Module,
    /// A local variable or parameter.
    Local,
}

/// One completion: its name, what it is, and a line about it (a type or a signature).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    /// The name inserted.
    pub label: String,
    /// What it is.
    pub kind: CompletionKind,
    /// Its type or signature.
    pub detail: String,
}

/// The name put at the cursor to analyse the file there.
const PLACEHOLDER: &str = "__moddil_complete";

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The completions at byte `offset` of file `file`, those starting with the name being typed.
pub(crate) fn complete(
    db: &mut Database,
    program: Program,
    file: u32,
    offset: u32,
) -> Vec<Completion> {
    let source = program.files(db)[file as usize].source;
    let text = source.text(db).clone();
    let offset = (offset as usize).min(text.len());
    let before = &text[..offset];
    let start = before.trim_end_matches(is_ident).len();
    let prefix = before[start..].to_string();
    let context = &before[..start];
    let mut out = if let Some(path) = context.strip_suffix("::") {
        let segs: Vec<&str> = trailing_path(path);
        let items = moddil_sema::item_tree(db, program);
        path_members(items, module_of(items, file), &segs)
    } else {
        // Analyse with the placeholder where the name being typed is.
        let probe = format!("{}{PLACEHOLDER}{}", &text[..start], &text[offset..]);
        source.set_text(db).to(probe);
        let found = if context.ends_with('.') && !context.ends_with("..") {
            members(db, program, file, start as u32)
        } else {
            scope(db, program, file, start as u32)
        };
        source.set_text(db).to(text.clone());
        found
    };
    out.retain(|c| c.label.starts_with(&prefix) && c.label != PLACEHOLDER);
    out.sort_by(|a, b| a.label.cmp(&b.label));
    out.dedup_by(|a, b| a.label == b.label);
    out
}

/// The identifiers of the path that ends `s` (`a::b` of `let x = a::b`).
fn trailing_path(s: &str) -> Vec<&str> {
    let mut segs = Vec::new();
    let mut rest = s;
    loop {
        let start = rest.trim_end_matches(is_ident).len();
        let seg = &rest[start..];
        if seg.is_empty() {
            break;
        }
        segs.push(seg);
        match rest[..start].strip_suffix("::") {
            Some(r) => rest = r,
            None => break,
        }
    }
    segs.reverse();
    segs
}

/// The module file `file` is.
fn module_of(items: &ItemTree, file: u32) -> u32 {
    items
        .modules
        .iter()
        .position(|m| m.file == Some(file))
        .unwrap_or(0) as u32
}

/// The body the placeholder at `at` is in, and the expression it is.
fn placeholder_expr(
    db: &Database,
    program: Program,
    file: u32,
    at: u32,
) -> Option<(Owner<'_>, moddil_sema::hir::ExprId)> {
    let items = moddil_sema::item_tree(db, program);
    let fns = items
        .fns
        .iter()
        .enumerate()
        .filter(|(_, f)| {
            f.src == Src::File(file) && f.has_body && f.node.start <= at && at <= f.node.end
        })
        .map(|(i, f)| (moddil_sema::Item::Fn(i as u32), f.node));
    let tests = items
        .tests
        .iter()
        .enumerate()
        .filter(|(_, t)| t.file == file && t.node.start <= at && at <= t.node.end)
        .map(|(i, t)| (moddil_sema::Item::Test(i as u32), t.node));
    let (item, _) = fns.chain(tests).min_by_key(|(_, r)| r.len())?;
    let owner = Owner::new(db, program, item);
    let body = &moddil_sema::body(db, owner).body;
    let e = body
        .exprs
        .iter()
        .filter(|(_, x)| x.range.start <= at && at < x.range.end)
        .min_by_key(|(_, x)| x.range.len())
        .map(|(id, _)| id)?;
    Some((owner, e))
}

/// The fields and methods of the value before the `.` at the placeholder at `at`.
fn members(db: &Database, program: Program, file: u32, at: u32) -> Vec<Completion> {
    let Some((owner, e)) = placeholder_expr(db, program, file, at) else {
        return Vec::new();
    };
    let body = &moddil_sema::body(db, owner).body;
    let types = moddil_sema::infer(db, owner);
    // The placeholder is a field (`x.name`) or a method's name (`x.name(…)`).
    let base = match &body.exprs[e].kind {
        ExprKind::Field { base, .. } | ExprKind::MethodCall { receiver: base, .. } => *base,
        _ => return Vec::new(),
    };
    let items = moddil_sema::item_tree(db, program);
    type_members(
        db,
        program,
        items,
        module_of(items, file),
        &strip(&types.expr(base)),
    )
}

/// Whether impls for `impl_ty` apply to a value of `t`, by their outermost type.
fn same_head(impl_ty: &Ty, t: &Ty) -> bool {
    match (impl_ty, t) {
        (Ty::Adt(a, ..), Ty::Adt(b, ..)) | (Ty::Refined(a, _), Ty::Refined(b, _)) => a == b,
        (Ty::Num(a), Ty::Num(b)) => a == b,
        (Ty::Slice(_), Ty::Slice(_))
        | (Ty::Tuple(_), Ty::Tuple(_))
        | (Ty::Array(..), Ty::Array(..)) => true,
        (a, b) => {
            matches!(a, Ty::Int | Ty::Bool | Ty::Char | Ty::Str | Ty::Unit)
                && std::mem::discriminant(a) == std::mem::discriminant(b)
        }
    }
}

/// The fields (seen from module `from`) and methods of a value of type `t`.
fn type_members(
    db: &Database,
    program: Program,
    items: &ItemTree,
    from: u32,
    t: &Ty,
) -> Vec<Completion> {
    let mut out = Vec::new();
    if let Ty::Adt(a, _, args) = t {
        let adt = &items.adts[*a as usize];
        if !adt.is_enum
            && let Some(v) = adt.variants.first()
        {
            for f in &v.fields {
                if items.visible(adt.module, f.vis, from) && f.name.parse::<u32>().is_err() {
                    out.push(Completion {
                        label: f.name.clone(),
                        kind: CompletionKind::Field,
                        detail: f.ty.subst(args).to_string(),
                    });
                }
            }
        }
    }
    for imp in &items.impls {
        if !same_head(&imp.self_ty, t) {
            continue;
        }
        let mut names: Vec<&str> = Vec::new();
        for &m in &imp.methods {
            let f = &items.fns[m as usize];
            names.push(&f.name);
            if f.self_kind.is_some()
                && (imp.trait_ref.is_some() || items.visible(f.module, f.vis, from))
            {
                out.push(method(db, program, items, m));
            }
        }
        // A trait's methods the impl does not write: its defaults.
        if let Some(tr) = &imp.trait_ref {
            for &m in &items.traits[tr.trait_ as usize].methods {
                let f = &items.fns[m as usize];
                if f.self_kind.is_some() && !names.contains(&f.name.as_str()) {
                    out.push(method(db, program, items, m));
                }
            }
        }
    }
    out
}

fn method(db: &Database, program: Program, items: &ItemTree, f: u32) -> Completion {
    Completion {
        label: items.fns[f as usize].name.clone(),
        kind: CompletionKind::Method,
        detail: signature(db, program, f),
    }
}

/// A completion for `name`, which stands for `def`.
fn of_def(db: &Database, program: Program, items: &ItemTree, name: &str, def: Def) -> Completion {
    let (kind, detail) = match def {
        Def::Fn(f) => (CompletionKind::Function, signature(db, program, f)),
        Def::Adt(a) => {
            let adt = &items.adts[a as usize];
            let kw = if adt.is_enum { "enum" } else { "struct" };
            (CompletionKind::Type, format!("{kw} {}", adt.name))
        }
        Def::Type(t) => (
            CompletionKind::Type,
            format!(
                "type {} = {}",
                items.types[t as usize].name, items.types[t as usize].ty
            ),
        ),
        Def::Trait(t) => (
            CompletionKind::Trait,
            format!("trait {}", items.traits[t as usize].name),
        ),
        Def::Variant(a, v) => (
            CompletionKind::Variant,
            format!(
                "{}::{}",
                items.adts[a as usize].name, items.adts[a as usize].variants[v as usize].name
            ),
        ),
        Def::Module(m) => (
            CompletionKind::Module,
            format!("mod {}", items.module_path(m)),
        ),
    };
    Completion {
        label: name.to_string(),
        kind,
        detail,
    }
}

/// The names module `m` gives module `from`: its items, its imports and its modules.
fn module_names(
    db: &Database,
    program: Program,
    items: &ItemTree,
    m: u32,
    from: u32,
) -> Vec<Completion> {
    let module = &items.modules[m as usize];
    let mut out = Vec::new();
    for (name, e) in &module.items {
        if items.visible(m, e.vis, from) {
            out.push(of_def(db, program, items, name, e.def));
        }
    }
    for i in &module.imports {
        if items.visible(m, i.vis, from)
            && let Some(def) = items.import_target(m, i, 0)
        {
            out.push(of_def(db, program, items, &i.name, def));
        }
    }
    for (name, &c) in &module.children {
        out.push(of_def(db, program, items, name, Def::Module(c)));
    }
    out
}

/// The members of what `segs` names, seen from module `from`: a module's names, an enum's
/// variants, a type's associated functions, a trait's methods.
fn path_members(items: &ItemTree, from: u32, segs: &[&str]) -> Vec<Completion> {
    // Path completion reads only the item tree, which needs no database writes; signatures do,
    // through `effects`, so details here are short.
    let short = |name: &str, kind, detail: String| Completion {
        label: name.to_string(),
        kind,
        detail,
    };
    let Ok((def, n)) = items.path(from, segs, false) else {
        return Vec::new();
    };
    if n != segs.len() {
        return Vec::new();
    }
    let mut out = Vec::new();
    match def {
        Def::Module(m) => {
            let module = &items.modules[m as usize];
            for (name, e) in &module.items {
                if items.visible(m, e.vis, from) {
                    out.push(short(name, kind_of(e.def), String::new()));
                }
            }
            for i in &module.imports {
                if items.visible(m, i.vis, from)
                    && let Some(def) = items.import_target(m, i, 0)
                {
                    out.push(short(&i.name, kind_of(def), String::new()));
                }
            }
            for name in module.children.keys() {
                out.push(short(name, CompletionKind::Module, String::new()));
            }
        }
        Def::Adt(a) => {
            let adt = &items.adts[a as usize];
            if adt.is_enum {
                for v in &adt.variants {
                    out.push(short(&v.name, CompletionKind::Variant, adt.name.clone()));
                }
            }
            for imp in &items.impls {
                if !matches!(&imp.self_ty, Ty::Adt(b, ..) if *b == a) {
                    continue;
                }
                for &m in &imp.methods {
                    let f = &items.fns[m as usize];
                    if imp.trait_ref.is_some() || items.visible(f.module, f.vis, from) {
                        out.push(short(&f.name, CompletionKind::Function, String::new()));
                    }
                }
            }
        }
        Def::Trait(t) => {
            for &m in &items.traits[t as usize].methods {
                out.push(short(
                    &items.fns[m as usize].name,
                    CompletionKind::Function,
                    String::new(),
                ));
            }
        }
        Def::Fn(_) | Def::Type(_) | Def::Variant(..) => {}
    }
    out
}

fn kind_of(def: Def) -> CompletionKind {
    match def {
        Def::Fn(_) => CompletionKind::Function,
        Def::Adt(_) | Def::Type(_) => CompletionKind::Type,
        Def::Trait(_) => CompletionKind::Trait,
        Def::Variant(..) => CompletionKind::Variant,
        Def::Module(_) => CompletionKind::Module,
    }
}

/// The names in scope at the placeholder at `at`: the locals whose scope holds it, the innermost
/// of each name, then the module's names and the prelude's.
fn scope(db: &Database, program: Program, file: u32, at: u32) -> Vec<Completion> {
    let items = moddil_sema::item_tree(db, program);
    let from = module_of(items, file);
    let mut out = Vec::new();
    if let Some((owner, _)) = placeholder_expr(db, program, file, at) {
        let body = &moddil_sema::body(db, owner).body;
        let types = moddil_sema::infer(db, owner);
        let text = program.files(db)[file as usize].source.text(db).clone();
        let nodes = moddil_syntax::parse(&text).syntax().descendants();
        // The latest binding of a name first, so it is the one kept.
        let mut seen: Vec<String> = Vec::new();
        let mut locals: Vec<_> = body.locals.iter().collect();
        locals.sort_by_key(|(_, local)| std::cmp::Reverse(local.range.start));
        for (l, local) in locals {
            let visible = local.range.end <= at
                && local_scope(&nodes, local.range).is_none_or(|(s, e)| s <= at && at <= e);
            if visible
                && local.name != "_"
                && !local.name.starts_with('$')
                && !seen.contains(&local.name)
            {
                seen.push(local.name.clone());
                out.push(Completion {
                    label: local.name.clone(),
                    kind: CompletionKind::Local,
                    detail: types.local(l).to_string(),
                });
            }
        }
    }
    out.extend(module_names(db, program, items, from, from));
    out.extend(module_names(db, program, items, PRELUDE_MODULE, from));
    out
}

/// Where the local bound at `bind` can be named, of the file's `nodes`: from the end of its
/// `let` to the end of the block it is in, or its match arm, closure, loop or `if let`, or its
/// function; `None` if it is none of those (then the whole body).
fn local_scope(nodes: &[SyntaxNode], bind: TextRange) -> Option<(u32, u32)> {
    let node = nodes
        .iter()
        .filter(|n| {
            let r = n.text_range();
            r.start <= bind.start && bind.end <= r.end
        })
        .min_by_key(|n| n.text_range().len())?;
    for a in std::iter::once(node.clone()).chain(node.ancestors()) {
        let r = a.text_range();
        match a.kind() {
            K::LET_STMT => {
                let block = a.parent()?.text_range();
                return Some((r.end, block.end));
            }
            K::MATCH_ARM
            | K::SELECT_ARM
            | K::CLOSURE_EXPR
            | K::FOR_EXPR
            | K::WHILE_EXPR
            | K::IF_EXPR
            | K::FN_ITEM
            | K::TEST_ITEM => return Some((r.start, r.end)),
            _ => {}
        }
    }
    None
}
