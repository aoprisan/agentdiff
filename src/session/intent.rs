//! Recovering the agent's stated intent for each edit, then anchoring it to
//! the diff at two granularities.
//!
//! An `Edit`/`Write`/`MultiEdit` `tool_use` carries no reasoning of its own; the
//! "why" lives in the nearest preceding `assistant` text turn. We index records
//! by uuid, then for each edit walk the `parentUuid` chain up to the closest
//! assistant prose and attach it as an [`Intent`]. Confidence decays with the
//! number of hops; edits outside the repo are dropped.
//!
//! [`build`] folds the per-edit intents into a per-file map (later edits win) —
//! the coarse fallback. [`correlate`] goes finer: each edit also carries the
//! text it wrote/removed, and once the diff exists each hunk is matched against
//! that content so a file edited several times for different reasons shows each
//! hunk its *own* "why" instead of the file's last one.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::domain::diff::{Diff, LineKind};
use crate::domain::review::HunkRef;
use crate::domain::session::Intent;

use super::transcript::{Record, edit_tool};

/// Repo-relative path → recovered intent (per-file fallback).
pub type IntentMap = HashMap<PathBuf, Intent>;

/// Content-addressed hunk → the intent of the edit that produced it.
pub type HunkIntentMap = HashMap<HunkRef, Intent>;

/// Maximum `parentUuid` hops before we give up on finding intent.
const MAX_HOPS: usize = 25;

/// One edit with its recovered intent and the content it touched, in
/// transcript order. The content sets hold the *distinctive* trimmed lines of
/// what the edit wrote (`new_string`/`content`) and removed (`old_string`),
/// used by [`correlate`] to anchor intent onto hunks.
#[derive(Debug, Clone)]
pub struct EditIntent {
    pub path: PathBuf,
    new_lines: HashSet<String>,
    old_lines: HashSet<String>,
    pub intent: Intent,
}

/// Build the per-file intent map: the most recent edit's intent per path.
pub fn build(records: &[Record], repo_root: &Path) -> IntentMap {
    let mut map = IntentMap::new();
    for edit in edit_intents(records, repo_root) {
        map.insert(edit.path.clone(), edit.intent);
    }
    map
}

/// Every in-repo edit with recoverable intent, in transcript order.
pub fn edit_intents(records: &[Record], repo_root: &Path) -> Vec<EditIntent> {
    let by_uuid: HashMap<&str, &Record> = records
        .iter()
        .filter_map(|r| Some((r.as_entry()?.uuid.as_deref()?, r)))
        .collect();

    let mut edits = Vec::new();
    for record in records {
        if !record.is_assistant() {
            continue;
        }
        let Some(entry) = record.as_entry() else {
            continue;
        };
        for (name, input) in entry.blocks().iter().filter_map(|b| b.as_tool_use()) {
            if edit_tool(name).is_none() {
                continue;
            }
            let Some(path_str) = input.get("file_path").and_then(|v| v.as_str()) else {
                continue;
            };
            let Some(rel) = relativize(path_str, repo_root) else {
                continue; // edit outside the repo
            };
            if let Some((text, source_uuid, hops)) = walk_to_intent(record, &by_uuid) {
                let (new_lines, old_lines) = content_lines(input);
                edits.push(EditIntent {
                    path: rel.clone(),
                    new_lines,
                    old_lines,
                    intent: Intent {
                        file_path: rel,
                        text,
                        source_uuid,
                        confidence: confidence(hops),
                    },
                });
            }
        }
    }
    edits
}

