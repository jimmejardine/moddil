//! A program's items, the prelude's and the files', as tables: functions, structs and enums,
//! traits and impls. Built once, then read-only.
//!
//! Items are kept by a **key**: a prelude item's is its name (`Option`), and a file's item's is
//! its module and name (`3:Point`), so two modules may each have a `Point`. [`Program::resolve`]
//! says what a path names in a module (`modules.md`), and [`dispatch`] which method a value's
//! kind gets.

use std::collections::HashMap;

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{SyntaxKind as K, SyntaxNode};

use crate::value::{Head, Value};
use crate::{CORE_MODULES, PRELUDE};

mod dispatch;

/// How a struct or variant is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Shape {
    Named,
    Tuple,
    Unit,
}

#[derive(Clone, Debug)]
pub(crate) struct VariantInfo {
    pub(crate) name: String,
    pub(crate) shape: Shape,
    /// Field names in declaration order (`0`, `1`, … for positional fields).
    pub(crate) fields: Vec<String>,
    /// The fields' types as written.
    pub(crate) types: Vec<Option<ast::Type>>,
    /// Whether it is marked `#[default]`.
    pub(crate) default: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct AdtInfo {
    /// Its name, as written.
    pub(crate) name: String,
    pub(crate) is_enum: bool,
    pub(crate) variants: Vec<VariantInfo>,
    /// The type parameters' names.
    pub(crate) params: Vec<String>,
    /// The traits it derives.
    pub(crate) derives: Vec<String>,
    /// Whether its variants are named on their own (`Some`, `None`).
    pub(crate) bare: bool,
    /// The module it is declared in, whose names its fields' types use.
    pub(crate) module: usize,
}

/// How a method takes `self`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SelfKind {
    Value,
    Ref,
    RefMut,
}

/// A function: free, or a method of an impl or a trait.
#[derive(Clone, Debug)]
pub(crate) struct FnDef {
    pub(crate) node: ast::FnItem,
    /// Whether it is the files' code (not the prelude's).
    pub(crate) user: bool,
    /// The file it is in (for the prelude's code, none of the program's).
    pub(crate) file: u32,
    /// The module whose names it uses.
    pub(crate) module: u32,
    /// Its own type parameters' names.
    pub(crate) params: Vec<String>,
    pub(crate) self_kind: Option<SelfKind>,
    /// Whether it is a destructor, whose `self` is not dropped again.
    pub(crate) destructor: bool,
    /// Whether it is `#[track_caller]` (`attr.track-caller.1`).
    pub(crate) tracked: bool,
    /// Its wrappers, if it has any (`attr.around.1`).
    pub(crate) wrapping: Option<std::sync::Arc<Wrapping>>,
    /// For a method of a derive's template (`comptime.template.1`): the template's parameter,
    /// and the type it is instantiated for: a struct, or a built-in type
    /// (`comptime.derive.for.1`).
    pub(crate) template: Option<(String, Head)>,
}

/// What a wrapped function's expansion needs (`attr.around.1`).
#[derive(Debug)]
pub(crate) struct Wrapping {
    /// The function's key.
    pub(crate) key: String,
    /// Each `#[around(…)]`'s input, outermost first: a function's path, or a call of one.
    pub(crate) wrappers: Vec<ast::Expr>,
    /// The key of its `f::Args` struct.
    pub(crate) args: String,
    /// Its path, as `FnInfo` gives it.
    pub(crate) path: String,
    /// Its parameters: each one's name and type as written.
    pub(crate) params: Vec<(String, String)>,
}

pub(crate) struct ImplInfo {
    /// The trait's key.
    pub(crate) trait_: Option<String>,
    pub(crate) head: Head,
    /// The module it is in.
    pub(crate) module: u32,
    /// The trait as written, until it is resolved.
    trait_path: Option<Vec<String>>,
    pub(crate) self_ty: ast::Type,
    pub(crate) params: Vec<String>,
    pub(crate) methods: HashMap<String, FnDef>,
    /// Whether it is for every type: its self type is a type parameter of its own with no
    /// bounds (`impl<T> ToRc for T`).
    pub(crate) blanket: bool,
}

pub(crate) struct TraitInfo {
    pub(crate) methods: HashMap<String, FnDef>,
    /// The derive function `#[derive_by(f)]` names (`attr.derive-by.1`), and the module to find
    /// it in.
    pub(crate) derive_by: Option<(String, usize)>,
}

/// Where a method comes from.
#[derive(Clone)]
pub(crate) enum Method {
    /// Code: a function, with the impl it is in (`None` for a trait's default).
    Code(FnDef, Option<usize>),
    /// A derived trait's method, built in: `Clone`, `PartialEq`, `PartialOrd` or `Ord`.
    Derived(String),
    /// The built-in `Clone` of a borrow, a tuple or `()`: a copy.
    Copy,
}

impl Method {
    pub(crate) fn self_kind(&self) -> Option<SelfKind> {
        match self {
            Method::Code(f, _) => f.self_kind,
            Method::Derived(_) | Method::Copy => Some(SelfKind::Ref),
        }
    }
}

/// What a name in a module names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Def {
    /// A function, a struct or enum, or a trait, by its key.
    Item(String),
    /// A variant named on its own (`Some`): its enum's key and its name.
    Variant(String, String),
    /// A module.
    Module(usize),
}

