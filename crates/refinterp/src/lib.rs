//! The reference interpreter: the executable form of the language reference, running the
//! canonical AST directly. It depends only on `moddil-syntax`; a test enforces this.
//!
//! It is written for clarity, not speed, and follows the reference paragraph by paragraph rather
//! than the compiler's design: it walks the syntax tree, keeps variables in a list of scopes, and
//! leaves loops and functions through Rust's `?`. It runs **checked programs only**: a file the
//! compiler rejects has no meaning to run, so it does not repeat the checks and may panic on one.
//!
//! Slices S0 and S1: functions, tests, `let`, `int`, `bool`, `()`, tuples, structs, enums,
//! patterns, `match`, `if let`, `let … else`, loops, arithmetic with its traps, comparisons,
//! logic and calls. See this crate's `README.md`.

use std::collections::HashMap;
use std::fmt;

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{SyntaxKind as K, SyntaxNode, SyntaxToken, TextRange};

/// Why a program traps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TrapClass {
    /// Integer overflow: arithmetic, negation or a shift (`trap.overflow`).
    Overflow,
    /// Integer division or remainder by zero.
    DivisionByZero,
    /// Calls nested more than [`MAX_DEPTH`] deep.
    StackOverflow,
    /// A failed `assert`.
    Assertion,
}

impl fmt::Display for TrapClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            TrapClass::Overflow => "overflow",
            TrapClass::DivisionByZero => "division by zero",
            TrapClass::StackOverflow => "stack overflow",
            TrapClass::Assertion => "assertion failed",
        })
    }
}

/// A trap: what kind, and the source of the operation that failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trap {
    /// What kind of trap.
    pub class: TrapClass,
    /// Where.
    pub range: TextRange,
}

/// How deeply calls may nest: the call that would make the stack deeper than this traps.
pub const MAX_DEPTH: usize = 10_000;

/// What running one `test` item did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TestRun {
    /// The test's name.
    pub name: String,
    /// Each observed value (an expression followed by `// =>`), in the order observed: its
    /// range and the value as Moddil source.
    pub observed: Vec<(TextRange, String)>,
    /// The trap that stopped the test, if one did.
    pub trap: Option<Trap>,
}

/// Runs every `test` item of a checked program, in order.
///
/// The program runs on its own thread with a large stack, since this interpreter recurses on the
/// host's stack.
///
/// `None` if the program could not run: it does not pass the compiler's checks, which this
/// interpreter does not repeat.
pub fn run_tests(text: &str) -> Option<Vec<TestRun>> {
    let text = text.to_string();
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(move || {
            let program = Program::new(&text);
            (0..program.tests.len())
                .map(|i| program.run_test(i))
                .collect()
        })
        .ok()?
        .join()
        .ok()
}

/// A value.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Value {
    Int(i64),
    Bool(bool),
    Unit,
    Tuple(Vec<Value>),
    /// A struct (whose variant is named like it) or an enum's variant.
    Adt {
        ty: String,
        variant: String,
        fields: Vec<Value>,
    },
}

/// How a struct or variant is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    Named,
    Tuple,
    Unit,
}

#[derive(Clone, Debug)]
struct VariantInfo {
    name: String,
    shape: Shape,
    /// Field names in declaration order (`0`, `1`, … for positional fields).
    fields: Vec<String>,
}

#[derive(Clone, Debug)]
struct AdtInfo {
    is_enum: bool,
    variants: Vec<VariantInfo>,
}

/// A program's items.
struct Program {
    text: String,
    fns: HashMap<String, ast::FnItem>,
    adts: HashMap<String, AdtInfo>,
    tests: Vec<(String, ast::Block)>,
}

/// Leaving an expression other than by producing its value.
enum Flow {
    Trap(Trap),
    Return(Value),
    /// `break`, with its label (including the `'`) and value.
    Break(Option<String>, Value),
    /// `continue`, with its label.
    Continue(Option<String>),
}

type Eval<T = Value> = Result<T, Flow>;

fn trap(class: TrapClass, range: TextRange) -> Flow {
    Flow::Trap(Trap { class, range })
}

