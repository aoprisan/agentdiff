//! Recovering the agent's stated intent for each edited file.
//!
//! An `Edit`/`Write`/`MultiEdit` `tool_use` carries no reasoning of its own; the
//! "why" lives in the nearest preceding `assistant` text turn. We index records
//! by uuid, then for each edit walk the `parentUuid` chain up to the closest
//! assistant prose and attach it as an [`Intent`], keyed by repo-relative path.
//! Confidence decays with the number of hops. Edits outside the repo are
//! dropped; files edited several times keep the most recent intent.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::domain::session::Intent;

use super::transcript::{Record, edit_tool};

/// Repo-relative path → recovered intent.
pub type IntentMap = HashMap<PathBuf, Intent>;

/// Maximum `parentUuid` hops before we give up on finding intent.
const MAX_HOPS: usize = 25;

/// Build the intent map for a parsed session, relative to `repo_root`.
pub fn build(records: &[Record], repo_root: &Path) -> IntentMap {
    let by_uuid: HashMap<&str, &Record> = records
        .iter()
        .filter_map(|r| Some((r.as_entry()?.uuid.as_deref()?, r)))
        .collect();

    let mut map = IntentMap::new();
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
                // Later edits overwrite earlier ones: keep the most recent intent.
                map.insert(
                    rel.clone(),
                    Intent {
                        file_path: rel,
                        text,
                        source_uuid,
                        confidence: confidence(hops),
                    },
                );
            }
        }
    }
    map
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
}
