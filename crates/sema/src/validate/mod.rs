//! Checks on the finished item tree, run after collecting: each is a function of the tree
//! that returns what it found, which the collector reports.
//!
//! - [`types`]: structs and enums that contain themselves, `Copy` types, derived impls;
//! - [`impls`]: impls against their traits, and impls that overlap;
//! - [`entry`]: entry points, which cannot fail (R3);
//! - [`effects`]: the effects signatures list, and `dyn` types in them.

use moddil_syntax::TextRange;

use crate::Diagnostic;
use crate::items::Src;

mod effects;
mod entry;
mod impls;
mod types;

pub(crate) use effects::signature_effects;
pub(crate) use entry::entry_points;
pub(crate) use impls::{check_impls, overlap};
pub(crate) use types::{copy_types, derived_fields, recursive_types};

/// A problem found, with the source it is in and, for a derived item, the `derive` attribute
/// it comes from.
pub(crate) type Found = (Src, Option<TextRange>, Diagnostic);
