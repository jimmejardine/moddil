//! The MIR interpreter: runs analysed or runtime MIR for compile-time evaluation, tests before
//! any backend, the Miri-style checker and the playground.
//!
//! It runs calls on its own stack of frames, not the host's, so deep recursion in a Moddil
//! program is a [`TrapClass::StackOverflow`] trap and never crashes the interpreter. A borrow is
//! a pointer to a frame's local and a path of fields in it; borrows cannot outlive what they
//! point to (the borrow checker sees to that), so the frame is always there. See this crate's
//! `README.md`.
//!
//! Generic code runs once for every type: each frame carries its type arguments, and a call of a
//! trait's method is resolved from them, through a **witness table** built the first time a type
//! is asked for that trait (`types.md`: Generics). Moving a value out of a place leaves the place
//! empty, which is the drop flag: dropping an empty place does nothing.

mod calls;
mod cmem;
mod dispatch;
mod frozen;
mod machine;
mod natives;
mod num;
mod ops;
mod sched;
mod stack;
mod tags;
mod texts;

pub use frozen::{Frozen, FrozenValue};
pub use tags::{TagEvaluator, database};
mod value;

use moddil_mir::TrapClass;
use moddil_sema::{Code, Db, Diagnostic, Item, Program};
use moddil_syntax::TextRange;

pub use value::{Pointer, Value};

/// A trap: what kind, and the source of the check that failed (or of the call, for a stack
/// overflow).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trap {
    /// What kind of trap.
    pub class: TrapClass,
    /// The file it is in, by its index in the program.
    pub file: u32,
    /// Where.
    pub range: TextRange,
    /// For a failed `assert`: each part of the condition and its value, or `None` if it was not
    /// evaluated.
    pub parts: Vec<(TextRange, Option<Value>)>,
    /// For a failed `?!`: its reason; for `compile_error`, its message.
    pub reason: Option<String>,
    /// For `compile_error`: the bytes it names, of the text of the tagged literal being
    /// evaluated (`comptime.error.1`).
    pub at: Option<(i64, i64)>,
}

/// Why running a function or test stopped without a result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stop {
    /// A trap.
    Trap(Trap),
    /// An error escaped it (a test's, or a function asked to run that can fail): where it left
    /// the function or test, and the error.
    Failed {
        /// The file the error left from.
        file: u32,
        /// Where the error left.
        range: TextRange,
        /// The error.
        error: Value,
    },
}

/// How deeply calls may nest before a [`TrapClass::StackOverflow`] trap.
pub const MAX_DEPTH: usize = 10_000;

/// Reports the values of observed expressions (those followed by `// =>`): their file, range and
/// value.
pub type Observer<'a> = &'a mut dyn FnMut(u32, TextRange, Value);

/// Calls function `index` of `program` with `args`.
///
/// # Panics
///
/// If the file has errors (check `moddil_mir::has_errors` first), or `args` does not match the
/// function's parameters.
///
/// # Errors
///
/// The trap, if the program traps, or the error, if one escapes.
pub fn call_fn(
    db: &dyn Db,
    program: Program,
    index: u32,
    args: &[Value],
    observer: Observer<'_>,
) -> Result<Value, Stop> {
    machine::run(db, program, Item::Fn(index), args, observer, None, None)
}

/// Runs `main`, function `index` of `program`, with its root capabilities `args`, as a program
/// runs (`os.1`): with the OS, given the program's arguments `argv`, and the wall clock's time.
///
/// # Errors
///
/// The trap, if it traps.
pub fn run_main(
    db: &dyn Db,
    program: Program,
    index: u32,
    args: &[Value],
    argv: Vec<String>,
) -> Result<Value, Stop> {
    machine::run(
        db,
        program,
        Item::Fn(index),
        args,
        &mut |_, _, _| {},
        None,
        Some(argv),
    )
}

/// The steps compile-time evaluation may take (`comptime.md`: Evaluation model).
pub const FUEL: u64 = 10_000_000;

/// Evaluates const item `index` of `program` (a function with no parameters), as compiling does:
/// with [`FUEL`] steps at most.
///
/// # Errors
///
/// The trap, if it traps or runs out of fuel.
pub fn eval_const(db: &dyn Db, program: Program, index: u32) -> Result<Value, Stop> {
    machine::run(
        db,
        program,
        Item::Fn(index),
        &[],
        &mut |_, _, _| {},
        Some(fuel_of(db, program, Item::Fn(index))),
        None,
    )
}

/// Evaluates const item `index` of `program` as compiling does (`comptime.static.1`), with
/// [`FUEL`] steps at most, and freezes its value: what it is, and what its pointers point to.
///
/// # Errors
///
/// The trap, if it traps or runs out of fuel; or, inside, what in the value cannot be static
/// data.
pub fn frozen(
    db: &dyn Db,
    program: Program,
    index: u32,
) -> Result<Result<FrozenValue, String>, Stop> {
    frozen_item(db, program, Item::Fn(index))
}

