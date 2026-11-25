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
