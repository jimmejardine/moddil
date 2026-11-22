//! The parser against the grammar.
//!
//! - The checked-in corpus (`reference/grammar/tests/`): every valid case and sample parses
//!   without errors, and every invalid case has at least one.
//! - Differential: thousands of random token edits of the valid corpus, each judged by the
//!   grammar's own LL(3) recogniser (`moddil-grammar`); the parser must agree on every one.
//! - Every tree is lossless, including for every prefix of the samples (truncated input).

use std::path::{Path, PathBuf};

fn grammar_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../reference/grammar")
}

/// The cases of a corpus file: each whole file after a `// ---` line.
fn cases(file: &str) -> Vec<String> {
    let path = grammar_dir().join("tests").join(file);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .replace("\r\n", "\n");
    let mut cases: Vec<Vec<&str>> = Vec::new();
    for line in text.lines() {
        if line == "// ---" {
            cases.push(Vec::new());
        } else if let Some(case) = cases.last_mut() {
            case.push(line);
        }
    }
    cases.into_iter().map(|lines| lines.join("\n")).collect()
}

fn lossless(text: &str) -> moddil_syntax::Parse {
    let parse = moddil_syntax::parse(text);
    assert_eq!(
        parse.syntax().text(),
        text,
        "the tree must hold the whole text"
    );
    parse
}

fn show(failures: &[String], total: usize, what: &str) -> String {
    format!(
        "{} of {total} {what}:\n\n{}",
        failures.len(),
        failures
            .iter()
            .take(12)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n---\n")
    )
}

#[test]
fn valid_cases_parse_without_errors() {
    let cases: Vec<String> = cases("valid.moddil")
        .into_iter()
        .chain(cases("samples.moddil"))
        .collect();
    assert_eq!(cases.len(), 775);
    let mut failures = Vec::new();
    for (i, case) in cases.iter().enumerate() {
        if let Some(e) = lossless(case).errors().first() {
            failures.push(format!("case {i}: {e}\n{case}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{}",
        show(&failures, cases.len(), "valid cases fail")
    );
}

#[test]
fn invalid_cases_report_errors() {
    let cases = cases("invalid.moddil");
    assert_eq!(cases.len(), 300);
    let accepted: Vec<String> = cases
        .iter()
        .enumerate()
        .filter(|(_, case)| lossless(case).errors().is_empty())
        .map(|(i, case)| format!("case {i}:\n{case}"))
        .collect();
    assert!(
        accepted.is_empty(),
        "{}",
        show(&accepted, cases.len(), "invalid cases are accepted")
    );
}

/// The random edits are split into this many tests, so the test runner checks them in parallel.
/// Each pays the grammar's set-up once, so more shards stop paying off quickly.
const EDIT_SHARDS: u64 = 4;

/// One shard of the differential test: its share of the edits, from its own seed.
fn the_parser_agrees_with_the_grammar_on_random_edits(shard: u64) {
    let source = std::fs::read_to_string(grammar_dir().join("moddil.ebnf")).expect("the grammar");
    // 1000 in all by default; CI's grammars step runs many more in release (`MODDIL_EDITS`, the
    // total over the shards), and `MODDIL_SEED` explores other edits.
    let var = |name: &str| std::env::var(name).ok().and_then(|n| n.parse::<u64>().ok());
    let total = var("MODDIL_EDITS").map_or(1000, |n| n);
    let count = usize::try_from(total.div_ceil(EDIT_SHARDS)).expect("a count");
    let seed = var("MODDIL_SEED").unwrap_or(0x0005_EED0_F6A3_3A12)
        ^ shard.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let cases = moddil_grammar::edited_cases(&source, count, seed).expect("the grammar is valid");
    let mut disagreements = Vec::new();
    let accepted = cases.iter().filter(|c| c.1).count();
    // Both verdicts must be well represented, or the test proves little.
    assert!(
        accepted * 10 > cases.len() && accepted * 10 < cases.len() * 9,
        "{accepted} of {} edits are valid",
        cases.len()
    );
    for (text, grammar_accepts) in &cases {
        let parse = lossless(text);
        let parser_accepts = parse.errors().is_empty();
        if parser_accepts != *grammar_accepts {
            let detail = match parse.errors().first() {
                Some(e) => format!("the parser reports {e}"),
                None => "the parser accepts it".to_string(),
            };
            let verdict = if *grammar_accepts {
                "accepts"
            } else {
                "rejects"
            };
            disagreements.push(format!("the grammar {verdict}, {detail}:\n{text}"));
        }
    }
    assert!(
        disagreements.is_empty(),
        "({accepted} of the edits are valid) {}",
        show(&disagreements, cases.len(), "edits get a different verdict")
    );
}

macro_rules! edit_shards {
    ($($name:ident = $shard:literal),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                the_parser_agrees_with_the_grammar_on_random_edits($shard);
            }
        )*
        #[test]
        fn every_edit_shard_has_a_test() {
            assert_eq!(u64::try_from([$($shard),*].len()).ok(), Some(EDIT_SHARDS));
        }
    };
}

edit_shards!(
    random_edits_0 = 0,
    random_edits_1 = 1,
    random_edits_2 = 2,
    random_edits_3 = 3
);

#[test]
fn every_prefix_of_the_samples_parses_losslessly() {
    for case in cases("samples.moddil") {
        let mut cut = 0;
        while cut < case.len() {
            lossless(&case[..cut]);
            cut += 5;
            while !case.is_char_boundary(cut.min(case.len())) {
                cut += 1;
            }
        }
    }
}

#[test]
fn random_text_never_panics_and_stays_lossless() {
    const PIECES: &[&str] = &[
        "fn", "let", "if", "else", "match", "{", "}", "(", ")", "[", "]", "<", ">", ">>", "&&",
        "&", "|", "||", "=", "==", "=>", "->", "::", ":", ";", ",", ".", "..", "..=", "?", "?!",
        "#[", "#![", "x", "_", "1", "1.5", "'a", "'c'", "\"s\"", "\"", "r#\"", "\"#", "fmt\"{",
        "}\"", "/*", "*/", "//", "\n", " ", "\\", "@", "%", "+%", "é", "\u{202E}", "struct",
        "impl", "with", "unsafe", "gen", "const", "for", "in", "'", "b'", "\"\"\"\n", "\n\"\"\"",
    ];
    let mut state: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = |bound: usize| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        usize::try_from(state % bound as u64).expect("fits")
    };
    for _ in 0..3000 {
        let len = next(40);
        let text: String = (0..len).map(|_| PIECES[next(PIECES.len())]).collect();
        lossless(&text);
    }
}
