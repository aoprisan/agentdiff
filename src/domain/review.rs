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

/// Persisted per (repo, base). The `HunkRef`-keyed maps cannot be written to
/// TOML directly (TOML has no struct keys), so persistence goes through the
/// flat [`StoredReview`] DTO via [`ReviewState::to_toml`] / [`from_toml`]. The
/// derive stays for in-memory/JSON use but is never used for the TOML round-trip.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewState {
    pub verdicts: HashMap<HunkRef, HunkVerdict>,
    pub notes: HashMap<HunkRef, String>,
    pub collapsed: HashMap<PathBuf, bool>,
    pub last_session: Option<SessionId>,
}

impl ReviewState {
    /// Verdict for a hunk, defaulting to `Unreviewed` when none is recorded.
    pub fn verdict(&self, href: &HunkRef) -> HunkVerdict {
        self.verdicts.get(href).copied().unwrap_or_default()
    }

    /// Record (or clear) a verdict. `Unreviewed` removes the entry entirely so it
    /// is not persisted.
    pub fn set_verdict(&mut self, href: HunkRef, verdict: HunkVerdict) {
        match verdict {
            HunkVerdict::Unreviewed => {
                self.verdicts.remove(&href);
            }
            _ => {
                self.verdicts.insert(href, verdict);
            }
        }
    }

    /// Serialize to human-diffable TOML via the flat DTO. Pure (no filesystem).
    pub fn to_toml(&self) -> std::result::Result<String, toml::ser::Error> {
        toml::to_string_pretty(&StoredReview::from_state(self))
    }

    /// Parse from the TOML DTO. Pure (no filesystem).
    pub fn from_toml(s: &str) -> std::result::Result<Self, toml::de::Error> {
        Ok(toml::from_str::<StoredReview>(s)?.into_state())
    }
}

// --- On-disk representation -------------------------------------------------
//
// TOML keys must be strings, so the `HunkRef`-keyed maps are flattened into
// arrays of tables. This is also far more human-diffable than an encoded map key.

#[derive(Debug, Default, Serialize, Deserialize)]
struct StoredReview {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    last_session: Option<String>,
    #[serde(rename = "verdict", default, skip_serializing_if = "Vec::is_empty")]
    verdicts: Vec<StoredVerdict>,
    #[serde(rename = "note", default, skip_serializing_if = "Vec::is_empty")]
    notes: Vec<StoredNote>,
    #[serde(rename = "collapsed", default, skip_serializing_if = "Vec::is_empty")]
    collapsed: Vec<StoredCollapse>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredVerdict {
    path: PathBuf,
    fingerprint: u64,
    verdict: HunkVerdict,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredNote {
    path: PathBuf,
    fingerprint: u64,
    text: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredCollapse {
    path: PathBuf,
    collapsed: bool,
}

impl StoredReview {
    fn from_state(state: &ReviewState) -> Self {
        let mut verdicts: Vec<_> = state
            .verdicts
            .iter()
            .filter(|(_, v)| **v != HunkVerdict::Unreviewed)
            .map(|(href, verdict)| StoredVerdict {
                path: href.path.clone(),
                fingerprint: href.fingerprint,
                verdict: *verdict,
            })
            .collect();
        let mut notes: Vec<_> = state
            .notes
            .iter()
            .map(|(href, text)| StoredNote {
                path: href.path.clone(),
                fingerprint: href.fingerprint,
                text: text.clone(),
            })
            .collect();
        let mut collapsed: Vec<_> = state
            .collapsed
            .iter()
            .map(|(path, collapsed)| StoredCollapse {
                path: path.clone(),
                collapsed: *collapsed,
            })
            .collect();
        // Stable order so the file diffs cleanly across saves.
        verdicts.sort_by(|a, b| (&a.path, a.fingerprint).cmp(&(&b.path, b.fingerprint)));
        notes.sort_by(|a, b| (&a.path, a.fingerprint).cmp(&(&b.path, b.fingerprint)));
        collapsed.sort_by(|a, b| a.path.cmp(&b.path));
        StoredReview {
            last_session: state.last_session.as_ref().map(|s| s.0.clone()),
            verdicts,
            notes,
            collapsed,
        }
    }

    fn into_state(self) -> ReviewState {
        ReviewState {
            verdicts: self
                .verdicts
                .into_iter()
                .map(|v| {
                    (
                        HunkRef {
                            path: v.path,
                            fingerprint: v.fingerprint,
                        },
                        v.verdict,
                    )
                })
                .collect(),
            notes: self
                .notes
                .into_iter()
                .map(|n| {
                    (
                        HunkRef {
                            path: n.path,
                            fingerprint: n.fingerprint,
                        },
                        n.text,
                    )
                })
                .collect(),
            collapsed: self
                .collapsed
                .into_iter()
                .map(|c| (c.path, c.collapsed))
                .collect(),
            last_session: self.last_session.map(SessionId),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_state_round_trips_through_toml() {
        let mut state = ReviewState::default();
        state.set_verdict(
            HunkRef {
                path: PathBuf::from("src/lib.rs"),
                fingerprint: 42,
            },
            HunkVerdict::Approved,
        );
        state.set_verdict(
            HunkRef {
                path: PathBuf::from("src/main.rs"),
                fingerprint: 7,
            },
            HunkVerdict::NeedsAttention,
        );
        state
            .notes
            .insert(
                HunkRef {
                    path: PathBuf::from("src/lib.rs"),
                    fingerprint: 42,
                },
                "double-check this".into(),
            );
        state.collapsed.insert(PathBuf::from("big.json"), true);
        state.last_session = Some(SessionId("abc-123".into()));

        let toml = state.to_toml().expect("serialize");
        let back = ReviewState::from_toml(&toml).expect("deserialize");
        assert_eq!(state, back);
    }

    #[test]
    fn unreviewed_verdict_clears_entry() {
        let mut state = ReviewState::default();
        let href = HunkRef {
            path: PathBuf::from("a.rs"),
            fingerprint: 1,
        };
        state.set_verdict(href.clone(), HunkVerdict::Approved);
        assert_eq!(state.verdict(&href), HunkVerdict::Approved);
        state.set_verdict(href.clone(), HunkVerdict::Unreviewed);
        assert!(state.verdicts.is_empty());
        assert_eq!(state.verdict(&href), HunkVerdict::Unreviewed);
    }
}
