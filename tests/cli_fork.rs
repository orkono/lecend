use assert_cmd::Command;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::{tempdir, TempDir};

fn fresh() -> (TempDir, PathBuf) {
    let dir = tempdir().unwrap();
    let db = dir.path().join("l.db");
    (dir, db)
}

fn run(db: &Path, sid: &str, args: &[&str], stdin: &str) -> std::process::Output {
    let mut full = vec![args[0], "--db", db.to_str().unwrap(), "--session", sid];
    full.extend(&args[1..]);
    Command::cargo_bin("lecend")
        .unwrap()
        .args(full)
        .write_stdin(stdin.to_string())
        .output()
        .unwrap()
}

fn ok(db: &Path, sid: &str, args: &[&str]) -> Value {
    let out = run(db, sid, args, "");
    assert!(out.status.success(), "{args:?} failed: {out:?}");
    serde_json::from_slice(&out.stdout).expect("stdout is JSON")
}

fn err(db: &Path, sid: &str, args: &[&str]) -> String {
    let out = run(db, sid, args, "");
    assert_eq!(out.status.code(), Some(1), "{args:?} should fail: {out:?}");
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    v["error"].as_str().unwrap().to_string()
}

fn turn(db: &Path, sid: &str, id: &str) -> i64 {
    let out = run(db, sid, &["turn"], &json!({ "turnId": id }).to_string());
    assert!(out.status.success(), "turn failed: {out:?}");
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    v["turnIndex"].as_i64().unwrap()
}

fn apply(db: &Path, sid: &str, turn_id: &str, answer: &str, diff: Value) -> Value {
    let input = json!({ "turnId": turn_id, "prompt": "", "answer": answer, "diff": diff });
    let out = run(db, sid, &["apply"], &input.to_string());
    assert!(out.status.success(), "apply failed: {out:?}");
    serde_json::from_slice(&out.stdout).unwrap()
}

fn count(db: &Path, sql: &str) -> i64 {
    Connection::open(db)
        .unwrap()
        .query_row(sql, [], |r| r.get(0))
        .unwrap()
}

/// Parent "p": W6 defined on turn 1, status on turn 2, "Late" defined on turn 3;
/// W6 pinned, "Noise" ignored.
fn seed_parent(db: &Path) {
    turn(db, "p", "t-1");
    let a1 = "W6 is the Saturday night window.";
    let d1 = json!({"new":[{"label":"W6","definition":"Saturday window","quote":a1}]});
    assert_eq!(apply(db, "p", "t-1", a1, d1)["applied"], 1);
    turn(db, "p", "t-2");
    let a2 = "W6 is blocked.";
    let d2 = json!({"status":[{"label":"W6","status":"blocked","quote":a2}]});
    assert_eq!(apply(db, "p", "t-2", a2, d2)["applied"], 1);
    turn(db, "p", "t-3");
    let a3 = "Late is a later term.";
    let d3 = json!({"new":[{"label":"Late","definition":"later term","quote":a3}]});
    assert_eq!(apply(db, "p", "t-3", a3, d3)["applied"], 1);
    ok(db, "p", &["edit", "--pin", "W6"]);
    ok(db, "p", &["edit", "--definition", "temp", "Noise"]);
    ok(db, "p", &["forget", "Noise"]);
}

fn child_count(db: &Path, table: &str) -> i64 {
    count(
        db,
        &format!("SELECT count(*) FROM {table} WHERE session_id='c'"),
    )
}

#[test]
fn fork_copies_everything() {
    let (_d, db) = fresh();
    seed_parent(&db);
    let v = ok(&db, "c", &["fork", "--from", "p"]);
    assert_eq!(v["already"], false);
    assert_eq!(child_count(&db, "terms"), 2);
    assert_eq!(child_count(&db, "definitions"), 2);
    assert_eq!(child_count(&db, "statuses"), 1);
    assert_eq!(child_count(&db, "ignores"), 1);
    assert_eq!(v["copied"], 2 + 2 + 1 + 1);
    assert_eq!(
        count(
            &db,
            "SELECT pinned FROM terms WHERE session_id='c' AND label_key='w6'"
        ),
        1
    );
    assert_eq!(
        count(
            &db,
            "SELECT created_turn FROM terms WHERE session_id='c' AND label_key='late'"
        ),
        3
    );
    let s = ok(&db, "c", &["state"]);
    assert_eq!(s["forkedFrom"], "p");
    assert_eq!(s["forkDone"], true);
    assert_eq!(s["forkTurn"], 3);
}

#[test]
fn fork_is_exactly_once() {
    let (_d, db) = fresh();
    seed_parent(&db);
    ok(&db, "c", &["fork", "--from", "p"]);
    let again = ok(&db, "c", &["fork", "--from", "p"]);
    assert_eq!(again, json!({"copied": 0, "already": true}));
    assert_eq!(child_count(&db, "definitions"), 2);
}

