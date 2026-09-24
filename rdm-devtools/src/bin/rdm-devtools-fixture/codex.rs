//! Fake `codex` (argv0 `codex`) for the coexistence flow: `--version`,
//! `plugin add <id> --json`, the `app-server --stdio` skill-catalog protocol
//! and `exec`. Every call is recorded (argv, cwd, the environment names it
//! saw — never values of secrets).
//!
//! Scenario keys (all optional): `record`, `pidfile` (written by a hanging
//! step), `appServer` `{ exitBeforeReply, errorReply, hang, omit, disable,
//! catalogErrors }`, `exec` `{ hang, exit, wrongPath, planGate,
//! mutateProtected }`.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use serde_json::{Value, json};

use crate::scenario;

fn env_path(key: &str) -> PathBuf {
    std::env::var_os(key).map(PathBuf::from).unwrap_or_default()
}

fn hang(scenario: &Value) -> ! {
    if let Some(p) = scenario.get("pidfile").and_then(Value::as_str) {
        let _ = std::fs::write(p, std::process::id().to_string());
    }
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let dest = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &dest)?;
        } else {
            std::fs::copy(entry.path(), dest)?;
        }
    }
    Ok(())
}

fn plugin_add(id: &str) -> ExitCode {
    let home = env_path("HOME");
    let name = id.split('@').next().unwrap_or(id);
    let market: Value = std::fs::read(home.join(".agents/plugins/marketplace.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null);
    let Some(rel) = market
        .get("plugins")
        .and_then(Value::as_array)
        .and_then(|ps| {
            ps.iter()
                .find(|p| p.get("name").and_then(Value::as_str) == Some(name))
        })
        .and_then(|p| {
            p.get("source")
                .and_then(|s| s.get("path"))
                .and_then(Value::as_str)
        })
    else {
        eprintln!("fake codex: plugin {id} is not in the personal marketplace");
        return ExitCode::from(4);
    };
    let installed = env_path("CODEX_HOME")
        .join("plugins/cache/personal")
        .join(name)
        .join("0.1.0");
    if let Err(e) = copy_tree(&home.join(rel), &installed) {
        eprintln!("fake codex: install failed: {e}");
        return ExitCode::from(4);
    }
    println!("{}", json!({ "installedPath": installed }));
    ExitCode::SUCCESS
}

fn skills_in(dir: &Path, depth: usize) -> Vec<PathBuf> {
    // Every `<dir>/<…depth levels…>/SKILL.md`, sorted.
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for e in entries {
        let p = e.path();
        if depth == 0 {
            if p.file_name().is_some_and(|n| n == "SKILL.md") {
                out.push(p);
            }
        } else if p.is_dir() {
            out.extend(skills_in(&p, depth - 1));
        }
    }
    out
}

fn catalog(cwd: &str, knobs: &Value) -> Value {
    let home = env_path("HOME");
    let codex_home = env_path("CODEX_HOME");
    let mut skills = Vec::new();
    let skill = |path: &Path, scope: &str, plugin: Option<String>| {
        let name = path
            .parent()
            .and_then(Path::file_name)
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut s = json!({"name": name, "path": path, "enabled": true, "scope": scope});
        if let Some(p) = plugin {
            s["pluginId"] = Value::String(p);
        }
        s
    };
    for p in skills_in(&Path::new(cwd).join(".agents/skills"), 1) {
        skills.push(skill(&p, "repo", None));
    }
    for p in skills_in(&home.join(".agents/skills"), 1) {
        skills.push(skill(&p, "user", None));
    }
    let cache = codex_home.join("plugins/cache/personal");
    for p in skills_in(&cache, 4) {
        let plugin = p
            .strip_prefix(&cache)
            .ok()
            .and_then(|r| r.iter().next())
            .map(|n| format!("{}@personal", n.to_string_lossy()));
        skills.push(skill(&p, "plugin", plugin));
    }
    if let Some(omit) = knobs.get("omit").and_then(Value::as_str) {
        skills.retain(|s| !s["path"].as_str().unwrap_or("").contains(omit));
    }
    if let Some(off) = knobs.get("disable").and_then(Value::as_str) {
        for s in &mut skills {
            if s["path"].as_str().unwrap_or("").contains(off) {
                s["enabled"] = Value::Bool(false);
            }
        }
    }
    json!({
        "cwd": cwd,
        "skills": skills,
        "errors": knobs.get("catalogErrors").cloned().unwrap_or_else(|| json!([])),
    })
}

fn app_server(scenario: &Value) -> ExitCode {
    let knobs = scenario.get("appServer").cloned().unwrap_or(Value::Null);
    let flag = |k: &str| knobs.get(k).and_then(Value::as_bool).unwrap_or(false);
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        match msg.get("method").and_then(Value::as_str) {
            Some("initialize") => {
                if flag("exitBeforeReply") {
                    eprintln!("fake codex: exiting before the initialize reply");
                    return ExitCode::from(3);
                }
                // Noise the client must skip.
                let _ = writeln!(out, "fake codex app-server starting (not JSON)");
                let _ = writeln!(
                    out,
                    "{}",
                    json!({"id": 1, "result": {"serverInfo": {"name": "fake-codex"}}})
                );
                let _ = out.flush();
            }
            Some("skills/list") => {
                if flag("hang") {
                    hang(scenario);
                }
                if flag("errorReply") {
                    let _ = writeln!(
                        out,
                        "{}",
                        json!({"id": 2, "error": {"code": -32000, "message": "catalog unavailable"}})
                    );
                } else {
                    let cwd = msg
                        .get("params")
                        .and_then(|p| p.get("cwds"))
                        .and_then(|c| c.get(0))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned();
                    let data = catalog(&cwd, &knobs);
                    let _ = writeln!(out, "{}", json!({"id": 2, "result": {"data": [data]}}));
                }
                let _ = out.flush();
            }
            _ => {}
        }
    }
    ExitCode::SUCCESS
}

fn arg_after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

fn exec(args: &[String], scenario: &Value) -> ExitCode {
    let knobs = scenario.get("exec").cloned().unwrap_or(Value::Null);
    let flag = |k: &str| knobs.get(k).and_then(Value::as_bool).unwrap_or(false);
    let home = env_path("HOME");
    if !env_path("CODEX_HOME").join("auth.json").is_file() {
        eprintln!("fake codex: not logged in (no CODEX_HOME/auth.json)");
        return ExitCode::from(5);
    }
    if flag("mutateProtected") {
        let user = home.join(".agents/skills/rdm-roadmap/SKILL.md");
        let mut text = std::fs::read(&user).unwrap_or_default();
        text.extend_from_slice(b"\ntampered\n");
        let _ = std::fs::write(&user, text);
    }
    if flag("hang") {
        hang(scenario);
    }
    if let Some(code) = knobs.get("exit").and_then(Value::as_u64) {
        eprintln!("fake codex: exec failing with {code}");
        return ExitCode::from(u8::try_from(code).unwrap_or(1));
    }
    let cd = arg_after(args, "--cd").unwrap_or(".");
    let repository = Path::new(cd).join(".agents/skills/rdm-roadmap/SKILL.md");
    let content = std::fs::read_to_string(&repository).unwrap_or_default();
    println!(
        "{}",
        json!({"type": "item.completed", "item": {
            "type": "command_execution", "exit_code": 0,
            "command": format!("cat {}", repository.display()), "aggregated_output": content
        }})
    );
    // `wrongPath` reads the right file but answers with a competing copy.
    let selected = if flag("wrongPath") {
        home.join(".agents/skills/rdm-roadmap/SKILL.md")
    } else {
        repository
    };
    let gate = knobs
        .get("planGate")
        .and_then(Value::as_str)
        .unwrap_or("needs-plan-review: an independent plan review gates implementation");
    if let Some(answer) = arg_after(args, "--output-last-message") {
        let _ = std::fs::write(
            answer,
            json!({"selected_path": selected, "plan_gate": gate}).to_string(),
        );
    }
    ExitCode::SUCCESS
}

pub fn run() -> ExitCode {
    let scenario = scenario::load("codex");
    let args: Vec<String> = std::env::args().skip(1).collect();
    let auth_present = env_path("CODEX_HOME").join("auth.json").is_file();
    scenario::record(
        &scenario,
        "codex",
        json!({ "authCopyPresent": auth_present }),
    );
    match args.first().map(String::as_str) {
        Some("--version") => {
            println!("codex-cli 0.0.0-fake");
            ExitCode::SUCCESS
        }
        Some("plugin") if args.get(1).map(String::as_str) == Some("add") => {
            plugin_add(args.get(2).map_or("", String::as_str))
        }
        Some("app-server") => app_server(&scenario),
        Some("exec") => exec(&args, &scenario),
        _ => {
            eprintln!("fake codex: unsupported invocation {args:?}");
            ExitCode::from(2)
        }
    }
}
