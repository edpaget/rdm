use std::path::PathBuf;
use std::process;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use rdm_core::repo::PlanRepo;

#[derive(Parser)]
#[command(name = "rdm", about = "Manage project roadmaps, phases, and tasks")]
struct Cli {
    /// Path to the plan repo root.
    #[arg(long, env = "RDM_ROOT")]
    root: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Initialize a new plan repo.
    Init,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err:#}");
        process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let root = cli
        .root
        .unwrap_or_else(|| std::env::current_dir().expect("cannot determine current directory"));

    match cli.command {
        Command::Init => {
            PlanRepo::init(&root).context("failed to initialize plan repo")?;
            println!("Initialized plan repo at {}", root.display());
        }
    }

    Ok(())
}