#[test]
fn fork_at_turn_cuts_later_rows() {
    let (_d, db) = fresh();
    seed_parent(&db);
    let v = ok(&db, "c", &["fork", "--from", "p", "--at-turn", "2"]);
    assert_eq!(v["already"], false);
    assert_eq!(child_count(&db, "terms"), 1);
    assert_eq!(child_count(&db, "definitions"), 1);
    assert_eq!(child_count(&db, "statuses"), 1);
    assert_eq!(child_count(&db, "ignores"), 1);
    let s = ok(&db, "c", &["state"]);
    assert_eq!(s["forkTurn"], 2);
}

#[test]
fn fork_at_turn_drops_statuses_of_uncopied_versions() {
    let (_d, db) = fresh();
    seed_parent(&db);
    // Edit on parent turn 3 makes W6 v2; a status on v2 at turn 3 must not travel.
    ok(&db, "p", &["edit", "--definition", "Sunday window", "W6"]);
    ok(&db, "p", &["edit", "--status", "done", "W6"]);
    ok(&db, "c", &["fork", "--from", "p", "--at-turn", "2"]);
    assert_eq!(child_count(&db, "definitions"), 1);
    assert_eq!(child_count(&db, "statuses"), 1);
}

#[test]
fn child_turns_continue_after_parent_max() {
    let (_d, db) = fresh();
    seed_parent(&db);
    ok(&db, "c", &["fork", "--from", "p"]);
    assert_eq!(turn(&db, "c", "c-1"), 4);
    let (_d2, db2) = fresh();
    seed_parent(&db2);
    ok(&db2, "c", &["fork", "--from", "p", "--at-turn", "2"]);
    assert_eq!(turn(&db2, "c", "c-1"), 3);
}

#[test]
fn child_and_parent_diverge() {
    let (_d, db) = fresh();
    seed_parent(&db);
    ok(&db, "c", &["fork", "--from", "p"]);
    ok(&db, "c", &["edit", "--definition", "child only", "W6"]);
    ok(&db, "p", &["edit", "--status", "parent only", "Late"]);
    assert_eq!(child_count(&db, "definitions"), 3);
    assert_eq!(
        count(&db, "SELECT count(*) FROM definitions WHERE session_id='p'"),
        2
    );
    assert_eq!(child_count(&db, "statuses"), 1);
}

#[test]
fn fork_refuses_session_with_own_glossary() {
    let (_d, db) = fresh();
    seed_parent(&db);
    turn(&db, "c", "c-1");
    assert_eq!(
        err(&db, "c", &["fork", "--from", "p"]),
        "session already has a glossary"
    );
    assert_eq!(child_count(&db, "definitions"), 0);
}

#[test]
fn fork_rejects_self_and_bad_flags() {
    let (_d, db) = fresh();
    assert!(err(&db, "c", &["fork", "--from", "c"]).contains("itself"));
    assert!(!err(&db, "c", &["fork"]).is_empty());
    assert!(!err(&db, "c", &["fork", "--from", "p", "--none"]).is_empty());
}

#[test]
fn fork_none_marks_unknown_parent() {
    let (_d, db) = fresh();
    ok(&db, "c", &["fork", "--none"]);
    let s = ok(&db, "c", &["state"]);
    assert_eq!(s["forkDone"], true);
    assert_eq!(s["forkedFrom"], Value::Null);
    assert_eq!(s["forkTurn"], -1);
}

fn user_line(uuid: &str) -> String {
    json!({"type":"user","uuid":uuid,"sessionId":"x","message":{"content":"hi"}}).to_string()
}

