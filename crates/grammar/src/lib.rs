//! Owns `reference/grammar/moddil.ebnf`: parses the grammar's EBNF dialect, expands its flags and
//! checks that the grammar is LL(k), which also proves it unambiguous.
//!
//! See this crate's `README.md`, and `reference/grammar/README.md` for the dialect. The generated
//! parser tests, tree-sitter grammar and constrained-decoding grammars come next.

mod expand;
mod ll;
mod parse;

use std::fmt;

/// This crate's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The lookahead Moddil's grammar is designed for.
pub const DEFAULT_K: usize = 3;

/// A problem that stops the grammar from being analysed: a syntax error in the dialect, an
/// undefined rule, an unknown flag, or left recursion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrammarError {
    /// The 1-based line, or 0 when the problem is not tied to one line.
    pub line: usize,
    /// What is wrong.
    pub message: String,
}

impl GrammarError {
    fn at(line: usize, message: impl Into<String>) -> Self {
        Self {
            line,
            message: message.into(),
        }
    }
}

impl fmt::Display for GrammarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line == 0 {
            write!(f, "{}", self.message)
        } else {
            write!(f, "{}: {}", self.line, self.message)
        }
    }
}

/// Productions of one decision that the next `k` tokens cannot tell apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    /// The rule (with its flags, as `expr[ns]`), or a helper for a group, `?`, `*` or `+` in it
    /// (as `primary/3?`).
    pub decision: String,
    /// A token sequence the productions share.
    pub tokens: Vec<String>,
    /// The productions involved, as symbol sequences.
    pub alternatives: Vec<String>,
}

impl fmt::Display for Conflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}: on {}", self.decision, self.tokens.join(" "))?;
        for alt in &self.alternatives {
            writeln!(f, "    | {alt}")?;
        }
        Ok(())
    }
}

/// The result of checking a grammar.
#[derive(Debug, Clone, Default)]
pub struct Report {
    /// The lookahead checked.
    pub k: usize,
    /// Syntax rules in the file.
    pub syntax_rules: usize,
    /// Token rules in the file.
    pub token_rules: usize,
    /// Syntax-rule copies after expanding flags.
    pub instances: usize,
    /// Decisions checked: rules and helpers with more than one production.
    pub decisions: usize,
    /// Syntax rules that `file` never reaches, with their lines.
    pub unreachable: Vec<String>,
    /// Decisions that are not LL(k).
    pub conflicts: Vec<Conflict>,
}

impl Report {
    /// Whether the grammar is LL(k): every rule reachable, and no conflicts.
    pub fn is_ll(&self) -> bool {
        self.conflicts.is_empty() && self.unreachable.is_empty()
    }
}

/// Checks `source`, a grammar in the dialect, for lookahead `k` (at least 1).
///
/// # Errors
///
/// Returns every problem that stops the analysis; conflicts are not errors but part of the report.
pub fn check(source: &str, k: usize) -> Result<Report, Vec<GrammarError>> {
    let grammar = parse::parse(source)?;
    let bnf = expand::expand(&grammar)?;
    let (decisions, conflicts) = ll::conflicts(&bnf, k.max(1)).map_err(|e| vec![e])?;
    let token_rules = grammar
        .rules
        .iter()
        .filter(|r| parse::is_token_name(&r.name))
        .count();
    Ok(Report {
        k: k.max(1),
        syntax_rules: grammar.rules.len() - token_rules,
        token_rules,
        instances: bnf.instances,
        decisions,
        unreachable: bnf.unreachable,
        conflicts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conflicts(src: &str, k: usize) -> Vec<String> {
        check(src, k)
            .expect("grammar parses")
            .conflicts
            .iter()
            .map(|c| c.decision.clone())
            .collect()
    }

    #[test]
    fn version_has_three_parts() {
        assert_eq!(VERSION.split('.').count(), 3);
    }

    #[test]
    fn ll1_grammar_has_no_conflicts() {
        let src = "file ::= item*\nitem ::= 'a' IDENT | 'b' ( ',' IDENT )* ';'\nIDENT ::= [a-z]+\n";
        assert!(conflicts(src, 1).is_empty());
    }

    #[test]
    fn shared_first_token_needs_more_lookahead() {
        let src = "file ::= x\nx ::= 'a' 'b' | 'a' 'c'\n";
        assert_eq!(conflicts(src, 1), vec!["x"]);
        assert!(conflicts(src, 2).is_empty());
    }

    #[test]
    fn ambiguity_is_never_resolved() {
        let src = "file ::= x\nx ::= y | z\ny ::= 'a'\nz ::= 'a'\n";
        assert_eq!(conflicts(src, 3), vec!["x"]);
    }

    #[test]
    fn optional_conflicting_with_what_follows() {
        let src = "file ::= 'a'? 'a'\n";
        assert_eq!(conflicts(src, 1), vec!["file/1?"]);
    }

    #[test]
    fn guards_remove_alternatives_under_flags() {
        // Without the flag, `{` after `if` could start the optional head or the block after it.
        let src = "file ::= 'if' e+ns? '{' '}'\ne ::= 'x' | !ns '{' '}'\n";
        assert!(conflicts(src, 1).is_empty());
        let plain = "file ::= 'if' e? '{' '}'\ne ::= 'x' | '{' '}'\n";
        assert_eq!(conflicts(plain, 2), vec!["file/1?"]); // `if { } {` vs `if { }` then the end: k = 3
        assert!(conflicts(plain, 3).is_empty());
    }

    #[test]
    fn flags_are_inherited_only_where_written() {
        let src = "file ::= a+stmt\na ::= b@ ',' b\nb ::= ?stmt 's' | 'x'\n";
        let report = check(src, 1).expect("parses");
        assert!(report.conflicts.is_empty());
        assert_eq!(report.instances, 4); // file, a[stmt], b[stmt], b
    }

    #[test]
    fn keyword_lists_expand_to_terminals() {
        let src = "file ::= name | 'if' IDENT\nname ::= IDENT | KEYWORD\nKEYWORD ::= 'if' | 'fn'\nIDENT ::= [a-z]+\n";
        assert_eq!(conflicts(src, 1), vec!["file"]);
        assert!(conflicts(src, 2).is_empty());
    }

    #[test]
    fn undefined_rules_are_errors() {
        let errors = check("file ::= missing\n", 1).expect_err("undefined");
        assert!(errors[0].message.contains("`missing` is not defined"));
    }

    #[test]
    fn left_recursion_is_an_error() {
        let errors = check("file ::= e\ne ::= e '+' 'x' | 'x'\n", 2).expect_err("left recursive");
        assert!(errors[0].message.contains("left recursion"));
    }

    #[test]
    fn unreachable_rules_fail_the_check() {
        let report = check("file ::= 'a'\nlost ::= 'b'\n", 1).expect("parses");
        assert_eq!(report.unreachable, vec!["lost (line 2)"]);
        assert!(!report.is_ll());
    }

    #[test]
    fn comments_and_split_rule_heads() {
        let src = "# header\nfile   # the start\n       ::= 'a' '#['   # a terminal holding '#'\n         'b'\n";
        assert!(check(src, 1).expect("parses").is_ll());
    }
}
