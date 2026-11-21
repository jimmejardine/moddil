//! Modules (`modules.md`): each `use` names something the module can see, and an item names
//! nothing less visible than itself in its signature.

use moddil_syntax::TextRange;

use super::Found;
use crate::items::{Container, Def, ItemTree, Src, Vis};
use crate::modules::PathError;
use crate::{Code, Diagnostic, Ty};

/// The `use`s that name nothing (E0702) or something the module cannot see (E0701).
pub(crate) fn imports(tree: &ItemTree) -> Vec<Found> {
    let mut found = Vec::new();
    for (m, module) in tree.modules.iter().enumerate() {
        let Some(file) = module.file else { continue };
        let src = Src::File(file);
        for i in &module.imports {
            let segs: Vec<&str> = i.path.iter().map(|(s, _)| s.as_str()).collect();
            let at = |k: usize| i.path.get(k).map_or(i.range, |(_, r)| *r);
            let d = match tree.path(m as u32, &segs, false) {
                Ok((_, n)) if n == segs.len() => continue,
                Ok((Def::Adt(a), n))
                    if n + 1 == segs.len() && tree.adts[a as usize].variant(segs[n]).is_some() =>
                {
                    continue;
                }
                Ok((def, n)) => Diagnostic::error(
                    Code::E0702,
                    at(n),
                    format!("`{}` has nothing called `{}`", tree.def_name(def), segs[n]),
                ),
                Err(PathError::Unresolved(0)) => Diagnostic::error(
                    Code::E0702,
                    at(0),
                    format!(
                        "there is nothing called `{}` here: a path starts at `crate`, a \
                         dependency, or a name in scope",
                        segs[0]
                    ),
                ),
                Err(PathError::Unresolved(k)) => Diagnostic::error(
                    Code::E0702,
                    at(k),
                    format!(
                        "there is nothing called `{}` in `{}`",
                        segs[k],
                        segs[..k].join("::")
                    ),
                ),
                Err(PathError::Private(k, def)) => tree.hidden(def, at(k)),
            };
            found.push((src, None, d));
        }
    }
    found
}

/// The items whose signatures name something less visible than they are (E0703): a `pub`
/// function's parameter, result, error or effect types, a `pub` struct's or enum's field types.
pub(crate) fn exposed(tree: &ItemTree) -> Vec<Found> {
    let mut found = Vec::new();
    for f in &tree.fns {
        if !f.src.is_file()
            || f.vis == Vis::Private
            || !matches!(f.container, Container::Free | Container::Impl(_))
        {
            continue;
        }
        let mut types: Vec<&Ty> = f.params.iter().map(|p| &p.ty).collect();
        types.push(&f.ret);
        types.push(&f.fails);
        types.extend(f.effects.iter().map(|e| &e.ty));
        let traits: Vec<Def> = f
            .generics
            .preds
            .iter()
            .map(|p| Def::Trait(p.trait_ref.trait_))
            .collect();
        if let Some(d) = less_visible(
            tree,
            f.vis,
            f.module,
            &types,
            &traits,
            &f.name,
            f.name_range,
        ) {
            found.push((f.src, f.origin, d));
        }
    }
    for a in &tree.adts {
        if !a.src.is_file() || a.vis == Vis::Private {
            continue;
        }
        let types: Vec<&Ty> = a
            .variants
            .iter()
            .flat_map(|v| &v.fields)
            .map(|f| &f.ty)
            .collect();
        if let Some(d) = less_visible(tree, a.vis, a.module, &types, &[], &a.name, a.name_range) {
            found.push((a.src, None, d));
        }
    }
    found
}

/// E0703 for an item of module `module` that `vis` describes, if one of `types` or `traits` is
/// less visible than it.
fn less_visible(
    tree: &ItemTree,
    vis: Vis,
    module: u32,
    types: &[&Ty],
    traits: &[Def],
    name: &str,
    at: TextRange,
) -> Option<Diagnostic> {
    let mut named = traits.to_vec();
    for t in types {
        defs_in(t, &mut named);
    }
    for def in named {
        let (their, owner) = match def {
            Def::Adt(a) => (tree.adts[a as usize].vis, tree.adts[a as usize].module),
            Def::Trait(t) => (tree.traits[t as usize].vis, tree.traits[t as usize].module),
            _ => continue,
        };
        // Private < `pub(mod)` < `pub`.
        let less = their < vis;
        // An item of another module that the item's module cannot see is caught by naming it.
        if less && (owner == module || tree.mod_of(owner) == tree.mod_of(module)) {
            let what = match vis {
                Vis::Pub => "`pub`",
                _ => "`pub(mod)`",
            };
            return Some(Diagnostic::error(
                Code::E0703,
                at,
                format!(
                    "`{name}` is {what}, but its signature names `{}`, which fewer modules can \
                     name: make `{}` as visible, or `{name}` less so",
                    tree.def_name(def),
                    tree.def_name(def)
                ),
            ));
        }
    }
    None
}

/// The structs, enums and traits `t` names.
fn defs_in(t: &Ty, out: &mut Vec<Def>) {
    match t {
        Ty::Adt(a, ..) => out.push(Def::Adt(*a)),
        Ty::Dyn(tr) => out.push(Def::Trait(tr.trait_)),
        _ => {}
    }
    let _ = t.map(&mut |x| {
        defs_in(x, out);
        x.clone()
    });
}

impl ItemTree {
    /// E0701 at `range`: `def` is not visible from where it is named.
    pub(crate) fn hidden(&self, def: Def, range: TextRange) -> Diagnostic {
        let (vis, module) = match def {
            Def::Fn(f) => (self.fns[f as usize].vis, self.fns[f as usize].module),
            Def::Adt(a) | Def::Variant(a, _) => {
                (self.adts[a as usize].vis, self.adts[a as usize].module)
            }
            Def::Trait(t) => (self.traits[t as usize].vis, self.traits[t as usize].module),
            Def::Module(m) => (Vis::Pub, m),
        };
        let name = self.def_name(def);
        let why = match vis {
            Vis::Mod => "`pub(mod)`, so only the modules of its mod can name it".to_string(),
            _ => format!("private to `{}`", self.module_path(module)),
        };
        Diagnostic::error(Code::E0701, range, format!("`{name}` is {why}"))
    }
}
