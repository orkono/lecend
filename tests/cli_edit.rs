use assert_cmd::Command;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tempfile::{tempdir, TempDir};

const W6_ANSWER: &str = "W6 is the maintenance window on Saturday night.";

fn fresh() -> (TempDir, PathBuf) {
    let dir = tempdir().unwrap();
    let db = dir.path().join("l.db");
    (dir, db)
}

fn run(db: &Path, args: &[&str], stdin: &str) -> std::process::Output {
    let mut full = vec![args[0], "--db", db.to_str().unwrap(), "--session", "s1"];
    full.extend(&args[1..]);
    Command::cargo_bin("lecend")
        .unwrap()
        .args(full)
        .write_stdin(stdin.to_string())
        .output()
        .unwrap()
}

/// Runs a command that must succeed and returns its stdout as JSON.
fn ok(db: &Path, args: &[&str]) -> Value {
    let out = run(db, args, "");
    assert!(out.status.success(), "{args:?} failed: {out:?}");
    serde_json::from_slice(&out.stdout).expect("stdout is JSON")
}

/// Runs a command that must fail and returns the error message.
fn err(db: &Path, args: &[&str]) -> String {
    let out = run(db, args, "");
    assert_eq!(out.status.code(), Some(1), "{args:?} should fail: {out:?}");
    let v: Value = serde_json::from_slice(&out.stdout).expect("stdout is JSON");
    v["error"].as_str().unwrap().to_string()
}

fn turn(db: &Path, id: &str) -> i64 {
    let out = run(db, &["turn"], &json!({ "turnId": id }).to_string());
    assert!(out.status.success(), "turn failed: {out:?}");
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    v["turnIndex"].as_i64().unwrap()
}

fn apply(db: &Path, turn_id: &str, answer: &str, diff: Value) -> Value {
    let input = json!({ "turnId": turn_id, "prompt": "", "answer": answer, "diff": diff });
    let out = run(db, &["apply"], &input.to_string());
    assert!(out.status.success(), "apply failed: {out:?}");
    serde_json::from_slice(&out.stdout).unwrap()
}

/// Records turn `t-1` and defines W6 from it (version 1, model).
fn define_w6(db: &Path) {
    turn(db, "t-1");
    let diff = json!({"new":[{"label":"W6","definition":"Saturday night maintenance window","quote":W6_ANSWER}]});
    assert_eq!(apply(db, "t-1", W6_ANSWER, diff)["applied"], 1);
}

fn model_status(db: &Path, turn_id: &str, status: &str) -> Value {
    let answer = format!("W6 is {status}.");
    let diff = json!({"status":[{"label":"W6","status":status,"quote":answer}]});
    apply(db, turn_id, &answer, diff)
}

fn conn(db: &Path) -> Connection {
    Connection::open(db).unwrap()
}

fn count(db: &Path, sql: &str) -> i64 {
    conn(db).query_row(sql, [], |r| r.get(0)).unwrap()
}

type DefRow = (i64, String, String, i64, String);

