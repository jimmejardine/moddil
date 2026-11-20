//! The reference interpreter: the executable form of the language reference, running the
//! canonical AST directly. It depends only on `moddil-syntax`; a test enforces this.
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

    #[test]
    fn dependencies_are_wired() {
        assert_eq!(moddil_syntax::VERSION, super::VERSION);
    }
}
