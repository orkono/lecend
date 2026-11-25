use assert_cmd::Command;
use rusqlite::Connection;
use serde_json::Value;
use tempfile::tempdir;

fn run(db: &std::path::Path, session: &str, args: &[&str]) -> Value {
    let out = Command::cargo_bin("lecend")
        .unwrap()
        .args(args)
        .args(["--db", db.to_str().unwrap(), "--session", session])
        .assert()
        .success()
        .get_output()
        .clone();
    serde_json::from_slice(&out.stdout).expect("stdout is JSON")
}

#[test]
fn state_is_read_only() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("l.db");
    run(&db, "s1", &["state"]);
    let v = run(&db, "s1", &["state"]);
    assert_eq!(v["modelCalls"], 0);
    assert_eq!(v["captureOff"], false);
    assert_eq!(v["forkedFrom"], Value::Null);
    assert_eq!(v["forkTurn"], Value::Null);
    assert_eq!(v["forkDone"], false);
}

#[test]
fn state_counts_turns_and_labels() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("l.db");
    run(&db, "s1", &["state"]);
    let conn = Connection::open(&db).unwrap();
    conn.execute_batch(
        "INSERT INTO terms (session_id,label,label_key) VALUES ('s1','A','a'),('s1','B','b'),('s2','C','c');
         INSERT INTO turns (session_id,turn_id,turn_index) VALUES ('s1','t1',1),('s1','t2',2),('s2','t1',1);",
    )
    .unwrap();
    let v = run(&db, "s1", &["state"]);
    assert_eq!(v["labels"], 2);
    assert_eq!(v["turns"], 2);
}

#[test]
fn usage_bump_increments() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("l.db");
    assert_eq!(run(&db, "s1", &["usage", "--bump"])["modelCalls"], 1);
    assert_eq!(run(&db, "s1", &["usage", "--bump"])["modelCalls"], 2);
    assert_eq!(run(&db, "s1", &["state"])["modelCalls"], 2);
    assert_eq!(run(&db, "s2", &["usage", "--bump"])["modelCalls"], 1);
}

#[test]
fn notice_true_only_once_across_sessions() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("l.db");
    assert_eq!(
        run(&db, "s1", &["notice", "--check-and-set"])["shown"],
        false
    );
    assert_eq!(
        run(&db, "s2", &["notice", "--check-and-set"])["shown"],
        true
    );
}

#[test]
fn capture_off_persists() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("l.db");
    assert_eq!(run(&db, "s1", &["capture", "--off"])["captureOff"], true);
    assert_eq!(run(&db, "s1", &["state"])["captureOff"], true);
    assert_eq!(run(&db, "s2", &["state"])["captureOff"], false);
    assert_eq!(run(&db, "s1", &["capture", "--on"])["captureOff"], false);
    assert_eq!(run(&db, "s1", &["state"])["captureOff"], false);
}
