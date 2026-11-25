use assert_cmd::Command;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::time::Instant;
use tempfile::{tempdir, TempDir};

fn scan(db: &std::path::Path, input: Value) -> Value {
    let out = Command::cargo_bin("lecend")
        .unwrap()
        .args(["scan", "--db", db.to_str().unwrap(), "--session", "s1"])
        .write_stdin(input.to_string())
        .assert()
        .success()
        .get_output()
        .clone();
    serde_json::from_slice(&out.stdout).expect("stdout is JSON")
}

fn fresh() -> (TempDir, std::path::PathBuf) {
    let dir = tempdir().unwrap();
    let db = dir.path().join("l.db");
    // Create the schema and the session row.
    Command::cargo_bin("lecend")
        .unwrap()
        .args(["doctor", "--db", db.to_str().unwrap(), "--session", "s1"])
        .assert()
        .success();
    (dir, db)
}

fn seed(db: &std::path::Path, label: &str, def: &str, turn: i64) {
    let c = Connection::open(db).unwrap();
    let key = label.to_lowercase();
    c.execute(
        "INSERT INTO terms (session_id,label,label_key,pinned,created_turn) VALUES ('s1',?1,?2,0,?3)",
        rusqlite::params![label, key, turn],
    )
    .unwrap();
    c.execute(
        "INSERT INTO definitions (session_id,label_key,version,definition,turn_index) VALUES ('s1',?1,1,?2,?3)",
        rusqlite::params![key, def, turn],
    )
    .unwrap();
}

fn seed_status(db: &std::path::Path, label: &str, status: &str, turn: i64) {
    Connection::open(db)
        .unwrap()
        .execute(
            "INSERT INTO statuses (session_id,label_key,definition_version,status,turn_index) VALUES ('s1',?1,1,?2,?3)",
            rusqlite::params![label.to_lowercase(), status, turn],
        )
        .unwrap();
}

#[test]
fn empty_session_prose_should_not_call() {
    let (_d, db) = fresh();
    let v = scan(&db, json!({"prompt":"hello there","answer":"all good."}));
    assert_eq!(v["shouldCall"], false);
    assert_eq!(v["knownLabels"], json!([]));
    assert_eq!(v["candidates"], json!([]));
    assert_eq!(v["entries"], json!([]));
}

#[test]
fn candidate_triggers_call() {
    let (_d, db) = fresh();
    let v = scan(
        &db,
        json!({"prompt":"","answer":"Run W6 tonight","stoplist":[]}),
    );
    assert_eq!(v["shouldCall"], true);
    assert_eq!(v["candidates"], json!(["W6"]));
}

#[test]
fn known_label_mentioned_returns_its_entry() {
    let (_d, db) = fresh();
    seed(&db, "W6", "maintenance window", 1);
    seed_status(&db, "W6", "approved", 2);
    seed(&db, "OLD", "old server", 1);
    let v = scan(&db, json!({"prompt":"what about w6?","answer":""}));
    assert_eq!(v["shouldCall"], true);
    assert_eq!(v["knownLabels"], json!(["W6", "OLD"]));
    assert_eq!(v["candidates"], json!([]));
    assert_eq!(v["entries"][0]["label"], "W6");
    assert_eq!(v["entries"][0]["definition"], "maintenance window");
    assert_eq!(v["entries"][0]["status"], "approved");
    assert_eq!(v["entries"][1]["label"], "OLD");
    assert_eq!(v["entries"][1]["status"], Value::Null);
}

#[test]
fn forgotten_label_is_excluded() {
    let (_d, db) = fresh();
    seed(&db, "W6", "maintenance window", 1);
    Connection::open(&db)
        .unwrap()
        .execute(
            "INSERT INTO ignores (session_id,label_key) VALUES ('s1','w6')",
            [],
        )
        .unwrap();
    let v = scan(&db, json!({"prompt":"","answer":"W6 again"}));
    assert_eq!(v["shouldCall"], false);
    assert_eq!(v["knownLabels"], json!([]));
    assert_eq!(v["candidates"], json!([]));
    assert_eq!(v["entries"], json!([]));
}

#[test]
fn entries_capped_at_12() {
    let (_d, db) = fresh();
    for i in 1..=60 {
        seed(&db, &format!("X{i}"), "thing", i);
    }
    let v = scan(&db, json!({"prompt":"","answer":"plain words"}));
    assert_eq!(v["entries"].as_array().unwrap().len(), 12);
    assert_eq!(v["entries"][0]["label"], "X60");
    assert_eq!(v["knownLabels"].as_array().unwrap().len(), 50);
    assert_eq!(v["knownLabels"][0], "X60");
}

#[test]
fn huge_answer_100kb_is_fine() {
    let (_d, db) = fresh();
    seed(&db, "W6", "maintenance window", 1);
    let answer = "lorem ipsum dolor sit amet ".repeat(4000);
    let t = Instant::now();
    let v = scan(&db, json!({"prompt":"","answer":answer}));
    assert!(t.elapsed().as_secs_f64() < 1.0);
    assert_eq!(v["shouldCall"], false);
}
