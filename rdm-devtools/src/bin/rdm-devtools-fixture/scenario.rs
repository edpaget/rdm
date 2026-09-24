//! Shared plumbing for the argv0-dispatched fake roles: the scenario file
//! (named by `DEVTOOLS_FAKE_SCENARIO`, deliberately not `RDM_`-prefixed so it
//! survives an `RDM_*` scrub; otherwise `<role>.scenario.json` beside the
//! symlink the role was started through) and the per-call record log.

use std::fs::OpenOptions;
use std::io::Write;

use serde_json::{Value, json};

/// The environment variable naming the scenario JSON file.
pub const SCENARIO_ENV: &str = "DEVTOOLS_FAKE_SCENARIO";

/// The scenario, or `null` when none is configured or it cannot be read.
pub fn load(role: &str) -> Value {
    let beside = || {
        let argv0 = std::env::args_os().next()?;
        let dir = std::path::Path::new(&argv0).parent()?.to_owned();
        Some(dir.join(format!("{role}.scenario.json")).into_os_string())
    };
    std::env::var_os(SCENARIO_ENV)
        .filter(|p| !p.is_empty())
        .or_else(beside)
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null)
}

/// Appends one JSON line describing this call (role, argv, cwd, the names of
/// the environment variables it saw, plus `extra`) to the scenario's `record`
/// file, when one is named.
pub fn record(scenario: &Value, role: &str, extra: Value) {
    let Some(path) = scenario.get("record").and_then(Value::as_str) else {
        return;
    };
    let mut env_keys: Vec<String> = std::env::vars_os()
        .map(|(k, _)| k.to_string_lossy().into_owned())
        .collect();
    env_keys.sort();
    let pick = |k: &str| std::env::var(k).ok();
    let mut line = json!({
        "role": role,
        "argv": std::env::args().skip(1).collect::<Vec<_>>(),
        "cwd": std::env::current_dir().map(|d| d.display().to_string()).unwrap_or_default(),
        "envKeys": env_keys,
        "env": {
            "HOME": pick("HOME"),
            "CODEX_HOME": pick("CODEX_HOME"),
            "XDG_CONFIG_HOME": pick("XDG_CONFIG_HOME"),
            "GIT_CONFIG_GLOBAL": pick("GIT_CONFIG_GLOBAL"),
            "GIT_CONFIG_SYSTEM": pick("GIT_CONFIG_SYSTEM"),
        },
    });
    if let (Some(obj), Value::Object(more)) = (line.as_object_mut(), extra) {
        obj.extend(more);
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        // One write per line, so concurrent callers never interleave.
        let _ = f.write_all(format!("{line}\n").as_bytes());
    }
}

/// Sleeps `ms` milliseconds (a scenario's `sleepMs`).
pub fn sleep_ms(v: Option<&Value>) {
    if let Some(ms) = v.and_then(Value::as_u64) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
}