fn definitions(db: &Path, key: &str) -> Vec<DefRow> {
    let c = conn(db);
    let mut stmt = c
        .prepare(
            "SELECT version, definition, source, turn_index, quote FROM definitions
             WHERE session_id='s1' AND label_key=?1 ORDER BY version",
        )
        .unwrap();
    stmt.query_map([key], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

/// (definition_version, status, source, turn_index) in insertion order.
fn statuses(db: &Path, key: &str) -> Vec<(i64, String, String, i64)> {
    let c = conn(db);
    let mut stmt = c
        .prepare(
            "SELECT definition_version, status, source, turn_index FROM statuses
             WHERE session_id='s1' AND label_key=?1 ORDER BY rowid",
        )
        .unwrap();
    stmt.query_map([key], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn row(v: i64, d: &str, src: &str, t: i64) -> DefRow {
    (v, d.to_string(), src.to_string(), t, String::new())
}

#[test]
fn edit_definition_creates_user_version() {
    let (_d, db) = fresh();
    define_w6(&db);
    turn(&db, "t-2");
    let v = ok(
        &db,
        &["edit", "--definition", "Sunday morning window", "W6"],
    );
    assert_eq!(v, json!({"ok": true}));
    let defs = definitions(&db, "w6");
    assert_eq!(defs.len(), 2);
    assert_eq!(defs[1], row(2, "Sunday morning window", "user", 2));
}

#[test]
fn edit_definition_creates_missing_label() {
    let (_d, db) = fresh();
    turn(&db, "t-1");
    ok(
        &db,
        &["edit", "--definition", "the  old plan", "Old", "  Plan"],
    );
    assert_eq!(
        count(&db, "SELECT count(*) FROM terms WHERE label='Old Plan' AND label_key='old plan' AND pinned=0 AND created_turn=1"),
        1
    );
    assert_eq!(
        definitions(&db, "old plan"),
        [row(1, "the  old plan", "user", 1)]
    );
}

#[test]
fn edit_definition_with_same_text_is_noop() {
    let (_d, db) = fresh();
    define_w6(&db);
    ok(
        &db,
        &[
            "edit",
            "--definition",
            "Saturday night maintenance window",
            "w6",
        ],
    );
    assert_eq!(definitions(&db, "w6").len(), 1);
}

#[test]
fn edit_status_attaches_to_current_version_as_user() {
    let (_d, db) = fresh();
    define_w6(&db);
    turn(&db, "t-2");
    ok(&db, &["edit", "--status", "blocked on DBA", "W6"]);
    assert_eq!(
        statuses(&db, "w6"),
        [(1, "blocked on DBA".to_string(), "user".to_string(), 2)]
    );
}

#[test]
fn edit_status_without_definition_fails() {
    let (_d, db) = fresh();
    assert_eq!(
        err(&db, &["edit", "--status", "done", "Nope"]),
        "no definition for Nope"
    );
    assert_eq!(count(&db, "SELECT count(*) FROM statuses"), 0);
}

#[test]
fn edit_requires_exactly_one_action_and_valid_text() {
    let (_d, db) = fresh();
    define_w6(&db);
    err(&db, &["edit", "W6"]);
    err(&db, &["edit", "--pin", "--unpin", "W6"]);
    err(&db, &["edit", "--definition", "x", "--status", "y", "W6"]);
    err(&db, &["edit", "--definition", "   ", "W6"]);
    err(&db, &["edit", "--definition", &"x".repeat(201), "W6"]);
    err(&db, &["edit", "--status", &"x".repeat(201), "W6"]);
    err(&db, &["edit", "--definition", "ok", &"L".repeat(33)]);
    assert_eq!(definitions(&db, "w6").len(), 1);
}

#[test]
fn user_status_beats_later_model_apply_of_older_turn() {
    let (_d, db) = fresh();
    define_w6(&db);
    turn(&db, "t-2");
    turn(&db, "t-3");
    ok(&db, &["edit", "--status", "approved by user", "W6"]);
    let v = model_status(&db, "t-2", "rolled back");
    assert_eq!(v["applied"], 0);
    assert_eq!(v["rejected"][0]["reason"], "not-newer");
    assert_eq!(statuses(&db, "w6").len(), 1);
}

#[test]
fn model_apply_of_newer_turn_updates_status_after_user_edit() {
    let (_d, db) = fresh();
    define_w6(&db);
    turn(&db, "t-2");
    ok(&db, &["edit", "--status", "approved by user", "W6"]);
    turn(&db, "t-3");
    let v = model_status(&db, "t-3", "rolled back");
    assert_eq!(v["applied"], 1);
    let st = statuses(&db, "w6");
    assert_eq!(st.len(), 2);
    assert_eq!(
        st[1],
        (1, "rolled back".to_string(), "model".to_string(), 3)
    );
}

#[test]
fn pin_blocks_model() {
    let (_d, db) = fresh();
    define_w6(&db);
    ok(&db, &["edit", "--pin", "W6"]);
    assert_eq!(
        count(&db, "SELECT pinned FROM terms WHERE label_key='w6'"),
        1
    );
    turn(&db, "t-2");
    let v = model_status(&db, "t-2", "blocked");
    assert_eq!(v["rejected"][0]["reason"], "pinned");
    ok(&db, &["edit", "--unpin", "w6"]);
    assert_eq!(
        count(&db, "SELECT pinned FROM terms WHERE label_key='w6'"),
        0
    );
    turn(&db, "t-3");
    assert_eq!(model_status(&db, "t-3", "blocked")["applied"], 1);
}

#[test]
fn pin_unknown_label_fails() {
    let (_d, db) = fresh();
    assert_eq!(err(&db, &["edit", "--pin", "Nope"]), "unknown label Nope");
    err(&db, &["edit", "--unpin", "Nope"]);
}

#[test]
fn forget_deletes_and_ignores() {
    let (_d, db) = fresh();
    define_w6(&db);
    turn(&db, "t-2");
    ok(&db, &["edit", "--status", "approved", "W6"]);
    assert_eq!(ok(&db, &["forget", "W6"]), json!({"ok": true}));
    for t in ["terms", "definitions", "statuses"] {
        assert_eq!(count(&db, &format!("SELECT count(*) FROM {t}")), 0, "{t}");
    }
    assert_eq!(
        count(&db, "SELECT count(*) FROM ignores WHERE label_key='w6'"),
        1
    );
}

#[test]
fn forget_unknown_label_still_ignores_it() {
    let (_d, db) = fresh();
    ok(&db, &["forget", "Ghost"]);
    ok(&db, &["forget", "ghost"]);
    assert_eq!(count(&db, "SELECT count(*) FROM ignores"), 1);
    turn(&db, "t-1");
    let answer = "Ghost is the maintenance window on Saturday night.";
    let diff = json!({"new":[{"label":"Ghost","definition":"Saturday maintenance window","quote":answer}]});
    assert_eq!(
        apply(&db, "t-1", answer, diff)["rejected"][0]["reason"],
        "ignored"
    );
}

#[test]
fn forgotten_label_not_recaptured_by_apply() {
    let (_d, db) = fresh();
    define_w6(&db);
    ok(&db, &["forget", "W6"]);
    turn(&db, "t-2");
    let diff = json!({"new":[{"label":"W6","definition":"Saturday night maintenance window","quote":W6_ANSWER}]});
    let v = apply(&db, "t-2", W6_ANSWER, diff);
    assert_eq!(v["applied"], 0);
    assert_eq!(v["rejected"][0]["reason"], "ignored");
    assert_eq!(count(&db, "SELECT count(*) FROM terms"), 0);
}

#[test]
fn unforget_allows_capture_but_history_gone() {
    let (_d, db) = fresh();
    define_w6(&db);
    turn(&db, "t-2");
    ok(&db, &["edit", "--status", "approved", "W6"]);
    ok(&db, &["forget", "W6"]);
    assert_eq!(ok(&db, &["unforget", "w6"]), json!({"ok": true}));
    assert_eq!(count(&db, "SELECT count(*) FROM ignores"), 0);
    assert_eq!(count(&db, "SELECT count(*) FROM definitions"), 0);
    turn(&db, "t-3");
    let diff = json!({"new":[{"label":"W6","definition":"Saturday night maintenance window","quote":W6_ANSWER}]});
    assert_eq!(apply(&db, "t-3", W6_ANSWER, diff)["applied"], 1);
    assert_eq!(definitions(&db, "w6").len(), 1);
    assert!(statuses(&db, "w6").is_empty());
}

#[test]
fn unforget_of_non_ignored_label_is_noop() {
    let (_d, db) = fresh();
    define_w6(&db);
    assert_eq!(ok(&db, &["unforget", "W6"]), json!({"ok": true}));
    assert_eq!(definitions(&db, "w6").len(), 1);
}

#[test]
fn edit_unforgets() {
    let (_d, db) = fresh();
    define_w6(&db);
    ok(&db, &["forget", "W6"]);
    turn(&db, "t-2");
    ok(&db, &["edit", "--definition", "back again", "W6"]);
    assert_eq!(count(&db, "SELECT count(*) FROM ignores"), 0);
    assert_eq!(definitions(&db, "w6"), [row(1, "back again", "user", 2)]);
}

#[test]
fn failed_edit_does_not_unforget() {
    let (_d, db) = fresh();
    ok(&db, &["forget", "W6"]);
    err(&db, &["edit", "--status", "done", "W6"]);
    assert_eq!(count(&db, "SELECT count(*) FROM ignores"), 1);
}

/// A forked child starts at `turn_base` with the parent's rows copied at that
/// turn (simulated here; `fork` itself comes later).
#[test]
fn edit_right_after_fork_wins_over_copied_status() {
    let (_d, db) = fresh();
    ok(&db, &["doctor"]);
    let c = conn(&db);
    c.execute(
        "UPDATE sessions SET turn_base = 7 WHERE session_id='s1'",
        [],
    )
    .unwrap();
    c.execute("INSERT INTO terms VALUES ('s1','W6','w6',0,7)", [])
        .unwrap();
    c.execute(
        "INSERT INTO definitions VALUES ('s1','w6',1,'old window',7,'q','x','model')",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO statuses VALUES ('s1','w6',1,'copied status',7,'q','x','model')",
        [],
    )
    .unwrap();
    drop(c);
    ok(&db, &["edit", "--status", "user status", "W6"]);
    let st = statuses(&db, "w6");
    assert_eq!(st[1], (1, "user status".to_string(), "user".to_string(), 7));
    // A model op on the first child turn (index 8) is newer than both and wins.
    turn(&db, "t-8");
    assert_eq!(model_status(&db, "t-8", "model status")["applied"], 1);
    // Copied row, user row and the model row all remain.
    assert_eq!(statuses(&db, "w6").len(), 3);
}
