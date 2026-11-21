//! The reference interpreter: the executable form of the language reference, running the
//! canonical AST directly. It depends only on `moddil-syntax`; a test enforces this.
//!
//! It is written for clarity, not speed, and follows the reference paragraph by paragraph rather
//! than the compiler's design: it walks the syntax tree, keeps variables in a list of scopes, and
//! leaves loops and functions through Rust's `?`. It runs **checked programs only**: a file the
//! compiler rejects has no meaning to run, so it does not repeat the checks and may panic on one.
//!
//! Slices S0 to S3. A borrow is a pointer to a variable of some call and a path of fields in it.
//! The prelude (`reference/2027/prelude.moddil`) is read with the program. Values carry enough
//! of their type to choose a method: a trait's method is found from the type of the value it is
//! called on, and a type parameter (for `T::f()`) is known from the values of the arguments
//! given for it, or from type arguments written at the call. Moving a value out of a variable
//! leaves it empty, so it is not dropped again. Derives are built in, structurally. See this
//! crate's `README.md`.

use std::collections::HashMap;
use std::fmt;

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{SyntaxKind as K, SyntaxNode, SyntaxToken, TextRange};

/// The prelude's source.
const PRELUDE: &str = include_str!("../../../reference/2027/prelude.moddil");

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
    /// A borrow, and whether it is `&mut`.
    Ref(Pointer, bool),
    /// Nothing: a variable whose value was moved out (`own.move.1`), so it is not dropped.
    Moved,
}

/// Where a borrow points: a call's frame, a variable in it, and a path of field positions.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Pointer {
    frame: usize,
    slot: usize,
    path: Vec<usize>,
}

/// What a pattern is matched against (`pat.mode.1`): a place the value is in, whose parts the
/// names move out (unless the place is reached through a borrow, when they are copied); a place
/// a borrow points to, whose parts the names borrow; or a value, which the names copy.
enum Src {
    Place(Pointer, bool),
    At(Pointer, bool),
    Val(Value),
}

/// A name bound by a pattern: its value, and the place it is moved out of, if it is.
type Bind = (String, Value, Option<Pointer>);

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
    /// The fields' types as written.
    types: Vec<Option<ast::Type>>,
}

#[derive(Clone, Debug)]
struct AdtInfo {
    is_enum: bool,
    variants: Vec<VariantInfo>,
    /// The type parameters' names.
    params: Vec<String>,
    /// The traits it derives.
    derives: Vec<String>,
    /// Whether its variants are named on their own (`Some`, `None`).
    bare: bool,
}

/// How a method takes `self`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelfKind {
    Value,
    Ref,
    RefMut,
}

/// A function: free, or a method of an impl or a trait.
#[derive(Clone, Debug)]
struct FnDef {
    node: ast::FnItem,
    /// Whether it is the file's code (not the prelude's).
    user: bool,
    /// Its own type parameters' names.
    params: Vec<String>,
    self_kind: Option<SelfKind>,
    /// Whether it is a destructor, whose `self` is not dropped again.
    destructor: bool,
}

/// The kind of a value, which decides which impl a method comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Head {
    Int,
    Bool,
    Unit,
    Tuple,
    Ref,
    Adt(String),
}

struct ImplInfo {
    trait_: Option<String>,
    head: Head,
    self_ty: ast::Type,
    params: Vec<String>,
    methods: HashMap<String, FnDef>,
}

struct TraitInfo {
    methods: HashMap<String, FnDef>,
}

/// Where a method comes from.
#[derive(Clone)]
enum Method {
    /// Code: a function, with the impl it is in (`None` for a trait's default).
    Code(FnDef, Option<usize>),
    /// A derived trait's method, built in: `Clone`, `PartialEq`, `PartialOrd` or `Ord`.
    Derived(String),
    /// The built-in `Clone` of a borrow, a tuple or `()`: a copy.
    Copy,
}

impl Method {
    fn self_kind(&self) -> Option<SelfKind> {
        match self {
            Method::Code(f, _) => f.self_kind,
            Method::Derived(_) | Method::Copy => Some(SelfKind::Ref),
        }
    }
}

/// A program's items: the prelude's and the file's.
struct Program {
    text: String,
    fns: HashMap<String, FnDef>,
    adts: HashMap<String, AdtInfo>,
    traits: HashMap<String, TraitInfo>,
    impls: Vec<ImplInfo>,
    tests: Vec<(String, ast::Block)>,
}

/// Leaving an expression other than by producing its value.
enum Flow {
    /// A trap, and whether it has been placed in the file's code yet: a trap in the prelude's
    /// code is reported where the file's code called it.
    Trap(Trap, bool),
    Return(Value),
    /// `break`, with its label (including the `'`) and value.
    Break(Option<String>, Value),
    /// `continue`, with its label.
    Continue(Option<String>),
}

type Eval<T = Value> = Result<T, Flow>;

fn trap(class: TrapClass, range: TextRange) -> Flow {
    Flow::Trap(Trap { class, range }, false)
}

fn generic_names(node: &SyntaxNode) -> Vec<String> {
    node.child(K::GENERIC_PARAMS)
        .map(|g| {
            g.children()
                .filter(|c| c.kind() == K::TYPE_PARAM)
                .map(|p| name_of(&p))
                .collect()
        })
        .unwrap_or_default()
}

fn fn_def(f: &ast::FnItem, user: bool, destructor: bool) -> FnDef {
    let self_kind = f
        .syntax()
        .child(K::PARAM_LIST)
        .and_then(|l| l.child(K::SELF_PARAM))
        .map(
            |s| match (s.token(K::AMP).is_some(), s.token(K::MUT_KW).is_some()) {
                (true, true) => SelfKind::RefMut,
                (true, false) => SelfKind::Ref,
                (false, _) => SelfKind::Value,
            },
        );
    FnDef {
        node: f.clone(),
        user,
        params: generic_names(f.syntax()),
        self_kind,
        destructor,
    }
}

/// The kind of value a type written in an impl's header has.
fn head_of_type(t: &ast::Type) -> Head {
    match t {
        ast::Type::Tuple(n) if n.children().next().is_none() => Head::Unit,
        ast::Type::Tuple(_) => Head::Tuple,
        ast::Type::Ref(_) => Head::Ref,
        _ => match first_name(t.syntax()).as_str() {
            "int" => Head::Int,
            "bool" => Head::Bool,
            name => Head::Adt(name.to_string()),
        },
    }
}

/// The first name in a type's path: `Option` for `Option<T>`.
fn first_name(node: &SyntaxNode) -> String {
    node.descendants()
        .into_iter()
        .find(|n| n.kind() == K::PATH_SEGMENT)
        .and_then(|s| {
            s.child_tokens()
                .find(|t| matches!(t.kind(), K::IDENT | K::SELF_TYPE_KW))
                .map(|t| t.text().to_string())
        })
        .unwrap_or_default()
}

impl Program {
    fn new(text: &str) -> Program {
        let mut p = Program {
            text: text.to_string(),
            fns: HashMap::new(),
            adts: HashMap::new(),
            traits: HashMap::new(),
            impls: Vec::new(),
            tests: Vec::new(),
        };
        let prelude = moddil_syntax::parse(PRELUDE).syntax();
        let file = moddil_syntax::parse(text).syntax();
        for (user, root) in [(false, prelude), (true, file)] {
            for item in root.children() {
                p.item(user, item);
            }
        }
        p
    }