impl Program {
    fn new(text: &str) -> Program {
        let root = moddil_syntax::parse(text).syntax();
        let mut p = Program {
            text: text.to_string(),
            fns: HashMap::new(),
            adts: HashMap::new(),
            tests: Vec::new(),
        };
        for item in root.children() {
            match item.kind() {
                K::FN_ITEM => {
                    let f = ast::FnItem::cast(item).expect("a function");
                    let name = f.name().expect("a checked function has a name").text();
                    p.fns.insert(name, f);
                }
                K::STRUCT_ITEM => {
                    let name = name_of(&item);
                    let v = variant_info(&item, name.clone());
                    p.adts.insert(
                        name,
                        AdtInfo {
                            is_enum: false,
                            variants: vec![v],
                        },
                    );
                }
                K::ENUM_ITEM => {
                    let variants = item
                        .child(K::VARIANT_LIST)
                        .map(|l| {
                            l.children()
                                .filter(|v| v.kind() == K::VARIANT)
                                .map(|v| variant_info(&v, name_of(&v)))
                                .collect()
                        })
                        .unwrap_or_default();
                    p.adts.insert(
                        name_of(&item),
                        AdtInfo {
                            is_enum: true,
                            variants,
                        },
                    );
                }
                K::TEST_ITEM => {
                    let t = ast::TestItem::cast(item).expect("a test");
                    let name = t
                        .name_token()
                        .map(|n| n.text().trim_matches('"').to_string())
                        .unwrap_or_default();
                    p.tests
                        .push((name, t.body().expect("a checked test has a body")));
                }
                _ => {}
            }
        }
        p
    }

    fn run_test(&self, index: usize) -> TestRun {
        let (name, body) = &self.tests[index];
        let mut m = Machine {
            program: self,
            scopes: Vec::new(),
            depth: 1,
            observed: Vec::new(),
        };
        let trap = match m.block(body) {
            Err(Flow::Trap(t)) => Some(t),
            _ => None,
        };
        TestRun {
            name: name.clone(),
            observed: m.observed,
            trap,
        }
    }

    /// The struct or variant a path names: `Point`, `Shape::Circle`.
    fn ctor(&self, segments: &[String]) -> Option<(String, &VariantInfo)> {
        match segments {
            [name] => {
                let adt = self.adts.get(name).filter(|a| !a.is_enum)?;
                Some((name.clone(), &adt.variants[0]))
            }
            [ty, variant] => {
                let adt = self.adts.get(ty).filter(|a| a.is_enum)?;
                adt.variants
                    .iter()
                    .find(|v| v.name == *variant)
                    .map(|v| (ty.clone(), v))
            }
            _ => None,
        }
    }

    fn variant(&self, ty: &str, variant: &str) -> &VariantInfo {
        self.adts[ty]
            .variants
            .iter()
            .find(|v| v.name == variant)
            .expect("a known variant")
    }

    /// `v` as Moddil source.
    fn render(&self, v: &Value) -> String {
        match v {
            Value::Int(i) => i.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Unit => "()".to_string(),
            Value::Tuple(xs) if xs.len() == 1 => format!("({},)", self.render(&xs[0])),
            Value::Tuple(xs) => {
                let xs: Vec<String> = xs.iter().map(|x| self.render(x)).collect();
                format!("({})", xs.join(", "))
            }
            Value::Adt {
                ty,
                variant,
                fields,
            } => {
                let path = if self.adts[ty].is_enum {
                    format!("{ty}::{variant}")
                } else {
                    ty.clone()
                };
                let info = self.variant(ty, variant);
                match info.shape {
                    Shape::Unit => path,
                    Shape::Tuple => {
                        let xs: Vec<String> = fields.iter().map(|x| self.render(x)).collect();
                        format!("{path}({})", xs.join(", "))
                    }
                    Shape::Named => {
                        let xs: Vec<String> = info
                            .fields
                            .iter()
                            .zip(fields)
                            .map(|(n, x)| format!("{n}: {}", self.render(x)))
                            .collect();
                        format!("{path} {{ {} }}", xs.join(", "))
                    }
                }
            }
        }
    }
}

fn name_of(node: &SyntaxNode) -> String {
    node.child(K::NAME)
        .and_then(ast::Name::cast)
        .map(|n| n.text())
        .unwrap_or_default()
}

fn variant_info(node: &SyntaxNode, name: String) -> VariantInfo {
    if let Some(body) = node.child(K::STRUCT_BODY) {
        let fields = body
            .children()
            .filter(|f| f.kind() == K::FIELD)
            .map(|f| name_of(&f))
            .collect();
        VariantInfo {
            name,
            shape: Shape::Named,
            fields,
        }
    } else if let Some(list) = node.child(K::TUPLE_FIELDS) {
        let n = list
            .children()
            .filter(|f| f.kind() == K::TUPLE_FIELD)
            .count();
        VariantInfo {
            name,
            shape: Shape::Tuple,
            fields: (0..n).map(|i| i.to_string()).collect(),
        }
    } else {
        VariantInfo {
            name,
            shape: Shape::Unit,
            fields: Vec::new(),
        }
    }
}

/// The state of a running test.
struct Machine<'p> {
    program: &'p Program,
    /// The current call's variables, innermost last.
    scopes: Vec<(String, Value)>,
    /// How many calls are active, counting the test.
    depth: usize,
    observed: Vec<(TextRange, String)>,
}

