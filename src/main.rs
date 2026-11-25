mod db;
mod session;

use std::path::PathBuf;
use std::process;

use anyhow::Result;
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
