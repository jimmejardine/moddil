//! Token rules: the character-level subset of the dialect (`reference/grammar/README.md`).
//!
//! Quoted literals, character classes, the built-in classes `XID_START` and `XID_CONTINUE`,
//! references to other token rules, `A - B` (A, except the literals B lists), and the markers
//! `%skip` (dropped between syntax tokens) and `%scanner` (not regular; each generator implements
//! the rule stated in the grammar's comment natively).

use crate::GrammarError;
use crate::parse::Quant;

/// A token expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Tx {
    Lit(String),
    Class(Class),
    Named(Named),
    Ref(String),
    Seq(Vec<Tx>),
    Alt(Vec<Tx>),
    Rep(Box<Tx>, Quant),
    /// `A - B`: what A matches, except the literals B lists.
    Minus(Box<Tx>, Box<Tx>),
}

/// The built-in Unicode classes (UAX #31).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Named {
    XidStart,
    XidContinue,
}

/// `[…]`: characters and inclusive ranges, possibly negated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Class {
    pub(crate) negated: bool,
    pub(crate) ranges: Vec<(char, char)>,
}

/// A token rule's definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TokenDef {
    pub(crate) skip: bool,
    pub(crate) scanner: bool,
    /// `None` for scanner tokens.
    pub(crate) expr: Option<Tx>,
}

#[derive(Debug, Clone, PartialEq)]
enum T {
    Lit(String),
    Class(Class),
    Name(String),
    Skip,
    Scanner,
    Bar,
    Open,
    Close,
    Minus,
    Quant(Quant),
}

fn lex(body: &str, line: usize) -> Result<Vec<T>, GrammarError> {
    let chars: Vec<char> = body.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            _ if c.is_whitespace() => i += 1,
            '\'' | '"' => {
                let len = chars[i + 1..]
                    .iter()
                    .position(|&d| d == c)
                    .ok_or_else(|| GrammarError::at(line, "unterminated literal"))?;
                out.push(T::Lit(chars[i + 1..=i + len].iter().collect()));
                i += len + 2;
            }
            '[' => {
                let (class, used) = class(&chars[i + 1..], line)?;
                out.push(T::Class(class));
                i += used + 1;
            }
            '%' => {
                let end = (i + 1..chars.len())
                    .find(|&j| !chars[j].is_ascii_alphabetic())
                    .unwrap_or(chars.len());
                let word: String = chars[i + 1..end].iter().collect();
                out.push(match word.as_str() {
                    "skip" => T::Skip,
                    "scanner" => T::Scanner,
                    _ => return Err(GrammarError::at(line, format!("unknown marker `%{word}`"))),
                });
                i = end;
            }
            '|' => {
                out.push(T::Bar);
                i += 1;
            }
            '(' => {
                out.push(T::Open);
                i += 1;
            }
            ')' => {
                out.push(T::Close);
                i += 1;
            }
            '-' => {
                out.push(T::Minus);
                i += 1;
            }
            '?' => {
                out.push(T::Quant(Quant::Opt));
                i += 1;
            }
            '*' => {
                out.push(T::Quant(Quant::Star));
                i += 1;
            }
            '+' => {
                out.push(T::Quant(Quant::Plus));
                i += 1;
            }
            _ if c.is_ascii_alphabetic() || c == '_' => {
                let end = (i..chars.len())
                    .find(|&j| !(chars[j].is_ascii_alphanumeric() || chars[j] == '_'))
                    .unwrap_or(chars.len());
                out.push(T::Name(chars[i..end].iter().collect()));
                i = end;
            }
            _ => {
                return Err(GrammarError::at(
                    line,
                    format!("unexpected `{c}` in a token rule"),
                ));
            }
        }
    }
    Ok(out)
}

/// Parses a class body after `[`; returns it and the characters used, including `]`.
fn class(chars: &[char], line: usize) -> Result<(Class, usize), GrammarError> {
    let mut i = 0;
    let negated = chars.first() == Some(&'^');
    if negated {
        i += 1;
    }
    let mut singles: Vec<char> = Vec::new();
    let mut ranges = Vec::new();
    let next = |i: &mut usize| -> Result<Option<char>, GrammarError> {
        let Some(&c) = chars.get(*i) else {
            return Err(GrammarError::at(line, "unterminated class"));
        };
        *i += 1;
        if c == ']' {
            return Ok(None);
        }
        if c != '\\' {
            return Ok(Some(c));
        }
        let Some(&e) = chars.get(*i) else {
            return Err(GrammarError::at(line, "unterminated class"));
        };
        *i += 1;
        Ok(Some(match e {
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            other => other,
        }))
    };
    loop {
        let raw_dash = chars.get(i) == Some(&'-');
        let Some(c) = next(&mut i)? else { break };
        // `a-z` is a range; a `-` first or last (or escaped) is itself.
        if !raw_dash && chars.get(i) == Some(&'-') && chars.get(i + 1).is_some_and(|&d| d != ']') {
            i += 1;
            let Some(hi) = next(&mut i)? else {
                return Err(GrammarError::at(line, "unterminated range"));
            };
            if hi < c {
                return Err(GrammarError::at(line, format!("empty range {c}-{hi}")));
            }
            ranges.push((c, hi));
        } else {
            singles.push(c);
        }
    }
    ranges.extend(singles.into_iter().map(|c| (c, c)));
    if ranges.is_empty() {
        return Err(GrammarError::at(line, "empty class"));
    }
    Ok((Class { negated, ranges }, i))
}