impl Machine<'_> {
    fn lookup(&self, name: &str) -> Option<&Value> {
        self.scopes
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v)
    }

    fn lookup_mut(&mut self, name: &str) -> &mut Value {
        self.scopes
            .iter_mut()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v)
            .expect("a checked program assigns only to variables in scope")
    }

    /// Whether a `// =>` expectation follows `end` on the same line, after an optional `;`
    /// (`items.test.observe.1`).
    fn observed(&self, end: u32) -> bool {
        let rest = self.program.text[end as usize..].trim_start_matches([' ', '\t']);
        let rest = rest
            .strip_prefix(';')
            .unwrap_or(rest)
            .trim_start_matches([' ', '\t']);
        rest.starts_with("// =>")
    }

    fn observe(&mut self, range: TextRange, v: &Value) {
        if self.observed(range.end) {
            let shown = self.program.render(v);
            self.observed.push((range, shown));
        }
    }

    // Blocks and statements.

    fn block(&mut self, b: &ast::Block) -> Eval {
        let mark = self.scopes.len();
        let result = self.block_inner(b);
        self.scopes.truncate(mark);
        result
    }

    fn block_inner(&mut self, b: &ast::Block) -> Eval {
        for s in b.stmts() {
            match s {
                ast::Stmt::Let(l) => self.let_stmt(&l)?,
                ast::Stmt::Expr(e) => {
                    let x = e.expr().expect("a checked statement");
                    let v = self.eval(&x)?;
                    self.observe(x.syntax().text_range(), &v);
                }
                ast::Stmt::BlockLike(x) => {
                    let v = self.eval(&x)?;
                    self.observe(x.syntax().text_range(), &v);
                }
                ast::Stmt::Item(_) => {}
            }
        }
        match b.tail_expr() {
            Some(t) => {
                let v = self.eval(&t)?;
                self.observe(t.syntax().text_range(), &v);
                Ok(v)
            }
            None => Ok(Value::Unit),
        }
    }

    /// `let p = e;` and `let p = e else { … };` (`stmt.let.1`, `stmt.let.else.1`).
    fn let_stmt(&mut self, l: &ast::LetStmt) -> Eval<()> {
        let v = self.eval(&l.init().expect("a checked `let` has a value"))?;
        let pat = l.pat().expect("a checked `let` has a pattern");
        let mut binds = Vec::new();
        if self.matches(&pat, &v, &mut binds) {
            self.scopes.extend(binds);
            return Ok(());
        }
        let else_ = l
            .syntax()
            .child(K::LET_ELSE)
            .and_then(|e| e.child(K::BLOCK))
            .and_then(ast::Block::cast)
            .expect("a refutable `let` has an `else`");
        // The `else` block never finishes normally, so this returns its `Flow`.
        self.block(&else_)?;
        unreachable!("the `else` block of `let … else` diverges")
    }

    // Expressions.

    fn eval(&mut self, e: &ast::Expr) -> Eval {
        let node = e.syntax();
        let range = node.text_range();
        match e {
            ast::Expr::Literal(_) => Ok(literal(node, false)),
            ast::Expr::Path(_) => {
                let segments = segments(node);
                if let [name] = segments.as_slice()
                    && let Some(v) = self.lookup(name)
                {
                    return Ok(v.clone());
                }
                let (ty, info) = self.program.ctor(&segments).expect("a checked path");
                Ok(Value::Adt {
                    ty,
                    variant: info.name.clone(),
                    fields: Vec::new(),
                })
            }
            ast::Expr::Paren(_) => self.eval(&e.sub_exprs()[0]),
            ast::Expr::Tuple(_) => {
                let xs = self.eval_all(&e.sub_exprs())?;
                Ok(if xs.is_empty() {
                    Value::Unit
                } else {
                    Value::Tuple(xs)
                })
            }
            ast::Expr::StructLit(_) => self.struct_lit(node),
            ast::Expr::Field(_) => {
                let base = self.eval(&e.sub_exprs()[0])?;
                Ok(field(self.program, &base, &member_name(node)).clone())
            }
            ast::Expr::Block(_) => self.block(&ast::Block::cast(node.clone()).expect("a block")),
            ast::Expr::If(_) => self.if_expr(e),
            ast::Expr::LetCond(_) => {
                let init = node.children().find_map(ast::Expr::cast).expect("a value");
                let v = self.eval(&init)?;
                let pat = node.children().find_map(ast::Pat::cast).expect("a pattern");
                let mut binds = Vec::new();
                let matched = self.matches(&pat, &v, &mut binds);
                if matched {
                    // In scope for the rest of the condition and the block it guards.
                    self.scopes.extend(binds);
                }
                Ok(Value::Bool(matched))
            }
            ast::Expr::Match(_) => self.match_expr(node),
            ast::Expr::Loop(_) | ast::Expr::While(_) => self.loop_expr(node, None),
            ast::Expr::Labelled(_) => {
                let label = node.token(K::LABEL).map(|t| t.text().to_string());
                let inner = node.children().find_map(ast::Expr::cast).expect("a loop");
                self.loop_expr(inner.syntax(), label.as_deref())
            }
            ast::Expr::Break(_) => {
                let label = node.token(K::LABEL).map(|t| t.text().to_string());
                let v = match e.sub_exprs().first() {
                    Some(x) => self.eval(x)?,
                    None => Value::Unit,
                };
                Err(Flow::Break(label, v))
            }
            ast::Expr::Continue(_) => {
                let label = node.token(K::LABEL).map(|t| t.text().to_string());
                Err(Flow::Continue(label))
            }
            ast::Expr::Return(_) => {
                let v = match e.sub_exprs().first() {
                    Some(x) => self.eval(x)?,
                    None => Value::Unit,
                };
                Err(Flow::Return(v))
            }
            ast::Expr::Prefix(_) => self.prefix(node, e),
            ast::Expr::Bin(_) => self.binary(node, e),
            ast::Expr::CompareChain(_) => self.chain(node, e),
            ast::Expr::Assign(_) => self.assign(node, e),
            ast::Expr::Call(_) => self.call(node, e, range),
            _ => unreachable!("a checked program of slices S0 and S1"),
        }
    }

    fn eval_all(&mut self, xs: &[ast::Expr]) -> Eval<Vec<Value>> {
        xs.iter().map(|x| self.eval(x)).collect()
    }

    /// `Point { x: 1, y }`: fields evaluated in the order written (`expr.struct.order.1`).
    fn struct_lit(&mut self, node: &SyntaxNode) -> Eval {
        let segments = segments(node);
        let (ty, info) = self.program.ctor(&segments).expect("a checked struct");
        let info = info.clone();
        let mut fields = vec![Value::Unit; info.fields.len()];
        for init in node.children().filter(|c| c.kind() == K::FIELD_INIT) {
            let name = member_name(&init);
            let v = match init.children().find_map(ast::Expr::cast) {
                Some(x) => self.eval(&x)?,
                None => self.lookup(&name).expect("a checked shorthand").clone(),
            };
            let i = info
                .fields
                .iter()
                .position(|f| *f == name)
                .expect("a checked field");
            fields[i] = v;
        }
        Ok(Value::Adt {
            ty,
            variant: info.name,
            fields,
        })
    }

    fn if_expr(&mut self, e: &ast::Expr) -> Eval {
        let subs = e.sub_exprs();
        let mark = self.scopes.len();
        let cond = self.eval(&subs[0]);
        let taken = match cond {
            Ok(Value::Bool(b)) => b,
            Ok(_) => unreachable!("a checked condition"),
            Err(f) => {
                self.scopes.truncate(mark);
                return Err(f);
            }
        };
        let result = if taken {
            self.eval(&subs[1])
        } else {
            self.scopes.truncate(mark);
            match subs.get(2) {
                Some(x) => self.eval(x),
                None => Ok(Value::Unit),
            }
        };
        self.scopes.truncate(mark);
        result
    }

    /// The arms are tried in order; the first whose pattern matches and whose guard holds runs
    /// (`expr.match.1`).
    fn match_expr(&mut self, node: &SyntaxNode) -> Eval {
        let scrutinee = node.children().find_map(ast::Expr::cast).expect("a value");
        let v = self.eval(&scrutinee)?;
        let list = node.child(K::MATCH_ARM_LIST).expect("arms");
        for arm in list.children().filter(|c| c.kind() == K::MATCH_ARM) {
            let pat = arm.children().find_map(ast::Pat::cast).expect("a pattern");
            let mut binds = Vec::new();
            if !self.matches(&pat, &v, &mut binds) {
                continue;
            }
            let mark = self.scopes.len();
            self.scopes.extend(binds);
            let guard = arm
                .child(K::MATCH_GUARD)
                .and_then(|g| g.children().find_map(ast::Expr::cast));
            let go = match guard {
                Some(g) => match self.eval(&g) {
                    Ok(Value::Bool(b)) => b,
                    Ok(_) => unreachable!("a checked guard"),
                    Err(f) => {
                        self.scopes.truncate(mark);
                        return Err(f);
                    }
                },
                None => true,
            };
            if go {
                let body = arm.children().filter_map(ast::Expr::cast).last();
                let result = self.eval(&body.expect("an arm's value"));
                self.scopes.truncate(mark);
                return result;
            }
            self.scopes.truncate(mark);
        }
        unreachable!("a checked `match` is exhaustive")
    }

    /// `loop` and `while`, maybe labelled (`expr.loop.1`, `expr.while.1`).
    fn loop_expr(&mut self, node: &SyntaxNode, label: Option<&str>) -> Eval {
        let exprs: Vec<ast::Expr> = node.children().filter_map(ast::Expr::cast).collect();
        let is_while = node.kind() == K::WHILE_EXPR;
        let body =
            ast::Block::cast(exprs.last().expect("a body").syntax().clone()).expect("a block");
        let ours = |l: &Option<String>| l.is_none() || l.as_deref() == label;
        loop {
            let mark = self.scopes.len();
            if is_while {
                match self.eval(&exprs[0]) {
                    Ok(Value::Bool(true)) => {}
                    Ok(_) => {
                        self.scopes.truncate(mark);
                        return Ok(Value::Unit);
                    }
                    Err(f) => {
                        self.scopes.truncate(mark);
                        return Err(f);
                    }
                }
            }
            let result = self.block(&body);
            self.scopes.truncate(mark);
            match result {
                Ok(_) => {}
                Err(Flow::Break(l, v)) if ours(&l) => {
                    return Ok(if is_while { Value::Unit } else { v });
                }
                Err(Flow::Continue(l)) if ours(&l) => {}
                Err(f) => return Err(f),
            }
        }
    }

    fn prefix(&mut self, node: &SyntaxNode, e: &ast::Expr) -> Eval {
        let range = node.text_range();
        let op = node
            .child_tokens()
            .find(|t| !t.kind().is_trivia())
            .expect("an operator");
        let operand = &e.sub_exprs()[0];
        if op.kind() == K::MINUS
            && let ast::Expr::Literal(lit) = operand
            && lit.token(K::INTEGER).is_some()
        {
            // `-9223372036854775808` is one literal (`expr.lit.int.1`).
            return Ok(literal(lit, true));
        }
        match (op.kind(), self.eval(operand)?) {
            (K::MINUS, Value::Int(v)) => v
                .checked_neg()
                .map(Value::Int)
                .ok_or_else(|| trap(TrapClass::Overflow, range)),
            (K::BANG, Value::Int(v)) => Ok(Value::Int(!v)),
            (K::BANG, Value::Bool(v)) => Ok(Value::Bool(!v)),
            _ => unreachable!("a checked prefix operator"),
        }
    }

    fn binary(&mut self, node: &SyntaxNode, e: &ast::Expr) -> Eval {
        let range = node.text_range();
        let op = node
            .child_tokens()
            .find(|t| !t.kind().is_trivia())
            .expect("an operator");
        let (l, r) = e.operands();
        let (l, r) = (l.expect("operands"), r.expect("operands"));
        match op.kind() {
            K::AMP_AMP | K::PIPE_PIPE => {
                let short = op.kind() == K::PIPE_PIPE;
                if self.eval(&l)? == Value::Bool(short) {
                    return Ok(Value::Bool(short));
                }
                self.eval(&r)
            }
            k if compare(k).is_some() => {
                let a = self.eval(&l)?;
                let b = self.eval(&r)?;
                Ok(Value::Bool(compare(k).expect("a comparison")(&a, &b)))
            }
            k => {
                let a = self.eval(&l)?;
                let b = self.eval(&r)?;
                arith(k, a, b, range)
            }
        }
    }

    /// `a < b <= c`: each operand once, left to right, stopping at the first comparison that is
    /// false (`expr.cmp.chain.1`).
    fn chain(&mut self, node: &SyntaxNode, e: &ast::Expr) -> Eval {
        let ops: Vec<K> = node
            .child_tokens()
            .map(|t| t.kind())
            .filter(|&k| compare(k).is_some())
            .collect();
        let operands = e.sub_exprs();
        let mut prev = self.eval(&operands[0])?;
        for (op, x) in ops.into_iter().zip(&operands[1..]) {
            let cur = self.eval(x)?;
            if !compare(op).expect("a comparison")(&prev, &cur) {
                return Ok(Value::Bool(false));
            }
            prev = cur;
        }
        Ok(Value::Bool(true))
    }

    /// `place = e` and `place op= e`: the place is a variable or fields of one
    /// (`expr.assign.1`, `expr.field.assign.1`).
    fn assign(&mut self, node: &SyntaxNode, e: &ast::Expr) -> Eval {
        let range = node.text_range();
        let op = node
            .child_tokens()
            .find(|t| !t.kind().is_trivia())
            .expect("an operator");
        let (target, value) = e.operands();
        let (target, value) = (target.expect("a place"), value.expect("a value"));
        let (root, path) = place(&target);
        let new = if op.kind() == K::EQ {
            self.eval(&value)?
        } else {
            let old = self.read_place(&root, &path);
            let v = self.eval(&value)?;
            arith(compound(op.kind()), old, v, range)?
        };
        let program = self.program;
        let mut slot = self.lookup_mut(&root);
        for name in &path {
            slot = field_mut(program, slot, name);
        }
        *slot = new;
        Ok(Value::Unit)
    }

    fn read_place(&self, root: &str, path: &[String]) -> Value {
        let mut v = self.lookup(root).expect("a variable in scope");
        for name in path {
            v = field(self.program, v, name);
        }
        v.clone()
    }

    /// A call of a function, or of a tuple struct or variant (`expr.call.1`, `expr.struct.1`).
    fn call(&mut self, node: &SyntaxNode, e: &ast::Expr, range: TextRange) -> Eval {
        let callee = &e.sub_exprs()[0];
        let args: Vec<ast::Expr> = node
            .child(K::ARG_LIST)
            .and_then(ast::ArgList::cast)
            .map(|l| l.args().filter_map(|a| a.expr()).collect())
            .unwrap_or_default();
        let segments = segments(callee.syntax());
        let local = matches!(segments.as_slice(), [one] if self.lookup(one).is_some());
        let item =
            |one: &str| self.program.fns.contains_key(one) || self.program.adts.contains_key(one);
        // `assert` is in the prelude, so a local or item of that name comes first
        // (`expr.assert.1`).
        if !local && matches!(segments.as_slice(), [one] if one == "assert" && !item(one)) {
            return match self.eval(&args[0])? {
                Value::Bool(true) => Ok(Value::Unit),
                _ => Err(trap(TrapClass::Assertion, range)),
            };
        }
        if !local
            && !matches!(segments.as_slice(), [one] if self.program.fns.contains_key(one))
            && let Some((ty, info)) = self.program.ctor(&segments)
        {
            let variant = info.name.clone();
            let fields = self.eval_all(&args)?;
            return Ok(Value::Adt {
                ty,
                variant,
                fields,
            });
        }
        let f = self.program.fns[&segments[0]].clone();
        let values = self.eval_all(&args)?;
        if self.depth >= MAX_DEPTH {
            return Err(trap(TrapClass::StackOverflow, range));
        }
        // The callee sees only its parameters.
        let saved = std::mem::take(&mut self.scopes);
        self.depth += 1;
        let params: Vec<ast::Param> = f.params().map(|l| l.params().collect()).unwrap_or_default();
        for (p, v) in params.iter().zip(values) {
            let mut binds = Vec::new();
            let matched = self.matches(&p.pat().expect("a parameter pattern"), &v, &mut binds);
            debug_assert!(matched, "parameter patterns are irrefutable");
            self.scopes.extend(binds);
        }
        let result = self.block(&f.body().expect("a body"));
        self.depth -= 1;
        self.scopes = saved;
        match result {
            Ok(v) | Err(Flow::Return(v)) => Ok(v),
            Err(f) => Err(f),
        }
    }

    // Patterns.

    /// Whether `v` matches `p`; if it does, `binds` gets the names it binds (`pat.1`).
    fn matches(&self, p: &ast::Pat, v: &Value, binds: &mut Vec<(String, Value)>) -> bool {
        let node = p.syntax();
        match p {
            ast::Pat::Wildcard(_) => true,
            ast::Pat::Literal(_) => literal_pat(node) == *v,
            ast::Pat::Range(_) => {
                let bounds: Vec<Value> = node
                    .children()
                    .filter(|c| c.kind() == K::LITERAL_PAT)
                    .map(|b| literal_pat(&b))
                    .collect();
                let (Value::Int(x), Some(Value::Int(lo))) = (v, bounds.first()) else {
                    unreachable!("a checked range pattern");
                };
                let below = match bounds.get(1) {
                    None => true,
                    Some(Value::Int(hi)) if node.token(K::DOT_DOT_EQ).is_some() => x <= hi,
                    Some(Value::Int(hi)) => x < hi,
                    Some(_) => unreachable!("an integer bound"),
                };
                lo <= x && below
            }
            ast::Pat::Ident(_) => {
                if let Some(sub) = node.children().find_map(ast::Pat::cast)
                    && !self.matches(&sub, v, binds)
                {
                    return false;
                }
                binds.push((name_of(node), v.clone()));
                true
            }
            ast::Pat::At(_) => {
                let sub = node.children().find_map(ast::Pat::cast).expect("a pattern");
                if !self.matches(&sub, v, binds) {
                    return false;
                }
                binds.push((segments(node)[0].clone(), v.clone()));
                true
            }
            ast::Pat::Path(_) => {
                let segments = segments(node);
                match self.program.ctor(&segments) {
                    Some((ty, info)) => is_ctor(v, &ty, &info.name),
                    None => {
                        binds.push((segments[0].clone(), v.clone()));
                        true
                    }
                }
            }
            ast::Pat::TupleStruct(_) => {
                let (ty, info) = self
                    .program
                    .ctor(&segments(node))
                    .expect("a checked pattern");
                if !is_ctor(v, &ty, &info.name) {
                    return false;
                }
                let Value::Adt { fields, .. } = v else {
                    unreachable!()
                };
                self.positional(node, fields, binds)
            }
            ast::Pat::Struct(_) => {
                let (ty, info) = self
                    .program
                    .ctor(&segments(node))
                    .expect("a checked pattern");
                if !is_ctor(v, &ty, &info.name) {
                    return false;
                }
                let Value::Adt { fields, .. } = v else {
                    unreachable!()
                };
                for f in node.children().filter(|c| c.kind() == K::FIELD_PAT) {
                    if f.token(K::DOT_DOT).is_some() {
                        continue;
                    }
                    let name = member_name(&f);
                    let i = info
                        .fields
                        .iter()
                        .position(|n| *n == name)
                        .expect("a checked field");
                    match f.children().find_map(ast::Pat::cast) {
                        Some(sub) => {
                            if !self.matches(&sub, &fields[i], binds) {
                                return false;
                            }
                        }
                        None => binds.push((name, fields[i].clone())),
                    }
                }
                true
            }
            ast::Pat::Tuple(_) => {
                let subs: Vec<ast::Pat> = node.children().filter_map(ast::Pat::cast).collect();
                if subs.is_empty() {
                    return *v == Value::Unit;
                }
                if subs.len() == 1
                    && node.token(K::COMMA).is_none()
                    && !matches!(subs[0], ast::Pat::Rest(_))
                {
                    return self.matches(&subs[0], v, binds);
                }
                let Value::Tuple(xs) = v else {
                    unreachable!("a checked tuple pattern")
                };
                self.positional(node, xs, binds)
            }
            ast::Pat::Or(_) => node.children().filter_map(ast::Pat::cast).any(|alt| {
                let mut tried = Vec::new();
                let ok = self.matches(&alt, v, &mut tried);
                if ok {
                    binds.extend(tried);
                }
                ok
            }),
            _ => unreachable!("a checked pattern of slices S0 and S1"),
        }
    }

    /// The positional sub-patterns of `node` (with an optional `..`) against `values`.
    fn positional(
        &self,
        node: &SyntaxNode,
        values: &[Value],
        binds: &mut Vec<(String, Value)>,
    ) -> bool {
        let subs: Vec<ast::Pat> = node.children().filter_map(ast::Pat::cast).collect();
        let rest = subs.iter().position(|s| matches!(s, ast::Pat::Rest(_)));
        let pats: Vec<&ast::Pat> = subs
            .iter()
            .filter(|s| !matches!(s, ast::Pat::Rest(_)))
            .collect();
        let n = pats.len();
        pats.iter().enumerate().all(|(i, p)| {
            // After `..`, patterns match the last elements.
            let at = match rest {
                Some(r) if i >= r => values.len() - (n - i),
                _ => i,
            };
            self.matches(p, &values[at], binds)
        })
    }
}

