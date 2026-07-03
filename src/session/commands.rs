//! Classifying the shell commands an agent ran and inferring whether they
//! passed.
//!
//! Both are deliberately small, conservative heuristics over the transcript —
//! the result is **advisory**, surfaced next to the agent's stated intent, never
//! treated as authoritative. [`classify`] buckets a command so the UI can single
//! out verification work (tests/build/lint); [`outcome`] reads the result
//! content because Claude Code's `is_error` flag is unreliable for command exit
//! status (a non-zero exit instead shows up as an `Exit code N` line).

use crate::domain::session::{CommandKind, CommandOutcome};

/// Characters of result tail kept for the detail overlay.
const EXCERPT_CHARS: usize = 600;

/// Bucket a shell command by what it's for. Matches whole *tokens* of the
/// command (quoted strings stripped first), in priority order so e.g.
/// `cargo test` is Test, not Build. Substring matching misfired badly here:
/// `git commit -m "add tests"` read as Test, `curl …/latest` matched "test".
/// Compound commands (`a && b`) classify by the highest-priority signal found.
pub fn classify(command: &str) -> CommandKind {
    let unquoted = strip_quoted(command).to_ascii_lowercase();
    // Shell operators separate tokens just like whitespace; flags are dropped
    // (`cargo build --tests` builds tests, it doesn't run them).
    let tokens: Vec<&str> = unquoted
        .split(|c: char| c.is_whitespace() || matches!(c, '&' | '|' | ';' | '(' | ')'))
        .filter(|t| !t.is_empty() && !t.starts_with('-'))
        .collect();
    let has = |names: &[&str]| tokens.iter().any(|t| names.contains(t));
    let pair = |a: &str, b: &str| tokens.windows(2).any(|w| w == [a, b]);

    if has(&["test", "tests", "nextest", "pytest", "jest", "vitest", "ctest"]) {
        CommandKind::Test
    } else if has(&["clippy", "eslint", "lint", "ruff", "vet", "shellcheck", "golangci-lint"]) {
        CommandKind::Lint
    } else if has(&["fmt", "rustfmt", "prettier", "gofmt", "black"]) {
        CommandKind::Format
    } else if has(&["build", "tsc", "make", "compile", "mvn", "gradle", "gradlew"])
        || pair("cargo", "check")
    {
        CommandKind::Build
    } else if has(&["node", "python", "python3"])
        || pair("cargo", "run")
        || pair("npm", "run")
        || pair("npm", "start")
        || tokens.first().is_some_and(|t| t.starts_with("./"))
    {
        CommandKind::Run
    } else if has(&["git"]) {
        CommandKind::Vcs
    } else {
        CommandKind::Other
    }
}

/// Remove single/double-quoted spans so words inside a commit message or an
/// argument string can't masquerade as command tokens.
fn strip_quoted(command: &str) -> String {
    let mut out = String::with_capacity(command.len());
    let mut quote: Option<char> = None;
    for c in command.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '\'' || c == '"' => quote = Some(c),
            None => out.push(c),
        }
    }
    out
}

/// Infer a command's outcome from its result. We trust an explicit tool-level
/// error, then a non-zero `Exit code` line, then a small high-precision set of
/// tool failure signals; absent any of those a captured result is treated as
/// `Ok`. A result whose content couldn't be read at all (empty, unknown shape)
/// with no error flag stays `Unknown` — claiming ✓ on evidence we never saw is
/// the wrong direction to be wrong in.
pub fn outcome(is_error: Option<bool>, output: &str) -> CommandOutcome {
    if is_error == Some(true) || looks_failed(output) {
        CommandOutcome::Failed
    } else if is_error.is_none() && output.trim().is_empty() {
        CommandOutcome::Unknown
    } else {
        CommandOutcome::Ok
    }
}

fn looks_failed(output: &str) -> bool {
    nonzero_exit(output)
        || output.contains("test result: FAILED")
        || output.contains("error[E") // rustc diagnostic codes
        || output.contains("could not compile")
        || output.contains("npm ERR!")
        || output.contains("Traceback (most recent call last)")
        || output.contains("= FAILURES =") // pytest section banner
        // jest/vitest summary: "Tests:       1 failed, 11 passed"
        || output
            .lines()
            .any(|l| l.trim_start().starts_with("Tests:") && l.contains("failed"))
}

/// A `Exit code N` line (CC appends one for non-zero exits) with `N != 0`.
fn nonzero_exit(output: &str) -> bool {
    output.lines().any(|line| {
        line.trim()
            .strip_prefix("Exit code ")
            .and_then(|n| n.trim().parse::<i32>().ok())
            .is_some_and(|code| code != 0)
    })
}

