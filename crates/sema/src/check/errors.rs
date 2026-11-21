//! Errors (`errors.md`): where each error can go. A body is checked inside a stack of failure
//! contexts: the function's declared errors, a test (any error fails it), a `try` block (which
//! collects them), an `errdefer` block (whose errors are suppressed), or a place where nothing
//! can fail (a `defer` block, a destructor). `fail`, `?`, `?!`, `try` and `catch` are checked
//! here, and so is each call of a function that can fail.

use moddil_syntax::TextRange;

use crate::hir::{Arm, ExprId, ExprKind, PatId, PatKind};
use crate::{Code, Ty, exhaust};

use super::Checker;

/// Where an error raised here goes.
pub(super) enum FailCx {
    /// Out of the function: its errors, whether it returns an `Option` (so `?` on `None` returns
    /// `None`), and whether its errors are inferred (so any error can leave it).
    Fn {
        allowed: Vec<Ty>,
        option: bool,
        infer: bool,
    },
    /// Out of a test, which fails.
    Test,
    /// To a `try` block: the errors found so far, and whether its `?`s are on `Option`s (once one
    /// says).
    Try {
        errors: Vec<Ty>,
        option: Option<bool>,
    },
    /// Into an `errdefer` block's suppressed errors.
    Suppressed,
    /// Nowhere: a `defer` block or a destructor.
    Never(&'static str),
    /// Out of a closure: the errors found so far, which become its type's, and whether it
    /// returns an `Option` (so `?` on `None` returns `None`).
    Closure { errors: Vec<Ty>, option: bool },
}

/// What a `?` or `?!` takes apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Propagated {
    /// A call of a function that can fail.
    Call,
    /// A `Result`.
    Result,
    /// An `Option`.
    Option,
}