fn is_ctor(v: &Value, ty: &str, variant: &str) -> bool {
    matches!(v, Value::Adt { ty: t, variant: n, .. } if t == ty && n == variant)
}

/// Field `name` (a name, or a position in a tuple or tuple struct) of `v`.
fn field<'v>(program: &Program, v: &'v Value, name: &str) -> &'v Value {
    match v {
        Value::Tuple(xs) => &xs[name.parse::<usize>().expect("a tuple index")],
        Value::Adt {
            ty,
            variant,
            fields,
        } => {
            let i = program
                .variant(ty, variant)
                .fields
                .iter()
                .position(|f| f == name)
                .expect("a checked field");
            &fields[i]
        }
        _ => unreachable!("a checked field access"),
    }
}

fn field_mut<'v>(program: &Program, v: &'v mut Value, name: &str) -> &'v mut Value {
    match v {
        Value::Tuple(xs) => &mut xs[name.parse::<usize>().expect("a tuple index")],
        Value::Adt {
            ty,
            variant,
            fields,
        } => {
            let i = program
                .variant(ty, variant)
                .fields
                .iter()
                .position(|f| f == name)
                .expect("a checked field");
            &mut fields[i]
        }
        _ => unreachable!("a checked field access"),
    }
}

/// The variable and field path of an assignment's target: `p.x.0` is `("p", ["x", "0"])`.
fn place(target: &ast::Expr) -> (String, Vec<String>) {
    match target {
        ast::Expr::Field(_) => {
            let (root, mut path) = place(&target.sub_exprs()[0]);
            path.push(member_name(target.syntax()));
            (root, path)
        }
        ast::Expr::Paren(_) => place(&target.sub_exprs()[0]),
        _ => (segments(target.syntax())[0].clone(), Vec::new()),
    }
}

