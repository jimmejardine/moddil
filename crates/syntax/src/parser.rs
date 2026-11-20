//! The parser's machinery: lookahead over grammar tokens, events, markers and building the tree.
//!
//! The grammar functions (`grammar/`) see the token stream without trivia, as the grammar's tokens:
//! joint single characters are glued into compounds (`>`, `>`, `=` reads as `>>=`) by maximal munch,
//! as the grammar's lexer would read them. Where the grammar splits a compound (`>>` closing two
//! generic lists, `&&` as two borrows) the parser takes just its first character with
//! `bump_split`. Parsing emits events (start a node, a token, finish a node, an error) and the
//! builder turns them into a lossless tree, putting the trivia back.

use crate::lexer::Token;
use crate::tree::{GreenBuilder, GreenNode};
use crate::{SyntaxError, SyntaxKind, TextRange};

/// A set of token kinds.
#[derive(Clone, Copy, Default)]
pub(crate) struct TokenSet([u64; 4]);

impl TokenSet {
    pub(crate) const fn new(kinds: &[SyntaxKind]) -> TokenSet {
        let mut bits = [0u64; 4];
        let mut i = 0;
        while i < kinds.len() {
            let k = kinds[i].0 as usize;
            bits[k / 64] |= 1 << (k % 64);
            i += 1;
        }
        TokenSet(bits)
    }

    pub(crate) const fn union(self, other: TokenSet) -> TokenSet {
        TokenSet([
            self.0[0] | other.0[0],
            self.0[1] | other.0[1],
            self.0[2] | other.0[2],
            self.0[3] | other.0[3],
        ])
    }

    pub(crate) fn contains(self, kind: SyntaxKind) -> bool {
        let k = kind.0 as usize;
        k < 256 && self.0[k / 64] & (1 << (k % 64)) != 0
    }
}

#[derive(Debug)]
enum Event {
    /// `forward_parent`: the index of a later `Start` that wraps this node (`precede`).
    Start {
        kind: Option<SyntaxKind>,
        forward_parent: Option<u32>,
    },
    Finish,
    /// `n_raw` lexer tokens glued into one token of `kind`.
    Token {
        kind: SyntaxKind,
        n_raw: u8,
    },
    Error(String),
}

pub(crate) struct Parser {
    /// The tokens without trivia: kind and joint flag.
    tokens: Vec<(SyntaxKind, bool)>,
    pos: usize,
    events: Vec<Event>,
    /// Lookahead calls since the last token was consumed: a guard against loops that make no
    /// progress.
    steps: u32,
}

const STEP_LIMIT: u32 = 100_000;

/// A node being parsed: complete or abandon it.
pub(crate) struct Marker {
    pos: u32,
    done: bool,
}

impl Drop for Marker {
    fn drop(&mut self) {
        if !self.done && !std::thread::panicking() {
            panic!("a marker was neither completed nor abandoned");
        }
    }
}

/// A finished node, which can still be wrapped by a new one (`precede`).
#[derive(Clone, Copy)]
pub(crate) struct CompletedMarker {
    pos: u32,
}

impl Parser {
    pub(crate) fn new(raw: &[Token]) -> Parser {
        let tokens = raw
            .iter()
            .filter(|t| !t.kind.is_trivia())
            .map(|t| (t.kind, t.joint))
            .collect();
        Parser {
            tokens,
            pos: 0,
            events: Vec::new(),
            steps: 0,
        }
    }

    fn raw(&self, i: usize) -> SyntaxKind {
        self.tokens.get(i).map_or(SyntaxKind::EOF, |t| t.0)
    }

