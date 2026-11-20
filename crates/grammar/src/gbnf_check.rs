//! Test support: reads the GBNF this crate emits and recognises text with it (an Earley parser
//! over characters), so the generated grammar is checked against the corpus without llama.cpp.

use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum El {
    Rule(usize),
    Chars {
        negated: bool,
        ranges: Vec<(char, char)>,
    },
}

pub(crate) struct Cfg {
    rules: Vec<Vec<Vec<El>>>,
    root: usize,
    nullable: Vec<bool>,
}

struct Reader<'a> {
    chars: Vec<char>,
    pos: usize,
    names: &'a mut HashMap<String, usize>,
    rules: &'a mut Vec<Vec<Vec<El>>>,
}

impl Reader<'_> {
    fn skip_space(&mut self) {
        while let Some(&c) = self.chars.get(self.pos) {
            if c == ' ' || c == '\t' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn rule_id(&mut self, name: &str) -> usize {
        if let Some(&id) = self.names.get(name) {
            return id;
        }
        let id = self.rules.len();
        self.rules.push(Vec::new());
        self.names.insert(name.to_string(), id);
        id
    }

    fn helper(&mut self, alts: Vec<Vec<El>>) -> usize {
        let id = self.rules.len();
        self.rules.push(alts);
        id
    }

    fn escape(&mut self) -> char {
        let c = self.chars[self.pos];
        self.pos += 1;
        let hex = |r: &mut Self, n: usize| {
            let s: String = r.chars[r.pos..r.pos + n].iter().collect();
            r.pos += n;
            char::from_u32(u32::from_str_radix(&s, 16).expect("hex escape")).expect("char")
        };
        match c {
            'n' => '\n',
            't' => '\t',
            'r' => '\r',
            'x' => hex(self, 2),
            'u' => hex(self, 4),
            'U' => hex(self, 8),
            other => other,
        }
    }

    fn alt(&mut self) -> Vec<Vec<El>> {
        let mut alts = vec![self.seq()];
        loop {
            self.skip_space();
            if self.chars.get(self.pos) == Some(&'|') {
                self.pos += 1;
                alts.push(self.seq());
            } else {
                return alts;
            }
        }
    }

    fn seq(&mut self) -> Vec<El> {
        let mut seq = Vec::new();
        loop {
            self.skip_space();
            let Some(&c) = self.chars.get(self.pos) else {
                return seq;
            };
            let mut items: Vec<El> = match c {
                '|' | ')' => return seq,
                '"' => {
                    self.pos += 1;
                    let mut lit = Vec::new();
                    while self.chars[self.pos] != '"' {
                        let ch = if self.chars[self.pos] == '\\' {
                            self.pos += 1;
                            self.escape()
                        } else {
                            self.pos += 1;
                            self.chars[self.pos - 1]
                        };
                        lit.push(El::Chars {
                            negated: false,
                            ranges: vec![(ch, ch)],
                        });
                    }
                    self.pos += 1;
                    lit
                }
                '[' => {
                    self.pos += 1;
                    let negated = self.chars[self.pos] == '^';
                    if negated {
                        self.pos += 1;
                    }
                    let mut ranges = Vec::new();
                    while self.chars[self.pos] != ']' {
                        let lo = self.class_char();
                        let hi = if self.chars[self.pos] == '-' && self.chars[self.pos + 1] != ']' {
                            self.pos += 1;
                            self.class_char()
                        } else {
                            lo
                        };
                        ranges.push((lo, hi));
                    }
                    self.pos += 1;
                    vec![El::Chars { negated, ranges }]
                }
                '(' => {
                    self.pos += 1;
                    let inner = self.alt();
                    self.skip_space();
                    self.pos += 1; // ')'
                    vec![El::Rule(self.helper(inner))]
                }
                _ => {
                    let start = self.pos;
                    while self
                        .chars
                        .get(self.pos)
                        .is_some_and(|c| c.is_ascii_alphanumeric() || *c == '-')
                    {
                        self.pos += 1;
                    }
                    let name: String = self.chars[start..self.pos].iter().collect();
                    vec![El::Rule(self.rule_id(&name))]
                }
            };
            if let Some(&q) = self.chars.get(self.pos)
                && matches!(q, '?' | '*' | '+')
            {
                self.pos += 1;
                let unit = self.helper(vec![items]);
                let rep = self.rules.len();
                self.rules.push(Vec::new());
                self.rules[rep] = match q {
                    '?' => vec![vec![El::Rule(unit)], vec![]],
                    '*' => vec![vec![El::Rule(unit), El::Rule(rep)], vec![]],
                    _ => vec![vec![El::Rule(unit), El::Rule(rep)], vec![El::Rule(unit)]],
                };
                items = vec![El::Rule(rep)];
            }
            seq.extend(items);
        }
    }

    fn class_char(&mut self) -> char {
        let c = self.chars[self.pos];
        self.pos += 1;
        if c == '\\' { self.escape() } else { c }
    }
}

/// Reads GBNF text (the subset this crate emits).
pub(crate) fn read(text: &str) -> Cfg {
    let mut names = HashMap::new();
    let mut rules = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (name, body) = line.split_once("::=").expect("a rule");
        let mut r = Reader {
            chars: body.chars().collect(),
            pos: 0,
            names: &mut names,
            rules: &mut rules,
        };
        let id = r.rule_id(name.trim());
        let alts = r.alt();
        r.rules[id] = alts;
    }
    let root = names["root"];
    let mut nullable = vec![false; rules.len()];
    let mut changed = true;
    while changed {
        changed = false;
        for (i, alts) in rules.iter().enumerate() {
            if !nullable[i]
                && alts
                    .iter()
                    .any(|a| a.iter().all(|e| matches!(e, El::Rule(r) if nullable[*r])))
            {
                nullable[i] = true;
                changed = true;
            }
        }
    }
    for (name, &id) in &names {
        assert!(
            !rules[id].is_empty(),
            "GBNF rule `{name}` is used but not defined"
        );
    }
    Cfg {
        rules,
        root,
        nullable,
    }
}

