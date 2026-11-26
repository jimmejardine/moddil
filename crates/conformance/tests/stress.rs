//! The stress suite of the runtimes' logic (P41): random trees of tasks (scopes, spawns, joins,
//! channels, `yield_now`, `Rc`s shared across tasks, and isolated scopes a task traps in) whose
//! value does not depend on the order the tasks run in. Each is run under many seeds of the
//! deterministic runtime (`task.runtime.1`) on the MIR interpreter and natively: every seed must
//! give the value the tree was made to give, with no trap and, natively, no leak.
//!
//! `MODDIL_STRESS_TREES` and `MODDIL_STRESS_SEEDS` set how many trees and seeds (natively, a
//! tenth as many seeds, as each run compiles).

use std::path::PathBuf;

use moddil_project::{File, Mod, Project};
use moddil_sema::Program;

/// SplitMix64, to make the trees: the same trees every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// A block giving an `int`, and the value it gives.
struct Block {
    code: String,
    value: i64,
}

/// A scope of tasks: each sends what it worked out to the scope's channel, or is joined; the
/// block sums what came back. `depth` limits how deep scopes nest.
fn scope(rng: &mut Rng, depth: u32, at: &mut u32) -> Block {
    *at += 1;
    let me = *at;
    let n = 1 + rng.below(4) as usize;
    let mut code = format!(
        "{{\n    let (tx{me}, rx{me}) = channel::<int>({n});\n    let mut sum{me} = 0;\n    \
         scope(|s| {{\n"
    );
    let mut value = 0;
    let mut sent = 0;
    for k in 0..n {
        let task = task(rng, depth, at);
        value += task.value;
        if rng.below(3) == 0 {
            // Joined: its value comes back through its `Task`.
            code.push_str(&format!(
                "        let t{me}_{k} = s.spawn(|| {});\n        sum{me} += t{me}_{k}.join();\n",
                task.code
            ));
        } else {
            sent += 1;
            code.push_str(&format!(
                "        s.spawn(|| {{\n            let v = {};\n            \
                 tx{me}.send(v) ?! \"the receiver outlives the scope\";\n        }});\n",
                task.code
            ));
        }
    }
    code.push_str(&format!(
        "    }});\n    for _ in 0..{sent} {{\n        sum{me} += rx{me}.recv() ?! \"each task sent\";\n    \
         }}\n    sum{me}\n}}"
    ));
    Block { code, value }
}

/// A task's body: a few steps, each giving a value or waiting.
fn task(rng: &mut Rng, depth: u32, at: &mut u32) -> Block {
    let mut code = String::new();
    let mut value = 0;
    for _ in 0..1 + rng.below(3) {
        match rng.below(6) {
            0 => code.push_str("    yield_now();\n"),
            1 => {
                // An `Rc` the test made, shared in this task, kept across a wait, dropped here.
                code.push_str("    let r = shared.share();\n    yield_now();\n    acc += *r;\n");
                value += 3;
            }
            2 if depth > 0 => {
                let inner = scope(rng, depth - 1, at);
                code.push_str(&format!("    acc += {};\n", inner.code));
                value += inner.value;
            }
            3 if depth > 0 => {
                // An isolated scope one of whose tasks traps: it fails, the others cancelled.
                code.push_str(
                    "    let failed = try {\n        scope(|s| {\n            s.spawn(|| {\n                \
                     yield_now();\n                let zero = 0;\n                10 / zero\n            \
                     });\n            s.spawn(|| yield_now());\n            0\n        }, isolate: \
                     true)?\n    };\n    if failed.is_err() {\n        acc += 1;\n    }\n",
                );
                value += 1;
            }
            _ => {
                let k = 1 + rng.below(9) as i64;
                code.push_str(&format!("    acc += {k};\n"));
                value += k;
            }
        }
    }
    let code = format!("{{\n    let mut acc = 0;\n{code}    acc\n}}");
    Block { code, value }
}

/// Tree `i`: a test of the `stress` file, and the value it gives.
fn tree(i: u64) -> (String, i64) {
    let mut rng = Rng(i.wrapping_mul(0x2545_f491_4f6c_dd1d) | 1);
    let mut at = 0;
    let b = scope(&mut rng, 2, &mut at);
    let text = format!(
        "test \"tree {i}\" {{\n    let shared = 3.to_rc();\n    let v = {};\n    v\n}}\n",
        b.code
    );
    (text, b.value)
}

