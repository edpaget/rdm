//! Fake `claude` (argv0 `claude`): reads the prompt from stdin, records the
//! call, and answers from the scenario.
//!
//! Scenario keys: `record` (call log path), `responses` (an array tried in
//! order; each may carry `match` — a substring the prompt must contain — and
//! `model` — the `--model` value), `default` (used when nothing matches).
//! A response is `{ "stdout": "...", "stderr": "...", "exit": 0, "sleepMs": 0 }`.

use std::io::{Read, Write};
use std::process::ExitCode;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::scenario;

pub fn run() -> ExitCode {
    let mut prompt = String::new();
    let _ = std::io::stdin().read_to_string(&mut prompt);
    let scenario = scenario::load("claude");
    let args: Vec<String> = std::env::args().skip(1).collect();
    let model = args
        .iter()
        .position(|a| a == "--model")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_default();
    let sha: String = Sha256::digest(prompt.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    scenario::record(
        &scenario,
        "claude",
        json!({ "model": model, "promptBytes": prompt.len(), "promptSha256": sha }),
    );
    let chosen = scenario
        .get("responses")
        .and_then(Value::as_array)
        .and_then(|rs| {
            rs.iter().find(|r| {
                r.get("match")
                    .and_then(Value::as_str)
                    .is_none_or(|m| prompt.contains(m))
                    && r.get("model")
                        .and_then(Value::as_str)
                        .is_none_or(|m| m == model)
            })
        })
        .or_else(|| scenario.get("default"))
        .cloned()
        .unwrap_or(Value::Null);
    if chosen.is_null() {
        eprintln!("fake claude: no scenario response (set DEVTOOLS_FAKE_SCENARIO)");
        return ExitCode::from(2);
    }
    scenario::sleep_ms(chosen.get("sleepMs"));
    if let Some(out) = chosen.get("stdout").and_then(Value::as_str) {
        let _ = std::io::stdout().lock().write_all(out.as_bytes());
    }
    if let Some(err) = chosen.get("stderr").and_then(Value::as_str) {
        let _ = std::io::stderr().lock().write_all(err.as_bytes());
    }
    let code = chosen.get("exit").and_then(Value::as_u64).unwrap_or(0);
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}
