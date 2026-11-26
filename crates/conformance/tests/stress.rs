//! The stress suite of the runtimes' logic (P41): random trees of tasks (scopes, spawns, joins,
//! channels, `yield_now`, `Rc`s shared across tasks, and isolated scopes a task traps in) whose
//! value does not depend on the order the tasks run in. Each is run under many seeds of the
//! deterministic runtime (`task.runtime.1`) on the MIR interpreter and natively: every seed must
//! give the value the tree was made to give, with no trap and, natively, no leak.
//!
//! `MODDIL_STRESS_TREES` and `MODDIL_STRESS_SEEDS` set how many trees and seeds (natively, a
//! tenth as many seeds). A tree's interpreted runs go on a thread a core; its native runs share
//! one compile (`moddil_native::run_test_runs`), one after another, as the runtime's state is
//! the process's.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use moddil_project::{File, Mod, Project};
use moddil_sema::{Database, Program};

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

/// Runs test 0 of `program` on the MIR interpreter under seeds 0 to `seeds`, on a thread a
/// core, each with its own handle to `db` (sharing what is already worked out): the seeds whose
/// runs did not end well, and how they ended.
fn interpreted(db: &Database, program: Program, seeds: u64) -> Vec<(u64, String)> {
    let next = AtomicU64::new(0);
    let next = &next;
    let threads = std::thread::available_parallelism().map_or(1, usize::from);
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                let db = db.clone();
                std::thread::Builder::new()
                    // The interpreter needs more stack than a spawned thread has by default.
                    .stack_size(64 << 20)
                    .spawn_scoped(scope, move || {
                        let mut bad = Vec::new();
                        loop {
                            let seed = next.fetch_add(1, Ordering::Relaxed);
                            if seed >= seeds {
                                return bad;
                            }
                            let r = moddil_interp::run_test_seeded(
                                &db,
                                program,
                                0,
                                seed,
                                &mut |_, _, _| {},
                            );
                            if let Err(e) = r {
                                bad.push((seed, format!("{e:?}")));
                            }
                        }
                    })
                    .expect("a thread")
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("an interpreted run does not panic"))
            .collect()
    })
}

/// Test 0 of `program` natively under each of `runs`, compiled once: the runs that did not end
/// well, or (with `want`) observed other than it, and what they did.
fn native(
    db: &Database,
    program: Program,
    runs: &[moddil_native::Options],
    want: Option<i64>,
) -> Vec<(usize, String)> {
    moddil_native::run_test_runs(db, program, 0, runs)
        .into_iter()
        .enumerate()
        .filter_map(|(k, (observed, r))| {
            let got: Vec<String> = observed.into_iter().map(|(_, _, v)| v).collect();
            match (r, want) {
                (Err(e), _) => Some((k, format!("{e:?}"))),
                (Ok(()), Some(w)) if got != vec![w.to_string()] => {
                    Some((k, format!("observed {got:?}")))
                }
                _ => None,
            }
        })
        .collect()
}

/// Options for the deterministic runtime under each of `seeds`.
fn seeded(seeds: impl Iterator<Item = u64>) -> Vec<moddil_native::Options> {
    seeds
        .map(|seed| moddil_native::Options {
            seed,
            ..Default::default()
        })
        .collect()
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
        let bad = interpreted(&db, program, seeds);
        assert!(
            bad.is_empty(),
            "tree {i}, interpreted (seed, how): {bad:?}\n{text}"
        );
        let bad = native(&db, program, &seeded(0..seeds.div_ceil(10)), None);
        assert!(
            bad.is_empty(),
            "tree {i}, native (seed, how): {bad:?}\n{text}"
        );
        // The value: a `// =>` on the test's last line, checked by both.
        let checked = text.replace("    v\n}", &format!("    v  // => {want}\n}}"));
        let db = moddil_interp::database();
        let program = Program::of_project(&db, &project(&checked));
        let checks = [0, 1, seeds / 2, seeds.saturating_sub(1)];
        for seed in checks {
            let mut got = Vec::new();
            let r = moddil_interp::run_test_seeded(&db, program, 0, seed, &mut |_, _, v| {
                got.push(v.render(moddil_sema::item_tree(&db, program)));
            });
            assert!(r.is_ok(), "tree {i}, seed {seed}: {r:?}");
            assert_eq!(got, vec![want.to_string()], "tree {i}, seed {seed}\n{text}");
        }
        let bad = native(&db, program, &seeded(checks.into_iter()), Some(want));
        assert!(
            bad.is_empty(),
            "tree {i}, native, of seeds {checks:?} (run, how): {bad:?}\n{text}"
        );
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
        let options = moddil_native::Options {
            parallel: true,
            ..Default::default()
        };
        let bad = native(&db, program, &vec![options; runs as usize], Some(want));
        assert!(
            bad.is_empty(),
            "tree {i}, parallel (run, how): {bad:?}\n{text}"
        );
    }
}

/// More tasks spinning than the machine has cores, and one more that ends them: under the
/// parallel runtime the spinners give way at their loops' back-edges once their slices are
/// over (`runtime.md`: a long CPU loop never yields), so the last one runs. Slices are 10 ms,
/// so a run takes a few of them: one that takes seconds gave way by luck, not by preemption.
#[test]
fn spinning_tasks_give_way_in_parallel() {
    let cores = std::thread::available_parallelism().map_or(4, usize::from);
    let spinners: String = (0..cores * 2)
        .map(|_| "        s.spawn(|| while atomic::load(stop.get(), Ordering::Acquire) == 0 {});\n")
        .collect();
    let text = format!(
        "use core::atomic::{{self, Ordering}};\nuse core::cell::UnsafeCell;\n\n\
         test \"spinners give way\" {{\n    let stop = UnsafeCell::new(0);\n    scope(|s| {{\n\
         {spinners}        s.spawn(|| atomic::store(stop.get(), 1, Ordering::Release));\n    \
         }});\n    1  // => 1\n}}\n"
    );
    let mut p = project(&text);
    p.mods[0].unsafe_ok = true;
    let db = moddil_interp::database();
    let program = Program::of_project(&db, &p);
    let diagnostics = moddil_interp::diagnostics(&db, program);
    assert!(diagnostics.is_empty(), "{diagnostics:?}\n{text}");
    let options = moddil_native::Options {
        parallel: true,
        ..Default::default()
    };
    let start = std::time::Instant::now();
    let runs = moddil_native::run_test_runs(&db, program, 0, &[options; 5]);
    let took = start.elapsed();
    for (k, (observed, r)) in runs.into_iter().enumerate() {
        assert!(r.is_ok(), "run {k}: {r:?}");
        assert_eq!(observed.len(), 1, "run {k}");
    }
    // Compiling and five runs: about a second. Minutes would mean the spinners gave way only
    // by chance.
    assert!(
        took < std::time::Duration::from_secs(60),
        "the spinners took {took:?} to give way five times"
    );
}
