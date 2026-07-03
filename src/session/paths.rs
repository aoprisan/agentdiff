//! Matching agent-recorded paths against the repository root.
//!
//! Transcripts record absolute paths as the agent saw them, which can differ
//! from git2's `workdir()` spelling for the same directory: a repo opened
//! through a symlink, macOS's `/tmp` → `/private/tmp`, or `..` segments. A
//! purely lexical `strip_prefix` silently drops every backup/intent entry in
//! those cases — losing the whole session overlay — so all relativization and
//! cwd comparison goes through [`RepoPaths`], which matches against both the
//! as-given and the symlink-resolved root spelling.

use std::path::{Component, Path, PathBuf};

/// The repo root in every spelling recorded paths might use.
#[derive(Debug, Clone)]
pub struct RepoPaths {
    /// The root as given (git2's `workdir()`), trailing separator stripped.
    given: PathBuf,
    /// The symlink-resolved root, kept only when it differs from `given`.
    canonical: Option<PathBuf>,
}

impl RepoPaths {
    pub fn new(repo_root: &Path) -> Self {
        let given = lexical_normalize(repo_root);
        let canonical = std::fs::canonicalize(&given).ok().filter(|c| *c != given);
        RepoPaths { given, canonical }
    }

    /// Every root spelling to match against.
    pub fn roots(&self) -> impl Iterator<Item = &Path> {
        std::iter::once(self.given.as_path()).chain(self.canonical.as_deref())
    }

    /// Repo-relative form of a recorded path (absolute, or relative to the
    /// repo root). `None` when it falls outside the repo — out-of-repo entries
    /// (`~/.claude/plans/*`, a home-level `CLAUDE.md`) are dropped.
    pub fn relativize(&self, recorded: &str) -> Option<PathBuf> {
        let p = Path::new(recorded);
        if !p.is_absolute() {
            let rel = lexical_normalize(p);
            return (!rel.as_os_str().is_empty() && !rel.starts_with("..")).then_some(rel);
        }
        let abs = lexical_normalize(p);
        self.strip(&abs)
            // The recorded path may spell a prefix through a symlink the roots
            // don't use (or vice versa); resolve it and retry before dropping.
            .or_else(|| canonicalize_lenient(&abs).and_then(|c| self.strip(&c)))
    }

    /// Whether `other` names the same directory as the repo root.
    pub fn matches_root(&self, other: &str) -> bool {
        let o = lexical_normalize(Path::new(other));
        self.roots().any(|r| r == o)
            || canonicalize_lenient(&o).is_some_and(|c| self.roots().any(|r| r == c))
    }

    /// Whether `other` is the repo root or nested inside it.
    pub fn contains(&self, other: &str) -> bool {
        let o = lexical_normalize(Path::new(other));
        let inside = |p: &Path| self.roots().any(|r| p.starts_with(r));
        inside(&o) || canonicalize_lenient(&o).is_some_and(|c| inside(&c))
    }

    fn strip(&self, abs: &Path) -> Option<PathBuf> {
        self.roots()
            .find_map(|root| abs.strip_prefix(root).ok())
            .filter(|rel| !rel.as_os_str().is_empty())
            .map(Path::to_path_buf)
    }
}

/// Fold `.`/`..` segments and strip a trailing separator, without touching the
/// filesystem.
fn lexical_normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push(Component::ParentDir);
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// `fs::canonicalize` that also works for paths that no longer exist (a
/// deleted file): resolve the parent and re-append the file name.
fn canonicalize_lenient(p: &Path) -> Option<PathBuf> {
    if let Ok(c) = std::fs::canonicalize(p) {
        return Some(c);
    }
    let (parent, name) = (p.parent()?, p.file_name()?);
    std::fs::canonicalize(parent).ok().map(|c| c.join(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relativizes_lexically_against_the_given_root() {
        let paths = RepoPaths::new(Path::new("/repo/"));
        assert_eq!(paths.relativize("/repo/src/a.rs"), Some(PathBuf::from("src/a.rs")));
        assert_eq!(paths.relativize("relative/b.rs"), Some(PathBuf::from("relative/b.rs")));
        assert_eq!(paths.relativize("/elsewhere/c.rs"), None);
        assert_eq!(paths.relativize("/repo"), None, "the root itself is not a file");
        // `..` segments are folded before matching.
        assert_eq!(
            paths.relativize("/repo/src/../src/a.rs"),
            Some(PathBuf::from("src/a.rs"))
        );
        assert_eq!(paths.relativize("/repo/../other/a.rs"), None);
    }

    #[cfg(unix)]
    #[test]
    fn matches_paths_recorded_through_a_symlinked_root() {
        // real/ is the repo; link/ is a symlink to it. Paths recorded under
        // either spelling must relativize regardless of which one we opened.
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("real");
        std::fs::create_dir_all(real.join("src")).unwrap();
        std::fs::write(real.join("src/a.rs"), "x").unwrap();
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        // Opened via the symlink, recorded via the real path (and vice versa).
        let via_link = RepoPaths::new(&link);
        let real_key = real.join("src/a.rs");
        assert_eq!(
            via_link.relativize(real_key.to_str().unwrap()),
            Some(PathBuf::from("src/a.rs"))
        );
        let via_real = RepoPaths::new(&real);
        let link_key = link.join("src/a.rs");
        assert_eq!(
            via_real.relativize(link_key.to_str().unwrap()),
            Some(PathBuf::from("src/a.rs"))
        );
        // A recorded file that has since been deleted still resolves via its
        // parent directory.
        let gone = link.join("src/deleted.rs");
        assert_eq!(
            via_real.relativize(gone.to_str().unwrap()),
            Some(PathBuf::from("src/deleted.rs"))
        );

        assert!(via_real.matches_root(link.to_str().unwrap()));
        assert!(via_link.matches_root(real.to_str().unwrap()));
        assert!(via_real.contains(link.join("src").to_str().unwrap()));
        assert!(!via_real.contains(tmp.path().to_str().unwrap()));
    }
}
