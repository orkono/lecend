//! The read-only `show`, `list` and `history` subcommands.

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};
use serde_json::{json, Value};

use crate::entry::{self, Entry};

/// Finds an entry by exact label, else by Unicode-lowercased key.
fn find(entries: Vec<Entry>, label: &str) -> Option<Entry> {
    let key = label.to_lowercase();
    let mut entries = entries;
    let pos = entries
        .iter()
        .position(|e| e.label == label)
        .or_else(|| entries.iter().position(|e| e.label_key == key))?;
    Some(entries.swap_remove(pos))
}

fn missing(label: &str) -> String {
    format!("No record for \"{label}\".")
}

/// Where the current text came from: the quote it was captured from, or a user edit.
struct Source {
    quote: String,
    turn: Option<i64>,
    by_user: bool,
}

impl Source {
    fn line(&self) -> String {
        let turn = self
            .turn
            .map(|t| format!(" (turn {t})"))
            .unwrap_or_default();
        if self.by_user {
            format!("    source: edited by you{turn}\n")
        } else {
            format!("    source: \"{}\"{turn}\n", self.quote)
        }
    }
}

/// Source of the current status, else of the current definition.
fn source(conn: &Connection, sid: &str, key: &str) -> Result<Option<Source>> {
    let row = |sql: &str| {
        conn.query_row(sql, rusqlite::params![sid, key], |r| {
            Ok(Source {
                quote: r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                turn: r.get::<_, Option<i64>>(1)?,
                by_user: r.get::<_, Option<String>>(2)?.as_deref() == Some("user"),
            })
        })
        .optional()
    };
    let status = row("SELECT quote, turn_index, source FROM statuses
         WHERE session_id = ?1 AND label_key = ?2
           AND definition_version IS (SELECT max(version) FROM definitions
                                      WHERE session_id = ?1 AND label_key = ?2)
         ORDER BY turn_index DESC, rowid DESC LIMIT 1")?;
    if status.is_some() {
        return Ok(status);
    }
    Ok(row(
        "SELECT quote, turn_index, source FROM definitions
         WHERE session_id = ?1 AND label_key = ?2 ORDER BY version DESC LIMIT 1",
    )?)
}

pub fn show(conn: &Connection, sid: &str, label: &str) -> Result<String> {
    let Some(e) = find(entry::load(conn, sid)?, label) else {
        return Ok(missing(label));
    };
    let mut out = format!("{}  {}\n", e.label, e.definition);
    if let Some(s) = &e.status {
        out.push_str(&format!("    status: {s}\n"));
    }
    if let Some(p) = &e.previously {
        out.push_str(&format!("    previously: {p}\n"));
    }
    if let Some(src) = source(conn, sid, &e.label_key)? {
        out.push_str(&src.line());
    }
    if e.pinned {
        out.push_str("    pinned\n");
    }
    Ok(out.trim_end().to_string())
}

fn fork_without_parent(conn: &Connection, sid: &str) -> Result<bool> {
    let fork_turn: Option<i64> = conn
        .query_row(
            "SELECT fork_turn FROM sessions WHERE session_id = ?1",
            [sid],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    Ok(fork_turn == Some(-1))
}

pub fn list(conn: &Connection, sid: &str, as_json: bool) -> Result<String> {
    let entries = entry::load(conn, sid)?;
    if as_json {
        let items: Vec<Value> = entries
            .iter()
            .map(|e| {
                json!({
                    "label": e.label,
                    "definition": e.definition,
                    "status": e.status,
                    "previously": e.previously,
                    "updatedTurn": e.updated,
                    "pinned": e.pinned,
                })
            })
            .collect();
        return Ok(Value::Array(items).to_string());
    }
    if entries.is_empty() {
        return Ok(if fork_without_parent(conn, sid)? {
            "No labels yet (no parent session found for this fork).".to_string()
        } else {
            "No labels yet.".to_string()
        });
    }
    let lines: Vec<String> = entries
        .iter()
        .map(|e| match &e.status {
            Some(s) => format!("{}  {} \u{2014} {}", e.label, e.definition, s),
            None => format!("{}  {}", e.label, e.definition),
        })
        .collect();
    Ok(lines.join("\n"))
}

fn turn_text(turn: Option<i64>) -> String {
    turn.map(|t| format!("turn {t}"))
        .unwrap_or_else(|| "turn ?".to_string())
}

fn source_text(src: Option<String>, quote: Option<String>) -> String {
    format!(
        "{}: \"{}\"",
        src.unwrap_or_else(|| "unknown".to_string()),
        quote.unwrap_or_default()
    )
}

type Def = (i64, String, Option<i64>, Option<String>, Option<String>);

pub fn history(conn: &Connection, sid: &str, label: &str) -> Result<String> {
    let Some(e) = find(entry::load(conn, sid)?, label) else {
        return Ok(missing(label));
    };
    let mut defs = conn.prepare(
        "SELECT version, definition, turn_index, source, quote FROM definitions
         WHERE session_id = ?1 AND label_key = ?2 ORDER BY version",
    )?;
    let defs: Vec<Def> = defs
        .query_map(rusqlite::params![sid, e.label_key], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut stat = conn.prepare(
        "SELECT status, turn_index, source, quote FROM statuses
         WHERE session_id = ?1 AND label_key = ?2 AND definition_version IS ?3
         ORDER BY turn_index, rowid",
    )?;
    let mut out = vec![e.label.clone()];
    for (version, definition, turn, src, quote) in defs {
        out.push(format!(
            "  v{version} {definition} ({}, {})",
            turn_text(turn),
            source_text(src, quote)
        ));
        let rows = stat.query_map(rusqlite::params![sid, e.label_key, version], |r| {
            Ok((r.get::<_, String>(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?;
        for row in rows {
            let (status, turn, src, quote) = row?;
            out.push(format!(
                "    status: {status} ({}, {})",
                turn_text(turn),
                source_text(src, quote)
            ));
        }
    }
    Ok(out.join("\n"))
}
