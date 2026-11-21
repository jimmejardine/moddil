//! Patterns: what a value is matched against, and the names a match binds (`pat.mode.1`).

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{SyntaxKind as K, SyntaxNode};

use crate::program::name_of;
use crate::value::{Pointer, Value};

use super::{Machine, is_ctor, literal, member_name, segments};

impl Machine<'_> {
    /// The value a source holds (looking through a borrow, for patterns that test it).
    pub(super) fn value_of(&self, src: &Src) -> Value {
        match src {
            Src::Val(v) => self.target(v.clone()),
            Src::Place(p, _) => self.target(self.memory.load(p)),
            Src::At(ptr, _) => self.memory.load(ptr),
        }
    }

    /// What a name bound to `src` holds: the value (moved out of its place), or a borrow of the
    /// place.
    pub(super) fn bound(&self, src: &Src) -> (Value, Option<Pointer>) {
        match src {
            Src::Val(v) => (v.clone(), None),
            Src::Place(p, indirect) => (self.memory.load(p), (!indirect).then(|| p.clone())),
            Src::At(ptr, m) => (Value::Ref(ptr.clone(), *m), None),
        }
    }

    /// The source a pattern that tests a value sees: matching a borrow tests the place it
    /// points to, and names inside bind borrows of its parts (`pat.mode.1`).
    pub(super) fn enter(&self, src: &Src) -> Src {
        match src {
            Src::Val(Value::Ref(ptr, m)) | Src::At(ptr, m) => Src::At(ptr.clone(), *m),
            Src::Place(p, indirect) => match self.memory.load(p) {
                Value::Ref(ptr, m) => Src::At(ptr, m),
                _ => Src::Place(p.clone(), *indirect),
            },
            Src::Val(v) => Src::Val(v.clone()),
        }
    }

    /// Part `i` of the value a source holds.
    pub(super) fn part(&self, src: &Src, i: usize) -> Src {
        let sub = |p: &Pointer| {
            let mut q = p.clone();
            q.path.push(i);
            q
        };
        match src {
            Src::Val(Value::Tuple(xs) | Value::Adt { fields: xs, .. }) => Src::Val(xs[i].clone()),
            Src::Val(_) => unreachable!("a checked pattern"),
            Src::Place(p, indirect) => Src::Place(sub(p), *indirect),
            Src::At(ptr, m) => Src::At(sub(ptr), *m),
        }
    }

    pub(super) fn bind_to(&self, name: String, src: &Src, binds: &mut Vec<Bind>) {
        let (v, from) = self.bound(src);
        binds.push((name, v, from));
    }

    /// Whether `src` matches `p`; if it does, `binds` gets the names it binds (`pat.1`).
    pub(super) fn matches(&self, p: &ast::Pat, src: &Src, binds: &mut Vec<Bind>) -> bool {
        let node = p.syntax();
        match p {
            ast::Pat::Wildcard(_) => true,
            ast::Pat::Literal(_) => literal_pat(node) == self.value_of(src),
            ast::Pat::Range(_) => {
                let bounds: Vec<Value> = node
                    .children()
                    .filter(|c| c.kind() == K::LITERAL_PAT)
                    .map(|b| literal_pat(&b))
                    .collect();
                let (Value::Int(x), Some(Value::Int(lo))) = (self.value_of(src), bounds.first())
                else {
                    unreachable!("a checked range pattern");
                };
                let below = match bounds.get(1) {
                    None => true,
                    Some(Value::Int(hi)) if node.token(K::DOT_DOT_EQ).is_some() => x <= *hi,
                    Some(Value::Int(hi)) => x < *hi,
                    Some(_) => unreachable!("an integer bound"),
                };
                *lo <= x && below
            }
            ast::Pat::Ident(_) => {
                if let Some(sub) = node.children().find_map(ast::Pat::cast)
                    && !self.matches(&sub, src, binds)
                {
                    return false;
                }
                self.bind_to(name_of(node), src, binds);
                true
            }
            ast::Pat::At(_) => {
                let sub = node.children().find_map(ast::Pat::cast).expect("a pattern");
                if !self.matches(&sub, src, binds) {
                    return false;
                }
                self.bind_to(segments(node)[0].clone(), src, binds);
                true
            }
            ast::Pat::Ref(_) => {
                // `&p` copies what the borrow points to.
                let sub = node.children().find_map(ast::Pat::cast).expect("a pattern");
                self.matches(&sub, &Src::Val(self.value_of(src)), binds)
            }
            ast::Pat::Path(_) => {
                let segments = segments(node);
                match self.ctor(&segments) {
                    Some((ty, info)) => is_ctor(&self.value_of(src), &ty, &info.name),
                    None => {
                        self.bind_to(segments[0].clone(), src, binds);
                        true
                    }
                }
            }
            ast::Pat::TupleStruct(_) => {
                let (ty, info) = self.ctor(&segments(node)).expect("a checked pattern");
                if !is_ctor(&self.value_of(src), &ty, &info.name) {
                    return false;
                }
                let n = info.fields.len();
                self.positional(node, &self.enter(src), n, binds)
            }
            ast::Pat::Struct(_) => {
                let (ty, info) = self.ctor(&segments(node)).expect("a checked pattern");
                if !is_ctor(&self.value_of(src), &ty, &info.name) {
                    return false;
                }
                let info = info.clone();
                let src = self.enter(src);
                for f in node.children().filter(|c| c.kind() == K::FIELD_PAT) {
                    if f.token(K::DOT_DOT).is_some() {
                        continue;
                    }
                    let name = member_name(&f);
                    let i = info
                        .fields
                        .iter()
                        .position(|n| *n == name)
                        .expect("a checked field");
                    let part = self.part(&src, i);
                    match f.children().find_map(ast::Pat::cast) {
                        Some(sub) => {
                            if !self.matches(&sub, &part, binds) {
                                return false;
                            }
                        }
                        None => self.bind_to(name, &part, binds),
                    }
                }
                true
            }
            ast::Pat::Tuple(_) => {
                let subs: Vec<ast::Pat> = node.children().filter_map(ast::Pat::cast).collect();
                if subs.is_empty() {
                    return self.value_of(src) == Value::Unit;
                }
                if subs.len() == 1
                    && node.token(K::COMMA).is_none()
                    && !matches!(subs[0], ast::Pat::Rest(_))
                {
                    return self.matches(&subs[0], src, binds);
                }
                let Value::Tuple(xs) = self.value_of(src) else {
                    unreachable!("a checked tuple pattern")
                };
                self.positional(node, &self.enter(src), xs.len(), binds)
            }
            ast::Pat::Or(_) => node.children().filter_map(ast::Pat::cast).any(|alt| {
                let mut tried = Vec::new();
                let ok = self.matches(&alt, src, &mut tried);
                if ok {
                    binds.extend(tried);
                }
                ok
            }),
            _ => unreachable!("a checked pattern of slices S0 to S3"),
        }
    }

    /// The positional sub-patterns of `node` (with an optional `..`) against the `len` parts of
    /// `src`.
    pub(super) fn positional(
        &self,
        node: &SyntaxNode,
        src: &Src,
        len: usize,
        binds: &mut Vec<Bind>,
    ) -> bool {
        let subs: Vec<ast::Pat> = node.children().filter_map(ast::Pat::cast).collect();
        let rest = subs.iter().position(|s| matches!(s, ast::Pat::Rest(_)));
        let pats: Vec<&ast::Pat> = subs
            .iter()
            .filter(|s| !matches!(s, ast::Pat::Rest(_)))
            .collect();
        let n = pats.len();
        pats.iter().enumerate().all(|(i, p)| {
            // After `..`, patterns match the last elements.
            let at = match rest {
                Some(r) if i >= r => len - (n - i),
                _ => i,
            };
            self.matches(p, &self.part(src, at), binds)
        })
    }
}

/// What a pattern is matched against (`pat.mode.1`): a place the value is in, whose parts the
/// names move out (unless the place is reached through a borrow, when they are copied); a place
/// a borrow points to, whose parts the names borrow; or a value, which the names copy.
pub(super) enum Src {
    Place(Pointer, bool),
    At(Pointer, bool),
    Val(Value),
}

/// A name bound by a pattern: its value, and the place it is moved out of, if it is.
pub(super) type Bind = (String, Value, Option<Pointer>);

pub(super) fn literal_pat(node: &SyntaxNode) -> Value {
    literal(node, node.token(K::MINUS).is_some())
}
