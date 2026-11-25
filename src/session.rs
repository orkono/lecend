use anyhow::Result;
use rusqlite::Connection;

/// Inserts the `sessions` row for `sid` if it is missing.
pub fn ensure(conn: &Connection, sid: &str) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO sessions (session_id) VALUES (?1)",
        [sid],
    )?;
    Ok(())
}

use rusqlite::{OptionalExtension, Transaction, TransactionBehavior};
use serde_json::{json, Value};

fn immediate(conn: &Connection) -> Result<Transaction<'_>> {
    Ok(Transaction::new_unchecked(
        conn,
        TransactionBehavior::Immediate,
    )?)
}

/// Highest turn index of the session, floored by `turn_base`; 0 when none.
pub fn current_turn(conn: &Connection, sid: &str) -> Result<i64> {
    Ok(conn
        .query_row(
            "SELECT max(
           (SELECT turn_base FROM sessions WHERE session_id = ?1),
           COALESCE((SELECT max(turn_index) FROM turns WHERE session_id = ?1), 0))",
            [sid],
            |r| r.get::<_, Option<i64>>(0),
        )?
        .unwrap_or(0))
}

/// Index recorded for `turn_id`, if the turn is known.
pub fn turn_index_of(conn: &Connection, sid: &str, turn_id: &str) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT turn_index FROM turns WHERE session_id = ?1 AND turn_id = ?2",
            [sid, turn_id],
            |r| r.get(0),
        )
        .optional()?)
}

/// Records `turn_id` (idempotently) and returns its index.
pub fn record_turn(conn: &Connection, sid: &str, turn_id: &str) -> Result<i64> {
    let tx = immediate(conn)?;
    let index = match turn_index_of(&tx, sid, turn_id)? {
        Some(i) => i,
        None => {
            let next = current_turn(&tx, sid)? + 1;
            tx.execute(
                "INSERT INTO turns (session_id, turn_id, turn_index, recorded_at)
                 VALUES (?1, ?2, ?3, strftime('%Y-%m-%dT%H:%M:%SZ','now'))",
                rusqlite::params![sid, turn_id, next],
            )?;
            next
        }
    };
    tx.commit()?;
    Ok(index)
}

/// Read-only snapshot of the session's state.
pub fn state(conn: &Connection, sid: &str) -> Result<Value> {
    let (off, calls, from, fork_turn, done): (i64, i64, Option<String>, Option<i64>, i64) = conn
        .query_row(
            "SELECT capture_off, model_calls, forked_from, fork_turn, fork_done
             FROM sessions WHERE session_id = ?1",
            [sid],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )?;
    let count = |table: &str| -> Result<i64> {
        Ok(conn.query_row(
            &format!("SELECT count(*) FROM {table} WHERE session_id = ?1"),
            [sid],
            |r| r.get(0),
        )?)
    };
    Ok(json!({
        "captureOff": off != 0,
        "modelCalls": calls,
        "forkedFrom": from,
        "forkTurn": fork_turn,
        "forkDone": done != 0,
        "labels": count("terms")?,
        "turns": count("turns")?,
    }))
}

/// Increments the model-call counter and returns the new value.
pub fn bump_usage(conn: &Connection, sid: &str) -> Result<i64> {
    Ok(conn.query_row(
        "UPDATE sessions SET model_calls = model_calls + 1
         WHERE session_id = ?1 RETURNING model_calls",
        [sid],
        |r| r.get(0),
    )?)
}

/// Sets the capture flag and returns it.
pub fn set_capture_off(conn: &Connection, sid: &str, off: bool) -> Result<bool> {
    let v: i64 = conn.query_row(
        "UPDATE sessions SET capture_off = ?2 WHERE session_id = ?1 RETURNING capture_off",
        rusqlite::params![sid, off as i64],
        |r| r.get(0),
    )?;
    Ok(v != 0)
}

/// Returns whether the cost notice was already shown; marks it shown atomically.
pub fn check_and_set_notice(conn: &Connection) -> Result<bool> {
    let tx = immediate(conn)?;
    let value: String = tx.query_row(
        "SELECT value FROM meta WHERE key = 'cost_notice_shown'",
        [],
        |r| r.get(0),
    )?;
    let shown = value == "1";
    if !shown {
        tx.execute(
            "UPDATE meta SET value = '1' WHERE key = 'cost_notice_shown'",
            [],
        )?;
    }
    tx.commit()?;
    Ok(shown)
}
