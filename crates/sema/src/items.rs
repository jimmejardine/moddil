//! The items of a program and their signatures: the prelude's, the file's, and those its derives
//! expand to. [`item_tree`](crate::item_tree) builds them (`collect.rs`).

use moddil_syntax::TextRange;

use crate::ty::TraitRef;
use crate::{Diagnostic, Ty};

/// Where an item's source is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Src {
    /// The prelude (`reference/2027/prelude/`).
    Prelude,
    /// A file of the program, by its index.
    File(u32),
    /// The impls the derives of the prelude and the file expand to.
    Derived,
}

impl Src {
    /// The file, if the source is one.
    pub fn file(self) -> Option<u32> {
        match self {
            Src::File(f) => Some(f),
            _ => None,
        }
    }

    /// Whether the source is one of the program's files.
    pub fn is_file(self) -> bool {
        matches!(self, Src::File(_))
    }
}

/// A program's items: the prelude's first, then the file's, then the derived impls. Items refer
/// to each other by index.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct ItemTree {
    /// The functions, including methods of traits and impls, in source order.
    pub fns: Vec<FnSig>,
    /// The structs and enums.
    pub adts: Vec<AdtDef>,
    /// The `type` items: aliases and refinements.
    pub types: Vec<TypeDef>,
    /// The traits.
    pub traits: Vec<TraitDef>,
    /// The impls.
    pub impls: Vec<ImplDef>,
    /// The program's tests, in the order of its files.
    pub tests: Vec<TestSig>,
    /// The modules: the prelude's first ([`PRELUDE_MODULE`]), then each mod's.
    pub modules: Vec<ModuleDef>,
    /// The mods, in the program's order: the first is the one being built.
    pub mods: Vec<ModDef>,
    /// Problems found in the items' headers, in the program's files.
    pub diagnostics: Vec<Diagnostic>,
    /// Problems found in the prelude and its derived impls, which a correct prelude has none of.
    pub hidden: Vec<Diagnostic>,
}

/// The prelude's module, the first.
pub const PRELUDE_MODULE: u32 = 0;

/// The mod the prelude's module belongs to: none of the program's.
pub const PRELUDE_MOD: u32 = u32::MAX;

/// Who can name an item (`modules.md`: Three visibility levels).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Vis {
    /// Its own module only: the default.
    Private,
    /// Every module of its mod: `pub(mod)`.
    Mod,
    /// Every mod: `pub`.
    Pub,
}

/// What a name in a module stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Def {
    /// A free function.
    Fn(u32),
    /// A struct or enum.
    Adt(u32),
    /// A `type` item: an alias or a refinement.
    Type(u32),
    /// A trait.
    Trait(u32),
    /// A variant named on its own (`Some`): its enum and index.
    Variant(u32, u32),
    /// A module.
    Module(u32),
}

/// A name a module declares: its item, who can name it, and where it is declared.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Entry {
    /// What it stands for.
    pub def: Def,
    /// Who can name it.
    pub vis: Vis,
    /// Where it is declared.
    pub range: TextRange,
}

/// A `use` of one name: `use mod::a::b as c;` imports `b` as `c`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Import {
    /// The name it brings in.
    pub name: String,
    /// The path it names, each segment with where it is.
    pub path: Vec<(String, TextRange)>,
    /// Who can name it through this module: a `pub use` re-exports it.
    pub vis: Vis,
    /// Where the imported name is written.
    pub range: TextRange,
}

/// A module: a file of a mod, or a directory of them, or the prelude.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ModuleDef {
    /// The mod it belongs to ([`PRELUDE_MOD`] for the prelude).
    pub mod_: u32,
    /// Its path in its mod (empty for the root module).
    pub path: Vec<String>,
    /// Its file, if it has one (a directory of modules may not).
    pub file: Option<u32>,
    /// Whether it is an executable's main module.
    pub exe: bool,
    /// The items it declares, by name.
    pub items: std::collections::BTreeMap<String, Entry>,
    /// Its imports.
    pub imports: Vec<Import>,
    /// The modules inside it, by name.
    pub children: std::collections::BTreeMap<String, u32>,
}

/// A mod: its name, root module and dependencies.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ModDef {
    /// Its name.
    pub name: String,
    /// Its root module (`mod`).
    pub root: u32,
    /// The mods it depends on, by the name it calls them.
    pub deps: std::collections::BTreeMap<String, u32>,
    /// Whether it is a stub, whose functions may be signatures only (`mod.stub.1`).
    pub stub: bool,
    /// Whether it may use `unsafe` blocks (`mod.caps.1`).
    pub unsafe_ok: bool,
    /// Whether it may name the `core` mod's `os` module (`os.cap.1`).
    pub os_ok: bool,
}

