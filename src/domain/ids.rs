//! Content-addressed hashing for hunks.
//!
//! The fingerprint hashes a hunk's *content* — the path plus the kind and text
//! of every line — and deliberately excludes line numbers and the `@@` header.
//! That makes it stable when surrounding edits shift a hunk up or down the file,
//! so a verdict re-attaches to "the same change" across a live re-diff. When the
//! content itself changes, the fingerprint changes and the old verdict surfaces
//! as "changed since reviewed."

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;

use crate::domain::diff::Line;

/// Compute a re-diff-stable fingerprint from a hunk's path and lines.
pub fn fingerprint(path: &Path, lines: &[Line]) -> u64 {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    for line in lines {
        (line.kind as u8).hash(&mut hasher);
        line.text.hash(&mut hasher);
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::diff::{Line, LineKind};

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
}
