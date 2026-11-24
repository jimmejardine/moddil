//! Exhaustiveness and reachability of patterns, by Maranget's usefulness algorithm ("Warnings for
//! pattern matching", 2007).
//!
//! A pattern vector `v` is *useful* with respect to a matrix of rows if some value matches `v` but
//! no row. A `match` is exhaustive if `_` is not useful after all its unguarded arms, and an arm
//! is unreachable if its pattern is not useful after the unguarded arms before it. Integer ranges
//! are split at every boundary that appears in the column, so each piece is either inside or
//! outside every range.

use moddil_syntax::TextRange;

use crate::check::{Inference, tuple_positions};
use crate::hir::{Arm, Body, PatId, PatKind};
use crate::items::Shape;
use crate::{Code, Diagnostic, ItemTree, Ty};

/// A constructor: what a pattern's head matches.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Ctor {
    /// `true` or `false`.
    Bool(bool),
    /// The integers from `lo` to `hi`, both included (`lo > hi` matches nothing).
    Int(i128, i128),
    /// The only constructor of a tuple, struct or `()`.
    Single,
    /// An enum's variant.
    Variant(u32),
    /// A string literal.
    Str(String),
    /// Any string no literal among the rows is: strings are too many to list.
    OtherStr,
    /// An array or slice of exactly this many elements, or (`true`) of at least this many: the
    /// lengths are split at every length the rows name, so each is one or the other.
    Len(usize, bool),
    /// A slice pattern with `..`: so many elements before it and so many after (its fields, in
    /// that order), of any length from their sum up.
    Rest(usize, usize),
    /// Anything.
    Wild,
    /// Any of the alternatives (held in the pattern's fields).
    Or,
}

/// A pattern taken apart: its constructor, its type and its fields' patterns.
#[derive(Clone, Debug)]
struct DPat {
    ctor: Ctor,
    ty: Ty,
    fields: Vec<DPat>,
}

impl DPat {
    fn wild(ty: Ty) -> DPat {
        DPat {
            ctor: Ctor::Wild,
            ty,
            fields: Vec::new(),
        }
    }
}

/// The largest code point.
const CHAR_MAX: i128 = 0x10_FFFF;

const INT_MIN: i128 = i64::MIN as i128;
const INT_MAX: i128 = i64::MAX as i128;

/// The greatest value of integer type `ty` the analysis counts (`int`'s for any other type).
fn int_top(ty: &Ty) -> i128 {
    match ty {
        // The analysis counts to one past the greatest value, so an `i128`'s or a `u128`'s top
        // value is left out of it.
        Ty::Num(n) if !n.is_float() => n.int_range().1.min(i128::MAX - 1),
        _ => INT_MAX,
    }
}

/// The least value of integer type `ty` (`int`'s for any other type).
fn int_bottom(ty: &Ty) -> i128 {
    match ty {
        Ty::Num(n) if !n.is_float() => n.int_range().0,
        _ => INT_MIN,
    }
}

struct Cx<'a> {
    items: &'a ItemTree,
    body: &'a Body,
    types: &'a Inference,
}