/// A module: the names it declares and imports, and the modules inside it.
#[derive(Default)]
pub(crate) struct Module {
    pub(crate) mod_: usize,
    names: HashMap<String, Def>,
    imports: Vec<(String, Vec<String>)>,
    children: HashMap<String, usize>,
}

/// A mod: its root module and the mods it depends on, by the names it calls them.
pub(crate) struct ModEntry {
    root: usize,
    deps: HashMap<String, usize>,
}

/// A test: its name, body, file and module.
pub(crate) struct TestDef {
    pub(crate) name: String,
    pub(crate) body: ast::Block,
    pub(crate) file: u32,
    pub(crate) module: u32,
}

/// The prelude's module.
pub(crate) const PRELUDE_MODULE: usize = 0;

/// A program's items: the prelude's and the files'.
pub(crate) struct Program {
    /// The files' texts.
    pub(crate) texts: Vec<String>,
    /// The files' paths, as messages show them.
    pub(crate) paths: Vec<String>,
    pub(crate) fns: HashMap<String, FnDef>,
    pub(crate) adts: HashMap<String, AdtInfo>,
    pub(crate) traits: HashMap<String, TraitInfo>,
    pub(crate) impls: Vec<ImplInfo>,
    pub(crate) tests: Vec<TestDef>,
    pub(crate) modules: Vec<Module>,
    mods: Vec<ModEntry>,
    /// The `core` mod's `os` module (`os.1`).
    pub(crate) core_os: usize,
    /// The `core` mod's `gpu` module (`gpu.1`).
    pub(crate) core_gpu: usize,
    /// The const items, by key: their initialiser and the module whose names it uses.
    pub(crate) consts: HashMap<String, (ast::Expr, usize)>,
    /// The `type` items, by key (`refine.1`).
    pub(crate) types: HashMap<String, TypeInfo>,
    /// Each file's module path.
    file_paths: Vec<Vec<String>>,
    /// The syntax trees: the prelude's, `core`'s, then each file's.
    roots: Vec<SyntaxNode>,
    /// The sources, which each task parses again (`task.sched.1`).
    sources: crate::machine::tasks::Sources,
}

/// A `type` item: an alias, or a refinement with its predicate.
#[derive(Clone, Debug)]
pub(crate) struct TypeInfo {
    pub(crate) name: String,
    /// The type it names, or the refinement's base.
    pub(crate) ty: ast::Type,
    /// A refinement's predicate, and its text as written.
    pub(crate) pred: Option<(ast::Expr, String)>,
    pub(crate) module: usize,
}

impl Program {
    pub(crate) fn new(files: &[crate::SourceSpec], mods: &[crate::ModSpec]) -> Program {
        let mut p = Program {
            texts: files.iter().map(|f| f.text.clone()).collect(),
            paths: files.iter().map(|f| f.path.clone()).collect(),
            fns: HashMap::new(),
            consts: HashMap::new(),
            types: HashMap::new(),
            adts: HashMap::new(),
            traits: HashMap::new(),
            impls: Vec::new(),
            tests: Vec::new(),
            modules: vec![Module::default()],
            mods: Vec::new(),
            file_paths: files.iter().map(|f| f.module.clone()).collect(),
            roots: Vec::new(),
            core_os: usize::MAX,
            core_gpu: usize::MAX,
            sources: std::sync::Arc::new((files.to_vec(), mods.to_vec())),
        };
        for (i, m) in mods.iter().enumerate() {
            let root = p.modules.len();
            p.modules.push(Module {
                mod_: i,
                ..Module::default()
            });
            p.mods.push(ModEntry {
                root,
                deps: m
                    .deps
                    .iter()
                    .map(|(n, d)| (n.clone(), *d as usize))
                    .collect(),
            });
        }
        let mut file_modules = Vec::new();
        for f in files {
            let mut at = p.mods[f.mod_ as usize].root;
            for name in &f.module {
                at = match p.modules[at].children.get(name) {
                    Some(&c) => c,
                    None => {
                        let c = p.modules.len();
                        p.modules.push(Module {
                            mod_: f.mod_ as usize,
                            ..Module::default()
                        });
                        p.modules[at].children.insert(name.clone(), c);
                        c
                    }
                };
            }
            file_modules.push(at);
        }
        // The built-in `core` mod and its modules; every mod depends on it.
        let core = p.mods.len();
        let core_root = p.modules.len();
        p.modules.push(Module {
            mod_: core,
            ..Module::default()
        });
        let mut core_modules = Vec::new();
        for (name, _) in CORE_MODULES {
            let m = p.modules.len();
            p.modules.push(Module {
                mod_: core,
                ..Module::default()
            });
            p.modules[core_root].children.insert(name.to_string(), m);
            core_modules.push(m);
            if name == "os" {
                p.core_os = m;
            }
            if name == "gpu" {
                p.core_gpu = m;
            }
        }
        for m in &mut p.mods {
            m.deps.entry("core".to_string()).or_insert(core);
        }
        p.mods.push(ModEntry {
            root: core_root,
            deps: HashMap::new(),
        });
        let prelude = moddil_syntax::parse(PRELUDE).syntax();
        for item in prelude.children() {
            p.item(None, PRELUDE_MODULE, item);
        }
        p.roots.push(prelude);
        for ((_, src), m) in CORE_MODULES.iter().zip(core_modules) {
            let core_items = moddil_syntax::parse(src).syntax();
            for item in core_items.children() {
                p.item(None, m, item);
            }
            p.roots.push(core_items);
        }
        for (f, file) in files.iter().enumerate() {
            let root = moddil_syntax::parse(&file.text).syntax();
            for item in root.children() {
                p.item(Some(f as u32), file_modules[f], item);
            }
            p.roots.push(root);
        }
        // Now every name is known: what each impl is for, and of which trait.
        for i in 0..p.impls.len() {
            let module = p.impls[i].module as usize;
            let head = p.head_in(module, &p.impls[i].self_ty);
            let trait_ = p.impls[i]
                .trait_path
                .clone()
                .map(|path| match p.resolve(module, &path) {
                    Some((Def::Item(k), _)) => k,
                    _ => path.join("::"),
                });
            p.impls[i].head = head;
            p.impls[i].trait_ = trait_;
        }
        p.template_impls();
        p
    }

