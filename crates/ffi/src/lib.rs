//! C interop for Moddil's runtimes (`reference/2027/interop.md`).
//!
//! - [`build`]: a mod's C sources (its manifest's `[c]` table) compiled by the system's C
//!   compiler, through the `cc` crate, into a static library (for `moddil build`) and a shared one
//!   (for the JIT and the MIR interpreter), cached by what went into them.
//! - [`load`] and [`lookup`]: the C libraries a run has, and the address of a C function by its
//!   symbol, in them or in the C runtime.
//! - [`call`]: a call of a C function by its address, with arguments as words, through a
//!   trampoline per signature built with Cranelift: how the MIR interpreter calls C.

#![allow(unsafe_code)]

pub mod abi;
mod build;
mod call;
mod libs;

pub use build::{Built, CSpec, build};
pub use call::{CType, c_signature, call, callback, with_callbacks};
pub use libs::{load, lookup};
