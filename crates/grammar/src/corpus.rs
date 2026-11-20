//! The generated parser corpus: valid sentences that together use every production the parser
//! can reach, and near-misses (single-token edits) that the parser must reject.
//!
//! Sentences are token sequences built from shortest derivations; the recogniser decides what is
//! valid, and coverage is counted from its actual parses. Rendering turns them into source text,
//! one sample spelling per token.

use std::collections::{HashSet, VecDeque};
use std::fmt::Write as _;

use crate::expand::{Bnf, Sym};
use crate::recognise::Recogniser;

const INF: u64 = u64::MAX;
/// Near-misses to keep.
const NEAR_MISSES: usize = 300;

pub(crate) struct Corpus {
    pub(crate) valid: Vec<Vec<u32>>,
    pub(crate) invalid: Vec<Vec<u32>>,
    /// Productions the valid sentences use, of all productions reachable from `file`.
    pub(crate) covered: usize,
    pub(crate) productions: usize,
    /// Productions no shortest-context sentence reaches as the parser parses it.
    pub(crate) uncovered: Vec<String>,
}

/// Generates the corpus. Deterministic: the same grammar always gives the same corpus.
pub(crate) fn generate(rec: &mut Recogniser<'_>) -> Corpus {
    let bnf = rec.bnf();
    let (minlen, minprod) = shortest(bnf);
    let parents = contexts(bnf, &minlen);
    let mut covered: HashSet<(u32, u32)> = HashSet::new();
    let mut tried: HashSet<(u32, u32)> = HashSet::new();
    let mut seen: HashSet<Vec<u32>> = HashSet::new();
    let mut valid = Vec::new();
    let mut uncovered = Vec::new();
    let mut productions = 0;
    for n in 0..bnf.nts.len() {
        let n = crate::ll::index(n);
        if parents[n as usize].is_none() && n != bnf.start {
            continue; // unreachable copy
        }
        for p in 0..bnf.nts[n as usize].prods.len() {
            let p = crate::ll::index(p);
            if prod_len(bnf, n, p, &minlen) == INF {
                continue;
            }
            productions += 1;
            if covered.contains(&(n, p)) || !tried.insert((n, p)) {
                continue;
            }
            let tokens = sentence(bnf, &parents, &minprod, n, p);
            match rec.recognise(&tokens) {
                Ok(used) if used.contains(&(n, p)) => {
                    covered.extend(used);
                    if seen.insert(tokens.clone()) {
                        valid.push(tokens);
                    }
                }
                _ => uncovered.push(format!("{} #{}", bnf.nts[n as usize].name, p + 1)),
            }
        }
    }
    let invalid = near_misses(rec, &valid);
    Corpus {
        covered: covered.len(),
        valid,
        invalid,
        productions,
        uncovered,
    }
}

fn prod_len(bnf: &Bnf, n: u32, p: u32, minlen: &[u64]) -> u64 {
    bnf.nts[n as usize].prods[p as usize]
        .iter()
        .map(|s| match s {
            Sym::T(_) => 1,
            Sym::N(m) => minlen[*m as usize],
        })
        .fold(0, u64::saturating_add)
}

/// The length of each nonterminal's shortest yield, and the production that gives it.
fn shortest(bnf: &Bnf) -> (Vec<u64>, Vec<u32>) {
    let mut minlen = vec![INF; bnf.nts.len()];
    let mut minprod = vec![0; bnf.nts.len()];
    let mut changed = true;
    while changed {
        changed = false;
        for n in 0..bnf.nts.len() {
            for p in 0..bnf.nts[n].prods.len() {
                let len = prod_len(bnf, crate::ll::index(n), crate::ll::index(p), &minlen);
                if len < minlen[n] {
                    minlen[n] = len;
                    minprod[n] = crate::ll::index(p);
                    changed = true;
                }
            }
        }
    }
    (minlen, minprod)
}

type Parent = Option<(u32, u32, usize)>;

