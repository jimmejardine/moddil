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
mod machine;
mod natives;
mod num;
mod ops;
mod sched;
mod stack;
mod texts;
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
    /// For a failed `?!`: its reason.
    pub reason: Option<String>,
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
        Some(FUEL),
        None,
    )
}

/// Every diagnostic for `program`: the front end's and the ownership check's, then, if it has
/// no errors, those of evaluating its const items when compiling (E1004).
pub fn diagnostics(db: &dyn Db, program: Program) -> Vec<Diagnostic> {
    let mut out = moddil_mir::diagnostics(db, program).clone();
    if moddil_mir::has_errors(db, program) {
        return out;
    }
    let items = moddil_sema::item_tree(db, program);
    for (i, f) in items.fns.iter().enumerate() {
        if !f.const_item || !f.src.is_file() {
            continue;
        }
        let why = match eval_const(db, program, i as u32) {
            Ok(_) => continue,
            Err(Stop::Trap(t)) if t.class == TrapClass::OutOfFuel => format!(
                "evaluating `{}` when compiling takes more than {FUEL} steps",
                f.name
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
    machine::run(db, program, Item::Test(index), &[], observer, None, None).map(|_| ())
}

/// This crate's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests;
