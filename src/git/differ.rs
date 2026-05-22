//! Map a libgit2 diff into the `domain::Diff` spine.
//!
//! Only `DiffBase::WorkingTreeVsHead` is built here (Phase 1). Tracked changes
//! come from `diff_tree_to_workdir_with_index` (staged + unstaged vs HEAD);
//! untracked files are appended by [`super::untracked`]. Intra-line word diff is
//! computed once, at model-build time, and stored on each `Line`.

use std::path::Path;

use git2::{Delta, DiffFlags, DiffFindOptions, DiffLineType, DiffOptions, Patch};
use similar::{ChangeTag, TextDiff};

use super::repo::Repo;
use super::untracked;
use crate::domain::diff::{
    ChangeKind, Diff, DiffBase, FileChange, FileId, Hunk, InlineSpan, Line, LineKind, LineRange,
};
use crate::domain::Timestamp;
use crate::domain::ids::fingerprint;
use crate::domain::review::HunkRef;
use crate::error::Result;

/// Build the working-tree-vs-HEAD diff, untracked files included.
pub fn diff_worktree_vs_head(repo: &Repo) -> Result<Diff> {
    let head_tree = repo.head_tree()?;

    let mut opts = DiffOptions::new();
    opts.include_untracked(false)
        .ignore_submodules(true)
        .context_lines(3);

    let mut git_diff =
        repo.inner()
            .diff_tree_to_workdir_with_index(head_tree.as_ref(), Some(&mut opts))?;

    let mut find = DiffFindOptions::new();
    find.renames(true).copies(true);
    git_diff.find_similar(Some(&mut find))?;

    let mut files = Vec::new();
    for idx in 0..git_diff.deltas().len() {
        if let Some(file) = file_change(&git_diff, idx)? {
            files.push(file);
        }
    }

    files.extend(untracked::collect(repo)?);
    files.sort_by(|a, b| a.path.cmp(&b.path));
    for (i, file) in files.iter_mut().enumerate() {
        file.id = FileId(i as u32);
    }

    Ok(Diff {
        base: DiffBase::WorkingTreeVsHead,
        files,
        generated_at: Timestamp::now(),
    })
}

/// Convert one delta (with its patch) into a `FileChange`, or `None` for deltas
/// we don't surface (unmodified, ignored, untracked — handled elsewhere).
fn file_change(git_diff: &git2::Diff<'_>, idx: usize) -> Result<Option<FileChange>> {
    let delta = git_diff
        .get_delta(idx)
        .ok_or_else(|| crate::error::Error::Other("delta index out of range".into()))?;

    let Some(change) = change_kind(delta.status()) else {
        return Ok(None);
    };

    let new_path = delta.new_file().path();
    let old_path = delta.old_file().path();
    let Some(path) = new_path.or(old_path).map(Path::to_path_buf) else {
        return Ok(None);
    };
    let renamed_from = matches!(change, ChangeKind::Renamed | ChangeKind::Copied)
        .then(|| old_path.map(Path::to_path_buf))
        .flatten()
        .filter(|old| Some(old.as_path()) != new_path);

    let binary_flag = delta.flags().contains(DiffFlags::BINARY);
    let patch = if binary_flag {
        None
    } else {
        Patch::from_diff(git_diff, idx)?
    };
    let is_binary = binary_flag || patch.is_none();

    let mut hunks = Vec::new();
    let mut stats = (0usize, 0usize);
    if let Some(patch) = &patch {
        let (_, added, removed) = patch.line_stats()?;
        stats = (added, removed);
        for h in 0..patch.num_hunks() {
            hunks.push(build_hunk(patch, h, &path)?);
        }
    }

    Ok(Some(FileChange {
        id: FileId(0),
        path: path.clone(),
        old_path: renamed_from,
        change,
        is_binary,
        is_created: change == ChangeKind::Added,
        language: language_for(&path),
        hunks,
        stats,
    }))
}

fn build_hunk(patch: &Patch<'_>, h: usize, path: &Path) -> Result<Hunk> {
    let (git_hunk, _) = patch.hunk(h)?;
    let header = String::from_utf8_lossy(git_hunk.header())
        .trim_end_matches(['\n', '\r'])
        .to_string();

    let mut lines = Vec::new();
    for l in 0..patch.num_lines_in_hunk(h)? {
        let dl = patch.line_in_hunk(h, l)?;
        let kind = match dl.origin_value() {
            DiffLineType::Addition => LineKind::Added,
            DiffLineType::Deletion => LineKind::Removed,
            DiffLineType::Context => LineKind::Context,
            // Skip the "\ No newline at end of file" markers and headers; the
            // line they annotate is already emitted as a normal line.
            _ => continue,
        };
        lines.push(Line {
            kind,
            old_no: dl.old_lineno(),
            new_no: dl.new_lineno(),
            text: trim_eol(&String::from_utf8_lossy(dl.content())).to_string(),
            intra: Vec::new(),
        });
    }
    compute_intra(&mut lines);

    Ok(Hunk {
        href: HunkRef {
            path: path.to_path_buf(),
            fingerprint: fingerprint(path, &lines),
        },
        old: LineRange {
            start: git_hunk.old_start(),
            count: git_hunk.old_lines(),
        },
        new: LineRange {
            start: git_hunk.new_start(),
            count: git_hunk.new_lines(),
        },
        header,
        lines,
    })
}

