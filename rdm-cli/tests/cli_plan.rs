//! Integration tests for `rdm plan` — the implementation-plan porcelain —
//! plus the two cross-cutting surfaces it feeds: plan backlinks on
//! `phase show`/`task show`, and review anchoring into a plan body.

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use std::fs;
use tempfile::TempDir;

fn rdm() -> Command {
    let mut cmd = Command::cargo_bin("rdm").unwrap();
    // Isolate from host global config (e.g. default_format = "json").
    cmd.env("XDG_CONFIG_HOME", "/dev/null/nonexistent");
    cmd
}

/// A plan repo with a `fbm` project, one task (`fix-login`), and one roadmap
/// (`auth`) with a single phase (`phase-1-design`).
fn init_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("init")
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "fbm"])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "create",
            "fix-login",
            "--title",
            "Fix login",
            "--body",
            "The login flow is broken.\n",
            "--no-edit",
            "--project",
            "fbm",
        ])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "roadmap",
            "create",
            "auth",
            "--title",
            "Auth",
            "--no-edit",
            "--project",
            "fbm",
        ])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            "design",
            "--title",
            "Design",
            "--number",
            "1",
            "--no-edit",
            "--roadmap",
            "auth",
            "--project",
            "fbm",
        ])
        .assert()
        .success();
    dir
}

fn create_plan(dir: &TempDir, slug: &str, implements: &str) {
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "plan",
            "create",
            slug,
            "--title",
            slug,
            "--implements",
            implements,
            "--body",
            "## Approach\n\nDetails here.\n",
            "--no-edit",
            "--project",
            "fbm",
        ])
        .assert()
        .success();
}

fn json_out(dir: &TempDir, args: &[&str]) -> Value {
    let out = rdm()
        .arg("--root")
        .arg(dir.path())
        .args(args)
        .args(["--format", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).unwrap()
}

// --- AC1/AC6: the create/show/list/update/delete round trip ---

#[test]
fn plan_create_show_list_update_delete_round_trip() {
    let dir = init_repo();
    create_plan(&dir, "impl-login", "task/fix-login");

    // The file lands where `docs/file-formats.md` says it does.
    let path = dir.path().join("projects/fbm/plans/impl-login.md");
    let content = fs::read_to_string(&path).unwrap();
    assert!(content.contains("plan: impl-login"), "{content}");
    assert!(
        content.contains("implements: rdm:task/fix-login"),
        "the canonical rdm: form must be written: {content}"
    );
    assert!(content.contains("status: draft"), "{content}");

    // show (human)
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["plan", "show", "impl-login", "--project", "fbm"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Slug: impl-login")
                .and(predicate::str::contains("Status: draft"))
                .and(predicate::str::contains("Implements: rdm:task/fix-login"))
                .and(predicate::str::contains("Details here.")),
        );

    // show (json)
    let j = json_out(&dir, &["plan", "show", "impl-login", "--project", "fbm"]);
    assert_eq!(j["slug"], "impl-login");
    assert_eq!(j["project"], "fbm");
    assert_eq!(j["status"], "draft");
    assert_eq!(j["implements"], "rdm:task/fix-login");
    assert!(j["supersedes"].is_null(), "absent when unset: {j}");
    assert!(j["reviews"].is_null(), "absent when there are none: {j}");

    // list (json)
    let list = json_out(&dir, &["plan", "list", "--project", "fbm"]);
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["slug"], "impl-login");

    // update
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "plan",
            "update",
            "impl-login",
            "--title",
            "Renamed plan",
            "--body",
            "## Revised\n\nNew approach.\n",
            "--no-edit",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Updated plan 'impl-login'"));
    let j = json_out(&dir, &["plan", "show", "impl-login", "--project", "fbm"]);
    assert_eq!(j["title"], "Renamed plan");
    assert_eq!(j["body"], "## Revised\n\nNew approach.\n");
    // The slug is never renamed by a title change.
    assert_eq!(j["slug"], "impl-login");

    // delete requires --force
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["plan", "delete", "impl-login", "--project", "fbm"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--force"));
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "plan",
            "delete",
            "impl-login",
            "--force",
            "--project",
            "fbm",
        ])
        .assert()
        .success();
    assert!(!path.exists());
}

#[test]
fn plan_create_accepts_a_numeric_phase_implements() {
    let dir = init_repo();
    create_plan(&dir, "impl-design", "phase/auth/1");
    let j = json_out(&dir, &["plan", "show", "impl-design", "--project", "fbm"]);
    assert_eq!(
        j["implements"], "rdm:phase/auth/phase-1-design",
        "the numeric stem must be normalized at create time: {j}"
    );
}

