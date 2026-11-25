//! The `apply` subcommand: validates a model diff for one turn and stores the
//! accepted changes in one transaction (spec 6.3).

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::labels::{self, Stoplist};
use crate::session;

pub(crate) const MAX_LABEL: usize = 32;
pub(crate) const MAX_DEFINITION: usize = 200;
pub(crate) const MAX_STATUS: usize = 200;
const MAX_QUOTE: usize = 300;
const MIN_MEANING_WORD: usize = 4;
pub(crate) const NOW: &str = "strftime('%Y-%m-%dT%H:%M:%SZ','now')";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApplyInput {
    turn_id: String,
    #[serde(default)]
    prompt: String,
    #[serde(default)]
    answer: String,
    #[serde(default)]
    stoplist: Vec<String>,
    #[serde(default)]
    diff: Diff,
}

#[derive(Deserialize, Default)]
struct Diff {
    #[serde(default)]
    new: Vec<DefinitionOp>,
    #[serde(default)]
    redefine: Vec<DefinitionOp>,
    #[serde(default)]
    status: Vec<StatusOp>,
}

#[derive(Deserialize, Serialize)]
struct DefinitionOp {
    #[serde(default)]
    label: String,
    #[serde(default)]
    definition: String,
    #[serde(default)]
    quote: String,
}

#[derive(Deserialize, Serialize)]
struct StatusOp {
    #[serde(default)]
    label: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    quote: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    New,
    Redefine,
    Status,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::New => "new",
            Kind::Redefine => "redefine",
            Kind::Status => "status",
        }
    }

    fn max_text(self) -> usize {
        match self {
            Kind::Status => MAX_STATUS,
            _ => MAX_DEFINITION,
        }
    }
}

/// One change of the diff, normalised; `text` is the definition or the status.
struct Op {
    kind: Kind,
    label: String,
    key: String,
    text: String,
    quote: String,
    payload: Value,
}

enum Outcome {
    Accept,
    Noop,
    Reject(&'static str),
}

/// The turn being applied: its index, its text and the state before this call.
struct TurnCtx {
    sid: String,
    turn_id: String,
    index: i64,
    prompt: String,
    answer: String,
    stop: Stoplist,
    /// Latest definition/status turn index per label key, before this call.
    last_update: HashMap<String, Option<i64>>,
}

/// Current state of a defined label.
struct LabelState {
    pinned: bool,
    version: i64,
    definition: String,
    user_edited: bool,
    /// The latest status of the current definition version.
    status: Option<String>,
}

fn op_list(diff: Diff) -> Result<Vec<Op>> {
    let make = |kind, label: &str, text: &str, quote: &str, payload: Value| {
        let label = collapse_ws(label);
        Op {
            kind,
            key: label.to_lowercase(),
            label,
            text: text.trim().to_string(),
            quote: quote.to_string(),
            payload,
        }
    };
    let mut ops = Vec::new();
    for (kind, list) in [(Kind::New, diff.new), (Kind::Redefine, diff.redefine)] {
        for d in list {
            let payload = serde_json::to_value(&d)?;
            ops.push(make(kind, &d.label, &d.definition, &d.quote, payload));
        }
    }
    for s in diff.status {
        let payload = serde_json::to_value(&s)?;
        ops.push(make(Kind::Status, &s.label, &s.status, &s.quote, payload));
    }
    Ok(keep_last_duplicates(ops))
}

/// Of several ops with the same kind and label key, only the last one is kept.
fn keep_last_duplicates(ops: Vec<Op>) -> Vec<Op> {
    let mut seen = HashSet::new();
    let mut kept: Vec<Op> = ops
        .into_iter()
        .rev()
        .filter(|op| seen.insert((op.kind, op.key.clone())))
        .collect();
    kept.reverse();
    kept
}

/// Collapses every run of whitespace to one space and trims the ends.
pub(crate) fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn mentions_label(text: &str, label: &str) -> bool {
    !labels::mentions(text, &[label.to_string()]).is_empty()
}

fn too_long(op: &Op) -> bool {
    op.label.chars().count() > MAX_LABEL
        || op.text.chars().count() > op.kind.max_text()
        || op.quote.chars().count() > MAX_QUOTE
}

/// Deterministic floor of "the quote states the meaning": the quote holds at
/// least one word of `MIN_MEANING_WORD`+ letters from the definition that is
/// not part of the label itself.
fn quote_states_meaning(op: &Op) -> bool {
    let label_words: Vec<String> = op
        .key
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_string)
        .collect();
    let words: Vec<String> = op
        .text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= MIN_MEANING_WORD && w.chars().all(char::is_alphabetic))
        .map(str::to_lowercase)
        .filter(|w| !label_words.contains(w))
        .collect();
    !labels::mentions(&op.quote, &words).is_empty()
}