fn segments(node: &SyntaxNode) -> Vec<String> {
    node.child(K::PATH)
        .and_then(ast::Path::cast)
        .map(|p| p.segments())
        .unwrap_or_default()
}

/// The name after `.` in a field expression, or of a field initialiser or field pattern.
fn member_name(node: &SyntaxNode) -> String {
    let tokens: Vec<SyntaxToken> = node
        .child_tokens()
        .filter(|t| !t.kind().is_trivia())
        .collect();
    let start = tokens
        .iter()
        .position(|t| t.kind() == K::DOT)
        .map_or(0, |i| i + 1);
    tokens[start..]
        .iter()
        .find(|t| matches!(t.kind(), K::IDENT | K::INTEGER) || t.kind().is_keyword())
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
            Value::Int(i64::try_from(v).expect("a checked literal fits in `int`"))
        }
        _ => unreachable!("a literal of slices S0 and S1"),
    }
}

fn literal_pat(node: &SyntaxNode) -> Value {
    literal(node, node.token(K::MINUS).is_some())
}

/// The value of an integer literal: decimal, or `0x`, `0o`, `0b`, with `_` separators
/// (`expr.lit.int.1`).
fn integer(text: &str) -> i128 {
    let digits: String = text.chars().filter(|&c| c != '_').collect();
    let (radix, digits) = match digits.get(..2) {
        Some("0x") => (16, &digits[2..]),
        Some("0o") => (8, &digits[2..]),
        Some("0b") => (2, &digits[2..]),
        _ => (10, digits.as_str()),
    };
    i128::from_str_radix(digits, radix).expect("a checked literal")
}

