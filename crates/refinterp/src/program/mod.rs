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
use crate::{CORE_COLLECTIONS, PRELUDE};

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
}

pub(crate) struct TraitInfo {
    pub(crate) methods: HashMap<String, FnDef>,
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
    pub(crate) fns: HashMap<String, FnDef>,
    pub(crate) adts: HashMap<String, AdtInfo>,
    pub(crate) traits: HashMap<String, TraitInfo>,
    pub(crate) impls: Vec<ImplInfo>,
    pub(crate) tests: Vec<TestDef>,
    pub(crate) modules: Vec<Module>,
    mods: Vec<ModEntry>,
    /// The const items, by key: their initialiser and the module whose names it uses.
    pub(crate) consts: HashMap<String, (ast::Expr, usize)>,
    /// The `type` items, by key (`refine.1`).
    pub(crate) types: HashMap<String, TypeInfo>,
    /// Each file's module path.
    file_paths: Vec<Vec<String>>,
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
        // The built-in `core` mod and its `collections` module; every mod depends on it.
        let core = p.mods.len();
        let core_root = p.modules.len();
        p.modules.push(Module {
            mod_: core,
            ..Module::default()
        });
        let collections = p.modules.len();
        p.modules.push(Module {
            mod_: core,
            ..Module::default()
        });
        p.modules[core_root]
            .children
            .insert("collections".to_string(), collections);
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
        let core_items = moddil_syntax::parse(CORE_COLLECTIONS).syntax();
        for item in core_items.children() {
            p.item(None, collections, item);
        }
        for (f, file) in files.iter().enumerate() {
            let root = moddil_syntax::parse(&file.text).syntax();
            for item in root.children() {
                p.item(Some(f as u32), file_modules[f], item);
            }
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
        p
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
            ast::Type::Tuple(_) => Head::Tuple,
            ast::Type::Array(_) | ast::Type::Slice(_) => Head::Array,
            ast::Type::Ref(_) => Head::Ref,
            _ => {
                let path = type_path(t.syntax());
                match path
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .as_slice()
                {
                    ["int"] => Head::Int,
                    ["bool"] => Head::Bool,
                    ["char"] => Head::Char,
                    ["str"] => Head::Str,
                    ["String"] => Head::String,
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
                self.traits.insert(key, TraitInfo { methods });
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
                let methods = item
                    .child(K::IMPL_BODY)
                    .map(|b| {
                        b.children()
                            .filter_map(ast::FnItem::cast)
                            .map(|f| {
                                let name = f.name().expect("a name").text();
                                (name, fn_def(&f, user, destructor, site))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                self.impls.push(ImplInfo {
                    trait_: None,
                    head: Head::Unit,
                    module: module as u32,
                    trait_path,
                    self_ty,
                    params: generic_names(&item),
                    methods,
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
            Value::Bool(b) => b.to_string(),
            Value::Char(c) => format!("{c:?}"),
            Value::Str(s) => format!("{:?}", &**s),
            Value::String(s) => format!("{s:?}"),
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
        path.extend(type_path(&p));
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