    /// The grammar token at raw position `i`, by maximal munch: its kind and how many raw tokens
    /// it spans.
    fn munch(&self, i: usize) -> (SyntaxKind, usize) {
        let first = self.raw(i);
        let Some(text) = first
            .text()
            .filter(|t| t.len() == 1 && !t.chars().all(char::is_alphanumeric))
        else {
            return (first, 1);
        };
        let mut s = String::from(text);
        let mut j = i;
        while s.len() < 3 && self.tokens.get(j).is_some_and(|t| t.1) {
            j += 1;
            match self.raw(j).text() {
                Some(t) if t.len() == 1 && !t.chars().all(char::is_alphanumeric) => s.push_str(t),
                _ => break,
            }
        }
        for &(compound, kind) in SyntaxKind::compounds() {
            if s.starts_with(compound) {
                return (kind, compound.len());
            }
        }
        (first, 1)
    }

    /// The `n`th grammar token ahead.
    pub(crate) fn nth(&mut self, n: usize) -> SyntaxKind {
        self.steps += 1;
        assert!(
            self.steps < STEP_LIMIT,
            "the parser is stuck at token {}",
            self.pos
        );
        let mut i = self.pos;
        for _ in 0..n {
            i += self.munch(i).1;
        }
        self.munch(i).0
    }

    pub(crate) fn current(&mut self) -> SyntaxKind {
        self.nth(0)
    }

    pub(crate) fn at(&mut self, kind: SyntaxKind) -> bool {
        self.nth(0) == kind
    }

    pub(crate) fn nth_at(&mut self, n: usize, kind: SyntaxKind) -> bool {
        self.nth(n) == kind
    }

    pub(crate) fn at_set(&mut self, set: TokenSet) -> bool {
        let k = self.nth(0);
        set.contains(k)
    }

    /// The next raw character is `kind`, even as the first character of a compound.
    pub(crate) fn at_split(&mut self, kind: SyntaxKind) -> bool {
        self.steps += 1;
        self.raw(self.pos) == kind
    }

    /// The raw token position: compare before and after to check for progress.
    pub(crate) fn position(&self) -> usize {
        self.pos
    }

    pub(crate) fn at_eof(&mut self) -> bool {
        self.at(SyntaxKind::EOF)
    }

    fn do_bump(&mut self, kind: SyntaxKind, n_raw: usize) {
        self.pos += n_raw;
        self.steps = 0;
        self.events.push(Event::Token {
            kind,
            n_raw: n_raw as u8,
        });
    }

    /// Consumes the current grammar token, which must be `kind`.
    pub(crate) fn bump(&mut self, kind: SyntaxKind) {
        let (k, n) = self.munch(self.pos);
        assert_eq!(k, kind, "bump: expected {kind:?}");
        self.do_bump(kind, n);
    }

    /// Consumes the current grammar token, whatever it is.
    pub(crate) fn bump_any(&mut self) {
        if self.pos >= self.tokens.len() {
            return;
        }
        let (k, n) = self.munch(self.pos);
        self.do_bump(k, n);
    }

    /// Consumes one raw character, splitting a compound: `kind` must be its first character.
    pub(crate) fn bump_split(&mut self, kind: SyntaxKind) {
        assert_eq!(self.raw(self.pos), kind, "bump_split");
        self.do_bump(kind, 1);
    }

    pub(crate) fn eat(&mut self, kind: SyntaxKind) -> bool {
        if self.at(kind) {
            self.bump(kind);
            true
        } else {
            false
        }
    }

    /// Consumes `kind`, or records "expected …" and consumes nothing.
    pub(crate) fn expect(&mut self, kind: SyntaxKind) -> bool {
        if self.eat(kind) {
            return true;
        }
        let found = self.current();
        self.error(format!(
            "expected {}, found {}",
            kind.describe(),
            found.describe()
        ));
        false
    }

    pub(crate) fn error(&mut self, message: impl Into<String>) {
        self.events.push(Event::Error(message.into()));
    }

    /// Records an error and wraps the current token in an `ERROR` node, unless it is in
    /// `recovery` (or the end of the file), where the caller can carry on.
    pub(crate) fn err_recover(&mut self, message: impl Into<String>, recovery: TokenSet) {
        self.error(message);
        if self.at_eof() || self.at_set(recovery) {
            return;
        }
        let m = self.start();
        self.bump_any();
        m.complete(self, SyntaxKind::ERROR);
    }

