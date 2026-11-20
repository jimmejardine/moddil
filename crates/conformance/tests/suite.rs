//! Runs the conformance suite (`conformance/2027/`) on the MIR interpreter and the reference
//! interpreter, and checks that every paragraph of the reference (`reference/2027/`) is cited by
//! a test and every cited paragraph exists.

use std::path::Path;

use moddil_conformance::{
    EDITION, Implementation, MirInterpreter, ReferenceInterpreter, run_suite,
};

#[test]
fn the_suite_passes_and_covers_the_reference() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let implementations: [&dyn Implementation; 2] = [&MirInterpreter, &ReferenceInterpreter];
    let report = run_suite(&repo, EDITION, &implementations);
    println!(
        "{} files, {} reference paragraphs, {} disputed",
        report.files,
        report.paragraphs,
        report.disputed.len()
    );
    assert!(report.files > 0, "no conformance tests found");
    assert!(report.paragraphs > 0, "no reference paragraphs found");
    let mut problems = report.failures.clone();
    problems.extend(
        report
            .uncovered
            .iter()
            .map(|id| format!("reference paragraph {id} is not cited by any test")),
    );
    problems.extend(
        report
            .unknown
            .iter()
            .map(|id| format!("tests cite {id}, which is not in the reference")),
    );
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
}