impl Checker<'_> {
    /// `members` raised at `range`: they go to the innermost context that takes them.
    pub(super) fn deliver(&mut self, members: &[Ty], range: TextRange) {
        if members.is_empty() {
            return;
        }
        let show: Vec<String> = members.iter().map(|m| format!("`{m}`")).collect();
        let show = show.join(", ");
        match self.fail_cx.last() {
            Some(FailCx::Test | FailCx::Suppressed) | None => {}
            Some(FailCx::Try {
                option: Some(true), ..
            }) => self.report(
                Code::E0506,
                range,
                "this `try` block's `?`s are on `Option`s, so it cannot also fail with an error",
            ),
            Some(FailCx::Closure { errors, .. }) => {
                let mut errors = errors.clone();
                let mut ds = Vec::new();
                for m in members {
                    if !errors.contains(m) && crate::collect::error_type(m, range, &errors, &mut ds)
                    {
                        errors.push(m.clone());
                    }
                }
                self.out.diagnostics.extend(ds);
                if let Some(FailCx::Closure { errors: e, .. }) = self.fail_cx.last_mut() {
                    *e = errors;
                }
            }
            Some(FailCx::Try { errors, .. }) => {
                let mut errors = errors.clone();
                let mut ds = Vec::new();
                for m in members {
                    if !errors.contains(m) && crate::collect::error_type(m, range, &errors, &mut ds)
                    {
                        errors.push(m.clone());
                    }
                }
                self.out.diagnostics.extend(ds);
                if let Some(FailCx::Try { errors: e, option }) = self.fail_cx.last_mut() {
                    *e = errors;
                    *option = Some(false);
                }
            }
            Some(FailCx::Fn {
                allowed,
                infer: true,
                ..
            }) => {
                let mut all = allowed.clone();
                all.extend(self.out.escaping_errors.iter().cloned());
                let mut ds = Vec::new();
                for m in members {
                    if !all.contains(m)
                        && !m.has_error()
                        && crate::collect::error_type(m, range, &all, &mut ds)
                    {
                        all.push(m.clone());
                        self.out.escaping_errors.push(m.clone());
                    }
                }
                self.out.diagnostics.extend(ds);
            }
            Some(FailCx::Never(why)) => {
                let why = *why;
                self.report(
                    Code::E0501,
                    range,
                    format!("{why} cannot fail, so the error {show} must be handled here"),
                );
            }
            Some(FailCx::Fn { allowed, .. }) if allowed.is_empty() => self.report(
                Code::E0501,
                range,
                format!(
                    "this function cannot fail (it has no `with fail(…)`), so the error {show} \
                     must be handled here"
                ),
            ),
            Some(FailCx::Fn { allowed, .. }) => {
                let missing: Vec<String> = members
                    .iter()
                    .filter(|m| !allowed.contains(m) && !m.has_error())
                    .map(|m| format!("`{m}`"))
                    .collect();
                if !missing.is_empty() {
                    self.report(
                        Code::E0502,
                        range,
                        format!(
                            "this function's `with fail(…)` does not list {}: add it, or handle \
                             it here",
                            missing.join(", ")
                        ),
                    );
                }
            }
        }
    }

    /// `?` on an `Option`'s `None` at `range`: it goes to a function returning `Option`, or to a
    /// `try` block of `Option`s.
    fn deliver_none(&mut self, range: TextRange) {
        let ok = match self.fail_cx.last_mut() {
            Some(FailCx::Fn { option, .. } | FailCx::Closure { option, .. }) => *option,
            Some(FailCx::Try { option, .. }) => {
                let ok = *option != Some(false);
                *option = Some(true);
                ok
            }
            _ => false,
        };
        if !ok {
            self.report(
                Code::E0506,
                range,
                "`?` on an `Option` needs a function that returns an `Option`, or a `try` block of \
                 `Option`s, for `None` to go to: handle it with `match`, or `?!` if it cannot be \
                 `None`",
            );
        }
    }

    /// Whether a call of `func` with type arguments `args` can fail, and so must be marked: the
    /// errors it can fail with, recorded for the `?` or `?!` around it (E0501 if there is none).
    pub(super) fn fallible_call(&mut self, e: ExprId, func: u32, args: &[Ty], range: TextRange) {
        let fails = self.fx.fns[func as usize].fails.subst(args);
        let fails = self.solver.deep(&fails);
        if fails == Ty::Never {
            return;
        }
        if self.marked == Some(e) {
            self.call_fails = Some(fails);
        } else {
            self.report(
                Code::E0501,
                range,
                format!(
                    "this call can fail with `{fails}`: propagate the error with `?`, handle it \
                     with `try` and `catch`, or say it cannot happen with `?!`"
                ),
            );
        }
    }

    /// What `x` gives once `?` (or `?!`, `bang`) takes it apart: the value, and the errors to
    /// raise (`None` for an `Option`'s absence).
    fn propagate(&mut self, e: ExprId, x: ExprId) -> (Ty, Option<Vec<Ty>>) {
        let outer = self.marked.replace(x);
        let saved = self.call_fails.take();
        let t = self.infer(x);
        self.marked = outer;
        if let Some(fails) = std::mem::replace(&mut self.call_fails, saved) {
            self.out.propagated.insert(e, Propagated::Call);
            return (t, Some(fails.members()));
        }
        let range = self.range(x);
        match self.norm(&t) {
            Ty::Adt(a, _, args) if Some(a) == self.items.lang("Result") => {
                self.out.propagated.insert(e, Propagated::Result);
                let err = self.norm(&args[1]);
                if let Ty::Infer(_) = err {
                    self.unknown(range, "to propagate its error");
                    return (args[0].clone(), Some(Vec::new()));
                }
                (args[0].clone(), Some(err.members()))
            }
            Ty::Adt(a, _, args) if Some(a) == self.items.lang("Option") => {
                self.out.propagated.insert(e, Propagated::Option);
                (args[0].clone(), None)
            }
            Ty::Error => (Ty::Error, Some(Vec::new())),
            Ty::Infer(_) => (
                self.unknown(range, "to propagate from it"),
                Some(Vec::new()),
            ),
            t => {
                let t = self.show(&t);
                self.report(
                    Code::E0504,
                    range,
                    format!(
                        "`?` takes apart a call that can fail, a `Result` or an `Option`, and this \
                         is a `{t}`"
                    ),
                );
                (Ty::Error, Some(Vec::new()))
            }
        }
    }

    /// `x?`.
    pub(super) fn question(&mut self, e: ExprId, x: ExprId) -> Ty {
        let range = self.range(e);
        let (t, errors) = self.propagate(e, x);
        match errors {
            Some(errors) => self.deliver(&errors, range),
            None => self.deliver_none(range),
        }
        t
    }

    /// A wrapper's call in a wrapped function's body (`attr.around.1`): its errors, if it can
    /// fail, pass on as a marked call's do.
    pub(super) fn pass(&mut self, e: ExprId, x: ExprId) -> Ty {
        let range = self.range(e);
        let outer = self.marked.replace(x);
        let saved = self.call_fails.take();
        let t = self.infer(x);
        self.marked = outer;
        if let Some(fails) = std::mem::replace(&mut self.call_fails, saved) {
            self.out.propagated.insert(e, Propagated::Call);
            self.deliver(&fails.members(), range);
        }
        t
    }

    /// `x ?! "reason"`: an error, or `None`, traps.
    pub(super) fn bang(&mut self, e: ExprId, x: ExprId) -> Ty {
        self.propagate(e, x).0
    }

    /// `fail(x)`.
    pub(super) fn fail(&mut self, e: ExprId, x: ExprId) -> Ty {
        let range = self.range(e);
        let t = self.infer(x);
        let t = self.show(&t);
        let members = t.members();
        let mut ok = Vec::new();
        let mut ds = Vec::new();
        for m in &members {
            if matches!(m, Ty::Infer(_)) {
                self.unknown(self.range(x), "to fail with it");
            } else if crate::collect::error_type(m, self.range(x), &ok, &mut ds) {
                ok.push(m.clone());
            }
        }
        self.out.diagnostics.extend(ds);
        self.deliver(&ok, range);
        Ty::Never
    }

    /// `discard(r, "reason")`.
    pub(super) fn discard(&mut self, x: ExprId) -> Ty {
        let t = self.infer(x);
        if !self.is_result(&t) && !self.norm(&t).has_error() {
            let t = self.show(&t);
            self.report(
                Code::E0201,
                self.range(x),
                format!("`discard` drops a `Result` on purpose, and this is a `{t}`"),
            );
        }
        Ty::Unit
    }

    /// Whether `t` is a `Result`.
    pub(super) fn is_result(&mut self, t: &Ty) -> bool {
        matches!(self.norm(t), Ty::Adt(a, ..) if Some(a) == self.items.lang("Result"))
    }

    /// Checks `body` with errors going to `cx`.
    pub(super) fn within(&mut self, cx: FailCx, body: ExprId, expected: &Ty) -> FailCx {
        self.fail_cx.push(cx);
        self.check(body, expected);
        self.fail_cx.pop().expect("the context pushed")
    }

    /// `try { body }` and `try { body } catch { arms }`.
    pub(super) fn try_expr(&mut self, e: ExprId, body: ExprId, catch: Option<&[Arm]>) -> Ty {
        let range = self.range(e);
        self.fail_cx.push(FailCx::Try {
            errors: Vec::new(),
            option: None,
        });
        let t = self.infer(body);
        let Some(FailCx::Try { errors, option }) = self.fail_cx.pop() else {
            unreachable!("the `try` block's context")
        };
        let Some(arms) = catch else {
            let wrap = if option == Some(true) {
                "Option"
            } else {
                "Result"
            };
            let Some(adt) = self.items.lang(wrap) else {
                return Ty::Error;
            };
            let args = if option == Some(true) {
                vec![t]
            } else {
                vec![t, Ty::errors(errors)]
            };
            return self.items.adts[adt as usize].ty_with(adt, args);
        };
        if option == Some(true) {
            self.report(
                Code::E0505,
                range,
                "`catch` handles errors, but this block's `?`s are on `Option`s",
            );
        } else if errors.is_empty() {
            self.report(
                Code::E0505,
                range,
                "nothing in this block can fail, so there is nothing to catch",
            );
        }
        let set = Ty::errors(errors.clone());
        self.out.try_errors.insert(e, set.clone());
        let mut result = t;
        let mut rows: Vec<(Option<u32>, PatId, bool)> = Vec::new();
        for arm in arms {
            let tested = match &self.body.pats[arm.pat].kind {
                PatKind::OfType { adt, .. } | PatKind::Adt { adt, .. } => Some(*adt),
                _ => None,
            };
            let expected = match tested {
                Some(adt) => match errors.iter().find(|m| m.adt() == Some(adt)) {
                    Some(m) => m.clone(),
                    None => {
                        let name = &self.items.adts[adt as usize].name;
                        let what = if errors.is_empty() {
                            "nothing here can fail".to_string()
                        } else {
                            format!("this block fails only with `{set}`")
                        };
                        self.report(
                            Code::E0505,
                            self.body.pats[arm.pat].range,
                            format!("`{name}` cannot be caught here: {what}"),
                        );
                        Ty::Error
                    }
                },
                None => set.clone(),
            };
            self.check_pat(arm.pat, &expected, None);
            let inner = match &self.body.pats[arm.pat].kind {
                PatKind::OfType { pat, .. } => *pat,
                _ => arm.pat,
            };
            rows.push((tested, inner, arm.guard.is_none()));
            if let Some(g) = arm.guard {
                self.check(g, &Ty::Bool);
            }
            let bt = self.infer(arm.body);
            match self.join(&result, &bt) {
                Some(j) => result = j,
                None => {
                    let (a, b) = (self.show(&result), self.show(&bt));
                    self.report(
                        Code::E0201,
                        self.range(arm.body),
                        format!(
                            "a `catch` arm must have the type of its `try` block: expected `{a}`, \
                             found `{b}`"
                        ),
                    );
                }
            }
        }
        // What the arms do not handle keeps propagating.
        let mut left = Vec::new();
        for m in &errors {
            let pats: Vec<PatId> = rows
                .iter()
                .filter(|(t, _, unguarded)| *unguarded && (t.is_none() || *t == m.adt()))
                .map(|&(_, p, _)| p)
                .collect();
            if !exhaust::covers(self.items, self.body, &self.out, m, &pats) {
                left.push(m.clone());
            }
        }
        self.deliver(&left, range);
        result
    }

    /// Whether the expression statement or `let _` of type `t` drops a `Result` (E0503).
    pub(super) fn dropped_result(&mut self, t: &Ty, range: TextRange) {
        if self.is_result(t) {
            self.report(
                Code::E0503,
                range,
                "this `Result` is dropped unused: handle its error (`?`, `match`), or drop it on \
                 purpose with `discard(r, \"reason\")`",
            );
        }
    }

    /// Whether `e` is `discard(…)`, whose value is `()`.
    pub(super) fn is_discard(&self, e: ExprId) -> bool {
        matches!(self.body.exprs[e].kind, ExprKind::Discard(_))
    }
}