    /// Records an error and wraps the current token in an `ERROR` node.
    pub(crate) fn err_and_bump(&mut self, message: impl Into<String>) {
        self.err_recover(message, TokenSet::default());
    }

    pub(crate) fn start(&mut self) -> Marker {
        let pos = self.events.len() as u32;
        self.events.push(Event::Start {
            kind: None,
            forward_parent: None,
        });
        Marker { pos, done: false }
    }

    pub(crate) fn finish(self, raw: &[Token], text: &str) -> (GreenNode, Vec<SyntaxError>) {
        build(self.events, raw, text)
    }
}

impl Marker {
    pub(crate) fn complete(mut self, p: &mut Parser, kind: SyntaxKind) -> CompletedMarker {
        self.done = true;
        match &mut p.events[self.pos as usize] {
            Event::Start { kind: k, .. } => *k = Some(kind),
            _ => unreachable!("a marker points at a Start"),
        }
        p.events.push(Event::Finish);
        CompletedMarker { pos: self.pos }
    }

    pub(crate) fn abandon(mut self, p: &mut Parser) {
        self.done = true;
        if self.pos as usize == p.events.len() - 1 {
            p.events.pop();
        }
    }
}

impl CompletedMarker {
    /// A new node that starts where this one starts and will contain it.
    pub(crate) fn precede(self, p: &mut Parser) -> Marker {
        let m = p.start();
        match &mut p.events[self.pos as usize] {
            Event::Start { forward_parent, .. } => *forward_parent = Some(m.pos),
            _ => unreachable!("a marker points at a Start"),
        }
        m
    }
}

/// Nodes that take the comments directly above them (with no blank line between).
fn attaches_comments(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::FN_ITEM
            | SyntaxKind::STRUCT_ITEM
            | SyntaxKind::VIEW_ITEM
            | SyntaxKind::ENUM_ITEM
            | SyntaxKind::TRAIT_ITEM
            | SyntaxKind::IMPL_ITEM
            | SyntaxKind::TYPE_ITEM
            | SyntaxKind::CONST_ITEM
            | SyntaxKind::USE_ITEM
            | SyntaxKind::MOD_ITEM
            | SyntaxKind::TEST_ITEM
            | SyntaxKind::PROPERTY_ITEM
            | SyntaxKind::BENCH_ITEM
            | SyntaxKind::GROUP_ITEM
            | SyntaxKind::UNIT_ITEM
            | SyntaxKind::DIMENSION_ITEM
            | SyntaxKind::ASSOC_TYPE
            | SyntaxKind::FIELD
            | SyntaxKind::VARIANT
            | SyntaxKind::MATCH_ARM
            | SyntaxKind::LET_STMT
    )
}

struct Sink<'a> {
    raw: &'a [Token],
    text: &'a str,
    /// The next raw token.
    at: usize,
    offset: usize,
    builder: GreenBuilder,
    errors: Vec<SyntaxError>,
    depth: usize,
}

