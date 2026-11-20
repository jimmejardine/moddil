//! A deterministic LL(k) recogniser over the expanded grammar.
//!
//! It chooses productions exactly as the hand-written parser must: from at most `k` tokens of
//! lookahead, with flags already expanded, and with a greedy optional (`a??`) taking `a` whenever
//! it can start. So it is the executable definition of which token sequences the grammar accepts,
//! and the judge for the generated corpus.

use std::collections::{HashMap, HashSet};

use crate::expand::{Bnf, Sym};
use crate::ll::{Analyzer, Config, EOF, Pos};

pub(crate) struct Recogniser<'b> {
    a: Analyzer<'b>,
    k: usize,
    starts: HashMap<(u32, u32), Vec<(u32, Config)>>,
    /// How many lookahead tokens each production matches (all of them: viable).
    viable: HashMap<(u32, u32, Vec<u32>), usize>,
}

impl<'b> Recogniser<'b> {
    pub(crate) fn new(bnf: &'b Bnf, k: usize) -> Self {
        Recogniser {
            a: Analyzer::new(bnf),
            k: k.max(1),
            starts: HashMap::new(),
            viable: HashMap::new(),
        }
    }

    pub(crate) fn bnf(&self) -> &'b Bnf {
        self.a.bnf
    }

    /// Parses `tokens` as a whole `file`. Returns the productions used, or the index of the
    /// token where parsing fails (`tokens.len()` for an unexpected end): the first token that no
    /// production can continue with.
    pub(crate) fn recognise(&mut self, tokens: &[u32]) -> Result<HashSet<(u32, u32)>, usize> {
        let bnf = self.a.bnf;
        let mut stack = vec![Sym::N(bnf.start)];
        let mut pos = 0;
        let mut used = HashSet::new();
        while let Some(sym) = stack.pop() {
            match sym {
                Sym::T(t) => {
                    if tokens.get(pos) != Some(&t) {
                        return Err(pos);
                    }
                    pos += 1;
                }
                Sym::N(n) => {
                    let prod = self
                        .predict(n, &tokens[pos..])
                        .map_err(|depth| pos + depth)?;
                    used.insert((n, prod));
                    stack.extend(bnf.nts[n as usize].prods[prod as usize].iter().rev());
                }
            }
        }
        if pos == tokens.len() {
            Ok(used)
        } else {
            Err(pos)
        }
    }

    /// The production to use, or how many tokens of lookahead some production matched before
    /// none could continue.
    fn predict(&mut self, n: u32, rest: &[u32]) -> Result<u32, usize> {
        let def = &self.a.bnf.nts[n as usize];
        let (count, greedy) = (def.prods.len(), def.greedy);
        if count <= 1 {
            return if count == 1 { Ok(0) } else { Err(0) };
        }
        let mut look = Vec::with_capacity(self.k);
        for i in 0..self.k {
            let t = rest.get(i).copied().unwrap_or(EOF);
            look.push(t);
            if t == EOF {
                break;
            }
        }
        let depths: Vec<usize> = (0..count)
            .map(|p| self.matched(n, crate::ll::index(p), &look))
            .collect();
        let viable: Vec<u32> = (0..count)
            .filter(|&p| depths[p] == look.len())
            .map(crate::ll::index)
            .collect();
        match viable.as_slice() {
            [p] => Ok(*p),
            [0, 1] if greedy => Ok(0),
            [] => Err(depths.iter().copied().max().unwrap_or(0)),
            // Ambiguous within k: the grammar is LL(k), so this cannot happen.
            _ => Err(0),
        }
    }

    /// How many tokens of `look` production `p` of `n` can match (`look.len()`: all of them).
    fn matched(&mut self, n: u32, p: u32, look: &[u32]) -> usize {
        let key = (n, p, look.to_vec());
        if let Some(&v) = self.viable.get(&key) {
            return v;
        }
        let mut frontier = match self.starts.get(&(n, p)) {
            Some(f) => f.clone(),
            None => {
                let start = Config {
                    pos: Pos {
                        nt: n,
                        prod: p,
                        dot: 0,
                    },
                    stack: Vec::new(),
                };
                let f = self.a.closure(vec![start]);
                self.starts.insert((n, p), f.clone());
                f
            }
        };
        let mut depth = look.len();
        for (i, &t) in look.iter().enumerate() {
            let matched: Vec<Config> = frontier
                .into_iter()
                .filter(|(tok, _)| *tok == t)
                .map(|(_, c)| c)
                .collect();
            if matched.is_empty() {
                depth = i;
                break;
            }
            if t == EOF {
                break;
            }
            let advanced = matched
                .into_iter()
                .map(|c| Config {
                    pos: Pos {
                        dot: c.pos.dot + 1,
                        ..c.pos
                    },
                    stack: c.stack,
                })
                .collect();
            frontier = self.a.closure(advanced);
        }
        self.viable.insert(key, depth);
        depth
    }
}
