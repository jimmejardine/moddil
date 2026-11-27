//! `dimension` and `unit` items (`unit.dim.1`, `unit.decl.1`): their names are declared with the
//! other items' names, then each is resolved after the ones it names, to its dimension (a product
//! of base dimensions), its scale and, for an affine unit, its offset.

use moddil_syntax::{SyntaxKind as K, SyntaxNode};

use super::{Collector, Site, name_of, vis_of};
use crate::items::{Def, DimensionDef, Src, UnitDef, Vis};
use crate::units::{Dim, Ratio, Scale, dim_mul};
use crate::{Code, Diagnostic};

/// Whether item `item` writes `name` as (part of) a unit: `f64[U]`, `f64[U^2]`, `Vec<f64[U]>`.
pub(super) fn names_in_units(item: &SyntaxNode, name: &str) -> bool {
    item.descendants()
        .iter()
        .filter(|n| n.kind() == K::UNIT_SUFFIX)
        .any(|s| {
            s.descendants().iter().any(|p| {
                p.kind() == K::PATH_TYPE
                    && p.child(K::PATH).is_some_and(|path| {
                        let segs: Vec<String> = path
                            .children()
                            .filter(|c| c.kind() == K::PATH_SEGMENT)
                            .filter_map(|c| c.token(K::IDENT).map(|t| t.text().to_string()))
                            .collect();
                        segs.len() == 1 && segs[0] == name
                    })
            })
        })
}

/// A unit expression's exponent: `2`, `-1`, `(1/2)`, `(-3/2)`.
pub(crate) fn exponent(node: &SyntaxNode) -> Option<Ratio> {
    let mut neg = false;
    let mut nums = Vec::new();
    for t in node.child_tokens() {
        match t.kind() {
            K::MINUS => neg = true,
            K::INTEGER => nums.push(t.text().replace('_', "").parse::<i128>().ok()?),
            _ => {}
        }
    }
    let (n, d) = match nums.as_slice() {
        [n] => (*n, 1),
        [n, d] => (*n, *d),
        _ => return None,
    };
    Ratio::new(if neg { -n } else { n }, d)
}

/// What a unit expression is made of, for [`eval`]: a value that multiplies and takes powers.
pub(crate) trait Product: Sized {
    /// `self · o^k`, or `None` past what it holds.
    fn mul_pow(self, o: Self, k: Ratio) -> Option<Self>;
    /// One.
    fn one() -> Self;
}

impl Product for Dim {
    fn mul_pow(mut self, o: Dim, k: Ratio) -> Option<Dim> {
        dim_mul(&mut self, &o, k)?;
        Some(self)
    }
    fn one() -> Dim {
        Dim::new()
    }
}

impl Product for (Dim, Scale) {
    fn mul_pow(self, o: (Dim, Scale), k: Ratio) -> Option<(Dim, Scale)> {
        let d = self.0.mul_pow(o.0, k)?;
        Some((d, self.1.mul(o.1.pow(k))))
    }
    fn one() -> (Dim, Scale) {
        (Dim::new(), Scale::ONE)
    }
}

impl Product for crate::units::UnitTy {
    fn mul_pow(self, o: Self, k: Ratio) -> Option<Self> {
        crate::units::UnitTy::mul_pow(&self, &o, k)
    }
    fn one() -> Self {
        crate::units::UnitTy::one()
    }
}

/// The value of the unit expression `node` (`m / s^2`, `1000 * m`, `(kg * m)`), each of its atoms
/// (a path, or a number) being what `atom` gives; `None` if an atom has none (it reported why)
/// or an exponent is malformed.
pub(crate) fn eval<V: Product>(
    node: &SyntaxNode,
    atom: &mut dyn FnMut(&SyntaxNode) -> Option<V>,
) -> Option<V> {
    let parts: Vec<SyntaxNode> = node.children().collect();
    match node.kind() {
        K::UNIT_BIN => {
            let [l, r] = parts.as_slice() else {
                return None;
            };
            let div = node.token(K::SLASH).is_some();
            let l = eval(l, atom);
            let r = eval(r, atom);
            l?.mul_pow(r?, if div { Ratio::int(-1) } else { Ratio::ONE })
        }
        K::UNIT_POWER => {
            let base = eval(parts.first()?, atom)?;
            let e = exponent(&node.child(K::EXPONENT)?)?;
            V::one().mul_pow(base, e)
        }
        K::UNIT_PAREN | K::UNIT_SUFFIX => eval(parts.first()?, atom),
        _ => atom(node),
    }
}

/// The names of a path atom (`m`, `si::m`).
pub(crate) fn atom_path(node: &SyntaxNode) -> Option<Vec<String>> {
    let path = node.child(K::PATH)?;
    let segs: Vec<String> = path
        .children()
        .filter(|c| c.kind() == K::PATH_SEGMENT)
        .filter_map(|c| c.token(K::IDENT).map(|t| t.text().to_string()))
        .collect();
    (!segs.is_empty()).then_some(segs)
}

