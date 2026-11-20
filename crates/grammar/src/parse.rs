//! Parsing the grammar's EBNF dialect (`reference/grammar/README.md`): rules, quoted terminals,
//! references with flag modes, alternative guards and quantifiers.

use crate::GrammarError;

/// The `ns` flag: no struct or record literal and no bare block.
pub(crate) const NS: u8 = 1;
/// The `stmt` flag: the expression starts a statement.
pub(crate) const STMT: u8 = 2;
/// The `cond` flag: a `let` condition may appear.
pub(crate) const COND: u8 = 4;

fn flag_bit(name: &str) -> Option<u8> {
    match name {
        "ns" => Some(NS),
        "stmt" => Some(STMT),
        "cond" => Some(COND),
        _ => None,
    }
}

/// Renders a flag set as `ns,stmt`.
pub(crate) fn flag_names(bits: u8) -> String {
    [(NS, "ns"), (STMT, "stmt"), (COND, "cond")]
        .iter()
        .filter(|(bit, _)| bits & bit != 0)
        .map(|(_, name)| *name)
        .collect::<Vec<_>>()
        .join(",")
}

/// How a reference passes flags to the rule it names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FlagMode {
    /// `name`: no flags.
    Fresh,
    /// `name@`: every current flag.
    Inherit,
    /// `name@ns`: only `ns` and `cond`.
    InheritNs,
    /// `name+f+g`: exactly these flags.
    Set(u8),
}

/// `!f` (removed while `f` is set) or `?f` (present only while `f` is set).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Guard {
    pub(crate) present: bool,
    pub(crate) flag: u8,
}

impl Guard {
    pub(crate) fn allows(self, flags: u8) -> bool {
        (flags & self.flag != 0) == self.present
    }
}

#[derive(Debug, Clone)]
pub(crate) enum Atom {
    Term(String),
    Ref {
        name: String,
        mode: FlagMode,
        line: usize,
    },
    Group(Alt),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Quant {
    One,
    Opt,
    /// `??`: optional, taken whenever it can start.
    Greedy,
    Star,
    Plus,
}

#[derive(Debug, Clone)]
pub(crate) struct Item {
    pub(crate) atom: Atom,
    pub(crate) quant: Quant,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Seq {
    pub(crate) guards: Vec<Guard>,
    pub(crate) items: Vec<Item>,
}

#[derive(Debug, Clone)]
pub(crate) struct Alt(pub(crate) Vec<Seq>);

#[derive(Debug, Clone)]
pub(crate) enum Body {
    /// A syntax rule (lower_snake name).
    Syntax(Alt),
    /// A token rule: one terminal, defined by the lexer.
    Token,
    /// A token rule that only lists quoted keywords, such as `KEYWORD`: each is its own terminal.
    Keywords(Vec<String>),
}

#[derive(Debug, Clone)]
pub(crate) struct Rule {
    pub(crate) name: String,
    pub(crate) line: usize,
    pub(crate) body: Body,
}

pub(crate) struct Grammar {
    pub(crate) rules: Vec<Rule>,
}

/// Token rules are named in UPPERCASE.
pub(crate) fn is_token_name(name: &str) -> bool {
    name.chars().any(|c| c.is_ascii_uppercase())
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn is_name_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Removes a `#` comment. Quotes hide `#`; in token rules, so do `[…]` character classes.
fn strip_comment(line: &str, token_rule: bool) -> &str {
    let mut quote: Option<char> = None;
    let mut in_class = false;
    for (i, c) in line.char_indices() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if in_class => in_class = c != ']',
            None => match c {
                '\'' | '"' => quote = Some(c),
                '[' if token_rule => in_class = true,
                '#' => return &line[..i],
                _ => {}
            },
        }
    }
    line
}

fn leading_name(line: &str) -> Option<&str> {
    let end = line
        .char_indices()
        .find(|&(_, c)| !is_name_char(c))
        .map_or(line.len(), |(i, _)| i);
    (end > 0 && line.starts_with(is_name_start)).then(|| &line[..end])
}

/// Parses a grammar file.
pub(crate) fn parse(source: &str) -> Result<Grammar, Vec<GrammarError>> {
    let lines: Vec<&str> = source.lines().collect();
    let mut errors = Vec::new();
    let mut rules = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if line.trim().is_empty() || line.starts_with(char::is_whitespace) || line.starts_with('#')
        {
            if !line.trim().is_empty() && !line.trim_start().starts_with('#') {
                errors.push(GrammarError::at(i + 1, "text outside a rule"));
            }
            i += 1;
            continue;
        }
        let start_line = i + 1;
        let Some(name) = leading_name(line) else {
            errors.push(GrammarError::at(start_line, "expected a rule name"));
            i += 1;
            continue;
        };
        let token_rule = is_token_name(name);
        let mut body = String::new();
        let rest = strip_comment(&line[name.len()..], token_rule).trim();
        if let Some(after) = rest.strip_prefix("::=") {
            body.push_str(after);
            i += 1;
        } else if rest.is_empty() {
            // The name stands alone; `::=` opens the next indented line.
            i += 1;
            while i < lines.len() && lines[i].trim().is_empty() {
                i += 1;
            }
            let next = lines.get(i).map(|l| strip_comment(l, token_rule).trim());
            match next.and_then(|l| l.strip_prefix("::=")) {
                Some(after) if lines[i].starts_with(char::is_whitespace) => {
                    body.push_str(after);
                    i += 1;
                }
                _ => {
                    errors.push(GrammarError::at(
                        start_line,
                        format!("expected `::=` after `{name}`"),
                    ));
                    continue;
                }
            }
        } else {
            errors.push(GrammarError::at(
                start_line,
                format!("expected `::=` after `{name}`"),
            ));
            i += 1;
            continue;
        }
        // Continuation lines are indented; comment lines in between are skipped.
        while i < lines.len() {
            let l = lines[i];
            if l.starts_with(char::is_whitespace) {
                body.push(' ');
                body.push_str(strip_comment(l, token_rule));
            } else if !(l.trim().is_empty() || l.starts_with('#')) {
                break;
            }
            i += 1;
        }
        let body = if token_rule {
            keyword_list(&body).map_or(Body::Token, Body::Keywords)
        } else {
            match parse_body(&body, start_line) {
                Ok(alt) => Body::Syntax(alt),
                Err(e) => {
                    errors.push(e);
                    continue;
                }
            }
        };
        if rules.iter().any(|r: &Rule| r.name == name) {
            errors.push(GrammarError::at(
                start_line,
                format!("`{name}` is defined twice"),
            ));
        }
        rules.push(Rule {
            name: name.to_string(),
            line: start_line,
            body,
        });
    }
    if errors.is_empty() {
        Ok(Grammar { rules })
    } else {
        Err(errors)
    }
}

