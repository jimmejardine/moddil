//! Value paths: what a name or `a::b` refers to (a local, a function, a constructor, an
//! associated or trait function), with its type arguments.

use moddil_syntax::{SyntaxKind as K, SyntaxNode, TextRange};

use super::{Lowerer, Resolved};
use crate::hir::{ExprId, ExprKind};
use crate::items::{Def, Shape};
use crate::modules::PathError;
use crate::resolve::{adt_args, generic_args, path_segments};
use crate::{Code, Diagnostic, Ty};

impl Lowerer<'_> {
    /// The segments of the `PATH` in `node` (without generic arguments), or `None` after
    /// reporting a path this slice does not support.
    pub(super) fn segments(&mut self, node: &SyntaxNode) -> Option<Vec<String>> {
        let segs = node.child(K::PATH).and_then(|p| path_segments(&p));
        match segs {
            Some(segs) if !segs.is_empty() => Some(segs.into_iter().map(|s| s.name).collect()),
            _ => {
                self.error(Diagnostic::unsupported(node.text_range(), "this path"));
                None
            }
        }
    }

    /// The type arguments of struct or enum `adt` written on a segment, or holes.
    pub(super) fn adt_args(
        &mut self,
        adt: u32,
        args: Option<&SyntaxNode>,
        range: TextRange,
    ) -> Vec<Ty> {
        let mut ds = Vec::new();
        let args = adt_args(adt, args, range, &self.types, &mut ds);
        self.out.diagnostics.extend(ds);
        args
    }

    /// What the value path in `node` names, or `None` after reporting why it names nothing.
    pub(super) fn resolve(&mut self, node: &SyntaxNode) -> Option<Resolved> {
        let range = node.text_range();
        let Some(segs) = node.child(K::PATH).and_then(|p| path_segments(&p)) else {
            self.error(Diagnostic::unsupported(range, "this path"));
            return None;
        };
        if let [seg] = segs.as_slice()
            && let Some(local) = self.lookup(&seg.name)
        {
            if seg.args.is_some() {
                self.report(
                    Code::E0404,
                    range,
                    format!("`{}` takes no type arguments", seg.name),
                );
            }
            return Some(Resolved::Local(local));
        }
        // `Self::f`, `T::f`, `int::f`: a function of a type that is not an item.
        if let [ty, item] = segs.as_slice()
            && let Some(base) = self.non_item_type(ty)
        {
            let base = base?;
            return self.of_type(base, item);
        }
        let names: Vec<&str> = segs.iter().map(|s| s.name.as_str()).collect();
        let (def, n) = match self
            .items
            .path(self.types.module, &names, self.types.prelude_first)
        {
            Ok(found) => found,
            Err(PathError::Private(k, def)) => {
                let d = self.items.hidden(def, segs[k].range);
                self.error(d);
                return None;
            }
            Err(PathError::Unresolved(k)) => {
                let name = names[k];
                if segs.len() == 1 && name == "Self" {
                    if let Some(Ty::Adt(a, _, args)) = self.types.self_ty.clone()
                        && !self.items.adts[a as usize].is_enum
                    {
                        return Some(Resolved::Ctor(a, 0, args.to_vec()));
                    }
                } else if segs.len() == 1 && name == "assert" {
                    self.report(
                        Code::E0112,
                        range,
                        "`assert` is a built-in, not a value: call it as `assert(condition)`",
                    );
                    return None;
                }
                let what = if k == 0 && segs.len() == 1 {
                    format!("there is nothing called `{name}` here")
                } else if k == 0 {
                    format!("there is no type or module called `{name}`")
                } else {
                    format!(
                        "there is nothing called `{name}` in `{}`",
                        names[..k].join("::")
                    )
                };
                self.report(Code::E0101, segs[k].range, what);
                return None;
            }
        };
        let last = &segs[segs.len() - 1];
        let rest = &segs[n..];
        match (def, rest) {
            (Def::Fn(f), []) => {
                let sig = &self.items.fns[f as usize];
                let want = sig.generics.params.len() - sig.generics.parent as usize;
                let args = match &last.args {
                    Some(a) => self.type_args(a, want, &format!("`{}`", last.name)),
                    None => vec![Ty::Infer(Ty::HOLE); want],
                };
                Some(Resolved::Fn(f, args))
            }
            (Def::Variant(a, v), []) => {
                let args = self.adt_args(a, last.args.as_ref(), range);
                Some(Resolved::Ctor(a, v, args))
            }
            (Def::Adt(a), []) => {
                if !self.items.adts[a as usize].is_enum {
                    let args = self.adt_args(a, last.args.as_ref(), range);
                    return Some(Resolved::Ctor(a, 0, args));
                }
                let name = &self.items.adts[a as usize].name;
                self.report(
                    Code::E0111,
                    range,
                    format!("`{name}` is an enum; name one of its variants: `{name}::…`"),
                );
                None
            }
            // `X::new` of a refinement (`refine.new.1`).
            (Def::Type(t), [item]) => {
                let def = &self.items.types[t as usize];
                match &def.refinement {
                    Some(r) if item.name == "new" => Some(Resolved::Fn(r.new, Vec::new())),
                    _ => {
                        let name = def.name.clone();
                        self.report(
                            Code::E0413,
                            item.range,
                            format!("`{name}` has nothing called `{}`", item.name),
                        );
                        None
                    }
                }
            }
            // `Type::item`: an enum's variant, or a function of the type.
            (Def::Adt(a), [item]) => {
                let ty_seg = &segs[n - 1];
                let args = self.adt_args(a, ty_seg.args.as_ref(), ty_seg.range);
                let base = self.items.adts[a as usize].ty_with(a, args);
                self.of_type(base, item)
            }
            // `Trait::method`.
            (Def::Trait(t), [item]) => {
                let ty_seg = &segs[n - 1];
                let def = &self.items.traits[t as usize];
                let Some(&func) = def
                    .methods
                    .iter()
                    .find(|&&m| self.items.fns[m as usize].name == item.name)
                else {
                    self.report(
                        Code::E0413,
                        item.range,
                        format!("`{}` has no method `{}`", ty_seg.name, item.name),
                    );
                    return None;
                };
                let want = def.generics.params.len() - 1;
                let trait_args = match &ty_seg.args {
                    Some(a) => self.type_args(a, want, &format!("`{}`", ty_seg.name)),
                    None => vec![Ty::Infer(Ty::HOLE); want],
                };
                let args = self.own_args(item);
                Some(Resolved::TraitFn(t, func, trait_args, args))
            }
            (Def::Module(_), []) => {
                self.report(
                    Code::E0101,
                    range,
                    format!("`{}` is a module, not a value", names.join("::")),
                );
                None
            }
            (Def::Trait(_), []) => {
                self.report(
                    Code::E0101,
                    range,
                    format!("`{}` is a trait, not a value", names.join("::")),
                );
                None
            }
            (def, _) => {
                self.report(
                    Code::E0101,
                    segs[n].range,
                    format!(
                        "`{}` has nothing called `{}`",
                        self.items.def_name(def),
                        names[n]
                    ),
                );
                None
            }
        }
    }

    /// For `ty::item` where `ty` is not an item: `Self`, a type parameter or a built-in type
    /// (`Some(None)` after reporting a `Self` with no type), or `None` if it is an item's name.
    fn non_item_type(&mut self, ty: &crate::resolve::Segment) -> Option<Option<Ty>> {
        let name = ty.name.as_str();
        Some(Some(match name {
            "int" | "i64" => Ty::Int,
            n if crate::ty::Num::from_name(n).is_some() => {
                Ty::Num(crate::ty::Num::from_name(n).expect("a number type"))
            }
            "bool" => Ty::Bool,
            "char" => Ty::Char,
            "str" => Ty::Str,
            "Buf" => Ty::Buf(std::sync::Arc::new(crate::resolve::hole())),
            "Self" => match &self.types.self_ty {
                Some(t) => t.clone(),
                None => {
                    self.report(
                        Code::E0102,
                        ty.range,
                        "`Self` is the type of an impl or trait, and there is none here",
                    );
                    return Some(None);
                }
            },
            _ => {
                let i = self.types.params.iter().rposition(|p| p.name == name)?;
                Ty::Param(i as u32, name.into())
            }
        }))
    }

    /// The type arguments written on `item`'s segment, if any.
    fn own_args(&mut self, item: &crate::resolve::Segment) -> Option<Vec<Ty>> {
        item.args.as_ref().map(|a| {
            let mut ds = Vec::new();
            let (types, _) = generic_args(a, &self.types, &mut ds);
            self.out.diagnostics.extend(ds);
            types
        })
    }

    /// `base::item`: an enum's variant, or a function of the type.
    fn of_type(&mut self, base: Ty, item: &crate::resolve::Segment) -> Option<Resolved> {
        if let Ty::Adt(a, _, args) = &base {
            let def = &self.items.adts[*a as usize];
            if def.is_enum
                && let Some(v) = def.variant(&item.name)
            {
                if item.args.is_some() {
                    self.report(
                        Code::E0404,
                        item.range,
                        "a variant takes no type arguments: write them on the enum",
                    );
                }
                return Some(Resolved::Ctor(*a, v, args.to_vec()));
            }
        }
        let args = self.own_args(item);
        Some(Resolved::AssocFn(base, item.name.clone(), item.range, args))
    }

    /// The expression for a resolved path.
    /// A function of the `core` mod's `os` module is named only in a mod whose manifest lists
    /// the `os` capability (`os.cap.1`).
    fn check_os(&mut self, func: u32, range: TextRange) {
        let m = &self.items.modules[self.items.fns[func as usize].module as usize];
        // `core::ffi` only with the `unsafe` capability (`ffi.core.1`).
        if m.path == ["ffi"]
            && self
                .items
                .mods
                .get(m.mod_ as usize)
                .is_some_and(|d| d.name == "core")
        {
            let here = &self.items.modules[self.types.module as usize];
            if !self
                .items
                .mods
                .get(here.mod_ as usize)
                .is_none_or(|d| d.unsafe_ok)
            {
                self.report(
                    Code::E0219,
                    range,
                    "`core::ffi` is named only in a mod whose manifest lists the `unsafe`                      capability: `capabilities = [\"unsafe\"]` under `[mod]`",
                );
            }
            return;
        }
        let in_os = (m.path == ["os"] || m.path == ["gpu"])
            && self
                .items
                .mods
                .get(m.mod_ as usize)
                .is_some_and(|d| d.name == "core");
        if !in_os {
            return;
        }
        let here = &self.items.modules[self.types.module as usize];
        let allowed = self
            .items
            .mods
            .get(here.mod_ as usize)
            .is_none_or(|d| d.os_ok);
        if !allowed {
            self.report(
                Code::E0220,
                range,
                "`core::os` and `core::gpu` are named only in a mod whose manifest lists the \
                 capability: `capabilities = [\"os\"]` under `[mod]`; an application reaches \
                 the OS through the root capabilities `main` takes",
            );
        }
    }

    pub(super) fn resolved(&mut self, r: Resolved, range: TextRange) -> ExprId {
        if let Resolved::Fn(func, _) = &r {
            self.check_os(*func, range);
        }
        match r {
            Resolved::Local(l) => self.alloc(ExprKind::Local(l), range),
            // A const item's value: a call of it.
            Resolved::Fn(func, args) if self.items.fns[func as usize].const_item => {
                let callee = self.alloc(ExprKind::Fn { func, args }, range);
                self.alloc(
                    ExprKind::Call {
                        callee,
                        args: Vec::new(),
                    },
                    range,
                )
            }
            Resolved::Fn(func, args) => self.alloc(ExprKind::Fn { func, args }, range),
            Resolved::AssocFn(ty, name, name_range, args) => self.alloc(
                ExprKind::AssocFn {
                    ty,
                    name,
                    name_range,
                    args,
                },
                range,
            ),
            Resolved::TraitFn(trait_, func, trait_args, args) => self.alloc(
                ExprKind::TraitFn {
                    trait_,
                    func,
                    trait_args,
                    args,
                },
                range,
            ),
            Resolved::Ctor(adt, variant, args) => {
                let def = self.items.variant(adt, variant);
                match def.shape {
                    Shape::Unit => self.alloc(
                        ExprKind::Adt {
                            adt,
                            variant,
                            fields: Vec::new(),
                            args,
                        },
                        range,
                    ),
                    Shape::Tuple => self.unsupported(range, "a constructor used as a value"),
                    Shape::Named => {
                        let path = self.items.adts[adt as usize].path(variant);
                        self.report(
                            Code::E0111,
                            range,
                            format!("`{path}` has named fields: write `{path} {{ … }}`"),
                        );
                        self.missing(range)
                    }
                }
            }
        }
    }
}
