//! `moddil-wasm`: the WASM backend (M1). The native lowering builds Cranelift IR into a
//! [`module::WasmModule`], which translates each function to WebAssembly with our own emitter (on
//! `wasm-encoder`, as `specs/compiler.md` decides), its control flow structured from the
//! dominator tree; the runtime is `moddil-rt` built for wasm32, sharing the module's memory; and
//! the module runs in Node. See `crates/wasm/README.md`.

mod cfg;
mod clif;
mod compile;
pub mod module;
mod run;
mod runtime;

pub use compile::{Compiled, compile};
pub use run::{Observed, TestRuns, Wasm, host_available, run_main, run_tests};

/// This crate's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests;
