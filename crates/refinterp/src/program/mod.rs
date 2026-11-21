//! A program's items, the prelude's and the file's, as tables: functions, structs and enums,
//! traits and impls. Built once, then read-only.
//!
//! [`dispatch`] answers which method a value's kind gets.

use std::collections::HashMap;

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{SyntaxKind as K, SyntaxNode};

use crate::PRELUDE;
use crate::value::{Head, Value};

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
}

#[derive(Clone, Debug)]
pub(crate) struct AdtInfo {
    pub(crate) is_enum: bool,
    pub(crate) variants: Vec<VariantInfo>,
    /// The type parameters' names.
    pub(crate) params: Vec<String>,
    /// The traits it derives.
    pub(crate) derives: Vec<String>,
    /// Whether its variants are named on their own (`Some`, `None`).
    pub(crate) bare: bool,
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
    /// Whether it is the file's code (not the prelude's).
    pub(crate) user: bool,
    /// Its own type parameters' names.
    pub(crate) params: Vec<String>,
    pub(crate) self_kind: Option<SelfKind>,
    /// Whether it is a destructor, whose `self` is not dropped again.
    pub(crate) destructor: bool,
}

pub(crate) struct ImplInfo {
    pub(crate) trait_: Option<String>,
    pub(crate) head: Head,
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

/// A program's items: the prelude's and the file's.
pub(crate) struct Program {
    pub(crate) text: String,
    pub(crate) fns: HashMap<String, FnDef>,
    pub(crate) adts: HashMap<String, AdtInfo>,
    pub(crate) traits: HashMap<String, TraitInfo>,
    pub(crate) impls: Vec<ImplInfo>,
    pub(crate) tests: Vec<(String, ast::Block)>,
}

impl Program {
    pub(crate) fn new(text: &str) -> Program {
        let mut p = Program {
            text: text.to_string(),
            fns: HashMap::new(),
            adts: HashMap::new(),
            traits: HashMap::new(),
            impls: Vec::new(),
            tests: Vec::new(),
        };
        let prelude = moddil_syntax::parse(PRELUDE).syntax();
        let file = moddil_syntax::parse(text).syntax();
        for (user, root) in [(false, prelude), (true, file)] {
            for item in root.children() {
                p.item(user, item);
            }
        }
        p
    }

    pub(crate) fn item(&mut self, user: bool, item: SyntaxNode) {
        match item.kind() {
            K::FN_ITEM => {
                let f = ast::FnItem::cast(item).expect("a function");
                let name = f.name().expect("a checked function has a name").text();
                self.fns.insert(name, fn_def(&f, user, false));
            }
            K::STRUCT_ITEM | K::ENUM_ITEM => {
                let name = name_of(&item);
                let variants = if item.kind() == K::STRUCT_ITEM {
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
                self.adts.insert(
                    name.clone(),
                    AdtInfo {
                        is_enum: item.kind() == K::ENUM_ITEM,
                        variants,
                        params: generic_names(&item),
                        derives,
                        bare: !user && name == "Option",
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
                                (name, fn_def(&f, user, false))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                self.traits.insert(name_of(&item), TraitInfo { methods });
            }
            K::IMPL_ITEM => {
                let types: Vec<ast::Type> = item.children().filter_map(ast::Type::cast).collect();
                let has_trait = item.token(K::FOR_KW).is_some();
                let trait_ = has_trait.then(|| first_name(types[0].syntax()));
                let self_ty = types[usize::from(has_trait)].clone();
                let destructor = trait_.as_deref() == Some("Drop");
                let methods = item
                    .child(K::IMPL_BODY)
                    .map(|b| {
                        b.children()
                            .filter_map(ast::FnItem::cast)
                            .map(|f| {
                                let name = f.name().expect("a name").text();
                                (name, fn_def(&f, user, destructor))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                self.impls.push(ImplInfo {
                    trait_,
                    head: head_of_type(&self_ty),
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
                self.tests
                    .push((name, t.body().expect("a checked test has a body")));
            }
            _ => {}
        }
    }

    /// The struct or variant a path names: `Point`, `Shape::Circle`, `Some`.
    pub(crate) fn ctor(&self, segments: &[String]) -> Option<(String, &VariantInfo)> {
        match segments {
            [name] => {
                if let Some(adt) = self.adts.get(name).filter(|a| !a.is_enum) {
                    return Some((name.clone(), &adt.variants[0]));
                }
                self.adts.iter().find_map(|(ty, a)| {
                    if a.bare {
                        a.variants
                            .iter()
                            .find(|v| v.name == *name)
                            .map(|v| (ty.clone(), v))
                    } else {
                        None
                    }
                })
            }
            [ty, variant] => {
                let adt = self.adts.get(ty).filter(|a| a.is_enum)?;
                adt.variants
                    .iter()
                    .find(|v| v.name == *variant)
                    .map(|v| (ty.clone(), v))
            }
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

    /// `v` as Moddil source.
    pub(crate) fn render(&self, v: &Value) -> String {
        match v {
            Value::Int(i) => i.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Unit => "()".to_string(),
            Value::Tuple(xs) if xs.len() == 1 => format!("({},)", self.render(&xs[0])),
            Value::Tuple(xs) => {
                let xs: Vec<String> = xs.iter().map(|x| self.render(x)).collect();
                format!("({})", xs.join(", "))
            }
            Value::Ref(..) => unreachable!("a borrow is shown by the machine"),
            Value::Moved => "<moved>".to_string(),
            Value::Adt {
                ty,
                variant,
                fields,
            } => {
                let adt = &self.adts[ty];
                let path = if adt.bare {
                    variant.clone()
                } else if adt.is_enum {
                    format!("{ty}::{variant}")
                } else {
                    ty.clone()
                };
                let info = self.variant(ty, variant);
                match info.shape {
                    Shape::Unit => path,
                    Shape::Tuple => {
                        let xs: Vec<String> = fields.iter().map(|x| self.render(x)).collect();
                        format!("{path}({})", xs.join(", "))
                    }
                    Shape::Named => {
                        let xs: Vec<String> = info
                            .fields
                            .iter()
                            .zip(fields)
                            .map(|(n, x)| format!("{n}: {}", self.render(x)))
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
                .filter(|c| c.kind() == K::TYPE_PARAM)
                .map(|p| name_of(&p))
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn fn_def(f: &ast::FnItem, user: bool, destructor: bool) -> FnDef {
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
    FnDef {
        node: f.clone(),
        user,
        params: generic_names(f.syntax()),
        self_kind,
        destructor,
    }
}

/// The kind of value a type written in an impl's header has.
pub(crate) fn head_of_type(t: &ast::Type) -> Head {
    match t {
        ast::Type::Tuple(n) if n.children().next().is_none() => Head::Unit,
        ast::Type::Tuple(_) => Head::Tuple,
        ast::Type::Ref(_) => Head::Ref,
        _ => match first_name(t.syntax()).as_str() {
            "int" => Head::Int,
            "bool" => Head::Bool,
            name => Head::Adt(name.to_string()),
        },
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
        }
    } else {
        VariantInfo {
            name,
            shape: Shape::Unit,
            fields: Vec::new(),
            types: Vec::new(),
        }
    }
}
