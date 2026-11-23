//! Running a test: the [`Machine`] evaluates the syntax tree, keeping its variables in
//! [`Memory`] and its items in a [`Program`].
//!
//! Evaluation follows the reference by topic: [`blocks`] (and statements), [`exprs`],
//! [`ops`] (arithmetic and comparisons), [`calls`] and [`patterns`].

use std::collections::HashMap;

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{SyntaxKind as K, SyntaxNode, TextRange};

use crate::memory::Memory;
use crate::program::{Method, Program};
use crate::value::{Head, Pointer, Value};
use crate::{Eval, Flow, TestRun};

mod blocks;
mod calls;
mod closures;
mod effects;
mod errors;
mod exprs;
mod ops;
mod patterns;
pub(crate) mod tasks;
mod text;
mod wrappers;

use patterns::Bind;

/// The state of a running test.
struct Machine<'p> {
    program: &'p Program,
    memory: Memory,
    /// The `defer` blocks of each open block, innermost last, with how many variables there
    /// were when each was reached, and whether it is an `errdefer`.
    defers: Vec<Vec<(ast::Block, usize, bool)>>,
    observed: Vec<(u32, TextRange, String)>,
    /// For each `try` block being run, innermost last: whether its `?`s are on `Option`s, once
    /// one has said.
    try_modes: Vec<Option<bool>>,
    /// The handlers installed by the `with` blocks being run, innermost last: how the effect is
    /// looked up, and a borrow of the handler.
    handlers: Vec<(String, Value)>,
    /// Whether the call that last returned is one that can fail, for a `?` around it.
    failing_call: bool,
    /// A trap a refinement test raised while a pattern was matched, until what is matching
    /// raises it.
    pending_trap: Option<Flow>,
    /// For each call being run, innermost last: the values its postconditions' `old(e)` calls
    /// stand for, by the call's range.
    olds: Vec<HashMap<TextRange, Value>>,
    /// The nodes closure values have named so far, found once.
    nodes: HashMap<crate::value::NodeRef, SyntaxNode>,
    /// The fields `inline for` names (`comptime.template.inline.1`), innermost last: each
    /// name, the struct's key and the field's index.
    reflected: Vec<(String, String, usize)>,
    /// While a `#[selectable]` function runs for a `select` (`task.select.4`): its receive, by
    /// range, and what was received in its place.
    received: Option<(TextRange, Value)>,
    /// For a conversion being run (`num.convert.1`): the type it converts to.
    convert_to: Option<Head>,
}

impl Program {
    pub(crate) fn run_test(&self, index: usize) -> TestRun {
        let test = &self.tests[index];
        let (name, body) = (&test.name, &test.body);
        let mut m = Machine::new(self, Memory::new(test.file, test.module as usize));
        let result = m.block(body);
        // A test's value is dropped at its end, like its variables.
        let result = match result {
            Ok(v) => m.drop_value(v, true, body.syntax().text_range()),
            Err(f) => Err(f),
        };
        m.end_tasks();
        let (trap, error) = match result {
            Err(Flow::Trap(t, true)) => (Some(t), None),
            Err(Flow::Trap(t, false)) => (
                Some(crate::Trap {
                    file: test.file,
                    ..t
                }),
                None,
            ),
            // An error that leaves a test fails it (`err.test.1`).
            Err(Flow::Fail(_, at)) => (None, Some((test.file, at))),
            _ => (None, None),
        };
        TestRun {
            name: name.clone(),
            observed: m.observed,
            trap,
            error,
        }
    }
}

impl<'p> Machine<'p> {
    /// A machine running `program`'s code with `memory`.
    fn new(program: &'p Program, memory: Memory) -> Self {
        Machine {
            program,
            memory,
            defers: Vec::new(),
            observed: Vec::new(),
            try_modes: Vec::new(),
            handlers: Vec::new(),
            failing_call: false,
            pending_trap: None,
            olds: Vec::new(),
            nodes: HashMap::new(),
            reflected: Vec::new(),
            received: None,
            convert_to: None,
        }
    }
}

