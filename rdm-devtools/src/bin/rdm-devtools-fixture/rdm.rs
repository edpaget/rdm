//! Fake `rdm` (argv0 `rdm`) for the coexistence flow: records its call and,
//! for `agent-config codex --skills … --out <dir>`, writes one repository
//! skill carrying a `needs-plan-review` gate line. Real emission bytes are
//! covered by `codex_distribution.rs`; this fake covers only the coexistence
//! contract.

use std::process::ExitCode;

use serde_json::json;

use crate::scenario;

pub fn run() -> ExitCode {
    let scenario = scenario::load("rdm");
    scenario::record(&scenario, "rdm", json!({}));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let is_emit = args.first().map(String::as_str) == Some("agent-config")
        && args.get(1).map(String::as_str) == Some("codex")
        && args.iter().any(|a| a == "--skills");
    let out = args
        .iter()
        .position(|a| a == "--out")
        .and_then(|i| args.get(i + 1));
    let (true, Some(out)) = (is_emit, out) else {
        eprintln!("fake rdm: unsupported invocation {args:?}");
        return ExitCode::from(2);
    };
    let skill = std::path::Path::new(out).join(".agents/skills/rdm-roadmap/SKILL.md");
    let body = "---\nname: rdm-roadmap\ndescription: Create an rdm roadmap (repository copy).\n---\n\nEvery item you create is stamped needs-plan-review; an independent plan review must clear it before implementation.\n";
    let written = skill
        .parent()
        .map(std::fs::create_dir_all)
        .transpose()
        .and_then(|_| std::fs::write(&skill, body));
    match written {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("fake rdm: {e}");
            ExitCode::from(3)
        }
    }
}
