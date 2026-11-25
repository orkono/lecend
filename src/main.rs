mod apply;
mod db;
mod edit;
mod entry;
mod fork;
mod labels;
mod lineage;
mod query;
mod scan;
mod session;

use std::io::Read;
use std::path::PathBuf;
use std::process;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use serde_json::{json, Value};

#[derive(Parser)]
#[command(
    name = "lecend",
    version,
    about = "Glossary storage for Claude Code sessions"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print the crate version as JSON.
    Version,
    /// Open (creating if needed) the database and report its state.
    Doctor {
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        session: String,
    },
    /// Record a turn id (stdin `{"turnId":..}`) and print its index.
    Turn {
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        session: String,
    },
    /// Print the session state.
    State {
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        session: String,
    },
    /// Model-call budget counter.
    Usage {
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        session: String,
        #[arg(long, required = true)]
        bump: bool,
    },
    /// One-time cost notice flag: check and set it, or reset it after a failed showing.
    Notice {
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        session: String,
        #[arg(
            long = "check-and-set",
            conflicts_with = "reset",
            required_unless_present = "reset"
        )]
        check_and_set: bool,
        #[arg(long)]
        reset: bool,
    },
    /// Decide whether a turn needs the model (stdin `{prompt, answer, stoplist}`).
    Scan {
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        session: String,
    },
    /// Validate and store a model diff (stdin `{turnId, prompt, answer, stoplist, diff}`).
    Apply {
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        session: String,
    },
    /// Switch capture off or on for the session.
    Capture {
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        session: String,
        #[arg(long, conflicts_with = "on", required_unless_present = "on")]
        off: bool,
        #[arg(long)]
        on: bool,
    },
    /// Show one label (remaining args joined by a space).
    Show {
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        session: String,
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        label: Vec<String>,
    },
    /// List all labels, most recently updated first.
    List {
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        session: String,
        #[arg(long)]
        json: bool,
    },
    /// Show every definition version and status of a label.
    History {
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        session: String,
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        label: Vec<String>,
    },
    /// Correct a label: --definition, --status, --pin or --unpin (one per call).
    #[command(group(clap::ArgGroup::new("action").required(true).multiple(false)
        .args(["definition", "status", "pin", "unpin"])))]
    Edit {
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        session: String,
        #[arg(long, allow_hyphen_values = true)]
        definition: Option<String>,
        #[arg(long, allow_hyphen_values = true)]
        status: Option<String>,
        #[arg(long)]
        pin: bool,
        #[arg(long)]
        unpin: bool,
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        label: Vec<String>,
    },
    /// Delete a label with its history and keep capture from bringing it back.
    Forget {
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        session: String,
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        label: Vec<String>,
    },
    /// Let capture define a forgotten label again (deleted history stays gone).
    Unforget {
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        session: String,
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        label: Vec<String>,
    },
    /// Copy a parent session's glossary once (`--from`), or mark no parent (`--none`).
    #[command(group(clap::ArgGroup::new("source").required(true).multiple(false)
        .args(["from", "none"])))]
    Fork {
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        session: String,
        #[arg(long)]
        from: Option<String>,
        #[arg(long = "at-turn", requires = "from")]
        at_turn: Option<i64>,
        #[arg(long)]
        none: bool,
    },
    /// Find the fork parent of a transcript by shared user-message uuids.
    Lineage {
        #[arg(long)]
        transcript: PathBuf,
    },
}

/// Which correction `edit` makes; clap guarantees exactly one is given.
fn edit_action(
    definition: Option<String>,
    status: Option<String>,
    pin: bool,
    unpin: bool,
) -> Result<edit::Action> {
    match (definition, status, pin, unpin) {
        (Some(text), None, false, false) => Ok(edit::Action::Definition(text)),
        (None, Some(text), false, false) => Ok(edit::Action::Status(text)),
        (None, None, true, false) => Ok(edit::Action::Pin(true)),
        (None, None, false, true) => Ok(edit::Action::Pin(false)),
        _ => bail!("edit needs exactly one of --definition, --status, --pin, --unpin"),
    }
}

fn open_session(db: &std::path::Path, session: &str) -> Result<rusqlite::Connection> {
    let conn = db::open(db)?;
    session::ensure(&conn, session)?;
    Ok(conn)
}

fn read_stdin() -> Result<String> {
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .context("cannot read stdin")?;
    Ok(buf)
}