/// A type parameter.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct GenericParam {
    /// Its name.
    pub name: String,
    /// Where it is declared.
    pub range: TextRange,
    /// Whether it is a const parameter (`const N: int`), whose arguments are lengths
    /// ([`Ty::Len`]) rather than types.
    pub konst: bool,
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
    /// For a closure bound, the control effects it lists (`with suspend`): the bounded closure's
    /// are then its caller's, not where it is written (`expr.closure.control.1`).
    pub control: Control,
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
    /// The effects its `with` clause lists, other than `fail`, in order.
    pub effects: Vec<EffectSig>,
    /// Whether it is `#[track_caller]`: a trap in it is reported where its caller called it.
    pub track_caller: bool,
    /// The control effects it lists (`with suspend, spawn`).
    pub control: Control,
    /// For each parameter with a default (`port: u16 = 443`): the hidden const that is its
    /// value (`expr.call.default.1`).
    pub defaults: Vec<Option<u32>>,
    /// For a result type `impl Trait` (`type.impl.ret.1`): its bounds, on the opaque type with
    /// the function's own type parameters as its arguments.
    pub opaque: Vec<Pred>,
    /// For a result type `impl Fn(A) -> R with …` (`expr.closure.result.1`): the closure's type
    /// as a stored one's, `dyn Fn(A) -> R with …`, which says what a call of it needs.
    pub opaque_fn: Option<Ty>,
    /// Whether it is marked `#[selectable]`: a `select` arm may wait on it (`task.select.3`).
    pub selectable: bool,
    /// Whether it is marked `#[kernel]`: a GPU kernel, one instance per tile (`gpu.kernel.1`).
    pub kernel: bool,
    /// Whether it is a C function, declared in an `extern "C"` block (`ffi.extern.1`): its name
    /// is its symbol, and it has no body.
    pub foreign: bool,
    /// Whether it is marked `#[export]`: callable from C under its own name (`ffi.export.1`).
    pub export: bool,
    /// `#[export(status)]`: an export whose trap makes it give 2 rather than end the process
    /// (`ffi.export.2`), for generated bindings.
    pub export_status: bool,
    /// `#[export(python)]`: a function of the mod's Python module (`ffi.python.1`).
    pub py_export: bool,
    /// Whether it is such a hidden const, whose body is the default written at `node`.
    pub default_arg: bool,
    /// Whether it is the prelude's and built in (`#[intrinsic]`): it has no body, and the
    /// implementations run it themselves.
    pub intrinsic: bool,
    /// Whether it may run at compile time: a `const fn`, or a const item.
    pub is_const: bool,
    /// For the hidden functions of a refinement (`type X = T where p;`): its index among the
    /// `type` items, and whether this is `X::new` (or else `X::holds`, the predicate).
    pub refine: Option<(u32, bool)>,
    /// Whether it is a const item (`const N: T = e;`), kept as a function with no parameters
    /// whose body is the initialiser: a use of `N` calls it, and its value is computed when
    /// compiling.
    pub const_item: bool,
    /// Who can name it.
    pub vis: Vis,
    /// The module it is declared in (for a derived method, its type's; for a template's
    /// method, the template's).
    pub module: u32,
    /// For a derive function (`const fn f(t: type) -> Impl<Tr>`, `comptime.derive.1`): the name
    /// of its parameter, the type it is instantiated for. It is a template, never called, and
    /// its body is not lowered: each `#[derive(Tr)]` instantiates it.
    pub deriver: Option<String>,
    /// For a method instantiated from a derive function's template (`comptime.template.1`):
    /// what the template's parameter is here.
    pub template: Option<Template>,
}

/// A derive function's template, instantiated for one struct: its parameter's name, and the
/// struct (as a type, and by its index among the structs and enums).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Template {
    /// The parameter's name (`t`), which names the type in the template.
    pub var: String,
    /// The type it is instantiated for: a struct, or a built-in type (`comptime.derive.for.1`).
    pub ty: Ty,
    /// The struct's index among the structs and enums; `None` for a built-in type, which has no
    /// fields to reflect on.
    pub adt: Option<u32>,
}

