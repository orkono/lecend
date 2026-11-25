use std::path::Path;

use anyhow::{bail, Context, Result};
use rusqlite::Connection;

pub const SCHEMA_VERSION: i64 = 1;

const SCHEMA_V1: &str = "
CREATE TABLE terms (
  session_id TEXT NOT NULL,
  label TEXT NOT NULL,
  label_key TEXT NOT NULL,
  pinned INTEGER NOT NULL DEFAULT 0,
  created_turn INTEGER,
  PRIMARY KEY (session_id, label_key)
);
CREATE TABLE definitions (
  session_id TEXT NOT NULL,
  label_key TEXT NOT NULL,
  version INTEGER NOT NULL,
  definition TEXT NOT NULL,
  turn_index INTEGER,
  quote TEXT,
  created_at TEXT,
  source TEXT,
  PRIMARY KEY (session_id, label_key, version)
);
CREATE TABLE statuses (
  session_id TEXT NOT NULL,
  label_key TEXT NOT NULL,
  definition_version INTEGER,
  status TEXT NOT NULL,
  turn_index INTEGER,
  quote TEXT,
  created_at TEXT,
  source TEXT
);
CREATE TABLE ignores (
  session_id TEXT NOT NULL,
  label_key TEXT NOT NULL,
  PRIMARY KEY (session_id, label_key)
);
CREATE TABLE turns (
  session_id TEXT NOT NULL,
  turn_id TEXT NOT NULL,
  turn_index INTEGER NOT NULL,
  recorded_at TEXT,
  applied_at TEXT,
  PRIMARY KEY (session_id, turn_id),
  UNIQUE (session_id, turn_index)
);
CREATE TABLE sessions (
  session_id TEXT PRIMARY KEY,
  forked_from TEXT,
  fork_turn INTEGER,
  turn_base INTEGER NOT NULL DEFAULT 0,
  fork_done INTEGER NOT NULL DEFAULT 0,
  capture_off INTEGER NOT NULL DEFAULT 0,
  model_calls INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE rejections (
  session_id TEXT NOT NULL,
  turn_id TEXT,
  op TEXT,
  label TEXT,
  reason TEXT,
  payload TEXT,
  created_at TEXT
);
CREATE TABLE meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
INSERT INTO meta (key, value) VALUES ('schema_version', '1'), ('cost_notice_shown', '0');
";

/// Opens (creating if needed) the database at `path` and migrates it to the current schema.
pub fn open(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create directory {}", parent.display()))?;
    }
    restrict_permissions(path)?;
    let conn = Connection::open(path)
        .with_context(|| format!("cannot open database {}", path.display()))?;
    conn.busy_timeout(std::time::Duration::from_millis(5000))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    migrate(&conn)?;
    Ok(conn)
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    // Create the file with mode 0600 up front; an existing file is left untouched.
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("cannot create database file {}", path.display()))?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

fn migrate(conn: &Connection) -> Result<()> {
    let has_meta: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='meta')",
        [],
        |r| r.get(0),
    )?;
    if !has_meta {
        // IMMEDIATE so parallel first-openers serialize; the loser re-checks inside the lock.
        conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = create_if_still_missing(conn);
        match result {
            Ok(()) => conn.execute_batch("COMMIT")?,
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(e);
            }
        }
    }
    let version: i64 = conn
        .query_row(
            "SELECT value FROM meta WHERE key='schema_version'",
            [],
            |r| r.get::<_, String>(0),
        )?
        .parse()
        .context("corrupt schema_version in meta")?;
    if version != SCHEMA_VERSION {
        bail!("unsupported schema version {version} (this lecend supports {SCHEMA_VERSION})");
    }
    Ok(())
}

fn create_if_still_missing(conn: &Connection) -> Result<()> {
    let has_meta: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='meta')",
        [],
        |r| r.get(0),
    )?;
    if !has_meta {
        conn.execute_batch(SCHEMA_V1)?;
    }
    Ok(())
}
