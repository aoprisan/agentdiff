//! Content-addressed hashing for hunks.
//!
//! The fingerprint hashes a hunk's *content* — the path plus the kind and text
//! of every line — and deliberately excludes line numbers and the `@@` header.
//! That makes it stable when surrounding edits shift a hunk up or down the file,
//! so a verdict re-attaches to "the same change" across a live re-diff. When the
//! content itself changes, the fingerprint changes and the old verdict surfaces
//! as "changed since reviewed."
//!
//! Fingerprints are **persisted** (they key the saved review state), so the
//! hash must be stable across program runs, Rust releases, and platforms.
//! `DefaultHasher` guarantees none of that; this is a hand-rolled FNV-1a.

use std::collections::HashMap;
use std::path::Path;

use crate::domain::diff::{Hunk, Line};

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// 64-bit FNV-1a, chosen because it is trivially stable and dependency-free.
/// Not collision-resistant against adversaries — fine for review anchoring.
struct Fnv1a(u64);

impl Fnv1a {
    fn new() -> Self {
        Fnv1a(FNV_OFFSET)
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(FNV_PRIME);
        }
    }

    fn write_u8(&mut self, b: u8) {
        self.write(&[b]);
    }
}

/// Compute a re-diff-stable fingerprint from a hunk's path and lines.
pub fn fingerprint(path: &Path, lines: &[Line]) -> u64 {
    let mut h = Fnv1a::new();
    h.write(path.to_string_lossy().as_bytes());
    h.write_u8(0);
    for line in lines {
        // Kind byte is offset so it can't collide with the NUL terminator.
        h.write_u8(line.kind as u8 + 1);
        h.write(line.text.as_bytes());
        h.write_u8(0);
    }
    h.0
}

/// Distinguish identical hunks within one file. Two hunks with the same
/// content (repeated code) would otherwise share a `HunkRef`, making one
/// verdict apply to both and re-anchoring always land on the first. The Nth
/// duplicate mixes its occurrence index into the fingerprint; the first
/// occurrence is left untouched so saved refs keep matching. Deterministic
/// across re-diffs because hunks arrive in file order.
pub fn disambiguate_duplicates(hunks: &mut [Hunk]) {
    let mut seen: HashMap<u64, u64> = HashMap::new();
    for hunk in hunks {
        let count = seen.entry(hunk.href.fingerprint).or_insert(0);
        if *count > 0 {
            let mut h = Fnv1a::new();
            h.write(&hunk.href.fingerprint.to_le_bytes());
            h.write(&count.to_le_bytes());
            hunk.href.fingerprint = h.0;
        }
        *count += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::diff::{Line, LineKind, LineRange};
    use crate::domain::review::HunkRef;
    use std::path::PathBuf;

    fn line(kind: LineKind, old: Option<u32>, new: Option<u32>, text: &str) -> Line {
        Line {
            kind,
            old_no: old,
            new_no: new,
            text: text.into(),
            intra: Vec::new(),
        }
    }

    #[test]
    fn fingerprint_ignores_line_numbers() {
        let path = Path::new("src/lib.rs");
        let at_top = vec![
            line(LineKind::Removed, Some(1), None, "let x = 1;"),
            line(LineKind::Added, None, Some(1), "let x = 2;"),
        ];
        // Same content, shifted down the file: only the line numbers differ.
        let shifted = vec![
            line(LineKind::Removed, Some(42), None, "let x = 1;"),
            line(LineKind::Added, None, Some(42), "let x = 2;"),
        ];
        assert_eq!(fingerprint(path, &at_top), fingerprint(path, &shifted));
    }

    #[test]
    fn fingerprint_changes_with_content() {
        let path = Path::new("src/lib.rs");
        let a = vec![line(LineKind::Added, None, Some(1), "let x = 2;")];
        let b = vec![line(LineKind::Added, None, Some(1), "let x = 3;")];
        assert_ne!(fingerprint(path, &a), fingerprint(path, &b));
    }

    #[test]
    fn fingerprint_distinguishes_paths() {
        let lines = vec![line(LineKind::Added, None, Some(1), "let x = 2;")];
        assert_ne!(
            fingerprint(Path::new("a.rs"), &lines),
            fingerprint(Path::new("b.rs"), &lines)
        );
    }

    #[test]
    fn fingerprint_is_stable_across_builds() {
        // Persisted review state depends on this exact value; if this test
        // breaks, saved verdicts everywhere orphan. Do not casually update it.
        let lines = vec![
            line(LineKind::Removed, Some(1), None, "let x = 1;"),
            line(LineKind::Added, None, Some(1), "let x = 2;"),
        ];
        assert_eq!(fingerprint(Path::new("src/lib.rs"), &lines), 0xfcf0_e283_d5cd_e54e);
    }

    #[test]
    fn duplicate_hunks_get_distinct_refs_deterministically() {
        let mk = |fp: u64| Hunk {
            href: HunkRef {
                path: PathBuf::from("a.rs"),
                fingerprint: fp,
            },
            old: LineRange { start: 1, count: 1 },
            new: LineRange { start: 1, count: 1 },
            header: String::new(),
            lines: Vec::new(),
        };
        let mut hunks = vec![mk(7), mk(7), mk(7), mk(9)];
        disambiguate_duplicates(&mut hunks);

        // First occurrence untouched; later ones distinct; unrelated untouched.
        assert_eq!(hunks[0].href.fingerprint, 7);
        assert_ne!(hunks[1].href.fingerprint, 7);
        assert_ne!(hunks[2].href.fingerprint, hunks[1].href.fingerprint);
        assert_eq!(hunks[3].href.fingerprint, 9);

        // Re-running on a fresh build (same order) produces the same refs.
        let mut again = vec![mk(7), mk(7), mk(7), mk(9)];
        disambiguate_duplicates(&mut again);
        assert_eq!(
            hunks.iter().map(|h| h.href.fingerprint).collect::<Vec<_>>(),
            again.iter().map(|h| h.href.fingerprint).collect::<Vec<_>>()
        );
    }
}