/// The built-in control effects a function lists or needs (`eff.control.1`), as bits:
/// [`SUSPEND`] and [`SPAWN`].
pub type Control = u8;
/// `suspend`: it may pause its task.
pub const SUSPEND: Control = 1;
/// `spawn`: it may start tasks.
pub const SPAWN: Control = 2;

/// How control effects `c` are written: `suspend, spawn`.
pub(crate) fn show_control(c: Control) -> String {
    let mut out = Vec::new();
    if c & SUSPEND != 0 {
        out.push("suspend");
    }
    if c & SPAWN != 0 {
        out.push("spawn");
    }
    out.join(", ")
}

/// An effect a signature lists: `time: Time` (named, for a body that calls it), `Time` (only
/// passed on), `config: &AppConfig` (context data).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EffectSig {
    /// The effect, as a type: `dyn Time` for a trait, `AppConfig` for context data. The body sees
    /// it as a shared borrow of this type.
    pub ty: Ty,
    /// The name it is bound to, if any.
    pub name: Option<String>,
    /// Where it is written.
    pub range: TextRange,
}

impl FnSig {
    /// A free function's signature before it is collected: its name, where, and who can name
    /// it.
    pub(crate) fn placeholder(name: String, src: Src, module: u32, vis: Vis) -> FnSig {
        let none = TextRange::new(0, 0);
        FnSig {
            name,
            name_range: none,
            src,
            node: none,
            container: Container::Free,
            generics: Generics::default(),
            self_kind: None,
            params: Vec::new(),
            ret: Ty::Unit,
            ret_range: None,
            ret_from: Vec::new(),
            has_body: false,
            origin: None,
            fails: Ty::Never,
            fails_range: None,
            effects: Vec::new(),
            track_caller: false,
            control: 0,
            defaults: Vec::new(),
            opaque: Vec::new(),
            opaque_fn: None,
            selectable: false,
            kernel: false,
            foreign: false,
            export: false,
            export_status: false,
            py_export: false,
            default_arg: false,
            intrinsic: false,
            is_const: false,
            const_item: false,
            refine: None,
            vis,
            module,
            deriver: None,
            template: None,
        }
    }
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
    /// Whether it is a view (`view Name { … }`): a struct whose fields may hold borrows, and
    /// whose values are second-class like a borrow.
    pub is_view: bool,
    /// Whether it is declared `#[derive(Copy, Clone)]`.
    pub copy: bool,
    /// Its type parameters.
    pub generics: Generics,
    /// The variants, in order (one for a struct).
    pub variants: Vec<VariantDef>,
    /// Whether its variants are named without the enum's name (`Some`, `None`).
    pub bare_variants: bool,
    /// Who can name it.
    pub vis: Vis,
    /// The module it is declared in.
    pub module: u32,
    /// Whether it is a root capability, marked `#[root_capability]` (`main.root.1`).
    pub root: bool,
    /// Whether it is marked `#[repr(C)]`: laid out as C lays out its struct (`ffi.repr.1`).
    pub repr_c: bool,
    /// `#[export(python)]`: a class of the mod's Python module (`ffi.python.2`).
    pub py_export: bool,
}

/// A `type` item (`refine.1`): `type X = T;`, an alias, or `type X = T where p;`, a refinement.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TypeDef {
    /// The name.
    pub name: String,
    /// Where the name is.
    pub name_range: TextRange,
    /// Which source it is in.
    pub src: Src,
    /// The module it is declared in.
    pub module: u32,
    /// Who can name it.
    pub vis: Vis,
    /// The `type` item's range.
    pub node: TextRange,
    /// The type it names (an alias), or its base (a refinement).
    pub ty: Ty,
    /// For a refinement: the predicate's text, and the functions `X::holds` and `X::new`.
    pub refinement: Option<Refinement>,
}

/// What makes a `type` item a refinement.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Refinement {
    /// The predicate as written, which `RefinementError` names.
    pub clause: String,
    /// `X::holds(self: T) -> (bool, T)`: whether the predicate holds, and the value back.
    pub holds: u32,
    /// `X::new(self: T) -> X with fail(RefinementError)`.
    pub new: u32,
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
    /// Whether it is an enum's variant marked `#[default]` (`attr.derive.default.1`).
    pub default: bool,
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
    /// Who can name it (`mod.vis.2`): a struct's fields are its module's unless marked.
    pub vis: Vis,
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
    /// Each associated type's default, which an impl that does not define it takes
    /// (`item.trait.assoc.default.1`).
    pub assoc_defaults: Vec<Option<Ty>>,
    /// The methods, by index among the functions.
    pub methods: Vec<u32>,
    /// Who can name it.
    pub vis: Vis,
    /// The module it is declared in.
    pub module: u32,
    /// The derive function `#[derive_by(f)]` names (`attr.derive-by.1`), by index among the
    /// functions: `#[derive(Tr)]` instantiates its template.
    pub derive_by: Option<u32>,
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
    /// The module it is declared in (for a derived impl, its type's).
    pub module: u32,
}