struct P {
    toks: Vec<T>,
    pos: usize,
    line: usize,
}

impl P {
    fn peek(&self) -> Option<&T> {
        self.toks.get(self.pos)
    }

    fn alt(&mut self) -> Result<Tx, GrammarError> {
        let mut alts = vec![self.seq()?];
        while self.peek() == Some(&T::Bar) {
            self.pos += 1;
            alts.push(self.seq()?);
        }
        Ok(if alts.len() == 1 {
            alts.remove(0)
        } else {
            Tx::Alt(alts)
        })
    }

    fn seq(&mut self) -> Result<Tx, GrammarError> {
        let mut items = Vec::new();
        while !matches!(self.peek(), None | Some(T::Bar | T::Close)) {
            let mut item = self.term()?;
            if self.peek() == Some(&T::Minus) {
                self.pos += 1;
                let except = self.term()?;
                item = Tx::Minus(Box::new(item), Box::new(except));
            }
            items.push(item);
        }
        match items.len() {
            0 => Err(GrammarError::at(
                self.line,
                "empty alternative in a token rule",
            )),
            1 => Ok(items.remove(0)),
            _ => Ok(Tx::Seq(items)),
        }
    }

    fn term(&mut self) -> Result<Tx, GrammarError> {
        let atom = match self.peek().cloned() {
            Some(T::Lit(s)) => Tx::Lit(s),
            Some(T::Class(c)) => Tx::Class(c),
            Some(T::Name(n)) => match n.as_str() {
                "XID_START" => Tx::Named(Named::XidStart),
                "XID_CONTINUE" => Tx::Named(Named::XidContinue),
                _ => Tx::Ref(n),
            },
            Some(T::Open) => {
                self.pos += 1;
                let inner = self.alt()?;
                if self.peek() != Some(&T::Close) {
                    return Err(GrammarError::at(self.line, "missing `)` in a token rule"));
                }
                inner
            }
            _ => return Err(GrammarError::at(self.line, "expected a token expression")),
        };
        self.pos += 1;
        if let Some(T::Quant(q)) = self.peek() {
            let q = *q;
            self.pos += 1;
            return Ok(Tx::Rep(Box::new(atom), q));
        }
        Ok(atom)
    }
}

/// Parses a token rule's body.
pub(crate) fn parse_token(body: &str, line: usize) -> Result<TokenDef, GrammarError> {
    let toks = lex(body, line)?;
    let mut p = P { toks, pos: 0, line };
    let mut def = TokenDef {
        skip: false,
        scanner: false,
        expr: None,
    };
    loop {
        match p.peek() {
            Some(T::Skip) => def.skip = true,
            Some(T::Scanner) => def.scanner = true,
            _ => break,
        }
        p.pos += 1;
    }
    if def.scanner {
        if p.peek().is_some() {
            return Err(GrammarError::at(line, "a %scanner token has no expression"));
        }
        return Ok(def);
    }
    def.expr = Some(p.alt()?);
    if p.pos < p.toks.len() {
        return Err(GrammarError::at(line, "unbalanced `)` in a token rule"));
    }
    Ok(def)
}

/// The literals an exclusion lists: `'a'` or `( 'a' | 'b' )`.
pub(crate) fn literals(tx: &Tx) -> Option<Vec<String>> {
    match tx {
        Tx::Lit(s) => Some(vec![s.clone()]),
        Tx::Alt(alts) => alts
            .iter()
            .map(|a| literals(a).map(|mut v| v.remove(0)))
            .collect(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_ranges_and_escapes() {
        let def = parse_token(r#"[^"\\] | [a-z_\-] | [ \t\n]"#, 1).expect("parses");
        let Some(Tx::Alt(alts)) = def.expr else {
            panic!("alternatives")
        };
        assert_eq!(
            alts[0],
            Tx::Class(Class {
                negated: true,
                ranges: vec![('"', '"'), ('\\', '\\')]
            })
        );
        assert_eq!(
            alts[1],
            Tx::Class(Class {
                negated: false,
                ranges: vec![('a', 'z'), ('_', '_'), ('-', '-')]
            })
        );
    }

    #[test]
    fn markers_and_exclusion() {
        let def = parse_token("%skip %scanner", 1).expect("parses");
        assert!(def.skip && def.scanner && def.expr.is_none());
        let def = parse_token("IDENT_TEXT - ( 'r' | 'b' )", 1).expect("parses");
        let Some(Tx::Minus(_, except)) = def.expr else {
            panic!("exclusion")
        };
        assert_eq!(
            literals(&except),
            Some(vec!["r".to_string(), "b".to_string()])
        );
    }
}
