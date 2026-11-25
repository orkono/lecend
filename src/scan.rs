//! The `scan` subcommand: decides whether a turn needs the model call and
//! gathers the context (known labels, relevant entries) for it.

use std::collections::HashSet;

use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::labels::{self, Stoplist};

const MAX_KNOWN_LABELS: usize = 50;
const MAX_ENTRIES: usize = 12;

#[derive(Deserialize)]
struct ScanInput {
    #[serde(default)]
    prompt: String,
    #[serde(default)]
    answer: String,
    #[serde(default)]
    stoplist: Vec<String>,
}

/// A glossary entry as the model sees it.
struct Entry {
    label: String,
    definition: String,
    status: Option<String>,
}

pub fn parse_input(raw: &str) -> Result<(String, Vec<String>)> {
    let input: ScanInput = serde_json::from_str(raw).context("stdin is not a valid scan object")?;
    Ok((
        format!("{}\n{}", input.prompt, input.answer),
        input.stoplist,
    ))
}

/// Current entries of the session, most recently updated first; forgotten
/// labels and stoplist hits are left out.
fn load_entries(conn: &Connection, sid: &str, stop: &Stoplist) -> Result<Vec<Entry>> {
    let mut stmt = conn.prepare(
        "SELECT t.label,
                COALESCE((SELECT d.definition FROM definitions d
                          WHERE d.session_id = t.session_id AND d.label_key = t.label_key
                          ORDER BY d.version DESC LIMIT 1), ''),
                (SELECT s.status FROM statuses s
                 WHERE s.session_id = t.session_id AND s.label_key = t.label_key
                   AND s.definition_version IS
                       (SELECT max(d.version) FROM definitions d
                        WHERE d.session_id = t.session_id AND d.label_key = t.label_key)
                 ORDER BY s.turn_index DESC, s.rowid DESC LIMIT 1),
                max(COALESCE(t.created_turn, 0),
                    COALESCE((SELECT max(d.turn_index) FROM definitions d
                              WHERE d.session_id = t.session_id AND d.label_key = t.label_key), 0),
                    COALESCE((SELECT max(s.turn_index) FROM statuses s
                              WHERE s.session_id = t.session_id AND s.label_key = t.label_key), 0))
                AS updated
         FROM terms t
         WHERE t.session_id = ?1
           AND t.label_key NOT IN (SELECT label_key FROM ignores WHERE session_id = ?1)
         ORDER BY updated DESC, t.rowid DESC",
    )?;
    let rows = stmt.query_map([sid], |r| {
        Ok(Entry {
            label: r.get(0)?,
            definition: r.get(1)?,
            status: r.get(2)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        let entry = row?;
        if !stop.contains(&entry.label) {
            out.push(entry);
        }
    }
    Ok(out)
}

fn ignored_keys(conn: &Connection, sid: &str) -> Result<HashSet<String>> {
    let mut stmt = conn.prepare("SELECT label_key FROM ignores WHERE session_id = ?1")?;
    let keys = stmt
        .query_map([sid], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<HashSet<_>>>()?;
    Ok(keys)
}

/// Mentioned entries first, then the most recently updated others, at most `MAX_ENTRIES`.
fn pick_entries<'a>(entries: &'a [Entry], mentioned: &HashSet<String>) -> Vec<&'a Entry> {
    let (hit, rest): (Vec<&Entry>, Vec<&Entry>) = entries
        .iter()
        .partition(|e| mentioned.contains(&e.label.to_lowercase()));
    hit.into_iter().chain(rest).take(MAX_ENTRIES).collect()
}

pub fn run(conn: &Connection, sid: &str, raw: &str) -> Result<Value> {
    let (text, user_stop) = parse_input(raw)?;
    let stop = Stoplist::new(&user_stop);
    let entries = load_entries(conn, sid, &stop)?;
    let known: Vec<String> = entries.iter().map(|e| e.label.clone()).collect();
    let mentioned = labels::mentions(&text, &known);
    let mentioned_keys: HashSet<String> = mentioned.iter().map(|l| l.to_lowercase()).collect();

    let ignored = ignored_keys(conn, sid)?;
    let known_keys: HashSet<String> = known.iter().map(|l| l.to_lowercase()).collect();
    let candidates: Vec<String> = labels::candidates(&text, &stop)
        .into_iter()
        .filter(|c| {
            let key = c.to_lowercase();
            !ignored.contains(&key) && !known_keys.contains(&key)
        })
        .collect();

    let picked: Vec<Value> = pick_entries(&entries, &mentioned_keys)
        .into_iter()
        .map(|e| json!({ "label": e.label, "definition": e.definition, "status": e.status }))
        .collect();
    Ok(json!({
        "shouldCall": !mentioned.is_empty() || !candidates.is_empty(),
        "knownLabels": known.iter().take(MAX_KNOWN_LABELS).collect::<Vec<_>>(),
        "candidates": candidates,
        "entries": picked,
    }))
}
