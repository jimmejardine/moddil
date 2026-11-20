//! Expanding flags and lowering EBNF to plain productions.
//!
//! Every syntax rule reachable from `file` becomes one nonterminal per flag set it is used with
//! (`expr`, `expr[ns,cond]`, …). Guards pick the alternatives of each copy. Groups and the
//! quantifiers `?`, `*` and `+` become helper nonterminals, so every decision a parser makes is a
//! choice between the productions of one nonterminal.

use std::collections::{HashMap, VecDeque};

use crate::GrammarError;
use crate::parse::{Alt, Atom, Body, COND, FlagMode, Grammar, NS, Quant, Rule, flag_names};

/// A grammar symbol: a terminal or a nonterminal, by index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Sym {
    T(u32),
    N(u32),
}

pub(crate) struct Nt {
    pub(crate) name: String,
    pub(crate) prods: Vec<Vec<Sym>>,
    /// A greedy optional (`a??`): production 0 takes `a`, production 1 skips it, and taking wins.
    pub(crate) greedy: bool,
}

pub(crate) struct Bnf {
    pub(crate) nts: Vec<Nt>,
    pub(crate) terms: Vec<String>,
    pub(crate) start: u32,
    /// Syntax-rule copies, one per (rule, flag set) in use.
    pub(crate) instances: usize,
    /// Syntax rules that `file` never reaches, with their lines.
    pub(crate) unreachable: Vec<String>,
}

/// Literal terminals are kept quoted (`'fn'`); token rules by name (`IDENT`).
fn quoted(text: &str) -> String {
    format!("'{text}'")
}

struct Builder<'g> {
    rules: HashMap<&'g str, &'g Rule>,
    nts: Vec<Nt>,
    terms: Vec<String>,
    term_ids: HashMap<String, u32>,
    instances: HashMap<(String, u8), u32>,
    queue: VecDeque<(u32, &'g Rule, u8)>,
    helpers: HashMap<u32, usize>,
    errors: Vec<GrammarError>,
}

/// Expands `grammar` from its `file` rule.
pub(crate) fn expand(grammar: &Grammar) -> Result<Bnf, Vec<GrammarError>> {
    let mut b = Builder {
        rules: grammar.rules.iter().map(|r| (r.name.as_str(), r)).collect(),
        nts: Vec::new(),
        terms: Vec::new(),
        term_ids: HashMap::new(),
        instances: HashMap::new(),
        queue: VecDeque::new(),
        helpers: HashMap::new(),
        errors: Vec::new(),
    };
    let Some(&file) = b.rules.get("file") else {
        return Err(vec![GrammarError::at(0, "no `file` rule to start from")]);
    };
    let start = b.instance(file, 0);
    while let Some((nt, rule, flags)) = b.queue.pop_front() {
        if let Body::Syntax(alt) = &rule.body {
            let prods = b.lower_alt(alt, flags, nt);
            b.nts[nt as usize].prods = prods;
        }
    }
    if !b.errors.is_empty() {
        return Err(b.errors);
    }
    let mut unreachable: Vec<String> = grammar
        .rules
        .iter()
        .filter(|r| matches!(r.body, Body::Syntax(_)))
        .filter(|r| !b.instances.keys().any(|(name, _)| name == &r.name))
        .map(|r| format!("{} (line {})", r.name, r.line))
        .collect();
    unreachable.sort();
    Ok(Bnf {
        instances: b.instances.len(),
        nts: b.nts,
        terms: b.terms,
        start,
        unreachable,
    })
}

impl<'g> Builder<'g> {
    fn term(&mut self, text: &str) -> Sym {
        if let Some(&id) = self.term_ids.get(text) {
            return Sym::T(id);
        }
        let id = u32::try_from(self.terms.len()).expect("fewer than 2^32 terminals");
        self.terms.push(text.to_string());
        self.term_ids.insert(text.to_string(), id);
        Sym::T(id)
    }

    fn new_nt(&mut self, name: String) -> u32 {
        let id = u32::try_from(self.nts.len()).expect("fewer than 2^32 nonterminals");
        self.nts.push(Nt {
            name,
            prods: Vec::new(),
            greedy: false,
        });
        id
    }

    /// The copy of `rule` for `flags`, created (and queued) on first use.
    fn instance(&mut self, rule: &'g Rule, flags: u8) -> u32 {
        let key = (rule.name.clone(), flags);
        if let Some(&id) = self.instances.get(&key) {
            return id;
        }
        let name = if flags == 0 {
            rule.name.clone()
        } else {
            format!("{}[{}]", rule.name, flag_names(flags))
        };
        let id = self.new_nt(name);
        self.instances.insert(key, id);
        match &rule.body {
            Body::Syntax(_) => self.queue.push_back((id, rule, flags)),
            Body::Keywords(words) => {
                let prods = words.iter().map(|w| vec![self.term(&quoted(w))]).collect();
                self.nts[id as usize].prods = prods;
            }
            Body::Token => unreachable!("token rules are terminals"),
        }
        id
    }

    fn helper(&mut self, owner: u32, kind: &str) -> u32 {
        let n = self.helpers.entry(owner).or_insert(0);
        *n += 1;
        let name = format!("{}/{n}{kind}", self.nts[owner as usize].name);
        self.new_nt(name)
    }

    fn lower_alt(&mut self, alt: &Alt, flags: u8, owner: u32) -> Vec<Vec<Sym>> {
        alt.0
            .iter()
            .filter(|seq| seq.guards.iter().all(|g| g.allows(flags)))
            .map(|seq| {
                let mut out = Vec::new();
                for item in &seq.items {
                    let base = match &item.atom {
                        Atom::Term(t) => self.term(&quoted(t)),
                        Atom::Ref { name, mode, line } => self.reference(name, *mode, flags, *line),
                        Atom::Group(inner) => {
                            let h = self.helper(owner, "()");
                            let prods = self.lower_alt(inner, flags, owner);
                            self.nts[h as usize].prods = prods;
                            Sym::N(h)
                        }
                    };
                    match item.quant {
                        Quant::One => out.push(base),
                        Quant::Opt | Quant::Greedy => {
                            let greedy = item.quant == Quant::Greedy;
                            let h = self.helper(owner, if greedy { "??" } else { "?" });
                            let nt = &mut self.nts[h as usize];
                            nt.prods = vec![vec![base], vec![]];
                            nt.greedy = greedy;
                            out.push(Sym::N(h));
                        }
                        Quant::Star | Quant::Plus => {
                            if item.quant == Quant::Plus {
                                out.push(base);
                            }
                            let h = self.helper(owner, "*");
                            self.nts[h as usize].prods = vec![vec![base, Sym::N(h)], vec![]];
                            out.push(Sym::N(h));
                        }
                    }
                }
                out
            })
            .collect()
    }

    fn reference(&mut self, name: &str, mode: FlagMode, flags: u8, line: usize) -> Sym {
        let Some(&rule) = self.rules.get(name) else {
            self.errors
                .push(GrammarError::at(line, format!("`{name}` is not defined")));
            return self.term(name);
        };
        match rule.body {
            Body::Token => self.term(name),
            Body::Keywords(_) => Sym::N(self.instance(rule, 0)),
            Body::Syntax(_) => {
                let next = match mode {
                    FlagMode::Fresh => 0,
                    FlagMode::Inherit => flags,
                    FlagMode::InheritNs => flags & (NS | COND),
                    FlagMode::Set(bits) => bits,
                };
                Sym::N(self.instance(rule, next))
            }
        }
    }
}
