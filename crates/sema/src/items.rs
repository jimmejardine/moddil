//! The items of a program and their signatures: the prelude's, the file's, and those its derives
//! expand to. [`item_tree`](crate::item_tree) builds them (`collect.rs`).

use moddil_syntax::TextRange;

use crate::ty::TraitRef;
use crate::{Diagnostic, Ty};

/// Where an item's source is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Src {
    /// The prelude (`reference/2027/prelude.moddil`).
    Prelude,
    /// The file being compiled.
    File,
    /// The impls the derives of the prelude and the file expand to.
    Derived,
}

/// A program's items: the prelude's first, then the file's, then the derived impls. Items refer
/// to each other by index.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct ItemTree {
    /// The functions, including methods of traits and impls, in source order.
    pub fns: Vec<FnSig>,
    /// The structs and enums.
    pub adts: Vec<AdtDef>,
    /// The traits.
    pub traits: Vec<TraitDef>,
    /// The impls.
    pub impls: Vec<ImplDef>,
    /// The file's tests, in source order.
    pub tests: Vec<TestSig>,
    /// Problems found in the items' headers, in the file.
    pub diagnostics: Vec<Diagnostic>,
    /// Problems found in the prelude and its derived impls, which a correct prelude has none of.
    pub hidden: Vec<Diagnostic>,
}

/// A type parameter.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct GenericParam {
    /// Its name.
    pub name: String,
    /// Where it is declared.
    pub range: TextRange,
}

/// The type parameters an item can name, and the bounds on them.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct Generics {
    /// The parameters: the enclosing impl's or trait's first (a trait's start with `Self`),
    /// then the item's own.
    pub params: Vec<GenericParam>,
    /// How many of `params` belong to the enclosing impl or trait.
    pub parent: u32,
    /// The bounds: `<T: Clone>`, `where` clauses, and in a trait, `Self: Trait`.
    pub preds: Vec<Pred>,
}

impl Generics {
    /// The parameters as types: `[T, U]`.
    pub fn identity(&self) -> Vec<Ty> {
        self.params
            .iter()
            .enumerate()
            .map(|(i, p)| Ty::Param(i as u32, p.name.as_str().into()))
            .collect()
    }
}

/// A bound: `ty` implements `trait_ref`, with some of its associated types fixed
/// (`T: Add<Output = int>`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Pred {
    /// The type bounded.
    pub ty: Ty,
    /// The trait.
    pub trait_ref: TraitRef,
    /// Associated types fixed by the bound: their index in the trait, and the type.
    pub assoc: Vec<(u32, Ty)>,
    /// Where the bound is written.
    pub range: TextRange,
}

/// What a function belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Container {
    /// Nothing: a free function.
    Free,
    /// An impl, by index.
    Impl(u32),
    /// A trait, by index.
    Trait(u32),
}

/// How a method takes `self`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SelfKind {
    /// `self` (or `mut self`): by value.
    Value,
    /// `&self`.
    Ref,
    /// `&mut self`.
    RefMut,
}

/// A function's signature.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FnSig {
    /// The name.
    pub name: String,
    /// Where the name is.
    pub name_range: TextRange,
    /// Which source it is in.
    pub src: Src,
    /// The range of its `fn` item in that source.
    pub node: TextRange,
    /// What it belongs to.
    pub container: Container,
    /// Its type parameters and bounds, with those of its impl or trait.
    pub generics: Generics,
    /// How a method takes `self`, which is then its first parameter.
    pub self_kind: Option<SelfKind>,
    /// The parameters (`self` first, for a method).
    pub params: Vec<ParamSig>,
    /// The return type (`()` when none is written).
    pub ret: Ty,
    /// Where the return type is written, if it is.
    pub ret_range: Option<TextRange>,
    /// The parameters a returned borrow comes from: those named by `from`, or else every
    /// borrowed parameter.
    pub ret_from: Vec<u32>,
    /// Whether it has a body (a trait's required methods do not).
    pub has_body: bool,
    /// For a derived method: the `derive` attribute it comes from, in the file.
    pub origin: Option<TextRange>,
    /// The errors it can fail with (`with fail(A | B)`), as an error set: `!` if it cannot fail.
    pub fails: Ty,
    /// Where its `with fail(…)` is written, if it is.
    pub fails_range: Option<TextRange>,
}

/// A parameter.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ParamSig {
    /// The name bound (`None` for `_` or a destructuring pattern).
    pub name: Option<String>,
    /// Whether it is declared `mut`.
    pub mutable: bool,
    /// Where the pattern is.
    pub range: TextRange,
    /// The declared type.
    pub ty: Ty,
    /// Whether the pattern is more than a name or `_`, so the body destructures it.
    pub destructures: bool,
}

