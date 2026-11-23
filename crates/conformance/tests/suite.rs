//! Runs the conformance suite (`conformance/2027/`) on the MIR interpreter and the reference
//! interpreter, and checks that every paragraph of the reference (`reference/2027/`) is cited by
//! a test and every cited paragraph exists.
//!
//! Each area (a directory of the suite) is its own test, so the test runner spreads the suite
//! over every core; `every_area_has_a_test` catches an area missing from the list.

use std::path::{Path, PathBuf};

use moddil_conformance::{
    EDITION, Implementation, MirInterpreter, Native, ReferenceInterpreter, Wasm, areas, coverage,
    run_area,
};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn run(area: &str) {
    let implementations: [&dyn Implementation; 4] =
        [&MirInterpreter, &ReferenceInterpreter, &Native, &Wasm];
    let report = run_area(&repo(), EDITION, area, &implementations);
    println!(
        "{area}: {} files, {} disputed, {} run natively, {} as WASM",
        report.files,
        report.disputed.len(),
        report.ran.get("native").copied().unwrap_or(0),
        report.ran.get("wasm").copied().unwrap_or(0)
    );
    assert!(report.files > 0, "no conformance tests in {area}");
    assert!(
        report.failures.is_empty(),
        "\n{}",
        report.failures.join("\n")
    );
}

/// One test per area, named like its directory.
macro_rules! areas {
    ($($area:ident),* $(,)?) => {
        const AREAS: &[&str] = &[$(stringify!($area)),*];
        $(
            #[test]
            fn $area() {
                run(stringify!($area).trim_start_matches("r#"));
            }
        )*
    };
}

areas!(
    arithmetic,
    arrays,
    assert,
    assignment,
    attributes,
    bitwise,
    blocks,
    borrows,
    boxes,
    calls,
    closures,
    collections,
    comparison,
    comptime,
    concurrency,
    contracts,
    derive,
    effects,
    enums,
    errors,
    generics,
    r#if,
    items,
    iteration,
    literals,
    logic,
    loops,
    main,
    r#match,
    modules,
    names,
    numbers,
    operators,
    ownership,
    patterns,
    prelude,
    refinements,
    slices,
    statements,
    structs,
    syntax,
    text,
    traits,
    tuples,
    types,
    views,
    wrappers,
);

#[test]
fn every_area_has_a_test() {
    let listed: Vec<String> = AREAS
        .iter()
        .map(|a| a.trim_start_matches("r#").to_string())
        .collect();
    let found = areas(&repo(), EDITION);
    let missing: Vec<&String> = found.iter().filter(|a| !listed.contains(a)).collect();
    let gone: Vec<&String> = listed.iter().filter(|a| !found.contains(a)).collect();
    assert!(
        missing.is_empty() && gone.is_empty(),
        "add {missing:?} to, and remove {gone:?} from, the `areas!` list in suite.rs"
    );
}

#[test]
fn every_reference_paragraph_is_cited() {
    let report = coverage(&repo(), EDITION);
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
