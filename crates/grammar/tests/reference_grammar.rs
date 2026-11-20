//! The checked-in grammar must be LL(3): this test is the CI gate for `reference/grammar/moddil.ebnf`.

#[test]
fn reference_grammar_is_ll3() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../reference/grammar/moddil.ebnf"
    );
    let source = std::fs::read_to_string(path).expect("the grammar file is readable");
    let report =
        moddil_grammar::check(&source, moddil_grammar::DEFAULT_K).unwrap_or_else(|errors| {
            let list: Vec<String> = errors.iter().map(ToString::to_string).collect();
            panic!("the grammar does not parse:\n{}", list.join("\n"))
        });
    let conflicts: String = report.conflicts.iter().map(ToString::to_string).collect();
    assert!(
        report.is_ll(),
        "not LL({}): unreachable {:?}\n{conflicts}",
        report.k,
        report.unreachable
    );
}

#[test]
fn reference_grammar_needs_more_than_one_token() {
    // Guards against a broken checker that accepts everything: the grammar is known not to be LL(1).
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../reference/grammar/moddil.ebnf"
    );
    let source = std::fs::read_to_string(path).expect("the grammar file is readable");
    let report = moddil_grammar::check(&source, 1).expect("the grammar parses");
    assert!(!report.conflicts.is_empty());
}
