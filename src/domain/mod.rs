//! Pure domain types — the contract every other module anchors to. No I/O lives
//! here. These types are frozen in Phase 0; later phases fill in the producers.

pub mod diff;
pub mod ids;
pub mod review;
pub mod session;

pub use diff::*;
pub use review::*;
pub use session::*;

use serde::{Deserialize, Serialize};

/// Unix-epoch milliseconds. A tiny newtype keeps timestamps deterministic in
/// tests and avoids a time-crate dependency until Phase 2 needs ISO-8601.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Timestamp(pub i64);

impl Timestamp {
    pub fn now() -> Self {
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        Self(ms)
    }

    pub fn from_millis(ms: i64) -> Self {
        Self(ms)
    }
}
