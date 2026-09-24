//! The one-worktree-per-roadmap review scoping every review host depends on:
//! two roadmaps, each implemented in place in its own `rdm worktree add`
//! worktree and finalized to `needs-review` there. Each worktree's
//! `rdm review pending` lists only its own roadmap, with the stamped branch
//! `roadmap/<slug>`; `main` sees nothing, and still exits cleanly once a
//! worktree and its branch are gone; and after an amend, `rdm review
//! restamp` re-points the stamp at the new HEAD, is a no-op when repeated,
//! and keeps the item in scope.
//!
//! The retired shell's case A replayed the Pi `agent_end` review-on-finalize
//! extension's inject rule, which unify-code-review phases 6–7 retired; the
//! live scoping it rested on is asserted here directly.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::{Plan, field, items, stderr, stdout};

const PROJECT: &str = "verify";

/// `rdm review <sub> --format json` from `cwd`, with the plan and project
/// resolved through `RDM_ROOT`/`RDM_PROJECT` the way a host trigger runs it.
fn review_json(plan: &Plan, cwd: &Path, sub: &str) -> Value {
    let out = plan
        .sandbox
        .rdm()
        .args(["review", sub, "--format", "json"])
        .env("RDM_ROOT", &plan.root)
        .env("RDM_PROJECT", PROJECT)
        .env("RDM_SESSION", "cli-loop")
        .current_dir(cwd)
        .output()
        .expect("spawn rdm");
    assert!(
        out.status.success(),
        "`rdm review {sub}` from {} exited {:?}: {}",
        cwd.display(),
        out.status.code(),
        stderr(&out)
    );
    serde_json::from_str(&stdout(&out))
        .unwrap_or_else(|e| panic!("`rdm review {sub}` json: {e}: {}", stdout(&out)))
}

fn identifiers(v: &Value) -> Vec<String> {
    field(v, "identifier")
}

fn branch_of(v: &Value, identifier: &str) -> Value {
    items(v)
        .into_iter()
        .find(|i| i["identifier"] == identifier)
        .map(|i| i["branch"].clone())
        .unwrap_or(Value::Null)
}

#[test]
fn roadmap_worktrees_scope_pending_and_restamp() {
    let plan = Plan::empty();
    plan.ok(&["init", "--default-project", PROJECT]);
    for rm in ["alpha", "beta"] {
        plan.ok(&[
            "roadmap",
            "create",
            rm,
            "--title",
            &format!("Roadmap {rm}"),
            "--body",
            &format!("Isolation regression roadmap {rm}."),
            "--no-edit",
            "--project",
            PROJECT,
        ]);
        plan.ok(&[
            "phase",
            "create",
            "work",
            "--title",
            "Work",
            "--number",
            "1",
            "--body",
            &format!("## Purpose\n\nImplement-in-place phase for roadmap {rm}."),
            "--no-edit",
            "--roadmap",
            rm,
            "--project",
            PROJECT,
        ]);
    }

    let src = plan.dir.path().join("src");
    std::fs::create_dir_all(&src).expect("create source repo");
    plan.git(&src, &["init", "--quiet", "-b", "main"]);
    plan.git(
        &src,
        &["commit", "--quiet", "--allow-empty", "-m", "chore: initial"],
    );

    let add = |rm: &str| -> PathBuf {
        PathBuf::from(
            plan.ok_in(&src, &["worktree", "add", rm, "--project", PROJECT])
                .trim(),
        )
    };
    let alpha = add("alpha");
    let beta = add("beta");
    for (wt, rm) in [(&alpha, "alpha"), (&beta, "beta")] {
        assert!(wt.is_dir(), "{rm} worktree created at {}", wt.display());
        assert!(
            wt.starts_with(plan.dir.path().canonicalize().unwrap_or_default())
                || wt.starts_with(plan.dir.path()),
            "the worktree stays inside the test's temp dir: {}",
            wt.display()
        );
        assert_eq!(
            plan.git(wt, &["rev-parse", "--abbrev-ref", "HEAD"]),
            format!("roadmap/{rm}")
        );
        // Implement in place, then finalize from the worktree so the stamp
        // records its branch and HEAD.
        plan.git(
            wt,
            &[
                "commit",
                "--quiet",
                "--allow-empty",
                "-m",
                &format!("feat: {rm} phase 1 work"),
            ],
        );
        plan.ok_in(
            wt,
            &[
                "phase",
                "update",
                "phase-1-work",
                "--status",
                "needs-review",
                "--no-edit",
                "--roadmap",
                rm,
                "--project",
                PROJECT,
            ],
        );
    }

    // Isolation: each worktree sees only its own roadmap, on its own branch.
    let pa = review_json(&plan, &alpha, "pending");
    assert_eq!(
        identifiers(&pa),
        vec!["alpha/phase-1-work".to_owned()],
        "{pa}"
    );
    assert_eq!(branch_of(&pa, "alpha/phase-1-work"), "roadmap/alpha");
    let pb = review_json(&plan, &beta, "pending");
    assert_eq!(
        identifiers(&pb),
        vec!["beta/phase-1-work".to_owned()],
        "{pb}"
    );
    assert_eq!(branch_of(&pb, "beta/phase-1-work"), "roadmap/beta");

    // From main nothing is in scope, before and after alpha's worktree and
    // branch are removed.
    assert!(identifiers(&review_json(&plan, &src, "pending")).is_empty());
    plan.git(&src, &["worktree", "remove", &alpha.to_string_lossy()]);
    plan.git(&src, &["branch", "-D", "roadmap/alpha"]);
    assert!(!alpha.exists());
    assert!(identifiers(&review_json(&plan, &src, "pending")).is_empty());

    // Amend after finalize: restamp follows the new HEAD, once.
    let before = plan.git(&beta, &["rev-parse", "HEAD"]);
    plan.git(
        &beta,
        &[
            "commit",
            "--quiet",
            "--amend",
            "--allow-empty",
            "-m",
            "feat: beta phase 1 work (amended)",
        ],
    );
    let after = plan.git(&beta, &["rev-parse", "HEAD"]);
    assert_ne!(before, after, "the amend moved HEAD");
    let restamped = review_json(&plan, &beta, "restamp");
    let refreshed = items(&restamped)
        .into_iter()
        .find(|i| i["identifier"] == "beta/phase-1-work")
        .unwrap_or_else(|| panic!("restamp refreshes beta/phase-1-work: {restamped}"));
    assert_eq!(refreshed["sha"], after.as_str(), "{restamped}");
    let again = review_json(&plan, &beta, "restamp");
    assert!(
        identifiers(&again).is_empty(),
        "a second restamp is a no-op: {again}"
    );
    assert_eq!(
        identifiers(&review_json(&plan, &beta, "pending")),
        vec!["beta/phase-1-work".to_owned()]
    );
}
