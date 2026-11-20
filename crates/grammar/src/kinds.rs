//! The token kinds of `moddil-syntax` (`crates/syntax/src/generated/kinds.rs`).
//!
//! Every token the grammar names gets a `SyntaxKind` constant: the token rules, each punctuation
//! terminal, each keyword and each other literal terminal (`_`). The lexer emits single-character
//! punctuation with a joint flag; the parser glues compounds such as `>>=` from the `COMPOUNDS`
//! table, so the table and the lexer's alphabet are generated together and cannot drift.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use crate::parse::{Alt, Atom, Body, Grammar};

fn char_name(c: char) -> &'static str {
    match c {
        '+' => "PLUS",
        '-' => "MINUS",
        '*' => "STAR",
        '/' => "SLASH",
        '%' => "PERCENT",
        '^' => "CARET",
        '!' => "BANG",
        '&' => "AMP",
        '|' => "PIPE",
        '@' => "AT",
        '\\' => "BACKSLASH",
        '?' => "QUESTION",
        '=' => "EQ",
        '<' => "LT",
        '>' => "GT",
        '.' => "DOT",
        ',' => "COMMA",
        ';' => "SEMI",
        ':' => "COLON",
        '(' => "L_PAREN",
        ')' => "R_PAREN",
        '[' => "L_BRACK",
        ']' => "R_BRACK",
        '{' => "L_CURLY",
        '}' => "R_CURLY",
        '#' => "POUND",
        _ => panic!("no name for punctuation character {c:?}"),
    }
}

fn punct_name(p: &str) -> String {
    p.chars().map(char_name).collect::<Vec<_>>().join("_")
}

fn rust_str(s: &str) -> String {
    format!("{s:?}")
}

fn terms<'a>(alt: &'a Alt, out: &mut BTreeSet<&'a str>) {
    for seq in &alt.0 {
        for item in &seq.items {
            match &item.atom {
                Atom::Term(t) => {
                    out.insert(t.as_str());
                }
                Atom::Group(inner) => terms(inner, out),
                Atom::Ref { .. } => {}
            }
        }
    }
}

