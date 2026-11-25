//! Edge cases of `apply`: empty, malformed and duplicate ops, label words and
//! whitespace in labels.

use assert_cmd::Command;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tempfile::{tempdir, TempDir};

fn fresh() -> (TempDir, PathBuf) {
    let dir = tempdir().unwrap();
    let db = dir.path().join("l.db");
    (dir, db)
}

fn cmd(db: &Path, sub: &str, stdin: String) -> std::process::Output {
    Command::cargo_bin("lecend")
        .unwrap()
        .args([sub, "--db", db.to_str().unwrap(), "--session", "s1"])
        .write_stdin(stdin)
        .output()
        .unwrap()
}

fn turn(db: &Path, id: &str) {
    let out = cmd(db, "turn", json!({ "turnId": id }).to_string());
    assert!(out.status.success(), "turn failed: {out:?}");
}

fn apply(db: &Path, turn_id: &str, answer: &str, diff: Value) -> Value {
    let input = json!({ "turnId": turn_id, "prompt": "", "answer": answer, "diff": diff });
    let out = cmd(db, "apply", input.to_string());
    assert!(out.status.success(), "apply failed: {out:?}");
    serde_json::from_slice(&out.stdout).expect("stdout is JSON")
}

fn reasons(v: &Value) -> Vec<(String, String)> {
    v["rejected"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["op"].as_str().unwrap().to_string(),
                r["reason"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn r(op: &str, reason: &str) -> (String, String) {
    (op.to_string(), reason.to_string())
}

fn query<T: rusqlite::types::FromSql>(db: &Path, sql: &str) -> Vec<T> {
    let c = Connection::open(db).unwrap();
    let mut stmt = c.prepare(sql).unwrap();
    stmt.query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

const W6_ANSWER: &str = "W6 is the maintenance window on Saturday night.";

fn define_w6(db: &Path) {
    turn(db, "t-1");
    let v = apply(
        db,
        "t-1",
        W6_ANSWER,
        json!({"new":[{"label":"W6","definition":"Saturday night maintenance window","quote":W6_ANSWER}]}),
    );
    assert_eq!(v, json!({"applied":1,"rejected":[]}));
}

#[test]
fn decision_n_quote_with_only_label_words_rejected() {
    let (_d, db) = fresh();
    turn(&db, "t-1");
    // "decision" is a definition word, but only because it is part of the label.
    let answer = "We keep Decision 1 as it is.";
    let v = apply(
        &db,
        "t-1",
        answer,
        json!({"new":[{"label":"Decision 1","definition":"Decision about storage","quote":answer}]}),
    );
    assert_eq!(reasons(&v), [r("new", "quote-does-not-state-meaning")]);
}

#[test]
fn status_quote_not_verbatim_rejected() {
    let (_d, db) = fresh();
    define_w6(&db);
    turn(&db, "t-2");
    let v = apply(
        &db,
        "t-2",
        "W6 is approved.",
        json!({"status":[{"label":"W6","status":"approved","quote":"W6 got approved."}]}),
    );
    assert_eq!(reasons(&v), [r("status", "quote-not-in-turn")]);
}

#[test]
fn redefine_older_turn_not_newer() {
    let (_d, db) = fresh();
    define_w6(&db);
    turn(&db, "t-2");
    turn(&db, "t-3");
    let v = apply(
        &db,
        "t-3",
        "W6 is approved.",
        json!({"status":[{"label":"W6","status":"approved","quote":"W6 is approved."}]}),
    );
    assert_eq!(v["applied"], 1);
    let answer = "W6 now means the Sunday morning window.";
    let v = apply(
        &db,
        "t-2",
        answer,
        json!({"redefine":[{"label":"W6","definition":"Sunday morning window","quote":answer}]}),
    );
    assert_eq!(reasons(&v), [r("redefine", "not-newer")]);
    assert_eq!(query::<i64>(&db, "SELECT count(*) FROM definitions"), [1]);
}

#[test]
fn new_quote_does_not_state_meaning_rejected() {
    let (_d, db) = fresh();
    turn(&db, "t-1");
    let answer = "We will use W6 then.";
    let v = apply(
        &db,
        "t-1",
        answer,
        json!({"new":[{"label":"W6","definition":"Saturday night maintenance window","quote":answer}]}),
    );
    assert_eq!(reasons(&v), [r("new", "quote-does-not-state-meaning")]);
    assert_eq!(query::<i64>(&db, "SELECT count(*) FROM terms"), [0]);
}

#[test]
fn empty_status_rejected() {
    let (_d, db) = fresh();
    define_w6(&db);
    turn(&db, "t-2");
    let answer = "W6 is approved. D4 is the acceptance test.";
    let v = apply(
        &db,
        "t-2",
        answer,
        json!({
            "new":[{"label":"D4","definition":" \n ","quote":"D4 is the acceptance test."}],
            "status":[{"label":"W6","status":"   ","quote":"W6 is approved."}]
        }),
    );
    assert_eq!(v["applied"], 0);
    assert_eq!(reasons(&v), [r("new", "empty"), r("status", "empty")]);
    assert_eq!(query::<i64>(&db, "SELECT count(*) FROM statuses"), [0]);
}

#[test]
fn missing_quote_rejects_only_that_op() {
    let (_d, db) = fresh();
    turn(&db, "t-1");
    let answer = "W6 is the maintenance window. D4 is the acceptance test.";
    let v = apply(
        &db,
        "t-1",
        answer,
        json!({"new":[
            {"label":"W6","definition":"maintenance window"},
            {"label":"D4","definition":"acceptance test","quote":"D4 is the acceptance test."},
            {"label":"OLD","quote":"W6 is the maintenance window."}
        ]}),
    );
    assert_eq!(v["applied"], 1);
    assert_eq!(
        reasons(&v),
        [r("new", "quote-not-in-turn"), r("new", "empty")]
    );
    assert_eq!(query::<String>(&db, "SELECT label FROM terms"), ["D4"]);
}

#[test]
fn non_object_diff_fails_call() {
    let (_d, db) = fresh();
    turn(&db, "t-1");
    for diff in [json!("oops"), json!({"new": {"label": "W6"}})] {
        let input = json!({ "turnId": "t-1", "answer": W6_ANSWER, "diff": diff });
        let out = cmd(&db, "apply", input.to_string());
        assert!(!out.status.success());
        let v: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert!(v["error"].is_string());
    }
    assert_eq!(
        query::<i64>(
            &db,
            "SELECT count(*) FROM turns WHERE applied_at IS NOT NULL"
        ),
        [0]
    );
}

#[test]
fn label_internal_whitespace_collapsed() {
    let (_d, db) = fresh();
    turn(&db, "t-1");
    let answer = "Decision 1 is the storage choice.";
    let v = apply(
        &db,
        "t-1",
        answer,
        json!({"new":[{"label":" Decision \t 1 ","definition":"storage choice","quote":answer}]}),
    );
    assert_eq!(v, json!({"applied":1,"rejected":[]}));
    assert_eq!(
        query::<String>(&db, "SELECT label FROM terms"),
        ["Decision 1"]
    );
    assert_eq!(
        query::<String>(&db, "SELECT label_key FROM definitions"),
        ["decision 1"]
    );
}

#[test]
fn duplicate_ops_keep_last() {
    let (_d, db) = fresh();
    turn(&db, "t-1");
    let answer = "W6 is the maintenance window. W6 is approved. W6 is postponed.";
    let v = apply(
        &db,
        "t-1",
        answer,
        json!({
            "new":[
                {"label":"W6","definition":"bogus","quote":"never said"},
                {"label":"w6","definition":"maintenance window","quote":"W6 is the maintenance window."}
            ],
            "status":[
                {"label":"W6","status":"approved","quote":"W6 is approved."},
                {"label":"W6","status":"postponed","quote":"W6 is postponed."}
            ]
        }),
    );
    assert_eq!(v, json!({"applied":2,"rejected":[]}));
    assert_eq!(
        query::<String>(&db, "SELECT definition FROM definitions"),
        ["maintenance window"]
    );
    assert_eq!(
        query::<String>(&db, "SELECT status FROM statuses"),
        ["postponed"]
    );
    assert_eq!(query::<String>(&db, "SELECT label FROM terms"), ["w6"]);
}