fn read_turn_id() -> Result<String> {
    let buf = read_stdin()?;
    let v: Value = serde_json::from_str(&buf).context("stdin is not valid JSON")?;
    match v.get("turnId").and_then(Value::as_str) {
        Some(id) if !id.is_empty() => Ok(id.to_string()),
        _ => bail!("stdin must be a JSON object with a non-empty string \"turnId\""),
    }
}

fn run(cli: Cli) -> Result<Value> {
    match cli.command {
        Command::Version => Ok(json!({ "version": env!("CARGO_PKG_VERSION") })),
        Command::Doctor { db, session } => {
            let conn = db::open(&db)?;
            session::ensure(&conn, &session)?;
            Ok(json!({
                "version": env!("CARGO_PKG_VERSION"),
                "db": db.to_string_lossy(),
                "schemaVersion": db::SCHEMA_VERSION,
                "session": session,
            }))
        }
        Command::Turn { db, session } => {
            let conn = open_session(&db, &session)?;
            let turn_id = read_turn_id()?;
            Ok(json!({ "turnIndex": session::record_turn(&conn, &session, &turn_id)? }))
        }
        Command::State { db, session } => {
            let conn = open_session(&db, &session)?;
            session::state(&conn, &session)
        }
        Command::Usage { db, session, .. } => {
            let conn = open_session(&db, &session)?;
            Ok(json!({ "modelCalls": session::bump_usage(&conn, &session)? }))
        }
        Command::Notice {
            db, session, reset, ..
        } => {
            let conn = open_session(&db, &session)?;
            let shown = if reset {
                session::reset_notice(&conn)?;
                false
            } else {
                session::check_and_set_notice(&conn)?
            };
            Ok(json!({ "shown": shown }))
        }
        Command::Scan { db, session } => {
            let conn = open_session(&db, &session)?;
            scan::run(&conn, &session, &read_stdin()?)
        }
        Command::Apply { db, session } => {
            let mut conn = open_session(&db, &session)?;
            apply::run(&mut conn, &session, &read_stdin()?)
        }
        Command::Capture {
            db, session, off, ..
        } => {
            let conn = open_session(&db, &session)?;
            Ok(json!({ "captureOff": session::set_capture_off(&conn, &session, off)? }))
        }
        Command::Edit {
            db,
            session,
            definition,
            status,
            pin,
            unpin,
            label,
        } => {
            let action = edit_action(definition, status, pin, unpin)?;
            let mut conn = open_session(&db, &session)?;
            edit::edit(&mut conn, &session, &label.join(" "), action)?;
            Ok(json!({ "ok": true }))
        }
        Command::Forget { db, session, label } => {
            let mut conn = open_session(&db, &session)?;
            edit::forget(&mut conn, &session, &label.join(" "))?;
            Ok(json!({ "ok": true }))
        }
        Command::Unforget { db, session, label } => {
            let conn = open_session(&db, &session)?;
            edit::unforget(&conn, &session, &label.join(" "))?;
            Ok(json!({ "ok": true }))
        }
        Command::Fork {
            db,
            session,
            from,
            at_turn,
            ..
        } => {
            let conn = open_session(&db, &session)?;
            match from {
                Some(parent) => fork::from(&conn, &session, &parent, at_turn),
                None => fork::none(&conn, &session),
            }
        }
        Command::Lineage { transcript } => Ok(lineage::run(&transcript)),
        Command::Show { .. } | Command::List { .. } | Command::History { .. } => {
            unreachable!("text commands are handled by dispatch")
        }
    }
}

/// Runs a command and returns what to print on stdout (text for the query
/// commands, JSON otherwise).
fn dispatch(cli: Cli) -> Result<String> {
    match cli.command {
        Command::Show { db, session, label } => {
            query::show(&open_session(&db, &session)?, &session, &label.join(" "))
        }
        Command::List { db, session, json } => {
            query::list(&open_session(&db, &session)?, &session, json)
        }
        Command::History { db, session, label } => {
            query::history(&open_session(&db, &session)?, &session, &label.join(" "))
        }
        command => Ok(run(Cli { command })?.to_string()),
    }
}

fn fail(e: anyhow::Error) -> ! {
    println!("{}", json!({ "error": format!("{e:#}") }));
    process::exit(1)
}

fn main() {
    let cli = Cli::try_parse().unwrap_or_else(|e| {
        use clap::error::ErrorKind;
        if matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) {
            e.exit();
        }
        fail(anyhow::anyhow!(e.to_string().trim().to_string()))
    });
    match dispatch(cli) {
        Ok(out) => println!("{out}"),
        Err(e) => fail(e),
    }
}