impl Cx<'_> {
    /// The types of the fields of constructor `c` of type `ty`.
    fn field_types(&self, c: &Ctor, ty: &Ty) -> Vec<Ty> {
        // A borrow has the constructors of what it points to.
        match (c, ty.strip_ref()) {
            (Ctor::Single, Ty::Tuple(ts)) => ts.to_vec(),
            (Ctor::Single, t @ Ty::Adt(..)) => self.items.field_types(t, 0),
            (Ctor::Variant(v), t @ Ty::Adt(..)) => self.items.field_types(t, *v),
            (Ctor::Len(n, _), Ty::Array(t, _) | Ty::Slice(t)) => vec![(**t).clone(); *n],
            _ => Vec::new(),
        }
    }

    fn lower(&self, p: PatId) -> DPat {
        let ty = self.types.pat(p);
        match &self.body.pats[p].kind {
            PatKind::Missing | PatKind::Wild | PatKind::Bind { sub: None, .. } => DPat::wild(ty),
            &PatKind::Bind { sub: Some(s), .. }
            | &PatKind::Refined { pat: s, .. }
            | &PatKind::Ref { pat: s, .. }
            | &PatKind::OfType { pat: s, .. } => self.lower(s),
            &PatKind::Int(v) => {
                // `i128::MAX` is past the counted range: it stands for its top.
                let v = v.min(int_top(&self.items.base(ty.strip_ref())).max(INT_MAX));
                DPat {
                    ctor: Ctor::Int(v, v),
                    ty,
                    fields: Vec::new(),
                }
            }
            &PatKind::Bool(b) => DPat {
                ctor: Ctor::Bool(b),
                ty,
                fields: Vec::new(),
            },
            &PatKind::Char(c) => {
                let c = i128::from(u32::from(c));
                DPat {
                    ctor: Ctor::Int(c, c),
                    ty,
                    fields: Vec::new(),
                }
            }
            PatKind::Str(s) => DPat {
                ctor: Ctor::Str(s.clone()),
                ty,
                fields: Vec::new(),
            },
            &PatKind::Range {
                lo,
                hi,
                inclusive,
                chars,
            } => {
                let top = int_top(&self.items.base(ty.strip_ref())).max(INT_MAX);
                let hi = match hi {
                    None if chars => CHAR_MAX,
                    None => int_top(&self.items.base(ty.strip_ref())),
                    Some(h) if inclusive => h.min(top),
                    Some(h) => (h - 1).min(top),
                };
                let lo = lo.min(top);
                DPat {
                    ctor: Ctor::Int(lo, hi),
                    ty,
                    fields: Vec::new(),
                }
            }
            PatKind::Unit => DPat {
                ctor: Ctor::Single,
                ty,
                fields: Vec::new(),
            },
            PatKind::Slice { pats, rest, .. } => {
                // A pattern of the wrong length for its array is already an error (E0213).
                if let Ty::Array(_, n) = ty.strip_ref()
                    && let Ty::Len(n) = **n
                    && (pats.len() > n as usize || (rest.is_none() && pats.len() != n as usize))
                {
                    return DPat::wild(ty);
                }
                let fields: Vec<DPat> = pats.iter().map(|&s| self.lower(s)).collect();
                let ctor = match rest {
                    Some(r) => Ctor::Rest(*r, pats.len() - r),
                    None => Ctor::Len(pats.len(), false),
                };
                DPat { ctor, ty, fields }
            }
            PatKind::Tuple { pats, rest } => {
                let types = self.field_types(&Ctor::Single, &ty);
                let mut fields: Vec<DPat> = types.iter().cloned().map(DPat::wild).collect();
                for (&s, i) in pats
                    .iter()
                    .zip(tuple_positions(pats.len(), *rest, types.len()))
                {
                    if i < fields.len() {
                        fields[i] = self.lower(s);
                    }
                }
                DPat {
                    ctor: Ctor::Single,
                    ty,
                    fields,
                }
            }
            PatKind::Adt {
                adt,
                variant,
                fields: given,
            } => {
                let ctor = if self.items.adts[*adt as usize].is_enum {
                    Ctor::Variant(*variant)
                } else {
                    Ctor::Single
                };
                let types = match ty.strip_ref() {
                    t @ Ty::Adt(..) => self.items.field_types(t, *variant),
                    _ => vec![Ty::Error; self.items.variant(*adt, *variant).fields.len()],
                };
                let mut fields: Vec<DPat> = types.into_iter().map(DPat::wild).collect();
                for &(i, s) in given {
                    fields[i as usize] = self.lower(s);
                }
                DPat { ctor, ty, fields }
            }
            PatKind::Or(alts) => DPat {
                ctor: Ctor::Or,
                ty,
                fields: alts.iter().map(|&a| self.lower(a)).collect(),
            },
        }
    }

    /// Every constructor of `ty`, with integers split at the boundaries of the ranges in `heads`.
    /// `None` for a type whose values are not analysed (an error).
    fn all_ctors(&self, ty: &Ty, heads: &[&DPat]) -> Option<Vec<Ctor>> {
        Some(match ty.strip_ref() {
            Ty::Bool => vec![Ctor::Bool(false), Ctor::Bool(true)],
            Ty::Refined(..) => return self.all_ctors(&self.items.base(ty.strip_ref()), heads),
            Ty::Int => split_ints(heads, INT_MIN, INT_MAX),
            Ty::Num(n) if !n.is_float() => {
                // The analysis counts to one past the greatest value, so an `i128`'s or a
                // `u128`'s top value is left out of it.
                split_ints(heads, int_bottom(ty.strip_ref()), int_top(ty.strip_ref()))
            }
            // A float is matched by bindings and `_` only.
            Ty::Num(_) => vec![Ctor::Single],
            Ty::Char => split_ints(heads, 0, CHAR_MAX),
            // The literals the rows name, and the strings they do not.
            Ty::Str => {
                let mut cs: Vec<Ctor> = Vec::new();
                for h in heads {
                    if let Ctor::Str(_) = &h.ctor
                        && !cs.contains(&h.ctor)
                    {
                        cs.push(h.ctor.clone());
                    }
                }
                cs.push(Ctor::OtherStr);
                cs
            }
            // An array has its length; a slice any, split at the lengths the rows name, the
            // longest standing for all longer.
            Ty::Array(_, n) => match **n {
                Ty::Len(n) => vec![Ctor::Len(n as usize, false)],
                _ => vec![Ctor::Single],
            },
            Ty::Slice(_) => {
                let longest = heads
                    .iter()
                    .map(|h| match h.ctor {
                        Ctor::Len(n, _) => n,
                        Ctor::Rest(a, b) => a + b,
                        _ => 0,
                    })
                    .max()
                    .unwrap_or(0);
                let mut cs: Vec<Ctor> = (0..=longest).map(|n| Ctor::Len(n, false)).collect();
                cs.push(Ctor::Len(longest + 1, true));
                cs
            }
            // A value of a type parameter can only be bound: one constructor covers it.
            Ty::Unit
            | Ty::Tuple(_)
            | Ty::Buf(_)
            | Ty::Ptr(..)
            | Ty::FnPtr(..)
            | Ty::Param(..)
            | Ty::Opaque(_)
            | Ty::Assoc(_)
            | Ty::Dyn(..)
            | Ty::Closure(_)
            | Ty::Anon(_) => {
                vec![Ctor::Single]
            }
            Ty::Adt(a, ..) => {
                let def = &self.items.adts[*a as usize];
                if def.is_enum {
                    (0..def.variants.len() as u32).map(Ctor::Variant).collect()
                } else {
                    vec![Ctor::Single]
                }
            }
            Ty::Never => Vec::new(),
            Ty::Error | Ty::Ref(..) | Ty::Infer(_) | Ty::Errors(_) | Ty::Len(_) => return None,
        })
    }

    /// A value matched by `v` and by no row of `rows`, if there is one: the witness, one pattern
    /// per column.
    fn useful(&self, rows: &[Vec<DPat>], v: &[DPat]) -> Option<Vec<Witness>> {
        let Some(head) = v.first() else {
            return rows.is_empty().then(Vec::new);
        };
        // Expand or-patterns at the head of the rows.
        if rows.iter().any(|r| r[0].ctor == Ctor::Or) {
            let mut expanded = Vec::new();
            for r in rows {
                expand(r, &mut expanded);
            }
            return self.useful(&expanded, v);
        }
        if head.ctor == Ctor::Or {
            for alt in &head.fields {
                let mut w = vec![alt.clone()];
                w.extend_from_slice(&v[1..]);
                if let Some(found) = self.useful(rows, &w) {
                    return Some(found);
                }
            }
            return None;
        }
        let ty = &head.ty;
        let ctors = match &head.ctor {
            Ctor::Wild => {
                let heads: Vec<&DPat> = rows.iter().map(|r| &r[0]).collect();
                self.all_ctors(ty, &heads)?
            }
            // A slice pattern with `..` covers lengths split as `_`'s are.
            Ctor::Rest(..) => {
                let mut heads: Vec<&DPat> = rows.iter().map(|r| &r[0]).collect();
                heads.push(head);
                self.all_ctors(ty, &heads)?
            }
            Ctor::Int(lo, hi) => {
                let mut heads: Vec<&DPat> = rows.iter().map(|r| &r[0]).collect();
                heads.push(head);
                split_ints(&heads, *lo, *hi)
            }
            c => vec![c.clone()],
        };
        for c in ctors {
            let types = self.field_types(&c, ty);
            let specialised: Vec<Vec<DPat>> = rows
                .iter()
                .filter_map(|r| specialise(r, &c, &types))
                .collect();
            let Some(sv) = specialise(v, &c, &types) else {
                continue;
            };
            if let Some(mut w) = self.useful(&specialised, &sv) {
                let fields: Vec<Witness> = w.drain(..types.len()).collect();
                let mut out = vec![Witness::Ctor(c, ty.clone(), fields)];
                out.extend(w);
                return Some(out);
            }
        }
        None
    }
}