/// The trailing slice of result output, trimmed, for the detail overlay. We keep
/// the tail because failures (and exit codes) live at the end.
pub fn excerpt(output: &str) -> String {
    let trimmed = output.trim_end();
    let count = trimmed.chars().count();
    if count <= EXCERPT_CHARS {
        return trimmed.trim_start().to_string();
    }
    let tail: String = trimmed
        .chars()
        .skip(count - EXCERPT_CHARS)
        .collect::<String>();
    // Drop a partial first line so the excerpt starts cleanly.
    let tail = tail.find('\n').map(|i| &tail[i + 1..]).unwrap_or(&tail);
    format!("…\n{}", tail.trim_start())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_buckets_common_commands() {
        assert_eq!(classify("cargo test --all"), CommandKind::Test);
        assert_eq!(classify("npm test"), CommandKind::Test);
        assert_eq!(classify("cargo clippy --all-targets"), CommandKind::Lint);
        assert_eq!(classify("cargo fmt --check"), CommandKind::Format);
        assert_eq!(classify("cargo build --release"), CommandKind::Build);
        assert_eq!(classify("cargo check"), CommandKind::Build);
        assert_eq!(classify("git status"), CommandKind::Vcs);
        assert_eq!(classify("./target/release/app"), CommandKind::Run);
        assert_eq!(classify("ls -la"), CommandKind::Other);
    }

    #[test]
    fn test_beats_build_for_cargo_test() {
        // Must not be miscategorised as Build despite no "build" token.
        assert_eq!(classify("cargo test"), CommandKind::Test);
    }

    #[test]
    fn classify_matches_tokens_not_substrings() {
        // "tests" inside a commit message is not a test run.
        assert_eq!(classify(r#"git commit -m "add tests""#), CommandKind::Vcs);
        // "test" inside a URL is not a test run.
        assert_eq!(classify("curl https://example.com/latest"), CommandKind::Other);
        // A flag is not a command word: building tests ≠ running them.
        assert_eq!(classify("cargo build --tests"), CommandKind::Build);
        // Compound commands classify by the strongest signal.
        assert_eq!(classify("cargo build && cargo test"), CommandKind::Test);
        assert_eq!(classify("npm run lint"), CommandKind::Lint);
    }

    #[test]
    fn outcome_without_evidence_is_unknown_not_ok() {
        assert_eq!(outcome(None, ""), CommandOutcome::Unknown);
        assert_eq!(outcome(None, "   \n"), CommandOutcome::Unknown);
        // An explicit non-error flag with empty output is still a result.
        assert_eq!(outcome(Some(false), ""), CommandOutcome::Ok);
    }

    #[test]
    fn outcome_detects_pytest_and_jest_failures() {
        assert_eq!(
            outcome(None, "==================== FAILURES ====================\ntest_x"),
            CommandOutcome::Failed
        );
        assert_eq!(
            outcome(None, "Tests:       1 failed, 11 passed\nTime: 2s"),
            CommandOutcome::Failed
        );
        assert_eq!(
            outcome(None, "Tests:       12 passed\nTime: 2s"),
            CommandOutcome::Ok
        );
    }

    #[test]
    fn outcome_trusts_explicit_error_flag() {
        assert_eq!(outcome(Some(true), "whatever"), CommandOutcome::Failed);
    }

    #[test]
    fn outcome_reads_nonzero_exit_code_from_content() {
        let out = "running 3 tests\nsome failure\nExit code 1";
        assert_eq!(outcome(None, out), CommandOutcome::Failed);
        assert_eq!(outcome(Some(false), out), CommandOutcome::Failed);
    }

    #[test]
    fn outcome_ignores_zero_exit_code() {
        assert_eq!(outcome(None, "all good\nExit code 0"), CommandOutcome::Ok);
    }

    #[test]
    fn outcome_detects_known_failure_signals() {
        assert_eq!(
            outcome(None, "test result: FAILED. 1 passed; 2 failed"),
            CommandOutcome::Failed
        );
        assert_eq!(
            outcome(None, "error[E0599]: no method named foo"),
            CommandOutcome::Failed
        );
    }

    #[test]
    fn outcome_clean_result_is_ok() {
        assert_eq!(
            outcome(None, "test result: ok. 12 passed; 0 failed"),
            CommandOutcome::Ok
        );
    }

    #[test]
    fn excerpt_keeps_tail_and_marks_truncation() {
        let long = (0..200)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let ex = excerpt(&long);
        assert!(ex.starts_with('…'));
        assert!(ex.contains("line 199"));
        assert!(!ex.contains("line 0\n"));
        assert!(ex.chars().count() <= EXCERPT_CHARS + 4);
    }

    #[test]
    fn excerpt_passes_short_output_through() {
        assert_eq!(excerpt("  ok  \n"), "ok");
    }
}