/// For each nonterminal, where it is first reached from `file` (breadth first), through
/// productions whose other symbols can all be completed.
fn contexts(bnf: &Bnf, minlen: &[u64]) -> Vec<Parent> {
    let mut parents: Vec<Parent> = vec![None; bnf.nts.len()];
    let mut seen = vec![false; bnf.nts.len()];
    seen[bnf.start as usize] = true;
    let mut queue = VecDeque::from([bnf.start]);
    while let Some(n) = queue.pop_front() {
        for (p, syms) in bnf.nts[n as usize].prods.iter().enumerate() {
            if prod_len(bnf, n, crate::ll::index(p), minlen) == INF {
                continue;
            }
            for (i, s) in syms.iter().enumerate() {
                if let Sym::N(m) = *s
                    && !seen[m as usize]
                {
                    seen[m as usize] = true;
                    parents[m as usize] = Some((n, crate::ll::index(p), i));
                    queue.push_back(m);
                }
            }
        }
    }
    parents
}

fn expand_min(bnf: &Bnf, minprod: &[u32], sym: Sym, out: &mut Vec<u32>) {
    match sym {
        Sym::T(t) => out.push(t),
        Sym::N(n) => {
            let prod = &bnf.nts[n as usize].prods[minprod[n as usize] as usize];
            for &s in prod {
                expand_min(bnf, minprod, s, out);
            }
        }
    }
}

/// A sentence that reaches `n` by its shortest context and uses production `p` there.
fn sentence(bnf: &Bnf, parents: &[Parent], minprod: &[u32], n: u32, p: u32) -> Vec<u32> {
    let mut path = Vec::new();
    let mut at = n;
    while let Some((parent, prod, i)) = parents[at as usize] {
        path.push((parent, prod, i));
        at = parent;
    }
    path.reverse();
    let mut out = Vec::new();
    build(bnf, minprod, &path, (n, p), &mut out);
    out
}

fn build(
    bnf: &Bnf,
    minprod: &[u32],
    path: &[(u32, u32, usize)],
    target: (u32, u32),
    out: &mut Vec<u32>,
) {
    let Some(&(a, q, i)) = path.first() else {
        for &s in &bnf.nts[target.0 as usize].prods[target.1 as usize] {
            expand_min(bnf, minprod, s, out);
        }
        return;
    };
    for (j, &s) in bnf.nts[a as usize].prods[q as usize].iter().enumerate() {
        if j == i {
            build(bnf, minprod, &path[1..], target, out);
        } else {
            expand_min(bnf, minprod, s, out);
        }
    }
}

/// A small, seeded xorshift generator, so the corpus is the same on every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self, bound: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        usize::try_from(self.0 % bound as u64).expect("bound fits")
    }
}

fn near_misses(rec: &mut Recogniser<'_>, valid: &[Vec<u32>]) -> Vec<Vec<u32>> {
    let terms = crate::ll::index(rec.bnf().terms.len());
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut out = Vec::new();
    let mut seen: HashSet<Vec<u32>> = valid.iter().cloned().collect();
    for round in 0..4 {
        for sentence in valid {
            if out.len() >= NEAR_MISSES {
                return out;
            }
            if sentence.len() < 2 {
                continue;
            }
            let mut edited = sentence.clone();
            let at = rng.next(edited.len());
            match (round + rng.next(4)) % 4 {
                0 => {
                    edited.remove(at);
                }
                1 => edited.insert(at, edited[at]),
                2 if at + 1 < edited.len() => edited.swap(at, at + 1),
                _ => edited.insert(at, crate::ll::index(rng.next(terms as usize))),
            }
            if seen.insert(edited.clone()) && rec.recognise(&edited).is_err() {
                out.push(edited);
            }
        }
    }
    out
}

