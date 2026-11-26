//! The parallel runtime's benchmarks (P41): the tests of `benches/runtime/`, each compiled once
//! and run under the deterministic runtime and the parallel one, with what an operation costs (a
//! test's name ends with its count of operations). Not gated: `ci.py bench` prints the table, and
//! a commit changing the runtime gives its numbers.
//!
//! `cargo bench -p moddil-native --bench runtime [filter]`

use std::path::Path;
use std::time::Instant;

use moddil_native::{Options, run_test_runs};
use moddil_sema::{Database, Program};

/// How many runs each is timed over.
const RUNS: usize = 3;

fn main() {
    let filter = std::env::args().skip(1).find(|a| !a.starts_with('-'));
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("benches/runtime");
    let project = moddil_project::load(&dir).unwrap_or_else(|p| panic!("the benchmarks: {p:?}"));
    let db = Database::default();
    let program = Program::of_project(&db, &project);
    let items = moddil_sema::item_tree(&db, program);
    println!(
        "{:<52} {:>12} {:>12}",
        "benchmark", "sim (ns/op)", "parallel"
    );
    for (i, t) in items.tests.iter().enumerate() {
        if items.mod_of(t.module) != 0 || filter.as_ref().is_some_and(|f| !t.name.contains(f)) {
            continue;
        }
        let ops: f64 = t
            .name
            .rsplit(", ")
            .next()
            .and_then(|n| n.parse().ok())
            .unwrap_or(1.0);
        let timed = |options: &[Options]| {
            let start = Instant::now();
            let runs = run_test_runs(&db, program, i as u32, options);
            for (_, r) in &runs {
                assert!(r.is_ok(), "{}: {r:?}", t.name);
            }
            start.elapsed()
        };
        // Warmed up (what compiling it first works out is kept), compiled and run once, then
        // compiled and run once more `RUNS` times: what the more runs took, each.
        let per_op = |options: Options| {
            timed(&[options]);
            let one = timed(&[options]);
            let more = timed(&[options; RUNS + 1]);
            let each = more.saturating_sub(one).as_secs_f64() / RUNS as f64;
            each * 1e9 / ops
        };
        let sim = per_op(Options::default());
        let parallel = per_op(Options {
            parallel: true,
            ..Options::default()
        });
        println!("{:<52} {sim:>12.1} {parallel:>12.1}", t.name);
    }
}
