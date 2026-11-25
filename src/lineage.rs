use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// The `uuid`s of a transcript's `type:"user"` lines, in file order.
/// Unreadable files and lines that are not JSON yield nothing.
fn user_uuids(path: &Path) -> Vec<String> {
    let Ok(bytes) = fs::read(path) else {
        return Vec::new();
    };
    String::from_utf8_lossy(&bytes)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|v| v.get("type").and_then(Value::as_str) == Some("user"))
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

/// Finds the transcript that shares this one's first user uuid (its fork
/// parent). Returns `{"parent": <session id>|null, "sharedUserTurns": N}`.
pub fn run(transcript: &Path) -> Value {
    let none = json!({ "parent": null, "sharedUserTurns": 0 });
    let mine = user_uuids(transcript);
    let Some(first) = mine.first() else {
        return none;
    };
    for sibling in siblings(transcript) {
        let theirs: HashSet<String> = user_uuids(&sibling).into_iter().collect();
        if !theirs.contains(first) {
            continue;
        }
        let Some(stem) = sibling.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let shared = mine.iter().filter(|u| theirs.contains(*u)).count();
        return json!({ "parent": stem, "sharedUserTurns": shared });
    }
    none
}
