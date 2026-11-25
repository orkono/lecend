//! The `edit`, `forget` and `unforget` subcommands: user corrections that
//! win over the model (spec 6.4).

use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};

use crate::apply::{collapse_ws, MAX_DEFINITION, MAX_LABEL, MAX_STATUS, NOW};
use crate::session;

/// One user correction of a label.
pub enum Action {
    Definition(String),
    Status(String),
    Pin(bool),
}

/// A label as typed by the user: whitespace-collapsed, with its identity key.
struct Label {
    text: String,
    key: String,
}

fn parse_label(raw: &str) -> Result<Label> {
    let text = collapse_ws(raw);
    if text.is_empty() {
        bail!("label is empty");
    }
    if text.chars().count() > MAX_LABEL {
        bail!("label is longer than {MAX_LABEL} characters");
    }
    let key = text.to_lowercase();
    Ok(Label { text, key })
}

fn parse_text(raw: &str, what: &str, max: usize) -> Result<String> {
    let text = raw.trim().to_string();
    if text.is_empty() {
        bail!("{what} is empty");
    }
    if text.chars().count() > max {
        bail!("{what} is longer than {max} characters");
    }
    Ok(text)
}

fn immediate(conn: &mut Connection) -> Result<Transaction<'_>> {
    Ok(conn.transaction_with_behavior(TransactionBehavior::Immediate)?)
}

/// Current definition version and text of the label, if it has a definition.
fn current_definition(tx: &Transaction, sid: &str, key: &str) -> Result<Option<(i64, String)>> {
    Ok(tx
        .query_row(
            "SELECT version, definition FROM definitions
             WHERE session_id = ?1 AND label_key = ?2 ORDER BY version DESC LIMIT 1",
            params![sid, key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?)
}

fn term_exists(tx: &Transaction, sid: &str, key: &str) -> Result<bool> {
    Ok(tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM terms WHERE session_id = ?1 AND label_key = ?2)",
        params![sid, key],
        |r| r.get(0),
    )?)
}

fn set_definition(tx: &Transaction, sid: &str, label: &Label, text: &str, turn: i64) -> Result<()> {
    let version = match current_definition(tx, sid, &label.key)? {
        Some((_, current)) if current.trim() == text => return Ok(()),
        Some((v, _)) => v + 1,
        None => 1,
    };
    if !term_exists(tx, sid, &label.key)? {
        tx.execute(
            "INSERT INTO terms (session_id, label, label_key, pinned, created_turn)
             VALUES (?1, ?2, ?3, 0, ?4)",
            params![sid, label.text, label.key, turn],
        )?;
    }
    tx.execute(
        &format!(
            "INSERT INTO definitions
               (session_id, label_key, version, definition, turn_index, quote, created_at, source)
             VALUES (?1, ?2, ?3, ?4, ?5, '', {NOW}, 'user')"
        ),
        params![sid, label.key, version, text, turn],
    )?;
    Ok(())
}

fn set_status(tx: &Transaction, sid: &str, label: &Label, text: &str, turn: i64) -> Result<()> {
    let Some((version, _)) = current_definition(tx, sid, &label.key)? else {
        bail!("no definition for {}", label.text);
    };
    tx.execute(
        &format!(
            "INSERT INTO statuses
               (session_id, label_key, definition_version, status, turn_index, quote,
                created_at, source)
             VALUES (?1, ?2, ?3, ?4, ?5, '', {NOW}, 'user')"
        ),
        params![sid, label.key, version, text, turn],
    )?;
    Ok(())
}

fn set_pinned(tx: &Transaction, sid: &str, label: &Label, pinned: bool) -> Result<()> {
    let changed = tx.execute(
        "UPDATE terms SET pinned = ?3 WHERE session_id = ?1 AND label_key = ?2",
        params![sid, label.key, pinned],
    )?;
    if changed == 0 {
        bail!("unknown label {}", label.text);
    }
    Ok(())
}

fn remove_ignore(tx: &Transaction, sid: &str, key: &str) -> Result<()> {
    tx.execute(
        "DELETE FROM ignores WHERE session_id = ?1 AND label_key = ?2",
        params![sid, key],
    )?;
    Ok(())
}

/// Applies one user correction; an edit also unforgets the label.
pub fn edit(conn: &mut Connection, sid: &str, raw_label: &str, action: Action) -> Result<()> {
    let label = parse_label(raw_label)?;
    let text = match &action {
        Action::Definition(t) => Some(parse_text(t, "definition", MAX_DEFINITION)?),
        Action::Status(t) => Some(parse_text(t, "status", MAX_STATUS)?),
        Action::Pin(_) => None,
    };
    let tx = immediate(conn)?;
    let turn = session::current_turn(&tx, sid)?;
    remove_ignore(&tx, sid, &label.key)?;
    match (action, text) {
        (Action::Definition(_), Some(t)) => set_definition(&tx, sid, &label, &t, turn)?,
        (Action::Status(_), Some(t)) => set_status(&tx, sid, &label, &t, turn)?,
        (Action::Pin(pinned), _) => set_pinned(&tx, sid, &label, pinned)?,
        _ => unreachable!("definition and status always carry text"),
    }
    tx.commit()?;
    Ok(())
}

/// Deletes the label with its history and ignores it from now on.
pub fn forget(conn: &mut Connection, sid: &str, raw_label: &str) -> Result<()> {
    let label = parse_label(raw_label)?;
    let tx = immediate(conn)?;
    for table in ["statuses", "definitions", "terms"] {
        tx.execute(
            &format!("DELETE FROM {table} WHERE session_id = ?1 AND label_key = ?2"),
            params![sid, label.key],
        )?;
    }
    tx.execute(
        "INSERT OR IGNORE INTO ignores (session_id, label_key) VALUES (?1, ?2)",
        params![sid, label.key],
    )?;
    tx.commit()?;
    Ok(())
}

/// Lets capture define the label again; deleted history does not come back.
pub fn unforget(conn: &Connection, sid: &str, raw_label: &str) -> Result<()> {
    let label = parse_label(raw_label)?;
    conn.execute(
        "DELETE FROM ignores WHERE session_id = ?1 AND label_key = ?2",
        params![sid, label.key],
    )?;
    Ok(())
}