/// Anchor edit intents onto the diff's hunks by content. A hunk matches an
/// edit when its distinctive added lines appear in what the edit wrote (or,
/// for pure deletions, its removed lines in what the edit removed); the
/// best-scoring edit wins, latest on ties. Unmatched hunks fall back to the
/// per-file map. Advisory like everything session-derived: the tree may have
/// moved on since the edit, in which case nothing matches and that's fine.
pub fn correlate(diff: &Diff, edits: &[EditIntent]) -> HunkIntentMap {
    let mut map = HunkIntentMap::new();
    for file in &diff.files {
        let file_edits: Vec<&EditIntent> = edits.iter().filter(|e| e.path == file.path).collect();
        if file_edits.is_empty() {
            continue;
        }
        for hunk in &file.hunks {
            let added: Vec<&str> = distinctive_lines(hunk, LineKind::Added);
            let removed: Vec<&str> = distinctive_lines(hunk, LineKind::Removed);
            let mut best: Option<(usize, &EditIntent)> = None;
            for edit in &file_edits {
                let mut score = added
                    .iter()
                    .filter(|l| edit.new_lines.contains(**l))
                    .count();
                if score == 0 && added.is_empty() {
                    score = removed
                        .iter()
                        .filter(|l| edit.old_lines.contains(**l))
                        .count();
                }
                // `>=` so the latest edit wins a tie (transcript order).
                if score > 0 && best.is_none_or(|(s, _)| score >= s) {
                    best = Some((score, edit));
                }
            }
            if let Some((_, edit)) = best {
                map.insert(hunk.href.clone(), edit.intent.clone());
            }
        }
    }
    map
}

/// Trimmed lines of a hunk worth matching on. Short or symbol-only lines
/// (`}`, `end`) would match almost any edit, so they are excluded.
fn distinctive_lines(hunk: &crate::domain::diff::Hunk, kind: LineKind) -> Vec<&str> {
    hunk.lines
        .iter()
        .filter(|l| l.kind == kind)
        .map(|l| l.text.trim())
        .filter(|t| is_distinctive(t))
        .collect()
}

fn is_distinctive(trimmed: &str) -> bool {
    trimmed.len() >= 4 && trimmed.chars().any(|c| c.is_alphanumeric())
}

/// The distinctive `(written, removed)` lines of an edit's input. `Edit` has
/// `old_string`/`new_string`, `Write` has `content`, `MultiEdit` a list of
/// old/new pairs. Unknown shapes yield empty sets (the edit still anchors
/// per-file intent, it just can't claim individual hunks).
fn content_lines(input: &serde_json::Value) -> (HashSet<String>, HashSet<String>) {
    let mut new_lines = HashSet::new();
    let mut old_lines = HashSet::new();
    let add = |value: Option<&serde_json::Value>, set: &mut HashSet<String>| {
        if let Some(text) = value.and_then(|v| v.as_str()) {
            set.extend(
                text.lines()
                    .map(str::trim)
                    .filter(|t| is_distinctive(t))
                    .map(str::to_string),
            );
        }
    };
    add(input.get("new_string"), &mut new_lines);
    add(input.get("content"), &mut new_lines);
    add(input.get("old_string"), &mut old_lines);
    if let Some(multi) = input.get("edits").and_then(|v| v.as_array()) {
        for edit in multi {
            add(edit.get("new_string"), &mut new_lines);
            add(edit.get("old_string"), &mut old_lines);
        }
    }
    (new_lines, old_lines)
}

/// From an edit record, follow `parentUuid` to the nearest assistant text turn.
fn walk_to_intent(
    start: &Record,
    by_uuid: &HashMap<&str, &Record>,
) -> Option<(String, String, usize)> {
    let mut current = Some(start);
    for hops in 0..=MAX_HOPS {
        let record = current?;
        if record.is_assistant()
            && let Some(entry) = record.as_entry()
            && let Some(text) = entry.assistant_text()
        {
            let uuid = entry.uuid.clone().unwrap_or_default();
            return Some((text, uuid, hops));
        }
        let parent = record.as_entry().and_then(|e| e.parent_uuid.as_deref());
        current = parent.and_then(|p| by_uuid.get(p).copied());
    }
    None
}

fn confidence(hops: usize) -> f32 {
    (1.0 - 0.1 * hops as f32).clamp(0.3, 1.0)
}

