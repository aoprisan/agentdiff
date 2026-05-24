//! Recovering the agent's stated intent for each edited file (Copilot).
//!
//! Copilot gives us two "why" signals. The strongest is the explicit
//! `report_intent` tool the agent calls before acting — keyed here by `turnId`,
//! so an edit in the same turn inherits it at full confidence. Otherwise we fall
//! back to the nearest preceding `assistant.message` prose by walking the
//! `parentId` chain (the analog of the Claude [`intent`](super::super::intent)
//! path), with confidence decaying per hop. Edits outside the repo are dropped;
//! a file edited several times keeps the most recent intent.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::domain::session::Intent;

use super::events::{RawEvent, edit_path, edit_tool};

/// Repo-relative path → recovered intent.
pub type IntentMap = HashMap<PathBuf, Intent>;

/// Maximum `parentId` hops before we give up on finding intent.
const MAX_HOPS: usize = 25;

/// Build the intent map for a parsed Copilot session, relative to `repo_root`.
pub fn build(events: &[RawEvent], repo_root: &Path) -> IntentMap {
    let by_id: HashMap<&str, &RawEvent> = events
        .iter()
        .filter_map(|e| Some((e.id.as_deref()?, e)))
        .collect();

    // turnId → the agent's explicitly reported intent for that turn.
    let mut turn_intent: HashMap<&str, &str> = HashMap::new();
    for event in events {
        if let Some(("report_intent", args)) = event.tool_start()
            && let Some(intent) = args.get("intent").and_then(|v| v.as_str())
            && let Some(turn) = event.turn_id()
        {
            turn_intent.insert(turn, intent);
        }
    }

    let mut map = IntentMap::new();
    for event in events {
        let Some((name, args)) = event.tool_start() else {
            continue;
        };
        if edit_tool(name).is_none() {
            continue;
        }
        let Some(path_str) = edit_path(args) else {
            continue;
        };
        let Some(rel) = relativize(path_str, repo_root) else {
            continue; // edit outside the repo
        };

        // Prefer this turn's explicit report_intent, else the nearest assistant
        // prose up the parentId chain.
        let resolved = event
            .turn_id()
            .and_then(|t| turn_intent.get(t))
            .map(|text| ((*text).to_string(), event.id.clone().unwrap_or_default(), 0))
            .or_else(|| walk_to_intent(event, &by_id));

        if let Some((text, source_uuid, hops)) = resolved {
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
    map
}

/// Follow `parentId` from an edit event to the nearest assistant prose.
fn walk_to_intent(
    start: &RawEvent,
    by_id: &HashMap<&str, &RawEvent>,
) -> Option<(String, String, usize)> {
    let mut current = Some(start);
    for hops in 0..=MAX_HOPS {
        let event = current?;
        if let Some(text) = event.assistant_text() {
            return Some((text, event.id.clone().unwrap_or_default(), hops));
        }
        let parent = event.parent_id.as_deref();
        current = parent.and_then(|p| by_id.get(p).copied());
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
    use super::super::events::parse_reader;

    #[test]
    fn prefers_report_intent_then_falls_back_to_assistant_prose() {
        let jsonl = concat!(
            // Turn 1: explicit report_intent, then an edit in the same turn.
            r#"{"type":"assistant.message","id":"m1","data":{"turnId":"1","reasoningText":"thinking..."}}"#,
            "\n",
            r#"{"type":"tool.execution_start","id":"ri","parentId":"m1","data":{"toolName":"report_intent","turnId":"1","arguments":{"intent":"Add the greeting helper."}}}"#,
            "\n",
            r#"{"type":"tool.execution_start","id":"e1","parentId":"m1","data":{"toolName":"create","turnId":"1","arguments":{"path":"/repo/src/greet.rs"}}}"#,
            "\n",
            // Turn 2: no report_intent; edit resolves to assistant prose one hop up.
            r#"{"type":"assistant.message","id":"m2","data":{"turnId":"2","content":"Fixing the off-by-one."}}"#,
            "\n",
            r#"{"type":"tool.execution_start","id":"e2","parentId":"m2","data":{"toolName":"edit","turnId":"2","arguments":{"path":"/repo/src/lib.rs"}}}"#,
            "\n",
            // An out-of-repo edit is dropped.
            r#"{"type":"tool.execution_start","id":"e3","parentId":"m2","data":{"toolName":"edit","turnId":"2","arguments":{"path":"/outside/x.rs"}}}"#,
        );
        let map = build(&parse_reader(jsonl.as_bytes()), Path::new("/repo"));

        let greet = map.get(Path::new("src/greet.rs")).expect("greet intent");
        assert_eq!(greet.text, "Add the greeting helper.");
        assert!((greet.confidence - 1.0).abs() < 1e-6);

        let lib = map.get(Path::new("src/lib.rs")).expect("lib intent");
        assert_eq!(lib.text, "Fixing the off-by-one.");
        assert_eq!(lib.source_uuid, "m2");
        assert!((lib.confidence - 0.9).abs() < 1e-6);

        assert!(!map.contains_key(Path::new("x.rs")));
        assert_eq!(map.len(), 2);
    }
}
