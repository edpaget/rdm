//! Generic recorded fake for the argv0 roles `uv`, `docker` and `tar` (the
//! external tools `rdm-measure scbench run` drives; `tar` only when a test
//! puts it first on the runner's `PATH`).
//!
//! Scenario keys: `record` (call log path), `responses` (an array tried in
//! order; each carries `argvContains` — strings that must all be substrings
//! of the space-joined argv), `default` (used when nothing matches). A
//! response is `{ "stdout": "...", "stderr": "...", "exit": 0 }`, plus an
//! optional `place`: `[{ "from": "<abs>", "to": "<abs>" }]`, files copied
//! (parent directories created) before the fake exits, so a test can leave
//! output where the real tool would. Each record line also carries
//! `scbench`: the `SCBENCH_PROBLEMS_PATH` and `SCBENCH_HOME` the call saw.
//!
//! Optional keys that make a response a long `slop-code` run:
//!
//! - `ignore_sigterm: true` — SIGTERM only sets a flag; registered as soon
//!   as the response is chosen, so the readiness file never precedes it;
//! - `grandchild: true` — after `place`, start a long-lived `sleep`
//!   grandchild in this process's group;
//! - `ready: "<abs path>"` — then write
//!   `{ "pid": <own pid>, "grandchild": <pid|null>, "tmpdir": <$TMPDIR|null> }`
//!   there atomically;
//! - `sleepMs: <n>` — then sleep `n` milliseconds before printing and
//!   exiting, so a test can signal the runner while it waits on this call;
//! - `hang: true` — then sleep until killed instead of exiting.

use std::io::Write;
use std::process::ExitCode;

use serde_json::{Value, json};

use crate::scenario;

pub fn run(role: &str) -> ExitCode {
    let scenario = scenario::load(role);
    let pick = |k: &str| std::env::var(k).ok();
    scenario::record(
        &scenario,
        role,
        json!({ "scbench": {
            "SCBENCH_PROBLEMS_PATH": pick("SCBENCH_PROBLEMS_PATH"),
            "SCBENCH_HOME": pick("SCBENCH_HOME"),
        }}),
    );
    let joined = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    let chosen = scenario
        .get("responses")
        .and_then(Value::as_array)
        .and_then(|rs| {
            rs.iter().find(|r| {
                r.get("argvContains")
                    .and_then(Value::as_array)
                    .is_some_and(|needles| {
                        needles
                            .iter()
                            .all(|n| n.as_str().is_some_and(|n| joined.contains(n)))
                    })
            })
        })
        .or_else(|| scenario.get("default"))
        .cloned()
        .unwrap_or(Value::Null);
    if chosen.is_null() {
        eprintln!("fake {role}: no scenario response for `{joined}`");
        return ExitCode::from(2);
    }
    let flag_on = |key: &str| chosen.get(key).and_then(Value::as_bool) == Some(true);
    if flag_on("ignore_sigterm") {
        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        if signal_hook::flag::register(signal_hook::consts::SIGTERM, flag).is_err() {
            eprintln!("fake {role}: cannot register a SIGTERM handler");
            return ExitCode::from(2);
        }
    }
    for item in chosen
        .get("place")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let (Some(from), Some(to)) = (
            item.get("from").and_then(Value::as_str),
            item.get("to").and_then(Value::as_str),
        ) else {
            eprintln!("fake {role}: a `place` entry needs `from` and `to`: {item}");
            return ExitCode::from(2);
        };
        let to = std::path::Path::new(to);
        let placed = to
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::copy(from, to));
        if let Err(e) = placed {
            eprintln!("fake {role}: cannot place {from} at {}: {e}", to.display());
            return ExitCode::from(2);
        }
    }
    let grandchild = if flag_on("grandchild") {
        let Some(pid) = crate::start_grandchild() else {
            eprintln!("fake {role}: cannot start a grandchild");
            return ExitCode::from(2);
        };
        Some(pid)
    } else {
        None
    };
    if let Some(ready) = chosen.get("ready").and_then(Value::as_str) {
        let body = json!({
            "pid": std::process::id(),
            "grandchild": grandchild,
            "tmpdir": pick("TMPDIR"),
        });
        if !crate::write_atomically(ready, body.to_string().as_bytes()) {
            eprintln!("fake {role}: cannot write the readiness file {ready}");
            return ExitCode::from(2);
        }
    }
    scenario::sleep_ms(chosen.get("sleepMs"));
    if let Some(out) = chosen.get("stdout").and_then(Value::as_str) {
        let _ = std::io::stdout().lock().write_all(out.as_bytes());
    }
    if let Some(err) = chosen.get("stderr").and_then(Value::as_str) {
        let _ = std::io::stderr().lock().write_all(err.as_bytes());
    }
    if flag_on("hang") {
        crate::sleep_forever();
    }
    let code = chosen.get("exit").and_then(Value::as_u64).unwrap_or(0);
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}
