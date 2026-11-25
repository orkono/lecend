use assert_cmd::Command;
use rusqlite::Connection;
use serde_json::Value;
use tempfile::tempdir;

fn lecend() -> Command {
    Command::cargo_bin("lecend").unwrap()
}

fn stdout_json(out: &std::process::Output) -> Value {
    serde_json::from_slice(&out.stdout).expect("stdout is JSON")
}

#[test]
fn version_prints_crate_version() {
    let out = lecend()
        .arg("version")
        .assert()
        .success()
        .get_output()
        .clone();
    assert_eq!(stdout_json(&out)["version"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn doctor_creates_db_and_reports_schema() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("sub").join("l.db");
    let out = lecend()
        .args(["doctor", "--db", db.to_str().unwrap(), "--session", "s1"])
        .assert()
        .success()
        .get_output()
        .clone();
    let v = stdout_json(&out);
    assert_eq!(v["schemaVersion"], 1);
    assert_eq!(v["session"], "s1");
    assert_eq!(v["db"], db.to_str().unwrap());
    assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
    assert!(db.exists());
}

#[test]
fn doctor_is_idempotent() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("l.db");
    for _ in 0..2 {
        lecend()
            .args(["doctor", "--db", db.to_str().unwrap(), "--session", "s1"])
            .assert()
            .success();
    }
    let conn = Connection::open(&db).unwrap();
    let n: i64 = conn
        .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);
}

#[test]
fn errors_are_json() {
    let out = lecend().arg("doctor").assert().code(1).get_output().clone();
    assert!(stdout_json(&out)["error"].is_string());
}

#[test]
fn schema_has_all_v1_columns() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("l.db");
    lecend()
        .args(["doctor", "--db", db.to_str().unwrap(), "--session", "s1"])
        .assert()
        .success();
    let conn = Connection::open(&db).unwrap();
    let expected: &[(&str, &[&str])] = &[
        (
            "terms",
            &["session_id", "label", "label_key", "pinned", "created_turn"],
        ),
        (
            "definitions",
            &[
                "session_id",
                "label_key",
                "version",
                "definition",
                "turn_index",
                "quote",
                "created_at",
                "source",
            ],
        ),
        (
            "statuses",
            &[
                "session_id",
                "label_key",
                "definition_version",
                "status",
                "turn_index",
                "quote",
                "created_at",
                "source",
            ],
        ),
        ("ignores", &["session_id", "label_key"]),
        (
            "turns",
            &[
                "session_id",
                "turn_id",
                "turn_index",
                "recorded_at",
                "applied_at",
            ],
        ),
        (
            "sessions",
            &[
                "session_id",
                "forked_from",
                "fork_turn",
                "turn_base",
                "fork_done",
                "capture_off",
                "model_calls",
            ],
        ),
        (
            "rejections",
            &[
                "session_id",
                "turn_id",
                "op",
                "label",
                "reason",
                "payload",
                "created_at",
            ],
        ),
        ("meta", &["key", "value"]),
    ];
    for (table, cols) in expected {
        let mut stmt = conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap();
        let got: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .map(|c| c.unwrap())
            .collect();
        assert_eq!(&got, cols, "columns of {table}");
    }
    let v: String = conn
        .query_row(
            "SELECT value FROM meta WHERE key='schema_version'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(v, "1");
    let c: String = conn
        .query_row(
            "SELECT value FROM meta WHERE key='cost_notice_shown'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(c, "0");
}