/// `'a' | 'b' | …` and nothing else.
fn keyword_list(body: &str) -> Option<Vec<String>> {
    body.split('|')
        .map(|part| {
            let part = part.trim();
            let inner = part.strip_prefix('\'')?.strip_suffix('\'')?;
            (!inner.is_empty() && !inner.contains('\'')).then(|| inner.to_string())
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Term(String),
    Ref(String, FlagMode),
    Guard(Guard),
    Bar,
    Open,
    Close,
    Quant(Quant),
}

impl PartialEq for Guard {
    fn eq(&self, other: &Self) -> bool {
        self.present == other.present && self.flag == other.flag
    }
}

fn lex_body(body: &str, line: usize) -> Result<Vec<Tok>, GrammarError> {
    let chars: Vec<char> = body.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    let word = |i: &mut usize| {
        let start = *i;
        while *i < chars.len() && is_name_char(chars[*i]) {
            *i += 1;
        }
        chars[start..*i].iter().collect::<String>()
    };
    let flag = |name: String| {
        flag_bit(&name).ok_or_else(|| GrammarError::at(line, format!("unknown flag `{name}`")))
    };
    while i < chars.len() {
        let c = chars[i];
        match c {
            _ if c.is_whitespace() => i += 1,
            '\'' | '"' => {
                let end = chars[i + 1..]
                    .iter()
                    .position(|&d| d == c)
                    .ok_or_else(|| GrammarError::at(line, "unterminated terminal"))?;
                let text: String = chars[i + 1..i + 1 + end].iter().collect();
                if text.is_empty() {
                    return Err(GrammarError::at(line, "empty terminal"));
                }
                toks.push(Tok::Term(text));
                i += end + 2;
            }
            '|' => {
                toks.push(Tok::Bar);
                i += 1;
            }
            '(' => {
                toks.push(Tok::Open);
                i += 1;
            }
            ')' => {
                toks.push(Tok::Close);
                i += 1;
            }
            '*' => {
                toks.push(Tok::Quant(Quant::Star));
                i += 1;
            }
            '+' => {
                toks.push(Tok::Quant(Quant::Plus));
                i += 1;
            }
            '!' | '?' if chars.get(i + 1).is_some_and(|d| d.is_ascii_lowercase()) => {
                i += 1;
                let bit = flag(word(&mut i))?;
                toks.push(Tok::Guard(Guard {
                    present: c == '?',
                    flag: bit,
                }));
            }
            '?' if chars.get(i + 1) == Some(&'?') => {
                toks.push(Tok::Quant(Quant::Greedy));
                i += 2;
            }
            '?' => {
                toks.push(Tok::Quant(Quant::Opt));
                i += 1;
            }
            _ if is_name_start(c) => {
                let name = word(&mut i);
                let mode = if chars.get(i) == Some(&'@') {
                    i += 1;
                    if chars.get(i).is_some_and(|d| d.is_ascii_lowercase()) {
                        let f = word(&mut i);
                        if f != "ns" {
                            return Err(GrammarError::at(
                                line,
                                format!("`@{f}`: only `@` and `@ns` exist"),
                            ));
                        }
                        FlagMode::InheritNs
                    } else {
                        FlagMode::Inherit
                    }
                } else if chars.get(i) == Some(&'+')
                    && chars.get(i + 1).is_some_and(|d| d.is_ascii_lowercase())
                {
                    let mut bits = 0;
                    while chars.get(i) == Some(&'+')
                        && chars.get(i + 1).is_some_and(|d| d.is_ascii_lowercase())
                    {
                        i += 1;
                        bits |= flag(word(&mut i))?;
                    }
                    FlagMode::Set(bits)
                } else {
                    FlagMode::Fresh
                };
                toks.push(Tok::Ref(name, mode));
            }
            _ => {
                return Err(GrammarError::at(
                    line,
                    format!("unexpected `{c}` in a syntax rule"),
                ));
            }
        }
    }
    Ok(toks)
}

struct BodyParser {
    toks: Vec<Tok>,
    pos: usize,
    line: usize,
}

fn parse_body(body: &str, line: usize) -> Result<Alt, GrammarError> {
    let mut p = BodyParser {
        toks: lex_body(body, line)?,
        pos: 0,
        line,
    };
    let alt = p.alt()?;
    if p.pos < p.toks.len() {
        return Err(GrammarError::at(line, "unbalanced `)`"));
    }
    Ok(alt)
}

impl BodyParser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn alt(&mut self) -> Result<Alt, GrammarError> {
        let mut seqs = vec![self.seq()?];
        while self.peek() == Some(&Tok::Bar) {
            self.pos += 1;
            seqs.push(self.seq()?);
        }
        Ok(Alt(seqs))
    }

    fn seq(&mut self) -> Result<Seq, GrammarError> {
        let mut seq = Seq::default();
        while let Some(Tok::Guard(g)) = self.peek() {
            seq.guards.push(*g);
            self.pos += 1;
        }
        loop {
            let atom = match self.peek().cloned() {
                None | Some(Tok::Bar | Tok::Close) => break,
                Some(Tok::Term(t)) => {
                    self.pos += 1;
                    Atom::Term(t)
                }
                Some(Tok::Ref(name, mode)) => {
                    self.pos += 1;
                    Atom::Ref {
                        name,
                        mode,
                        line: self.line,
                    }
                }
                Some(Tok::Open) => {
                    self.pos += 1;
                    let inner = self.alt()?;
                    if self.peek() != Some(&Tok::Close) {
                        return Err(GrammarError::at(self.line, "missing `)`"));
                    }
                    self.pos += 1;
                    Atom::Group(inner)
                }
                Some(Tok::Guard(_)) => {
                    return Err(GrammarError::at(
                        self.line,
                        "a guard must start an alternative",
                    ));
                }
                Some(Tok::Quant(_)) => {
                    return Err(GrammarError::at(
                        self.line,
                        "a quantifier needs something to repeat",
                    ));
                }
            };
            let quant = match self.peek() {
                Some(Tok::Quant(q)) => {
                    let q = *q;
                    self.pos += 1;
                    q
                }
                _ => Quant::One,
            };
            seq.items.push(Item { atom, quant });
        }
        Ok(seq)
    }
}
