//! `rdm agent-config claude --skills` into a fresh temp tree: the shipped
//! inventory with structurally valid frontmatter, idempotent re-emission
//! that never touches a user's file, the Pi and `--user` scope boundaries,
//! and independence from the plan repo.
//!
//! Byte identity of the emitted workflows and agent definition with this
//! checkout's `.claude/` copies is `cli_agent_config.rs`'s
//! `agent_config_workflows_are_byte_identical_to_source` and
//! `agent_config_agents_are_byte_identical_to_source` (and rdm-core's
//! `generate_workflows_are_byte_identical_to_source`).

use std::collections::BTreeSet;

use crate::support::{
    Emit, SKILLS, cleanup_report, entries, frontmatter, repo_root, skill_problems, tree,
};

#[test]
fn emitted_skills_and_agent_have_structured_frontmatter() {
    let emit = Emit::new();
    let (out, _) = emit.skills("cli");
    assert_eq!(
        entries(&out),
        BTreeSet::from([".claude".to_owned()]),
        "the emission lands only under its --out directory"
    );
    let skills: BTreeSet<String> = SKILLS.iter().map(|s| (*s).to_owned()).collect();
    let problems = skill_problems(&out.join(".claude/skills"), &skills);
    assert!(problems.is_empty(), "{problems:#?}");

    let agents = out.join(".claude/agents");
    let defs = entries(&agents);
    assert!(!defs.is_empty(), "at least one agent definition ships");
    for def in defs {
        let text = std::fs::read_to_string(agents.join(&def)).expect("read agent");
        let f = frontmatter(&text).unwrap_or_else(|e| panic!("{def}: {e}"));
        assert_eq!(
            f.get("name").map(String::as_str),
            def.strip_suffix(".md"),
            "{def}: the agent's name is its file stem"
        );
    }
    assert!(
        !entries(&out.join(".claude/workflows")).is_empty(),
        "the workflow engines ship alongside the skills"
    );
}

#[test]
fn reemit_is_idempotent_and_spares_user_files() {
    let emit = Emit::new();
    let (out, _) = emit.skills("cli");
    let workflows = out.join(".claude/workflows");
    let notes = workflows.join("notes.txt");
    std::fs::write(
        &notes,
        "these are my own notes, rdm did not write this file\n",
    )
    .expect("plant a user file");
    let before = tree(&out);

    let (_, again) = emit.skills("cli");
    assert!(
        cleanup_report(&again)
            .iter()
            .all(|l| !l.starts_with("Removed ")),
        "nothing superseded was present, so nothing is removed: {:?}",
        cleanup_report(&again)
    );
    assert_eq!(
        tree(&out),
        before,
        "re-emission rewrote every file byte for byte"
    );
    let source = repo_root().join(".claude/workflows");
    for name in entries(&workflows) {
        if name == "notes.txt" {
            continue;
        }
        assert_eq!(
            std::fs::read(workflows.join(&name)).ok(),
            std::fs::read(source.join(&name)).ok(),
            "{name} is still byte-identical to this checkout's copy"
        );
    }
}

#[test]
fn pi_and_user_emissions_write_no_runtime_and_run_no_cleanup() {
    let emit = Emit::new();
    let pi_out = emit.path("pi");
    let pi = emit.ok(&[
        "agent-config",
        "pi",
        "--skills",
        "--project",
        "distro-check",
        "--out",
        &pi_out.to_string_lossy(),
    ]);
    assert!(!pi_out.join(".claude/workflows").exists());
    assert!(!pi_out.join(".claude/agents").exists());
    assert!(cleanup_report(&pi).is_empty(), "{:?}", cleanup_report(&pi));

    let user = emit.ok(&["agent-config", "claude", "--skills", "--user"]);
    let home = &emit.sandbox.home;
    assert!(
        home.join(".claude/skills").is_dir(),
        "--user writes skills under the sandbox HOME"
    );
    assert!(!home.join(".claude/workflows").exists());
    assert!(!home.join(".claude/agents").exists());
    assert!(
        cleanup_report(&user).is_empty(),
        "{:?}",
        cleanup_report(&user)
    );
}

#[test]
fn skills_emission_ignores_a_missing_rdm_root() {
    let emit = Emit::new();
    let missing = emit.path("does-not-exist-plan-repo");
    let out_dir = emit.path("no-plan-repo");
    let out = emit.rdm_env(
        &[
            "agent-config",
            "claude",
            "--skills",
            "--project",
            "distro-check",
            "--out",
            &out_dir.to_string_lossy(),
        ],
        &[("RDM_ROOT", &missing)],
    );
    assert!(
        out.status.success(),
        "--skills never needs the plan repo: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out_dir.join(".claude/skills").is_dir());
    assert!(!missing.exists(), "the missing plan repo is not created");
}
