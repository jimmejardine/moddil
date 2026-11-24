//! Lowering a derive's template (`comptime.template.1`): in a method instantiated from a derive
//! function, `inline for f in fields(t) { … }` is the block once per field of the struct, and
//! `f` names that field there: `f.name` is its name, `x.[f]` and `[f]: e` the field itself,
//! `f.is_refined()` whether its type is a refinement and `f.new(e)` that refinement's `new`.
//! `if const c { … } else { … }` is the branch `c` chooses, `c` being such reflection. What
//! comes out is ordinary HIR, checked like hand-written code.

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{SyntaxKind as K, SyntaxNode, TextRange};

use super::Lowerer;
use crate::hir::{Arm, ExprId, ExprKind, Local, PatKind, Stmt};
use crate::{Code, Ty};

/// A field: of variant `variant` (0 for a struct or a tuple), the `k`th.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FieldRef {
    pub(super) variant: u32,
    pub(super) k: u32,
}

/// What a name an `inline for` binds reflects: a field, or an enum's variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Reflect {
    Field(FieldRef),
    Variant(u32),
}

impl Lowerer<'_> {
    /// What a reflected name `node` (a path of one name) stands for, if it is one.
    fn reflected_any(&self, node: &SyntaxNode) -> Option<Reflect> {
        if node.kind() != K::PATH_EXPR {
            return None;
        }
        let path = node.child(K::PATH)?;
        let segs = ast::Path::cast(path)?.segments();
        let [name] = segs.as_slice() else {
            return None;
        };
        self.reflected
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, r)| *r)
    }

    /// The field a reflected name `node` stands for, if it names a field.
    pub(super) fn reflected(&self, node: &SyntaxNode) -> Option<FieldRef> {
        match self.reflected_any(node)? {
            Reflect::Field(f) => Some(f),
            Reflect::Variant(_) => None,
        }
    }

    /// The variant a reflected name `node` stands for, if it names a variant
    /// (`comptime.template.variant.1`).
    pub(super) fn reflected_variant(&self, node: &SyntaxNode) -> Option<u32> {
        match self.reflected_any(node)? {
            Reflect::Variant(v) => Some(v),
            Reflect::Field(_) => None,
        }
    }

    /// For a struct literal `node` whose path is a reflected variant (`v { [f]: e, … }`): the
    /// template's enum and that variant (`comptime.template.variant.1`).
    pub(super) fn variant_literal(&self, node: &SyntaxNode) -> Option<(u32, u32)> {
        let path = node.child(K::PATH).and_then(ast::Path::cast)?;
        let segs = path.segments();
        let [one] = segs.as_slice() else {
            return None;
        };
        let v = self
            .reflected
            .iter()
            .rev()
            .find(|(n, _)| n == one)
            .and_then(|(_, r)| match r {
                Reflect::Variant(v) => Some(*v),
                Reflect::Field(_) => None,
            })?;
        Some((self.template_enum()?, v))
    }

    /// `t.name`, for `base` the template's parameter: the name of the type it is instantiated
    /// for, as written in its declaration, without type arguments (`comptime.template.1`).
    pub(super) fn template_name(&self, base: &SyntaxNode) -> Option<String> {
        if !self.names_template(base) {
            return None;
        }
        let t = self.template?;
        Some(match t.adt {
            Some(adt) => self.items.adts[adt as usize].name.clone(),
            None => t.ty.to_string(),
        })
    }

    /// Whether `node` is the template's parameter, `t`.
    fn names_template(&self, node: &SyntaxNode) -> bool {
        self.template
            .is_some_and(|t| node.kind() == K::PATH_EXPR && node.text().trim() == t.var)
    }

    /// The enum the template is instantiated for, if it is one.
    fn template_enum(&self) -> Option<u32> {
        let adt = self.template?.adt?;
        self.items.adts[adt as usize].is_enum.then_some(adt)
    }

    /// How many fields the template's type has: a struct's, or a tuple's elements, named `0`,
    /// `1`, …; `None` for a type with none to reflect on (an enum has variants instead).
    fn field_count(&self) -> Option<u32> {
        let t = self.template?;
        if self.template_enum().is_some() {
            return None;
        }
        match (t.adt, &t.ty) {
            (Some(adt), _) => Some(self.items.variant(adt, 0).fields.len() as u32),
            (None, Ty::Tuple(elems)) => Some(elems.len() as u32),
            _ => None,
        }
    }

    /// Field `f` of the template's type (of its variant, for an enum): its name and its type. A
    /// tuple's elements are named `0`, `1`, …
    pub(super) fn field_of(&self, f: FieldRef) -> (String, Ty) {
        let FieldRef { variant, k } = f;
        if let Some(t) = self.template
            && t.adt.is_none()
            && let Ty::Tuple(elems) = &t.ty
        {
            let ty = elems.get(k as usize).cloned().unwrap_or(Ty::Error);
            return (k.to_string(), ty);
        }
        let Some((t, adt)) = self.template.and_then(|t| t.adt.map(|a| (t, a))) else {
            return (String::new(), Ty::Error);
        };
        let name = self
            .items
            .variant(adt, variant)
            .fields
            .get(k as usize)
            .map(|f| f.name.clone())
            .unwrap_or_default();
        let ty = self
            .items
            .field_types(&t.ty, variant)
            .get(k as usize)
            .cloned()
            .unwrap_or(Ty::Error);
        (name, ty)
    }

    /// What `inline for`'s head ranges over, and the name it binds: `inline for f in fields(t)`;
    /// for an enum, `inline for v in variants(t)`, and in one, `inline for f in fields(v)`
    /// (`comptime.template.variant.1`). The name, and what it reflects each time. E1007
    /// otherwise.
    fn inline_head(&mut self, node: &SyntaxNode) -> Option<(String, Vec<Reflect>)> {
        let range = node.text_range();
        let Some(t) = self.template else {
            self.report(
                Code::E1007,
                range,
                "`inline for` unrolls only in a derive's template (`comptime.template.1`)",
            );
            return None;
        };
        let name = node
            .children()
            .find_map(ast::Pat::cast)
            .filter(|p| matches!(p.syntax().kind(), K::IDENT_PAT | K::PATH_PAT))
            .map(|p| p.syntax().text().trim().to_string());
        let iter = node.children().find_map(ast::Expr::cast);
        let head: String = iter
            .as_ref()
            .map(|e| {
                e.syntax()
                    .text()
                    .to_string()
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .collect()
            })
            .unwrap_or_default();
        let fields_of_t = head == format!("fields({})", t.var);
        let variants_of_t = head == format!("variants({})", t.var);
        // `fields(v)`, for a reflected variant `v`: that variant's fields.
        let of_variant = head
            .strip_prefix("fields(")
            .and_then(|r| r.strip_suffix(')'))
            .and_then(|v| {
                self.reflected
                    .iter()
                    .rev()
                    .find(|(n, _)| n == v)
                    .and_then(|(_, r)| match r {
                        Reflect::Variant(k) => Some(*k),
                        Reflect::Field(_) => None,
                    })
            });
        if let (Some(n), Some(v), Some(adt)) = (&name, of_variant, self.template_enum()) {
            let count = self.items.variant(adt, v).fields.len() as u32;
            let each = (0..count)
                .map(|k| Reflect::Field(FieldRef { variant: v, k }))
                .collect();
            return Some((n.clone(), each));
        }
        match name {
            Some(n) if fields_of_t => {
                let Some(count) = self.field_count() else {
                    let why = if self.template_enum().is_some() {
                        format!(
                            "an enum has no fields to unroll over: its variants are \
                             `variants({})` (`comptime.template.variant.1`)",
                            t.var
                        )
                    } else {
                        "a template for a built-in type other than a tuple has no fields to \
                         unroll over (`comptime.derive.for.1`)"
                            .to_string()
                    };
                    self.report(Code::E1007, range, why);
                    return None;
                };
                let each = (0..count)
                    .map(|k| Reflect::Field(FieldRef { variant: 0, k }))
                    .collect();
                Some((n, each))
            }
            Some(n) if variants_of_t => {
                let Some(adt) = self.template_enum() else {
                    self.report(
                        Code::E1007,
                        range,
                        "only an enum has variants to unroll over (`comptime.template.variant.1`)",
                    );
                    return None;
                };
                let count = self.items.adts[adt as usize].variants.len() as u32;
                Some((n, (0..count).map(Reflect::Variant).collect()))
            }
            _ => {
                self.report(
                    Code::E1007,
                    range,
                    format!(
                        "`inline for` is written `inline for f in fields({})`: once for each \
                         field, `f` naming it",
                        t.var
                    ),
                );
                None
            }
        }
    }

    /// `inline for f in fields(t) { … }`: the block once for each field, in order, as a block
    /// of those blocks.
    pub(super) fn inline_for(&mut self, node: &SyntaxNode) -> ExprId {
        let range = node.text_range();
        let Some((name, each)) = self.inline_head(node) else {
            return self.missing(range);
        };
        let Some(block) = node.child(K::BLOCK).and_then(ast::Block::cast) else {
            return self.missing(range);
        };
        let mut stmts = Vec::new();
        for r in each {
            self.reflected.push((name.clone(), r));
            let expr = self.block(&block);
            self.reflected.pop();
            stmts.push(Stmt::Expr {
                expr,
                semi: true,
                observe: false,
            });
        }
        self.alloc(
            ExprKind::Block {
                stmts,
                tail: None,
                observe_tail: false,
            },
            range,
        )
    }

    /// A struct literal's fields, as names, where each name is, and values: `name: e` and
    /// `name`, and in a template `[f]: e` and `inline for f in fields(t) { … }`.
    pub(super) fn struct_inits(&mut self, node: &SyntaxNode) -> Vec<(String, TextRange, ExprId)> {
        let mut out = Vec::new();
        for init in node.children() {
            let init_range = init.text_range();
            match init.kind() {
                K::FIELD_INIT => {
                    if init.token(K::DOT_DOT).is_some() {
                        self.error(crate::Diagnostic::unsupported(
                            init_range,
                            "struct update syntax",
                        ));
                        continue;
                    }
                    let Some(name_tok) = ast::member_name(&init) else {
                        continue;
                    };
                    let name = name_tok.text().to_string();
                    let value = match init.children().find_map(ast::Expr::cast) {
                        Some(v) => self.expr(&v),
                        None => match self.lookup(&name) {
                            Some(l) => self.alloc(ExprKind::Local(l), name_tok.text_range()),
                            None => {
                                self.report(
                                    Code::E0101,
                                    name_tok.text_range(),
                                    format!("there is nothing called `{name}` here"),
                                );
                                self.missing(init_range)
                            }
                        },
                    };
                    out.push((name, name_tok.text_range(), value));
                }
                K::REFLECTED_FIELD_INIT => {
                    let exprs: Vec<ast::Expr> =
                        init.children().filter_map(ast::Expr::cast).collect();
                    let field = exprs.first().and_then(|e| self.reflected(e.syntax()));
                    let value = match exprs.get(1) {
                        Some(v) => self.expr(v),
                        None => self.missing(init_range),
                    };
                    match field {
                        Some(k) => {
                            let (name, _) = self.field_of(k);
                            out.push((name, init_range, value));
                        }
                        None => self.report(
                            Code::E1007,
                            init_range,
                            "`[f]: e` gives a reflected field `f`: one an `inline for` names",
                        ),
                    }
                }
                K::INLINE_FOR_FIELDS => {
                    let Some((name, each)) = self.inline_head(&init) else {
                        continue;
                    };
                    if each.iter().any(|r| matches!(r, Reflect::Variant(_))) {
                        self.report(
                            Code::E1007,
                            init_range,
                            "a struct literal's `inline for` unrolls over fields: `fields(t)`, \
                             or `fields(v)` for a variant `v`",
                        );
                        continue;
                    }
                    for r in each {
                        self.reflected.push((name.clone(), r));
                        let inner = self.struct_inits(&init);
                        self.reflected.pop();
                        out.extend(inner);
                    }
                }
                K::SPREAD => {
                    self.error(crate::Diagnostic::unsupported(
                        init_range,
                        "struct update syntax",
                    ));
                }
                _ => {}
            }
        }
        out
    }

    /// `x.[f]`: field `f` of `x`.
    pub(super) fn reflected_field(&mut self, node: &SyntaxNode) -> ExprId {
        let range = node.text_range();
        let exprs: Vec<ast::Expr> = node.children().filter_map(ast::Expr::cast).collect();
        let base = match exprs.first() {
            Some(b) => self.expr(b),
            None => self.missing(range),
        };
        match exprs.get(1).and_then(|f| self.reflected(f.syntax())) {
            // A variant's field: `x` is a borrow of an enum value, and the field is reached
            // through a match (`comptime.template.variant.1`). If `x` is another variant, it
            // traps.
            Some(f) if self.template_enum().is_some() => self.variant_field(base, f, range),
            Some(k) => {
                let (name, _) = self.field_of(k);
                self.alloc(
                    ExprKind::Field {
                        base,
                        name,
                        name_range: range,
                    },
                    range,
                )
            }
            None => {
                self.report(
                    Code::E1007,
                    range,
                    "`x.[f]` names a reflected field `f`: one an `inline for` names",
                );
                self.missing(range)
            }
        }
    }

    /// Field `f` of `base`, a borrow of a value of the template's enum that is `f`'s variant:
    /// `*match base { V { k: x, .. } => x, _ => { assert(false); loop {} } }`, a place of the
    /// field's type.
    fn variant_field(&mut self, base: ExprId, f: FieldRef, range: TextRange) -> ExprId {
        let Some(adt) = self.template_enum() else {
            return self.missing(range);
        };
        let local = self.out.body.locals.alloc(Local {
            name: "{field}".to_string(),
            mutable: false,
            range,
            closure: self.closure,
        });
        let bind = self.alloc_pat(PatKind::Bind { local, sub: None }, range);
        let pat = self.alloc_pat(
            PatKind::Adt {
                adt,
                variant: f.variant,
                fields: vec![(f.k, bind)],
            },
            range,
        );
        let wild = self.alloc_pat(PatKind::Wild, range);
        let value = self.alloc(ExprKind::Local(local), range);
        let no = self.alloc(ExprKind::Bool(false), range);
        let assert = self.alloc(ExprKind::Assert(no), range);
        let empty = self.alloc(
            ExprKind::Block {
                stmts: Vec::new(),
                tail: None,
                observe_tail: false,
            },
            range,
        );
        let never = self.alloc(ExprKind::Loop { body: empty }, range);
        let trap = self.alloc(
            ExprKind::Block {
                stmts: vec![Stmt::Expr {
                    expr: assert,
                    semi: true,
                    observe: false,
                }],
                tail: Some(never),
                observe_tail: false,
            },
            range,
        );
        let projected = self.alloc(
            ExprKind::Match {
                scrutinee: base,
                arms: vec![
                    Arm {
                        pat,
                        guard: None,
                        body: value,
                    },
                    Arm {
                        pat: wild,
                        guard: None,
                        body: trap,
                    },
                ],
            },
            range,
        );
        self.alloc(ExprKind::Deref(projected), range)
    }

    /// `f.name` of reflected field `k`: its name, as text.
    pub(super) fn reflected_member(
        &mut self,
        k: FieldRef,
        member: &str,
        range: TextRange,
    ) -> ExprId {
        if member == "name" {
            let (name, _) = self.field_of(k);
            return self.alloc(ExprKind::Str(name), range);
        }
        self.report(
            Code::E1007,
            range,
            format!("a reflected field has `name`, `is_refined()` and `new(e)`, not `{member}`"),
        );
        self.missing(range)
    }

    /// `v.name` of reflected variant `k`: its name, as text (`comptime.template.variant.1`).
    pub(super) fn variant_member(&mut self, k: u32, member: &str, range: TextRange) -> ExprId {
        if member == "name"
            && let Some(adt) = self.template_enum()
        {
            let name = self.items.variant(adt, k).name.clone();
            return self.alloc(ExprKind::Str(name), range);
        }
        // `v.index`: the variant's place in the enum's order, from 0, as an `int`.
        if member == "index" {
            return self.alloc(ExprKind::Int(i128::from(k).into()), range);
        }
        self.report(
            Code::E1007,
            range,
            format!(
                "a reflected variant has `name`, `index`, `is(x)` and `make()`, not `{member}`"
            ),
        );
        self.missing(range)
    }

    /// `v.is(x)` and `v.make()` of reflected variant `k`: whether `x` is that variant, and its
    /// value (`comptime.template.variant.1`).
    pub(super) fn variant_call(
        &mut self,
        k: u32,
        method: &str,
        args: &[ast::Expr],
        range: TextRange,
    ) -> ExprId {
        let Some(adt) = self.template_enum() else {
            return self.missing(range);
        };
        let ty_args = match self.template.map(|t| &t.ty) {
            Some(Ty::Adt(_, _, args)) => args.to_vec(),
            _ => Vec::new(),
        };
        match (method, args) {
            ("make", []) if !self.items.variant(adt, k).fields.is_empty() => {
                self.report(
                    Code::E1007,
                    range,
                    "`v.make()` makes a unit variant; this one has fields, so it is written \
                     `v { inline for f in fields(v) { [f]: e, } }`",
                );
                self.missing(range)
            }
            ("make", []) => self.alloc(
                ExprKind::Adt {
                    adt,
                    variant: k,
                    fields: Vec::new(),
                    args: ty_args,
                },
                range,
            ),
            ("is", [x]) => {
                let scrutinee = self.expr(x);
                let pat = self.alloc_pat(
                    PatKind::Adt {
                        adt,
                        variant: k,
                        fields: Vec::new(),
                    },
                    range,
                );
                let wild = self.alloc_pat(PatKind::Wild, range);
                let yes = self.alloc(ExprKind::Bool(true), range);
                let no = self.alloc(ExprKind::Bool(false), range);
                self.alloc(
                    ExprKind::Match {
                        scrutinee,
                        arms: vec![
                            Arm {
                                pat,
                                guard: None,
                                body: yes,
                            },
                            Arm {
                                pat: wild,
                                guard: None,
                                body: no,
                            },
                        ],
                    },
                    range,
                )
            }
            _ => {
                self.report(
                    Code::E1007,
                    range,
                    format!("a reflected variant has `name`, `is(x)` and `make()`, not `{method}`"),
                );
                self.missing(range)
            }
        }
    }

    /// `f.is_refined()` and `f.new(e)` of reflected field `k`.
    pub(super) fn reflected_call(
        &mut self,
        k: FieldRef,
        method: &str,
        args: &[ast::Expr],
        range: TextRange,
    ) -> ExprId {
        let (_, ty) = self.field_of(k);
        let refined = match ty {
            Ty::Refined(t, _) => self.items.types[t as usize]
                .refinement
                .as_ref()
                .map(|r| r.new),
            _ => None,
        };
        match (method, args) {
            ("is_refined", []) => self.alloc(ExprKind::Bool(refined.is_some()), range),
            ("new", [arg]) => match refined {
                Some(new) => {
                    let callee = self.alloc(
                        ExprKind::Fn {
                            func: new,
                            args: Vec::new(),
                        },
                        range,
                    );
                    let arg = self.expr(arg);
                    self.alloc(
                        ExprKind::Call {
                            callee,
                            args: vec![arg],
                        },
                        range,
                    )
                }
                None => {
                    self.report(
                        Code::E1007,
                        range,
                        "`f.new(e)` makes a refined field's value; this field's type is no \
                         refinement (ask `f.is_refined()` with `if const`)",
                    );
                    self.missing(range)
                }
            },
            _ => {
                self.report(
                    Code::E1007,
                    range,
                    format!(
                        "a reflected field has `name`, `is_refined()` and `new(e)`, not \
                         `{method}`"
                    ),
                );
                self.missing(range)
            }
        }
    }

    /// The value of `if const`'s condition: reflection (`f.is_refined()`, `f.name == "x"`),
    /// `true`, `false`, `!`, `&&` and `||`. E1007 otherwise.
    pub(super) fn comptime_bool(&mut self, e: &ast::Expr) -> Option<bool> {
        let node = e.syntax();
        let subs = e.sub_exprs();
        let op = node
            .child_tokens()
            .find(|t| !t.kind().is_trivia())
            .map(|t| t.kind());
        let value = match e {
            ast::Expr::Paren(_) => subs.first().and_then(|s| self.comptime_bool(s)),
            ast::Expr::Literal(_) => match node.tokens().first().map(|t| t.kind()) {
                Some(K::TRUE_KW) => Some(true),
                Some(K::FALSE_KW) => Some(false),
                _ => None,
            },
            ast::Expr::Prefix(_) if op == Some(K::BANG) => {
                subs.first().and_then(|s| self.comptime_bool(s)).map(|b| !b)
            }
            ast::Expr::Bin(_) | ast::Expr::CompareChain(_)
                if matches!(op, Some(K::EQ_EQ | K::BANG_EQ)) =>
            {
                self.compare_names(node, &subs, op == Some(K::EQ_EQ))
            }
            ast::Expr::Bin(_) if matches!(op, Some(K::AMP_AMP | K::PIPE_PIPE)) => {
                let a = subs.first().and_then(|s| self.comptime_bool(s))?;
                let b = subs.get(1).and_then(|s| self.comptime_bool(s))?;
                Some(if op == Some(K::AMP_AMP) {
                    a && b
                } else {
                    a || b
                })
            }
            ast::Expr::MethodCall(_) => {
                let method = ast::member_name(node)?.text().to_string();
                let receiver = subs.first()?;
                // `t.is_enum()`: whether the template is instantiated for an enum; and the
                // shape of a struct `t` or a variant `v` (`comptime.template.variant.1`).
                let shape = |this: &Self, adt: u32, variant: u32| {
                    let def = this.items.variant(adt, variant);
                    match method.as_str() {
                        "is_unit" => Some(def.fields.is_empty()),
                        "is_positional" => {
                            Some(!def.fields.is_empty() && def.shape != crate::items::Shape::Named)
                        }
                        _ => None,
                    }
                };
                if self.names_template(receiver.syntax()) {
                    match method.as_str() {
                        "is_enum" => Some(self.template_enum().is_some()),
                        _ => match self.template.and_then(|t| t.adt) {
                            Some(adt) if self.template_enum().is_none() => shape(self, adt, 0),
                            _ => None,
                        },
                    }
                } else if let Some(v) = self.reflected_variant(receiver.syntax()) {
                    let adt = self.template_enum()?;
                    if method == "is_default" {
                        // Marked `#[default]` (`attr.derive.default.1`).
                        Some(self.items.variant(adt, v).default)
                    } else {
                        shape(self, adt, v)
                    }
                } else {
                    let k = self.reflected(receiver.syntax())?;
                    let (_, ty) = self.field_of(k);
                    (method == "is_refined").then_some(matches!(ty, Ty::Refined(..)))
                }
            }
            _ => None,
        };
        if value.is_none() {
            self.report(
                Code::E1007,
                node.text_range(),
                "`if const` decides by reflection: `f.is_refined()`, `f.name == \"…\"`, \
                 `t.is_enum()`, `is_unit()` and `is_positional()` of `t` or a variant `v`, \
                 `true` and `false`, with `!`, `&&` and `||`",
            );
        }
        value
    }

    /// `f.name == "x"` (or `!=`, either way round): whether the names are equal (or not).
    fn compare_names(&mut self, node: &SyntaxNode, subs: &[ast::Expr], eq: bool) -> Option<bool> {
        let _ = node;
        let [a, b] = subs else {
            return None;
        };
        let name = |this: &Self, x: &ast::Expr| -> Option<String> {
            if let ast::Expr::Field(_) = x {
                let base = x.sub_exprs().into_iter().next()?;
                let k = this.reflected(base.syntax())?;
                let m = ast::member_name(x.syntax())?;
                if m.text() == "name" {
                    return Some(this.field_of(k).0);
                }
            }
            None
        };
        let text = |x: &ast::Expr| -> Option<String> {
            let t = x.syntax().text();
            let t = t.trim();
            t.strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .map(str::to_string)
        };
        let (l, r) = match (name(self, a), name(self, b)) {
            (Some(l), _) => (l, text(b)?),
            (_, Some(r)) => (text(a)?, r),
            _ => return None,
        };
        Some((l == r) == eq)
    }

    /// `if const c { … } else { … }`: the branch `c` chooses, in a template.
    pub(super) fn if_const(&mut self, node: &SyntaxNode, e: &ast::Expr) -> ExprId {
        let range = node.text_range();
        if self.template.is_none() {
            return self.unsupported(range, "`if const` outside a derive's template");
        }
        let subs = e.sub_exprs();
        let Some(chosen) = subs.first().and_then(|c| self.comptime_bool(c)) else {
            return self.missing(range);
        };
        match (chosen, subs.get(1), subs.get(2)) {
            (true, Some(then), _) => self.expr(then),
            (false, _, Some(other)) => self.expr(other),
            (false, _, None) => self.alloc(ExprKind::Unit, range),
            _ => self.missing(range),
        }
    }
}
