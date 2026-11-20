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
    /// An assignment to a local not declared `mut`.
    E0301 = "assignment to an immutable local",
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
    /// Secondary ranges with their messages.
    pub labels: Vec<(TextRange, String)>,
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
        }
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