/// As [`frozen`], for a const item or a `const { }` block's item (`comptime.block.2`).
///
/// # Errors
///
/// As [`frozen`]'s.
pub fn frozen_item(
    db: &dyn Db,
    program: Program,
    item: Item,
) -> Result<Result<FrozenValue, String>, Stop> {
    machine::run_frozen(db, program, item, Some(fuel_of(db, program, item)))
}

/// The steps evaluating `item` when compiling may take: [`FUEL`], or what a `#[fuel(n)]` on
/// the const or the block's statement gives (`comptime.eval.2`).
pub fn fuel_of(db: &dyn Db, program: Program, item: Item) -> u64 {
    let given = match item {
        Item::Fn(_) => {
            let b = &moddil_sema::body(db, moddil_sema::Owner::new(db, program, item)).body;
            b.root.and_then(|r| b.fuel.get(&r).copied())
        }
        Item::Closure(_, _, e) => {
            let base = moddil_sema::Owner::new(db, program, item.owner());
            let b = &moddil_sema::body(db, base).body;
            let e = moddil_sema::hir::ExprId::from_raw(la_arena::RawIdx::from_u32(e));
            b.fuel.get(&e).copied()
        }
        Item::Test(_) => None,
    };
    given.unwrap_or(FUEL)
}

/// Every diagnostic for `program`: the front end's and the ownership check's, then, if it has
/// no errors, those of evaluating its const items when compiling (E1004), and of a value that
/// cannot be static data (E1008).
pub fn diagnostics(db: &dyn Db, program: Program) -> Vec<Diagnostic> {
    let mut out = moddil_mir::diagnostics(db, program).clone();
    if moddil_mir::has_errors(db, program) {
        return out;
    }
    // A kernel that cannot run on a GPU (`gpu.kernel.2`).
    out.extend(moddil_gpu::check(db, program));
    let items = moddil_sema::item_tree(db, program);
    for (i, f) in items.fns.iter().enumerate() {
        if !f.const_item || !f.src.is_file() {
            continue;
        }
        let result = if items.is_frozen_const(i as u32) {
            frozen(db, program, i as u32).map(Some)
        } else {
            eval_const(db, program, i as u32).map(|_| None)
        };
        let why = match result {
            // `compile_error` (`comptime.error.1`), at the const.
            Err(Stop::Trap(t)) if t.at.is_some() => {
                let d = Diagnostic::error(Code::E1010, f.name_range, t.reason.unwrap_or_default());
                out.extend(moddil_sema::in_file(items, Item::Fn(i as u32), &[d]));
                continue;
            }
            Ok(Some(Err(what))) => {
                let d = Diagnostic::error(
                    Code::E1008,
                    f.name_range,
                    format!("`{}` holds {what}, which cannot be static data", f.name),
                );
                out.extend(moddil_sema::in_file(items, Item::Fn(i as u32), &[d]));
                continue;
            }
            Ok(_) => continue,
            Err(Stop::Trap(t)) if t.class == TrapClass::OutOfFuel => format!(
                "evaluating `{}` when compiling takes more than {} steps (`#[fuel(n)]` allows                  more)",
                f.name,
                fuel_of(db, program, Item::Fn(i as u32))
            ),
            Err(Stop::Trap(t)) => format!(
                "evaluating `{}` when compiling traps: {}",
                f.name,
                t.class.name()
            ),
            Err(Stop::Failed { .. }) => format!("evaluating `{}` when compiling fails", f.name),
        };
        let d = Diagnostic::error(Code::E1004, f.name_range, why);
        out.extend(moddil_sema::in_file(items, Item::Fn(i as u32), &[d]));
    }
    // Each `const { }` block computed when compiling (`comptime.block.2`): as a const's.
    for owner in moddil_sema::owners(db, program) {
        let base = owner.item(db);
        let body = &moddil_sema::body(db, owner).body;
        if !items.freezes_blocks(base) && body.wrapper_args.is_empty() {
            continue;
        }
        let types = moddil_sema::infer(db, owner);
        for (e, x) in body.exprs.iter() {
            let moddil_sema::hir::ExprKind::ConstBlock(_) = x.kind else {
                continue;
            };
            if !items.freezes_block(base, body, e, &types.expr(e)) {
                continue;
            }
            let index = e.into_raw().into_u32();
            let item = match base {
                Item::Fn(i) => Item::Closure(i, false, index),
                Item::Test(i) => Item::Closure(i, true, index),
                Item::Closure(..) => continue,
            };
            let d = match frozen_item(db, program, item) {
                Ok(Ok(_)) => continue,
                // `compile_error` (`comptime.error.1`): where it says, in a tagged literal's text.
                Err(Stop::Trap(t)) if t.at.is_some() => {
                    let (s, end) = t.at.unwrap_or_default();
                    let at = match body.tagged.get(&e) {
                        Some(text) => TextRange::new(
                            text.file_offset(s),
                            text.file_offset(end.max(s)).max(text.file_offset(s)),
                        ),
                        None => x.range,
                    };
                    Diagnostic::error(Code::E1010, at, t.reason.unwrap_or_default())
                }
                Ok(Err(what)) => Diagnostic::error(
                    Code::E1008,
                    x.range,
                    format!("this block's value holds {what}, which cannot be static data"),
                ),
                Err(Stop::Trap(t)) if t.class == TrapClass::OutOfFuel => Diagnostic::error(
                    Code::E1004,
                    x.range,
                    format!(
                        "evaluating this block when compiling takes more than {} steps                          (`#[fuel(n)]` allows more)",
                        fuel_of(db, program, item)
                    ),
                ),
                Err(Stop::Trap(t)) => Diagnostic::error(
                    Code::E1004,
                    x.range,
                    format!(
                        "evaluating this block when compiling traps: {}",
                        t.class.name()
                    ),
                ),
                Err(Stop::Failed { .. }) => Diagnostic::error(
                    Code::E1004,
                    x.range,
                    "evaluating this block when compiling fails",
                ),
            };
            out.extend(moddil_sema::in_file(items, base, &[d]));
        }
    }
    // Each literal taken as a refined value satisfies the refinement (`refine.lit.1`).
    for owner in moddil_sema::owners(db, program) {
        let item = owner.item(db);
        let types = moddil_sema::infer(db, owner);
        if types.refined_literals().is_empty() {
            continue;
        }
        let body = &moddil_sema::body(db, owner).body;
        for &(e, t) in types.refined_literals() {
            let def = &items.types[t as usize];
            let Some(r) = &def.refinement else {
                continue;
            };
            let Some(v) = literal_value(body, e) else {
                continue;
            };
            let why = match machine::run(
                db,
                program,
                Item::Fn(r.holds),
                &[v],
                &mut |_, _, _| {},
                Some(FUEL),
                None,
            ) {
                Ok(Value::Tuple(xs)) if xs.first() == Some(&Value::Bool(true)) => continue,
                Ok(_) => Diagnostic::error(
                    Code::E1101,
                    body.exprs[e].range,
                    format!(
                        "this is not a `{}`: `{}` does not hold for it",
                        def.name, r.clause
                    ),
                ),
                Err(_) => Diagnostic::error(
                    Code::E1004,
                    body.exprs[e].range,
                    format!("checking this is a `{}` when compiling fails", def.name),
                ),
            };
            out.extend(moddil_sema::in_file(items, item, &[why]));
        }
    }
    out.sort_by_key(|d| (d.file, d.range.start, d.range.end, d.code));
    out
}