fn expand(row: &[DPat], out: &mut Vec<Vec<DPat>>) {
    if row[0].ctor == Ctor::Or {
        for alt in &row[0].fields {
            let mut r = vec![alt.clone()];
            r.extend_from_slice(&row[1..]);
            expand(&r, out);
        }
    } else {
        out.push(row.to_vec());
    }
}

/// The ranges from `lo` to `hi` split at every boundary of the integer patterns in `heads`.
fn split_ints(heads: &[&DPat], lo: i128, hi: i128) -> Vec<Ctor> {
    if lo > hi {
        return Vec::new();
    }
    // One past `hi`, saturating: `i128`'s greatest value is left out of the analysis, as is a
    // `u128`'s past it.
    let end = hi.saturating_add(1);
    let mut cuts: Vec<i128> = vec![lo, end];
    for h in heads {
        if let Ctor::Int(a, b) = h.ctor
            && a <= b
        {
            cuts.push(a.clamp(lo, end));
            cuts.push(b.saturating_add(1).clamp(lo, end));
        }
    }
    cuts.sort_unstable();
    cuts.dedup();
    cuts.windows(2).map(|w| Ctor::Int(w[0], w[1] - 1)).collect()
}

/// `row` with its head replaced by the fields of constructor `c`, if the head can match `c`.
fn specialise(row: &[DPat], c: &Ctor, types: &[Ty]) -> Option<Vec<DPat>> {
    let head = &row[0];
    let mut out: Vec<DPat> = match (&head.ctor, c) {
        (Ctor::Wild, _) => types.iter().cloned().map(DPat::wild).collect(),
        // A pattern with `..` matches each length from its count up: the elements before `..`,
        // `_` for those it stands for, and those after.
        (Ctor::Rest(a, b), Ctor::Len(n, _)) if a + b <= *n => {
            let mut out: Vec<DPat> = head.fields[..*a].to_vec();
            out.extend(types[*a..n - b].iter().cloned().map(DPat::wild));
            out.extend_from_slice(&head.fields[*a..]);
            out
        }
        (Ctor::Len(m, false), Ctor::Len(n, false)) if m == n => head.fields.clone(),
        // `c` is a piece that lies wholly inside or outside the head's range.
        (Ctor::Int(a, b), Ctor::Int(lo, hi)) if a <= lo && hi <= b => Vec::new(),
        (h, c) if h == c => head.fields.clone(),
        _ => return None,
    };
    out.extend_from_slice(&row[1..]);
    Some(out)
}

