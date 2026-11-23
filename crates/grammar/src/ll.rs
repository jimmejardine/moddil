//! The LL(k) check.
//!
//! For every nonterminal with more than one production, each production is explored the way an
//! LL parser would see it: a set of configurations (a position plus a stack of return positions),
//! advanced one token at a time. Tokens that only one production can start with settle the choice;
//! for tokens several can start with, those productions are advanced and compared again, up to
//! `k` tokens. Productions still sharing a `k`-token prefix (or both able to reach the end of the
//! file) are a conflict. When a production ends with an empty stack, every place that uses the
//! nonterminal is followed, which is what a parser needs to know to choose an empty production.

use std::collections::{BTreeMap, HashSet};

use crate::expand::{Bnf, Sym};
use crate::{Conflict, GrammarError};

/// The end of the file, as a lookahead token.
pub(crate) const EOF: u32 = u32::MAX;
/// Deeper stacks than this mean left recursion.
const MAX_STACK: usize = 400;
/// Token sequences reported per conflicting decision.
const MAX_EXAMPLES: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Pos {
    pub(crate) nt: u32,
    pub(crate) prod: u32,
    pub(crate) dot: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct Config {
    pub(crate) pos: Pos,
    pub(crate) stack: Vec<Pos>,
}

#[derive(Clone)]
pub(crate) struct Analyzer<'b> {
    pub(crate) bnf: &'b Bnf,
    /// For each nonterminal, the positions just after each place it is used.
    callers: Vec<Vec<Pos>>,
    left_recursion: Option<u32>,
}

impl<'b> Analyzer<'b> {
    pub(crate) fn new(bnf: &'b Bnf) -> Self {
        Analyzer {
            bnf,
            callers: callers(bnf),
            left_recursion: None,
        }
    }
}

fn callers(bnf: &Bnf) -> Vec<Vec<Pos>> {
    let mut callers = vec![Vec::new(); bnf.nts.len()];
    for (nt, def) in bnf.nts.iter().enumerate() {
        for (prod, syms) in def.prods.iter().enumerate() {
            for (dot, sym) in syms.iter().enumerate() {
                if let Sym::N(m) = sym {
                    callers[*m as usize].push(Pos {
                        nt: index(nt),
                        prod: index(prod),
                        dot: index(dot + 1),
                    });
                }
            }
        }
    }
    callers
}

/// Returns the conflicts at lookahead `k`.
pub(crate) fn conflicts(bnf: &Bnf, k: usize) -> Result<(usize, Vec<Conflict>), GrammarError> {
    let mut a = Analyzer::new(bnf);
    let mut found = Vec::new();
    let mut decisions = 0;
    for nt in 0..bnf.nts.len() {
        if bnf.nts[nt].prods.len() < 2 {
            continue;
        }
        decisions += 1;
        let nt = index(nt);
        let alts = (0..bnf.nts[nt as usize].prods.len())
            .map(|p| {
                let start = Config {
                    pos: Pos {
                        nt,
                        prod: index(p),
                        dot: 0,
                    },
                    stack: Vec::new(),
                };
                (p, a.closure(vec![start]))
            })
            .collect();
        let mut examples = Vec::new();
        a.split(alts, 1, k, &mut Vec::new(), &mut examples);
        if bnf.nts[nt as usize].greedy {
            // Taking the optional part wins over skipping it, by definition of `??`.
            examples.retain(|(_, prods)| prods.as_slice() != [0, 1]);
        }
        if let Some(nt) = a.left_recursion {
            return Err(GrammarError::at(
                0,
                format!("left recursion through `{}`", bnf.nts[nt as usize].name),
            ));
        }
        if !examples.is_empty() {
            examples.truncate(MAX_EXAMPLES);
            let def = &bnf.nts[nt as usize];
            for (tokens, prods) in examples {
                found.push(Conflict {
                    decision: def.name.clone(),
                    tokens: tokens.iter().map(|&t| a.term_name(t)).collect(),
                    alternatives: prods.iter().map(|&p| a.render(&def.prods[p])).collect(),
                });
            }
        }
    }
    Ok((decisions, found))
}

