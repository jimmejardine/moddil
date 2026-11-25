//! The canonical formatter for Moddil source (`moddil fmt`).
//!
//! [`format`] prints a file from its syntax tree. The layout is a function of the tree and its
//! comments alone: there is no line width and no setting, so the same code always prints the
//! same way. Lines break where the structure says (between items, members, fields, variants,
//! statements and match arms, inside a block with statements, and before a function's `where`
//! and contract clauses) and after a line comment, which is also how a writer breaks a list by
//! choice. See this crate's `README.md` for the rules.

mod out;
mod print;
mod space;

use std::fmt;

use moddil_syntax::{SyntaxError, SyntaxKind as K, SyntaxNode};

/// Why a file could not be formatted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The file has syntax errors: it is not formatted until it parses.
    Syntax(Vec<SyntaxError>),
    /// The formatted text would not mean what the source does (a bug in the formatter): the
    /// source is left as it is.
    Changed(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Syntax(errors) => {
                let n = errors.len();
                write!(f, "{n} syntax error{}", if n == 1 { "" } else { "s" })
            }
            Error::Changed(why) => write!(
                f,
                "formatting would change the program ({why}): a bug in moddil fmt"
            ),
        }
    }
}

/// `text` in canonical format, with `\n` line endings. It is checked before it is returned: it
/// parses, and it has the source's tokens (but for trailing commas) and comments, in order.
pub fn format(text: &str) -> Result<String, Error> {
    let text = text.replace("\r\n", "\n");
    let parse = moddil_syntax::parse(&text);
    if !parse.errors().is_empty() {
        return Err(Error::Syntax(parse.errors().to_vec()));
    }
    let root = parse.syntax();
    let formatted = print::print(&root);
    let again = moddil_syntax::parse(&formatted);
    if let Some(e) = again.errors().first() {
        return Err(Error::Changed(format!(
            "the result does not parse: {}",
            e.message
        )));
    }
    let (before, after) = (significant(&root), significant(&again.syntax()));
    if before != after {
        let at = before
            .iter()
            .zip(&after)
            .position(|(a, b)| a != b)
            .unwrap_or(before.len().min(after.len()));
        let show = |s: &[(K, String)]| {
            s.get(at)
                .map_or("the end".to_string(), |(_, t)| format!("`{t}`"))
        };
        return Err(Error::Changed(format!(
            "token {at} was {}, and would be {}",
            show(&before),
            show(&after)
        )));
    }
    Ok(formatted)
}

/// What formatting must keep: every token but whitespace, with comments' trailing spaces
/// trimmed, and without the commas the layout adds or drops (before a closing delimiter, and
/// after a match arm).
fn significant(root: &SyntaxNode) -> Vec<(K, String)> {
    let tokens: Vec<_> = root
        .tokens()
        .into_iter()
        .filter(|t| t.kind() != K::WHITESPACE && t.kind() != K::EOF)
        .collect();
    let mut out = Vec::new();
    for (i, t) in tokens.iter().enumerate() {
        if t.kind() == K::COMMA {
            let next = tokens[i + 1..]
                .iter()
                .find(|t| !t.kind().is_trivia())
                .map(|t| t.kind());
            let closes = matches!(
                next,
                Some(K::R_PAREN | K::R_BRACK | K::R_CURLY | K::GT | K::PIPE)
            );
            if closes || t.parent().kind() == K::MATCH_ARM {
                continue;
            }
        }
        out.push((t.kind(), t.text().trim_end().to_string()));
    }
    out
}

/// This crate's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
