//! Content-addressed hashing for hunks.
//!
//! Phase 0 stub: a plain hash over the header + line texts. Phase 1 replaces it
//! with a normalized fingerprint (insensitive to surrounding line-number shifts)
//! so verdicts re-attach across a live re-diff.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::domain::diff::Hunk;

pub fn fingerprint(hunk: &Hunk) -> u64 {
    let mut hasher = DefaultHasher::new();
    hunk.header.hash(&mut hasher);
    for line in &hunk.lines {
        (line.kind as u8).hash(&mut hasher);
        line.text.hash(&mut hasher);
    }
    hasher.finish()
}