/// A struct or an enum. A struct has exactly one variant, named like the struct.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AdtDef {
    /// The name.
    pub name: String,
    /// Where the name is.
    pub name_range: TextRange,
    /// Which source it is in.
    pub src: Src,
    /// Whether it is an enum.
    pub is_enum: bool,
    /// Whether it is declared `#[derive(Copy, Clone)]`.
    pub copy: bool,
    /// Its type parameters.
    pub generics: Generics,
    /// The variants, in order (one for a struct).
    pub variants: Vec<VariantDef>,
    /// Whether its variants are named without the enum's name (`Some`, `None`).
    pub bare_variants: bool,
}

/// One variant of an enum, or a struct's shape.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct VariantDef {
    /// The name.
    pub name: String,
    /// Where the name is.
    pub name_range: TextRange,
    /// Named fields `{ x: int }`, positional fields `(int, int)`, or none.
    pub shape: Shape,
    /// The fields, in order; positional fields are named `0`, `1`, ….
    pub fields: Vec<FieldDef>,
}

/// How a struct or variant writes its fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Shape {
    /// `{ x: int, y: int }`.
    Named,
    /// `(int, int)`.
    Tuple,
    /// No fields: `struct Unit;`, `Colour::Red`.
    Unit,
}

/// A field.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FieldDef {
    /// The name (`0`, `1`, … for positional fields).
    pub name: String,
    /// Where it is declared.
    pub range: TextRange,
    /// Its type, in terms of the struct's or enum's type parameters.
    pub ty: Ty,
}

/// A trait.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TraitDef {
    /// The name.
    pub name: String,
    /// Where the name is.
    pub name_range: TextRange,
    /// Which source it is in.
    pub src: Src,
    /// `Self` (parameter 0) and the trait's own parameters, with its bounds.
    pub generics: Generics,
    /// The supertraits, in terms of the trait's parameters (`Self` is parameter 0).
    pub supertraits: Vec<TraitRef>,
    /// The associated types' names and where they are declared.
    pub assoc: Vec<(String, TextRange)>,
    /// The methods, by index among the functions.
    pub methods: Vec<u32>,
}

/// An impl: inherent (`impl Point { … }`) or of a trait (`impl Clone for Point { … }`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ImplDef {
    /// Which source it is in.
    pub src: Src,
    /// The range of its header, from `impl` to the self type.
    pub range: TextRange,
    /// Its type parameters and bounds.
    pub generics: Generics,
    /// The trait implemented, if any.
    pub trait_ref: Option<TraitRef>,
    /// The type it is for.
    pub self_ty: Ty,
    /// Its associated types, by their index in the trait (`None` if missing).
    pub assoc: Vec<Option<Ty>>,
    /// The methods, by index among the functions.
    pub methods: Vec<u32>,
    /// For a derived impl: the `derive` attribute it comes from, in the file.
    pub origin: Option<TextRange>,
}

/// A test.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TestSig {
    /// The test's name, without quotes.
    pub name: String,
    /// Where the name is.
    pub name_range: TextRange,
}

impl VariantDef {
    /// The index of the field called `name`.
    pub fn field(&self, name: &str) -> Option<u32> {
        self.fields
            .iter()
            .position(|f| f.name == name)
            .map(|i| i as u32)
    }
}

impl AdtDef {
    /// The index of the variant called `name`.
    pub fn variant(&self, name: &str) -> Option<u32> {
        self.variants
            .iter()
            .position(|v| v.name == name)
            .map(|i| i as u32)
    }

    /// The type with these type arguments, given the struct's or enum's index.
    pub fn ty_with(&self, index: u32, args: Vec<Ty>) -> Ty {
        Ty::Adt(index, self.name.as_str().into(), args.into())
    }

    /// The type with its own parameters as arguments (`Option<T>`), given its index.
    pub fn ty(&self, index: u32) -> Ty {
        self.ty_with(index, self.generics.identity())
    }

    /// How variant `v` is written: `Point` for a struct, `Shape::Circle` for an enum's variant,
    /// `Some` for a variant named on its own.
    pub fn path(&self, v: u32) -> String {
        if self.bare_variants {
            self.variants[v as usize].name.clone()
        } else if self.is_enum {
            format!("{}::{}", self.name, self.variants[v as usize].name)
        } else {
            self.name.clone()
        }
    }
}

/// The traits, types and functions of the prelude the language itself refers to.
const LANG: &[&str] = &[
    "Option",
    "Result",
    "Ordering",
    "Clone",
    "Copy",
    "PartialEq",
    "Eq",
    "PartialOrd",
    "Ord",
    "Add",
    "Sub",
    "Mul",
    "Div",
    "Rem",
    "Neg",
    "Not",
    "Drop",
];

