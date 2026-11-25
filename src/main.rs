mod apply;
mod db;
mod entry;
mod labels;
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
    /// One-time cost notice flag.
    Notice {
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        session: String,
        #[arg(long = "check-and-set", required = true)]
        check_and_set: bool,
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
        Command::Notice { db, session, .. } => {
            let conn = open_session(&db, &session)?;
            Ok(json!({ "shown": session::check_and_set_notice(&conn)? }))
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
