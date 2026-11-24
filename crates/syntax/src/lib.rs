//! The lexer, the hand-written parser and the syntax tree for Moddil source.
//!
//! `parse` turns text into a **lossless** syntax tree (every byte of the input, trivia included,
//! is in it, and its text is the input) plus the syntax errors found. A tree always exists, even
//! for invalid input: errors become `ERROR` nodes and the parser carries on. The typed AST
//! (`ast`) is a view over the tree for the compiler and the reference interpreter.
//!
//! The parser follows `reference/grammar/moddil.ebnf` rule by rule, and the grammar's generated
//! corpus (`reference/grammar/tests/`) keeps the two in step. See this crate's `README.md`.

#[macro_use]
mod kinds;
pub mod ast;
mod doc;
mod grammar;
mod lexer;
mod parser;
mod text;
mod tree;

pub use doc::{DocExample, doc_examples};
pub use kinds::SyntaxKind;
pub use lexer::{Token, lex};
pub use text::{SyntaxError, TextRange, line_col};
pub use tree::{
    GreenBuilder, GreenElement, GreenNode, GreenToken, SyntaxElement, SyntaxNode, SyntaxToken,
};

/// The result of parsing: the tree, and the errors found on the way.
#[derive(Clone, PartialEq, Eq)]
pub struct Parse {
    green: GreenNode,
    errors: Vec<SyntaxError>,
}

impl Parse {
    /// The root of the syntax tree (a `SOURCE_FILE` node).
    pub fn syntax(&self) -> SyntaxNode {
        SyntaxNode::new_root(self.green.clone())
    }

    /// The typed view of the root.
    pub fn tree(&self) -> ast::SourceFile {
        ast::SourceFile::cast(self.syntax()).expect("the root is a SOURCE_FILE")
    }

    /// Lexical and syntax errors, in source order.
    pub fn errors(&self) -> &[SyntaxError] {
        &self.errors
    }

    /// The green tree.
    pub fn green(&self) -> &GreenNode {
        &self.green
    }
}

/// Parses a whole source file.
pub fn parse(text: &str) -> Parse {
    let (tokens, mut errors) = lex(text);
    let mut p = parser::Parser::new(&tokens);
    grammar::source_file(&mut p);
    let (green, parse_errors) = p.finish(&tokens, text);
    errors.extend(parse_errors);
    errors.sort_by_key(|e| e.range.start);
    Parse { green, errors }
}

/// This crate's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