type Comparison = fn(&Value, &Value) -> bool;

fn compare(k: K) -> Option<Comparison> {
    fn ints(a: &Value, b: &Value) -> (i64, i64) {
        match (a, b) {
            (Value::Int(a), Value::Int(b)) => (*a, *b),
            _ => unreachable!("ordering compares integers"),
        }
    }
    Some(match k {
        K::EQ_EQ => |a, b| a == b,
        K::BANG_EQ => |a, b| a != b,
        K::LT => |a, b| {
            let (a, b) = ints(a, b);
            a < b
        },
        K::LT_EQ => |a, b| {
            let (a, b) = ints(a, b);
            a <= b
        },
        K::GT => |a, b| {
            let (a, b) = ints(a, b);
            a > b
        },
        K::GT_EQ => |a, b| {
            let (a, b) = ints(a, b);
            a >= b
        },
        _ => return None,
    })
}

/// The operator of a compound assignment: `+` for `+=`.
fn compound(k: K) -> K {
    match k {
        K::PLUS_EQ => K::PLUS,
        K::MINUS_EQ => K::MINUS,
        K::STAR_EQ => K::STAR,
        K::SLASH_EQ => K::SLASH,
        K::PERCENT_EQ => K::PERCENT,
        K::AMP_EQ => K::AMP,
        K::PIPE_EQ => K::PIPE,
        K::CARET_EQ => K::CARET,
        K::LT_LT_EQ => K::LT_LT,
        K::GT_GT_EQ => K::GT_GT,
        _ => unreachable!("a compound assignment"),
    }
}