/// An Earley item: (rule, alternative, dot, origin).
type Item = (usize, usize, usize, usize);

fn matches(el: &El, c: char) -> bool {
    match el {
        El::Chars { negated, ranges } => {
            ranges.iter().any(|&(lo, hi)| lo <= c && c <= hi) != *negated
        }
        El::Rule(_) => false,
    }
}

/// Whether `text` is in the language (Earley, with nullable rules completed at prediction).
pub(crate) fn accepts(cfg: &Cfg, text: &str) -> bool {
    let input: Vec<char> = text.chars().collect();
    // An item: (rule, alternative, dot, origin).
    let mut sets: Vec<Vec<Item>> = vec![Vec::new(); input.len() + 1];
    let mut seen: Vec<HashSet<Item>> = vec![HashSet::new(); input.len() + 1];
    // Items in each set that wait for a rule, by rule: completion looks them up directly.
    let mut waiting: Vec<HashMap<usize, Vec<Item>>> = vec![HashMap::new(); input.len() + 1];
    for alt in 0..cfg.rules[cfg.root].len() {
        let item = (cfg.root, alt, 0, 0);
        seen[0].insert(item);
        sets[0].push(item);
    }
    for i in 0..=input.len() {
        let mut j = 0;
        while j < sets[i].len() {
            let (rule, alt, dot, origin) = sets[i][j];
            j += 1;
            let seq = &cfg.rules[rule][alt];
            if let Some(el) = seq.get(dot) {
                match el {
                    El::Rule(r) => {
                        waiting[i]
                            .entry(*r)
                            .or_default()
                            .push((rule, alt, dot, origin));
                        for a in 0..cfg.rules[*r].len() {
                            let item = (*r, a, 0, i);
                            if seen[i].insert(item) {
                                sets[i].push(item);
                            }
                        }
                        if cfg.nullable[*r] {
                            let item = (rule, alt, dot + 1, origin);
                            if seen[i].insert(item) {
                                sets[i].push(item);
                            }
                        }
                    }
                    El::Chars { .. } => {
                        if i < input.len() && matches(el, input[i]) {
                            let item = (rule, alt, dot + 1, origin);
                            if seen[i + 1].insert(item) {
                                sets[i + 1].push(item);
                            }
                        }
                    }
                }
            } else {
                let parents: Vec<_> = waiting[origin].get(&rule).cloned().unwrap_or_default();
                for (pr, pa, pd, po) in parents {
                    let item = (pr, pa, pd + 1, po);
                    if seen[i].insert(item) {
                        sets[i].push(item);
                    }
                }
            }
        }
    }
    sets[input.len()]
        .iter()
        .any(|&(r, a, d, o)| r == cfg.root && o == 0 && d == cfg.rules[r][a].len())
}
