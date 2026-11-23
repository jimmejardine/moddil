//! `moddil-wasm`: the WASM backend (M1). MIR is lowered to a core WebAssembly module with our own
//! emitter (on `wasm-encoder`, as `specs/compiler.md` decides), its control flow structured from
//! the dominator tree, and the module run in Node. See `crates/wasm/README.md`.

mod cfg;
mod emit;
mod run;

pub use emit::{Compiled, OUT, STACK_TOP, Site, compile};
pub use run::{Observed, TestRuns, Wasm, host_available, run_main, run_tests};

/// This crate's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests;
