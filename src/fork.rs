use anyhow::{bail, Result};
use rusqlite::{Connection, Transaction};
use serde_json::{json, Value};

use crate::session;

fn fork_done(conn: &Connection, sid: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT fork_done FROM sessions WHERE session_id = ?1",
        [sid],
        |r| r.get::<_, i64>(0),
    )? != 0)
}

fn has_own_data(conn: &Connection, sid: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT (SELECT count(*) FROM terms WHERE session_id = ?1)
              + (SELECT count(*) FROM turns WHERE session_id = ?1)
              + (SELECT count(*) FROM ignores WHERE session_id = ?1)",
        [sid],
        |r| r.get::<_, i64>(0),
    )? > 0)
}

/// Copies the parent's glossary rows into the child; returns rows inserted.
/// `cutoff` keeps only definitions/statuses with `turn_index <= cutoff`.
fn copy_rows(tx: &Transaction, parent: &str, child: &str, cutoff: Option<i64>) -> Result<usize> {
    let cut = cutoff.unwrap_or(i64::MAX);
    let p = rusqlite::params![parent, child, cut];
    let mut n = tx.execute(
        "INSERT INTO definitions
           (session_id, label_key, version, definition, turn_index, quote, created_at, source)
         SELECT ?2, label_key, version, definition, turn_index, quote, created_at, source
         FROM definitions WHERE session_id = ?1 AND COALESCE(turn_index, 0) <= ?3",
        p,
    )?;
    n += tx.execute(
        "INSERT INTO terms (session_id, label, label_key, pinned, created_turn)
         SELECT ?2, label, label_key, pinned, created_turn FROM terms t
         WHERE session_id = ?1
           AND EXISTS (SELECT 1 FROM definitions d
                       WHERE d.session_id = ?2 AND d.label_key = t.label_key)",
        rusqlite::params![parent, child],
    )?;
    n += tx.execute(
        "INSERT INTO statuses
           (session_id, label_key, definition_version, status, turn_index, quote,
            created_at, source)
         SELECT ?2, s.label_key, s.definition_version, s.status, s.turn_index, s.quote,
                s.created_at, s.source
         FROM statuses s
         WHERE s.session_id = ?1 AND COALESCE(s.turn_index, 0) <= ?3
           AND EXISTS (SELECT 1 FROM terms t
                       WHERE t.session_id = ?2 AND t.label_key = s.label_key)
           AND (s.definition_version IS NULL OR EXISTS (
                 SELECT 1 FROM definitions d
                 WHERE d.session_id = ?2 AND d.label_key = s.label_key
                   AND d.version = s.definition_version))",
        p,
    )?;
    n += tx.execute(
        "INSERT INTO ignores (session_id, label_key)
         SELECT ?2, label_key FROM ignores WHERE session_id = ?1",
        rusqlite::params![parent, child],
    )?;
    Ok(n)
}

fn max_copied_turn(tx: &Transaction, child: &str) -> Result<i64> {
    Ok(tx.query_row(
        "SELECT max(
           COALESCE((SELECT max(turn_index) FROM definitions WHERE session_id = ?1), 0),
           COALESCE((SELECT max(turn_index) FROM statuses WHERE session_id = ?1), 0))",
        [child],
        |r| r.get(0),
    )?)
}

/// Copies `parent`'s glossary into `child` exactly once.
pub fn from(conn: &Connection, child: &str, parent: &str, at_turn: Option<i64>) -> Result<Value> {
    if parent == child {
        bail!("a session cannot fork from itself");
    }
    if let Some(n) = at_turn {
        if n < 0 {
            bail!("--at-turn must not be negative");
        }
    }
    let tx = session::immediate(conn)?;
    if fork_done(&tx, child)? {
        return Ok(json!({ "copied": 0, "already": true }));
    }
    if has_own_data(&tx, child)? {
        bail!("session already has a glossary");
    }
    let parent_defs: i64 = tx.query_row(
        "SELECT count(*) FROM definitions WHERE session_id = ?1",
        [parent],
        |r| r.get(0),
    )?;
    if parent_defs == 0 {
        bail!("parent has no glossary");
    }
    let copied = copy_rows(&tx, parent, child, at_turn)?;
    let base = match at_turn {
        Some(n) => n,
        None => max_copied_turn(&tx, child)?,
    };
    tx.execute(
        "UPDATE sessions SET forked_from = ?2, fork_turn = ?3, turn_base = ?3, fork_done = 1
         WHERE session_id = ?1",
        rusqlite::params![child, parent, base],
    )?;
    tx.commit()?;
    Ok(json!({ "copied": copied, "already": false }))
}

/// Marks the session as having no detectable parent (once).
pub fn none(conn: &Connection, sid: &str) -> Result<Value> {
    let tx = session::immediate(conn)?;
    if fork_done(&tx, sid)? {
        return Ok(json!({ "ok": true, "already": true }));
    }
    if has_own_data(&tx, sid)? {
        bail!("session already has a glossary");
    }
    tx.execute(
        "UPDATE sessions SET fork_done = 1, forked_from = NULL, fork_turn = -1
         WHERE session_id = ?1",
        [sid],
    )?;
    tx.commit()?;
    Ok(json!({ "ok": true }))
}