/// A test.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TestSig {
    /// The test's name, without quotes.
    pub name: String,
    /// Where the name is.
    pub name_range: TextRange,
    /// The file it is in.
    pub file: u32,
    /// The range of its `test` item.
    pub node: TextRange,
    /// The module it is in.
    pub module: u32,
    /// The function it is for, if it names one (`test.for.1`).
    pub subject: Option<u32>,
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
    "FnOnce",
    "FnMut",
    "Fn",
    "Display",
    "Iterator",
    "IntoIterator",
    "Range",
    "RangeInclusive",
    "RangeFrom",
    "Index",
    "IndexMut",
    "Deref",
    "DerefMut",
    "Box",
    "Rc",
    "Weak",
    "StrPos",
    "FromArrayLiteral",
    "Debug",
    "Default",
    "RefinementError",
    "Refined",
    "Trapped",
    "String",
];

impl ItemTree {
    /// Why `ty` cannot be an effect, if it cannot: an effect is a trait whose methods, its
    /// supertraits' included, all take `&self` and which can be used as `dyn` (there is at least
    /// one such method), or a struct or enum of the program without type parameters (context
    /// data).
    pub fn not_an_effect(&self, ty: &Ty) -> Option<String> {
        match ty {
            Ty::Error => None,
            Ty::Dyn(tr, _) => {
                let name = &self.traits[tr.trait_ as usize].name;
                if let Some(why) = self.not_dyn(tr.trait_, true) {
                    return Some(format!("`{name}` cannot be an effect: {why}"));
                }
                let any = self
                    .supertraits_of(tr.trait_)
                    .iter()
                    .any(|&t| !self.traits[t as usize].methods.is_empty());
                (!any).then(|| format!("`{name}` has no methods, so it cannot be an effect"))
            }
            Ty::Adt(a, _, args) => {
                let def = &self.adts[*a as usize];
                if def.src == Src::Prelude {
                    Some(format!(
                        "`{}` is the prelude's: context data is a type of this program, so wrap \
                         it in one",
                        def.name
                    ))
                } else if !args.is_empty() {
                    Some(format!(
                        "`{ty}` is generic: context data is a type without type parameters, so \
                         wrap it in one"
                    ))
                } else {
                    None
                }
            }
            t => Some(format!(
                "`{t}` is not an effect: effects are traits, or structs and enums as context data"
            )),
        }
    }

    /// Why trait `t` cannot be used as `dyn Trait`, if it cannot: an associated type, or a
    /// method (its own or a supertrait's) that does not take `self` by borrow (by `&self` only,
    /// if `shared`), has type parameters of its own, or names `Self` other than as `self`.
    pub fn not_dyn(&self, t: u32, shared: bool) -> Option<String> {
        // A stored closure's type fixes `FnOnce`'s associated types (`type.box.dyn.1`).
        if ["Fn", "FnMut", "FnOnce"]
            .iter()
            .any(|n| self.lang(n) == Some(t))
        {
            return None;
        }
        for s in self.supertraits_of(t) {
            let def = &self.traits[s as usize];
            if let Some((a, _)) = def.assoc.first() {
                return Some(format!("`{}` has an associated type, `{a}`", def.name));
            }
            for &m in &def.methods {
                let f = &self.fns[m as usize];
                let why = match f.self_kind {
                    None => Some("takes no `self`"),
                    Some(SelfKind::Value) => Some("takes `self` by value"),
                    Some(SelfKind::RefMut) if shared => Some("takes `&mut self`"),
                    _ => None,
                };
                let why = why.or_else(|| {
                    (f.generics.params.len() > f.generics.parent as usize)
                        .then_some("has type parameters")
                });
                let names_self = |t: &Ty| t.any(&|x| matches!(x, Ty::Param(0, _)));
                let why = why.or_else(|| {
                    (f.params.iter().skip(1).any(|p| names_self(&p.ty)) || names_self(&f.ret))
                        .then_some("names `Self`")
                });
                if let Some(why) = why {
                    return Some(format!("its method `{}` {why}", f.name));
                }
            }
        }
        None
    }