fn lineage(path: &Path) -> Value {
    let out = Command::cargo_bin("lecend")
        .unwrap()
        .args(["lineage", "--transcript", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn lineage_finds_parent_by_shared_uuid() {
    let dir = tempdir().unwrap();
    let parent = [
        user_line("u1"),
        json!({"type":"assistant","uuid":"a1"}).to_string(),
        user_line("u2"),
        "not json".to_string(),
        user_line("u3"),
    ];
    fs::write(dir.path().join("parent-id.jsonl"), parent.join("\n")).unwrap();
    fs::write(dir.path().join("other.jsonl"), user_line("zzz")).unwrap();
    let child = [
        json!({"type":"attachment","uuid":"att"}).to_string(),
        user_line("u1"),
        user_line("u2"),
        user_line("new-1"),
    ];
    let path = dir.path().join("child-id.jsonl");
    fs::write(&path, child.join("\n")).unwrap();
    assert_eq!(
        lineage(&path),
        json!({"parent": "parent-id", "sharedUserTurns": 2})
    );
}

#[test]
fn lineage_fresh_transcript_has_no_parent() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("a.jsonl"), user_line("u1")).unwrap();
    let path = dir.path().join("b.jsonl");
    fs::write(&path, user_line("fresh")).unwrap();
    assert_eq!(
        lineage(&path),
        json!({"parent": null, "sharedUserTurns": 0})
    );
}

#[test]
fn lineage_missing_file_is_null() {
    let dir = tempdir().unwrap();
    assert_eq!(
        lineage(&dir.path().join("nope.jsonl")),
        json!({"parent": null, "sharedUserTurns": 0})
    );
}

fn prompt_line(uuid: &str) -> String {
    json!({"type":"user","uuid":uuid,"message":{"role":"user","content":"hello"}}).to_string()
}

fn tool_result_line(uuid: &str) -> String {
    json!({"type":"user","uuid":uuid,
           "message":{"role":"user","content":[{"type":"tool_result","content":"x"}]}})
    .to_string()
}

#[test]
fn lineage_ignores_tool_result_user_lines() {
    let dir = tempdir().unwrap();
    let parent = [
        tool_result_line("tr0"),
        prompt_line("u1"),
        tool_result_line("tr1"),
        json!({"type":"user","uuid":"meta","isMeta":true,"message":{"content":"m"}}).to_string(),
        json!({"type":"user","uuid":"blk","message":{"content":[{"type":"text","text":"hi"}]}})
            .to_string(),
    ];
    fs::write(dir.path().join("par.jsonl"), parent.join("\n")).unwrap();
    let child = [
        tool_result_line("tr0"),
        prompt_line("u1"),
        tool_result_line("tr1"),
        json!({"type":"user","uuid":"meta","isMeta":true,"message":{"content":"m"}}).to_string(),
        json!({"type":"user","uuid":"blk","message":{"content":[{"type":"text","text":"hi"}]}})
            .to_string(),
    ];
    let path = dir.path().join("kid.jsonl");
    fs::write(&path, child.join("\n")).unwrap();
    assert_eq!(
        lineage(&path),
        json!({"parent": "par", "sharedUserTurns": 2})
    );
    // A transcript whose only user lines are tool results has no first prompt.
    let only = dir.path().join("only.jsonl");
    fs::write(&only, tool_result_line("tr0")).unwrap();
    assert_eq!(
        lineage(&only),
        json!({"parent": null, "sharedUserTurns": 0})
    );
}

#[test]
fn lineage_prefers_candidate_with_most_shared_prompts() {
    let dir = tempdir().unwrap();
    // "a-sibling" sorts first but shares only the first prompt.
    fs::write(dir.path().join("a-sibling.jsonl"), prompt_line("u1")).unwrap();
    let real = [prompt_line("u1"), prompt_line("u2"), prompt_line("u3")];
    fs::write(dir.path().join("z-real.jsonl"), real.join("\n")).unwrap();
    let child = [prompt_line("u1"), prompt_line("u2"), prompt_line("u3")];
    let path = dir.path().join("m-kid.jsonl");
    fs::write(&path, child.join("\n")).unwrap();
    assert_eq!(
        lineage(&path),
        json!({"parent": "z-real", "sharedUserTurns": 3})
    );
    // Ties go to name order.
    fs::write(dir.path().join("b-tie.jsonl"), real.join("\n")).unwrap();
    assert_eq!(
        lineage(&path),
        json!({"parent": "b-tie", "sharedUserTurns": 3})
    );
}

#[test]
fn lineage_skips_unreadable_sibling() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("a-broken.jsonl")).unwrap();
    fs::write(dir.path().join("b-par.jsonl"), prompt_line("u1")).unwrap();
    let path = dir.path().join("kid.jsonl");
    fs::write(&path, prompt_line("u1")).unwrap();
    assert_eq!(
        lineage(&path),
        json!({"parent": "b-par", "sharedUserTurns": 1})
    );
}

#[test]
fn fork_from_empty_parent_rejected() {
    let (_d, db) = fresh();
    ok(&db, "p", &["state"]);
    assert_eq!(
        err(&db, "c", &["fork", "--from", "p"]),
        "parent has no glossary"
    );
    assert_eq!(ok(&db, "c", &["state"])["forkDone"], false);
}

#[test]
fn fork_none_after_fork_is_noop() {
    let (_d, db) = fresh();
    seed_parent(&db);
    ok(&db, "c", &["fork", "--from", "p"]);
    assert_eq!(
        ok(&db, "c", &["fork", "--none"]),
        json!({"ok": true, "already": true})
    );
    let s = ok(&db, "c", &["state"]);
    assert_eq!(s["forkedFrom"], "p");
    assert_eq!(s["forkTurn"], 3);
    turn(&db, "d", "d-1");
    assert_eq!(
        err(&db, "d", &["fork", "--none"]),
        "session already has a glossary"
    );
}

#[test]
fn child_with_only_ignores_refused() {
    let (_d, db) = fresh();
    seed_parent(&db);
    ok(&db, "c", &["edit", "--definition", "x", "Tmp"]);
    ok(&db, "c", &["forget", "Tmp"]);
    // Remove the term row so only the ignore remains.
    assert_eq!(child_count(&db, "terms"), 0);
    assert_eq!(child_count(&db, "ignores"), 1);
    assert_eq!(
        err(&db, "c", &["fork", "--from", "p"]),
        "session already has a glossary"
    );
}
