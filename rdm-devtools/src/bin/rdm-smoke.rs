//! `rdm-smoke` — run one program under the bounded, cleanup-guaranteeing
//! process runner from `rdm_devtools::process`.
//!
//! ```text
//! rdm-smoke run [--timeout-secs N] [--cwd DIR] [--private-copy SRC:DEST]
//!               [--stdout-file PATH] -- <program> [args...]
//! ```
//!
//! `--private-copy` is the explicit consent to copy a private file (for
//! example a login file) for the duration of the run: `DEST` is created with
//! mode 0600 just before spawning and removed on every catchable exit path.
//! `SRC:DEST` splits at the first `:`. Without it nothing is copied.
//!
//! After the program is spawned, one line `rdm-smoke: ready pid=<n>` is printed
//! to stderr. The program's stdout is written to `--stdout-file` (mode 0600)
//! or to rdm-smoke's stdout on success; its stderr is discarded.
//!
//! Exit codes: 0 success; the program's own code when it exits non-zero (128 +
//! signal when it was killed by a signal); 124 timeout; 125 stdout cap
//! exceeded; 127 the program could not be started; 128 + signal (130 SIGINT,
//! 143 SIGTERM) when rdm-smoke itself was interrupted; 71 the private copy
//! could not be prepared; 74 cleanup failed; 70 any other runner failure.

use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};
use rdm_devtools::process::{Hooks, ProcessSpec, RunError, run_bounded};

#[derive(Parser)]
#[command(
    name = "rdm-smoke",
    about = "Run a program with a hard timeout, process-group kill and guaranteed cleanup (repository-only tooling)"
)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run one program.
    Run(RunArgs),
}

#[derive(clap::Args)]
struct RunArgs {
    /// Wall-clock limit in seconds before the program's process group is killed.
    #[arg(long, default_value_t = 120)]
    timeout_secs: u64,
    /// Working directory for the program.
    #[arg(long)]
    cwd: Option<PathBuf>,
    /// Copy SRC to DEST (mode 0600) for the run and remove DEST afterwards.
    #[arg(long, value_name = "SRC:DEST")]
    private_copy: Option<String>,
    /// Write the program's stdout here (mode 0600) instead of to stdout.
    #[arg(long)]
    stdout_file: Option<PathBuf>,
    /// The program and its arguments.
    #[arg(last = true, required = true, num_args = 1..)]
    command: Vec<OsString>,
}

fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

fn exit_code(code: i32) -> ExitCode {
    ExitCode::from(u8::try_from(code.clamp(1, 255)).unwrap_or(1))
}

fn run(args: RunArgs) -> ExitCode {
    let Some((program, rest)) = args.command.split_first() else {
        eprintln!("rdm-smoke: no program given after `--`");
        return ExitCode::from(2);
    };
    let copy = match args.private_copy.as_deref().map(|s| s.split_once(':')) {
        None => None,
        Some(Some((src, dest))) if !src.is_empty() && !dest.is_empty() => {
            Some((PathBuf::from(src), PathBuf::from(dest)))
        }
        Some(_) => {
            eprintln!("rdm-smoke: --private-copy expects SRC:DEST with both paths non-empty");
            return ExitCode::from(2);
        }
    };

    let mut spec = ProcessSpec::new(program)
        .args(rest)
        .timeout(Duration::from_secs(args.timeout_secs));
    if let Some(dir) = &args.cwd {
        spec = spec.cwd(dir);
    }

    let mut hooks = Hooks::new().on_spawn(|pid| {
        eprintln!("rdm-smoke: ready pid={pid}");
    });
    if let Some((src, dest)) = &copy {
        hooks = hooks
            .prepare(move || {
                // Create the destination private before any bytes land in it.
                write_private(dest, b"")?;
                write_private(dest, &fs::read(src)?)
            })
            .cleanup(move || remove_if_present(dest));
    }

    match run_bounded(&spec, hooks) {
        Ok(output) => {
            let written = match &args.stdout_file {
                Some(path) => write_private(path, &output.stdout),
                None => io::stdout().lock().write_all(&output.stdout),
            };
            match written {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("rdm-smoke: could not write the program's stdout: {e}");
                    ExitCode::from(70)
                }
            }
        }
        Err(err) => {
            eprintln!("rdm-smoke: {err}");
            match err {
                RunError::NonZeroExit {
                    code: Some(code), ..
                } => exit_code(code),
                RunError::NonZeroExit {
                    signal: Some(sig), ..
                } => exit_code(128 + sig),
                RunError::TimedOut(_) => ExitCode::from(124),
                RunError::OutputCap(_) => ExitCode::from(125),
                RunError::Spawn(_) => ExitCode::from(127),
                RunError::Interrupted(sig) => exit_code(128 + sig),
                RunError::Prepare(_) => ExitCode::from(71),
                RunError::Cleanup(_) => ExitCode::from(74),
                _ => ExitCode::from(70),
            }
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Cmd::Run(args) => run(args),
    }
}
