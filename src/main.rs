mod db;
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
}

fn open_session(db: &std::path::Path, session: &str) -> Result<rusqlite::Connection> {
    let conn = db::open(db)?;
    session::ensure(&conn, session)?;
    Ok(conn)
}

fn read_turn_id() -> Result<String> {
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .context("cannot read stdin")?;
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
        Command::Capture {
            db, session, off, ..
        } => {
            let conn = open_session(&db, &session)?;
            Ok(json!({ "captureOff": session::set_capture_off(&conn, &session, off)? }))
        }
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
    match run(cli) {
        Ok(v) => println!("{v}"),
        Err(e) => fail(e),
    }
}
