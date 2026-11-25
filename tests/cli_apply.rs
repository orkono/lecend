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

fn cmd(db: &Path, sid: &str, sub: &str, stdin: String) -> std::process::Output {
    Command::cargo_bin("lecend")
        .unwrap()
        .args([sub, "--db", db.to_str().unwrap(), "--session", sid])
        .write_stdin(stdin)
        .output()
        .unwrap()
}

fn turn_in(db: &Path, sid: &str, id: &str) -> i64 {
    let out = cmd(db, sid, "turn", json!({ "turnId": id }).to_string());
    assert!(out.status.success(), "turn failed: {out:?}");
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    v["turnIndex"].as_i64().unwrap()
}

fn turn(db: &Path, id: &str) -> i64 {
    turn_in(db, "s1", id)
}

fn apply_in(db: &Path, sid: &str, input: Value) -> Value {
    let out = cmd(db, sid, "apply", input.to_string());
    assert!(out.status.success(), "apply failed: {out:?}");
    serde_json::from_slice(&out.stdout).expect("stdout is JSON")
}

fn apply(db: &Path, input: Value) -> Value {
    apply_in(db, "s1", input)
}

fn input(turn_id: &str, prompt: &str, answer: &str, diff: Value) -> Value {
    json!({ "turnId": turn_id, "prompt": prompt, "answer": answer, "diff": diff })
}

fn conn(db: &Path) -> Connection {
    Connection::open(db).unwrap()
}

fn count(db: &Path, sql: &str) -> i64 {
    conn(db).query_row(sql, [], |r| r.get(0)).unwrap()
}

