//! Diagnostics and their codes.
//!
//! Codes are part of conformance (`reference/2027/diagnostics.md`); message wording is not. Codes
//! are grouped in blocks of a hundred by area: E00xx syntax, E01xx names, E02xx types, E03xx
//! ownership and mutability, E04xx traits and generics, E05xx errors and `fail`, E06xx effects.
//! Codes starting with `I` are this implementation's own (such as "not supported yet") and are not
//! part of conformance.

use std::fmt;

use moddil_syntax::TextRange;

/// A diagnostic code: `E0201`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Code(&'static str);

impl Code {
    /// The code's text: `"E0201"`.
    pub fn as_str(self) -> &'static str {
        self.0
    }

    /// The one-line description from the codes table.
    pub fn description(self) -> &'static str {
        CODES
            .iter()
            .find(|(c, _)| *c == self)
            .map_or("", |(_, d)| d)
    }
}

impl fmt::Debug for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

macro_rules! codes {
    ($($(#[$doc:meta])* $name:ident = $desc:literal,)*) => {
        impl Code {
            $(
                $(#[$doc])*
                pub const $name: Code = Code(stringify!($name));
            )*
        }

        /// Every code with its description, in order.
        pub const CODES: &[(Code, &str)] = &[$((Code::$name, $desc),)*];
    };
}

codes! {
    /// A syntax error.
    E0001 = "syntax error",
    /// A comparison chain that mixes directions, or chains `==` or `!=`.
    E0002 = "comparison chain in more than one direction",
    /// A name that does not resolve to a value or function.
    E0101 = "unresolved name",
    /// A name that does not resolve to a type.
    E0102 = "unresolved type",
    /// Two items with the same name.
    E0103 = "duplicate definition",
    /// Two parameters with the same name.
    E0104 = "duplicate parameter",
    /// A name bound in some alternatives of an or-pattern but not all.
    E0105 = "binding missing from an alternative",
    /// A name bound twice in one pattern.
    E0106 = "name bound twice in a pattern",
    /// A field that the type does not have.
    E0107 = "no such field",
    /// A field given twice in a struct expression or pattern.
    E0108 = "field given twice",
    /// `break` or `continue` outside a loop.
    E0109 = "`break` or `continue` outside a loop",
    /// A label that names no enclosing loop.
    E0110 = "unresolved label",
    /// A struct or variant written in a form it does not have (`Point` for a struct with named
    /// fields, `Shape::Circle { … }` for a variant with positional ones).
    E0111 = "struct or variant in the wrong form",
    /// A built-in such as `assert` used other than by calling it.
    E0112 = "built-in used as a value",
    /// A doc comment's link to code (`[[Account::open]]`) that names nothing.
    E0113 = "unresolved doc link",
    /// A type other than the one expected.
    E0201 = "mismatched types",
    /// A call with the wrong number of arguments.
    E0202 = "wrong number of arguments",
    /// A call of something that is not a function.
    E0203 = "not a function",
    /// An integer literal that does not fit its type.
    E0204 = "literal out of range",
    /// An operator applied to operands it is not defined for.
    E0205 = "operator not defined for these types",
    /// An assignment to something that is not a place.
    E0206 = "invalid assignment target",
    /// A struct expression that leaves out fields, or a pattern that leaves them out without `..`.
    E0207 = "missing fields",
    /// A `match` whose arms do not cover every value.
    E0208 = "non-exhaustive patterns",
    /// A pattern that can fail where one that cannot is needed (`let` without `else`, a
    /// parameter).
    E0209 = "refutable pattern",
    /// The `else` block of `let … else` can finish normally.
    E0210 = "`let … else` that does not diverge",
    /// A struct or enum that contains itself without indirection.
    E0211 = "recursive type of infinite size",
    /// `break` with a value out of a loop other than `loop`.
    E0212 = "`break` with a value out of `while`",
    /// A tuple-struct pattern or constructor with the wrong number of fields.
    E0213 = "wrong number of fields",
    /// A range pattern that matches nothing, such as `5..=1`.
    E0214 = "empty range pattern",
    /// A named argument with no parameter of its name, given twice, or before a positional one.
    E0215 = "invalid named argument",
    /// A default on a method of an impl of a trait, which inherits the trait's.
    E0216 = "default on an impl's method",
    /// A `select` arm that does not wait on an operation a `select` can wait on.
    E0217 = "invalid `select` arm",
    /// A raw pointer dereferenced outside an `unsafe` block.
    E0218 = "dereference of a raw pointer outside `unsafe`",
    /// An `unsafe` block in a mod whose manifest does not declare the `unsafe` capability.
    E0219 = "`unsafe` without the capability",
    /// A function of `core::os` named in a mod whose manifest does not declare the `os`
    /// capability.
    E0220 = "`core::os` or `core::gpu` without the capability",
    /// A `#[kernel]` that is not a free function taking `&P` and `&mut Tile`, or that needs
    /// handlers, can fail, suspends or spawns, or holds what a GPU does not have.
    E0221 = "invalid kernel",
    /// A C function or an `#[export]` with a type C does not have, or with effects, errors or
    /// suspension; an `extern` block that is not `extern "C"`; a C function with a body.
    E0222 = "invalid C function or export",
    /// An assignment to, or a `&mut` borrow of, a local not declared `mut`.
    E0301 = "mutation of an immutable local",
    /// An assignment through, or a `&mut` borrow through, a shared borrow.
    E0302 = "mutation through a shared borrow",
    /// A use of a value after it was moved.
    E0303 = "use of a moved value",
    /// A move of a value out of a borrow, which does not own it.
    E0304 = "move out of a borrow",
    /// A borrow that conflicts with another borrow still in use.
    E0305 = "conflicting borrows",
    /// An assignment to, or a move of, a value while it is borrowed.
    E0306 = "assignment to or move of a borrowed value",
    /// A use of a value while it is borrowed with `&mut`.
    E0307 = "use of a value borrowed with `&mut`",
    /// A borrow used after the local it borrows has gone, or returned from where it was made.
    E0308 = "borrow outlives its value",
    /// A borrow stored in a struct, enum or tuple, or a borrow of a borrow.
    E0309 = "borrow stored in data",
    /// A returned borrow that does not come from a borrowed parameter.
    E0310 = "returned borrow without a source",
    /// `Copy` on a type that is not plain `Copy` data, or without `Clone`.
    E0311 = "type cannot be `Copy`",
    /// A `Copy` type larger than 64 bytes.
    E0312 = "`Copy` type over the size cap",
    /// `return`, `break` or `continue` out of a `defer` block.
    E0313 = "leaving a `defer` block",
    /// A move out of a field of a value whose type has a destructor.
    E0314 = "move out of a type with a destructor",
    /// Moving a value that is not `Copy` out of an element of an array or slice.
    E0315 = "move out of an element",
    /// Text sliced with a position found in other text (`&b[a.find(' ')?..]`).
    E0316 = "position of other text",
    /// A name used as a trait that is not one.
    E0401 = "not a trait",
    /// A type that does not implement a trait it must.
    E0402 = "unsatisfied bound",
    /// A method or associated function that the type does not have.
    E0403 = "no such method",
    /// A generic item given the wrong number of type arguments.
    E0404 = "wrong number of type arguments",
    /// A type that cannot be inferred and must be written.
    E0405 = "type annotations needed",
    /// Two impls of a trait that apply to the same type.
    E0406 = "conflicting impls",
    /// An impl that leaves out a method or associated type of its trait.
    E0407 = "missing item in impl",
    /// An impl that defines something its trait does not have.
    E0408 = "item not in trait",
    /// An impl's method whose signature differs from the trait's.
    E0409 = "impl method does not match the trait",
    /// A method that several traits give the type.
    E0410 = "ambiguous method",
    /// A destructor for a type that is not the impl's own, for only some of its type arguments,
    /// with bounds, or on a `Copy` type.
    E0411 = "invalid destructor",
    /// A call of a destructor.
    E0412 = "explicit call of a destructor",
    /// An associated type or function that does not exist.
    E0413 = "no such associated item",
    /// Methods added to a type not defined here.
    E0414 = "inherent impl for a foreign type",
    /// An impl's type parameter that neither the type nor the trait uses.
    E0415 = "unconstrained type parameter",
    /// `#[derive(Default)]` on an enum with no `#[default]` unit variant.
    E0416 = "no default variant",
    /// `.clone()` written on an `Rc` or a `Weak`, which is `.share()` (`rc.clone.1`).
    E0417 = "an rc cloned",
    /// An error with nowhere to go: a fallible call not marked with `?`, `?!` or `try`, or an
    /// error where nothing can fail (a function without `with fail`, `main`, a `defer` block, a
    /// destructor).
    E0501 = "unhandled error",
    /// An error that the function's `with fail(…)` does not list.
    E0502 = "error not in the declared set",
    /// A `Result` dropped without being used or `discard`ed.
    E0503 = "unused `Result`",
    /// `?` or `?!` on something that cannot fail.
    E0504 = "nothing to propagate",
    /// A `catch` arm for an error that cannot occur, or a `catch` with nothing to catch.
    E0505 = "error that cannot occur",
    /// `?` on an `Option` where absence cannot propagate, or `?`s on both `Option` and errors in
    /// one `try` block.
    E0506 = "absence that cannot propagate",
    /// An entry point (`main`) that declares errors.
    E0507 = "entry point with errors",
    /// A type in an error set that is not a struct or enum, or a second instance of one.
    E0508 = "not an error type",
    /// Something after `with` that is not an effect: a trait that cannot be used as `&dyn Trait`
    /// with methods taking `&self`, or a type that is not a struct or enum of this program.
    E0601 = "not an effect",
    /// An entry point (`main`, a test) that needs an effect no `with` installs a handler for.
    E0602 = "no handler",
    /// A function whose effects are written out (`pub`, a trait's method or its impl's) that
    /// needs an effect it does not list.
    E0603 = "effect not in the signature",
    /// A trait's method, or a method of an impl of a trait, that lists an effect other than
    /// `fail`.
    E0604 = "effect on a trait method",
    /// `dyn` used wrongly: not behind a borrow, of a trait that is not dyn-compatible, or of
    /// something that is not one trait.
    E0605 = "invalid `dyn` type",
    /// An effect listed twice in one signature or one `with`.
    E0606 = "effect listed twice",
    /// Effects inferred without end (a generic function calling itself with ever larger types).
    E0607 = "effects do not settle",
    /// A parameter of `main` that is not a root capability, or a root capability taken twice.
    E0608 = "invalid parameter of `main`",
    /// A handler's effect in the bound of a closure parameter, whose needs are met where the
    /// closure is written.
    E0609 = "need in a closure bound",
    /// A task spawned into a scope opened outside a `with` whose handler it uses, so that it
    /// could outlive the handler.
    E0610 = "task outliving its handler",
    /// A runtime installed other than by `main`'s outermost `with`, once (`task.runtime.1`).
    E0611 = "runtime installed outside `main`",
    /// A closure whose kind is weaker than a bound needs: it changes or moves what it captures,
    /// where `Fn` or `FnMut` is needed.
    E0801 = "closure of the wrong kind",
    /// A stored closure (`Box<dyn Fn…>`) that captures a borrow, or borrows a handler.
    E0802 = "stored closure capturing a borrow",
    /// Compile-time code calling a function that is not `const fn`.
    E1001 = "non-const call in compile-time code",
    /// Compile-time code with an effect or an error.
    E1002 = "effect in compile-time code",
    /// A const whose type holds a borrow that is not of text.
    E1003 = "const holding a borrow",
    /// Compile-time evaluation that traps or runs out of fuel.
    E1004 = "compile-time evaluation failed",
    /// An `#[around(…)]` that names no function, nor calls one.
    E1005 = "invalid wrapper",
    /// A derive function not written as one (`const fn f(t: type) -> Impl<Tr>`, its body one
    /// `impl Tr for t { … }`), a `#[derive_by]` naming none, or a derive function called.
    E1006 = "invalid derive function",
    /// A template's `inline for`, `if const` or reflected field used where it cannot be.
    E1007 = "invalid template",
    /// A const item whose value cannot be static data: it holds a closure, a `dyn` value, or a C
    /// pointer or function pointer.
    E1008 = "const that cannot be static data",
    /// A const item that is not `Copy` used by value: it is static data, only borrowed.
    E1009 = "move of a const",
    /// An error compile-time code reported with `compile_error`, where it said.
    E1010 = "error from compile-time code",
    /// A tagged literal whose tag is not a `#[tag]` function in scope, or a `#[tag]` that is not
    /// a `const fn` taking one `&str` or `&[&str]`.
    E1011 = "invalid tag",
    /// A tagged literal whose fields cannot be (`comptime.tag.fields.2`): its tag is in the
    /// literal's own mod, or its fields' names are not distinct names.
    E1012 = "invalid tag fields",
    /// A literal that its refined type's predicate does not hold for.
    E1101 = "literal outside its refinement",
    /// A contract that is not pure: it has an effect, an error or an assignment.
    E1102 = "impure contract",
    /// `old(e)` of a value that is not `Copy`, or outside an `ensures`.
    E1103 = "invalid `old`",
    /// A type with no size of its own (`str`) used by value.
    E0901 = "unsized type",
    /// A text literal that is not well formed: an unknown escape, a block indented wrongly.
    E0902 = "invalid literal",
    /// A name another module may not name: private to its module, or `pub(mod)` and in
    /// another mod.
    E0701 = "not visible here",
    /// A `use` that names nothing, a glob import, or a declared module.
    E0702 = "invalid import",
    /// A `pub` item whose signature names something less visible than itself.
    E0703 = "private type in a public signature",
    /// A manifest that cannot be read, or names files or mods that are not there.
    E0704 = "invalid manifest",
    /// Mods that depend on each other in a cycle.
    E0705 = "dependency cycle",
    /// A `match` arm no value can reach.
    W0201 = "unreachable pattern",
    /// Valid Moddil that this implementation does not support yet.
    I0001 = "not supported yet",
}

/// How serious a diagnostic is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Severity {
    /// The program is rejected.
    Error,
    /// The program is accepted, but something is likely wrong.
    Warning,
}

/// A diagnostic: a code, a primary range, a message and secondary labels.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Diagnostic {
    /// The code.
    pub code: Code,
    /// How serious it is.
    pub severity: Severity,
    /// The primary range.
    pub range: TextRange,
    /// The message.
    pub message: String,
    /// Secondary ranges with their messages, in the same file.
    pub labels: Vec<(TextRange, String)>,
    /// The file it is in, by its index in the program.
    pub file: u32,
}

impl Diagnostic {
    /// An error.
    pub fn error(code: Code, range: TextRange, message: impl Into<String>) -> Self {
        Diagnostic {
            code,
            severity: Severity::Error,
            range,
            message: message.into(),
            labels: Vec::new(),
            file: 0,
        }
    }

    /// The same diagnostic, in file `file`.
    #[must_use]
    pub fn in_file(mut self, file: u32) -> Self {
        self.file = file;
        self
    }

    /// A warning.
    pub fn warning(code: Code, range: TextRange, message: impl Into<String>) -> Self {
        Diagnostic {
            severity: Severity::Warning,
            ..Diagnostic::error(code, range, message)
        }
    }

    /// "not supported yet" for `what`.
    pub fn unsupported(range: TextRange, what: &str) -> Self {
        Diagnostic::error(Code::I0001, range, format!("{what} is not supported yet"))
    }

    /// The same diagnostic with a secondary label.
    #[must_use]
    pub fn with_label(mut self, range: TextRange, message: impl Into<String>) -> Self {
        self.labels.push((range, message.into()));
        self
    }
}
