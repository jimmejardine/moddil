//! Owns `reference/grammar/moddil.ebnf`: LL(k) and ambiguity checks, and generated grammar tests,
//! tree-sitter and constrained-decoding grammars.
//!
//! See this crate's `README.md`. No language code yet: it arrives with slice S0.

/// This crate's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn version_has_three_parts() {
        assert_eq!(super::VERSION.split('.').count(), 3);
    }
}
