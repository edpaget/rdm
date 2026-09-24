//! Session-scoped `rdm commit`, `status` and `discard` across real processes:
//! two sessions' commits carry exactly their own paths (with explicit ids and
//! from two bare shells), a rung-2 changeset carries across processes, the
//! `Done:` hook commits only its own changeset, a legacy repo gains no
//! rdm-authored dirt, a scoped discard spares another session's work, and
//! reads stay shared. Ported from the retired `scripts/verify-scoped-commit.sh`;
//! the case map — including the sections owned by `cli_commit.rs`,
//! `cli_status.rs` and `cli_init.rs` — is `docs/test-migration-inventory.md`
//! § 9.

use std::collections::BTreeSet;

use crate::support::{Plan, ShellDriver, must, real_bin};

fn create_task(slug: &str) -> Vec<&str> {
    vec![
        "task",
        "create",
        slug,
        "--title",
        slug,
        "--body",
        "Body.",
        "--no-edit",
        "--project",
        "demo",
    ]
}

fn set(paths: &[&str]) -> BTreeSet<String> {
    paths.iter().map(|p| (*p).to_owned()).collect()
}

/// Sections A and F: two explicit sessions each create a task; A commits
/// while B's work is still on disk, then B commits. Exactly two commits, each
/// holding exactly its own path; B's file is byte-identical after A's commit;
/// nothing is left dirty.
#[test]
fn explicit_sessions_land_disjoint_exact_commits() {
    let plan = Plan::demo(&real_bin());
    let base = plan.head();
    plan.ok(Some("sess-alpha"), &create_task("alpha-task"));
    plan.ok(Some("sess-beta"), &create_task("beta-task"));
    let beta_file = plan.path("projects/demo/tasks/beta-task.md");
    let beta_before = std::fs::read(&beta_file).unwrap();

    plan.ok(Some("sess-alpha"), &["commit", "-m", "add alpha-task"]);
    assert_eq!(
        plan.commit_files("HEAD"),
        set(&["projects/demo/tasks/alpha-task.md"])
    );
    assert_eq!(
        std::fs::read(&beta_file).unwrap(),
        beta_before,
        "A's commit touched B's uncommitted file"
    );

    plan.ok(Some("sess-beta"), &["commit", "-m", "add beta-task"]);
    assert_eq!(
        plan.commit_files("HEAD"),
        set(&["projects/demo/tasks/beta-task.md"])
    );
    let count = plan.git(&["rev-list", "--count", &format!("{base}..HEAD")]);
    assert_eq!(count.trim(), "2");
    assert_eq!(plan.porcelain(), "");
}

/// Section B: the same from two bare shells (rung 2), each gated between its
/// create and its commit so the first commit runs while the other shell's
/// work is on disk. Their ids differ, so the disjointness is not vacuous.
#[test]
fn bare_shells_land_disjoint_exact_commits() {
    let plan = Plan::demo(&real_bin());
    let base = plan.head();
    let spawn = |who: &str| {
        let slug = format!("{who}-task");
        let message = format!("add {slug}");
        must(
            ShellDriver::new()
                .direct(&["session", "id", "--format", "json"])
                .direct(&create_task(&slug))
                .gate()
                .direct(&["commit", "-m", &message])
                .spawn(&plan, who),
        )
    };
    let (mut gamma, mut delta) = (spawn("gamma"), spawn("delta"));
    must(gamma.wait_step(1));
    must(delta.wait_step(1));
    must(gamma.release_gate());
    let gamma = must(gamma.finish());
    assert_eq!(
        plan.commit_files("HEAD"),
        set(&["projects/demo/tasks/gamma-task.md"]),
        "gamma's commit is not exactly its own task: {gamma:?}"
    );
    must(delta.release_gate());
    let delta = must(delta.finish());
    assert_eq!(
        plan.commit_files("HEAD"),
        set(&["projects/demo/tasks/delta-task.md"]),
        "delta's commit is not exactly its own task: {delta:?}"
    );
    let (g, d) = (
        must(gamma.steps[0].session()),
        must(delta.steps[0].session()),
    );
    assert_ne!(
        g.id, d.id,
        "both shells resolved one id, so this test is vacuous"
    );
    let count = plan.git(&["rev-list", "--count", &format!("{base}..HEAD")]);
    assert_eq!(count.trim(), "2");
}

/// Section B2: one bare shell's create and commit, two processes, share one
/// changeset: the commit lands the file and never says "Nothing".
#[test]
fn a_bare_shells_changeset_carries_across_processes() {
    let plan = Plan::demo(&real_bin());
    let run = must(
        ShellDriver::new()
            .direct(&create_task("carried"))
            .direct(&["commit", "-m", "add carried"])
            .run(&plan, "b2"),
    );
    assert!(run.steps.iter().all(|s| s.status == 0), "{run:?}");
    assert!(!run.steps[1].all().contains("Nothing"), "{run:?}");
    assert!(
        plan.commit_files("HEAD")
            .contains("projects/demo/tasks/carried.md")
    );
}