/// `count` random edits of the valid sentences (one to three token edits each: delete,
/// duplicate, swap, insert or replace), each with the recogniser's verdict: as many valid as
/// invalid, where the attempts allow (most random edits are invalid). An oracle for testing
/// the hand-written parser against the grammar on far more cases than the checked-in corpus.
pub(crate) fn edits(
    rec: &mut Recogniser<'_>,
    valid: &[Vec<u32>],
    count: usize,
    seed: u64,
) -> Vec<(Vec<u32>, bool)> {
    let terms = rec.bnf().terms.len();
    let mut rng = Rng(seed | 1);
    let mut out = Vec::new();
    let mut seen: HashSet<Vec<u32>> = HashSet::new();
    let pool: Vec<&Vec<u32>> = valid.iter().filter(|s| !s.is_empty()).collect();
    let (mut valid_left, mut invalid_left) = (count - count / 2, count / 2);
    let mut attempts = 0;
    while valid_left + invalid_left > 0 && attempts < count * 50 {
        attempts += 1;
        let mut edited = pool[rng.next(pool.len())].clone();
        // Mostly single edits: they are valid far more often than multiple ones.
        let edits = if rng.next(4) == 0 { 2 + rng.next(2) } else { 1 };
        for _ in 0..edits {
            if edited.is_empty() {
                break;
            }
            let at = rng.next(edited.len());
            let random = crate::ll::index(rng.next(terms));
            match rng.next(5) {
                0 => {
                    edited.remove(at);
                }
                1 => edited.insert(at, edited[at]),
                2 if at + 1 < edited.len() => edited.swap(at, at + 1),
                3 => edited[at] = random,
                _ => edited.insert(at, random),
            }
        }
        if seen.insert(edited.clone()) {
            let ok = accepts_with_splitting(rec, &edited);
            let left = if ok {
                &mut valid_left
            } else {
                &mut invalid_left
            };
            if *left > 0 {
                *left -= 1;
                out.push((edited, ok));
            }
        }
    }
    out
}

/// Whether the grammar accepts `tokens`, applying the Tokens section's splitting rules: where
/// the recogniser rejects at `&&` (two `&`) or at `>>`, `>=` or `>>=` (a `>` closing generic
/// arguments), it retries with the token split after its first character.
fn accepts_with_splitting(rec: &mut Recogniser<'_>, tokens: &[u32]) -> bool {
    let bnf = rec.bnf();
    let term = |name: &str| {
        bnf.terms
            .iter()
            .position(|t| t == name)
            .map(crate::ll::index)
    };
    let splits: Vec<(u32, [u32; 2])> = [
        ("'&&'", "'&'", "'&'"),
        ("'>>'", "'>'", "'>'"),
        ("'>='", "'>'", "'='"),
        ("'>>='", "'>'", "'>='"),
    ]
    .iter()
    .filter_map(|(whole, a, b)| Some((term(whole)?, [term(a)?, term(b)?])))
    .collect();
    let mut tokens = tokens.to_vec();
    for _ in 0..8 {
        match rec.recognise(&tokens) {
            Ok(_) => return true,
            Err(at) => match tokens
                .get(at)
                .and_then(|t| splits.iter().find(|s| s.0 == *t))
            {
                Some(&(_, parts)) => {
                    tokens.splice(at..=at, parts);
                }
                None => return false,
            },
        }
    }
    false
}

/// The sample spelling of each token kind in rendered sentences.
fn sample(name: &str) -> &'static str {
    match name {
        "IDENT" => "x",
        "INTEGER" => "1",
        "FLOAT" => "1.5",
        "STRING" => "\"s\"",
        "BYTE_STRING" => "b\"s\"",
        "CHAR" => "'c'",
        "BYTE" => "b'c'",
        "RAW_STRING" => "r#\"raw \"text\"\"#",
        "BLOCK_STRING" => "\"\"\"\n    text\n    \"\"\"",
        "TAGGED_STRING" => "fmt\"{x.f(\"y\")}\"",
        "LABEL" => "'a",
        "DOC_OUTER" => "/// doc\n",
        "DOC_INNER" => "//! doc\n",
        _ => panic!("no sample spelling for token `{name}`"),
    }
}

/// Renders a sentence as source text: tokens separated by spaces, a doc comment ending its line.
pub(crate) fn render(bnf: &Bnf, tokens: &[u32]) -> String {
    let mut out = String::new();
    for &t in tokens {
        let term = &bnf.terms[t as usize];
        let text = term
            .strip_prefix('\'')
            .and_then(|s| s.strip_suffix('\''))
            .unwrap_or_else(|| sample(term));
        if !out.is_empty() && !out.ends_with('\n') {
            out.push(' ');
        }
        out.push_str(text);
    }
    out
}

/// One corpus file: a header, then each case after a `// ---` line.
pub(crate) fn file(bnf: &Bnf, cases: &[Vec<u32>], what: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "// Generated by moddil-grammar from reference/grammar/moddil.ebnf: do not edit."
    );
    let _ = writeln!(
        out,
        "// {what} Each case is a whole file, after a `// ---` line."
    );
    for case in cases {
        out.push_str("// ---\n");
        out.push_str(&render(bnf, case));
        if !out.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}
