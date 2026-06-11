//! Suspend the TUI and open the user's editor at a hunk (`e`).
//!
//! The terminal is shared mutable state three ways: ratatui owns it while the
//! alternate screen is up, the input thread reads it, and the editor needs it
//! exclusively. [`InputGate`] parks the input thread first so the editor and
//! `event::read` never compete for stdin; the caller restores the alternate
//! screen afterwards and forces a re-diff to pick up whatever was edited.

use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::app::state::EditRequest;

/// Cooperative pause for the input thread. The thread polls `paused` between
/// reads and acknowledges via `parked`, so the caller knows stdin is free.
#[derive(Default)]
pub struct InputGate {
    paused: AtomicBool,
    parked: AtomicBool,
}

impl InputGate {
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    pub fn set_parked(&self, parked: bool) {
        self.parked.store(parked, Ordering::SeqCst);
    }

    /// Ask the input thread to park and wait (bounded) for it to acknowledge.
    /// The thread polls with a 100ms timeout, so it parks within one cycle; on
    /// timeout we proceed anyway — worst case one keystroke goes astray.
    pub fn pause(&self) {
        self.paused.store(true, Ordering::SeqCst);
        let deadline = Instant::now() + Duration::from_millis(400);
        while !self.parked.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    pub fn resume(&self) {
        self.paused.store(false, Ordering::SeqCst);
    }
}

/// `$VISUAL` falling back to `$EDITOR`, the conventional precedence. `None`
/// (both unset/blank) disables the feature rather than guessing a binary.
pub fn resolve_editor() -> Option<String> {
    ["VISUAL", "EDITOR"]
        .iter()
        .filter_map(|var| std::env::var(var).ok())
        .map(|v| v.trim().to_string())
        .find(|v| !v.is_empty())
}

/// Build the argv for `editor` opening `path` at `line`. The editor value may
/// carry its own flags (`EDITOR="code --wait"`). Line addressing differs by
/// family: `+N file` is the vi/nano/emacs convention, helix wants `file:line`,
/// and VS Code-style editors want `--goto file:line`.
pub fn invocation(editor: &str, path: &Path, line: u32) -> Vec<String> {
    let mut argv: Vec<String> = editor.split_whitespace().map(str::to_string).collect();
    let program = argv
        .first()
        .map(|p| {
            Path::new(p)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.clone())
        })
        .unwrap_or_default();
    let path = path.display();
    match program.as_str() {
        "hx" | "helix" => argv.push(format!("{path}:{line}")),
        "code" | "code-insiders" | "codium" | "cursor" | "windsurf" => {
            argv.push("--goto".into());
            argv.push(format!("{path}:{line}"));
        }
        _ => {
            argv.push(format!("+{line}"));
            argv.push(path.to_string());
        }
    }
    argv
}

/// Run the editor over the real terminal. The caller has already left the
/// alternate screen and parked the input thread; this just blocks until the
/// editor exits. Failure is advisory — log and return to the review.
pub fn run(editor: &str, workdir: &Path, request: &EditRequest) {
    let abs = workdir.join(&request.path);
    let argv = invocation(editor, &abs, request.line);
    let Some((program, args)) = argv.split_first() else {
        return;
    };
    match Command::new(program).args(args).current_dir(workdir).status() {
        Ok(status) if !status.success() => {
            tracing::warn!(%editor, %status, "editor exited with failure");
        }
        Ok(_) => {}
        Err(e) => tracing::warn!(%editor, error = %e, "failed to launch editor"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn vi_family_uses_plus_line() {
        let argv = invocation("vim", &PathBuf::from("/repo/src/a.rs"), 42);
        assert_eq!(argv, ["vim", "+42", "/repo/src/a.rs"]);
    }

    #[test]
    fn editor_value_keeps_its_own_flags() {
        let argv = invocation("code --wait", &PathBuf::from("/repo/a.rs"), 7);
        assert_eq!(argv, ["code", "--wait", "--goto", "/repo/a.rs:7"]);
    }

    #[test]
    fn helix_uses_colon_addressing() {
        let argv = invocation("hx", &PathBuf::from("a.rs"), 3);
        assert_eq!(argv, ["hx", "a.rs:3"]);
    }

    #[test]
    fn absolute_editor_path_is_classified_by_basename() {
        let argv = invocation("/usr/local/bin/hx", &PathBuf::from("a.rs"), 3);
        assert_eq!(argv, ["/usr/local/bin/hx", "a.rs:3"]);
    }

    #[test]
    fn gate_pause_waits_for_park_acknowledgement() {
        let gate = std::sync::Arc::new(InputGate::default());
        let worker = {
            let gate = gate.clone();
            std::thread::spawn(move || {
                while !gate.is_paused() {
                    std::thread::sleep(Duration::from_millis(5));
                }
                gate.set_parked(true);
            })
        };
        gate.pause();
        assert!(gate.parked.load(Ordering::SeqCst));
        worker.join().unwrap();
    }
}