fn change_kind(status: Delta) -> Option<ChangeKind> {
    match status {
        Delta::Added => Some(ChangeKind::Added),
        Delta::Deleted => Some(ChangeKind::Deleted),
        Delta::Modified => Some(ChangeKind::Modified),
        Delta::Renamed => Some(ChangeKind::Renamed),
        Delta::Copied => Some(ChangeKind::Copied),
        Delta::Typechange => Some(ChangeKind::TypeChange),
        _ => None,
    }
}

/// Pair each removed line with the added line that replaces it and mark the
/// changed substrings on both, so the renderer can emphasize just the edit.
fn compute_intra(lines: &mut [Line]) {
    let mut i = 0;
    while i < lines.len() {
        if lines[i].kind != LineKind::Removed {
            i += 1;
            continue;
        }
        let rem_start = i;
        while i < lines.len() && lines[i].kind == LineKind::Removed {
            i += 1;
        }
        let add_start = i;
        while i < lines.len() && lines[i].kind == LineKind::Added {
            i += 1;
        }
        let pairs = (add_start - rem_start).min(i - add_start);
        for k in 0..pairs {
            let old_text = lines[rem_start + k].text.clone();
            let new_text = lines[add_start + k].text.clone();
            let (old_spans, new_spans) = word_diff(&old_text, &new_text);
            lines[rem_start + k].intra = old_spans;
            lines[add_start + k].intra = new_spans;
        }
    }
}

/// Word-level diff of two lines, returning the changed byte ranges on the old
/// and new sides respectively. Returns empty spans when the lines share nothing
/// (the whole-line add/remove color already conveys that).
fn word_diff(old: &str, new: &str) -> (Vec<InlineSpan>, Vec<InlineSpan>) {
    let diff = TextDiff::from_words(old, new);
    let mut old_spans = Vec::new();
    let mut new_spans = Vec::new();
    let (mut old_pos, mut new_pos) = (0usize, 0usize);
    let mut any_equal = false;

    for change in diff.iter_all_changes() {
        let len = change.value().len();
        match change.tag() {
            ChangeTag::Equal => {
                any_equal = true;
                old_pos += len;
                new_pos += len;
            }
            ChangeTag::Delete => {
                push_span(&mut old_spans, old_pos, old_pos + len);
                old_pos += len;
            }
            ChangeTag::Insert => {
                push_span(&mut new_spans, new_pos, new_pos + len);
                new_pos += len;
            }
        }
    }

    if !any_equal {
        return (Vec::new(), Vec::new());
    }
    (old_spans, new_spans)
}

/// Push a changed span, merging it with the previous one when adjacent (word and
/// whitespace tokens often abut).
fn push_span(spans: &mut Vec<InlineSpan>, start: usize, end: usize) {
    if let Some(last) = spans.last_mut()
        && last.end == start
    {
        last.end = end;
        return;
    }
    spans.push(InlineSpan {
        start,
        end,
        changed: true,
    });
}

fn trim_eol(s: &str) -> &str {
    s.strip_suffix('\n')
        .map(|s| s.strip_suffix('\r').unwrap_or(s))
        .unwrap_or(s)
}