    /// The impls derives with templates make (`comptime.template.1`): for each struct deriving
    /// a trait whose `#[derive_by(f)]` names a derive function, `f`'s template's methods, run
    /// where `f` is written, with its parameter naming the struct. And for each type a prelude
    /// derive function's `#[derive_for(…)]` lists, its template's impl for that type
    /// (`comptime.derive.for.1`).
    fn template_impls(&mut self) {
        let mut made = Vec::new();
        for (key, adt) in &self.adts {
            for d in &adt.derives {
                // The prelude's derives (`attr.derive.1`) are built in here, structurally, as
                // the reference states them; the compiler runs their templates.
                if matches!(
                    d.as_str(),
                    "Clone" | "PartialEq" | "PartialOrd" | "Ord" | "Hash" | "Debug" | "Default"
                ) {
                    continue;
                }
                let Some((Def::Item(tr), 1)) = self.resolve(adt.module, std::slice::from_ref(d))
                else {
                    continue;
                };
                let Some((f, module)) = self.traits.get(&tr).and_then(|t| t.derive_by.clone())
                else {
                    continue;
                };
                let Some((Def::Item(fk), 1)) = self.resolve(module, &[f]) else {
                    continue;
                };
                let Some(def) = self.fns.get(&fk) else {
                    continue;
                };
                made.extend(template_impl(def, tr, Head::Adt(key.clone())));
            }
        }
        for def in self.fns.values().filter(|d| !d.user) {
            let Some(types) = derive_for(def.node.syntax()) else {
                continue;
            };
            let Some(tr) = def.node.ret_type().and_then(|r| {
                let text = r.syntax().text().to_string();
                let t = text.trim_start_matches("->").trim();
                Some(
                    t.strip_prefix("Impl<")?
                        .strip_suffix('>')?
                        .trim()
                        .to_string(),
                )
            }) else {
                continue;
            };
            let Some((Def::Item(tr), 1)) = self.resolve(def.module as usize, &[tr]) else {
                continue;
            };
            for ty in types {
                for head in for_heads(&ty) {
                    made.extend(template_impl(def, tr.clone(), head));
                }
            }
        }
        self.impls.extend(made);
    }

    /// The program's sources.
    pub(crate) fn sources(&self) -> crate::machine::tasks::Sources {
        self.sources.clone()
    }

    /// How a value holds `node` (`NodeRef`).
    pub(crate) fn node_ref(&self, node: &SyntaxNode) -> crate::value::NodeRef {
        let root = node.ancestors().last().expect("a root");
        let tree = self
            .roots
            .iter()
            .position(|r| *r == root)
            .expect("a node of the program");
        crate::value::NodeRef {
            tree,
            range: node.text_range(),
            kind: node.kind(),
        }
    }

    /// The node `r` is, in this program's trees.
    pub(crate) fn node(&self, r: &crate::value::NodeRef) -> SyntaxNode {
        self.roots[r.tree]
            .descendants()
            .into_iter()
            .find(|n| n.text_range() == r.range && n.kind() == r.kind)
            .expect("a node of the program")
    }

    /// The key of item `name` of module `module`.
    fn key(module: usize, name: &str) -> String {
        if module == PRELUDE_MODULE {
            name.to_string()
        } else {
            format!("{module}:{name}")
        }
    }

    /// Declares `name` in `module` as the item `key`.
    fn declare(&mut self, module: usize, name: &str, def: Def) {
        self.modules[module].names.insert(name.to_string(), def);
    }

    /// What `name` means in `module`: its own names, its imports, the modules inside it, then the
    /// prelude's names.
    pub(crate) fn lookup(&self, module: usize, name: &str) -> Option<Def> {
        self.member(module, name, 0)
            .or_else(|| self.member(PRELUDE_MODULE, name, 0))
    }

    fn member(&self, module: usize, name: &str, depth: u32) -> Option<Def> {
        let m = &self.modules[module];
        if let Some(d) = m.names.get(name) {
            return Some(d.clone());
        }
        if depth < 32
            && let Some((_, path)) = m.imports.iter().find(|(n, _)| n == name)
        {
            return match self.resolve_depth(module, path, depth + 1) {
                Some((d, n)) if n == path.len() => Some(d),
                Some((Def::Item(k), n)) if n + 1 == path.len() => {
                    Some(Def::Variant(k, path[n].clone()))
                }
                _ => None,
            };
        }
        m.children.get(name).map(|&c| Def::Module(c))
    }

