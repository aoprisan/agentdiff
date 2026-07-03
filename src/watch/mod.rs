//! Filesystem watchers that signal "something under review changed".
//!
//! Watches the working tree (recursively) and the active session transcript,
//! debouncing bursty agent writes. Tree events are filtered through the repo's
//! gitignore (and `.git` is always skipped) so build artifacts and git's own
//! churn don't trigger spurious re-diffs. Change notification is a plain
//! callback so this module depends only on `domain`-level concepts, not `app`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{DebounceEventResult, Debouncer, RecommendedCache, new_debouncer};

/// Debounce window for coalescing bursty writes during an agent run.
const DEBOUNCE: Duration = Duration::from_millis(250);

/// The watcher guard. Dropping it stops watching, so the caller must keep it.
pub type Watch = Debouncer<RecommendedWatcher, RecommendedCache>;

/// Start watching `workdir` and (optionally) the active transcript, invoking
/// `on_change` after each debounced burst of relevant changes. Returns `None`
/// if the platform watcher can't be created (watching is best-effort; the tool
/// stays fully usable without it).
pub fn spawn(
    workdir: &Path,
    session_file: Option<PathBuf>,
    on_change: impl Fn() + Send + 'static,
) -> Option<Watch> {
    let gitignore = build_gitignore(workdir);
    let root = workdir.to_path_buf();
    let session = session_file.clone();

    let handler = move |result: DebounceEventResult| {
        let Ok(events) = result else { return };
        let relevant = events
            .iter()
            .flat_map(|e| e.paths.iter())
            .any(|p| is_relevant(p, &root, &gitignore, session.as_deref()));
        if relevant {
            on_change();
        }
    };

    let mut debouncer = new_debouncer(DEBOUNCE, None, handler).ok()?;
    debouncer.watch(workdir, RecursiveMode::Recursive).ok()?;
    if let Some(file) = &session_file {
        // The transcript may live outside the tree; best effort.
        let _ = debouncer.watch(file, RecursiveMode::NonRecursive);
    }
    Some(debouncer)
}

/// Whether a changed path should trigger a re-diff.
pub(crate) fn is_relevant(
    path: &Path,
    workdir: &Path,
    gitignore: &Gitignore,
    session_file: Option<&Path>,
) -> bool {
    if session_file == Some(path) {
        return true;
    }
    if path.components().any(|c| c.as_os_str() == ".git") {
        return false;
    }
    if !path.starts_with(workdir) {
        return false;
    }
    !gitignore
        .matched_path_or_any_parents(path, path.is_dir())
        .is_ignore()
}

fn build_gitignore(workdir: &Path) -> Gitignore {
    let mut builder = GitignoreBuilder::new(workdir);
    let _ = builder.add(workdir.join(".gitignore"));
    builder.build().unwrap_or_else(|_| Gitignore::empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gitignore(workdir: &Path, rules: &str) -> Gitignore {
        let mut b = GitignoreBuilder::new(workdir);
        for line in rules.lines() {
            let _ = b.add_line(None, line);
        }
        b.build().unwrap()
    }

    #[test]
    fn filters_git_and_ignored_paths() {
        let wd = Path::new("/repo");
        let gi = gitignore(wd, "/target\n");

        assert!(is_relevant(Path::new("/repo/src/main.rs"), wd, &gi, None));
        // git internals and gitignored build output are noise.
        assert!(!is_relevant(Path::new("/repo/.git/index"), wd, &gi, None));
        assert!(!is_relevant(Path::new("/repo/target/debug/x"), wd, &gi, None));
        // outside the tree → ignore, unless it's the watched transcript.
        assert!(!is_relevant(Path::new("/other/file"), wd, &gi, None));
        let sf = Path::new("/home/.claude/projects/p/s.jsonl");
        assert!(is_relevant(sf, wd, &gi, Some(sf)));
    }
}