/// Where a declaration is while it is resolved.
#[derive(Clone, Copy, PartialEq)]
enum State {
    Pending,
    Busy,
    Done,
}

/// The `dimension` and `unit` items being resolved.
pub(super) struct UnitItems {
    dims: Vec<Site>,
    units: Vec<Site>,
    dim_state: Vec<State>,
    unit_state: Vec<State>,
    first_dim: u32,
    first_unit: u32,
}

impl Collector<'_> {
    /// Declares the names of the `dimension` and `unit` items `dims` and `units`.
    pub(super) fn unit_skeletons(&mut self, dims: &[Site], units: &[Site]) -> UnitItems {
        let first_dim = self.tree.dimensions.len() as u32;
        let first_unit = self.tree.units.len() as u32;
        for (src, module, node) in dims {
            self.enter(*src, *module);
            let (name, name_range) = name_of(node);
            let vis = item_vis(*src, node);
            let index = self.tree.dimensions.len() as u32;
            self.declare(*src, &name, Def::Dimension(index), vis, name_range);
            self.tree.dimensions.push(DimensionDef {
                name,
                name_range,
                src: *src,
                module: *module,
                vis,
                dim: Dim::new(),
            });
        }
        for (src, module, node) in units {
            self.enter(*src, *module);
            let (name, name_range) = name_of(node);
            let vis = item_vis(*src, node);
            let index = self.tree.units.len() as u32;
            self.declare(*src, &name, Def::Unit(index), vis, name_range);
            self.tree.units.push(UnitDef {
                name,
                name_range,
                src: *src,
                module: *module,
                vis,
                dim: Dim::new(),
                scale: Scale::ONE,
                offset: None,
            });
        }
        UnitItems {
            dims: dims.to_vec(),
            units: units.to_vec(),
            dim_state: vec![State::Pending; dims.len()],
            unit_state: vec![State::Pending; units.len()],
            first_dim,
            first_unit,
        }
    }

    /// Resolves each declaration of `items`, each after those it names.
    pub(super) fn unit_defs(&mut self, items: &mut UnitItems) {
        for k in 0..items.dims.len() {
            self.dim_def(items, k);
        }
        for k in 0..items.units.len() {
            self.unit_def(items, k);
        }
    }

    fn report_unit(&mut self, src: Src, d: Diagnostic) {
        self.reporter.report(src, None, vec![d]);
    }

    /// What `node`, a path in module `module`, names, reported (E0223) if `want` says it is not
    /// that kind of thing.
    fn unit_name(
        &mut self,
        src: Src,
        module: u32,
        node: &SyntaxNode,
        dimension: bool,
    ) -> Option<Def> {
        let names = atom_path(node)?;
        let strs: Vec<&str> = names.iter().map(String::as_str).collect();
        let found = match self.tree.path(module, &strs, false) {
            Ok((def, n)) if n == strs.len() => Some(def),
            _ => None,
        };
        let ok = matches!(
            (found, dimension),
            (Some(Def::Dimension(_)), true) | (Some(Def::Unit(_)), false)
        );
        if !ok {
            let what = if dimension { "dimension" } else { "unit" };
            self.report_unit(
                src,
                Diagnostic::error(
                    Code::E0223,
                    node.text_range(),
                    format!("there is no {what} called `{}`", names.join("::")),
                ),
            );
            return None;
        }
        found
    }

    /// Dimension `k` of `items`, resolved.
    fn dim_def(&mut self, items: &mut UnitItems, k: usize) -> Option<Dim> {
        let index = items.first_dim as usize + k;
        match items.dim_state[k] {
            State::Done => return Some(self.tree.dimensions[index].dim.clone()),
            State::Busy => {
                let (src, _, node) = items.dims[k].clone();
                self.report_unit(
                    src,
                    Diagnostic::error(
                        Code::E0223,
                        name_of(&node).1,
                        "this dimension is defined in terms of itself",
                    ),
                );
                return None;
            }
            State::Pending => {}
        }
        items.dim_state[k] = State::Busy;
        let (src, module, node) = items.dims[k].clone();
        let expr = node.children().find(|c| is_unit_expr(c.kind()));
        let dim = match expr {
            // A base dimension: itself.
            None => Some(Dim::from([(index as u32, Ratio::ONE)])),
            Some(e) => eval::<Dim>(&e, &mut |a| {
                let Def::Dimension(d) = self.unit_name(src, module, a, true)? else {
                    return None;
                };
                self.dim_of_index(items, d)
            }),
        };
        items.dim_state[k] = State::Done;
        let dim = dim.unwrap_or_default();
        self.tree.dimensions[index].dim = dim.clone();
        Some(dim)
    }

    fn dim_of_index(&mut self, items: &mut UnitItems, d: u32) -> Option<Dim> {
        match d.checked_sub(items.first_dim) {
            Some(k) if (k as usize) < items.dims.len() => self.dim_def(items, k as usize),
            _ => Some(self.tree.dimensions[d as usize].dim.clone()),
        }
    }

    fn unit_of_index(&mut self, items: &mut UnitItems, u: u32) -> Option<UnitDef> {
        match u.checked_sub(items.first_unit) {
            Some(k) if (k as usize) < items.units.len() => {
                self.unit_def(items, k as usize);
            }
            _ => {}
        }
        let def = self.tree.units[u as usize].clone();
        Some(def)
    }

    /// Unit `k` of `items`, resolved.
    fn unit_def(&mut self, items: &mut UnitItems, k: usize) {
        let index = items.first_unit as usize + k;
        match items.unit_state[k] {
            State::Done => return,
            State::Busy => {
                let (src, _, node) = items.units[k].clone();
                self.report_unit(
                    src,
                    Diagnostic::error(
                        Code::E0223,
                        name_of(&node).1,
                        "this unit is defined in terms of itself",
                    ),
                );
                return;
            }
            State::Pending => {}
        }
        items.unit_state[k] = State::Busy;
        let (src, module, node) = items.units[k].clone();
        let mut offset = None;
        let found: Option<(Dim, Scale)> = if node.token(K::COLON).is_some() {
            // `unit m: Length;`: a dimension's unit of scale one.
            node.child(K::PATH_TYPE).and_then(|p| {
                let Def::Dimension(d) = self.unit_name(src, module, &p, true)? else {
                    return None;
                };
                Some((self.dim_of_index(items, d)?, Scale::ONE))
            })
        } else {
            node.child(K::UNIT_DEF).and_then(|def| {
                let expr = def.children().find(|c| is_unit_expr(c.kind()))?;
                let mut affine_in = false;
                let value = eval::<(Dim, Scale)>(&expr, &mut |a| match a.kind() {
                    K::LITERAL => {
                        let t = a.child_tokens().find(|t| !t.kind().is_trivia())?;
                        match Ratio::parse_decimal(t.text()) {
                            Some(r) => Some((Dim::new(), Scale::Exact(r))),
                            None => {
                                self.report_unit(
                                    src,
                                    Diagnostic::error(
                                        Code::E0223,
                                        a.text_range(),
                                        "a unit's factor is a decimal number",
                                    ),
                                );
                                None
                            }
                        }
                    }
                    _ => {
                        let Def::Unit(u) = self.unit_name(src, module, a, false)? else {
                            return None;
                        };
                        let def = self.unit_of_index(items, u)?;
                        if def.offset.is_some() {
                            affine_in = true;
                        }
                        Some((def.dim, def.scale))
                    }
                })?;
                if affine_in {
                    self.report_unit(
                        src,
                        Diagnostic::error(
                            Code::E0223,
                            expr.text_range(),
                            "an affine unit is not part of another unit's definition",
                        ),
                    );
                    return None;
                }
                // `K + 273.15`: an affine unit, whose zero is that far from its base unit's.
                let sign = def
                    .child_tokens()
                    .find(|t| matches!(t.kind(), K::PLUS | K::MINUS));
                let num = def
                    .child_tokens()
                    .find(|t| matches!(t.kind(), K::INTEGER | K::FLOAT));
                if let (Some(sign), Some(num)) = (sign, num) {
                    let Some(r) = Ratio::parse_decimal(num.text()) else {
                        self.report_unit(
                            src,
                            Diagnostic::error(
                                Code::E0223,
                                num.text_range(),
                                "an affine unit's offset is a decimal number",
                            ),
                        );
                        return None;
                    };
                    let o = Scale::Exact(r).mul(value.1);
                    offset = Some(if sign.kind() == K::MINUS { o.neg() } else { o });
                }
                Some(value)
            })
        };
        items.unit_state[k] = State::Done;
        if let Some((dim, scale)) = found {
            if scale.is_zero() {
                self.report_unit(
                    src,
                    Diagnostic::error(Code::E0223, name_of(&node).1, "a unit's scale is not zero"),
                );
                return;
            }
            let u = &mut self.tree.units[index];
            u.dim = dim;
            u.scale = scale;
            u.offset = offset;
        }
    }
}

/// Whether a node of kind `k` is a unit expression or one of its atoms.
fn is_unit_expr(k: K) -> bool {
    matches!(
        k,
        K::UNIT_BIN | K::UNIT_POWER | K::UNIT_PAREN | K::PATH_TYPE | K::LITERAL
    )
}

fn item_vis(src: Src, node: &SyntaxNode) -> Vis {
    match src {
        Src::File(_) => vis_of(node),
        _ => Vis::Pub,
    }
}