    /// What the leading segments of `path`, in `module`, name, and how many they are: modules,
    /// then one item.
    pub(crate) fn resolve(&self, module: usize, path: &[String]) -> Option<(Def, usize)> {
        self.resolve_depth(module, path, 0)
    }

    fn resolve_depth(&self, module: usize, path: &[String], depth: u32) -> Option<(Def, usize)> {
        let first = path.first()?;
        let here = self.modules[module].mod_;
        let mut def = match first.as_str() {
            "mod" => Def::Module(self.mods[here].root),
            "self" => Def::Module(module),
            name => match self
                .member(module, name, depth)
                .or_else(|| self.member(PRELUDE_MODULE, name, depth))
            {
                Some(d) => d,
                None => Def::Module(self.mods[here].root_of_dep(name, &self.mods)?),
            },
        };
        for (i, seg) in path.iter().enumerate().skip(1) {
            let Def::Module(m) = def else {
                return Some((def, i));
            };
            def = self.member(m, seg, depth)?;
        }
        Some((def, path.len()))
    }

    /// The kind of value a type written in module `module` has.
    pub(crate) fn head_in(&self, module: usize, t: &ast::Type) -> Head {
        match t {
            ast::Type::Tuple(n) if n.children().next().is_none() => Head::Unit,
            ast::Type::Tuple(n) => Head::Tuple(n.children().filter_map(ast::Type::cast).count()),
            ast::Type::Array(_) | ast::Type::Slice(_) => Head::Array,
            // A `&str` is its text here, so an impl for `&str` is one for the text.
            ast::Type::Ref(r)
                if r.children()
                    .find_map(ast::Type::cast)
                    .is_some_and(|x| x.syntax().text() == "str") =>
            {
                Head::Str
            }
            ast::Type::Ref(_) => Head::Ref,
            _ => {
                let path = type_path(t.syntax());
                match path
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .as_slice()
                {
                    ["int" | "i64"] => Head::Int,
                    [n] if crate::num::Kind::named(n).is_some() => {
                        Head::Num(crate::num::Kind::named(n).expect("a number type"))
                    }
                    ["bool"] => Head::Bool,
                    ["char"] => Head::Char,
                    ["str"] => Head::Str,
                    ["Buf"] => Head::Buf,
                    _ => match self.resolve(module, &path) {
                        // A refined value is its base's, and an alias names its type.
                        Some((Def::Item(k), _)) if self.types.contains_key(&k) => {
                            let t = &self.types[&k];
                            self.head_in(t.module, &t.ty)
                        }
                        Some((Def::Item(k), _)) => Head::Adt(k),
                        _ => Head::Adt(path.join("::")),
                    },
                }
            }
        }
    }

