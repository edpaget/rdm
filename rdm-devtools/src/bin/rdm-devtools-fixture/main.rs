//! Test fixture executable for `rdm-devtools` process-lifecycle tests.
//!
//! Each argv mode produces one deterministic child behaviour, so tests can
//! drive the bounded runner through real processes without any interpreter:
//!
//! - `print <text>` — print `text` and exit 0
//! - `exit <code>` — exit with `code`
//! - `sleep` — sleep until killed
//! - `flood` — write to stdout without end
//! - `stderr-flood <bytes>` — write `bytes` to stderr, then `done` to stdout, exit 0
//! - `spawn-grandchild <pidfile>` — start a long-lived grandchild in the same
//!   process group (its stdio detached), write its pid to `pidfile`, then sleep
//! - `spawn-grandchild-then-exit <pidfile> <code>` — the same grandchild and
//!   pidfile, then exit with `code` at once, leaving the grandchild behind
//! - `print-env <VAR>...` — print `VAR=value` (or `VAR unset`) per name, exit 0
//! - `ready-then-sleep` — print `ready`, then sleep until killed
//! - `ignore-sigterm-then-sleep` — ignore SIGTERM (the handler only sets a
//!   flag), then print `ready`, then sleep until killed
//! - `cat <path>` — print the bytes of `path` and exit 0 (exit 4 if unreadable)
//! - `stderr-exit <text> <code>` — write `text` to stderr and exit with `code`
//!   at once, without reading stdin or writing stdout
//!
//! Invoked under another name (tests symlink it), it plays a fake external
//! tool chosen by its argv0 basename, driven by the scenario JSON file named in
//! `DEVTOOLS_FAKE_SCENARIO`:
//!
//! - `claude` — the paid-dispatch CLI (see `claude.rs`)
//! - `codex` — the Codex CLI and app-server (see `codex.rs`)
//! - `rdm` — rdm's skill emission (see `rdm.rs`)
//! - `uv`, `docker` — the tools `rdm-measure scbench run` drives (see
//!   `tool.rs`), each reading `<role>.scenario.json` beside its symlink; a
//!   `uv` response can also hang, ignore SIGTERM, start a grandchild and
//!   write a readiness file, playing a long `slop-code` run

mod claude;
mod codex;
mod rdm;
mod scenario;
mod tool;

use std::io::Write;
use std::process::{Command, ExitCode, Stdio};
use std::time::Duration;

pub(crate) fn sleep_forever() -> ! {
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}

/// Starts a long-lived `sleep` grandchild in this process's group and
/// returns its pid, or `None` when it cannot be started.
pub(crate) fn start_grandchild() -> Option<u32> {
    let exe = std::env::current_exe().ok()?;
    // Inherits this process's group; stdio detached so a leaked grandchild
    // never holds the runner's pipes.
    let grandchild = Command::new(exe)
        .arg("sleep")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    Some(grandchild.id())
}

/// Writes `bytes` to `path` atomically: `<path>.tmp`, then a rename.
pub(crate) fn write_atomically(path: &str, bytes: &[u8]) -> bool {
    let tmp = format!("{path}.tmp");
    let written = std::fs::File::create(&tmp).and_then(|mut f| f.write_all(bytes));
    written.is_ok() && std::fs::rename(&tmp, path).is_ok()
}

/// Starts a [`start_grandchild`] grandchild and writes its pid to `pidfile`
/// atomically. Returns whether both steps succeeded.
fn spawn_grandchild(pidfile: &str) -> bool {
    start_grandchild().is_some_and(|pid| write_atomically(pidfile, pid.to_string().as_bytes()))
}

/// The argv0 basename this invocation was started under.
fn argv0_name() -> String {
    std::env::args_os()
        .next()
        .and_then(|a| {
            std::path::Path::new(&a)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        })
        .unwrap_or_default()
}

fn main() -> ExitCode {
    match argv0_name().as_str() {
        "claude" => return claude::run(),
        "codex" => return codex::run(),
        "rdm" => return rdm::run(),
        role @ ("uv" | "docker") => return tool::run(role),
        _ => {}
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = args.first().map(String::as_str).unwrap_or("");
    let rest = args.get(1..).unwrap_or(&[]);
    let mut stdout = std::io::stdout().lock();
    match mode {
        "print" => {
            let _ = writeln!(stdout, "{}", rest.join(" "));
            ExitCode::SUCCESS
        }
        "exit" => {
            let code = rest.first().and_then(|c| c.parse::<u8>().ok()).unwrap_or(1);
            ExitCode::from(code)
        }
        "sleep" => sleep_forever(),
        "flood" => {
            let block = vec![b'x'; 64 * 1024];
            loop {
                if stdout.write_all(&block).is_err() {
                    return ExitCode::from(3);
                }
            }
        }
        "stderr-flood" => {
            let total: usize = rest.first().and_then(|n| n.parse().ok()).unwrap_or(0);
            let block = vec![b'e'; 64 * 1024];
            let mut stderr = std::io::stderr().lock();
            let mut written = 0;
            while written < total {
                let n = block.len().min(total - written);
                if stderr.write_all(&block[..n]).is_err() {
                    return ExitCode::from(3);
                }
                written += n;
            }
            let _ = writeln!(stdout, "done");
            ExitCode::SUCCESS
        }
        "spawn-grandchild" => {
            if rest.first().is_none_or(|p| !spawn_grandchild(p)) {
                return ExitCode::from(2);
            }
            let _ = writeln!(stdout, "ready");
            let _ = stdout.flush();
            sleep_forever()
        }
        "spawn-grandchild-then-exit" => {
            let code = rest.get(1).and_then(|c| c.parse::<u8>().ok());
            match (rest.first(), code) {
                (Some(pidfile), Some(code)) if spawn_grandchild(pidfile) => ExitCode::from(code),
                _ => ExitCode::from(2),
            }
        }
        "print-env" => {
            for name in rest {
                match std::env::var_os(name) {
                    Some(v) => {
                        let _ = writeln!(stdout, "{name}={}", v.to_string_lossy());
                    }
                    None => {
                        let _ = writeln!(stdout, "{name} unset");
                    }
                }
            }
            ExitCode::SUCCESS
        }
        "cat" => match rest.first().map(std::fs::read) {
            Some(Ok(bytes)) => {
                let _ = stdout.write_all(&bytes);
                ExitCode::SUCCESS
            }
            _ => ExitCode::from(4),
        },
        "stderr-exit" => {
            let code = rest.get(1).and_then(|c| c.parse::<u8>().ok()).unwrap_or(1);
            eprintln!("{}", rest.first().map_or("", String::as_str));
            ExitCode::from(code)
        }
        "ready-then-sleep" => {
            let _ = writeln!(stdout, "ready");
            let _ = stdout.flush();
            sleep_forever()
        }
        "ignore-sigterm-then-sleep" => {
            let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            if signal_hook::flag::register(signal_hook::consts::SIGTERM, flag).is_err() {
                return ExitCode::from(2);
            }
            let _ = writeln!(stdout, "ready");
            let _ = stdout.flush();
            sleep_forever()
        }
        _ => {
            eprintln!("rdm-devtools-fixture: unknown mode {mode:?}");
            ExitCode::from(2)
        }
    }
}
