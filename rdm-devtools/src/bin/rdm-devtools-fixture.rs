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
//! - `print-env <VAR>...` — print `VAR=value` (or `VAR unset`) per name, exit 0
//! - `ready-then-sleep` — print `ready`, then sleep until killed
//! - `cat <path>` — print the bytes of `path` and exit 0 (exit 4 if unreadable)

use std::io::Write;
use std::process::{Command, ExitCode, Stdio};
use std::time::Duration;

fn sleep_forever() -> ! {
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}

fn main() -> ExitCode {
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
            let Some(pidfile) = rest.first() else {
                return ExitCode::from(2);
            };
            let Ok(exe) = std::env::current_exe() else {
                return ExitCode::from(2);
            };
            // Inherits this process's group; stdio detached so a leaked
            // grandchild never holds the runner's pipes.
            let Ok(grandchild) = Command::new(exe)
                .arg("sleep")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            else {
                return ExitCode::from(2);
            };
            let tmp = format!("{pidfile}.tmp");
            let written = std::fs::File::create(&tmp)
                .and_then(|mut f| f.write_all(grandchild.id().to_string().as_bytes()));
            if written.is_err() || std::fs::rename(&tmp, pidfile).is_err() {
                return ExitCode::from(2);
            }
            let _ = writeln!(stdout, "ready");
            let _ = stdout.flush();
            sleep_forever()
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
        "ready-then-sleep" => {
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
