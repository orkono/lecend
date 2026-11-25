use assert_cmd::Command;
use rusqlite::Connection;
use serde_json::Value;
use tempfile::{tempdir, TempDir};

fn run(db: &std::path::Path, session: &str, args: &[&str], stdin: &str) -> Value {
    let out = Command::cargo_bin("lecend")
        .unwrap()
        .args(args)
        .args(["--db", db.to_str().unwrap(), "--session", session])
        .write_stdin(stdin.to_string())
        .assert()
        .success()
        .get_output()
        .clone();
    serde_json::from_slice(&out.stdout).expect("stdout is JSON")
}

fn turn(db: &std::path::Path, session: &str, id: &str) -> i64 {
    let v = run(db, session, &["turn"], &format!(r#"{{"turnId":"{id}"}}"#));
    v["turnIndex"].as_i64().unwrap()
}

fn fresh() -> (TempDir, std::path::PathBuf) {
    let dir = tempdir().unwrap();
    let db = dir.path().join("l.db");
    (dir, db)
}

#[test]
fn first_turn_is_1() {
    let (_d, db) = fresh();
    assert_eq!(turn(&db, "s1", "t-1"), 1);
}

#[test]
fn same_turn_id_same_index() {
    let (_d, db) = fresh();
    assert_eq!(turn(&db, "s1", "t-1"), 1);
    assert_eq!(turn(&db, "s1", "t-1"), 1);
}

#[test]
fn next_turn_increments() {
    let (_d, db) = fresh();
    assert_eq!(turn(&db, "s1", "t-1"), 1);
    assert_eq!(turn(&db, "s1", "t-2"), 2);
}

#[test]
fn continues_from_turn_base() {
    let (_d, db) = fresh();
    run(&db, "s1", &["state"], "");
    Connection::open(&db)
        .unwrap()
        .execute(
            "UPDATE sessions SET turn_base = 40 WHERE session_id = 's1'",
            [],
        )
        .unwrap();
    assert_eq!(turn(&db, "s1", "t-1"), 41);
}

#[test]
fn sessions_are_independent() {
    let (_d, db) = fresh();
    assert_eq!(turn(&db, "s1", "t-1"), 1);
    assert_eq!(turn(&db, "s2", "t-1"), 1);
}

#[test]
fn turn_rejects_bad_stdin() {
    let (_d, db) = fresh();
    let out = Command::cargo_bin("lecend")
        .unwrap()
        .args(["turn", "--db", db.to_str().unwrap(), "--session", "s1"])
        .write_stdin("{}")
        .assert()
        .failure()
        .get_output()
        .clone();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["error"].as_str().unwrap().contains("turnId"));
}