/// Best-effort language token from a file extension, for display and as a hint
/// to the syntax highlighter. Returns `None` for unknown/extensionless files.
pub fn language_for(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    let lang = match ext.as_str() {
        "rs" => "rust",
        "py" | "pyi" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" => "typescript",
        "tsx" => "tsx",
        "jsx" => "jsx",
        "go" => "go",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => "cpp",
        "java" => "java",
        "rb" => "ruby",
        "php" => "php",
        "sh" | "bash" | "zsh" => "bash",
        "json" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "md" | "markdown" => "markdown",
        "html" | "htm" => "html",
        "css" => "css",
        "scss" | "sass" => "scss",
        "sql" => "sql",
        "kt" | "kts" => "kotlin",
        "swift" => "swift",
        "lua" => "lua",
        "xml" => "xml",
        _ => return None,
    };
    Some(lang.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::diff::ChangeKind;
    use git2::{Repository, Signature};
    use std::fs;
    use std::path::Path;

    fn write(root: &Path, rel: &str, content: &[u8]) {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    /// Build a repo with one commit, then leave a mix of working-tree changes:
    /// a modify, a delete, a staged rename, and several untracked files (text,
    /// binary, CRLF, no trailing newline). Returns the temp dir (kept alive).
    fn fixture_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let repo = Repository::init(root).unwrap();

        write(root, "keep.txt", b"unchanged\n");
        write(root, "remove.txt", b"to be deleted\n");
        write(root, "rename_me.txt", b"alpha\nbeta\ngamma\n");
        write(root, "modify.rs", b"fn main() {\n    let x = 1;\n}\n");

        let mut index = repo.index().unwrap();
        for f in ["keep.txt", "remove.txt", "rename_me.txt", "modify.rs"] {
            index.add_path(Path::new(f)).unwrap();
        }
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = Signature::now("Test", "test@example.com").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .unwrap();

        // Modify a tracked file (unstaged).
        write(root, "modify.rs", b"fn main() {\n    let x = 2;\n}\n");
        // Delete a tracked file.
        fs::remove_file(root.join("remove.txt")).unwrap();
        // Stage a rename so libgit2 can pair the delete + add.
        fs::rename(root.join("rename_me.txt"), root.join("renamed.txt")).unwrap();
        let mut index = repo.index().unwrap();
        index.remove_path(Path::new("rename_me.txt")).unwrap();
        index.add_path(Path::new("renamed.txt")).unwrap();
        index.write().unwrap();
        // Untracked: text, binary, CRLF, and a file with no trailing newline.
        write(root, "new.py", b"def greet():\n    print('hi')\n");
        write(root, "logo.bin", &[0u8, 159, 146, 150, 0, 1, 2, 3]);
        write(root, "crlf.txt", b"first\r\nsecond\r\n");
        write(root, "nonl.txt", b"no trailing newline");

        dir
    }

    fn find<'a>(diff: &'a Diff, path: &str) -> &'a FileChange {
        diff.files
            .iter()
            .find(|f| f.path == Path::new(path))
            .unwrap_or_else(|| panic!("expected {path} in diff"))
    }

    #[test]
    fn classifies_mixed_working_tree_changes() {
        let dir = fixture_repo();
        let repo = Repo::discover(dir.path()).unwrap();
        let diff = diff_worktree_vs_head(&repo).unwrap();

        // The file set matches `git status` + untracked; unchanged keep.txt is absent.
        let paths: Vec<_> = diff.files.iter().map(|f| f.path.clone()).collect();
        assert!(!paths.iter().any(|p| p == Path::new("keep.txt")));

        assert_eq!(find(&diff, "modify.rs").change, ChangeKind::Modified);
        assert_eq!(find(&diff, "remove.txt").change, ChangeKind::Deleted);

        let renamed = find(&diff, "renamed.txt");
        assert_eq!(renamed.change, ChangeKind::Renamed);
        assert_eq!(renamed.old_path.as_deref(), Some(Path::new("rename_me.txt")));

        let created = find(&diff, "new.py");
        assert_eq!(created.change, ChangeKind::Added);
        assert!(created.is_created);

        let binary = find(&diff, "logo.bin");
        assert!(binary.is_binary);
        assert!(binary.hunks.is_empty());

        // No trailing newline: the synthesized hunk has one added line, no marker.
        let nonl = find(&diff, "nonl.txt");
        assert_eq!(nonl.hunks[0].lines.len(), 1);
        assert_eq!(nonl.hunks[0].lines[0].text, "no trailing newline");

        // Word diff isolates the changed digit on the modified line.
        let modified = find(&diff, "modify.rs");
        let added_line = modified.hunks[0]
            .lines
            .iter()
            .find(|l| l.kind == LineKind::Added)
            .unwrap();
        assert!(!added_line.intra.is_empty());
        let span = &added_line.intra[0];
        // The change is isolated to the tail of the line — there's an unchanged
        // prefix and the span is a strict subset — and it covers the new digit.
        assert!(span.start > 0);
        assert!(span.end - span.start < added_line.text.len());
        assert!(added_line.text[span.start..span.end].contains('2'));
    }

    #[test]
    fn worktree_diff_model_snapshot() {
        let dir = fixture_repo();
        let repo = Repo::discover(dir.path()).unwrap();
        let diff = diff_worktree_vs_head(&repo).unwrap();

        // `generated_at` is wall-clock and fingerprints are hash values; redact
        // both so the snapshot captures structure, not environment.
        insta::assert_json_snapshot!(diff, {
            ".generated_at" => "[ts]",
            ".files[].hunks[].href.fingerprint" => "[fingerprint]",
        });
    }
}