#[test]
fn plan_create_rejects_a_missing_implements_target() {
    let dir = init_repo();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "plan",
            "create",
            "impl-ghost",
            "--implements",
            "task/no-such-task",
            "--no-edit",
            "--project",
            "fbm",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("plan target not found"));
    assert!(
        !dir.path().join("projects/fbm/plans/impl-ghost.md").exists(),
        "a rejected create must leave nothing behind"
    );
}

#[test]
fn plan_create_rejects_a_roadmap_implements_kind() {
    let dir = init_repo();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "plan",
            "create",
            "impl-auth",
            "--implements",
            "roadmap/auth",
            "--no-edit",
            "--project",
            "fbm",
        ])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("cannot be implemented by a plan")
                .and(predicate::str::contains("--implements phase/")),
        );
}

#[test]
fn plan_create_rejects_a_malformed_implements_reference() {
    let dir = init_repo();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "plan",
            "create",
            "impl-bad",
            "--implements",
            "not-a-reference",
            "--no-edit",
            "--project",
            "fbm",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not a valid item reference"));
}

#[test]
fn plan_create_rejects_a_non_plan_supersedes_kind() {
    let dir = init_repo();
    create_plan(&dir, "impl-auth", "task/fix-login");
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "plan",
            "create",
            "impl-auth-v2",
            "--implements",
            "task/fix-login",
            "--supersedes",
            "task/fix-login",
            "--no-edit",
            "--project",
            "fbm",
        ])
        .assert()
        .failure()
        // The message must name the flag that was actually wrong. Here
        // `--implements` was valid and only `--supersedes` was not, so
        // pointing the reader at `--implements` would misdirect them.
        .stderr(
            predicate::str::contains("is not a plan and cannot be superseded")
                .and(predicate::str::contains("--supersedes plan/<slug>"))
                .and(predicate::str::contains("--implements").not()),
        );
}

#[test]
fn plan_create_supersedes_flips_the_predecessor() {
    let dir = init_repo();
    create_plan(&dir, "impl-v1", "task/fix-login");
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "plan",
            "create",
            "impl-v2",
            "--implements",
            "task/fix-login",
            "--supersedes",
            "plan/impl-v1",
            "--no-edit",
            "--project",
            "fbm",
        ])
        .assert()
        .success();
    let v1 = json_out(&dir, &["plan", "show", "impl-v1", "--project", "fbm"]);
    let v2 = json_out(&dir, &["plan", "show", "impl-v2", "--project", "fbm"]);
    assert_eq!(v1["status"], "superseded");
    assert_eq!(v2["status"], "draft");
    assert_eq!(v2["supersedes"], "rdm:plan/impl-v1");
}

fn stdout_of(dir: &TempDir, args: &[&str]) -> String {
    let out = rdm()
        .arg("--root")
        .arg(dir.path())
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(out).unwrap()
}

#[test]
fn plan_show_and_list_render_markdown() {
    let dir = init_repo();
    create_plan(&dir, "impl-auth", "task/fix-login");

    let show = stdout_of(
        &dir,
        &[
            "plan",
            "show",
            "impl-auth",
            "--project",
            "fbm",
            "--format",
            "markdown",
        ],
    );
    assert!(show.starts_with("# impl-auth"), "{show}");
    assert!(show.contains("- **Slug:** impl-auth"), "{show}");
    assert!(show.contains("- **Status:** draft"), "{show}");
    assert!(
        show.contains("- **Implements:** rdm:task/fix-login"),
        "{show}"
    );
    assert!(show.contains("## Approach"), "{show}");

    let list = stdout_of(
        &dir,
        &["plan", "list", "--project", "fbm", "--format", "markdown"],
    );
    assert!(list.contains("## Plans"), "{list}");
    assert!(
        list.contains("| Slug | Title | Status | Implements |"),
        "{list}"
    );
    assert!(list.contains("rdm:task/fix-login"), "{list}");
}

