//! The `scan` subcommand: decides whether a turn needs the model call and
//! gathers the context (known labels, relevant entries) for it.

use std::collections::HashSet;

use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::entry::{self, Entry};
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
    /// Set for a turn the person did not type (a notification): only known
    /// labels mentioned in it may trigger the model call, no new candidates.
    #[serde(default, rename = "mentionsOnly")]
    mentions_only: bool,
}

struct Scan {
    text: String,
    stoplist: Vec<String>,
    mentions_only: bool,
}

fn parse_input(raw: &str) -> Result<Scan> {
    let input: ScanInput = serde_json::from_str(raw).context("stdin is not a valid scan object")?;
    Ok(Scan {
        text: format!("{}\n{}", input.prompt, input.answer),
        stoplist: input.stoplist,
        mentions_only: input.mentions_only,
    })
}

/// Current entries of the session, most recently updated first; forgotten
/// labels and stoplist hits are left out.
fn load_entries(conn: &Connection, sid: &str, stop: &Stoplist) -> Result<Vec<Entry>> {
    Ok(entry::load(conn, sid)?
        .into_iter()
        .filter(|e| !stop.contains(&e.label))
        .collect())
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

/// Label-shaped tokens of `text` that are neither known nor forgotten.
fn new_candidates(
    conn: &Connection,
    sid: &str,
    text: &str,
    stop: &Stoplist,
    known: &[String],
) -> Result<Vec<String>> {
    let ignored = ignored_keys(conn, sid)?;
    let known_keys: HashSet<String> = known.iter().map(|l| l.to_lowercase()).collect();
    Ok(labels::candidates(text, stop)
        .into_iter()
        .filter(|c| {
            let key = c.to_lowercase();
            !ignored.contains(&key) && !known_keys.contains(&key)
        })
        .collect())
}

pub fn run(conn: &Connection, sid: &str, raw: &str) -> Result<Value> {
    let Scan {
        text,
        stoplist,
        mentions_only,
    } = parse_input(raw)?;
    let stop = Stoplist::new(&stoplist);
    let entries = load_entries(conn, sid, &stop)?;
    let known: Vec<String> = entries.iter().map(|e| e.label.clone()).collect();
    let mentioned = labels::mentions(&text, &known);
    let mentioned_keys: HashSet<String> = mentioned.iter().map(|l| l.to_lowercase()).collect();
    let candidates = if mentions_only {
        Vec::new()
    } else {
        new_candidates(conn, sid, &text, &stop, &known)?
    };

    let picked: Vec<Value> = pick_entries(&entries, &mentioned_keys)
        .into_iter()
        .map(|e| json!({ "label": e.label, "definition": e.definition, "status": e.status }))
        .collect();
    Ok(json!({
        "shouldCall": !mentioned.is_empty() || !candidates.is_empty(),
        "knownLabels": known.iter().take(MAX_KNOWN_LABELS).collect::<Vec<_>>(),
        "candidates": candidates,
        "mentioned": mentioned,
        "entries": picked,
    }))
}
