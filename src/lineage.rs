use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// True for a line that is a real user prompt: `type:"user"`, not meta, and
/// not a tool result (tool results are `user` lines too in Claude Code).
fn is_prompt(v: &Value) -> bool {
    if v.get("type").and_then(Value::as_str) != Some("user")
        || v.get("isMeta").and_then(Value::as_bool) == Some(true)
    {
        return false;
    }
    match v.pointer("/message/content") {
        Some(Value::String(_)) => true,
        Some(Value::Array(blocks)) => !blocks
            .iter()
            .any(|b| b.get("type").and_then(Value::as_str) == Some("tool_result")),
        _ => false,
    }
}

/// The `uuid`s of a transcript's real prompts, in file order. Unreadable
/// files and lines that are not JSON yield nothing.
fn prompt_uuids(path: &Path) -> Vec<String> {
    let Ok(bytes) = fs::read(path) else {
        return Vec::new();
    };
    String::from_utf8_lossy(&bytes)
        .lines()
        .filter(|line| line.contains("\"user\""))
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(is_prompt)
        .filter_map(|v| v.get("uuid").and_then(Value::as_str).map(str::to_string))
        .collect()
}

/// Sibling `*.jsonl` files of `transcript`, sorted by name, excluding itself.
fn siblings(transcript: &Path) -> Vec<PathBuf> {
    let dir = match transcript.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .filter(|p| p.file_name() != transcript.file_name())
        .collect();
    files.sort();
    files
}

/// Finds the transcript that shares this one's first prompt uuid and the
/// most prompts overall (its fork parent; ties go to the first by name).
/// Returns `{"parent": <session id>|null, "sharedUserTurns": N}`.
pub fn run(transcript: &Path) -> Value {
    let mine = prompt_uuids(transcript);
    let Some(first) = mine.first() else {
        return json!({ "parent": null, "sharedUserTurns": 0 });
    };
    let mut best: Option<(String, usize)> = None;
    for sibling in siblings(transcript) {
        let theirs: HashSet<String> = prompt_uuids(&sibling).into_iter().collect();
        let Some(stem) = sibling.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if !theirs.contains(first) {
            continue;
        }
        let shared = mine.iter().filter(|u| theirs.contains(*u)).count();
        if best.as_ref().map_or(true, |(_, n)| shared > *n) {
            best = Some((stem.to_string(), shared));
        }
    }
    match best {
        Some((parent, n)) => json!({ "parent": parent, "sharedUserTurns": n }),
        None => json!({ "parent": null, "sharedUserTurns": 0 }),
    }
}