#[test]
fn phase_and_task_show_render_a_markdown_plans_bullet() {
    let dir = init_repo();
    create_plan(&dir, "impl-auth", "task/fix-login");
    create_plan(&dir, "impl-design", "phase/auth/phase-1-design");

    let task_md = stdout_of(
        &dir,
        &[
            "task",
            "show",
            "fix-login",
            "--project",
            "fbm",
            "--format",
            "markdown",
        ],
    );
    assert!(
        task_md.contains("- **Plans:** impl-auth (draft)"),
        "{task_md}"
    );

    let phase_md = stdout_of(
        &dir,
        &[
            "phase",
            "show",
            "phase-1-design",
            "--roadmap",
            "auth",
            "--project",
            "fbm",
            "--format",
            "markdown",
        ],
    );
    assert!(
        phase_md.contains("- **Plans:** impl-design (draft)"),
        "{phase_md}"
    );

    // The bullet is absent when nothing implements the item.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            "build",
            "--title",
            "Build",
            "--number",
            "2",
            "--no-edit",
            "--roadmap",
            "auth",
            "--project",
            "fbm",
        ])
        .assert()
        .success();
    let other_md = stdout_of(
        &dir,
        &[
            "phase",
            "show",
            "phase-2-build",
            "--roadmap",
            "auth",
            "--project",
            "fbm",
            "--format",
            "markdown",
        ],
    );
    assert!(!other_md.contains("**Plans:**"), "{other_md}");
}

#[test]
fn plan_list_filters_by_implements_and_status() {
    let dir = init_repo();
    create_plan(&dir, "impl-login", "task/fix-login");
    create_plan(&dir, "impl-design", "phase/auth/phase-1-design");

    // `--implements` narrows, accepting either phase form.
    for form in ["phase/auth/1", "phase/auth/phase-1-design"] {
        let list = json_out(
            &dir,
            &["plan", "list", "--implements", form, "--project", "fbm"],
        );
        let slugs: Vec<&str> = list
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["slug"].as_str().unwrap())
            .collect();
        assert_eq!(slugs, ["impl-design"], "form {form}");
    }

    // `--status` narrows independently.
    let list = json_out(
        &dir,
        &["plan", "list", "--status", "approved", "--project", "fbm"],
    );
    assert!(list.as_array().unwrap().is_empty(), "{list}");
}

// --- the body contract, matching `task` exactly ---

#[test]
fn plan_create_body_flag_beats_stdin() {
    let dir = init_repo();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "plan",
            "create",
            "impl-login",
            "--implements",
            "task/fix-login",
            "--body",
            "AUTHORITATIVE BODY",
            "--no-edit",
            "--project",
            "fbm",
        ])
        .write_stdin("SNEAKY STDIN BODY")
        .assert()
        .success();
    let content = fs::read_to_string(dir.path().join("projects/fbm/plans/impl-login.md")).unwrap();
    assert!(content.contains("AUTHORITATIVE BODY"), "{content}");
    assert!(!content.contains("SNEAKY STDIN BODY"), "{content}");
}

#[test]
fn plan_create_reads_a_piped_body_from_stdin() {
    let dir = init_repo();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "plan",
            "create",
            "impl-login",
            "--implements",
            "task/fix-login",
            "--no-edit",
            "--project",
            "fbm",
        ])
        .write_stdin("## Piped\n\nMultiline body.\n")
        .assert()
        .success();
    let content = fs::read_to_string(dir.path().join("projects/fbm/plans/impl-login.md")).unwrap();
    assert!(content.contains("Multiline body."), "{content}");
}

#[test]
fn plan_update_never_reads_stdin() {
    let dir = init_repo();
    create_plan(&dir, "impl-login", "task/fix-login");
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "plan",
            "update",
            "impl-login",
            "--title",
            "Retitled",
            "--no-edit",
            "--project",
            "fbm",
        ])
        .write_stdin("SNEAKY STDIN BODY")
        .assert()
        .success();
    let content = fs::read_to_string(dir.path().join("projects/fbm/plans/impl-login.md")).unwrap();
    assert!(content.contains("Details here."), "{content}");
    assert!(!content.contains("SNEAKY STDIN BODY"), "{content}");
}

#[test]
fn plan_update_empty_body_refuses_clobber_and_clear_body_succeeds() {
    let dir = init_repo();
    create_plan(&dir, "impl-login", "task/fix-login");
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "plan",
            "update",
            "impl-login",
            "--body",
            "",
            "--no-edit",
            "--project",
            "fbm",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--clear-body"));

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "plan",
            "update",
            "impl-login",
            "--clear-body",
            "--no-edit",
            "--project",
            "fbm",
        ])
        .assert()
        .success();
    let j = json_out(&dir, &["plan", "show", "impl-login", "--project", "fbm"]);
    assert_eq!(j["body"], "");
}

// --- AC3: plan backlinks on phase show / task show, reviews on plan show ---

