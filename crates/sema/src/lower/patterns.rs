//! Patterns, and the names they bind.

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{SyntaxKind as K, SyntaxNode, TextRange};

use super::{Binds, Lowerer};
use crate::hir::{Local, LocalId, PatId, PatKind};
use crate::items::{Def, Shape};
use crate::{Code, Diagnostic, Ty};

impl Lowerer<'_> {
    pub(super) fn pat(&mut self, p: &ast::Pat, b: &mut Binds) -> PatId {
        let node = p.syntax().clone();
        let range = node.text_range();
        let kind = match p {
            ast::Pat::Wildcard(_) => PatKind::Wild,
            ast::Pat::Literal(_) => self.literal_pat(&node),
            ast::Pat::Range(_) => self.range_pat(&node),
            ast::Pat::Ident(_) => {
                let mutable = node.token(K::MUT_KW).is_some();
                let name = node.child(K::NAME).and_then(ast::Name::cast);
                let sub = node.children().find_map(ast::Pat::cast);
                let sub = sub.map(|s| self.pat(&s, b));
                match name {
                    Some(n) => {
                        let local = self.bind(&n.text(), mutable, n.syntax().text_range(), b);
                        PatKind::Bind { local, sub }
                    }
                    None => PatKind::Missing,
                }
            }
            ast::Pat::At(_) => {
                let sub = node.children().find_map(ast::Pat::cast);
                let sub = sub.map(|s| self.pat(&s, b));
                match self.segments(&node).as_deref() {
                    Some([name]) => {
                        let at = node.child(K::PATH).map_or(range, |n| n.text_range());
                        let local = self.bind(name, false, at, b);
                        PatKind::Bind { local, sub }
                    }
                    _ => PatKind::Missing,
                }
            }
            ast::Pat::Path(_) => self.path_pat(&node, b),
            ast::Pat::TupleStruct(_) => self.tuple_struct_pat(&node, b),
            ast::Pat::Struct(_) => self.struct_pat(&node, b),
            ast::Pat::Tuple(_) => {
                let subs: Vec<ast::Pat> = node.children().filter_map(ast::Pat::cast).collect();
                if subs.is_empty() {
                    PatKind::Unit
                } else if subs.len() == 1
                    && node.token(K::COMMA).is_none()
                    && !matches!(subs[0], ast::Pat::Rest(_))
                {
                    // `(p)` is `p` in parentheses.
                    return self.pat(&subs[0], b);
                } else {
                    let (pats, rest) = self.with_rest(&subs, b);
                    PatKind::Tuple { pats, rest }
                }
            }
            ast::Pat::Or(_) => self.or_pat(&node, b),
            ast::Pat::Rest(_) => {
                self.report(
                    Code::E0001,
                    range,
                    "`..` is only allowed among the fields of a tuple or struct pattern",
                );
                PatKind::Missing
            }
            ast::Pat::Ref(_) => {
                let mutable = node.token(K::MUT_KW).is_some();
                match node.children().find_map(ast::Pat::cast) {
                    Some(inner) => PatKind::Ref {
                        mutable,
                        pat: self.pat(&inner, b),
                    },
                    None => PatKind::Missing,
                }
            }
            ast::Pat::Typed(_) => {
                self.error(Diagnostic::unsupported(range, "a refinement test"));
                PatKind::Missing
            }
            _ => {
                self.error(Diagnostic::unsupported(range, "this pattern"));
                PatKind::Missing
            }
        };
        self.alloc_pat(kind, range)
    }

    /// A new local for `name`, or the one the first alternative of an or-pattern bound.
    pub(super) fn bind(
        &mut self,
        name: &str,
        mutable: bool,
        range: TextRange,
        b: &mut Binds,
    ) -> LocalId {
        if let Some(&(_, _, first)) = b.names.iter().find(|(n, _, _)| n == name) {
            let d = Diagnostic::error(
                Code::E0106,
                range,
                format!("`{name}` is bound more than once in this pattern"),
            )
            .with_label(first, "first bound here");
            self.error(d);
        }
        let local = match b.reuse.iter().find(|(n, _)| n == name) {
            Some(&(_, l)) => l,
            None => self.out.body.locals.alloc(Local {
                name: name.to_string(),
                mutable,
                range,
            }),
        };
        b.names.push((name.to_string(), local, range));
        local
    }

    /// The patterns among `subs`, and where `..` is among them.
    pub(super) fn with_rest(
        &mut self,
        subs: &[ast::Pat],
        b: &mut Binds,
    ) -> (Vec<PatId>, Option<usize>) {
        let mut pats = Vec::new();
        let mut rest = None;
        for s in subs {
            if let ast::Pat::Rest(r) = s {
                if rest.is_some() {
                    self.report(
                        Code::E0001,
                        r.text_range(),
                        "`..` may appear only once in a pattern",
                    );
                }
                rest = Some(pats.len());
            } else {
                pats.push(self.pat(s, b));
            }
        }
        (pats, rest)
    }

    pub(super) fn literal_pat(&mut self, node: &SyntaxNode) -> PatKind {
        let range = node.text_range();
        let negate = node.token(K::MINUS).is_some();
        let Some(tok) = node
            .child_tokens()
            .find(|t| !t.kind().is_trivia() && t.kind() != K::MINUS)
        else {
            return PatKind::Missing;
        };
        match tok.kind() {
            K::TRUE_KW => PatKind::Bool(true),
            K::FALSE_KW => PatKind::Bool(false),
            K::INTEGER => match ast::int_value(tok.text()) {
                Some(v) => PatKind::Int(if negate { -v } else { v }),
                None => {
                    self.report(
                        Code::E0204,
                        range,
                        format!("`{}` is too large for any integer type", tok.text()),
                    );
                    PatKind::Missing
                }
            },
            _ => {
                self.error(Diagnostic::unsupported(range, "this kind of literal"));
                PatKind::Missing
            }
        }
    }

    pub(super) fn range_pat(&mut self, node: &SyntaxNode) -> PatKind {
        let range = node.text_range();
        let bounds: Vec<SyntaxNode> = node
            .children()
            .filter(|c| c.kind() == K::LITERAL_PAT)
            .collect();
        let inclusive = node.token(K::DOT_DOT_EQ).is_some();
        let mut values = Vec::new();
        for b in &bounds {
            match self.literal_pat(b) {
                PatKind::Int(v) => values.push(v),
                PatKind::Missing => return PatKind::Missing,
                _ => {
                    self.report(
                        Code::E0201,
                        b.text_range(),
                        "the bounds of a range pattern are integers",
                    );
                    return PatKind::Missing;
                }
            }
        }
        match values.as_slice() {
            [lo] => PatKind::Range {
                lo: *lo,
                hi: None,
                inclusive,
            },
            [lo, hi] => PatKind::Range {
                lo: *lo,
                hi: Some(*hi),
                inclusive,
            },
            _ => {
                self.error(Diagnostic::unsupported(range, "this range pattern"));
                PatKind::Missing
            }
        }
    }

    /// `x` (a binding), a unit struct, or a unit variant `E::A`.
    pub(super) fn path_pat(&mut self, node: &SyntaxNode, b: &mut Binds) -> PatKind {
        let range = node.text_range();
        let Some(segments) = self.segments(node) else {
            return PatKind::Missing;
        };
        if let [name] = segments.as_slice()
            && !matches!(self.item(name), Some(Def::Adt(_) | Def::Variant(..)))
            && name != "Self"
        {
            let local = self.bind(name, false, range, b);
            return PatKind::Bind { local, sub: None };
        }
        let Some((adt, variant)) = self.ctor_pat_target(node) else {
            return PatKind::Missing;
        };
        let def = self.items.variant(adt, variant);
        if def.shape != Shape::Unit {
            let path = self.items.adts[adt as usize].path(variant);
            let form = if def.shape == Shape::Tuple {
                format!("{path}(…)")
            } else {
                format!("{path} {{ … }}")
            };
            self.report(
                Code::E0111,
                range,
                format!("`{path}` has fields: match it as `{form}`"),
            );
            return PatKind::Missing;
        }
        PatKind::Adt {
            adt,
            variant,
            fields: Vec::new(),
        }
    }

    /// The struct or variant a pattern's path names.
    pub(super) fn ctor_pat_target(&mut self, node: &SyntaxNode) -> Option<(u32, u32)> {
        let range = node.text_range();
        let segments = self.segments(node)?;
        if let [name] = segments.as_slice()
            && let Some(Def::Variant(a, v)) = self.item(name)
        {
            return Some((a, v));
        }
        let self_adt = match &self.types.self_ty {
            Some(Ty::Adt(a, ..)) => Some(*a),
            _ => None,
        };
        let adt_named = |cx: &Self, name: &str| {
            if name == "Self" {
                self_adt
            } else {
                match cx.item(name) {
                    Some(Def::Adt(a)) => Some(a),
                    _ => None,
                }
            }
        };
        // A path through modules: `crate::shapes::Shape::Circle`, `shapes::Point`.
        if segments.len() > 2 || (segments.len() == 2 && adt_named(self, &segments[0]).is_none()) {
            let names: Vec<&str> = segments.iter().map(String::as_str).collect();
            let found = self.items.path(self.types.module, &names, false);
            return match found {
                Ok((Def::Adt(a), n))
                    if n == names.len() && !self.items.adts[a as usize].is_enum =>
                {
                    Some((a, 0))
                }
                Ok((Def::Variant(a, v), n)) if n == names.len() => Some((a, v)),
                Ok((Def::Adt(a), n)) if n + 1 == names.len() => {
                    let adt = &self.items.adts[a as usize];
                    match adt.variant(names[n]).filter(|_| adt.is_enum) {
                        Some(v) => Some((a, v)),
                        None => {
                            self.report(
                                Code::E0101,
                                range,
                                format!("`{}` has no variant called `{}`", adt.name, names[n]),
                            );
                            None
                        }
                    }
                }
                Err(crate::PathError::Private(_, def)) => {
                    let d = self.items.hidden(def, range);
                    self.error(d);
                    None
                }
                _ => {
                    self.report(
                        Code::E0101,
                        range,
                        format!(
                            "there is no struct or variant called `{}`",
                            names.join("::")
                        ),
                    );
                    None
                }
            };
        }
        match segments.as_slice() {
            [name] => match adt_named(self, name) {
                Some(a) if !self.items.adts[a as usize].is_enum => Some((a, 0)),
                Some(_) => {
                    self.report(
                        Code::E0111,
                        range,
                        format!("`{name}` is an enum; match one of its variants: `{name}::…`"),
                    );
                    None
                }
                None => {
                    self.report(
                        Code::E0101,
                        range,
                        format!("there is no struct called `{name}`"),
                    );
                    None
                }
            },
            [ty, variant] => {
                let Some(a) = adt_named(self, ty) else {
                    self.report(
                        Code::E0101,
                        range,
                        format!("there is no enum called `{ty}`"),
                    );
                    return None;
                };
                let adt = &self.items.adts[a as usize];
                match adt.variant(variant).filter(|_| adt.is_enum) {
                    Some(v) => Some((a, v)),
                    None => {
                        self.report(
                            Code::E0101,
                            range,
                            format!("`{ty}` has no variant called `{variant}`"),
                        );
                        None
                    }
                }
            }
            _ => {
                self.error(Diagnostic::unsupported(range, "this path"));
                None
            }
        }
    }

    /// `Pair(a, b)` or `Shape::Circle(r)`, maybe with `..`.
    pub(super) fn tuple_struct_pat(&mut self, node: &SyntaxNode, b: &mut Binds) -> PatKind {
        let range = node.text_range();
        let target = self.ctor_pat_target(node);
        let subs: Vec<ast::Pat> = node.children().filter_map(ast::Pat::cast).collect();
        let (pats, rest) = self.with_rest(&subs, b);
        let Some((adt, variant)) = target else {
            return PatKind::Missing;
        };
        let def = self.items.variant(adt, variant);
        let path = self.items.adts[adt as usize].path(variant);
        if def.shape != Shape::Tuple {
            self.report(
                Code::E0111,
                range,
                format!("`{path}` has no positional fields"),
            );
            return PatKind::Missing;
        }
        let n = def.fields.len();
        let fits = match rest {
            Some(_) => pats.len() <= n,
            None => pats.len() == n,
        };
        if !fits {
            self.report(
                Code::E0213,
                range,
                format!(
                    "`{path}` has {n} field{}, but this pattern has {}",
                    if n == 1 { "" } else { "s" },
                    pats.len()
                ),
            );
            return PatKind::Missing;
        }
        let fields = match rest {
            None => pats
                .into_iter()
                .enumerate()
                .map(|(i, p)| (i as u32, p))
                .collect(),
            Some(at) => {
                let after = pats.len() - at;
                pats.into_iter()
                    .enumerate()
                    .map(|(i, p)| {
                        let index = if i < at { i } else { n - after + (i - at) };
                        (index as u32, p)
                    })
                    .collect()
            }
        };
        PatKind::Adt {
            adt,
            variant,
            fields,
        }
    }

    /// `Point { x, y: 0, .. }`.
    pub(super) fn struct_pat(&mut self, node: &SyntaxNode, b: &mut Binds) -> PatKind {
        let range = node.text_range();
        let target = self.ctor_pat_target(node);
        let target = target.filter(|&(adt, variant)| {
            let def = self.items.variant(adt, variant);
            if def.shape == Shape::Named {
                return true;
            }
            let path = self.items.adts[adt as usize].path(variant);
            self.report(Code::E0111, range, format!("`{path}` has no named fields"));
            false
        });
        let mut fields: Vec<(u32, PatId)> = Vec::new();
        let mut given: Vec<(u32, TextRange)> = Vec::new();
        let mut rest = false;
        for f in node.children().filter(|c| c.kind() == K::FIELD_PAT) {
            if f.token(K::DOT_DOT).is_some() {
                rest = true;
                continue;
            }
            let Some(name_tok) = ast::member_name(&f) else {
                continue;
            };
            let name = name_tok.text().to_string();
            let pat = match f.children().find_map(ast::Pat::cast) {
                Some(p) => self.pat(&p, b),
                None => {
                    let local = self.bind(&name, false, name_tok.text_range(), b);
                    self.alloc_pat(PatKind::Bind { local, sub: None }, f.text_range())
                }
            };
            let Some((adt, variant)) = target else {
                continue;
            };
            let def = self.items.variant(adt, variant);
            match def.field(&name) {
                Some(i) => {
                    if let Some(&(_, first)) = given.iter().find(|(g, _)| *g == i) {
                        let d = Diagnostic::error(
                            Code::E0108,
                            name_tok.text_range(),
                            format!("the field `{name}` is matched twice"),
                        )
                        .with_label(first, "first matched here");
                        self.error(d);
                    } else {
                        given.push((i, name_tok.text_range()));
                        fields.push((i, pat));
                    }
                }
                None => {
                    let path = self.items.adts[adt as usize].path(variant);
                    self.report(
                        Code::E0107,
                        name_tok.text_range(),
                        format!("`{path}` has no field called `{name}`"),
                    );
                }
            }
        }
        let Some((adt, variant)) = target else {
            return PatKind::Missing;
        };
        let def = self.items.variant(adt, variant);
        let missing: Vec<String> = (0..def.fields.len() as u32)
            .filter(|i| !given.iter().any(|(g, _)| g == i))
            .map(|i| format!("`{}`", def.fields[i as usize].name))
            .collect();
        if !rest && !missing.is_empty() {
            let path = self.items.adts[adt as usize].path(variant);
            self.report(
                Code::E0207,
                range,
                format!(
                    "this pattern leaves out {} of `{path}`; match them or write `..`",
                    missing.join(", ")
                ),
            );
        }
        PatKind::Adt {
            adt,
            variant,
            fields,
        }
    }

    /// `a | b`: every alternative binds the same names, to the same locals.
    pub(super) fn or_pat(&mut self, node: &SyntaxNode, b: &mut Binds) -> PatKind {
        let before = b.names.len();
        let saved = b.reuse.clone();
        let mut first: Option<Vec<(String, LocalId, TextRange)>> = None;
        let mut alts = Vec::new();
        for alt in node.children().filter_map(ast::Pat::cast) {
            b.names.truncate(before);
            if let Some(f) = &first {
                b.reuse = saved.clone();
                b.reuse.extend(f.iter().map(|(n, l, _)| (n.clone(), *l)));
            }
            let id = self.pat(&alt, b);
            let bound: Vec<(String, LocalId, TextRange)> = b.names[before..].to_vec();
            match &first {
                None => first = Some(bound),
                Some(f) => {
                    let alt_range = alt.syntax().text_range();
                    for (n, _, _) in f {
                        if !bound.iter().any(|(m, _, _)| m == n) {
                            self.report(
                                Code::E0105,
                                alt_range,
                                format!("`{n}` is bound in the first alternative but not this one"),
                            );
                        }
                    }
                    for (n, _, r) in &bound {
                        if !f.iter().any(|(m, _, _)| m == n) {
                            self.report(
                                Code::E0105,
                                *r,
                                format!("`{n}` is not bound in the first alternative"),
                            );
                        }
                    }
                }
            }
            alts.push(id);
        }
        b.names.truncate(before);
        b.names.extend(first.unwrap_or_default());
        b.reuse = saved;
        PatKind::Or(alts)
    }
}