impl Machine<'_> {
    /// The node a closure value names.
    fn node(&mut self, r: &crate::value::NodeRef) -> SyntaxNode {
        if let Some(n) = self.nodes.get(r) {
            return n.clone();
        }
        let n = self.program.node(r);
        self.nodes.insert(*r, n.clone());
        n
    }

    fn bind(&mut self, binds: Vec<Bind>) {
        for (name, v, from) in binds {
            if let Some(p) = from
                && self.needs_drop(&v)
            {
                self.memory.store(&p, Value::Moved);
            }
            self.memory.push(name, v);
        }
    }

    /// `v`, or what it points to if it is a borrow.
    fn target(&self, v: Value) -> Value {
        match v {
            Value::Ref(ptr, _) => self.memory.load(&ptr),
            v => v,
        }
    }

    /// The position of field `name` in `v` (a tuple index, or a struct's or variant's field).
    /// The field a reflected name `node` (a path of one name) stands for: the struct's key and
    /// the field's index.
    fn reflected(&self, node: &SyntaxNode) -> Option<(String, usize)> {
        if node.kind() != K::PATH_EXPR {
            return None;
        }
        let segs = segments(node);
        let [name] = segs.as_slice() else {
            return None;
        };
        self.reflected
            .iter()
            .rev()
            .find(|(n, ..)| n == name)
            .map(|(_, k, i)| (k.clone(), *i))
    }

    /// The refinement field `i` of struct `key` is of, by its key, if it is one.
    fn field_refinement(&self, key: &str, i: usize) -> Option<String> {
        let adt = &self.program.adts[key];
        let ty = adt.variants[0].types.get(i).cloned().flatten()?;
        let path = crate::program::type_path(ty.syntax());
        match self.program.resolve(adt.module, &path) {
            Some((crate::program::Def::Item(k), n))
                if n == path.len()
                    && self.program.types.get(&k).is_some_and(|t| t.pred.is_some()) =>
            {
                Some(k)
            }
            _ => None,
        }
    }

    fn field_index(&self, v: &Value, name: &str) -> usize {
        match v {
            Value::Tuple(_) => name.parse().expect("a tuple index"),
            Value::Adt { ty, variant, .. } => self
                .program
                .variant(ty, variant)
                .fields
                .iter()
                .position(|f| f == name)
                .expect("a checked field"),
            _ => unreachable!("a checked field access"),
        }
    }

    /// Where the place `e` is, if it is one (a variable, a dereferenced borrow, or fields of
    /// one; field access looks through a borrow), and whether it is reached through a borrow.
    fn place_of(&mut self, e: &ast::Expr) -> Eval<Option<(Pointer, bool)>> {
        Ok(match e {
            ast::Expr::Path(_) => {
                let segs = segments(e.syntax());
                match segs.as_slice() {
                    [n] => self.memory.slot(n).map(|s| (self.memory.here(s), false)),
                    _ => None,
                }
            }
            ast::Expr::Paren(_) => self.place_of(&e.sub_exprs()[0])?,
            // `f.name` of a reflected field is a value, not a place.
            ast::Expr::Field(_) if self.reflected(e.sub_exprs()[0].syntax()).is_some() => {
                let v = self.eval(e)?;
                Some((self.memory.temp(v), false))
            }
            ast::Expr::Field(_) | ast::Expr::ReflectedField(_) => {
                let base = &e.sub_exprs()[0];
                let (mut p, mut indirect) = match self.place_of(base)? {
                    Some(p) => p,
                    None => {
                        // A field of a temporary: the temporary is dropped at the end of the
                        // statement.
                        let v = self.eval(base)?;
                        (self.memory.temp(v), false)
                    }
                };
                if let Value::Ref(target, _) = self.memory.load(&p) {
                    p = target;
                    indirect = true;
                }
                let i = match e {
                    // `x.[f]`: the field `f` names (`comptime.template.field.1`).
                    ast::Expr::ReflectedField(_) => {
                        self.reflected(e.sub_exprs()[1].syntax())
                            .expect("a checked reflected field")
                            .1
                    }
                    _ => self.field_index(&self.memory.load(&p), &member_name(e.syntax())),
                };
                p.path.push(i);
                Some((p, indirect))
            }
            ast::Expr::Index(_) => {
                let parts = e.sub_exprs();
                let (mut p, mut indirect) = match self.place_of(&parts[0])? {
                    Some(p) => p,
                    None => {
                        let v = self.eval(&parts[0])?;
                        (self.memory.temp(v), false)
                    }
                };
                if let Value::Ref(target, _) = self.memory.load(&p) {
                    p = target;
                    indirect = true;
                }
                // `xs[a..b]` (`expr.slice.1`): part of the array, or of what a `Vec`'s
                // `as_slice` gives.
                if let ast::Expr::Range(_) = &parts[1] {
                    return Ok(Some((
                        self.slice_place(p, &parts[1], range_of(e))?,
                        indirect,
                    )));
                }
                let Value::Int(i) = self.eval(&parts[1])? else {
                    unreachable!("a checked index")
                };
                // An index into part of an array is one into the array.
                let i = if let Some((start, n)) = p.span.take() {
                    if i < 0 || i as usize >= n {
                        return Err(crate::trap(
                            crate::TrapClass::IndexOutOfBounds,
                            e.syntax().text_range(),
                        ));
                    }
                    i + start as i64
                } else {
                    i
                };
                let (Value::Array(xs) | Value::Buf(_, xs)) = self.memory.load(&p) else {
                    // Through `Index` (`expr.index.trait.1`): the element its borrow points to.
                    let head = self.memory.load(&p).head();
                    let m = self
                        .program
                        .trait_method("Index", &head, "index")
                        .expect("a checked index");
                    let r = self.run_method(
                        m,
                        &head,
                        vec![Value::Ref(p, false), Value::Int(i)],
                        e.syntax().text_range(),
                    )?;
                    let Value::Ref(q, _) = r else {
                        unreachable!("a borrow of the element")
                    };
                    return Ok(Some((q, true)));
                };
                // `expr.index.1`: out of bounds traps.
                if i < 0 || i as usize >= xs.len() {
                    return Err(crate::trap(
                        crate::TrapClass::IndexOutOfBounds,
                        e.syntax().text_range(),
                    ));
                }
                p.path.push(i as usize);
                Some((p, indirect))
            }
            ast::Expr::Prefix(_) if e.syntax().token(K::STAR).is_some() => {
                let x = &e.sub_exprs()[0];
                let p = match self.place_of(x)? {
                    Some((p, _)) => p,
                    None => {
                        let v = self.eval(x)?;
                        self.memory.temp(v)
                    }
                };
                match self.memory.load(&p) {
                    Value::Ref(target, _) => Some((target, true)),
                    // Through `Deref` (`type.box.1`): the value its borrow points to.
                    v => {
                        let head = v.head();
                        let m = self
                            .program
                            .trait_method("Deref", &head, "deref")
                            .expect("a checked dereference");
                        let r = self.run_method(
                            m,
                            &head,
                            vec![Value::Ref(p, false)],
                            e.syntax().text_range(),
                        )?;
                        let Value::Ref(q, _) = r else {
                            unreachable!("a borrow of the value")
                        };
                        Some((q, true))
                    }
                }
            }
            _ => None,
        })
    }

    /// Part `range` (a range expression) of the array or slice at `p`, or of what the `Vec` at
    /// `p` holds: checked to be in bounds (`expr.slice.1`).
    fn slice_place(&mut self, mut p: Pointer, range: &ast::Expr, at: TextRange) -> Eval<Pointer> {
        let v = self.memory.load(&p);
        if let v @ Value::Adt { .. } = &v {
            let head = v.head();
            let m = self
                .program
                .methods(&head, "as_slice")
                .into_iter()
                .next()
                .expect("a checked slice");
            match self.run_method(m, &head, vec![Value::Ref(p.clone(), false)], at)? {
                Value::Ref(q, _) => p = q,
                _ => unreachable!("a slice borrow"),
            }
        }
        let len = match self.memory.load(&p) {
            Value::Array(xs) | Value::Buf(_, xs) => xs.len() as i64,
            _ => unreachable!("a checked slice"),
        };
        let node = range.syntax();
        let inclusive = node.token(K::DOT_DOT_EQ).is_some();
        let op = node
            .child_tokens()
            .find(|t| matches!(t.kind(), K::DOT_DOT | K::DOT_DOT_EQ))
            .map_or(node.text_range().start, |t| t.text_range().start);
        let (mut a, mut b) = (0, len);
        for part in range.sub_exprs() {
            let Value::Int(x) = self.eval(&part)? else {
                unreachable!("a checked end")
            };
            if part.syntax().text_range().start < op {
                a = x;
            } else {
                b = if inclusive { x.wrapping_add(1) } else { x };
            }
        }
        if !(0 <= a && a <= b && b <= len) {
            return Err(crate::trap(crate::TrapClass::IndexOutOfBounds, at));
        }
        let start = p.span.map_or(0, |(s, _)| s);
        p.span = Some((start + a as usize, (b - a) as usize));
        Ok(p)
    }

    /// The closure at `p`, through borrows and boxes (`expr.closure.stored.1`), if there is one.
    /// One kept in a box runs with its caller's handlers too, which meet the needs its type
    /// lists.
    pub(super) fn closure_at(
        &mut self,
        mut p: Pointer,
        range: TextRange,
    ) -> Eval<Option<std::sync::Arc<crate::value::Closure>>> {
        loop {
            match self.memory.load(&p) {
                Value::Ref(q, _) => p = q,
                // It runs with the handlers where it was written (`expr.closure.effects.1`).
                Value::Closure(c) => return Ok(Some(c)),
                v @ Value::Adt { .. } => {
                    let head = v.head();
                    let Some(m) = self.program.trait_method("Deref", &head, "deref") else {
                        return Ok(None);
                    };
                    match self.run_method(m, &head, vec![Value::Ref(p, false)], range)? {
                        Value::Ref(q, _) => p = q,
                        _ => return Ok(None),
                    }
                }
                _ => return Ok(None),
            }
        }
    }

    /// Where the place `e` is; any other expression is evaluated into a nameless variable, so
    /// that it can be borrowed (`borrow.1`).
    fn place(&mut self, e: &ast::Expr) -> Eval<Pointer> {
        if let Some((p, _)) = self.place_of(e)? {
            return Ok(p);
        }
        let v = self.eval(e)?;
        Ok(self.memory.temp(v))
    }

    /// The value at a place, moved out if it needs dropping (other values are copies, which
    /// are the same), unless the place is reached through a borrow.
    fn take(&mut self, p: &Pointer, indirect: bool) -> Value {
        let v = self.memory.load(p);
        if !indirect && self.needs_drop(&v) {
            self.memory.store(p, Value::Moved);
        }
        v
    }

    /// `v` as Moddil source; a borrow shows what it points to (`items.test.observe.1`).
    fn show(&self, v: &Value) -> String {
        match v {
            Value::Ref(ptr, true) => format!("&mut {}", self.show(&self.memory.load(ptr))),
            Value::Ref(ptr, false) => format!("&{}", self.show(&self.memory.load(ptr))),
            v => self.program.render_with(v, &|x| self.show(x)),
        }
    }

    /// Whether a `// =>` expectation follows `end` on the same line, after an optional `;`
    /// (`items.test.observe.1`).
    fn observed(&self, end: u32) -> bool {
        let Some(file) = self.memory.file() else {
            return false;
        };
        let rest =
            self.program.texts[file as usize][end as usize..].trim_start_matches([' ', '\t']);
        let rest = rest
            .strip_prefix(';')
            .unwrap_or(rest)
            .trim_start_matches([' ', '\t']);
        rest.starts_with("// =>")
    }

    fn observe(&mut self, range: TextRange, v: &Value) {
        if self.observed(range.end) {
            let shown = self.show(v);
            let file = self.memory.file().unwrap_or(0);
            self.observed.push((file, range, shown));
        }
    }

    // Drops.

    /// Whether dropping `v` runs a destructor: it is, or contains, a value whose type has one.
    fn needs_drop(&self, v: &Value) -> bool {
        match v {
            Value::Adt { ty, fields, .. } => {
                self.has_destructor(ty) || fields.iter().any(|f| self.needs_drop(f))
            }
            Value::Tuple(xs) | Value::Array(xs) | Value::Buf(_, xs) => {
                xs.iter().any(|x| self.needs_drop(x))
            }
            _ => false,
        }
    }

    fn has_destructor(&self, ty: &str) -> bool {
        self.program
            .impls
            .iter()
            .any(|i| i.trait_.as_deref() == Some("Drop") && i.head == Head::Adt(ty.to_string()))
    }

    /// Drops `v` (`own.drop.1`): runs its destructor if `own` and its type has one (the
    /// destructor then drops the fields), or else drops its fields, last first.
    fn drop_value(&mut self, v: Value, own: bool, range: TextRange) -> Eval<()> {
        match v {
            Value::Adt { ref ty, .. } if own && self.has_destructor(ty) => {
                let head = Head::Adt(ty.clone());
                let Some(Method::Code(f, _)) = self.program.trait_method("Drop", &head, "drop")
                else {
                    unreachable!("a destructor")
                };
                self.invoke(&f, vec![v], HashMap::new(), range)?;
                Ok(())
            }
            Value::Adt { fields: xs, .. } | Value::Tuple(xs) => {
                for x in xs.into_iter().rev() {
                    self.drop_value(x, true, range)?;
                }
                Ok(())
            }
            // Elements are dropped first to last.
            Value::Array(xs) | Value::Buf(_, xs) => {
                for x in xs {
                    self.drop_value(x, true, range)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Drops the variables from `mark` on, last first, and removes them; then the result
    /// stands, unless a destructor traps. Nothing is dropped after a trap.
    /// Moves the variables of the innermost call from slot `mark` on that a closure in `v`
    /// captures out to slots of their own, which outlive the call: a closure that leaves the
    /// scope it was written in owns what it captures (`expr.closure.result.1`,
    /// `expr.closure.stored.1`).
    pub(super) fn keep_captures(&mut self, v: &mut Value, mark: usize, returned: bool) {
        let frame = self.memory.depth() - 1;
        let task = self.memory.task();
        match v {
            Value::Closure(c) => {
                let leaving = |p: &Pointer| p.task == task && p.frame == frame && p.slot >= mark;
                if !c.vars.iter().any(|(_, p)| leaving(p)) && (!returned || c.escaped) {
                    return;
                }
                let mut owned = (**c).clone();
                owned.escaped |= returned;
                for (name, p) in &mut owned.vars {
                    if leaving(p) && p.path.is_empty() {
                        let x = self.memory.load(p);
                        self.memory.store(p, Value::Moved);
                        *p = self.memory.add_static(format!("{{captured {name}}}"), x);
                    }
                }
                *c = std::sync::Arc::new(owned);
            }
            Value::Tuple(xs)
            | Value::Array(xs)
            | Value::Buf(_, xs)
            | Value::Adt { fields: xs, .. } => {
                for x in xs {
                    self.keep_captures(x, mark, returned);
                }
            }
            _ => {}
        }
    }

    fn exit<T>(&mut self, mark: usize, result: Eval<T>, range: TextRange) -> Eval<T> {
        if matches!(result, Err(Flow::Trap(..))) {
            self.memory.truncate(mark);
            return result;
        }
        while self.memory.mark() > mark {
            let (_, v) = self.memory.pop().expect("a variable");
            self.drop_value(v, true, range)?;
        }
        result
    }

    /// Drops the nameless variables from `mark` on (the temporaries of a statement); they are
    /// left empty rather than removed, since borrows may point past them.
    fn drop_temps(&mut self, mark: usize, range: TextRange) -> Eval<()> {
        for i in (mark..self.memory.mark()).rev() {
            if self
                .memory
                .temp_value(i)
                .is_some_and(|v| self.needs_drop(&v))
            {
                let v = self.memory.take(i);
                self.drop_value(v, true, range)?;
            }
        }
        Ok(())
    }
}

fn is_ctor(v: &Value, ty: &str, variant: &str) -> bool {
    matches!(v, Value::Adt { ty: t, variant: n, .. } if t == ty && n == variant)
}

/// The names of a path's segments, without generic arguments.
fn segments(node: &SyntaxNode) -> Vec<String> {
    node.child(K::PATH)
        .map(|p| {
            p.children()
                .filter(|c| c.kind() == K::PATH_SEGMENT)
                .filter_map(|s| {
                    s.child_tokens()
                        .find(|t| t.kind() == K::IDENT || t.kind().is_keyword())
                        .map(|t| t.text().to_string())
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The name after `.` in a field expression, or of a field initialiser or field pattern.
fn member_name(node: &SyntaxNode) -> String {
    ast::member_name(node)
        .map(|t| t.text().to_string())
        .expect("a member name")
}

fn literal(node: &SyntaxNode, negate: bool) -> Value {
    let tok = node
        .child_tokens()
        .find(|t| !t.kind().is_trivia() && t.kind() != K::MINUS)
        .expect("a literal token");
    match tok.kind() {
        K::TRUE_KW => Value::Bool(true),
        K::FALSE_KW => Value::Bool(false),
        K::INTEGER => {
            let v = integer(tok.text());
            let v = if negate { -v } else { v };
            i64::try_from(v).map_or(Value::BigLit(v), Value::Int)
        }
        // A fractional literal: exact until it meets its type (`num.lit.1`).
        K::FLOAT => {
            let t: String = tok.text().chars().filter(|&c| c != '_').collect();
            Value::FloatLit(if negate { format!("-{t}") } else { t }.into())
        }
        K::CHAR => Value::Char(text::character(tok.text())),
        K::STRING | K::RAW_STRING | K::BLOCK_STRING => Value::Str(text::string(tok.text()).into()),
        _ => unreachable!("a literal checked to be supported"),
    }
}

/// The value of an integer literal: decimal, or `0x`, `0o`, `0b`, with `_` separators
/// (`expr.lit.int.1`).
fn integer(text: &str) -> i128 {
    ast::int_value(text).expect("a checked literal")
}

/// Where expression `e` is.
fn range_of(e: &ast::Expr) -> TextRange {
    e.syntax().text_range()
}
