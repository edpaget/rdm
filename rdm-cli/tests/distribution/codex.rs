//! The Codex manual-distribution channel: which skills are withheld and
//! said so, the emitted skills' commands executed against a foreign plan,
//! the checked-in `.agents/skills` kept equal to what the real
//! `scripts/gen-codex-skills.sh` generates, and the `scripts/rdm-dev.sh`
//! development entrypoint.
//!
//! Both scripts end in `cargo run … -- <args>`. They run against a fake
//! `cargo` first on `PATH` — a two-line exec stub this module writes into the
//! test's temp tree — that records the call and execs the binary under test
//! on the arguments after `--`, so no nested build runs. `gen-codex-skills.sh`
//! runs from a scratch copy (it derives its output root from its own
//! location, so it can only write into scratch); `rdm-dev.sh` runs in place
//! and writes nothing.
//!
//! Project and user placement, the obsolete-path negatives and the `--plugin`
//! rejection are `cli_agent_config.rs`'s
//! `codex_project_emits_instructions_and_supported_skills`,
//! `codex_user_paths_separate_config_and_skills` and
//! `codex_plugin_rejection_names_the_supported_channel`.

use std::collections::BTreeSet;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

use crate::downstream::{BRANCH, Downstream, PHASE, PROJECT, ROADMAP};
use crate::plan_fixture::Sandbox;
use crate::support::{Emit, entries, repo_root, tree, tree_diff};

/// The fenced ```bash blocks of a Markdown document, one entry per line.
fn bash_lines(markdown: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in markdown.lines() {
        if inside {
            if line.trim_start().starts_with("```") {
                inside = false;
            } else {
                out.push(line.to_owned());
            }
        } else if line.trim_start() == "```bash" {
            inside = true;
        }
    }
    out
}

/// Splits one shell command line into words (single and double quotes,
/// backslash escapes; `#` starts a comment between words). Expansions are
/// kept as text: the words only select which line to run.
fn words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' => {
                if in_word {
                    words.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            '#' if !in_word => break,
            '\'' => {
                in_word = true;
                for q in chars.by_ref() {
                    if q == '\'' {
                        break;
                    }
                    cur.push(q);
                }
            }
            '"' => {
                in_word = true;
                while let Some(q) = chars.next() {
                    match q {
                        '"' => break,
                        '\\' => cur.extend(chars.next()),
                        _ => cur.push(q),
                    }
                }
            }
            '\\' => {
                in_word = true;
                cur.extend(chars.next());
            }
            _ => {
                in_word = true;
                cur.push(c);
            }
        }
    }
    if in_word {
        words.push(cur);
    }
    words
}

/// The lines of `markdown`'s bash blocks whose subcommand (the words after
/// the binary) starts with `sub`.
fn commands(markdown: &str, sub: &[&str]) -> Vec<String> {
    bash_lines(markdown)
        .into_iter()
        .filter(|l| {
            let w = words(l);
            w.len() > sub.len() && w[1..=sub.len()].iter().zip(sub).all(|(a, b)| a == b)
        })
        .collect()
}

#[test]
fn every_withheld_skill_is_named_in_the_notice() {
    let emit = Emit::new();
    let (claude, _) = emit.skills("claude");
    let codex = emit.path("codex");
    let out = emit.ok(&[
        "agent-config",
        "codex",
        "--skills",
        "--project",
        PROJECT,
        "--out",
        &codex.to_string_lossy(),
    ]);
    let shipped = entries(&claude.join(".claude/skills"));
    let emitted = entries(&codex.join(".agents/skills"));
    assert!(
        !emitted.is_empty() && emitted.is_subset(&shipped),
        "{emitted:?}"
    );
    let withheld: BTreeSet<&String> = shipped.difference(&emitted).collect();
    assert_eq!(withheld.len(), 7, "{withheld:?}");
    let notice = String::from_utf8_lossy(&out.stderr);
    for skill in withheld {
        assert!(
            notice.contains(skill.as_str()),
            "{skill} is withheld silently; the notice was:\n{notice}"
        );
    }
}

