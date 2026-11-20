//! Moddil's mid-level IR (MIR): places and a control-flow graph per item, its phases, and the
//! ownership checking done on it (`specs/mir.md`).
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
        assert_eq!(moddil_sema::VERSION, super::VERSION);
    }
}