#[test]
fn phase_show_json_lists_implementing_plans() {
    let dir = init_repo();
    // One plan stored via the numeric form, one via the canonical stem.
    create_plan(&dir, "impl-a", "phase/auth/1");
    create_plan(&dir, "impl-b", "phase/auth/phase-1-design");

    let j = json_out(
        &dir,
        &[
            "phase",
            "show",
            "1",
            "--roadmap",
            "auth",
            "--project",
            "fbm",
        ],
    );
    let plans = j["plans"].as_array().unwrap();
    let slugs: Vec<&str> = plans.iter().map(|p| p["slug"].as_str().unwrap()).collect();
    assert_eq!(slugs, ["impl-a", "impl-b"]);
    assert_eq!(plans[0]["title"], "impl-a");
    assert_eq!(plans[0]["status"], "draft");

    // ...and the human view names them too.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "1",
            "--roadmap",
            "auth",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Plans: impl-a (draft), impl-b (draft)",
        ));
}

#[test]
fn phase_show_json_omits_plans_when_none_implement_it() {
    let dir = init_repo();
    // A plan exists, but on the *task* — the phase must not claim it.
    create_plan(&dir, "impl-login", "task/fix-login");
    let j = json_out(
        &dir,
        &[
            "phase",
            "show",
            "1",
            "--roadmap",
            "auth",
            "--project",
            "fbm",
        ],
    );
    assert!(
        j["plans"].is_null(),
        "the skip-if-empty contract keeps existing goldens stable: {j}"
    );
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "1",
            "--roadmap",
            "auth",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Plans:").not());
}

#[test]
fn task_show_json_lists_implementing_plans() {
    let dir = init_repo();
    create_plan(&dir, "impl-login", "task/fix-login");
    let j = json_out(&dir, &["task", "show", "fix-login", "--project", "fbm"]);
    let plans = j["plans"].as_array().unwrap();
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0]["slug"], "impl-login");

    // ...and a task with nothing implementing it omits the key entirely.
    let dir2 = init_repo();
    let j2 = json_out(&dir2, &["task", "show", "fix-login", "--project", "fbm"]);
    assert!(j2["plans"].is_null(), "{j2}");
}

#[test]
fn plan_show_json_lists_its_reviews() {
    let dir = init_repo();
    create_plan(&dir, "impl-login", "task/fix-login");
    let id = start_review(&dir, "plan/impl-login");
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "review",
            "comment",
            &id,
            "--body",
            "Whole-document feedback.",
            "--no-edit",
            "--project",
            "fbm",
        ])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "review",
            "submit",
            &id,
            "--verdict",
            "approve",
            "--no-edit",
            "--project",
            "fbm",
        ])
        .assert()
        .success();

    let j = json_out(&dir, &["plan", "show", "impl-login", "--project", "fbm"]);
    let reviews = j["reviews"].as_array().unwrap();
    assert_eq!(reviews.len(), 1);
    assert_eq!(reviews[0]["id"], id.as_str());
    assert_eq!(reviews[0]["author"], "tester");
    assert_eq!(reviews[0]["state"], "submitted");
    assert_eq!(reviews[0]["verdict"], "approve");
    // ...and the verdict derived the plan's status, end to end.
    assert_eq!(j["status"], "approved");
}

#[test]
fn review_submit_request_changes_flips_the_plan() {
    let dir = init_repo();
    create_plan(&dir, "impl-login", "task/fix-login");
    let id = start_review(&dir, "plan/impl-login");
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "review",
            "comment",
            &id,
            "--body",
            "Needs work.",
            "--no-edit",
            "--project",
            "fbm",
        ])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "review",
            "submit",
            &id,
            "--verdict",
            "request-changes",
            "--no-edit",
            "--project",
            "fbm",
        ])
        .assert()
        .success();
    let j = json_out(&dir, &["plan", "show", "impl-login", "--project", "fbm"]);
    assert_eq!(j["status"], "changes-requested");
}

#[test]
fn review_start_on_a_missing_plan_reports_a_missing_target() {
    let dir = init_repo();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "review",
            "start",
            "--on",
            "plan/no-such-plan",
            "--author",
            "tester",
            "--no-edit",
            "--project",
            "fbm",
        ])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("review target not found")
                .and(predicate::str::contains("plan 'no-such-plan'")),
        );
}

// --- AC4: quote anchoring into a plan body ---

fn start_review(dir: &TempDir, on: &str) -> String {
    let out = rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "review",
            "start",
            "--on",
            on,
            "--author",
            "tester",
            "--no-edit",
            "--project",
            "fbm",
            "--format",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&out).unwrap();
    json["id"].as_str().unwrap().to_string()
}