fn relativize(path_str: &str, repo_root: &Path) -> Option<PathBuf> {
    let p = Path::new(path_str);
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        repo_root.join(p)
    };
    abs.strip_prefix(repo_root)
        .ok()
        .filter(|r| !r.as_os_str().is_empty())
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::transcript::parse_reader;

    #[test]
    fn resolves_edit_to_nearest_assistant_text() {
        let jsonl = r#"
{"type":"user","uuid":"u1","message":{"content":"build the parser"}}
{"type":"assistant","uuid":"a1","parentUuid":"u1","message":{"content":[{"type":"text","text":"Now writing the parser."}]}}
{"type":"assistant","uuid":"a2","parentUuid":"a1","message":{"content":[{"type":"thinking","thinking":"..."},{"type":"tool_use","name":"Write","input":{"file_path":"/repo/src/parser.rs"}}]}}
{"type":"assistant","uuid":"a3","parentUuid":"a2","message":{"content":[{"type":"tool_use","name":"Edit","input":{"file_path":"/outside/x.rs"}}]}}
"#;
        let records = parse_reader(jsonl.as_bytes());
        let map = build(&records, Path::new("/repo"));

        // The parser edit resolves one hop up to its reasoning.
        let intent = map.get(Path::new("src/parser.rs")).expect("intent for parser");
        assert_eq!(intent.text, "Now writing the parser.");
        assert_eq!(intent.source_uuid, "a1");
        assert!((intent.confidence - 0.9).abs() < 1e-6);

        // The out-of-repo edit is dropped.
        assert!(!map.contains_key(Path::new("x.rs")));
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn edit_with_inline_text_resolves_at_zero_hops() {
        let jsonl = r#"{"type":"assistant","uuid":"a1","message":{"content":[{"type":"text","text":"Fixing the bug."},{"type":"tool_use","name":"Edit","input":{"file_path":"/repo/a.rs"}}]}}"#;
        let map = build(&parse_reader(jsonl.as_bytes()), Path::new("/repo"));
        let intent = map.get(Path::new("a.rs")).unwrap();
        assert_eq!(intent.text, "Fixing the bug.");
        assert!((intent.confidence - 1.0).abs() < 1e-6);
    }

    // --- hunk-level correlation ---------------------------------------------

    use crate::domain::Timestamp;
    use crate::domain::diff::{
        ChangeKind, DiffBase, FileChange, FileId, Hunk, Line, LineRange,
    };

    /// Two edits to the same file, each with its own reasoning and content.
    const TWO_EDITS: &str = r#"
{"type":"assistant","uuid":"a1","message":{"content":[{"type":"text","text":"Add the parser."},{"type":"tool_use","name":"Edit","input":{"file_path":"/repo/a.rs","old_string":"fn old_parser() {}","new_string":"fn parse_tokens(input: &str) {}"}}]}}
{"type":"assistant","uuid":"a2","parentUuid":"a1","message":{"content":[{"type":"text","text":"Fix the off-by-one."},{"type":"tool_use","name":"Edit","input":{"file_path":"/repo/a.rs","old_string":"while i <= len_total","new_string":"while i < len_total"}}]}}
"#;

    fn hunk_of(fp: u64, lines: Vec<(LineKind, &str)>) -> Hunk {
        Hunk {
            href: HunkRef {
                path: PathBuf::from("a.rs"),
                fingerprint: fp,
            },
            old: LineRange { start: 1, count: 1 },
            new: LineRange { start: 1, count: 1 },
            header: format!("@@ {fp} @@"),
            lines: lines
                .into_iter()
                .map(|(kind, text)| Line {
                    kind,
                    old_no: None,
                    new_no: None,
                    text: text.into(),
                    intra: Vec::new(),
                })
                .collect(),
        }
    }

    fn diff_of(hunks: Vec<Hunk>) -> Diff {
        Diff {
            base: DiffBase::WorkingTreeVsHead,
            generated_at: Timestamp(0),
            files: vec![FileChange {
                id: FileId(0),
                path: PathBuf::from("a.rs"),
                old_path: None,
                change: ChangeKind::Modified,
                is_binary: false,
                is_created: false,
                base_fallback: false,
                language: None,
                hunks,
                stats: (0, 0),
            }],
        }
    }

    #[test]
    fn correlate_gives_each_hunk_its_own_edit_intent() {
        let records = parse_reader(TWO_EDITS.trim().as_bytes());
        let edits = edit_intents(&records, Path::new("/repo"));
        assert_eq!(edits.len(), 2);

        let h1 = hunk_of(1, vec![(LineKind::Added, "fn parse_tokens(input: &str) {}")]);
        let h2 = hunk_of(2, vec![(LineKind::Added, "while i < len_total")]);
        let diff = diff_of(vec![h1.clone(), h2.clone()]);

        let map = correlate(&diff, &edits);
        assert_eq!(map.get(&h1.href).unwrap().text, "Add the parser.");
        assert_eq!(map.get(&h2.href).unwrap().text, "Fix the off-by-one.");

        // The per-file fallback keeps only the most recent intent — the
        // behavior hunk correlation exists to improve on.
        let file_map = build(&records, Path::new("/repo"));
        assert_eq!(file_map.get(Path::new("a.rs")).unwrap().text, "Fix the off-by-one.");
    }

    #[test]
    fn pure_deletions_match_on_what_the_edit_removed() {
        let records = parse_reader(TWO_EDITS.trim().as_bytes());
        let edits = edit_intents(&records, Path::new("/repo"));

        let h = hunk_of(3, vec![(LineKind::Removed, "fn old_parser() {}")]);
        let map = correlate(&diff_of(vec![h.clone()]), &edits);
        assert_eq!(map.get(&h.href).unwrap().text, "Add the parser.");
    }

    #[test]
    fn unmatched_or_indistinct_hunks_get_no_hunk_intent() {
        let records = parse_reader(TWO_EDITS.trim().as_bytes());
        let edits = edit_intents(&records, Path::new("/repo"));

        // Content the agent never wrote, and a symbol-only line that would
        // match anything — neither may claim an edit's intent.
        let unrelated = hunk_of(4, vec![(LineKind::Added, "completely unrelated line")]);
        let braces = hunk_of(5, vec![(LineKind::Added, "}")]);
        let map = correlate(&diff_of(vec![unrelated.clone(), braces.clone()]), &edits);
        assert!(map.is_empty());
    }

    #[test]
    fn multiedit_and_write_content_both_anchor() {
        let jsonl = r#"
{"type":"assistant","uuid":"a1","message":{"content":[{"type":"text","text":"Write the module."},{"type":"tool_use","name":"Write","input":{"file_path":"/repo/a.rs","content":"pub fn entry_point() {}\n"}}]}}
{"type":"assistant","uuid":"a2","parentUuid":"a1","message":{"content":[{"type":"text","text":"Batch rename."},{"type":"tool_use","name":"MultiEdit","input":{"file_path":"/repo/a.rs","edits":[{"old_string":"entry_point","new_string":"fn renamed_entry() {}"}]}}]}}
"#;
        let records = parse_reader(jsonl.trim().as_bytes());
        let edits = edit_intents(&records, Path::new("/repo"));

        let from_write = hunk_of(1, vec![(LineKind::Added, "pub fn entry_point() {}")]);
        let from_multi = hunk_of(2, vec![(LineKind::Added, "fn renamed_entry() {}")]);
        let map = correlate(&diff_of(vec![from_write.clone(), from_multi.clone()]), &edits);
        assert_eq!(map.get(&from_write.href).unwrap().text, "Write the module.");
        assert_eq!(map.get(&from_multi.href).unwrap().text, "Batch rename.");
    }
}
