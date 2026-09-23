//! Foreign installation exercises production runtime modules with Rust-owned fixtures.
#[path = "support/codex_bridge.rs"]
#[allow(dead_code)]
mod bridge;
#[path = "git_test_support.rs"]
mod git_test_support;

use serde_json::json;
use std::path::Path;

fn rdm(root: &Path, home: &Path, args: &[&str]) {
    assert_cmd::Command::cargo_bin("rdm")
        .unwrap()
        .timeout(std::time::Duration::from_secs(20))
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("RDM_ROOT", root)
        .env("RDM_SESSION", "foreign-distribution")
        .args(args)
        .assert()
        .success();
}

#[test]
fn codex_emitted_runtime_executes_foreign_project_without_source_checkout_assets() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let source = root.join("foreign-source");
    let plan = root.join("foreign-plans");
    let home = root.join("isolated-home");
    for path in [&source, &plan, &home] {
        std::fs::create_dir(path).unwrap();
    }
    rdm(&plan, &home, &["init"]);
    rdm(
        &plan,
        &home,
        &["project", "create", "foreign", "--title", "Foreign"],
    );
    rdm(
        &plan,
        &home,
        &[
            "roadmap",
            "create",
            "delivery",
            "--title",
            "Delivery",
            "--project",
            "foreign",
        ],
    );
    git_test_support::git(&plan, &["add", "."]);
    git_test_support::git(&plan, &["commit", "-m", "seed plans"]);
    rdm(
        &plan,
        &home,
        &[
            "agent-config",
            "codex",
            "--skills",
            "--project",
            "foreign",
            "--out",
            source.to_str().unwrap(),
        ],
    );
    let custom = source.join(".agents/rdm-runtime/custom.mjs");
    std::fs::write(&custom, "// user-owned helper\n").unwrap();
    rdm(
        &plan,
        &home,
        &[
            "agent-config",
            "codex",
            "--skills",
            "--project",
            "foreign",
            "--out",
            source.to_str().unwrap(),
        ],
    );
    assert_eq!(
        std::fs::read_to_string(custom).unwrap(),
        "// user-owned helper\n"
    );
    git_test_support::git(&source, &["init", "-b", "main"]);
    git_test_support::git(&source, &["add", "."]);
    git_test_support::git(&source, &["commit", "-m", "install runtime"]);
    let before = git_test_support::git(&plan, &["rev-parse", "HEAD"]).stdout;
    let module = source.join(".agents/rdm-runtime/lib/codex-runtime.mjs");
    let result = bridge::invoke_request(
        json!({
            "module": module, "export": "runRuntime", "args": [{
                "operation": "estimate", "sourceDir": source, "planRoot": plan,
                "rdmBin": env!("CARGO_BIN_EXE_rdm"), "project": "foreign",
                "session": "foreign-parent", "runDir": root.join("evidence"), "roadmap": "delivery",
                "host": {"capabilities": {"fixture-model": ["high"]}, "tiers": {
                    "small": {"model": "fixture-model", "effort": "high"},
                    "medium": {"model": "fixture-model", "effort": "high"},
                    "large": {"model": "fixture-model", "effort": "high"}
                }}
            }]
        }),
        |name, _| panic!("empty roadmap must not invoke a host: {name}"),
    )
    .expect("emitted runtime must resolve all production dependencies and run");
    assert_eq!(result["reportOnly"], true);
    assert_eq!(result["result"]["proposed"], json!([]));
    assert_eq!(
        git_test_support::git(&plan, &["rev-parse", "HEAD"]).stdout,
        before
    );
    assert!(!source.join(".claude").exists());
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("evidence/manifest.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["identity"]["project"], "foreign");
    assert_eq!(manifest["identity"]["rdmBin"], env!("CARGO_BIN_EXE_rdm"));
    assert_eq!(manifest["status"], "completed");
}

#[test]
fn codex_emitted_manual_commands_read_and_finalize_foreign_phase() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let source = root.join("source");
    let plan = root.join("plans");
    let home = root.join("home");
    for path in [&source, &plan, &home] {
        std::fs::create_dir(path).unwrap();
    }
    rdm(&plan, &home, &["init"]);
    rdm(
        &plan,
        &home,
        &["project", "create", "foreign", "--title", "Foreign"],
    );
    rdm(
        &plan,
        &home,
        &[
            "roadmap",
            "create",
            "delivery",
            "--title",
            "Delivery",
            "--project",
            "foreign",
        ],
    );
    rdm(
        &plan,
        &home,
        &[
            "phase",
            "create",
            "delivery",
            "--roadmap",
            "delivery",
            "--title",
            "Delivery",
            "--project",
            "foreign",
        ],
    );
    rdm(
        &plan,
        &home,
        &[
            "agent-config",
            "codex",
            "--skills",
            "--project",
            "foreign",
            "--out",
            source.to_str().unwrap(),
        ],
    );
    git_test_support::git(&source, &["init", "-b", "main"]);
    git_test_support::git(&source, &["add", "."]);
    git_test_support::git(&source, &["commit", "-m", "install"]);
    let skill = std::fs::read_to_string(source.join(".agents/skills/rdm-do/SKILL.md")).unwrap();
    let invoke = |skill: &str, needle: &str| {
        let command = skill
            .lines()
            .find(|line| line.starts_with('"') && line.contains(needle))
            .expect("emitted command")
            .replace("<phase>", "phase-1-delivery")
            .replace("<roadmap>", "delivery");
        let mut shell = assert_cmd::Command::new("sh");
        shell
            .timeout(std::time::Duration::from_secs(20))
            .current_dir(&source)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", &home)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("RDM_ROOT", &plan)
            .env("RDM_PROJECT", "foreign")
            .env("RDM_BIN", env!("CARGO_BIN_EXE_rdm"))
            .env("RDM_SESSION", "manual-foreign")
            .args(["-c", &command]);
        shell.assert().success().get_output().stdout.clone()
    };
    let roadmap_skill =
        std::fs::read_to_string(source.join(".agents/skills/rdm-roadmap/SKILL.md")).unwrap();
    assert!(
        String::from_utf8(invoke(&roadmap_skill, "roadmap list"))
            .unwrap()
            .contains("delivery")
    );
    let before: serde_json::Value = serde_json::from_slice(&invoke(&skill, "phase show")).unwrap();
    assert_eq!(before["status"], "not-started");
    assert_eq!(before["roadmap"], "delivery");
    invoke(&skill, "--status in-progress");
    invoke(&skill, "--status needs-review");
    let after: serde_json::Value = serde_json::from_slice(&invoke(&skill, "phase show")).unwrap();
    assert_eq!(after["status"], "needs-review");
    let doc = std::fs::read_to_string(
        plan.join("projects/foreign/roadmaps/delivery/phase-1-delivery.md"),
    )
    .unwrap();
    let doc = rdm_core::document::Document::<rdm_core::model::Phase>::parse(&doc).unwrap();
    let head = git_test_support::git(&source, &["rev-parse", "HEAD"]);
    assert_eq!(
        doc.frontmatter.review_sha.as_deref(),
        Some(String::from_utf8_lossy(&head.stdout).trim())
    );
    assert_eq!(doc.frontmatter.review_branch.as_deref(), Some("main"));
}
