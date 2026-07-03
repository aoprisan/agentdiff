//! Filesystem locations for state and logs.
//!
//! Phase 0 only needs a state directory and a log file path; later phases add
//! the persisted review-state file and the config (keymap/theme) loader here.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use serde::Deserialize;

use crate::domain::diff::DiffBase;
use crate::domain::review::ReviewState;
use crate::error::Result;

/// Resolved on-disk locations for this run.
pub struct Paths {
    /// Directory holding persisted state (review verdicts land here in Phase 1).
    pub state_dir: PathBuf,
    /// Append-only log file (we never log to stdout/stderr while the TUI owns
    /// the terminal).
    pub log_file: PathBuf,
}

/// Resolve and create the application's state directory.
pub fn paths() -> Result<Paths> {
    let state_dir = ProjectDirs::from("dev", "agentdiff", "agentdiff")
        .map(|dirs| dirs.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".agentdiff"));
    fs::create_dir_all(&state_dir)?;
    let log_file = state_dir.join("agentdiff.log");
    Ok(Paths { state_dir, log_file })
}

/// Path to the persisted review state for a given repo + diff base. Keyed by a
/// hash of the absolute repo path so unrelated repos never collide, and tagged
/// by base so a future per-run review doesn't clobber the working-tree one.
/// The hash must be stable across builds (it names files on disk), hence the
/// shared FNV fingerprint rather than `DefaultHasher`.
pub fn review_state_path(state_dir: &Path, repo_workdir: &Path, base: &DiffBase) -> PathBuf {
    let repo_hash = crate::domain::ids::fingerprint(repo_workdir, &[]);
    let file = format!("{repo_hash:016x}-{}.toml", base_tag(base));
    state_dir.join("reviews").join(file)
}

fn base_tag(base: &DiffBase) -> String {
    match base {
        DiffBase::WorkingTreeVsHead => "worktree-head".into(),
        DiffBase::WorkingTreeVsIndex => "worktree-index".into(),
        // Revspecs (`origin/main..HEAD`) and session ids are user/agent input;
        // they must not smuggle separators into the file name.
        DiffBase::Range { from, to } => format!("range-{}-{}", sanitize(from), sanitize(to)),
        DiffBase::AgentRun { session, run } => {
            format!("run-{}-{}", sanitize(&session.0), run.0)
        }
    }
}

/// Make an arbitrary revspec/id safe as a file-name component.
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// Load persisted review state, or a default when the file is missing or
/// unreadable. Review state is advisory: a parse failure must never block the
/// reviewer, so we log and start fresh.
pub fn load_review_state(path: &Path) -> ReviewState {
    let contents = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return ReviewState::default(),
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "could not read review state");
            return ReviewState::default();
        }
    };
    ReviewState::from_toml(&contents).unwrap_or_else(|e| {
        tracing::warn!(path = %path.display(), error = %e, "could not parse review state; starting fresh");
        ReviewState::default()
    })
}

/// User configuration from `~/.config/agentdiff/config.toml`. Everything is
/// optional; missing/unreadable config falls back to built-in defaults.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub theme: ThemeConfig,
    /// `command-name → key` overrides, e.g. `approve = "v"`.
    #[serde(default)]
    pub keys: HashMap<String, String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ThemeConfig {
    /// Built-in palette name: "default", "solarized-dark", or "solarized-light".
    pub name: Option<String>,
    /// syntect theme name (e.g. "base16-ocean.dark", "InspiredGitHub"). Defaults
    /// to the one paired with the palette.
    pub syntax: Option<String>,
    /// `#rrggbb` overrides for the add / remove / intent foregrounds.
    pub added: Option<String>,
    pub removed: Option<String>,
    pub intent: Option<String>,
}

/// Path to the user config file, if a config directory can be resolved.
pub fn config_path() -> Option<PathBuf> {
    ProjectDirs::from("dev", "agentdiff", "agentdiff")
        .map(|dirs| dirs.config_dir().join("config.toml"))
}

/// Load user config, or defaults when the file is absent or unparseable.
pub fn load_config() -> Config {
    let Some(path) = config_path() else {
        return Config::default();
    };
    match fs::read_to_string(&path) {
        Ok(contents) => toml::from_str(&contents).unwrap_or_else(|e| {
            tracing::warn!(path = %path.display(), error = %e, "invalid config.toml; using defaults");
            Config::default()
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Config::default(),
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "could not read config.toml");
            Config::default()
        }
    }
}

/// Persist review state as human-diffable TOML.
pub fn save_review_state(path: &Path, state: &ReviewState) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, state.to_toml()?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::review::{HunkRef, HunkVerdict};

    #[test]
    fn missing_file_loads_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("absent.toml");
        assert_eq!(load_review_state(&path), ReviewState::default());
    }

    #[test]
    fn review_state_survives_save_and_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reviews").join("repo.toml");

        let mut state = ReviewState::default();
        let href = HunkRef {
            path: PathBuf::from("src/main.rs"),
            fingerprint: 0xdead_beef,
        };
        state.set_verdict(href.clone(), HunkVerdict::Approved);

        save_review_state(&path, &state).unwrap();
        let reloaded = load_review_state(&path);
        // The verdict re-anchors to the same fingerprint after a round trip.
        assert_eq!(reloaded.verdict(&href), HunkVerdict::Approved);
    }

    #[test]
    fn base_keys_distinguish_repos_and_bases() {
        let dir = PathBuf::from("/state");
        let a = review_state_path(&dir, Path::new("/repo/a"), &DiffBase::WorkingTreeVsHead);
        let b = review_state_path(&dir, Path::new("/repo/b"), &DiffBase::WorkingTreeVsHead);
        let c = review_state_path(&dir, Path::new("/repo/a"), &DiffBase::WorkingTreeVsIndex);
        assert_ne!(a, b);
        assert_ne!(a, c);
    }
}