/// Checks that need only the turn text and the stoplist.
fn check_text(op: &Op, ctx: &TurnCtx) -> Option<&'static str> {
    if too_long(op) {
        return Some("too-long");
    }
    if op.text.is_empty() {
        return Some("empty");
    }
    if ctx.stop.contains(&op.label) {
        return Some("stoplisted");
    }
    None
}

/// Checks on the quote against the turn's full prompt and answer.
fn check_quote(op: &Op, ctx: &TurnCtx) -> Option<&'static str> {
    let quote = collapse_ws(&op.quote);
    if quote.is_empty() || !(ctx.prompt.contains(&quote) || ctx.answer.contains(&quote)) {
        return Some("quote-not-in-turn");
    }
    if op.kind == Kind::Status {
        let turn_text = format!("{}\n{}", ctx.prompt, ctx.answer);
        if !mentions_label(&turn_text, &op.label) {
            return Some("label-not-in-turn");
        }
        return None;
    }
    if !mentions_label(&quote, &op.label) {
        return Some("label-not-in-quote");
    }
    if !quote_states_meaning(op) {
        return Some("quote-does-not-state-meaning");
    }
    None
}

fn is_ignored(tx: &Transaction, sid: &str, key: &str) -> Result<bool> {
    Ok(tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM ignores WHERE session_id = ?1 AND label_key = ?2)",
        params![sid, key],
        |r| r.get(0),
    )?)
}

fn label_state(tx: &Transaction, sid: &str, key: &str) -> Result<Option<LabelState>> {
    let Some(mut state) = definition_state(tx, sid, key)? else {
        return Ok(None);
    };
    state.status = current_status(tx, sid, key, state.version)?;
    Ok(Some(state))
}

fn current_status(tx: &Transaction, sid: &str, key: &str, version: i64) -> Result<Option<String>> {
    Ok(tx
        .query_row(
            "SELECT status FROM statuses
             WHERE session_id = ?1 AND label_key = ?2 AND definition_version = ?3
             ORDER BY turn_index DESC, rowid DESC LIMIT 1",
            params![sid, key, version],
            |r| r.get(0),
        )
        .optional()?)
}

fn definition_state(tx: &Transaction, sid: &str, key: &str) -> Result<Option<LabelState>> {
    Ok(tx
        .query_row(
            "SELECT t.pinned, d.version, d.definition, d.source
             FROM terms t JOIN definitions d
               ON d.session_id = t.session_id AND d.label_key = t.label_key
             WHERE t.session_id = ?1 AND t.label_key = ?2
             ORDER BY d.version DESC LIMIT 1",
            params![sid, key],
            |r| {
                Ok(LabelState {
                    pinned: r.get::<_, i64>(0)? != 0,
                    version: r.get(1)?,
                    definition: r.get(2)?,
                    user_edited: r.get::<_, Option<String>>(3)?.as_deref() == Some("user"),
                    status: None,
                })
            },
        )
        .optional()?)
}

fn last_update(tx: &Transaction, sid: &str, key: &str) -> Result<Option<i64>> {
    Ok(tx.query_row(
        "SELECT max(t) FROM (
           SELECT max(turn_index) AS t FROM definitions WHERE session_id = ?1 AND label_key = ?2
           UNION ALL
           SELECT max(turn_index) FROM statuses WHERE session_id = ?1 AND label_key = ?2)",
        params![sid, key],
        |r| r.get(0),
    )?)
}

/// Whether `text` restates `current` (trimmed, case-insensitive).
fn same_status(current: Option<&str>, text: &str) -> bool {
    current.is_some_and(|c| c.trim().to_lowercase() == text.trim().to_lowercase())
}

/// Checks against the stored glossary; `state` is the label's live state.
fn check_state(op: &Op, ctx: &TurnCtx, state: Option<&LabelState>) -> Outcome {
    let Some(state) = state else {
        return match op.kind {
            Kind::New => Outcome::Accept,
            _ => Outcome::Reject("not-defined"),
        };
    };
    if op.kind == Kind::New {
        return Outcome::Reject("already-defined");
    }
    if state.pinned {
        return Outcome::Reject("pinned");
    }
    if op.kind == Kind::Redefine && state.user_edited {
        return Outcome::Reject("user-edited");
    }
    let last = ctx.last_update.get(&op.key).copied().flatten();
    if last.is_some_and(|t| ctx.index <= t) {
        return Outcome::Reject("not-newer");
    }
    if op.kind == Kind::Redefine && state.definition.trim() == op.text {
        return Outcome::Noop;
    }
    if op.kind == Kind::Status && same_status(state.status.as_deref(), &op.text) {
        return Outcome::Noop;
    }
    Outcome::Accept
}

fn validate(tx: &Transaction, op: &Op, ctx: &TurnCtx) -> Result<Outcome> {
    if let Some(reason) = check_text(op, ctx) {
        return Ok(Outcome::Reject(reason));
    }
    if is_ignored(tx, &ctx.sid, &op.key)? {
        return Ok(Outcome::Reject("ignored"));
    }
    if let Some(reason) = check_quote(op, ctx) {
        return Ok(Outcome::Reject(reason));
    }
    let state = label_state(tx, &ctx.sid, &op.key)?;
    Ok(check_state(op, ctx, state.as_ref()))
}