    fn item(&mut self, user: bool, item: SyntaxNode) {
        match item.kind() {
            K::FN_ITEM => {
                let f = ast::FnItem::cast(item).expect("a function");
                let name = f.name().expect("a checked function has a name").text();
                self.fns.insert(name, fn_def(&f, user, false));
            }
            K::STRUCT_ITEM | K::ENUM_ITEM => {
                let name = name_of(&item);
                let variants = if item.kind() == K::STRUCT_ITEM {
                    vec![variant_info(&item, name.clone())]
                } else {
                    item.child(K::VARIANT_LIST)
                        .map(|l| {
                            l.children()
                                .filter(|v| v.kind() == K::VARIANT)
                                .map(|v| variant_info(&v, name_of(&v)))
                                .collect()
                        })
                        .unwrap_or_default()
                };
                let derives = derives(&item);
                self.adts.insert(
                    name.clone(),
                    AdtInfo {
                        is_enum: item.kind() == K::ENUM_ITEM,
                        variants,
                        params: generic_names(&item),
                        derives,
                        bare: !user && name == "Option",
                    },
                );
            }
            K::TRAIT_ITEM => {
                let methods = item
                    .child(K::TRAIT_BODY)
                    .map(|b| {
                        b.children()
                            .filter_map(ast::FnItem::cast)
                            .map(|f| {
                                let name = f.name().expect("a name").text();
                                (name, fn_def(&f, user, false))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                self.traits.insert(name_of(&item), TraitInfo { methods });
            }
            K::IMPL_ITEM => {
                let types: Vec<ast::Type> = item.children().filter_map(ast::Type::cast).collect();
                let has_trait = item.token(K::FOR_KW).is_some();
                let trait_ = has_trait.then(|| first_name(types[0].syntax()));
                let self_ty = types[usize::from(has_trait)].clone();
                let destructor = trait_.as_deref() == Some("Drop");
                let methods = item
                    .child(K::IMPL_BODY)
                    .map(|b| {
                        b.children()
                            .filter_map(ast::FnItem::cast)
                            .map(|f| {
                                let name = f.name().expect("a name").text();
                                (name, fn_def(&f, user, destructor))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                self.impls.push(ImplInfo {
                    trait_,
                    head: head_of_type(&self_ty),
                    self_ty,
                    params: generic_names(&item),
                    methods,
                });
            }
            K::TEST_ITEM if user => {
                let t = ast::TestItem::cast(item).expect("a test");
                let name = t
                    .name_token()
                    .map(|n| n.text().trim_matches('"').to_string())
                    .unwrap_or_default();
                self.tests
                    .push((name, t.body().expect("a checked test has a body")));
            }
            _ => {}
        }
    }

    fn run_test(&self, index: usize) -> TestRun {
        let (name, body) = &self.tests[index];
        let mut m = Machine {
            program: self,
            frames: vec![Frame {
                vars: Vec::new(),
                types: HashMap::new(),
                user: true,
            }],
            defers: Vec::new(),
            observed: Vec::new(),
        };
        let result = m.block(body);
        // A test's value is dropped at its end, like its variables.
        let result = match result {
            Ok(v) => m.drop_value(v, true, body.syntax().text_range()),
            Err(f) => Err(f),
        };
        let trap = match result {
            Err(Flow::Trap(t, _)) => Some(t),
            _ => None,
        };
        TestRun {
            name: name.clone(),
            observed: m.observed,
            trap,
        }
    }

    /// The struct or variant a path names: `Point`, `Shape::Circle`, `Some`.
    fn ctor(&self, segments: &[String]) -> Option<(String, &VariantInfo)> {
        match segments {
            [name] => {
                if let Some(adt) = self.adts.get(name).filter(|a| !a.is_enum) {
                    return Some((name.clone(), &adt.variants[0]));
                }
                self.adts.iter().find_map(|(ty, a)| {
                    if a.bare {
                        a.variants
                            .iter()
                            .find(|v| v.name == *name)
                            .map(|v| (ty.clone(), v))
                    } else {
                        None
                    }
                })
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
            Value::Ref(..) => unreachable!("a borrow is shown by the machine"),
            Value::Moved => "<moved>".to_string(),
            Value::Adt {
                ty,
                variant,
                fields,
            } => {
                let adt = &self.adts[ty];
                let path = if adt.bare {
                    variant.clone()
                } else if adt.is_enum {
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

    /// Whether values of kind `head` implement trait `name`: by an impl, a derive, or (for
    /// `Clone` and `Copy`) built in.
    fn implements(&self, head: &Head, name: &str) -> bool {
        if self
            .impls
            .iter()
            .any(|i| i.head == *head && i.trait_.as_deref() == Some(name))
        {
            return true;
        }
        match head {
            Head::Adt(ty) => self.adts[ty].derives.iter().any(|d| d == name),
            Head::Ref | Head::Tuple | Head::Unit => matches!(name, "Clone" | "Copy"),
            _ => false,
        }
    }

    /// The methods called `name` that values of kind `head` have, in the order they are
    /// chosen (`expr.method.1`): an inherent impl's, then a trait's (from an impl, a derive, or
    /// the trait's default).
    fn methods(&self, head: &Head, name: &str) -> Vec<Method> {
        let mut out = Vec::new();
        for (k, i) in self.impls.iter().enumerate() {
            if i.trait_.is_none()
                && i.head == *head
                && let Some(f) = i.methods.get(name)
            {
                out.push(Method::Code(f.clone(), Some(k)));
            }
        }
        for (k, i) in self.impls.iter().enumerate() {
            if i.trait_.is_some()
                && i.head == *head
                && let Some(f) = i.methods.get(name)
            {
                out.push(Method::Code(f.clone(), Some(k)));
            }
        }
        if let Head::Adt(ty) = head {
            for d in &self.adts[ty].derives {
                let provides = matches!(
                    (d.as_str(), name),
                    ("Clone", "clone")
                        | ("PartialEq", "eq")
                        | ("PartialOrd", "partial_cmp")
                        | ("Ord", "cmp")
                );
                if provides {
                    out.push(Method::Derived(d.clone()));
                }
            }
        }
        if name == "clone" && matches!(head, Head::Ref | Head::Tuple | Head::Unit) {
            out.push(Method::Copy);
        }
        for (tname, t) in &self.traits {
            if let Some(f) = t.methods.get(name)
                && f.node.body().is_some()
                && self.implements(head, tname)
                && !out.iter().any(|m| matches!(m, Method::Code(g, Some(k)) if g.node == f.node || self.impls[*k].trait_.as_deref() == Some(tname)))
            {
                out.push(Method::Code(f.clone(), None));
            }
        }
        out
    }

    /// The method called `name` of trait `tr` for values of kind `head`.
    fn trait_method(&self, tr: &str, head: &Head, name: &str) -> Option<Method> {
        for (k, i) in self.impls.iter().enumerate() {
            if i.trait_.as_deref() == Some(tr)
                && i.head == *head
                && let Some(f) = i.methods.get(name)
            {
                return Some(Method::Code(f.clone(), Some(k)));
            }
        }
        if let Head::Adt(ty) = head
            && self.adts[ty].derives.iter().any(|d| d == tr)
            && matches!(
                (tr, name),
                ("Clone", "clone")
                    | ("PartialEq", "eq")
                    | ("PartialOrd", "partial_cmp")
                    | ("Ord", "cmp")
            )
        {
            return Some(Method::Derived(tr.to_string()));
        }
        if tr == "Clone" && matches!(head, Head::Ref | Head::Tuple | Head::Unit) {
            return Some(Method::Copy);
        }
        let f = self.traits.get(tr)?.methods.get(name)?;
        f.node
            .body()
            .is_some()
            .then(|| Method::Code(f.clone(), None))
    }
}

fn name_of(node: &SyntaxNode) -> String {
    node.child(K::NAME)
        .and_then(ast::Name::cast)
        .map(|n| n.text())
        .unwrap_or_default()
}

/// The traits `#[derive(…)]` on an item names.
fn derives(node: &SyntaxNode) -> Vec<String> {
    let mut out = Vec::new();
    for attr in node.children().filter(|c| c.kind() == K::OUTER_ATTR) {
        let Some(a) = attr.child(K::ATTR) else {
            continue;
        };
        if let Some(input) = a.child(K::ATTR_INPUT) {
            out.extend(
                input
                    .child_tokens()
                    .filter(|t| t.kind() == K::IDENT)
                    .map(|t| t.text().to_string()),
            );
        }
    }
    out
}

fn variant_info(node: &SyntaxNode, name: String) -> VariantInfo {
    if let Some(body) = node.child(K::STRUCT_BODY) {
        let fields: Vec<SyntaxNode> = body.children().filter(|f| f.kind() == K::FIELD).collect();
        VariantInfo {
            name,
            shape: Shape::Named,
            fields: fields.iter().map(name_of).collect(),
            types: fields
                .iter()
                .map(|f| f.children().find_map(ast::Type::cast))
                .collect(),
        }
    } else if let Some(list) = node.child(K::TUPLE_FIELDS) {
        let fields: Vec<SyntaxNode> = list
            .children()
            .filter(|f| f.kind() == K::TUPLE_FIELD)
            .collect();
        VariantInfo {
            name,
            shape: Shape::Tuple,
            fields: (0..fields.len()).map(|i| i.to_string()).collect(),
            types: fields
                .iter()
                .map(|f| f.children().find_map(ast::Type::cast))
                .collect(),
        }
    } else {
        VariantInfo {
            name,
            shape: Shape::Unit,
            fields: Vec::new(),
            types: Vec::new(),
        }
    }
}

/// One call's variables and the kinds of its type parameters.
struct Frame {
    /// Its variables, innermost last. Temporaries that are borrowed or dropped get a nameless
    /// variable.
    vars: Vec<(String, Value)>,
    /// The kind of each type parameter it knows (and of `Self`).
    types: HashMap<String, Head>,
    /// Whether it runs the file's code (not the prelude's).
    user: bool,
}

/// The state of a running test.
struct Machine<'p> {
    program: &'p Program,
    /// One frame per active call (the test first).
    frames: Vec<Frame>,
    /// The `defer` blocks of each open block, innermost last, with how many variables there
    /// were when each was reached.
    defers: Vec<Vec<(ast::Block, usize)>>,
    observed: Vec<(TextRange, String)>,
}

impl Machine<'_> {
    fn frame(&self) -> &Frame {
        self.frames.last().expect("a frame")
    }

    fn vars(&self) -> &Vec<(String, Value)> {
        &self.frame().vars
    }

    fn vars_mut(&mut self) -> &mut Vec<(String, Value)> {
        &mut self.frames.last_mut().expect("a frame").vars
    }

    fn mark(&self) -> usize {
        self.vars().len()
    }

    fn bind(&mut self, binds: Vec<Bind>) {
        for (name, v, from) in binds {
            if let Some(p) = from
                && self.needs_drop(&v)
            {
                self.store(&p, Value::Moved);
            }
            self.vars_mut().push((name, v));
        }
    }

    fn slot(&self, name: &str) -> Option<usize> {
        self.vars().iter().rposition(|(n, _)| n == name)
    }

    fn here(&self, slot: usize) -> Pointer {
        Pointer {
            frame: self.frames.len() - 1,
            slot,
            path: Vec::new(),
        }
    }

    /// A nameless variable holding `v`, so that it can be borrowed or dropped.
    fn temp(&mut self, v: Value) -> Pointer {
        self.vars_mut().push((String::new(), v));
        self.here(self.mark() - 1)
    }

    fn load(&self, ptr: &Pointer) -> Value {
        let mut v = &self.frames[ptr.frame].vars[ptr.slot].1;
        for &i in &ptr.path {
            v = match v {
                Value::Tuple(xs) | Value::Adt { fields: xs, .. } => &xs[i],
                _ => unreachable!("a checked field"),
            };
        }
        v.clone()
    }

    fn store(&mut self, ptr: &Pointer, value: Value) {
        let mut v = &mut self.frames[ptr.frame].vars[ptr.slot].1;
        for &i in &ptr.path {
            v = match v {
                Value::Tuple(xs) | Value::Adt { fields: xs, .. } => &mut xs[i],
                _ => unreachable!("a checked field"),
            };
        }
        *v = value;
    }

    /// `v`, or what it points to if it is a borrow.
    fn target(&self, v: Value) -> Value {
        match v {
            Value::Ref(ptr, _) => self.load(&ptr),
            v => v,
        }
    }

    /// The kind of a value, for choosing a method.
    fn head(v: &Value) -> Head {
        match v {
            Value::Int(_) => Head::Int,
            Value::Bool(_) => Head::Bool,
            Value::Unit | Value::Moved => Head::Unit,
            Value::Tuple(_) => Head::Tuple,
            Value::Ref(..) => Head::Ref,
            Value::Adt { ty, .. } => Head::Adt(ty.clone()),
        }
    }

    /// The position of field `name` in `v` (a tuple index, or a struct's or variant's field).
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
                    [n] => self.slot(n).map(|s| (self.here(s), false)),
                    _ => None,
                }
            }
            ast::Expr::Paren(_) => self.place_of(&e.sub_exprs()[0])?,
            ast::Expr::Field(_) => {
                let base = &e.sub_exprs()[0];
                let (mut p, mut indirect) = match self.place_of(base)? {
                    Some(p) => p,
                    None => {
                        // A field of a temporary: the temporary is dropped at the end of the
                        // statement.
                        let v = self.eval(base)?;
                        (self.temp(v), false)
                    }
                };
                if let Value::Ref(target, _) = self.load(&p) {
                    p = target;
                    indirect = true;
                }
                let i = self.field_index(&self.load(&p), &member_name(e.syntax()));
                p.path.push(i);
                Some((p, indirect))
            }
            ast::Expr::Prefix(_) if e.syntax().token(K::STAR).is_some() => {
                let v = self.eval(&e.sub_exprs()[0])?;
                match v {
                    Value::Ref(target, _) => Some((target, true)),
                    _ => unreachable!("a checked dereference"),
                }
            }
            _ => None,
        })
    }

    /// Where the place `e` is; any other expression is evaluated into a nameless variable, so
    /// that it can be borrowed (`borrow.1`).
    fn place(&mut self, e: &ast::Expr) -> Eval<Pointer> {
        if let Some((p, _)) = self.place_of(e)? {
            return Ok(p);
        }
        let v = self.eval(e)?;
        Ok(self.temp(v))
    }

    /// The value at a place, moved out if it needs dropping (other values are copies, which
    /// are the same), unless the place is reached through a borrow.
    fn take(&mut self, p: &Pointer, indirect: bool) -> Value {
        let v = self.load(p);
        if !indirect && self.needs_drop(&v) {
            self.store(p, Value::Moved);
        }
        v
    }

    /// `v` as Moddil source; a borrow shows what it points to (`items.test.observe.1`).
    fn show(&self, v: &Value) -> String {
        match v {
            Value::Ref(ptr, true) => format!("&mut {}", self.show(&self.load(ptr))),
            Value::Ref(ptr, false) => format!("&{}", self.show(&self.load(ptr))),
            Value::Tuple(xs) => {
                let xs: Vec<String> = xs.iter().map(|x| self.show(x)).collect();
                if xs.len() == 1 {
                    format!("({},)", xs[0])
                } else {
                    format!("({})", xs.join(", "))
                }
            }
            v => self.program.render(v),
        }
    }

    /// Whether a `// =>` expectation follows `end` on the same line, after an optional `;`
    /// (`items.test.observe.1`).
    fn observed(&self, end: u32) -> bool {
        if !self.frame().user {
            return false;
        }
        let rest = self.program.text[end as usize..].trim_start_matches([' ', '\t']);
        let rest = rest
            .strip_prefix(';')
            .unwrap_or(rest)
            .trim_start_matches([' ', '\t']);
        rest.starts_with("// =>")
    }

    fn observe(&mut self, range: TextRange, v: &Value) {
        if self.observed(range.end) {
            let shown = self.show(v);
            self.observed.push((range, shown));
        }
    }

    // Drops.

    /// Whether dropping `v` runs a destructor: it is, or contains, a value whose type has one.
    fn needs_drop(&self, v: &Value) -> bool {
        match v {
            Value::Adt { ty, fields, .. } => {
                self.has_destructor(ty) || fields.iter().any(|f| self.needs_drop(f))
            }
            Value::Tuple(xs) => xs.iter().any(|x| self.needs_drop(x)),
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
            _ => Ok(()),
        }
    }

    /// Drops the variables from `mark` on, last first, and removes them; then the result
    /// stands, unless a destructor traps. Nothing is dropped after a trap.
    fn exit<T>(&mut self, mark: usize, result: Eval<T>, range: TextRange) -> Eval<T> {
        if matches!(result, Err(Flow::Trap(..))) {
            self.vars_mut().truncate(mark);
            return result;
        }
        while self.mark() > mark {
            let (_, v) = self.vars_mut().pop().expect("a variable");
            self.drop_value(v, true, range)?;
        }
        result
    }

    /// Drops the nameless variables from `mark` on (the temporaries of a statement); they are
    /// left empty rather than removed, since borrows may point past them.
    fn drop_temps(&mut self, mark: usize, range: TextRange) -> Eval<()> {
        for i in (mark..self.mark()).rev() {
            if self.vars()[i].0.is_empty() && self.needs_drop(&self.vars()[i].1) {
                let v = std::mem::replace(&mut self.vars_mut()[i].1, Value::Moved);
                self.drop_value(v, true, range)?;
            }
        }
        Ok(())
    }

    // Blocks and statements.

    /// A block. Leaving it in any way but a trap runs its `defer` blocks and drops its
    /// variables, in the reverse of the order they were reached (`stmt.defer.1`,
    /// `own.drop.1`).
    fn block(&mut self, b: &ast::Block) -> Eval {
        let range = b.syntax().text_range();
        let mark = self.mark();
        self.defers.push(Vec::new());
        let mut result = self.block_inner(b);
        let defers = self.defers.pop().expect("this block's defers");
        if !matches!(result, Err(Flow::Trap(..))) {
            for (d, at) in defers.iter().rev() {
                // The variables declared after the `defer` are dropped before it runs.
                result = self.exit(*at, result, range);
                if matches!(result, Err(Flow::Trap(..))) {
                    break;
                }
                if let Err(f) = self.block(d) {
                    result = Err(f);
                    break;
                }
            }
        }
        self.exit(mark, result, range)
    }

    fn block_inner(&mut self, b: &ast::Block) -> Eval {
        for s in b.stmts() {
            let mark = self.mark();
            let range = match &s {
                ast::Stmt::Let(l) => l.syntax().text_range(),
                ast::Stmt::Expr(e) => e.syntax().text_range(),
                ast::Stmt::BlockLike(x) => x.syntax().text_range(),
                ast::Stmt::Item(n) => n.text_range(),
            };
            match s {
                ast::Stmt::Let(l) => self.let_stmt(&l)?,
                ast::Stmt::Expr(e) => {
                    let x = e.expr().expect("a checked statement");
                    self.expr_stmt(&x)?;
                }
                ast::Stmt::BlockLike(x) => self.expr_stmt(&x)?,
                ast::Stmt::Item(n) => {
                    if n.kind() == K::DEFER_STMT
                        && let Some(d) = n.child(K::BLOCK).and_then(ast::Block::cast)
                    {
                        let at = self.mark();
                        self.defers.last_mut().expect("a block").push((d, at));
                    }
                }
            }
            // Temporaries are dropped at the end of their statement.
            self.drop_temps(mark, range)?;
        }
        match b.tail_expr() {
            Some(t) => {
                let mark = self.mark();
                let v = self.eval(&t)?;
                self.observe(t.syntax().text_range(), &v);
                self.drop_temps(mark, t.syntax().text_range())?;
                Ok(v)
            }
            None => Ok(Value::Unit),
        }
    }

    /// An expression statement: a place's value is read but not moved (`stmt.expr.1`); any
    /// other value is dropped.
    fn expr_stmt(&mut self, x: &ast::Expr) -> Eval<()> {
        let range = x.syntax().text_range();
        if let Some((p, _)) = self.place_of(x)? {
            let v = self.load(&p);
            self.observe(range, &v);
            return Ok(());
        }
        let v = self.eval(x)?;
        self.observe(range, &v);
        self.drop_value(v, true, range)
    }

    /// `let p = e;` and `let p = e else { … };` (`stmt.let.1`, `stmt.let.else.1`).
    fn let_stmt(&mut self, l: &ast::LetStmt) -> Eval<()> {
        let init = l.init().expect("a checked `let` has a value");
        let src = self.scrutinee(&init)?;
        let pat = l.pat().expect("a checked `let` has a pattern");
        let mut binds = Vec::new();
        if self.matches(&pat, &src, &mut binds) {
            self.bind(binds);
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

    /// What a pattern matches against the value of `e`: `e`'s own place if it is one, or else
    /// a nameless variable holding its value.
    fn scrutinee(&mut self, e: &ast::Expr) -> Eval<Src> {
        if let Some((p, indirect)) = self.place_of(e)? {
            return Ok(Src::Place(p, indirect));
        }
        let v = self.eval(e)?;
        Ok(Src::Place(self.temp(v), false))
    }

    // Expressions.

    fn eval(&mut self, e: &ast::Expr) -> Eval {
        let node = e.syntax();
        let range = node.text_range();
        match e {
            ast::Expr::Literal(_) => Ok(literal(node, false)),
            ast::Expr::Path(_) => {
                if let Some((p, indirect)) = self.place_of(e)? {
                    return Ok(self.take(&p, indirect));
                }
                let segments = segments(node);
                let (ty, info) = self.ctor(&segments).expect("a checked path");
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
                // Field access looks through a borrow (`expr.field.1`).
                let (p, indirect) = self.place_of(e)?.expect("a field is a place");
                Ok(self.take(&p, indirect))
            }
            ast::Expr::Ref(_) => {
                let mutable = node.token(K::MUT_KW).is_some();
                let p = self.place(&e.sub_exprs()[0])?;
                Ok(Value::Ref(p, mutable))
            }
            ast::Expr::Block(_) => self.block(&ast::Block::cast(node.clone()).expect("a block")),
            ast::Expr::If(_) => self.if_expr(e),
            ast::Expr::LetCond(_) => {
                let init = node.children().find_map(ast::Expr::cast).expect("a value");
                let src = self.scrutinee(&init)?;
                let pat = node.children().find_map(ast::Pat::cast).expect("a pattern");
                let mut binds = Vec::new();
                let matched = self.matches(&pat, &src, &mut binds);
                if matched {
                    // In scope for the rest of the condition and the block it guards.
                    self.bind(binds);
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
            ast::Expr::MethodCall(_) => self.method_call(node, e, range),
            _ => unreachable!("a checked program of slices S0 to S3"),
        }
    }

    fn eval_all(&mut self, xs: &[ast::Expr]) -> Eval<Vec<Value>> {
        xs.iter().map(|x| self.eval(x)).collect()
    }

    /// The struct or variant a path names, with `Self` for the impl's type.
    fn ctor(&self, segments: &[String]) -> Option<(String, &VariantInfo)> {
        let self_ty = match self.frame().types.get("Self") {
            Some(Head::Adt(t)) => Some(t.clone()),
            _ => None,
        };
        let resolved: Vec<String> = segments
            .iter()
            .map(|s| match (s.as_str(), &self_ty) {
                ("Self", Some(t)) => t.clone(),
                _ => s.clone(),
            })
            .collect();
        self.program.ctor(&resolved)
    }

    /// `Point { x: 1, y }`: fields evaluated in the order written (`expr.struct.order.1`).
    fn struct_lit(&mut self, node: &SyntaxNode) -> Eval {
        let segments = segments(node);
        let (ty, info) = self.ctor(&segments).expect("a checked struct");
        let info = info.clone();
        let mut fields = vec![Value::Unit; info.fields.len()];
        for init in node.children().filter(|c| c.kind() == K::FIELD_INIT) {
            let name = member_name(&init);
            let v = match init.children().find_map(ast::Expr::cast) {
                Some(x) => self.eval(&x)?,
                None => {
                    let s = self.slot(&name).expect("a checked shorthand");
                    let p = self.here(s);
                    self.take(&p, false)
                }
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
        let range = e.syntax().text_range();
        let subs = e.sub_exprs();
        let mark = self.mark();
        let taken = match self.eval(&subs[0]) {
            Ok(Value::Bool(b)) => b,
            Ok(_) => unreachable!("a checked condition"),
            Err(f) => return self.exit(mark, Err(f), range),
        };
        if taken {
            let result = self.eval(&subs[1]);
            self.exit(mark, result, range)
        } else {
            self.exit(mark, Ok(()), range)?;
            match subs.get(2) {
                Some(x) => self.eval(x),
                None => Ok(Value::Unit),
            }
        }
    }

    /// The arms are tried in order; the first whose pattern matches and whose guard holds runs
    /// (`expr.match.1`). Its names move or borrow parts of the value only once it is chosen.
    fn match_expr(&mut self, node: &SyntaxNode) -> Eval {
        let range = node.text_range();
        let scrutinee = node.children().find_map(ast::Expr::cast).expect("a value");
        let src = self.scrutinee(&scrutinee)?;
        let list = node.child(K::MATCH_ARM_LIST).expect("arms");
        for arm in list.children().filter(|c| c.kind() == K::MATCH_ARM) {
            let pat = arm.children().find_map(ast::Pat::cast).expect("a pattern");
            let mut binds = Vec::new();
            if !self.matches(&pat, &src, &mut binds) {
                continue;
            }
            let mark = self.mark();
            let guard = arm
                .child(K::MATCH_GUARD)
                .and_then(|g| g.children().find_map(ast::Expr::cast));
            if let Some(g) = guard {
                // While the guard runs, the names hold copies of what they would bind.
                let copies: Vec<Bind> = binds
                    .iter()
                    .map(|(n, v, _)| (n.clone(), v.clone(), None))
                    .collect();
                self.bind(copies);
                let go = match self.eval(&g) {
                    Ok(Value::Bool(b)) => b,
                    Ok(_) => unreachable!("a checked guard"),
                    Err(f) => {
                        self.vars_mut().truncate(mark);
                        return Err(f);
                    }
                };
                self.vars_mut().truncate(mark);
                if !go {
                    continue;
                }
            }
            self.bind(binds);
            let body = arm.children().filter_map(ast::Expr::cast).last();
            let result = self.eval(&body.expect("an arm's value"));
            return self.exit(mark, result, range);
        }
        unreachable!("a checked `match` is exhaustive")
    }

    /// `loop` and `while`, maybe labelled (`expr.loop.1`, `expr.while.1`).
    fn loop_expr(&mut self, node: &SyntaxNode, label: Option<&str>) -> Eval {
        let range = node.text_range();
        let exprs: Vec<ast::Expr> = node.children().filter_map(ast::Expr::cast).collect();
        let is_while = node.kind() == K::WHILE_EXPR;
        let body =
            ast::Block::cast(exprs.last().expect("a body").syntax().clone()).expect("a block");
        let ours = |l: &Option<String>| l.is_none() || l.as_deref() == label;
        loop {
            let mark = self.mark();
            if is_while {
                match self.eval(&exprs[0]) {
                    Ok(Value::Bool(true)) => {}
                    Ok(_) => return self.exit(mark, Ok(Value::Unit), range),
                    Err(f) => return self.exit(mark, Err(f), range),
                }
            }
            let result = self.block(&body);
            let result = self.exit(mark, result, range);
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
        if op.kind() == K::STAR {
            // What a borrow points to (`expr.deref.1`).
            let v = self.eval(operand)?;
            return Ok(self.target(v));
        }
        match (op.kind(), self.eval(operand)?) {
            (K::MINUS, Value::Int(v)) => v
                .checked_neg()
                .map(Value::Int)
                .ok_or_else(|| trap(TrapClass::Overflow, range)),
            (K::BANG, Value::Int(v)) => Ok(Value::Int(!v)),
            (K::BANG, Value::Bool(v)) => Ok(Value::Bool(!v)),
            // Other types, through the operator's trait (`expr.op.trait.1`).
            (K::MINUS, v) => self.call_trait("Neg", "neg", vec![v], range),
            (K::BANG, v) => self.call_trait("Not", "not", vec![v], range),
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
                let pairs = [(k, r)];
                self.compare_all(&l, &pairs)
            }
            k => {
                let a = self.eval(&l)?;
                let b = self.eval(&r)?;
                if let (Value::Int(_) | Value::Bool(_), _) = (&a, &b) {
                    return arith(k, a, b, range);
                }
                let (tr, method) = match k {
                    K::PLUS => ("Add", "add"),
                    K::MINUS => ("Sub", "sub"),
                    K::STAR => ("Mul", "mul"),
                    K::SLASH => ("Div", "div"),
                    K::PERCENT => ("Rem", "rem"),
                    _ => unreachable!("a checked operator"),
                };
                self.call_trait(tr, method, vec![a, b], range)
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
        let pairs: Vec<(K, ast::Expr)> =
            ops.into_iter().zip(operands[1..].iter().cloned()).collect();
        self.compare_all(&operands[0], &pairs)
    }

    /// Comparisons: on `int`, `bool` and `()` directly; on other types through `PartialEq` and
    /// `PartialOrd`, with borrows of the operands (`expr.cmp.trait.1`).
    fn compare_all(&mut self, first: &ast::Expr, rest: &[(K, ast::Expr)]) -> Eval {
        let range = first
            .syntax()
            .parent()
            .map_or_else(|| first.syntax().text_range(), |p| p.text_range());
        let mut prev = self.place(first)?;
        for (op, x) in rest {
            let cur = self.place(x)?;
            let (a, b) = (self.load(&prev), self.load(&cur));
            let holds = match (&a, &b) {
                (Value::Int(_) | Value::Bool(_) | Value::Unit, _) => {
                    compare(*op).expect("a comparison")(&a, &b)
                }
                _ => {
                    let (tr, method) = match *op {
                        K::EQ_EQ => ("PartialEq", "eq"),
                        K::BANG_EQ => ("PartialEq", "ne"),
                        K::LT => ("PartialOrd", "lt"),
                        K::LT_EQ => ("PartialOrd", "le"),
                        K::GT => ("PartialOrd", "gt"),
                        _ => ("PartialOrd", "ge"),
                    };
                    let args = vec![
                        Value::Ref(prev.clone(), false),
                        Value::Ref(cur.clone(), false),
                    ];
                    self.call_trait(tr, method, args, range)? == Value::Bool(true)
                }
            };
            if !holds {
                return Ok(Value::Bool(false));
            }
            prev = cur;
        }
        Ok(Value::Bool(true))
    }

    /// `place = e` and `place op= e`: the place is a variable or fields of one
    /// (`expr.assign.1`, `expr.field.assign.1`). The old value is dropped first (`own.drop.1`).
    fn assign(&mut self, node: &SyntaxNode, e: &ast::Expr) -> Eval {
        let range = node.text_range();
        let op = node
            .child_tokens()
            .find(|t| !t.kind().is_trivia())
            .expect("an operator");
        let (target, value) = e.operands();
        let (target, value) = (target.expect("a place"), value.expect("a value"));
        // `x = e` evaluates `e` first; `x op= e` reads `x` first (`expr.assign.1`).
        if op.kind() == K::EQ {
            let v = self.eval(&value)?;
            let p = self.place(&target)?;
            let old = self.load(&p);
            self.store(&p, Value::Moved);
            self.drop_value(old, true, range)?;
            self.store(&p, v);
        } else {
            let p = self.place(&target)?;
            let old = self.load(&p);
            let v = self.eval(&value)?;
            let new = arith(compound(op.kind()), old, v, range)?;
            self.store(&p, new);
        }
        Ok(Value::Unit)
    }

    /// The kind a type written in this function stands for.
    fn head_of(&self, t: &ast::Type) -> Head {
        let name = first_name(t.syntax());
        if let Some(h) = self.frame().types.get(&name)
            && matches!(t, ast::Type::Path(_))
        {
            return h.clone();
        }
        head_of_type(t)
    }

    /// The kinds the type parameters `names` get from the types written in `args`.
    fn turbofish(&self, args: Option<SyntaxNode>, names: &[String]) -> HashMap<String, Head> {
        let mut out = HashMap::new();
        if let Some(a) = args {
            let types = a
                .children()
                .filter(|c| c.kind() == K::TYPE_ARG)
                .filter_map(|c| c.children().find_map(ast::Type::cast));
            for (n, t) in names.iter().zip(types) {
                out.insert(n.clone(), self.head_of(&t));
            }
        }
        out
    }

    /// Finds the kinds of type parameters `names` from a value of type `t` (`v`), as far as
    /// the value shows them.
    fn infer_types(
        &self,
        t: &ast::Type,
        v: &Value,
        names: &[String],
        out: &mut HashMap<String, Head>,
    ) {
        match (t, v) {
            (_, Value::Moved) => {}
            (ast::Type::Ref(_), Value::Ref(p, _)) => {
                if let Some(inner) = t.syntax().children().find_map(ast::Type::cast) {
                    let target = self.load(p);
                    self.infer_types(&inner, &target, names, out);
                }
            }
            (ast::Type::Paren(_), v) => {
                if let Some(inner) = t.syntax().children().find_map(ast::Type::cast) {
                    self.infer_types(&inner, v, names, out);
                }
            }
            (ast::Type::Tuple(_), Value::Tuple(xs)) => {
                for (inner, x) in t.syntax().children().filter_map(ast::Type::cast).zip(xs) {
                    self.infer_types(&inner, x, names, out);
                }
            }
            (ast::Type::Path(_), v) => {
                let name = first_name(t.syntax());
                if names.contains(&name) {
                    out.entry(name).or_insert_with(|| Self::head(v));
                    return;
                }
                // A generic struct or enum: what its fields' values say about its arguments.
                let Value::Adt {
                    ty,
                    variant,
                    fields,
                } = v
                else {
                    return;
                };
                let Some(adt) = self.program.adts.get(ty) else {
                    return;
                };
                let Some(args) = t
                    .syntax()
                    .descendants()
                    .into_iter()
                    .find(|n| n.kind() == K::GENERIC_ARGS)
                else {
                    return;
                };
                let mut inner = HashMap::new();
                let info = self.program.variant(ty, variant);
                for (ft, fv) in info.types.iter().zip(fields) {
                    if let Some(ft) = ft {
                        self.infer_types(ft, fv, &adt.params, &mut inner);
                    }
                }
                let written = args
                    .children()
                    .filter(|c| c.kind() == K::TYPE_ARG)
                    .filter_map(|c| c.children().find_map(ast::Type::cast));
                for (p, w) in adt.params.iter().zip(written) {
                    let wn = first_name(w.syntax());
                    if names.contains(&wn)
                        && let Some(h) = inner.get(p)
                    {
                        out.entry(wn).or_insert_with(|| h.clone());
                    }
                }
            }
            _ => {}
        }
    }

    /// Calls method `m` (found for `self_head`) with `args`, the first being `self` if it has one.
    fn run_method(
        &mut self,
        m: Method,
        self_head: &Head,
        args: Vec<Value>,
        range: TextRange,
    ) -> Eval {
        match m {
            Method::Copy => match &args[0] {
                Value::Ref(p, _) => Ok(self.load(p)),
                v => Ok(v.clone()),
            },
            Method::Derived(tr) => self.derived(&tr, &args, range),
            Method::Code(f, imp) => {
                let mut types = HashMap::new();
                types.insert("Self".to_string(), self_head.clone());
                if let Some(k) = imp {
                    let i = &self.program.impls[k];
                    if let Some(first) = args.first() {
                        let self_value = self.target(first.clone());
                        self.infer_types(&i.self_ty, &self_value, &i.params, &mut types);
                    }
                }
                self.invoke(&f, args, types, range)
            }
        }
    }

    /// Calls trait `tr`'s method `name` for the kind of value its first argument (or, through a
    /// borrow, what it points to) is.
    fn call_trait(&mut self, tr: &str, name: &str, args: Vec<Value>, range: TextRange) -> Eval {
        let head = Self::head(&self.target(args[0].clone()));
        let m = self
            .program
            .trait_method(tr, &head, name)
            .expect("a checked trait call");
        self.run_method(m, &head, args, range)
    }

    /// The derived methods, built in and structural (`attr.derive.1`): `self` and `other` are
    /// borrows.
    fn derived(&mut self, tr: &str, args: &[Value], range: TextRange) -> Eval {
        let ptr = |v: &Value| match v {
            Value::Ref(p, _) => p.clone(),
            _ => unreachable!("a borrow"),
        };
        let a = ptr(&args[0]);
        let av = self.load(&a);
        let Value::Adt {
            ty,
            variant,
            fields,
        } = &av
        else {
            unreachable!("a struct or enum")
        };
        let field = |p: &Pointer, i: usize| {
            let mut q = p.clone();
            q.path.push(i);
            q
        };
        match tr {
            "Clone" => {
                let mut out = Vec::new();
                for i in 0..fields.len() {
                    let x = vec![Value::Ref(field(&a, i), false)];
                    out.push(self.call_trait("Clone", "clone", x, range)?);
                }
                Ok(Value::Adt {
                    ty: ty.clone(),
                    variant: variant.clone(),
                    fields: out,
                })
            }
            "PartialEq" => {
                let b = ptr(&args[1]);
                let Value::Adt { variant: bv, .. } = self.load(&b) else {
                    unreachable!("a struct or enum")
                };
                if *variant != bv {
                    return Ok(Value::Bool(false));
                }
                for i in 0..fields.len() {
                    let x = vec![
                        Value::Ref(field(&a, i), false),
                        Value::Ref(field(&b, i), false),
                    ];
                    if self.call_trait("PartialEq", "eq", x, range)? != Value::Bool(true) {
                        return Ok(Value::Bool(false));
                    }
                }
                Ok(Value::Bool(true))
            }
            _ => {
                // `PartialOrd` and `Ord`: by variant, in declaration order, then field by field.
                let partial = tr == "PartialOrd";
                let b = ptr(&args[1]);
                let Value::Adt { variant: bv, .. } = self.load(&b) else {
                    unreachable!("a struct or enum")
                };
                let order = |n: &str| {
                    self.program.adts[ty]
                        .variants
                        .iter()
                        .position(|v| v.name == n)
                        .expect("a variant")
                };
                let wrap = |o: &str| {
                    let ordering = Value::Adt {
                        ty: "Ordering".to_string(),
                        variant: o.to_string(),
                        fields: Vec::new(),
                    };
                    if partial {
                        Value::Adt {
                            ty: "Option".to_string(),
                            variant: "Some".to_string(),
                            fields: vec![ordering],
                        }
                    } else {
                        ordering
                    }
                };
                let (i, j) = (order(variant), order(&bv));
                if i != j {
                    return Ok(wrap(if i < j { "Less" } else { "Greater" }));
                }
                let method = if partial { "partial_cmp" } else { "cmp" };
                for k in 0..fields.len() {
                    let x = vec![
                        Value::Ref(field(&a, k), false),
                        Value::Ref(field(&b, k), false),
                    ];
                    let c = self.call_trait(tr, method, x, range)?;
                    if c != wrap("Equal") {
                        return Ok(c);
                    }
                }
                Ok(wrap("Equal"))
            }
        }
    }

    /// Runs function `f` with `args` and the kinds of its parent's type parameters `types`
    /// (`expr.call.1`). Its own type parameters' kinds come from the arguments.
    fn invoke(
        &mut self,
        f: &FnDef,
        args: Vec<Value>,
        mut types: HashMap<String, Head>,
        range: TextRange,
    ) -> Eval {
        if self.frames.len() >= MAX_DEPTH {
            return Err(trap(TrapClass::StackOverflow, range));
        }
        let params: Vec<ast::Param> = f
            .node
            .params()
            .map(|l| l.params().collect())
            .unwrap_or_default();
        let skip = usize::from(f.self_kind.is_some());
        for (p, v) in params.iter().zip(&args[skip..]) {
            if let Some(t) = p.ty() {
                self.infer_types(&t, v, &f.params, &mut types);
            }
        }
        let caller_user = self.frame().user;
        // The callee sees only its parameters, in a frame of its own.
        self.frames.push(Frame {
            vars: Vec::new(),
            types,
            user: f.user,
        });
        let mut args = args.into_iter();
        if f.self_kind.is_some() {
            let v = args.next().expect("`self`");
            self.vars_mut().push(("self".to_string(), v));
        }
        for (p, v) in params.iter().zip(args) {
            let pat = p.pat().expect("a parameter pattern");
            let slot = self.temp(v);
            let mut binds = Vec::new();
            let matched = self.matches(&pat, &Src::Place(slot, false), &mut binds);
            debug_assert!(matched, "parameter patterns are irrefutable");
            self.bind(binds);
        }
        let result = self.block(&f.node.body().expect("a body"));
        let result = match result {
            Ok(v) | Err(Flow::Return(v)) => Ok(v),
            Err(f) => Err(f),
        };
        // The parameters are dropped at the end of the call; a destructor's `self` only by its
        // fields.
        let result = if matches!(result, Err(Flow::Trap(..))) {
            result
        } else {
            let mut result = result;
            while let Some((name, v)) = self.vars_mut().pop() {
                let own = !(f.destructor && name == "self");
                if let Err(e) = self.drop_value(v, own, range) {
                    result = Err(e);
                    break;
                }
            }
            result
        };
        self.frames.pop();
        // A trap in the prelude's code is reported where the file's code called it.
        match result {
            Err(Flow::Trap(t, false)) if f.user => Err(Flow::Trap(t, true)),
            Err(Flow::Trap(mut t, false)) if caller_user => {
                t.range = range;
                Err(Flow::Trap(t, true))
            }
            r => r,
        }
    }

    /// A call of a function, a tuple struct or variant, a function of a type (`Type::f`,
    /// `T::f`), or a trait's method (`Trait::f`) (`expr.call.1`, `expr.struct.1`,
    /// `expr.path.assoc.1`).
    fn call(&mut self, node: &SyntaxNode, e: &ast::Expr, range: TextRange) -> Eval {
        let callee = &e.sub_exprs()[0];
        let args: Vec<ast::Expr> = node
            .child(K::ARG_LIST)
            .and_then(ast::ArgList::cast)
            .map(|l| l.args().filter_map(|a| a.expr()).collect())
            .unwrap_or_default();
        let segments = segments(callee.syntax());
        let local = matches!(segments.as_slice(), [one] if self.slot(one).is_some());
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
            && let Some((ty, info)) = self.ctor(&segments)
        {
            let variant = info.name.clone();
            let fields = self.eval_all(&args)?;
            return Ok(Value::Adt {
                ty,
                variant,
                fields,
            });
        }
        let turbofish = callee
            .syntax()
            .child(K::PATH)
            .and_then(|p| p.children().find(|c| c.kind() == K::GENERIC_ARGS));
        match segments.as_slice() {
            [name] => {
                let f = self.program.fns[name].clone();
                let types = self.turbofish(turbofish, &f.params);
                let values = self.eval_all(&args)?;
                self.invoke(&f, values, types, range)
            }
            [ty, name] => {
                let values = self.eval_all(&args)?;
                if self.program.traits.contains_key(ty) {
                    // `Trait::method(x, …)`: for the kind of value `x` is.
                    return self.call_trait(ty, name, values, range);
                }
                let head = match ty.as_str() {
                    "int" => Head::Int,
                    "bool" => Head::Bool,
                    t => match self.frame().types.get(t) {
                        Some(h) => h.clone(),
                        None => Head::Adt(t.to_string()),
                    },
                };
                let m = self
                    .program
                    .methods(&head, name)
                    .into_iter()
                    .next()
                    .expect("a checked function of a type");
                if let Method::Code(f, imp) = m {
                    let mut types = HashMap::new();
                    types.insert("Self".to_string(), head.clone());
                    if let Some(k) = imp
                        && f.self_kind.is_some()
                        && let Some(first) = values.first()
                    {
                        let i = &self.program.impls[k];
                        let v = self.target(first.clone());
                        self.infer_types(&i.self_ty, &v, &i.params, &mut types);
                    }
                    let own = callee
                        .syntax()
                        .child(K::PATH)
                        .and_then(|p| p.children().filter(|c| c.kind() == K::GENERIC_ARGS).last());
                    types.extend(self.turbofish(own, &f.params));
                    return self.invoke(&f, values, types, range);
                }
                self.run_method(m, &head, values, range)
            }
            _ => unreachable!("a checked call"),
        }
    }

    /// `receiver.name(args)`: the method is looked for on the receiver's value, then on a
    /// borrow of it, then on a `&mut` borrow of it, and then the same for what it points to if
    /// it is a borrow (`expr.method.1`).
    fn method_call(&mut self, node: &SyntaxNode, e: &ast::Expr, range: TextRange) -> Eval {
        let receiver = &e.sub_exprs()[0];
        let name = node
            .child_tokens()
            .find(|t| t.kind() == K::IDENT)
            .expect("a method name")
            .text()
            .to_string();
        let args: Vec<ast::Expr> = node
            .child(K::ARG_LIST)
            .and_then(ast::ArgList::cast)
            .map(|l| l.args().filter_map(|a| a.expr()).collect())
            .unwrap_or_default();
        let (place, indirect) = match self.place_of(receiver)? {
            Some(p) => p,
            None => {
                let v = self.eval(receiver)?;
                (self.temp(v), false)
            }
        };
        let value = self.load(&place);
        // The steps: the receiver itself, then what it points to.
        let mut steps = vec![(place.clone(), value.clone(), indirect)];
        if let Value::Ref(q, _) = &value {
            steps.push((q.clone(), self.load(q), true));
        }
        let mut found = None;
        'search: for (p, v, ind) in &steps {
            let head = Self::head(v);
            // By value: `self` of the receiver's kind, or `&self` / `&mut self` of what a borrow
            // points to.
            let mut by_value = Vec::new();
            for m in self.program.methods(&head, &name) {
                if m.self_kind() == Some(SelfKind::Value) {
                    by_value.push((m, head.clone()));
                }
            }
            if let Value::Ref(q, mutable) = v {
                let inner = Self::head(&self.load(q));
                for m in self.program.methods(&inner, &name) {
                    match m.self_kind() {
                        Some(SelfKind::Ref) => by_value.push((m, inner.clone())),
                        Some(SelfKind::RefMut) if *mutable => by_value.push((m, inner.clone())),
                        _ => {}
                    }
                }
            }
            if let Some((m, h)) = by_value.into_iter().next() {
                let recv = if matches!(m.self_kind(), Some(SelfKind::Value)) {
                    self.take(p, *ind)
                } else {
                    v.clone()
                };
                found = Some((m, h, recv));
                break 'search;
            }
            for (kind, mutable) in [(SelfKind::Ref, false), (SelfKind::RefMut, true)] {
                if let Some(m) = self
                    .program
                    .methods(&head, &name)
                    .into_iter()
                    .find(|m| m.self_kind() == Some(kind))
                {
                    found = Some((m, head.clone(), Value::Ref(p.clone(), mutable)));
                    break 'search;
                }
            }
        }
        let (m, head, recv) = found.expect("a checked method call");
        let mut values = vec![recv];
        values.extend(self.eval_all(&args)?);
        if let Method::Code(f, imp) = &m {
            let mut types = HashMap::new();
            types.insert("Self".to_string(), head.clone());
            if let Some(k) = *imp {
                let i = &self.program.impls[k];
                let sv = self.target(values[0].clone());
                self.infer_types(&i.self_ty, &sv, &i.params, &mut types);
            }
            types.extend(self.turbofish(node.child(K::GENERIC_ARGS), &f.params));
            let f = f.clone();
            return self.invoke(&f, values, types, range);
        }
        self.run_method(m, &head, values, range)
    }

    // Patterns.

    /// The value a source holds (looking through a borrow, for patterns that test it).
    fn value_of(&self, src: &Src) -> Value {
        match src {
            Src::Val(v) => self.target(v.clone()),
            Src::Place(p, _) => self.target(self.load(p)),
            Src::At(ptr, _) => self.load(ptr),
        }
    }

    /// What a name bound to `src` holds: the value (moved out of its place), or a borrow of the
    /// place.
    fn bound(&self, src: &Src) -> (Value, Option<Pointer>) {
        match src {
            Src::Val(v) => (v.clone(), None),
            Src::Place(p, indirect) => (self.load(p), (!indirect).then(|| p.clone())),
            Src::At(ptr, m) => (Value::Ref(ptr.clone(), *m), None),
        }
    }

    /// The source a pattern that tests a value sees: matching a borrow tests the place it
    /// points to, and names inside bind borrows of its parts (`pat.mode.1`).
    fn enter(&self, src: &Src) -> Src {
        match src {
            Src::Val(Value::Ref(ptr, m)) | Src::At(ptr, m) => Src::At(ptr.clone(), *m),
            Src::Place(p, indirect) => match self.load(p) {
                Value::Ref(ptr, m) => Src::At(ptr, m),
                _ => Src::Place(p.clone(), *indirect),
            },
            Src::Val(v) => Src::Val(v.clone()),
        }
    }

    /// Part `i` of the value a source holds.
    fn part(&self, src: &Src, i: usize) -> Src {
        let sub = |p: &Pointer| {
            let mut q = p.clone();
            q.path.push(i);
            q
        };
        match src {
            Src::Val(Value::Tuple(xs) | Value::Adt { fields: xs, .. }) => Src::Val(xs[i].clone()),
            Src::Val(_) => unreachable!("a checked pattern"),
            Src::Place(p, indirect) => Src::Place(sub(p), *indirect),
            Src::At(ptr, m) => Src::At(sub(ptr), *m),
        }
    }

    fn bind_to(&self, name: String, src: &Src, binds: &mut Vec<Bind>) {
        let (v, from) = self.bound(src);
        binds.push((name, v, from));
    }

    /// Whether `src` matches `p`; if it does, `binds` gets the names it binds (`pat.1`).
    fn matches(&self, p: &ast::Pat, src: &Src, binds: &mut Vec<Bind>) -> bool {
        let node = p.syntax();
        match p {
            ast::Pat::Wildcard(_) => true,
            ast::Pat::Literal(_) => literal_pat(node) == self.value_of(src),
            ast::Pat::Range(_) => {
                let bounds: Vec<Value> = node
                    .children()
                    .filter(|c| c.kind() == K::LITERAL_PAT)
                    .map(|b| literal_pat(&b))
                    .collect();
                let (Value::Int(x), Some(Value::Int(lo))) = (self.value_of(src), bounds.first())
                else {
                    unreachable!("a checked range pattern");
                };
                let below = match bounds.get(1) {
                    None => true,
                    Some(Value::Int(hi)) if node.token(K::DOT_DOT_EQ).is_some() => x <= *hi,
                    Some(Value::Int(hi)) => x < *hi,
                    Some(_) => unreachable!("an integer bound"),
                };
                *lo <= x && below
            }
            ast::Pat::Ident(_) => {
                if let Some(sub) = node.children().find_map(ast::Pat::cast)
                    && !self.matches(&sub, src, binds)
                {
                    return false;
                }
                self.bind_to(name_of(node), src, binds);
                true
            }
            ast::Pat::At(_) => {
                let sub = node.children().find_map(ast::Pat::cast).expect("a pattern");
                if !self.matches(&sub, src, binds) {
                    return false;
                }
                self.bind_to(segments(node)[0].clone(), src, binds);
                true
            }
            ast::Pat::Ref(_) => {
                // `&p` copies what the borrow points to.
                let sub = node.children().find_map(ast::Pat::cast).expect("a pattern");
                self.matches(&sub, &Src::Val(self.value_of(src)), binds)
            }
            ast::Pat::Path(_) => {
                let segments = segments(node);
                match self.ctor(&segments) {
                    Some((ty, info)) => is_ctor(&self.value_of(src), &ty, &info.name),
                    None => {
                        self.bind_to(segments[0].clone(), src, binds);
                        true
                    }
                }
            }
            ast::Pat::TupleStruct(_) => {
                let (ty, info) = self.ctor(&segments(node)).expect("a checked pattern");
                if !is_ctor(&self.value_of(src), &ty, &info.name) {
                    return false;
                }
                let n = info.fields.len();
                self.positional(node, &self.enter(src), n, binds)
            }
            ast::Pat::Struct(_) => {
                let (ty, info) = self.ctor(&segments(node)).expect("a checked pattern");
                if !is_ctor(&self.value_of(src), &ty, &info.name) {
                    return false;
                }
                let info = info.clone();
                let src = self.enter(src);
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
                    let part = self.part(&src, i);
                    match f.children().find_map(ast::Pat::cast) {
                        Some(sub) => {
                            if !self.matches(&sub, &part, binds) {
                                return false;
                            }
                        }
                        None => self.bind_to(name, &part, binds),
                    }
                }
                true
            }
            ast::Pat::Tuple(_) => {
                let subs: Vec<ast::Pat> = node.children().filter_map(ast::Pat::cast).collect();
                if subs.is_empty() {
                    return self.value_of(src) == Value::Unit;
                }
                if subs.len() == 1
                    && node.token(K::COMMA).is_none()
                    && !matches!(subs[0], ast::Pat::Rest(_))
                {
                    return self.matches(&subs[0], src, binds);
                }
                let Value::Tuple(xs) = self.value_of(src) else {
                    unreachable!("a checked tuple pattern")
                };
                self.positional(node, &self.enter(src), xs.len(), binds)
            }
            ast::Pat::Or(_) => node.children().filter_map(ast::Pat::cast).any(|alt| {
                let mut tried = Vec::new();
                let ok = self.matches(&alt, src, &mut tried);
                if ok {
                    binds.extend(tried);
                }
                ok
            }),
            _ => unreachable!("a checked pattern of slices S0 to S3"),
        }
    }

    /// The positional sub-patterns of `node` (with an optional `..`) against the `len` parts of
    /// `src`.
    fn positional(&self, node: &SyntaxNode, src: &Src, len: usize, binds: &mut Vec<Bind>) -> bool {
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
                Some(r) if i >= r => len - (n - i),
                _ => i,
            };
            self.matches(p, &self.part(src, at), binds)
        })
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
                        .find(|t| matches!(t.kind(), K::IDENT | K::SELF_TYPE_KW | K::SELF_KW))
                        .map(|t| t.text().to_string())
                })
                .collect()
        })
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

/// Comparisons of `int`s, `bool`s (`false` before `true`) and `()`.
fn compare(k: K) -> Option<Comparison> {
    fn ints(a: &Value, b: &Value) -> (i64, i64) {
        match (a, b) {
            (Value::Int(a), Value::Int(b)) => (*a, *b),
            (Value::Bool(a), Value::Bool(b)) => (i64::from(*a), i64::from(*b)),
            _ => unreachable!("ordering compares integers or booleans"),
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