    pub(crate) fn item(&mut self, file: Option<u32>, module: usize, item: SyntaxNode) {
        let user = file.is_some();
        let site = (file.unwrap_or(u32::MAX), module as u32);
        match item.kind() {
            K::USE_ITEM => {
                if let Some(t) = item.child(K::USE_TREE) {
                    let mut out = Vec::new();
                    use_tree(&t, &[], &mut out);
                    self.modules[module].imports.extend(out);
                }
            }
            // `type X = T;` and `type X = T where p;` (`refine.1`).
            K::TYPE_ITEM => {
                let name = name_of(&item);
                let key = Program::key(module, &name);
                self.declare(module, &name, Def::Item(key.clone()));
                let ty = item
                    .children()
                    .find_map(ast::Type::cast)
                    .expect("a checked `type` names a type");
                let pred = item.children().find_map(ast::Expr::cast).map(|e| {
                    let text = e.syntax().text().to_string().trim().to_string();
                    (e, text)
                });
                self.types.insert(
                    key,
                    TypeInfo {
                        name,
                        ty,
                        pred,
                        module,
                    },
                );
            }
            // `const NAME: T = e;` (`comptime.const.1`).
            K::CONST_ITEM => {
                let name = name_of(&item);
                let key = Program::key(module, &name);
                self.declare(module, &name, Def::Item(key.clone()));
                let init = item
                    .children()
                    .find_map(ast::Expr::cast)
                    .expect("a checked const has a value");
                self.consts.insert(key, (init, module));
            }
            K::FN_ITEM => {
                let f = ast::FnItem::cast(item).expect("a function");
                let name = f.name().expect("a checked function has a name").text();
                let key = Program::key(module, &name);
                self.declare(module, &name, Def::Item(key.clone()));
                let mut def = fn_def(&f, user, false, site);
                if let Some(file) = file {
                    def.wrapping = self.wrapping(&f, &name, file, module).map(|mut w| {
                        w.key = key.clone();
                        std::sync::Arc::new(w)
                    });
                }
                self.fns.insert(key, def);
            }
            K::STRUCT_ITEM | K::ENUM_ITEM | K::VIEW_ITEM => {
                let name = name_of(&item);
                let variants = if matches!(item.kind(), K::STRUCT_ITEM | K::VIEW_ITEM) {
                    vec![variant_info(&item, name.clone())]
                } else {
                    item.child(K::VARIANT_LIST)
                        .map(|l| {
                            l.children()
                                .filter(|v| v.kind() == K::VARIANT)
                                .map(|v| variant_info(&v, name_of(&v)))
                                .collect()
                        })
                        .unwrap_or_default()
                };
                let derives = derives(&item);
                let key = Program::key(module, &name);
                let bare = !user && matches!(name.as_str(), "Option" | "Result");
                self.declare(module, &name, Def::Item(key.clone()));
                if bare {
                    for v in &variants {
                        self.declare(module, &v.name, Def::Variant(key.clone(), v.name.clone()));
                    }
                }
                self.adts.insert(
                    key,
                    AdtInfo {
                        name,
                        is_enum: item.kind() == K::ENUM_ITEM,
                        variants,
                        params: generic_names(&item),
                        derives,
                        bare,
                        module,
                    },
                );
            }
            K::TRAIT_ITEM => {
                let methods = item
                    .child(K::TRAIT_BODY)
                    .map(|b| {
                        b.children()
                            .filter_map(ast::FnItem::cast)
                            .map(|f| {
                                let name = f.name().expect("a name").text();
                                (name, fn_def(&f, user, false, site))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let name = name_of(&item);
                let key = Program::key(module, &name);
                self.declare(module, &name, Def::Item(key.clone()));
                let derive_by = item
                    .children()
                    .filter(|c| c.kind() == K::OUTER_ATTR)
                    .find_map(|a| {
                        let t = a.text().to_string().replace(' ', "");
                        t.strip_prefix("#[derive_by(")
                            .and_then(|r| r.strip_suffix(")]"))
                            .map(|f| (f.to_string(), module))
                    });
                self.traits.insert(key, TraitInfo { methods, derive_by });
            }
            K::IMPL_ITEM => {
                let types: Vec<ast::Type> = item.children().filter_map(ast::Type::cast).collect();
                let has_trait = item.token(K::FOR_KW).is_some();
                let trait_path = has_trait.then(|| type_path(types[0].syntax()));
                let self_ty = types[usize::from(has_trait)].clone();
                // Only the prelude's `Drop` is a destructor's trait.
                let destructor = trait_path.as_deref().is_some_and(
                    |p| matches!(self.resolve(module, p), Some((Def::Item(k), _)) if k == "Drop"),
                );
                // A function without `self` that shares a method's name is kept under
                // `"name fn"`, which no name can be (`item.impl.same-name.1`).
                let mut methods: HashMap<String, FnDef> = HashMap::new();
                for f in item.child(K::IMPL_BODY).into_iter().flat_map(|b| {
                    b.children()
                        .filter_map(ast::FnItem::cast)
                        .collect::<Vec<_>>()
                }) {
                    let name = f.name().expect("a name").text();
                    let def = fn_def(&f, user, destructor, site);
                    let key = match methods.get(&name) {
                        Some(other) if other.self_kind.is_some() && def.self_kind.is_none() => {
                            format!("{name} fn")
                        }
                        Some(other) if other.self_kind.is_none() && def.self_kind.is_some() => {
                            let moved = methods.remove(&name).expect("there");
                            methods.insert(format!("{name} fn"), moved);
                            name
                        }
                        _ => name,
                    };
                    methods.insert(key, def);
                }
                let blanket = trait_path.is_some() && unbounded_self_param(&item, &self_ty);
                self.impls.push(ImplInfo {
                    trait_: None,
                    head: Head::Unit,
                    module: module as u32,
                    trait_path,
                    self_ty,
                    params: generic_names(&item),
                    methods,
                    blanket,
                });
            }
            K::TEST_ITEM if user => {
                let t = ast::TestItem::cast(item).expect("a test");
                let name = t
                    .name_token()
                    .map(|n| n.text().trim_matches('"').to_string())
                    .unwrap_or_default();
                self.tests.push(TestDef {
                    name,
                    body: t.body().expect("a checked test has a body"),
                    file: site.0,
                    module: site.1,
                });
            }
            _ => {}
        }
    }

    /// The wrappers of function `f`, named `name`, in `file` and `module`, if it has any
    /// (`attr.around.1`); its `f::Args` struct is added, with `Debug`, `Clone` and `PartialEq`.
    fn wrapping(
        &mut self,
        f: &ast::FnItem,
        name: &str,
        file: u32,
        module: usize,
    ) -> Option<Wrapping> {
        let wrappers: Vec<ast::Expr> = f
            .syntax()
            .children()
            .filter(|c| c.kind() == K::OUTER_ATTR)
            .flat_map(|a| {
                a.children()
                    .filter(|c| c.kind() == K::ATTR)
                    .collect::<Vec<_>>()
            })
            .filter(|a| {
                a.child(K::PATH)
                    .and_then(ast::Path::cast)
                    .is_some_and(|p| p.segments() == ["around"])
            })
            .filter_map(|a| around_input(&a))
            .collect();
        if wrappers.is_empty() {
            return None;
        }
        let params: Vec<(String, String)> = f
            .params()
            .map(|l| l.params().collect::<Vec<_>>())
            .unwrap_or_default()
            .iter()
            .map(|p| {
                let name = p
                    .pat()
                    .map(|q| {
                        q.syntax()
                            .text()
                            .to_string()
                            .trim_start_matches("mut")
                            .trim()
                            .to_string()
                    })
                    .unwrap_or_default();
                let ty = p
                    .ty()
                    .map(|t| t.syntax().text().to_string())
                    .unwrap_or_default();
                (name, ty.split_whitespace().collect::<Vec<_>>().join(" "))
            })
            .collect();
        let args = Program::key(module, &format!("{name}::Args"));
        let fields: Vec<String> = params.iter().map(|(n, _)| n.clone()).collect();
        self.adts.insert(
            args.clone(),
            AdtInfo {
                name: format!("{name}::Args"),
                is_enum: false,
                variants: vec![VariantInfo {
                    name: format!("{name}::Args"),
                    shape: if fields.is_empty() {
                        Shape::Unit
                    } else {
                        Shape::Named
                    },
                    types: vec![None; fields.len()],
                    fields,
                    default: false,
                }],
                params: (0..params.len()).map(|i| format!("P{i}")).collect(),
                derives: ["Debug", "Clone", "PartialEq"].map(String::from).to_vec(),
                bare: false,
                module,
            },
        );
        let mut path = self.file_paths[file as usize].clone();
        path.push(name.to_string());
        Some(Wrapping {
            key: String::new(),
            wrappers,
            args,
            path: path.join("::"),
            params,
        })
    }

    /// The struct or variant a path names in `module`: `Point`, `Shape::Circle`, `Some`,
    /// `crate::shapes::Point`. A path that starts with a struct's or enum's key (as `Self` does)
    /// names it directly.
    pub(crate) fn ctor(
        &self,
        module: usize,
        segments: &[String],
    ) -> Option<(String, &VariantInfo)> {
        let (key, rest) = match segments.first().and_then(|s| self.adts.get_key_value(s)) {
            Some((k, _)) if k.contains(':') || self.lookup(module, k).is_none() => {
                (k.clone(), &segments[1..])
            }
            _ => match self.resolve(module, segments)? {
                (Def::Item(k), n) => (k, &segments[n..]),
                (Def::Variant(k, v), n) if n == segments.len() => {
                    let adt = self.adts.get(&k)?;
                    let info = adt.variants.iter().find(|x| x.name == v)?;
                    return Some((k, info));
                }
                _ => return None,
            },
        };
        let adt = self.adts.get(&key)?;
        match rest {
            [] if !adt.is_enum => Some((key, &adt.variants[0])),
            [variant] if adt.is_enum => adt
                .variants
                .iter()
                .find(|v| v.name == *variant)
                .map(|v| (key, v)),
            _ => None,
        }
    }

    pub(crate) fn variant(&self, ty: &str, variant: &str) -> &VariantInfo {
        self.adts[ty]
            .variants
            .iter()
            .find(|v| v.name == variant)
            .expect("a known variant")
    }

    /// `v` as Moddil source, with `part` rendering the values inside it (a machine shows the
    /// borrows among them as what they point to).
    pub(crate) fn render_with(&self, v: &Value, part: &dyn Fn(&Value) -> String) -> String {
        match v {
            Value::Int(i) => i.to_string(),
            Value::Num(k, bits) => crate::num::text(*k, *bits),
            v @ (Value::FloatLit(_) | Value::BigLit(_) | Value::HugeLit(_)) => {
                self.render_with(&crate::num::settle(v.clone()), part)
            }
            Value::Bool(b) => b.to_string(),
            Value::Char(c) => format!("{c:?}"),
            Value::Str(s) => format!("{:?}", &**s),
            // A `String` as its text, in quotes.
            v @ Value::Adt { ty, .. } if ty == "String" => {
                format!("{:?}", crate::value::string_text(v).unwrap_or_default())
            }
            Value::Unit => "()".to_string(),
            Value::Array(xs) | Value::Buf(_, xs) => {
                let xs: Vec<String> = xs.iter().map(part).collect();
                format!("[{}]", xs.join(", "))
            }
            // A `Vec` is shown as the literal that makes one.
            Value::Adt { ty, fields, .. } if ty == "Vec" && fields.len() == 1 => part(&fields[0]),
            Value::Tuple(xs) if xs.len() == 1 => format!("({},)", part(&xs[0])),
            Value::Tuple(xs) => {
                let xs: Vec<String> = xs.iter().map(part).collect();
                format!("({})", xs.join(", "))
            }
            Value::Ref(..) => unreachable!("a borrow is shown by the machine"),
            Value::Moved => "<moved>".to_string(),
            Value::Closure(_) => "{closure}".to_string(),
            Value::Anon(..) => "{impl}".to_string(),
            Value::Alias(_) => unreachable!("an alias is followed"),
            Value::Adt {
                ty,
                variant,
                fields,
            } => {
                let adt = &self.adts[ty];
                let path = if adt.bare {
                    variant.clone()
                } else if adt.is_enum {
                    format!("{}::{variant}", adt.name)
                } else {
                    adt.name.clone()
                };
                let info = self.variant(ty, variant);
                match info.shape {
                    Shape::Unit => path,
                    Shape::Tuple => {
                        let xs: Vec<String> = fields.iter().map(part).collect();
                        format!("{path}({})", xs.join(", "))
                    }
                    Shape::Named => {
                        let xs: Vec<String> = info
                            .fields
                            .iter()
                            .zip(fields)
                            .map(|(n, x)| format!("{n}: {}", part(x)))
                            .collect();
                        format!("{path} {{ {} }}", xs.join(", "))
                    }
                }
            }
        }
    }
}

/// Whether impl `item`'s self type, `self_ty`, is one of its type parameters, with no bounds and
/// no `where` clause: an impl for every type.
fn unbounded_self_param(item: &SyntaxNode, self_ty: &ast::Type) -> bool {
    // Only a bare path: `&[T]` and `Vec<T>` name `T` too.
    if !matches!(self_ty, ast::Type::Path(_)) || self_ty.syntax().text().to_string().contains('<') {
        return false;
    }
    let path = type_path(self_ty.syntax());
    let [name] = path.as_slice() else {
        return false;
    };
    item.child(K::WHERE_CLAUSE).is_none()
        && item.child(K::GENERIC_PARAMS).is_some_and(|g| {
            g.children().any(|p| {
                p.kind() == K::TYPE_PARAM
                    && name_of(&p) == *name
                    && p.children().all(|c| c.kind() == K::NAME)
            })
        })
}

pub(crate) fn generic_names(node: &SyntaxNode) -> Vec<String> {
    node.child(K::GENERIC_PARAMS)
        .map(|g| {
            g.children()
                .filter(|c| matches!(c.kind(), K::TYPE_PARAM | K::CONST_PARAM))
                .map(|p| name_of(&p))
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn fn_def(f: &ast::FnItem, user: bool, destructor: bool, site: (u32, u32)) -> FnDef {
    let self_kind = f
        .syntax()
        .child(K::PARAM_LIST)
        .and_then(|l| l.child(K::SELF_PARAM))
        .map(
            |s| match (s.token(K::AMP).is_some(), s.token(K::MUT_KW).is_some()) {
                (true, true) => SelfKind::RefMut,
                (true, false) => SelfKind::Ref,
                (false, _) => SelfKind::Value,
            },
        );
    let tracked = f
        .syntax()
        .children()
        .filter(|c| c.kind() == K::OUTER_ATTR)
        .any(|a| a.text().to_string().replace(' ', "") == "#[track_caller]");
    FnDef {
        node: f.clone(),
        tracked,
        user,
        file: site.0,
        module: site.1,
        params: generic_names(f.syntax()),
        self_kind,
        destructor,
        wrapping: None,
        template: None,
    }
}

/// The path a type's first `PATH` writes, without generic arguments: `["mod", "a", "Point"]`.
pub(crate) fn type_path(node: &SyntaxNode) -> Vec<String> {
    let path = if node.kind() == K::PATH {
        Some(node.clone())
    } else {
        node.descendants().into_iter().find(|n| n.kind() == K::PATH)
    };
    path.map(|p| {
        p.children()
            .filter(|c| c.kind() == K::PATH_SEGMENT)
            .filter_map(|s| {
                s.child_tokens()
                    .find(|t| !t.kind().is_trivia())
                    .map(|t| t.text().to_string())
            })
            .collect()
    })
    .unwrap_or_default()
}

/// The imports a `USE_TREE` makes, after the path `prefix`.
fn use_tree(tree: &SyntaxNode, prefix: &[String], out: &mut Vec<(String, Vec<String>)>) {
    let mut path = prefix.to_vec();
    if let Some(p) = tree.child(K::PATH) {
        let segs = type_path(&p);
        // `self` in a group names the module the path does (`mod.use.self.1`).
        if segs == ["self"] && !prefix.is_empty() {
            let name = tree
                .child(K::NAME)
                .and_then(ast::Name::cast)
                .map(|n| n.text())
                .or_else(|| prefix.last().cloned());
            if let Some(name) = name {
                out.push((name, prefix.to_vec()));
            }
            return;
        }
        path.extend(segs);
    }
    if let Some(list) = tree.child(K::USE_LIST) {
        for sub in list.children().filter(|c| c.kind() == K::USE_TREE) {
            use_tree(&sub, &path, out);
        }
        return;
    }
    let name = tree
        .child(K::NAME)
        .and_then(ast::Name::cast)
        .map(|n| n.text())
        .or_else(|| path.last().cloned());
    if let Some(name) = name {
        out.push((name, path));
    }
}

impl ModEntry {
    /// The root module of the dependency this mod calls `name`.
    fn root_of_dep(&self, name: &str, mods: &[ModEntry]) -> Option<usize> {
        self.deps.get(name).map(|&d| mods[d].root)
    }
}

/// The first name in a type's path: `Option` for `Option<T>`.
pub(crate) fn first_name(node: &SyntaxNode) -> String {
    node.descendants()
        .into_iter()
        .find(|n| n.kind() == K::PATH_SEGMENT)
        .and_then(|s| {
            s.child_tokens()
                .find(|t| matches!(t.kind(), K::IDENT | K::SELF_TYPE_KW))
                .map(|t| t.text().to_string())
        })
        .unwrap_or_default()
}

pub(crate) fn name_of(node: &SyntaxNode) -> String {
    ast::name(node).map(|n| n.text()).unwrap_or_default()
}

/// The traits `#[derive(…)]` on an item names.
pub(crate) fn derives(node: &SyntaxNode) -> Vec<String> {
    let mut out = Vec::new();
    for attr in node.children().filter(|c| c.kind() == K::OUTER_ATTR) {
        let Some(a) = attr.child(K::ATTR) else {
            continue;
        };
        if let Some(input) = a.child(K::ATTR_INPUT) {
            out.extend(
                input
                    .child_tokens()
                    .filter(|t| t.kind() == K::IDENT)
                    .map(|t| t.text().to_string()),
            );
        }
    }
    out
}

pub(crate) fn variant_info(node: &SyntaxNode, name: String) -> VariantInfo {
    if let Some(body) = node.child(K::STRUCT_BODY) {
        let fields: Vec<SyntaxNode> = body.children().filter(|f| f.kind() == K::FIELD).collect();
        VariantInfo {
            name,
            shape: Shape::Named,
            fields: fields.iter().map(name_of).collect(),
            types: fields
                .iter()
                .map(|f| f.children().find_map(ast::Type::cast))
                .collect(),
            default: false,
        }
    } else if let Some(list) = node.child(K::TUPLE_FIELDS) {
        let fields: Vec<SyntaxNode> = list
            .children()
            .filter(|f| f.kind() == K::TUPLE_FIELD)
            .collect();
        VariantInfo {
            name,
            shape: Shape::Tuple,
            fields: (0..fields.len()).map(|i| i.to_string()).collect(),
            types: fields
                .iter()
                .map(|f| f.children().find_map(ast::Type::cast))
                .collect(),
            default: false,
        }
    } else {
        VariantInfo {
            name,
            shape: Shape::Unit,
            fields: Vec::new(),
            types: Vec::new(),
            default: node
                .children()
                .filter(|c| c.kind() == K::OUTER_ATTR)
                .filter_map(|a| a.child(K::ATTR))
                .filter_map(|a| a.child(K::PATH))
                .any(|p| p.text().to_string().trim() == "default"),
        }
    }
}

/// The input of an `#[around(…)]` attribute, `attr`, parsed as an expression where it is in its
/// file: a function's path, or a call of one.
fn around_input(attr: &SyntaxNode) -> Option<ast::Expr> {
    let input = attr.child(K::ATTR_INPUT)?;
    let text = input.text().to_string();
    let inner = text.strip_prefix('(')?.strip_suffix(')')?;
    let start = input.text_range().start as usize + 1;
    // `#[around(` comes before it, so the prefix fits.
    let prefix = "fn f(){";
    let pad = start.checked_sub(prefix.len())?;
    let source = format!("{prefix}{}{inner}}}", " ".repeat(pad));
    let parse = moddil_syntax::parse(&source);
    if !parse.errors().is_empty() {
        return None;
    }
    let body = parse
        .syntax()
        .children()
        .find_map(ast::FnItem::cast)?
        .body()?;
    let e = body.syntax().children().find_map(ast::Expr::cast)?;
    Some(e)
}

/// The kinds of value one entry of `#[derive_for(…)]` stands for: a built-in type, or each
/// size of tuple `tuples(m..=n)` names.
fn for_heads(entry: &str) -> Vec<Head> {
    let sizes = entry
        .strip_prefix("tuples(")
        .and_then(|r| r.strip_suffix(')'))
        .and_then(|r| r.split_once("..="))
        .and_then(|(m, n)| {
            Some((
                m.trim().parse::<usize>().ok()?,
                n.trim().parse::<usize>().ok()?,
            ))
        });
    match sizes {
        Some((m, n)) => (m.max(2)..=n).map(Head::Tuple).collect(),
        None => vec![name_head(entry)],
    }
}

/// The kind of value built-in type `name` is (`i8`, `f64`), for `#[derive_for]`.
fn name_head(name: &str) -> Head {
    match name {
        "int" | "i64" => Head::Int,
        n if crate::num::Kind::named(n).is_some() => {
            Head::Num(crate::num::Kind::named(n).expect("a number type"))
        }
        "bool" => Head::Bool,
        "char" => Head::Char,
        "str" => Head::Str,
        _ => Head::Adt(name.to_string()),
    }
}

/// The types `#[derive_for(…)]` on function `node` lists (`comptime.derive.for.1`).
fn derive_for(node: &SyntaxNode) -> Option<Vec<String>> {
    node.children()
        .filter(|c| c.kind() == K::OUTER_ATTR)
        .find_map(|attr| {
            let text = attr.text().to_string();
            let inner = text
                .trim()
                .strip_prefix("#[derive_for(")?
                .trim_end()
                .strip_suffix(")]")?;
            Some(
                inner
                    .split(',')
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .map(str::to_string)
                    .collect(),
            )
        })
}

/// Derive function `def`'s template instantiated as an impl of trait `tr` for `head`: its
/// methods, run where `def` is written, with its parameter naming that type
/// (`comptime.template.1`).
fn template_impl(def: &FnDef, tr: String, head: Head) -> Option<ImplInfo> {
    let var = def
        .node
        .params()
        .and_then(|l| l.params().next())
        .and_then(|p| p.pat())
        .map(|p| p.syntax().text().trim().to_string())
        .unwrap_or_default();
    let template = def
        .node
        .body()
        .into_iter()
        .flat_map(|b| b.syntax().descendants())
        .find(|n| n.kind() == K::IMPL_ITEM)?;
    let self_ty = template
        .children()
        .filter_map(ast::Type::cast)
        .nth(1)
        .expect("a template's self type");
    let mut methods = HashMap::new();
    for m in template.child(K::IMPL_BODY).into_iter().flat_map(|b| {
        b.children()
            .filter_map(ast::FnItem::cast)
            .collect::<Vec<_>>()
    }) {
        let name = m.name().expect("a name").text();
        let mut fd = fn_def(&m, def.user, false, (def.file, def.module));
        fd.template = Some((var.clone(), head.clone()));
        let name = if fd.self_kind.is_none() && methods.contains_key(&name) {
            format!("{name} fn")
        } else {
            name
        };
        methods.insert(name, fd);
    }
    Some(ImplInfo {
        trait_: Some(tr),
        head,
        module: def.module,
        trait_path: None,
        self_ty,
        params: Vec::new(),
        methods,
        blanket: false,
    })
}
