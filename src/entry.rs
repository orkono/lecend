//! Current glossary entries of a session, shared by `scan` and the query commands.

use anyhow::Result;
use rusqlite::Connection;

/// A glossary entry in its current state.
pub struct Entry {
    pub label: String,
    pub label_key: String,
    pub definition: String,
    pub status: Option<String>,
    /// The newest definition older than the current one, if any.
    pub previously: Option<String>,
    pub pinned: bool,
    /// Highest turn index across the label's creation, definitions and statuses.
    pub updated: i64,
}

/// Current entries of the session, most recently updated first; forgotten
/// (ignored) labels are left out. Current definition = highest version;
/// current status = newest status of that definition version.
pub fn load(conn: &Connection, sid: &str) -> Result<Vec<Entry>> {
    let mut stmt = conn.prepare(
        "SELECT t.label, t.label_key,
                COALESCE((SELECT d.definition FROM definitions d
                          WHERE d.session_id = t.session_id AND d.label_key = t.label_key
                          ORDER BY d.version DESC LIMIT 1), ''),
                (SELECT s.status FROM statuses s
                 WHERE s.session_id = t.session_id AND s.label_key = t.label_key
                   AND s.definition_version IS
                       (SELECT max(d.version) FROM definitions d
                        WHERE d.session_id = t.session_id AND d.label_key = t.label_key)
                 ORDER BY s.turn_index DESC, s.rowid DESC LIMIT 1),
                (SELECT d.definition FROM definitions d
                 WHERE d.session_id = t.session_id AND d.label_key = t.label_key
                 ORDER BY d.version DESC LIMIT 1 OFFSET 1),
                t.pinned,
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
            label_key: r.get(1)?,
            definition: r.get(2)?,
            status: r.get(3)?,
            previously: r.get(4)?,
            pinned: r.get::<_, i64>(5)? != 0,
            updated: r.get(6)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}