fn insert_definition(tx: &Transaction, op: &Op, ctx: &TurnCtx, version: i64) -> Result<()> {
    tx.execute(
        &format!(
            "INSERT INTO definitions
               (session_id, label_key, version, definition, turn_index, quote, created_at, source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, {NOW}, 'model')"
        ),
        params![ctx.sid, op.key, version, op.text, ctx.index, op.quote],
    )?;
    Ok(())
}

fn store(tx: &Transaction, op: &Op, ctx: &TurnCtx) -> Result<()> {
    let current = label_state(tx, &ctx.sid, &op.key)?.map(|s| s.version);
    match (op.kind, current) {
        (Kind::New, _) => {
            tx.execute(
                "INSERT INTO terms (session_id, label, label_key, pinned, created_turn)
                 VALUES (?1, ?2, ?3, 0, ?4)",
                params![ctx.sid, op.label, op.key, ctx.index],
            )?;
            insert_definition(tx, op, ctx, 1)
        }
        (Kind::Redefine, Some(v)) => insert_definition(tx, op, ctx, v + 1),
        (Kind::Status, Some(v)) => {
            tx.execute(
                &format!(
                    "INSERT INTO statuses
                       (session_id, label_key, definition_version, status, turn_index, quote,
                        created_at, source)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, {NOW}, 'model')"
                ),
                params![ctx.sid, op.key, v, op.text, ctx.index, op.quote],
            )?;
            Ok(())
        }
        (_, None) => anyhow::bail!("label {} vanished during apply", op.label),
    }
}

fn record_rejection(tx: &Transaction, op: &Op, ctx: &TurnCtx, reason: &str) -> Result<()> {
    tx.execute(
        &format!(
            "INSERT INTO rejections (session_id, turn_id, op, label, reason, payload, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, {NOW})"
        ),
        params![
            ctx.sid,
            ctx.turn_id,
            op.kind.name(),
            op.label,
            reason,
            op.payload.to_string()
        ],
    )?;
    Ok(())
}

fn already_applied(tx: &Transaction, sid: &str, turn_id: &str) -> Result<bool> {
    Ok(tx.query_row(
        "SELECT applied_at IS NOT NULL FROM turns WHERE session_id = ?1 AND turn_id = ?2",
        params![sid, turn_id],
        |r| r.get(0),
    )?)
}

/// Latest update turn of every label the ops touch, read before any write.
fn snapshot_last_updates(
    tx: &Transaction,
    sid: &str,
    ops: &[Op],
) -> Result<HashMap<String, Option<i64>>> {
    let mut out = HashMap::new();
    for op in ops {
        if !out.contains_key(&op.key) {
            out.insert(op.key.clone(), last_update(tx, sid, &op.key)?);
        }
    }
    Ok(out)
}

/// Validates and stores every op; returns the applied count and the rejections.
fn apply_ops(tx: &Transaction, ops: &[Op], ctx: &TurnCtx) -> Result<(usize, Vec<Value>)> {
    let mut applied = 0;
    let mut rejected = Vec::new();
    for op in ops {
        match validate(tx, op, ctx)? {
            Outcome::Accept => {
                store(tx, op, ctx)?;
                applied += 1;
            }
            Outcome::Noop => {}
            Outcome::Reject(reason) => {
                record_rejection(tx, op, ctx, reason)?;
                rejected.push(json!({ "op": op.kind.name(), "label": op.label, "reason": reason }));
            }
        }
    }
    Ok((applied, rejected))
}

pub fn run(conn: &mut Connection, sid: &str, raw: &str) -> Result<Value> {
    let input: ApplyInput =
        serde_json::from_str(raw).context("stdin is not a valid apply object")?;
    let index = session::turn_index_of(conn, sid, &input.turn_id)?
        .with_context(|| format!("unknown turnId {:?}", input.turn_id))?;
    let ops = op_list(input.diff)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if already_applied(&tx, sid, &input.turn_id)? {
        return Ok(json!({ "applied": 0, "rejected": [] }));
    }
    let ctx = TurnCtx {
        sid: sid.to_string(),
        index,
        prompt: collapse_ws(&input.prompt),
        answer: collapse_ws(&input.answer),
        stop: Stoplist::new(&input.stoplist),
        last_update: snapshot_last_updates(&tx, sid, &ops)?,
        turn_id: input.turn_id,
    };
    let (applied, rejected) = apply_ops(&tx, &ops, &ctx)?;
    tx.execute(
        &format!("UPDATE turns SET applied_at = {NOW} WHERE session_id = ?1 AND turn_id = ?2"),
        params![sid, ctx.turn_id],
    )?;
    tx.commit()?;
    Ok(json!({ "applied": applied, "rejected": rejected }))
}
