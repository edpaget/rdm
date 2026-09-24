//! The Claude Code web sandbox loop, end to end: the shipped
//! `templates/claude-code-web/.claude/hooks/SessionStart.sh` bootstraps a
//! plan repo from a bare `file://` remote into a sandboxed `HOME`/XDG, the
//! bootstrapped clone serves reads through the global config, a source
//! commit carrying a `Done:` line flips the phase through `rdm hook
//! post-commit`, and the update pushes back to the remote.
//!
//! No network: the only `curl` on `PATH` is a stub that fails loudly, and the
//! hook finds `rdm` (a symlink to the binary under test) first, so its
//! install branch never runs. The two-worktree `rdm review pending` scoping
//! the retired shell also checked is `cli_review.rs`'s
//! `pending_scopes_to_current_branch_and_fails_open` and
//! [`crate::worktree_review`].

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use serde_json::Value;

use crate::workflow_support::repo_root;
use crate::{Plan, field, stderr, stdout};

const PROJECT: &str = "verify";
const ROADMAP: &str = "verify-demo";
const PHASE: &str = "phase-1-ping";

/// `rdm <args>` with no `--root`, resolved through the sandbox's global
/// config the way a sandboxed session would; returns stdout.
fn rdm_global(plan: &Plan, cwd: &Path, args: &[&str]) -> String {
    let out = plan
        .sandbox
        .rdm()
        .args(args)
        .env("RDM_SESSION", "cli-loop")
        .current_dir(cwd)
        .output()
        .expect("spawn rdm");
    assert!(
        out.status.success(),
        "`rdm {}` failed: {}",
        args.join(" "),
        stderr(&out)
    );
    stdout(&out)
}

#[test]
fn session_start_bootstraps_and_done_line_completes_the_phase() {
    let plan = Plan::empty();
    let tmp = plan.dir.path();

    // A seeded plan repo, pushed to a bare remote standing in for GitHub.
    plan.ok(&["init", "--default-project", PROJECT]);
    plan.ok(&[
        "roadmap",
        "create",
        ROADMAP,
        "--title",
        "Verify Demo",
        "--body",
        "End-to-end verification roadmap.",
        "--no-edit",
        "--project",
        PROJECT,
    ]);
    plan.ok(&[
        "phase",
        "create",
        "ping",
        "--title",
        "Ping",
        "--number",
        "1",
        "--body",
        "## Purpose\n\nEnd-to-end verification ping phase.",
        "--no-edit",
        "--roadmap",
        ROADMAP,
        "--project",
        PROJECT,
    ]);
    plan.ok(&["commit", "-m", "seed: verify-demo/phase-1-ping"]);
    let origin = tmp.join("plan-origin.git");
    plan.git(
        tmp,
        &[
            "clone",
            "--quiet",
            "--bare",
            &plan.root.to_string_lossy(),
            &origin.to_string_lossy(),
        ],
    );

    // The sandbox: rdm already installed, and a curl that must never run.
    let bin = plan.sandbox.home.join(".local/bin");
    std::fs::create_dir_all(&bin).expect("create sandbox bin");
    std::os::unix::fs::symlink(crate::plan_fixture::rdm_bin(), bin.join("rdm")).expect("link rdm");
    let curl = bin.join("curl");
    std::fs::write(
        &curl,
        "#!/bin/sh\necho 'curl stub: this test has no network' >&2\nexit 97\n",
    )
    .expect("write curl stub");
    std::fs::set_permissions(&curl, std::fs::Permissions::from_mode(0o755))
        .expect("chmod curl stub");

    let hook = repo_root().join("templates/claude-code-web/.claude/hooks/SessionStart.sh");
    let out = plan
        .sandbox
        .command("bash")
        .arg(&hook)
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("RDM_PLAN_REPO", format!("file://{}", origin.display()))
        .env("RDM_DEFAULT_PROJECT", PROJECT)
        .current_dir(tmp)
        .output()
        .expect("run SessionStart.sh");
    assert!(
        out.status.success(),
        "SessionStart.sh exited {:?}:\n{}\n{}",
        out.status.code(),
        stdout(&out),
        stderr(&out)
    );

    let clone = plan.sandbox.xdg_data.join("rdm/plan-repo");
    assert!(
        clone.join("rdm.toml").is_file(),
        "the plan repo was cloned into {}",
        clone.display()
    );
    let root = rdm_global(&plan, tmp, &["config", "get", "root", "--raw"]);
    assert_eq!(
        Path::new(root.trim()).canonicalize().ok(),
        clone.canonicalize().ok(),
        "the global config root names the bootstrapped clone: {root}"
    );

    let roadmaps: Value = serde_json::from_str(&rdm_global(
        &plan,
        tmp,
        &["roadmap", "list", "--project", PROJECT, "--format", "json"],
    ))
    .expect("roadmap list json");
    assert!(field(&roadmaps, "slug").contains(&ROADMAP.to_owned()));
    let phase = |plan: &Plan| -> Value {
        serde_json::from_str(&rdm_global(
            plan,
            tmp,
            &[
                "phase",
                "show",
                PHASE,
                "--roadmap",
                ROADMAP,
                "--project",
                PROJECT,
                "--no-body",
                "--format",
                "json",
            ],
        ))
        .expect("phase show json")
    };
    assert_eq!(phase(&plan)["status"], "not-started");

    // A source commit carrying a Done: line, applied by the post-commit hook.
    let source = tmp.join("source");
    std::fs::create_dir_all(&source).expect("create source repo");
    plan.git(&source, &["init", "--quiet", "-b", "main"]);
    plan.git(
        &source,
        &["commit", "--quiet", "--allow-empty", "-m", "chore: initial"],
    );
    std::fs::write(source.join("feature.txt"), "hello\n").expect("write feature");
    plan.git(&source, &["add", "feature.txt"]);
    plan.git(
        &source,
        &[
            "commit",
            "--quiet",
            "-m",
            &format!("feat: implement ping\n\nDone: {ROADMAP}/{PHASE}"),
        ],
    );
    let sha = plan.git(&source, &["rev-parse", "HEAD"]);
    let hooked = plan
        .sandbox
        .rdm()
        .arg("--root")
        .arg(&clone)
        .args(["hook", "post-commit"])
        .env("RDM_SESSION", "cli-loop")
        .current_dir(&source)
        .output()
        .expect("run hook post-commit");
    assert!(hooked.status.success(), "{}", stderr(&hooked));
    let done = phase(&plan);
    assert_eq!(done["status"], "done", "{done}");
    assert_eq!(done["commit"], sha.as_str(), "{done}");

    // Close the loop: push the plan update back to the remote.
    plan.git(&clone, &["push", "--quiet", "origin", "HEAD"]);
    let pushed = plan.git(
        tmp,
        &["--git-dir", &origin.to_string_lossy(), "rev-parse", "HEAD"],
    );
    assert_eq!(pushed, plan.git(&clone, &["rev-parse", "HEAD"]));
}
