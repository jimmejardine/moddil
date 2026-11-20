//! The conformance runner: runs `conformance/` tests on every implementation and reports any
//! disagreement.
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
        assert_eq!(moddil_refinterp::VERSION, super::VERSION);
        assert_eq!(moddil_interp::VERSION, super::VERSION);
    }
}