    /// Trait `t` and its supertraits, transitively, each once.
    pub fn supertraits_of(&self, t: u32) -> Vec<u32> {
        let mut out = vec![t];
        let mut i = 0;
        while i < out.len() {
            for s in &self.traits[out[i] as usize].supertraits {
                if !out.contains(&s.trait_) {
                    out.push(s.trait_);
                }
            }
            i += 1;
        }
        out
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

    /// The base of a refined type, or the type itself (`refine.base.1`).
    pub fn base(&self, ty: &Ty) -> Ty {
        match ty {
            Ty::Refined(t, _) => self.types[*t as usize].ty.clone(),
            t => t.clone(),
        }
    }

    /// The default of function `f`'s parameter `k`, if it has one: its own, or, for a method of
    /// an impl of a trait, the trait's method's (`expr.call.default.1`).
    pub fn default_of(&self, f: u32, k: usize) -> Option<u32> {
        let sig = &self.fns[f as usize];
        if let Some(d) = sig.defaults.get(k).copied().flatten() {
            return Some(d);
        }
        let Container::Impl(i) = sig.container else {
            return None;
        };
        let t = self.impls[i as usize].trait_ref.as_ref()?.trait_;
        let m = self.traits[t as usize]
            .methods
            .iter()
            .copied()
            .find(|&m| self.fns[m as usize].name == sig.name)?;
        self.fns[m as usize].defaults.get(k).copied().flatten()
    }

    /// The prelude item the language refers to by `name` (`"Copy"`, `"Option"`): a trait's or
    /// a struct's or enum's index.
    pub fn lang(&self, name: &str) -> Option<u32> {
        debug_assert!(LANG.contains(&name), "not a language item: {name}");
        match self
            .modules
            .get(PRELUDE_MODULE as usize)?
            .items
            .get(name)?
            .def
        {
            Def::Trait(t) | Def::Adt(t) => Some(t),
            _ => None,
        }
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

    /// Whether values of `ty` are second-class (`memory.md`: View types): it is or holds a borrow,
    /// a closure or a view, including in its type arguments.
    pub fn is_view(&self, ty: &Ty) -> bool {
        ty.any(&|t| match t {
            Ty::Ref(..) | Ty::Closure(_) | Ty::Anon(_) => true,
            Ty::Adt(a, ..) => self.adts.get(*a as usize).is_some_and(|d| d.is_view),
            _ => false,
        })
    }

    /// Whether something with visibility `vis`, declared in module `at`, can be named in module
    /// `from` (`mod.vis.1`).
    pub fn can_see(&self, vis: Vis, at: u32, from: u32) -> bool {
        match vis {
            Vis::Pub => true,
            Vis::Mod => self.mod_of(at) == self.mod_of(from),
            Vis::Private => at == from,
        }
    }

    /// The prelude's `String` (`text.string.1`), written in Moddil.
    pub fn string_ty(&self) -> Ty {
        match self.lang("String") {
            Some(a) => Ty::Adt(a, "String".into(), std::sync::Arc::from([])),
            None => Ty::Error,
        }
    }

    /// The method `name` of the prelude's inherent impl for a built-in type (`String::new`).
    pub fn builtin_method(&self, ty: &Ty, name: &str) -> Option<u32> {
        self.impls
            .iter()
            .filter(|i| i.trait_ref.is_none() && i.src == Src::Prelude && i.self_ty == *ty)
            .flat_map(|i| i.methods.iter().copied())
            .find(|&m| self.fns[m as usize].name == name)
    }

    /// The prelude's inherent method `name` of the struct or enum `ty` is an instance of, with
    /// the type arguments it is called with: `Vec::as_slice` for a `Vec<int>`.
    pub fn inherent_method(&self, ty: &Ty, name: &str) -> Option<(u32, Vec<Ty>)> {
        let Ty::Adt(a, _, args) = ty else {
            return None;
        };
        self.impls
            .iter()
            .filter(|i| {
                i.trait_ref.is_none() && i.src == Src::Prelude && i.self_ty.adt() == Some(*a)
            })
            .flat_map(|i| i.methods.iter().copied())
            .find(|&m| self.fns[m as usize].name == name)
            .map(|m| (m, args.to_vec()))
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

    /// Whether `ty` is one of C's types, as C functions and `#[export]` functions take and give
    /// them (`ffi.types.1`, `ffi.types.2`): a C scalar ([`ItemTree::is_c_scalar`]), or a
    /// `#[repr(C)]` struct, not generic, of C's types, by value.
    pub fn is_c_type(&self, ty: &Ty) -> bool {
        self.is_c_scalar(ty) || self.c_struct(ty, 0)
    }

    /// Whether `ty` is a C scalar: a sized number but the 128-bit ones, `int` (C's `int64_t`),
    /// `bool`, a raw pointer (to anything), or a C function pointer (`type.fnptr.1`).
    pub fn is_c_scalar(&self, ty: &Ty) -> bool {
        match ty {
            Ty::Int | Ty::Bool | Ty::Ptr(..) | Ty::FnPtr(..) => true,
            Ty::Num(n) => !matches!(n, crate::Num::I128 | crate::Num::U128),
            Ty::Refined(..) => self.is_c_scalar(&self.base(ty)),
            _ => false,
        }
    }

    /// Whether `ty` is a `#[repr(C)]` struct, not generic, whose fields are C's types.
    fn c_struct(&self, ty: &Ty, depth: u32) -> bool {
        let Ty::Adt(a, _, args) = ty else {
            return false;
        };
        let def = &self.adts[*a as usize];
        depth < 16
            && def.repr_c
            && args.is_empty()
            && def.variants.len() == 1
            && !def.variants[0].fields.is_empty()
            && def.variants[0]
                .fields
                .iter()
                .all(|f| self.is_c_scalar(&f.ty) || self.c_struct(&f.ty, depth + 1))
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

    /// The offset of field `i` of struct `ty`, laid out as [`ItemTree::size_of`] lays it out:
    /// as C lays out a `#[repr(C)]` struct (`ffi.repr.1`).
    pub fn field_offset(&self, ty: &Ty, i: u32) -> u64 {
        let (_, _, offsets) = self.fields_layout(&self.field_types(ty, 0), 0);
        offsets.get(i as usize).copied().unwrap_or(0)
    }

    /// The size and alignment of fields laid out in order, each at a multiple of its alignment,
    /// rounded up to the largest; and each one's offset.
    fn fields_layout(&self, fields: &[Ty], depth: u32) -> (u64, u64, Vec<u64>) {
        let mut size: u64 = 0;
        let mut align: u64 = 1;
        let mut offsets = Vec::with_capacity(fields.len());
        for f in fields {
            let (s, a) = self.size_of_depth(f, depth + 1);
            size = size.div_ceil(a) * a;
            offsets.push(size);
            size += s;
            align = align.max(a);
        }
        (size.div_ceil(align) * align, align, offsets)
    }

    fn size_of_depth(&self, ty: &Ty, depth: u32) -> (u64, u64) {
        let layout = |fields: &[Ty]| -> (u64, u64) {
            let (size, align, _) = self.fields_layout(fields, depth);
            (size, align)
        };
        if depth > 32 {
            return (0, 1);
        }
        match ty {
            Ty::Refined(t, _) => match self.types.get(*t as usize) {
                Some(d) => self.size_of_depth(&d.ty, depth + 1),
                None => (0, 1),
            },
            Ty::Int | Ty::Ref(..) | Ty::Ptr(..) | Ty::FnPtr(..) => (8, 8),
            Ty::Num(n) => {
                let b = u64::from(n.bits() / 8);
                (b, b)
            }
            Ty::Bool => (1, 1),
            Ty::Char => (4, 4),
            // A `Buf` is a pointer, a length and a capacity.
            Ty::Buf(_) => (24, 8),
            Ty::Array(t, n) => {
                let (s, a) = self.size_of_depth(t, depth + 1);
                match **n {
                    Ty::Len(n) => (s.div_ceil(a) * a * n.max(0) as u64, a),
                    _ => (0, a),
                }
            }
            Ty::Str
            | Ty::Slice(_)
            | Ty::Len(_)
            | Ty::Unit
            | Ty::Never
            | Ty::Error
            | Ty::Param(..)
            | Ty::Opaque(_)
            | Ty::Assoc(_)
            | Ty::Infer(_)
            | Ty::Dyn(..)
            | Ty::Errors(_) => (0, 1),
            Ty::Closure(c) => layout(&c.captures),
            Ty::Anon(a) => layout(&a.methods.iter().map(|(_, t)| t.clone()).collect::<Vec<_>>()),
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