/// The value of a literal expression of a body.
fn literal_value(body: &moddil_sema::hir::Body, e: moddil_sema::hir::ExprId) -> Option<Value> {
    use moddil_sema::hir::{ExprKind, UnaryOp};
    Some(match &body.exprs[e].kind {
        ExprKind::Int(v) => Value::Int(i64::try_from(v.to_i128()?).ok()?),
        ExprKind::Bool(b) => Value::Bool(*b),
        ExprKind::Char(c) => Value::Char(*c),
        ExprKind::Str(s) => Value::Str(s.as_str().into()),
        ExprKind::Unary(UnaryOp::Neg, x) => match literal_value(body, *x)? {
            Value::Int(v) => Value::Int(v.checked_neg()?),
            _ => return None,
        },
        _ => return None,
    })
}

/// Whether `program` has any errors, including those of evaluating its consts.
pub fn has_errors(db: &dyn Db, program: Program) -> bool {
    diagnostics(db, program)
        .iter()
        .any(|d| d.severity == moddil_sema::Severity::Error)
}

/// Runs test `index` of `program`.
///
/// # Panics
///
/// If the file has errors.
///
/// # Errors
///
/// The trap, if the test traps, or the error, if one escapes it (which fails it).
pub fn run_test(
    db: &dyn Db,
    program: Program,
    index: u32,
    observer: Observer<'_>,
) -> Result<(), Stop> {
    run_test_seeded(db, program, index, 0, observer)
}

/// As [`run_test`], its tasks run in the order seed `seed` picks (`task.runtime.1`): with 0,
/// first in, first out.
///
/// # Errors
///
/// As [`run_test`].
pub fn run_test_seeded(
    db: &dyn Db,
    program: Program,
    index: u32,
    seed: u64,
    observer: Observer<'_>,
) -> Result<(), Stop> {
    run_test_hashed(db, program, index, seed, 0, observer)
}

/// As [`run_test_seeded`], its maps in the order hash seed `hash_seed` gives (`coll.hash.2`):
/// with 0, the order every implementation's conformance tests see.
///
/// # Errors
///
/// As [`run_test`].
pub fn run_test_hashed(
    db: &dyn Db,
    program: Program,
    index: u32,
    seed: u64,
    hash_seed: u64,
    observer: Observer<'_>,
) -> Result<(), Stop> {
    let item = Item::Test(index);
    machine::run_seeded(
        db,
        program,
        item,
        &[],
        observer,
        None,
        None,
        seed,
        Some(hash_seed),
    )
    .map(|_| ())
}

/// This crate's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests;