fn rejected(v: &Value) -> Vec<(String, String, String)> {
    v["rejected"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["op"].as_str().unwrap().to_string(),
                r["label"].as_str().unwrap().to_string(),
                r["reason"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn rej(op: &str, label: &str, reason: &str) -> (String, String, String) {
    (op.to_string(), label.to_string(), reason.to_string())
}

const W6_ANSWER: &str = "W6 is the maintenance window on Saturday night.";

/// Records turn `t-1` and applies `new W6` from `W6_ANSWER`.
fn define_w6(db: &Path) {
    turn(db, "t-1");
    let v = apply(
        db,
        input(
            "t-1",
            "",
            W6_ANSWER,
            json!({"new":[{"label":"W6","definition":"Saturday night maintenance window","quote":W6_ANSWER}]}),
        ),
    );
    assert_eq!(v, json!({"applied":1,"rejected":[]}));
}

const W6_SUNDAY: &str = "W6 now means the Sunday morning window.";

fn redefine_w6(turn_id: &str) -> Value {
    input(
        turn_id,
        "",
        W6_SUNDAY,
        json!({"redefine":[{"label":"W6","definition":"Sunday morning window","quote":W6_SUNDAY}]}),
    )
}

fn status_w6(turn_id: &str, answer: &str, status: &str) -> Value {
    input(
        turn_id,
        "",
        answer,
        json!({"status":[{"label":"W6","status":status,"quote":answer}]}),
    )
}

fn definitions(db: &Path, key: &str) -> Vec<(i64, String, String, i64)> {
    let c = conn(db);
    let mut stmt = c
        .prepare(
            "SELECT version, definition, source, turn_index FROM definitions
             WHERE session_id='s1' AND label_key=?1 ORDER BY version",
        )
        .unwrap();
    stmt.query_map([key], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn statuses(db: &Path, key: &str) -> Vec<(i64, String, i64)> {
    let c = conn(db);
    let mut stmt = c
        .prepare(
            "SELECT definition_version, status, turn_index FROM statuses
             WHERE session_id='s1' AND label_key=?1 ORDER BY rowid",
        )
        .unwrap();
    stmt.query_map([key], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn new_label_is_stored_with_version_1() {
    let (_d, db) = fresh();
    define_w6(&db);
    let (label, key, pinned, created): (String, String, i64, i64) = conn(&db)
        .query_row(
            "SELECT label, label_key, pinned, created_turn FROM terms WHERE session_id='s1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        (label.as_str(), key.as_str(), pinned, created),
        ("W6", "w6", 0, 1)
    );
    assert_eq!(
        definitions(&db, "w6"),
        [(
            1,
            "Saturday night maintenance window".to_string(),
            "model".to_string(),
            1
        )]
    );
    let quote: String = conn(&db)
        .query_row("SELECT quote FROM definitions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(quote, W6_ANSWER);
}

#[test]
fn status_attaches_to_current_version() {
    let (_d, db) = fresh();
    define_w6(&db);
    turn(&db, "t-2");
    assert_eq!(apply(&db, redefine_w6("t-2"))["applied"], 1);
    turn(&db, "t-3");
    let v = apply(&db, status_w6("t-3", "W6 is approved.", "approved"));
    assert_eq!(v, json!({"applied":1,"rejected":[]}));
    assert_eq!(statuses(&db, "w6"), [(2, "approved".to_string(), 3)]);
}

#[test]
fn quote_must_be_verbatim() {
    let (_d, db) = fresh();
    turn(&db, "t-1");
    let v = apply(
        &db,
        input(
            "t-1",
            "",
            W6_ANSWER,
            json!({"new":[{"label":"W6","definition":"Saturday night maintenance window",
                           "quote":"W6 is the Saturday night maintenance window."}]}),
        ),
    );
    assert_eq!(v["applied"], 0);
    assert_eq!(rejected(&v), [rej("new", "W6", "quote-not-in-turn")]);
    assert_eq!(count(&db, "SELECT count(*) FROM terms"), 0);
}

#[test]
fn whitespace_differences_in_quote_are_tolerated() {
    let (_d, db) = fresh();
    turn(&db, "t-1");
    let v = apply(
        &db,
        input(
            "t-1",
            "D4 is   the acceptance test.",
            "W6 is the\n  maintenance   window on Saturday.",
            json!({"new":[
                {"label":"W6","definition":"maintenance window","quote":"W6 is the maintenance window on Saturday."},
                {"label":"D4","definition":"acceptance test","quote":"D4 is the\tacceptance test."}
            ]}),
        ),
    );
    assert_eq!(v, json!({"applied":2,"rejected":[]}));
}

#[test]
fn new_on_existing_label_rejected_already_defined() {
    let (_d, db) = fresh();
    define_w6(&db);
    turn(&db, "t-2");
    let v = apply(
        &db,
        input(
            "t-2",
            "",
            W6_SUNDAY,
            json!({"new":[{"label":"W6","definition":"Sunday morning window","quote":W6_SUNDAY}]}),
        ),
    );
    assert_eq!(rejected(&v), [rej("new", "W6", "already-defined")]);
    assert_eq!(definitions(&db, "w6").len(), 1);
}

#[test]
fn redefine_adds_version_2_and_previous_kept() {
    let (_d, db) = fresh();
    define_w6(&db);
    turn(&db, "t-2");
    let v = apply(&db, redefine_w6("t-2"));
    assert_eq!(v, json!({"applied":1,"rejected":[]}));
    let defs = definitions(&db, "w6");
    assert_eq!(defs.len(), 2);
    assert_eq!(defs[0].1, "Saturday night maintenance window");
    assert_eq!(
        defs[1],
        (
            2,
            "Sunday morning window".to_string(),
            "model".to_string(),
            2
        )
    );
}

#[test]
fn redefine_same_text_is_noop() {
    let (_d, db) = fresh();
    define_w6(&db);
    turn(&db, "t-2");
    let v = apply(
        &db,
        input(
            "t-2",
            "",
            W6_ANSWER,
            json!({"redefine":[{"label":"W6","definition":"Saturday night maintenance window","quote":W6_ANSWER}]}),
        ),
    );
    assert_eq!(v, json!({"applied":0,"rejected":[]}));
    assert_eq!(definitions(&db, "w6").len(), 1);
    assert_eq!(count(&db, "SELECT count(*) FROM rejections"), 0);
}

#[test]
fn redefine_and_status_same_turn_both_apply() {
    let (_d, db) = fresh();
    define_w6(&db);
    turn(&db, "t-2");
    let answer = "W6 now means the Sunday morning window. W6 is approved.";
    let v = apply(
        &db,
        input(
            "t-2",
            "",
            answer,
            json!({
                "redefine":[{"label":"W6","definition":"Sunday morning window","quote":W6_SUNDAY}],
                "status":[{"label":"W6","status":"approved","quote":"W6 is approved."}]
            }),
        ),
    );
    assert_eq!(v, json!({"applied":2,"rejected":[]}));
    assert_eq!(definitions(&db, "w6").len(), 2);
    assert_eq!(statuses(&db, "w6"), [(2, "approved".to_string(), 2)]);
}

#[test]
fn pinned_blocks_redefine_and_status() {
    let (_d, db) = fresh();
    define_w6(&db);
    conn(&db)
        .execute("UPDATE terms SET pinned = 1 WHERE label_key = 'w6'", [])
        .unwrap();
    turn(&db, "t-2");
    let answer = "W6 now means the Sunday morning window. W6 is approved.";
    let v = apply(
        &db,
        input(
            "t-2",
            "",
            answer,
            json!({
                "redefine":[{"label":"W6","definition":"Sunday morning window","quote":W6_SUNDAY}],
                "status":[{"label":"W6","status":"approved","quote":"W6 is approved."}]
            }),
        ),
    );
    assert_eq!(v["applied"], 0);
    assert_eq!(
        rejected(&v),
        [
            rej("redefine", "W6", "pinned"),
            rej("status", "W6", "pinned")
        ]
    );
    assert_eq!(definitions(&db, "w6").len(), 1);
    assert!(statuses(&db, "w6").is_empty());
}

#[test]
fn user_edited_definition_blocks_redefine() {
    let (_d, db) = fresh();
    define_w6(&db);
    conn(&db)
        .execute(
            "INSERT INTO definitions (session_id,label_key,version,definition,turn_index,source)
             VALUES ('s1','w6',2,'Saturday window, user wording',1,'user')",
            [],
        )
        .unwrap();
    turn(&db, "t-2");
    let v = apply(&db, redefine_w6("t-2"));
    assert_eq!(rejected(&v), [rej("redefine", "W6", "user-edited")]);
    assert_eq!(definitions(&db, "w6").len(), 2);
}

#[test]
fn older_turn_rejected_not_newer() {
    let (_d, db) = fresh();
    define_w6(&db);
    turn(&db, "t-2");
    turn(&db, "t-3");
    let v = apply(&db, status_w6("t-3", "W6 is approved.", "approved"));
    assert_eq!(v["applied"], 1);
    let v = apply(&db, status_w6("t-2", "W6 is postponed.", "postponed"));
    assert_eq!(rejected(&v), [rej("status", "W6", "not-newer")]);
    assert_eq!(statuses(&db, "w6"), [(1, "approved".to_string(), 3)]);
}

#[test]
fn user_edit_at_later_turn_wins_over_delayed_job() {
    let (_d, db) = fresh();
    define_w6(&db);
    turn(&db, "t-2");
    conn(&db)
        .execute(
            "INSERT INTO statuses (session_id,label_key,definition_version,status,turn_index,source)
             VALUES ('s1','w6',1,'cancelled',2,'user')",
            [],
        )
        .unwrap();
    let v = apply(&db, status_w6("t-2", "W6 is approved.", "approved"));
    assert_eq!(rejected(&v), [rej("status", "W6", "not-newer")]);
}

#[test]
fn same_turn_applied_twice_is_noop() {
    let (_d, db) = fresh();
    define_w6(&db);
    let v = apply(
        &db,
        input(
            "t-1",
            "",
            W6_ANSWER,
            json!({"new":[{"label":"W6","definition":"Saturday night maintenance window","quote":W6_ANSWER}]}),
        ),
    );
    assert_eq!(v, json!({"applied":0,"rejected":[]}));
    assert_eq!(count(&db, "SELECT count(*) FROM definitions"), 1);
    assert_eq!(count(&db, "SELECT count(*) FROM rejections"), 0);
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM turns WHERE applied_at IS NOT NULL"
        ),
        1
    );
}

#[test]
fn ignored_label_rejected() {
    let (_d, db) = fresh();
    turn(&db, "t-1");
    conn(&db)
        .execute(
            "INSERT INTO ignores (session_id,label_key) VALUES ('s1','w6')",
            [],
        )
        .unwrap();
    let v = apply(
        &db,
        input(
            "t-1",
            "",
            W6_ANSWER,
            json!({"new":[{"label":"W6","definition":"Saturday night maintenance window","quote":W6_ANSWER}]}),
        ),
    );
    assert_eq!(rejected(&v), [rej("new", "W6", "ignored")]);
    assert_eq!(count(&db, "SELECT count(*) FROM terms"), 0);
}

#[test]
fn too_long_definition_rejected() {
    let (_d, db) = fresh();
    turn(&db, "t-1");
    // 200 chars (but > 200 bytes) is accepted; 201 chars is rejected.
    let ok = format!("window {}", "ç".repeat(193));
    let long = format!("window {}", "ç".repeat(194));
    let answer = "W6 is the window. D4 is the window too.";
    let v = apply(
        &db,
        input(
            "t-1",
            "",
            answer,
            json!({"new":[
                {"label":"W6","definition":ok,"quote":"W6 is the window."},
                {"label":"D4","definition":long,"quote":"D4 is the window too."}
            ]}),
        ),
    );
    assert_eq!(v["applied"], 1);
    assert_eq!(rejected(&v), [rej("new", "D4", "too-long")]);
}

#[test]
fn too_long_label_status_and_quote_rejected() {
    let (_d, db) = fresh();
    define_w6(&db);
    turn(&db, "t-2");
    let label = "L".repeat(33);
    let long_quote = format!("W6 is {}", "x".repeat(300));
    let answer = format!("{label} is a window. W6 is approved. {long_quote}");
    let v = apply(
        &db,
        input(
            "t-2",
            "",
            &answer,
            json!({
                "new":[{"label":label,"definition":"a window","quote":format!("{label} is a window.")}],
                "redefine":[{"label":"W6","definition":"a window","quote":long_quote}],
                "status":[{"label":"W6","status":"a".repeat(201),"quote":"W6 is approved."}]
            }),
        ),
    );
    assert_eq!(v["applied"], 0);
    assert_eq!(
        rejected(&v),
        [
            rej("new", &label, "too-long"),
            rej("redefine", "W6", "too-long"),
            rej("status", "W6", "too-long")
        ]
    );
}

#[test]
fn rejections_are_recorded() {
    let (_d, db) = fresh();
    turn(&db, "t-1");
    let v = apply(
        &db,
        input(
            "t-1",
            "",
            W6_ANSWER,
            json!({"new":[{"label":"W6","definition":"Saturday window","quote":"W6 was never said"}]}),
        ),
    );
    assert_eq!(rejected(&v).len(), 1);
    let (sid, tid, op, label, reason, payload, created): (
        String,
        String,
        String,
        String,
        String,
        String,
        Option<String>,
    ) = conn(&db)
        .query_row(
            "SELECT session_id, turn_id, op, label, reason, payload, created_at FROM rejections",
            [],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        (
            sid.as_str(),
            tid.as_str(),
            op.as_str(),
            label.as_str(),
            reason.as_str()
        ),
        ("s1", "t-1", "new", "W6", "quote-not-in-turn")
    );
    let payload: Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(payload["quote"], "W6 was never said");
    assert!(created.is_some());
}

#[test]
fn stoplisted_label_rejected() {
    let (_d, db) = fresh();
    turn(&db, "t-1");
    let answer = "W6 is the maintenance window. API is the public interface.";
    let mut inp = input(
        "t-1",
        "",
        answer,
        json!({"new":[
            {"label":"W6","definition":"maintenance window","quote":"W6 is the maintenance window."},
            {"label":"API","definition":"public interface","quote":"API is the public interface."}
        ]}),
    );
    inp["stoplist"] = json!(["w6"]);
    let v = apply(&db, inp);
    assert_eq!(
        rejected(&v),
        [
            rej("new", "W6", "stoplisted"),
            rej("new", "API", "stoplisted")
        ]
    );
}

#[test]
fn label_not_in_quote_rejected() {
    let (_d, db) = fresh();
    turn(&db, "t-1");
    let answer = "The maintenance window is on Saturday night.";
    let v = apply(
        &db,
        input(
            "t-1",
            "",
            answer,
            json!({"new":[{"label":"W6","definition":"Saturday night maintenance window","quote":answer}]}),
        ),
    );
    assert_eq!(rejected(&v), [rej("new", "W6", "label-not-in-quote")]);
}

#[test]
fn status_label_must_be_in_turn() {
    let (_d, db) = fresh();
    define_w6(&db);
    turn(&db, "t-2");
    let v = apply(&db, status_w6("t-2", "It is approved.", "approved"));
    assert_eq!(rejected(&v), [rej("status", "W6", "label-not-in-turn")]);
    // Label elsewhere in the turn is enough for a status.
    turn(&db, "t-3");
    let v = apply(
        &db,
        input(
            "t-3",
            "Is w6 fine?",
            "It is approved.",
            json!({"status":[{"label":"W6","status":"approved","quote":"It is approved."}]}),
        ),
    );
    assert_eq!(v, json!({"applied":1,"rejected":[]}));
}

#[test]
fn status_for_unknown_label_not_defined() {
    let (_d, db) = fresh();
    turn(&db, "t-1");
    let v = apply(&db, status_w6("t-1", "W6 is approved.", "approved"));
    assert_eq!(rejected(&v), [rej("status", "W6", "not-defined")]);
    turn(&db, "t-2");
    let v = apply(&db, redefine_w6("t-2"));
    assert_eq!(rejected(&v), [rej("redefine", "W6", "not-defined")]);
}

#[test]
fn redefine_quote_only_mentioning_label_rejected() {
    let (_d, db) = fresh();
    define_w6(&db);
    turn(&db, "t-2");
    let answer = "Let us look at W6 again.";
    let v = apply(
        &db,
        input(
            "t-2",
            "",
            answer,
            json!({"redefine":[{"label":"W6","definition":"Sunday morning window","quote":answer}]}),
        ),
    );
    assert_eq!(
        rejected(&v),
        [rej("redefine", "W6", "quote-does-not-state-meaning")]
    );
    assert_eq!(definitions(&db, "w6").len(), 1);
}

#[test]
fn case_variant_is_same_label() {
    let (_d, db) = fresh();
    define_w6(&db);
    turn(&db, "t-2");
    let v = apply(
        &db,
        input(
            "t-2",
            "",
            "w6 is the maintenance window. w6 is approved.",
            json!({
                "new":[{"label":"w6","definition":"maintenance window","quote":"w6 is the maintenance window."}],
                "status":[{"label":"w6","status":"approved","quote":"w6 is approved."}]
            }),
        ),
    );
    assert_eq!(v["applied"], 1);
    assert_eq!(rejected(&v), [rej("new", "w6", "already-defined")]);
    let label: String = conn(&db)
        .query_row("SELECT label FROM terms", [], |r| r.get(0))
        .unwrap();
    assert_eq!(label, "W6");
    assert_eq!(statuses(&db, "w6"), [(1, "approved".to_string(), 2)]);
}

#[test]
fn tail_quote_of_100kb_answer_validates() {
    let (_d, db) = fresh();
    turn(&db, "t-1");
    let answer = format!("{}{W6_ANSWER}", "lorem ipsum dolor sit amet ".repeat(4000));
    assert!(answer.len() > 100_000);
    let tail: String = answer[answer.len() - 500..].to_string();
    let quote = &tail[tail.find("W6").unwrap()..];
    let v = apply(
        &db,
        input(
            "t-1",
            "",
            &answer,
            json!({"new":[{"label":"W6","definition":"Saturday night maintenance window","quote":quote}]}),
        ),
    );
    assert_eq!(v, json!({"applied":1,"rejected":[]}));
}

#[test]
fn unknown_turn_id_is_error() {
    let (_d, db) = fresh();
    turn(&db, "t-1");
    let out = cmd(&db, "s1", "apply", json!({"turnId":"t-9"}).to_string());
    assert!(!out.status.success());
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["error"].as_str().unwrap().contains("t-9"));
}

#[test]
fn missing_arrays_mean_empty() {
    let (_d, db) = fresh();
    turn(&db, "t-1");
    let v = apply(&db, json!({"turnId":"t-1","prompt":"","answer":""}));
    assert_eq!(v, json!({"applied":0,"rejected":[]}));
    let v2 = apply(&db, json!({"turnId":"t-1","diff":{}}));
    assert_eq!(v2, json!({"applied":0,"rejected":[]}));
}

#[test]
fn parallel_turn_and_apply_two_sessions() {
    let (_d, db) = fresh();
    let handles: Vec<_> = (0..8)
        .map(|t| {
            let db = db.clone();
            std::thread::spawn(move || {
                let sid = format!("s{}", t % 2);
                for j in 0..5 {
                    let id = format!("t{t}-{j}");
                    let label = format!("K{t}{j}");
                    let answer = format!("{label} is the knob setting.");
                    turn_in(&db, &sid, &id);
                    let v = apply_in(
                        &db,
                        &sid,
                        input(
                            &id,
                            "",
                            &answer,
                            json!({"new":[{"label":label,"definition":"knob setting","quote":answer}]}),
                        ),
                    );
                    assert_eq!(v, json!({"applied":1,"rejected":[]}));
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    for sid in ["s0", "s1"] {
        let q = |table: &str| {
            count(
                &db,
                &format!("SELECT count(*) FROM {table} WHERE session_id='{sid}'"),
            )
        };
        assert_eq!(q("terms"), 20);
        assert_eq!(q("definitions"), 20);
        assert_eq!(q("turns"), 20);
        assert_eq!(
            count(
                &db,
                &format!(
                    "SELECT count(*) FROM turns WHERE session_id='{sid}' AND applied_at IS NOT NULL"
                )
            ),
            20
        );
    }
}

#[test]
fn fixture_w6_turns_apply_cleanly() {
    let (_d, db) = fresh();
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/w6.json");
    let turns: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    for (i, t) in turns.iter().enumerate() {
        let id = t["turnId"].as_str().unwrap();
        turn(&db, id);
        let v = apply(&db, t.clone());
        assert_eq!(v["rejected"], json!([]), "turn {id}");
        let expected = [4, 1, 1][i];
        assert_eq!(v["applied"], expected, "turn {id}");
        if i == 1 {
            assert_eq!(
                statuses(&db, "w6"),
                [(1, "Approved for Saturday".to_string(), 2)]
            );
        }
    }
    assert_eq!(
        statuses(&db, "new"),
        [(1, "Unreachable over the network".to_string(), 3)]
    );
    assert_eq!(count(&db, "SELECT count(*) FROM terms"), 4);
}