impl ItemTree {
    /// The free function called `name`.
    pub fn fn_named(&self, name: &str) -> Option<u32> {
        self.fns
            .iter()
            .position(|f| f.name == name && f.container == Container::Free)
            .map(|i| i as u32)
    }

    /// The struct or enum called `name`.
    pub fn adt_named(&self, name: &str) -> Option<u32> {
        self.adts
            .iter()
            .position(|a| a.name == name)
            .map(|i| i as u32)
    }

    /// The trait called `name`.
    pub fn trait_named(&self, name: &str) -> Option<u32> {
        self.traits
            .iter()
            .position(|t| t.name == name)
            .map(|i| i as u32)
    }

    /// The variant named on its own (`Some`) called `name`: its enum and index.
    pub fn bare_variant(&self, name: &str) -> Option<(u32, u32)> {
        self.adts.iter().enumerate().find_map(|(i, a)| {
            if a.bare_variants {
                a.variant(name).map(|v| (i as u32, v))
            } else {
                None
            }
        })
    }

    /// The variant of struct or enum `adt`.
    pub fn variant(&self, adt: u32, variant: u32) -> &VariantDef {
        &self.adts[adt as usize].variants[variant as usize]
    }

    /// The types of the fields of a variant of `ty` (a struct or enum with its arguments).
    pub fn field_types(&self, ty: &Ty, variant: u32) -> Vec<Ty> {
        match ty {
            Ty::Adt(a, _, args) => self
                .variant(*a, variant)
                .fields
                .iter()
                .map(|f| f.ty.subst(args))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// The prelude item the language refers to by `name` (`"Copy"`, `"Option"`): a trait's or
    /// a struct's or enum's index.
    pub fn lang(&self, name: &str) -> Option<u32> {
        debug_assert!(LANG.contains(&name), "not a language item: {name}");
        if let Some(t) = self.trait_named(name) {
            return (self.traits[t as usize].src == Src::Prelude).then_some(t);
        }
        self.adt_named(name)
            .filter(|&a| self.adts[a as usize].src == Src::Prelude)
    }

    /// The prelude trait `name`, as a trait with no arguments.
    pub fn lang_ref(&self, name: &str) -> Option<TraitRef> {
        let t = self.lang(name)?;
        Some(TraitRef {
            trait_: t,
            name: name.into(),
            args: std::sync::Arc::from([]),
        })
    }

    /// The prelude's free function `drop`.
    pub fn drop_fn(&self) -> Option<u32> {
        self.fns
            .iter()
            .position(|f| {
                f.name == "drop" && f.src == Src::Prelude && f.container == Container::Free
            })
            .map(|i| i as u32)
    }

    /// Whether values of `ty` are copied rather than moved, with no bounds in scope.
    pub fn is_copy(&self, ty: &Ty) -> bool {
        crate::traits::Solver::new(self, &[]).is_copy(ty)
    }

    /// The size and alignment of `ty` in bytes, as the `Copy` cap counts them: `int` and borrows
    /// are 8, `bool` 1, `()` 0; tuples and structs lay their fields out in order, each at a
    /// multiple of its alignment, rounded up to the largest alignment; an enum is an 8-byte tag
    /// followed by its largest variant. A type parameter counts as nothing.
    pub fn size_of(&self, ty: &Ty) -> (u64, u64) {
        self.size_of_depth(ty, 0)
    }

    fn size_of_depth(&self, ty: &Ty, depth: u32) -> (u64, u64) {
        let layout = |fields: &[Ty]| -> (u64, u64) {
            let mut size: u64 = 0;
            let mut align: u64 = 1;
            for f in fields {
                let (s, a) = self.size_of_depth(f, depth + 1);
                size = size.div_ceil(a) * a + s;
                align = align.max(a);
            }
            (size.div_ceil(align) * align, align)
        };
        if depth > 32 {
            return (0, 1);
        }
        match ty {
            Ty::Int | Ty::Ref(..) => (8, 8),
            Ty::Bool => (1, 1),
            Ty::Unit
            | Ty::Never
            | Ty::Error
            | Ty::Param(..)
            | Ty::Assoc(_)
            | Ty::Infer(_)
            | Ty::Errors(_) => (0, 1),
            Ty::Tuple(ts) => layout(ts),
            Ty::Adt(a, _, _) => {
                let def = &self.adts[*a as usize];
                let shapes: Vec<(u64, u64)> = (0..def.variants.len() as u32)
                    .map(|v| layout(&self.field_types(ty, v)))
                    .collect();
                if def.is_enum {
                    let payload = shapes.iter().map(|s| s.0).max().unwrap_or(0);
                    (8 + payload.div_ceil(8) * 8, 8)
                } else {
                    shapes.first().copied().unwrap_or((0, 1))
                }
            }
        }
    }
}

/// The largest a `Copy` type may be, in bytes: one cache line.
pub(crate) const COPY_CAP: u64 = 64;