#[test]
fn emitted_commands_execute_against_a_foreign_plan() {
    let ds = Downstream::new().unwrap_or_else(|f| panic!("{f}"));
    let codex = ds.plan.dir.path().join("codex");
    let out = ds
        .sandbox
        .command(&ds.bin)
        .args([
            "agent-config",
            "codex",
            "--skills",
            "--project",
            PROJECT,
            "--out",
        ])
        .arg(&codex)
        .current_dir(ds.plan.dir.path())
        .output()
        .expect("emit the Codex skills");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let skill = |name: &str| {
        std::fs::read_to_string(codex.join(".agents/skills").join(name).join("SKILL.md"))
            .expect("read an emitted skill")
    };
    let run = ds
        .run()
        .env("RDM_BIN", &ds.bin_str())
        .session("codex-distribution-fixture");

    let list = commands(&skill("rdm-roadmap"), &["roadmap", "list"]);
    assert_eq!(
        list.len(),
        1,
        "one emitted `roadmap list` command: {list:?}"
    );
    let listed = ds
        .plan
        .run_ok(&run, &list[0])
        .unwrap_or_else(|f| panic!("{f}"));
    assert!(
        listed.contains(ROADMAP),
        "the emitted command read the foreign plan:\n{listed}"
    );

    let updates = commands(&skill("rdm-do"), &["phase", "update"]);
    assert_eq!(updates.len(), 2, "start and finalize updates: {updates:?}");
    for line in &updates {
        let line = line.replace("<phase>", PHASE).replace("<roadmap>", ROADMAP);
        ds.plan
            .run_ok(&run, &line)
            .unwrap_or_else(|f| panic!("{f}"));
    }
    let phase = ds
        .plan
        .phase(PROJECT, ROADMAP, PHASE)
        .unwrap_or_else(|f| panic!("{f}"));
    assert_eq!(phase["status"], "needs-review", "{phase}");

    let pending = ds
        .sandbox
        .rdm()
        .args(["review", "pending", "--format", "json"])
        .env("RDM_ROOT", &ds.plan.root)
        .env("RDM_PROJECT", PROJECT)
        .current_dir(&ds.repo)
        .output()
        .expect("review pending");
    assert!(
        pending.status.success(),
        "{}",
        String::from_utf8_lossy(&pending.stderr)
    );
    let pending: Value = serde_json::from_slice(&pending.stdout).expect("pending json");
    let item = pending
        .as_array()
        .into_iter()
        .flatten()
        .find(|i| i["identifier"] == format!("{ROADMAP}/{PHASE}"))
        .unwrap_or_else(|| panic!("the finalized phase is pending on this branch: {pending}"));
    assert_eq!(item["branch"], BRANCH, "{item}");
    assert_eq!(
        item["review_sha"],
        ds.rev("HEAD").expect("HEAD").as_str(),
        "{item}"
    );
}

/// A fake `cargo` first on `PATH`: records each call, then execs the binary
/// under test on the arguments after the first `--`.
struct FakeCargo {
    bin: PathBuf,
    record: PathBuf,
}

impl FakeCargo {
    fn new(dir: &Path) -> Self {
        let bin = dir.join("fake-bin");
        std::fs::create_dir_all(&bin).expect("create fake bin");
        let record = dir.join("cargo-calls.txt");
        let stub = format!(
            "#!/bin/sh\n{{ printf 'call cwd=%s\\n' \"$(pwd)\"; for a in \"$@\"; do printf 'arg=%s\\n' \"$a\"; done; }} >> '{record}'\nwhile [ \"$#\" -gt 0 ] && [ \"$1\" != -- ]; do shift; done\n[ \"$#\" -gt 0 ] || {{ echo 'fake cargo: no -- separator' >&2; exit 2; }}\nshift\nexec '{rdm}' \"$@\"\n",
            record = record.display(),
            rdm = crate::plan_fixture::rdm_bin(),
        );
        let cargo = bin.join("cargo");
        std::fs::write(&cargo, stub).expect("write fake cargo");
        std::fs::set_permissions(&cargo, std::fs::Permissions::from_mode(0o755))
            .expect("chmod fake cargo");
        Self { bin, record }
    }

    fn path(&self) -> String {
        format!("{}:/usr/bin:/bin", self.bin.display())
    }

    /// Each recorded call's arguments.
    fn calls(&self) -> Vec<Vec<String>> {
        let text = std::fs::read_to_string(&self.record).unwrap_or_default();
        let mut calls: Vec<Vec<String>> = Vec::new();
        for line in text.lines() {
            if line.starts_with("call ") {
                calls.push(Vec::new());
            } else if let (Some(arg), Some(call)) = (line.strip_prefix("arg="), calls.last_mut()) {
                call.push(arg.to_owned());
            }
        }
        calls
    }
}