impl Sink<'_> {
    fn raw_text(&self, i: usize) -> &str {
        let start: usize = self.offset
            + self.raw[self.at..i]
                .iter()
                .map(|t| t.len as usize)
                .sum::<usize>();
        &self.text[start..start + self.raw[i].len as usize]
    }

    fn emit_raw(&mut self) {
        let t = self.raw[self.at];
        let len = t.len as usize;
        self.builder
            .token(t.kind, &self.text[self.offset..self.offset + len]);
        self.offset += len;
        self.at += 1;
    }

    /// The number of trivia tokens ahead.
    fn trivia_ahead(&self) -> usize {
        self.raw[self.at..]
            .iter()
            .take_while(|t| t.kind.is_trivia())
            .count()
    }

    fn eat_trivia(&mut self, n: usize) {
        for _ in 0..n {
            self.emit_raw();
        }
    }

    /// How many of the `n` trivia tokens ahead belong to a node of `kind` starting after them.
    fn attached(&self, n: usize, kind: SyntaxKind) -> usize {
        if !attaches_comments(kind) {
            return 0;
        }
        let mut attached = 0;
        let mut i = n;
        while i > 0 {
            let t = self.raw[self.at + i - 1];
            match t.kind {
                SyntaxKind::WHITESPACE => {
                    let ws = self.raw_text(self.at + i - 1);
                    if ws.matches('\n').count() >= 2 {
                        break;
                    }
                }
                SyntaxKind::LINE_COMMENT | SyntaxKind::BLOCK_COMMENT => attached = n - i + 1,
                _ => break,
            }
            i -= 1;
        }
        attached
    }

    fn start(&mut self, kind: SyntaxKind) {
        if self.depth == 0 {
            self.builder.start_node(kind);
        } else {
            let n = self.trivia_ahead();
            let keep = self.attached(n, kind);
            self.eat_trivia(n - keep);
            self.builder.start_node(kind);
        }
        self.depth += 1;
    }

    fn token(&mut self, kind: SyntaxKind, n_raw: usize) {
        let n = self.trivia_ahead();
        self.eat_trivia(n);
        let len: usize = self.raw[self.at..self.at + n_raw]
            .iter()
            .map(|t| t.len as usize)
            .sum();
        self.builder
            .token(kind, &self.text[self.offset..self.offset + len]);
        self.offset += len;
        self.at += n_raw;
    }

    fn finish(&mut self) {
        self.depth -= 1;
        if self.depth == 0 {
            // The root takes everything left: trailing trivia, and any tokens the parser never
            // reached (it always reaches the end, but the tree must stay lossless regardless).
            while self.at < self.raw.len() {
                self.emit_raw();
            }
        }
        self.builder.finish_node();
    }

    fn error(&mut self, message: String) {
        let n = self.trivia_ahead();
        let skip: usize = self.raw[self.at..self.at + n]
            .iter()
            .map(|t| t.len as usize)
            .sum();
        let start = (self.offset + skip) as u32;
        let end = self.raw.get(self.at + n).map_or(start, |t| start + t.len);
        // One error per position: the first is the useful one; the rest follow from it.
        if self.errors.last().is_some_and(|e| e.range.start == start) {
            return;
        }
        self.errors
            .push(SyntaxError::new(TextRange::new(start, end), message));
    }
}

fn build(mut events: Vec<Event>, raw: &[Token], text: &str) -> (GreenNode, Vec<SyntaxError>) {
    let mut sink = Sink {
        raw,
        text,
        at: 0,
        offset: 0,
        builder: GreenBuilder::default(),
        errors: Vec::new(),
        depth: 0,
    };
    let mut parents = Vec::new();
    for i in 0..events.len() {
        match std::mem::replace(
            &mut events[i],
            Event::Start {
                kind: None,
                forward_parent: None,
            },
        ) {
            Event::Start {
                kind: Some(kind),
                forward_parent,
            } => {
                // Collect the chain of nodes that wrap this one, outermost first.
                parents.clear();
                parents.push(kind);
                let mut next = forward_parent;
                while let Some(j) = next {
                    match std::mem::replace(
                        &mut events[j as usize],
                        Event::Start {
                            kind: None,
                            forward_parent: None,
                        },
                    ) {
                        Event::Start {
                            kind: Some(k),
                            forward_parent: f,
                        } => {
                            parents.push(k);
                            next = f;
                        }
                        Event::Start { kind: None, .. } => next = None,
                        _ => unreachable!("forward parents are Starts"),
                    }
                }
                for &k in parents.iter().rev() {
                    sink.start(k);
                }
            }
            Event::Start { kind: None, .. } => {}
            Event::Finish => sink.finish(),
            Event::Token { kind, n_raw } => sink.token(kind, usize::from(n_raw)),
            Event::Error(message) => sink.error(message),
        }
    }
    (sink.builder.finish(), sink.errors)
}
