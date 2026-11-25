use assert_cmd::Command;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};

fn lecend(db: &std::path::Path, args: &[&str]) -> String {
    let mut full = vec![args[0], "--db", db.to_str().unwrap(), "--session", "s1"];
    full.extend(&args[1..]);
    let out = Command::cargo_bin("lecend")
        .unwrap()
        .args(full)
        .assert()
        .success()
        .get_output()
        .clone();
    String::from_utf8(out.stdout).unwrap()
}

fn fresh() -> (TempDir, std::path::PathBuf) {
    let dir = tempdir().unwrap();
    let db = dir.path().join("l.db");
    lecend(&db, &["doctor"]);
    (dir, db)
}

fn term(db: &std::path::Path, label: &str, turn: i64, pinned: bool) {
    Connection::open(db)
        .unwrap()
        .execute(
            "INSERT INTO terms (session_id,label,label_key,pinned,created_turn) VALUES ('s1',?1,?2,?3,?4)",
            params![label, label.to_lowercase(), pinned, turn],
        )
        .unwrap();
}

fn def(db: &std::path::Path, label: &str, version: i64, text: &str, turn: i64, src: &str, q: &str) {
    Connection::open(db)
        .unwrap()
        .execute(
            "INSERT INTO definitions (session_id,label_key,version,definition,turn_index,quote,source) VALUES ('s1',?1,?2,?3,?4,?5,?6)",
            params![label.to_lowercase(), version, text, turn, q, src],
        )
        .unwrap();
}

fn status(
    db: &std::path::Path,
    label: &str,
    version: i64,
    text: &str,
    turn: i64,
    src: &str,
    q: &str,
) {
    Connection::open(db)
        .unwrap()
        .execute(
            "INSERT INTO statuses (session_id,label_key,definition_version,status,turn_index,quote,source) VALUES ('s1',?1,?2,?3,?4,?5,?6)",
            params![label.to_lowercase(), version, text, turn, q, src],
        )
        .unwrap();
}

fn seeded_w6() -> (TempDir, std::path::PathBuf) {
    let (d, db) = fresh();
    term(&db, "W6", 3, true);
    def(&db, "W6", 1, "old window", 3, "model", "first");
    def(
        &db,
        "W6",
        2,
        "maintenance window: rebuild the panel only",
        5,
        "user",
        "rebuild only",
    );
    status(
        &db,
        "W6",
        2,
        "approved; opens on 61797a6",
        14,
        "user",
        "approved, go",
    );
    (d, db)
}

#[test]
fn show_formats_definition_and_status() {
    let (_d, db) = seeded_w6();
    let out = lecend(&db, &["show", "W6"]);
    assert_eq!(
        out,
        "W6  maintenance window: rebuild the panel only\n    status: approved; opens on 61797a6\n    previously: old window\n    source: \"approved, go\" (turn 14)\n    pinned\n"
    );
}

#[test]
fn show_without_status_uses_definition_quote() {
    let (_d, db) = fresh();
    term(&db, "A1", 2, false);
    def(&db, "A1", 1, "thing", 2, "model", "the thing");
    assert_eq!(
        lecend(&db, &["show", "A1"]),
        "A1  thing\n    source: \"the thing\" (turn 2)\n"
    );
}

#[test]
fn show_case_insensitive_fallback() {
    let (_d, db) = seeded_w6();
    assert!(lecend(&db, &["show", "w6"]).starts_with("W6  maintenance"));
}

#[test]
fn show_multiword_label() {
    let (_d, db) = fresh();
    term(&db, "Karar 1", 1, false);
    def(&db, "Karar 1", 1, "ship it", 1, "user", "q");
    assert!(lecend(&db, &["show", "Karar", "1"]).starts_with("Karar 1  ship it"));
}

#[test]
fn show_turkish_dotted_i() {
    let (_d, db) = fresh();
    term(&db, "İş", 1, false);
    def(&db, "İş", 1, "work", 1, "user", "q");
    assert!(lecend(&db, &["show", "İş"]).starts_with("İş  work"));
    assert!(lecend(&db, &["show", "i\u{307}ş"]).starts_with("İş  work"));
}

#[test]
fn show_missing_label() {
    let (_d, db) = fresh();
    assert_eq!(lecend(&db, &["show", "Nope"]), "No record for \"Nope\".\n");
}

#[test]
fn list_orders_by_last_update() {
    let (_d, db) = fresh();
    term(&db, "A1", 1, false);
    def(&db, "A1", 1, "first", 1, "model", "q");
    term(&db, "B2", 2, false);
    def(&db, "B2", 1, "second", 2, "model", "q");
    status(&db, "A1", 1, "done", 9, "user", "q");
    assert_eq!(
        lecend(&db, &["list"]),
        "A1  first \u{2014} done\nB2  second\n"
    );
}

#[test]
fn list_empty() {
    let (_d, db) = fresh();
    assert_eq!(lecend(&db, &["list"]), "No labels yet.\n");
}

#[test]
fn list_json_shape() {
    let (_d, db) = seeded_w6();
    let v: Value = serde_json::from_str(&lecend(&db, &["list", "--json"])).unwrap();
    assert_eq!(
        v,
        json!([{
            "label": "W6",
            "definition": "maintenance window: rebuild the panel only",
            "status": "approved; opens on 61797a6",
            "previously": "old window",
            "updatedTurn": 14,
            "pinned": true
        }])
    );
}

#[test]
fn history_lists_versions_with_sources_and_quotes() {
    let (_d, db) = seeded_w6();
    status(&db, "W6", 1, "proposed", 4, "model", "maybe");
    let out = lecend(&db, &["history", "W6"]);
    assert_eq!(
        out,
        "W6\n  v1 old window (turn 3, model: \"first\")\n    status: proposed (turn 4, model: \"maybe\")\n  v2 maintenance window: rebuild the panel only (turn 5, user: \"rebuild only\")\n    status: approved; opens on 61797a6 (turn 14, user: \"approved, go\")\n"
    );
}

#[test]
fn list_explains_fork_without_parent() {
    let (_d, db) = fresh();
    Connection::open(&db)
        .unwrap()
        .execute(
            "UPDATE sessions SET fork_turn = -1 WHERE session_id='s1'",
            [],
        )
        .unwrap();
    assert_eq!(
        lecend(&db, &["list"]),
        "No labels yet (no parent session found for this fork).\n"
    );
    term(&db, "A1", 1, false);
    def(&db, "A1", 1, "x", 1, "model", "q");
    assert_eq!(lecend(&db, &["list"]), "A1  x\n");
}
