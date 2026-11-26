//! The run's hash seed (`coll.hash.2`): what keys the `core` mod's `SipHasher`, so a `HashMap`'s
//! order is its hash order under that seed. A run draws one from the OS the first time a hash
//! needs it, unless the embedder set one (`moddil test` picks and prints it) or `main` installs
//! the deterministic runtime, whose seed it is.

use std::sync::atomic::{AtomicU64, Ordering};

/// No seed yet: the first hash draws one.
const UNSET: u64 = u64::MAX;

static SEED: AtomicU64 = AtomicU64::new(UNSET);

/// Sets the run's hash seed, or (`None`) leaves it to be drawn from the OS.
pub fn set_hash_seed(seed: Option<u64>) {
    SEED.store(seed.map_or(UNSET, |s| s & !(1 << 63)), Ordering::Relaxed);
}

/// The run's hash seed (`__hash_seed`), drawn from the OS the first time.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_hash_seed")]
pub(crate) extern "C" fn hash_seed() -> i64 {
    let s = SEED.load(Ordering::Relaxed);
    if s != UNSET {
        return s as i64;
    }
    let drawn = crate::os::random() as u64 & !(1 << 63);
    match SEED.compare_exchange(UNSET, drawn, Ordering::Relaxed, Ordering::Relaxed) {
        Ok(_) => drawn as i64,
        Err(other) => other as i64,
    }
}