fn show_review_json(dir: &TempDir, id: &str) -> Value {
    json_out(dir, &["review", "show", id, "--project", "fbm"])
}

#[test]
fn review_comment_quote_anchors_into_plan_body() {
    let dir = init_repo();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "plan",
            "create",
            "impl-login",
            "--title",
            "Login plan",
            "--implements",
            "task/fix-login",
            "--body",
            "Intro paragraph. The auth flow needs work here. Outro.\n",
            "--no-edit",
            "--project",
            "fbm",
        ])
        .assert()
        .success();
    // Land a real commit so the review's created_commit pins to a revision
    // where the plan actually exists in history.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["commit", "-m", "seed: add plan"])
        .assert()
        .success();

    let id = start_review(&dir, "plan/impl-login");
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "review",
            "comment",
            &id,
            "--quote",
            "auth flow",
            "--body",
            "Which auth flow?",
            "--no-edit",
            "--project",
            "fbm",
        ])
        .assert()
        .success();

    let j = show_review_json(&dir, &id);
    let c = &j["comments"][0];
    assert_eq!(c["anchor"]["anchor_type"], "text-quote");
    assert_eq!(c["anchor"]["quote"], "auth flow");
    assert_eq!(c["resolution"]["state"], "resolved");

    // Edit the plan body and land it: the same anchor now reports drift.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "plan",
            "update",
            "impl-login",
            "--body",
            "Intro paragraph. The reworded flow needs work here. Outro.\n",
            "--no-edit",
            "--project",
            "fbm",
        ])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["commit", "-m", "edit: revise plan"])
        .assert()
        .success();

    let j = show_review_json(&dir, &id);
    assert_eq!(j["comments"][0]["resolution"]["state"], "drifted");
}

#[test]
fn review_comment_ambiguous_quote_lists_occurrences_for_a_plan() {
    let dir = init_repo();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "plan",
            "create",
            "impl-login",
            "--implements",
            "task/fix-login",
            "--body",
            "the token check happens twice: the token check again.\n",
            "--no-edit",
            "--project",
            "fbm",
        ])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["commit", "-m", "seed: add plan"])
        .assert()
        .success();

    let id = start_review(&dir, "plan/impl-login");
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "review",
            "comment",
            &id,
            "--quote",
            "the token check",
            "--body",
            "Which one?",
            "--no-edit",
            "--project",
            "fbm",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--occurrence"));

    // ...and the 1-based selector resolves it, exactly as for a task.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "review",
            "comment",
            &id,
            "--quote",
            "the token check",
            "--occurrence",
            "2",
            "--body",
            "This one.",
            "--no-edit",
            "--project",
            "fbm",
        ])
        .assert()
        .success();
}

#[test]
fn review_comment_doc_rejected_on_a_plan_review() {
    let dir = init_repo();
    create_plan(&dir, "impl-login", "task/fix-login");
    let id = start_review(&dir, "plan/impl-login");
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "review",
            "comment",
            &id,
            "--doc",
            "phase/phase-1-design",
            "--body",
            "Scoped feedback.",
            "--no-edit",
            "--project",
            "fbm",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("roadmap review"));
}

// --- the `plan/<slug>` reference kind across the link surfaces ---

#[test]
fn plan_links_resolve_and_backlink() {
    let dir = init_repo();
    create_plan(&dir, "impl-login", "task/fix-login");
    // Reference the plan from the task body.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "update",
            "fix-login",
            "--body",
            "See [the plan](rdm:plan/impl-login).\n",
            "--no-edit",
            "--project",
            "fbm",
        ])
        .assert()
        .success();

    let resolved = json_out(
        &dir,
        &["link", "resolve", "rdm:plan/impl-login", "--project", "fbm"],
    );
    assert_eq!(resolved["kind"], "item");
    assert_eq!(resolved["exists"], true);

    let back = json_out(&dir, &["backlinks", "plan/impl-login", "--project", "fbm"]);
    let entries = back.as_array().unwrap();
    assert_eq!(entries.len(), 1, "{back}");
    // `BacklinkEntryJson` flattens the `DocRef`, so `kind`/`slug` are
    // top-level on each entry.
    assert_eq!(entries[0]["kind"], "task");
    assert_eq!(entries[0]["slug"], "fix-login");

    // `link check` sees the plan and passes.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["link", "check", "--project", "fbm"])
        .assert()
        .success();
}

#[test]
fn roadmap_create_rejects_the_reserved_plan_slug() {
    let dir = init_repo();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["roadmap", "create", "plan", "--no-edit", "--project", "fbm"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("reserved"));
}