/// A value no pattern matches, for messages.
#[derive(Clone, Debug)]
enum Witness {
    Ctor(Ctor, Ty, Vec<Witness>),
}

impl Witness {
    fn render(&self, items: &ItemTree) -> String {
        let Witness::Ctor(c, ty, fields) = self;
        let fields: Vec<String> = fields.iter().map(|f| f.render(items)).collect();
        match (c, ty.strip_ref()) {
            (Ctor::Bool(b), _) => b.to_string(),
            (Ctor::Str(s), _) => format!("{s:?}"),
            (Ctor::OtherStr, _) | (Ctor::Int(0, CHAR_MAX), Ty::Char) => "_".to_string(),
            (Ctor::Int(lo, hi), Ty::Char) => {
                let c = |v: &i128| {
                    u32::try_from(*v)
                        .ok()
                        .and_then(char::from_u32)
                        .map_or_else(|| format!("'\\u{{{v:x}}}'"), |c| format!("{c:?}"))
                };
                if lo == hi {
                    c(lo)
                } else {
                    format!("{}..={}", c(lo), c(hi))
                }
            }
            (Ctor::Int(lo, hi), t) if (*lo, *hi) == (int_bottom(t), int_top(t)) => "_".to_string(),
            (Ctor::Int(lo, hi), _) if lo == hi => lo.to_string(),
            (Ctor::Int(lo, hi), t) if *hi == int_top(t) => format!("{lo}.."),
            (Ctor::Int(lo, hi), _) => format!("{lo}..={hi}"),
            (Ctor::Single, Ty::Unit) => "()".to_string(),
            (Ctor::Single, Ty::Tuple(_)) if fields.len() == 1 => format!("({},)", fields[0]),
            (Ctor::Single, Ty::Tuple(_)) => format!("({})", fields.join(", ")),
            (Ctor::Single, Ty::Adt(a, ..)) => adt_witness(items, *a, 0, &fields),
            (Ctor::Variant(v), Ty::Adt(a, ..)) => adt_witness(items, *a, *v, &fields),
            (Ctor::Len(_, false), _) => format!("[{}]", fields.join(", ")),
            (Ctor::Len(_, true), _) => format!("[{}, ..]", fields.join(", ")),
            _ => "_".to_string(),
        }
    }
}