fn project(text: &str) -> Project {
    Project {
        files: vec![File {
            path: PathBuf::from("stress.moddil"),
            display: "stress.moddil".to_string(),
            text: text.to_string(),
            mod_: 0,
            module: Vec::new(),
            exe: false,
        }],
        mods: vec![Mod {
            name: String::new(),
            deps: Vec::new(),
            manifest: None,
            stub: false,
            unsafe_ok: false,
            os_ok: false,
            c: None,
            version: None,
            runtime: None,
            threads: None,
        }],
    }
}

fn count(var: &str, default: u64) -> u64 {
    std::env::var(var)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[test]
fn every_order_of_a_task_tree_gives_its_value() {
    let trees = count("MODDIL_STRESS_TREES", 12);
    let seeds = count("MODDIL_STRESS_SEEDS", 300);
    for i in 0..trees {
        let (text, want) = tree(i);
        let db = moddil_interp::database();
        let program = Program::of_project(&db, &project(&text));
        let diagnostics = moddil_interp::diagnostics(&db, program);
        assert!(diagnostics.is_empty(), "tree {i}: {diagnostics:?}\n{text}");
        for seed in 0..seeds {
            let r = moddil_interp::run_test_seeded(&db, program, 0, seed, &mut |_, _, _| {});
            assert!(
                r.is_ok(),
                "tree {i}, seed {seed}, interpreted: {r:?}\n{text}"
            );
        }
        for seed in 0..seeds.div_ceil(10) {
            let options = moddil_native::Options {
                seed,
                ..Default::default()
            };
            let (_, r) = moddil_native::run_test_with(&db, program, 0, options);
            assert!(r.is_ok(), "tree {i}, seed {seed}, native: {r:?}\n{text}");
        }
        // The value: a `// =>` on the test's last line, checked by both.
        let checked = text.replace("    v\n}", &format!("    v  // => {want}\n}}"));
        let db = moddil_interp::database();
        let program = Program::of_project(&db, &project(&checked));
        for seed in [0, 1, seeds / 2, seeds.saturating_sub(1)] {
            let mut got = Vec::new();
            let r = moddil_interp::run_test_seeded(&db, program, 0, seed, &mut |_, _, v| {
                got.push(v.render(moddil_sema::item_tree(&db, program)));
            });
            assert!(r.is_ok(), "tree {i}, seed {seed}: {r:?}");
            assert_eq!(got, vec![want.to_string()], "tree {i}, seed {seed}\n{text}");
            let options = moddil_native::Options {
                seed,
                ..Default::default()
            };
            let (observed, r) = moddil_native::run_test_with(&db, program, 0, options);
            assert!(r.is_ok(), "tree {i}, seed {seed}, native: {r:?}");
            let got: Vec<String> = observed.into_iter().map(|(_, _, v)| v).collect();
            assert_eq!(
                got,
                vec![want.to_string()],
                "tree {i}, seed {seed}, native\n{text}"
            );
        }
    }
}

/// The same trees under the parallel runtime (`task.runtime.1`), natively: their tasks run at
/// once, in whatever order the machine gives, and each run must still give the tree's value.
/// (The process then counts `Rc`s atomically for every run, the other test's too: correct, if
/// slower.)
#[test]
fn a_task_tree_gives_its_value_in_parallel() {
    let trees = count("MODDIL_STRESS_TREES", 12);
    let runs = count("MODDIL_STRESS_SEEDS", 300).div_ceil(15);
    for i in 0..trees {
        let (text, want) = tree(i);
        let checked = text.replace("    v\n}", &format!("    v  // => {want}\n}}"));
        let db = moddil_interp::database();
        let program = Program::of_project(&db, &project(&checked));
        for run in 0..runs {
            let options = moddil_native::Options {
                parallel: true,
                ..Default::default()
            };
            let (observed, r) = moddil_native::run_test_with(&db, program, 0, options);
            assert!(r.is_ok(), "tree {i}, run {run}, parallel: {r:?}\n{text}");
            let got: Vec<String> = observed.into_iter().map(|(_, _, v)| v).collect();
            assert_eq!(
                got,
                vec![want.to_string()],
                "tree {i}, run {run}, parallel\n{text}"
            );
        }
    }
}