/// Section C: the `Done:` hook commits only its own changeset — another
/// session's uncommitted file stays out of the hook commit, on disk, and in
/// its own journal.
#[test]
fn done_hook_commits_only_its_own_changeset() {
    let plan = Plan::demo(&real_bin());
    plan.ok(Some("sess-hook-a"), &create_task("hook-task"));
    plan.ok(Some("sess-hook-a"), &["commit", "-m", "add hook-task"]);
    plan.ok(Some("sess-hook-b"), &create_task("bystander"));
    let bystander = "projects/demo/tasks/bystander.md";
    plan.git_commit_empty("chore: land hook-task\n\nDone: task/hook-task");
    let before = plan.head();
    let mut hook = plan.cmd(Some("sess-hook-a"), &["hook", "post-commit"]);
    hook.current_dir(&plan.root);
    let out = plan.output(hook);
    assert!(out.success(), "{}", out.describe());
    assert_ne!(plan.head(), before, "the hook made no commit");
    assert_eq!(plan.task("demo", "hook-task")["status"], "done");
    assert!(
        !plan.commit_files("HEAD").contains(bystander),
        "the hook swept another session's file"
    );
    assert!(plan.path(bystander).exists(), "the hook deleted B's file");
    assert!(
        plan.journal("sess-hook-b").contains_key(bystander),
        "B's journal lost its file"
    );
}

/// Section E2: a legacy repo whose HEAD tracks the retired `merge=rdm-index`
/// attributes commits exactly the authored path, leaves the attributes
/// byte-identical, and shows a fresh session nothing to commit.
#[test]
fn legacy_repo_commit_carries_only_the_authored_path() {
    let plan = Plan::demo(&real_bin());
    let attrs = plan.path(".gitattributes");
    std::fs::write(
        &attrs,
        "INDEX.md merge=rdm-index\n**/INDEX.md merge=rdm-index\n",
    )
    .unwrap();
    plan.git(&["add", ".gitattributes"]);
    plan.git(&[
        "commit",
        "--quiet",
        "-m",
        "chore: legacy merge-driver attributes",
    ]);
    assert!(plan.tree("HEAD").contains(".gitattributes"));
    let before = std::fs::read(&attrs).unwrap();

    plan.ok(Some("sess-e2"), &create_task("e2-task"));
    plan.ok(Some("sess-e2"), &["commit", "-m", "add e2-task"]);
    assert_eq!(
        plan.commit_files("HEAD"),
        set(&["projects/demo/tasks/e2-task.md"])
    );
    assert_eq!(
        std::fs::read(&attrs).unwrap(),
        before,
        "rdm rewrote .gitattributes"
    );
    let status = plan.ok(Some("sess-e2b"), &["status"]);
    assert!(status.contains("No uncommitted changes."), "{status}");
}

/// Section G: a scoped discard removes only its own file; the other
/// session's file stays byte-identical and journaled, and its later commit is
/// exactly its own path.
#[test]
fn scoped_discard_spares_another_sessions_work() {
    let plan = Plan::demo(&real_bin());
    let roadmap = |session, slug: &str| {
        plan.ok(
            Some(session),
            &[
                "roadmap",
                "create",
                slug,
                "--title",
                slug,
                "--body",
                "Body.",
                "--no-edit",
                "--project",
                "demo",
            ],
        );
        format!("projects/demo/roadmaps/{slug}/roadmap.md")
    };
    let a = roadmap("sess-g-a", "gone-map");
    let b = roadmap("sess-g-b", "kept-map");
    let b_before = std::fs::read(plan.path(&b)).unwrap();
    plan.ok(Some("sess-g-a"), &["discard", "--force"]);
    assert!(!plan.path(&a).exists(), "A's own file survived its discard");
    assert_eq!(
        std::fs::read(plan.path(&b)).unwrap(),
        b_before,
        "A's discard changed B's file"
    );
    assert!(
        plan.journal("sess-g-b").contains_key(&b),
        "B's journal lost its file"
    );
    plan.ok(Some("sess-g-b"), &["commit", "-m", "add kept-map"]);
    assert_eq!(plan.commit_files("HEAD"), set(&[b.as_str()]));
}

/// Section H: reads are not isolated — one session sees another's
/// uncommitted task through `task show` and `search`.
#[test]
fn reads_cross_the_session_boundary() {
    let plan = Plan::demo(&real_bin());
    plan.ok(
        Some("sess-h-b"),
        &[
            "task",
            "create",
            "shared-read",
            "--title",
            "Shared Read",
            "--body",
            "Body.",
            "--no-edit",
            "--project",
            "demo",
        ],
    );
    let shown = plan.ok(
        Some("sess-h-a"),
        &[
            "task",
            "show",
            "shared-read",
            "--project",
            "demo",
            "--format",
            "json",
        ],
    );
    assert!(shown.contains("Shared Read"), "{shown}");
    let found = plan.ok(
        Some("sess-h-a"),
        &[
            "search",
            "Shared Read",
            "--project",
            "demo",
            "--format",
            "json",
        ],
    );
    assert!(found.contains("\"shared-read\""), "{found}");
}
