//! Read-only review state: a personal triage checklist over a large diff. No
//! tree mutation — verdicts and notes only record what the human has vetted.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::domain::session::SessionId;

/// Content-addressed handle to a hunk. Anchoring verdicts/notes here lets them
/// re-attach across a live re-diff even as line numbers shift.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HunkRef {
    pub path: PathBuf,
    pub fingerprint: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum HunkVerdict {
    #[default]
    Unreviewed,
    Approved,
    NeedsAttention,
}

/// Persisted per (repo, base). NOTE: the `HunkRef`-keyed maps need a string key
/// encoding before they can be written to TOML/JSON — handled in Phase 1.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewState {
    pub verdicts: HashMap<HunkRef, HunkVerdict>,
    pub notes: HashMap<HunkRef, String>,
    pub collapsed: HashMap<PathBuf, bool>,
    pub last_session: Option<SessionId>,
}
