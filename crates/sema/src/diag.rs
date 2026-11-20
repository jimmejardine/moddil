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
    /// An assignment to a local not declared `mut`.
    E0301 = "assignment to an immutable local",
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