fn adt_witness(items: &ItemTree, adt: u32, variant: u32, fields: &[String]) -> String {
    let path = items.adts[adt as usize].path(variant);
    let def = items.variant(adt, variant);
    match def.shape {
        Shape::Unit => path,
        Shape::Tuple => format!("{path}({})", fields.join(", ")),
        Shape::Named => {
            let all_wild = fields.iter().all(|f| f == "_");
            if all_wild {
                format!("{path} {{ .. }}")
            } else {
                let named: Vec<String> = def
                    .fields
                    .iter()
                    .zip(fields)
                    .filter(|(_, w)| *w != "_")
                    .map(|(f, w)| format!("{}: {w}", f.name))
                    .collect();
                format!("{path} {{ {}, .. }}", named.join(", "))
            }
        }
    }
}

/// The diagnostics for a `match` on a value of type `ty`: E0208 if its arms do not cover every
/// value, W0201 for each arm no value reaches.
pub(crate) fn check_match(
    items: &ItemTree,
    body: &Body,
    types: &Inference,
    range: TextRange,
    ty: &Ty,
    arms: &[Arm],
) -> Vec<Diagnostic> {
    let cx = Cx { items, body, types };
    let mut out = Vec::new();
    let mut rows: Vec<Vec<DPat>> = Vec::new();
    for arm in arms {
        let p = cx.lower(arm.pat);
        // An empty range is already an error (E0214); it would also be unreachable.
        if !has_empty_range(&p) && cx.useful(&rows, std::slice::from_ref(&p)).is_none() {
            out.push(Diagnostic::warning(
                Code::W0201,
                body.pats[arm.pat].range,
                "no value reaches this arm: the arms before it match everything it does",
            ));
        }
        // A refinement test is a guard too (`refine.pat.1`).
        let refined = matches!(body.pats[arm.pat].kind, PatKind::Refined { .. });
        if arm.guard.is_none() && !refined {
            rows.push(vec![p]);
        }
    }
    if let Some(w) = cx.useful(&rows, &[DPat::wild(ty.clone())]) {
        let missing = w
            .first()
            .map_or_else(|| "_".to_string(), |w| w.render(items));
        out.push(Diagnostic::error(
            Code::E0208,
            range,
            format!(
                "this `match` does not cover every `{ty}`: `{missing}` is not matched (an arm \
                 with a guard covers nothing for this check)"
            ),
        ));
    }
    out
}

fn has_empty_range(p: &DPat) -> bool {
    matches!(p.ctor, Ctor::Int(a, b) if a > b) || p.fields.iter().any(has_empty_range)
}

/// Whether the patterns `pats` together match every value of type `ty`: whether a `catch`'s
/// arms handle every error of one type.
pub(crate) fn covers(
    items: &ItemTree,
    body: &Body,
    types: &Inference,
    ty: &Ty,
    pats: &[PatId],
) -> bool {
    let cx = Cx { items, body, types };
    let rows: Vec<Vec<DPat>> = pats.iter().map(|&p| vec![cx.lower(p)]).collect();
    cx.useful(&rows, &[DPat::wild(ty.clone())]).is_none()
}

/// A value of `pat`'s type that `pat` does not match, if there is one, rendered as a pattern.
pub(crate) fn refutable(
    items: &ItemTree,
    body: &Body,
    types: &Inference,
    pat: PatId,
) -> Option<String> {
    let cx = Cx { items, body, types };
    let rows = vec![vec![cx.lower(pat)]];
    let w = cx.useful(&rows, &[DPat::wild(types.pat(pat))])?;
    Some(
        w.first()
            .map_or_else(|| "_".to_string(), |w| w.render(items)),
    )
}
