//! Generic recorded fake for the argv0 roles `uv` and `docker` (the external
//! tools `rdm-measure scbench run` drives).
//!
//! Scenario keys: `record` (call log path), `responses` (an array tried in
//! order; each carries `argvContains` — strings that must all be substrings
//! of the space-joined argv), `default` (used when nothing matches). A
//! response is `{ "stdout": "...", "stderr": "...", "exit": 0 }`. Each record
//! line also carries `scbench`: the `SCBENCH_PROBLEMS_PATH` and
//! `SCBENCH_HOME` the call saw.

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
    if let Some(out) = chosen.get("stdout").and_then(Value::as_str) {
        let _ = std::io::stdout().lock().write_all(out.as_bytes());
    }
    if let Some(err) = chosen.get("stderr").and_then(Value::as_str) {
        let _ = std::io::stderr().lock().write_all(err.as_bytes());
    }
    let code = chosen.get("exit").and_then(Value::as_u64).unwrap_or(0);
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}
