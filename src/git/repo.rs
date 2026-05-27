//! Opening a repository and resolving its HEAD. The only place a libgit2
//! `Repository` handle is created; everything downstream borrows it.

use std::path::{Path, PathBuf};

use git2::{ErrorCode, Repository, Tree};

use crate::error::{Error, Result};

/// A read-only handle to the repository under review.
pub struct Repo {
    inner: Repository,
    workdir: PathBuf,
}

impl Repo {
    /// Open the repository containing `start`, walking up for a `.git`.
    pub fn discover(start: &Path) -> Result<Repo> {
        let inner = Repository::discover(start)?;
        let workdir = inner
            .workdir()
            .ok_or_else(|| {
                Error::Other(format!(
                    "{} is a bare repository; nothing to review",
                    start.display()
                ))
            })?
            .to_path_buf();
        Ok(Repo { inner, workdir })
    }

    /// Absolute path to the working tree root.
    pub fn workdir(&self) -> &Path {
        &self.workdir
    }

    /// The underlying libgit2 handle, for the differ/untracked walker.
    pub(crate) fn inner(&self) -> &Repository {
        &self.inner
    }

    /// The tree at HEAD, or `None` when HEAD is unborn (no commits yet) — in
    /// which case every tracked file reads as an addition.
    pub fn head_tree(&self) -> Result<Option<Tree<'_>>> {
        match self.inner.head() {
            Ok(head) => Ok(Some(head.peel_to_commit()?.tree()?)),
            Err(e)
                if matches!(e.code(), ErrorCode::UnbornBranch | ErrorCode::NotFound) =>
            {
                Ok(None)
            }
            Err(e) => Err(e.into()),
        }
    }

    /// The verbatim content of `rel_path` as of `rev` (a commit-ish), lossily
    /// decoded as UTF-8. `None` when the revision doesn't resolve or the path
    /// didn't exist there (e.g. an agent-created file) — the differ treats that
    /// as an empty pre-run side. Used to recover a Copilot run's "before".
    pub fn blob_at(&self, rev: &str, rel_path: &Path) -> Option<String> {
        let tree = self.inner.revparse_single(rev).ok()?.peel_to_tree().ok()?;
        let entry = tree.get_path(rel_path).ok()?;
        let blob = entry.to_object(&self.inner).ok()?.peel_to_blob().ok()?;
        Some(String::from_utf8_lossy(blob.content()).into_owned())
    }
}