/// Emits `kinds.rs`, included into `moddil-syntax`'s `SyntaxKind`.
pub(crate) fn emit(grammar: &Grammar) -> String {
    let keywords: Vec<&str> = grammar
        .rules
        .iter()
        .find(|r| r.name == "KEYWORD")
        .and_then(|r| match &r.body {
            Body::Keywords(w) => Some(w.iter().map(String::as_str).collect()),
            _ => None,
        })
        .expect("a KEYWORD list");
    let punct: Vec<&str> = grammar
        .rules
        .iter()
        .find(|r| r.name == "PUNCT")
        .and_then(|r| match &r.body {
            Body::Keywords(w) => Some(w.iter().map(String::as_str).collect()),
            _ => None,
        })
        .expect("a PUNCT list");
    // Token rules: those syntax rules name, and the skipped ones.
    let mut used = BTreeSet::new();
    let mut literal_terms = BTreeSet::new();
    for rule in &grammar.rules {
        if let Body::Syntax(alt) = &rule.body {
            terms(alt, &mut literal_terms);
            crate::treesitter::refs(alt, &mut used);
        }
    }
    let tokens: Vec<&str> = grammar
        .rules
        .iter()
        .filter(|r| match &r.body {
            Body::Token(def) => def.skip || used.contains(r.name.as_str()),
            _ => false,
        })
        .map(|r| r.name.as_str())
        .collect();
    // Single characters: every character of a punctuation terminal that is itself a terminal.
    let singles: Vec<&str> = punct
        .iter()
        .copied()
        .filter(|p| p.chars().count() == 1)
        .collect();
    let is_single = |c: char| singles.iter().any(|s| s.starts_with(c));
    let compounds: Vec<&str> = punct
        .iter()
        .copied()
        .filter(|p| p.chars().count() > 1)
        .collect();
    // Compounds the lexer emits whole: those whose characters are not all single tokens (`#[`).
    let lexed: Vec<&str> = compounds
        .iter()
        .copied()
        .filter(|p| !p.chars().all(is_single))
        .collect();
    let glued: Vec<&str> = compounds
        .iter()
        .copied()
        .filter(|p| p.chars().all(is_single))
        .collect();
    let others: Vec<&str> = literal_terms
        .iter()
        .copied()
        .filter(|t| !keywords.contains(t) && !punct.contains(t))
        .collect();
    for o in &others {
        assert!(
            *o == "_",
            "an unexpected literal terminal {o:?}: give it a kind name"
        );
    }

    // (const name, display, fixed text)
    let mut kinds: Vec<(String, String, String)> = vec![
        ("EOF".into(), "end of file".into(), String::new()),
        ("ERROR".into(), "ERROR".into(), String::new()),
    ];
    for t in &tokens {
        kinds.push(((*t).to_string(), (*t).to_string(), String::new()));
    }
    for p in singles.iter().chain(&lexed).chain(&glued) {
        kinds.push((punct_name(p), format!("'{p}'"), (*p).to_string()));
    }
    for k in &keywords {
        kinds.push((
            format!("{}_KW", k.to_ascii_uppercase()),
            format!("'{k}'"),
            (*k).to_string(),
        ));
    }
    kinds.push(("UNDERSCORE".into(), "'_'".into(), "_".into()));
    // `Self` and `self` would collide: `Self` is SELF_TYPE_KW.
    for k in &mut kinds {
        if k.2 == "Self" {
            k.0 = "SELF_TYPE_KW".into();
        }
    }

    let mut out = String::new();
    out.push_str(
        "// Generated by moddil-grammar from reference/grammar/moddil.ebnf: do not edit.\n",
    );
    out.push_str("// Included into kinds.rs.\n\n");
    out.push_str("impl SyntaxKind {\n");
    for (i, (name, display, _)) in kinds.iter().enumerate() {
        let _ = writeln!(
            out,
            "    /// {}",
            display
                .replace('\\', "\\\\")
                .replace('[', "\\[")
                .replace(']', "\\]")
        );
        let _ = writeln!(out, "    pub const {name}: SyntaxKind = SyntaxKind({i});");
    }
    out.push_str("}\n\n");
    let _ = writeln!(out, "/// The number of token kinds; node kinds follow.");
    let _ = writeln!(
        out,
        "pub(crate) const TOKEN_COUNT: u16 = {};\n",
        kinds.len()
    );
    out.push_str("/// Each token kind's name, as in the grammar.\n");
    let _ = writeln!(
        out,
        "pub(crate) const TOKEN_NAMES: [&str; {}] = [",
        kinds.len()
    );
    for (name, _, _) in &kinds {
        let _ = writeln!(out, "    {},", rust_str(name));
    }
    out.push_str("];\n\n");
    out.push_str("/// Each token kind's fixed text (punctuation, keywords, `_`), or \"\".\n");
    let _ = writeln!(
        out,
        "pub(crate) const TOKEN_TEXT: [&str; {}] = [",
        kinds.len()
    );
    for (_, _, text) in &kinds {
        let _ = writeln!(out, "    {},", rust_str(text));
    }
    out.push_str("];\n\n");
    let kind_of = |text: &str| -> String {
        kinds
            .iter()
            .find(|k| k.2 == text)
            .map(|k| k.0.clone())
            .expect("a kind")
    };
    out.push_str("/// Keywords, which an identifier's text may not be.\n");
    out.push_str("pub(crate) const KEYWORDS: &[(&str, SyntaxKind)] = &[\n");
    for k in &keywords {
        let _ = writeln!(out, "    ({}, SyntaxKind::{}),", rust_str(k), kind_of(k));
    }
    out.push_str("];\n\n");
    out.push_str("/// Punctuation the lexer emits: single characters, and compounds that are not made of them.\n");
    out.push_str("pub(crate) const LEXED_PUNCT: &[(&str, SyntaxKind)] = &[\n");
    let mut lexed_sorted: Vec<&str> = lexed.iter().chain(&singles).copied().collect();
    lexed_sorted.sort_by_key(|p| std::cmp::Reverse(p.len()));
    for p in lexed_sorted {
        let _ = writeln!(out, "    ({}, SyntaxKind::{}),", rust_str(p), kind_of(p));
    }
    out.push_str("];\n\n");
    out.push_str(
        "/// Compound punctuation the parser glues from joint single characters, longest first.\n",
    );
    out.push_str("pub(crate) const COMPOUNDS: &[(&str, SyntaxKind)] = &[\n");
    let mut glued_sorted = glued.clone();
    glued_sorted.sort_by_key(|p| std::cmp::Reverse(p.len()));
    for p in glued_sorted {
        let _ = writeln!(out, "    ({}, SyntaxKind::{}),", rust_str(p), kind_of(p));
    }
    out.push_str("];\n\n");
    out.push_str("/// A token kind by its spelling: `T![fn]`, `T![\"+=\"]`, `T![_]`.\n");
    out.push_str("#[macro_export]\nmacro_rules! T {\n");
    for (name, _, text) in &kinds {
        if text.is_empty() {
            continue;
        }
        let pattern = if text.chars().all(|c| c.is_alphanumeric() || c == '_') {
            text.clone()
        } else {
            rust_str(text)
        };
        let _ = writeln!(out, "    [{pattern}] => {{ $crate::SyntaxKind::{name} }};");
    }
    out.push_str("}\n");
    out
}