#[test]
fn local_skills_match_gen_codex_skills_output() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let scratch = tmp.path().join("scratch");
    for rel in ["scripts/gen-codex-skills.sh", "docs/principles.md"] {
        let to = scratch.join(rel);
        std::fs::create_dir_all(to.parent().expect("a parent")).expect("mkdir");
        std::fs::copy(repo_root().join(rel), &to).expect("copy into scratch");
    }
    let cargo = FakeCargo::new(tmp.path());
    let sandbox = Sandbox::new(&tmp.path().join("user")).expect("sandbox");
    let out = sandbox
        .command("sh")
        .arg(scratch.join("scripts/gen-codex-skills.sh"))
        .env("PATH", cargo.path())
        .current_dir(&scratch)
        .output()
        .expect("run gen-codex-skills.sh");
    assert!(
        out.status.success(),
        "gen-codex-skills.sh failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let calls = cargo.calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    let manifest = calls[0]
        .iter()
        .skip_while(|a| *a != "--manifest-path")
        .nth(1)
        .map(PathBuf::from)
        .expect("a --manifest-path argument");
    assert_eq!(
        manifest.parent().and_then(|p| p.canonicalize().ok()),
        scratch.canonicalize().ok(),
        "the generator targets the scratch copy, never the checkout"
    );
    assert_eq!(
        entries(&scratch),
        BTreeSet::from([
            ".agents".to_owned(),
            "docs".to_owned(),
            "scripts".to_owned()
        ]),
        "the generator writes only the skills tree into scratch"
    );
    let drift = tree_diff(
        &tree(&repo_root().join(".agents/skills")),
        &tree(&scratch.join(".agents/skills")),
    );
    assert!(
        drift.is_empty(),
        "the checked-in .agents/skills differ from generator output (first tree: checked in; second: generated):\n  {}\nRegenerate with:\n  sh scripts/gen-codex-skills.sh",
        drift.join("\n  ")
    );
}

/// `scripts/rdm-dev.sh <args>` from a foreign cwd with `env` added to the
/// sandbox and the fake `cargo` on `PATH`.
fn dev_wrapper(tmp: &Path, cargo: &FakeCargo, env: &[(&str, &Path)], args: &[&str]) -> Output {
    let sandbox = Sandbox::new(&tmp.join("user")).expect("sandbox");
    let foreign = tmp.join("foreign-cwd");
    std::fs::create_dir_all(&foreign).expect("create a foreign cwd");
    let mut cmd: Command = sandbox.command("sh");
    cmd.arg(repo_root().join("scripts/rdm-dev.sh"))
        .args(args)
        .env("PATH", cargo.path())
        .current_dir(&foreign);
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("run rdm-dev.sh")
}

#[test]
fn dev_wrapper_runs_from_a_foreign_cwd_and_keeps_the_session() {
    let ds = Downstream::new().unwrap_or_else(|f| panic!("{f}"));
    let tmp = ds.plan.dir.path();
    let cargo = FakeCargo::new(tmp);
    let session = Path::new("codex-wrapper-fixture");
    let env = [
        ("RDM_ROOT", ds.plan.root.as_path()),
        ("RDM_SESSION", session),
    ];
    let first = dev_wrapper(tmp, &cargo, &env, &["session", "id"]);
    let second = dev_wrapper(tmp, &cargo, &env, &["session", "id"]);
    for out in [&first, &second] {
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    assert_eq!(
        String::from_utf8_lossy(&first.stdout).trim(),
        "codex-wrapper-fixture"
    );
    assert_eq!(
        first.stdout, second.stdout,
        "the session survives across shells"
    );
    let manifest = repo_root().join("Cargo.toml");
    for call in cargo.calls() {
        let tail: Vec<&str> = call.iter().map(String::as_str).collect();
        let at = tail
            .iter()
            .position(|a| *a == "--manifest-path")
            .expect("a --manifest-path argument");
        assert_eq!(Path::new(tail[at + 1]), manifest, "{call:?}");
        assert!(
            tail.ends_with(&["--bin", "rdm", "--", "session", "id"]),
            "{call:?}"
        );
    }

    let info = dev_wrapper(tmp, &cargo, &env, &["info", "--format", "json"]);
    assert!(
        info.status.success(),
        "{}",
        String::from_utf8_lossy(&info.stderr)
    );
    let info: Value = serde_json::from_slice(&info.stdout).expect("info json");
    assert_eq!(
        info["project"], "rdm",
        "RDM_PROJECT defaults to rdm: {info}"
    );
}

#[test]
fn dev_wrapper_refuses_missing_session_or_plan_repo() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let plan = tmp.path().join("plan");
    std::fs::create_dir_all(&plan).expect("create plan dir");
    let cargo = FakeCargo::new(tmp.path());
    let no_session = dev_wrapper(
        tmp.path(),
        &cargo,
        &[("RDM_ROOT", &plan)],
        &["session", "id"],
    );
    assert!(
        !no_session.status.success(),
        "a missing RDM_SESSION is refused"
    );
    let missing = tmp.path().join("missing-plan");
    let no_plan = dev_wrapper(
        tmp.path(),
        &cargo,
        &[("RDM_ROOT", &missing), ("RDM_SESSION", Path::new("s"))],
        &["session", "id"],
    );
    assert!(!no_plan.status.success(), "a missing plan repo is refused");
    assert!(
        cargo.calls().is_empty(),
        "cargo is never invoked: {:?}",
        cargo.calls()
    );
}