/// `a op b` for the arithmetic and bitwise operators, trapping as the reference says
/// (`expr.arith.overflow.1`, `expr.arith.div.1`, `expr.bit.shift.1`).
fn arith(op: K, a: Value, b: Value, range: TextRange) -> Eval {
    let overflow = || trap(TrapClass::Overflow, range);
    match (a, b) {
        (Value::Int(a), Value::Int(b)) => {
            let v = match op {
                K::PLUS => a.checked_add(b).ok_or_else(overflow)?,
                K::MINUS => a.checked_sub(b).ok_or_else(overflow)?,
                K::STAR => a.checked_mul(b).ok_or_else(overflow)?,
                K::SLASH | K::PERCENT if b == 0 => {
                    return Err(trap(TrapClass::DivisionByZero, range));
                }
                K::SLASH => a.checked_div(b).ok_or_else(overflow)?,
                // `int::MIN % -1` is 0: the exact answer fits.
                K::PERCENT => a.wrapping_rem(b),
                K::AMP => a & b,
                K::PIPE => a | b,
                K::CARET => a ^ b,
                K::LT_LT => {
                    if !(0..64).contains(&b) || (a << b) >> b != a {
                        return Err(overflow());
                    }
                    a << b
                }
                K::GT_GT => {
                    if !(0..64).contains(&b) {
                        return Err(overflow());
                    }
                    a >> b
                }
                _ => unreachable!("an arithmetic operator"),
            };
            Ok(Value::Int(v))
        }
        (Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(match op {
            K::AMP => a & b,
            K::PIPE => a | b,
            K::CARET => a ^ b,
            _ => unreachable!("a checked operator on `bool`"),
        })),
        _ => unreachable!("checked operands"),
    }
}

/// This crate's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests;