pub(crate) fn index(i: usize) -> u32 {
    u32::try_from(i).expect("grammar fits in u32 indices")
}

impl Analyzer<'_> {
    fn term_name(&self, t: u32) -> String {
        if t == EOF {
            "end of file".to_string()
        } else {
            self.bnf.terms[t as usize].clone()
        }
    }

    fn render(&self, syms: &[Sym]) -> String {
        if syms.is_empty() {
            return "(empty)".to_string();
        }
        syms.iter()
            .map(|s| match s {
                Sym::T(t) => self.term_name(*t),
                Sym::N(n) => self.bnf.nts[*n as usize].name.clone(),
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// All configurations reachable from `seeds` without consuming a token, at a terminal (or at
    /// the end of the file).
    pub(crate) fn closure(&mut self, seeds: Vec<Config>) -> Vec<(u32, Config)> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut work = seeds;
        while let Some(c) = work.pop() {
            if seen.contains(&c) {
                continue;
            }
            seen.insert(c.clone());
            if c.stack.len() > MAX_STACK {
                self.left_recursion = Some(c.pos.nt);
                continue;
            }
            let syms = &self.bnf.nts[c.pos.nt as usize].prods[c.pos.prod as usize];
            if let Some(&sym) = syms.get(c.pos.dot as usize) {
                match sym {
                    Sym::T(t) => out.push((t, c)),
                    Sym::N(m) => {
                        let ret = Pos {
                            dot: c.pos.dot + 1,
                            ..c.pos
                        };
                        for q in 0..self.bnf.nts[m as usize].prods.len() {
                            let mut stack = c.stack.clone();
                            stack.push(ret);
                            work.push(Config {
                                pos: Pos {
                                    nt: m,
                                    prod: index(q),
                                    dot: 0,
                                },
                                stack,
                            });
                        }
                    }
                }
            } else if let Some((&ret, rest)) = c.stack.split_last() {
                work.push(Config {
                    pos: ret,
                    stack: rest.to_vec(),
                });
            } else {
                if c.pos.nt == self.bnf.start {
                    out.push((EOF, c.clone()));
                }
                for &cont in &self.callers[c.pos.nt as usize] {
                    work.push(Config {
                        pos: cont,
                        stack: Vec::new(),
                    });
                }
            }
        }
        out
    }

    /// Compares productions token by token; records shared prefixes of length `k`.
    fn split(
        &mut self,
        alts: Vec<(usize, Vec<(u32, Config)>)>,
        depth: usize,
        k: usize,
        prefix: &mut Vec<u32>,
        examples: &mut Vec<(Vec<u32>, Vec<usize>)>,
    ) {
        let mut by_token: BTreeMap<u32, BTreeMap<usize, Vec<Config>>> = BTreeMap::new();
        for (alt, frontier) in alts {
            for (t, c) in frontier {
                by_token
                    .entry(t)
                    .or_default()
                    .entry(alt)
                    .or_default()
                    .push(c);
            }
        }
        for (t, per_alt) in by_token {
            if per_alt.len() < 2 || examples.len() >= MAX_EXAMPLES {
                continue;
            }
            prefix.push(t);
            if t == EOF || depth == k {
                examples.push((prefix.clone(), per_alt.keys().copied().collect()));
            } else {
                let next = per_alt
                    .into_iter()
                    .map(|(alt, configs)| {
                        let advanced = configs
                            .into_iter()
                            .map(|c| Config {
                                pos: Pos {
                                    dot: c.pos.dot + 1,
                                    ..c.pos
                                },
                                stack: c.stack,
                            })
                            .collect();
                        (alt, self.closure(advanced))
                    })
                    .collect();
                self.split(next, depth + 1, k, prefix, examples);
            }
            prefix.pop();
        }
    }
}
