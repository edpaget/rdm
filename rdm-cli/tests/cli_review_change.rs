//! Integration tests for `change/<sha>` reviews — the cross-repo review
//! target whose comment anchors live in the project's *source* repository
//! rather than in the plan repo.
//!
//! Every test drives the real `rdm` binary against a temp plan repo plus a
//! separate temp git source checkout, following `cli_link.rs`'s fixture
//! pattern.

use assert_cmd::Command;
use serde_json::Value;
use std::path::Path;
use tempfile::TempDir;

fn rdm() -> Command {
    let mut cmd = Command::cargo_bin("rdm").unwrap();
    cmd.env("XDG_CONFIG_HOME", "/dev/null/nonexistent");
    cmd
}

fn git(dir: &Path, args: &[&str]) -> std::process::Output {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@test.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@test.com")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

fn git_out(dir: &Path, args: &[&str]) -> String {
    String::from_utf8_lossy(&git(dir, args).stdout)
        .trim()
        .to_string()
}

/// A plan repo with a `demo` project, a roadmap `auth` with one phase, and
/// the source checkout configured as the project's `source.repo`.
fn init_plan_repo(source: &Path) -> TempDir {
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
        .args(["project", "create", "demo"])
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
            "demo",
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
            "demo",
        ])
        .assert()
        .success();
    set_project_source(dir.path(), "demo", &source.to_string_lossy());
    dir
}

/// Hand-edits `project.md`'s frontmatter to add `source: { repo: … }` —
/// there is no CLI command to set it, so fixtures write the file directly,
/// exactly as `cli_link.rs` does.
///
/// Clears any existing block first, so calling this twice *replaces* the
/// configured source instead of writing a duplicate YAML key (which parses
/// as an error and would silently read back as "no source configured").
fn set_project_source(plan: &Path, project: &str, repo: &str) {
    clear_project_source(plan, project);
    let path = plan.join("projects").join(project).join("project.md");
    let content = std::fs::read_to_string(&path).unwrap();
    let rest = content.strip_prefix("---\n").expect("frontmatter open");
    let end = rest.find("\n---").expect("frontmatter close");
    let (frontmatter, tail) = rest.split_at(end);
    std::fs::write(
        &path,
        format!("---\n{frontmatter}\nsource:\n  repo: \"{repo}\"{tail}"),
    )
    .unwrap();
}

/// Removes the `source:` block [`set_project_source`] wrote, leaving the
/// project configuring no source repo at all.
fn clear_project_source(plan: &Path, project: &str) {
    let path = plan.join("projects").join(project).join("project.md");
    let content = std::fs::read_to_string(&path).unwrap();
    let kept: Vec<&str> = content
        .lines()
        .filter(|l| !l.starts_with("source:") && !l.starts_with("  repo:"))
        .collect();
    std::fs::write(&path, format!("{}\n", kept.join("\n"))).unwrap();
}

/// A git repository that is emphatically *not* the project's source: its own
/// `main`, its own single commit, and no `origin` remote.
fn init_unrelated_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    git(p, &["init", "-b", "main"]);
    std::fs::write(p.join("README.md"), "not the source repo\n").unwrap();
    git(p, &["add", "."]);
    git(p, &["commit", "-m", "unrelated"]);
    dir
}

const BASE_FILE: &str = "fn one() {}\nfn two() {}\nfn three() {}\n";
const HEAD_FILE: &str = "fn one() {}\nfn two_renamed() {}\nfn three() {}\n";

/// A source repo on `main` with one commit, then a `topic` branch whose tip
/// edits line 2 of `src/lib.rs`.
fn init_source_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    git(p, &["init", "-b", "main"]);
    std::fs::create_dir_all(p.join("src")).unwrap();
    std::fs::write(p.join("src/lib.rs"), BASE_FILE).unwrap();
    git(p, &["add", "."]);
    git(p, &["commit", "-m", "base"]);
    git(p, &["checkout", "-b", "topic"]);
    std::fs::write(p.join("src/lib.rs"), HEAD_FILE).unwrap();
    git(p, &["add", "."]);
    git(p, &["commit", "-m", "rename two"]);
    dir
}

fn start_change_review(plan: &Path, source: &Path, on: &str, extra: &[&str]) -> String {
    let mut cmd = rdm();
    cmd.arg("--root")
        .arg(plan)
        .args([
            "review",
            "start",
            "--on",
            on,
            "--no-edit",
            "--project",
            "demo",
        ])
        .args(extra)
        .current_dir(source);
    let out = cmd.assert().success().get_output().stdout.clone();
    let text = String::from_utf8_lossy(&out).to_string();
    text.split('\'')
        .nth(1)
        .unwrap_or_else(|| panic!("could not read a review id out of: {text}"))
        .to_string()
}

fn review_json(plan: &Path, source: &Path, id: &str) -> Value {
    let out = rdm()
        .arg("--root")
        .arg(plan)
        .args([
            "review",
            "show",
            id,
            "--format",
            "json",
            "--project",
            "demo",
        ])
        .current_dir(source)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).unwrap()
}

/// Creates a plan implementing `phase/auth/phase-1-design`, optionally
/// driving it to `approved` via an approving review.
fn create_plan(plan: &Path, slug: &str, approve: bool) {
    rdm()
        .arg("--root")
        .arg(plan)
        .args([
            "plan",
            "create",
            slug,
            "--title",
            "A plan",
            "--implements",
            "phase/auth/phase-1-design",
            "--body",
            "Plan body.\n",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .success();
    if approve {
        let out = rdm()
            .arg("--root")
            .arg(plan)
            .args([
                "review",
                "start",
                "--on",
                &format!("plan/{slug}"),
                "--no-edit",
                "--project",
                "demo",
            ])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let text = String::from_utf8_lossy(&out).to_string();
        let id = text.split('\'').nth(1).unwrap().to_string();
        rdm()
            .arg("--root")
            .arg(plan)
            .args([
                "review",
                "submit",
                &id,
                "--verdict",
                "approve",
                "--body",
                "Looks good.",
                "--no-edit",
                "--project",
                "demo",
            ])
            .assert()
            .success();
    }
}

// --- AC1: start records head and base; change/HEAD pins the tip ---

#[test]
fn change_review_start_records_head_and_base() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    create_plan(plan.path(), "design-plan", true);
    let head = git_out(src.path(), &["rev-parse", "HEAD"]);
    let merge_base = git_out(src.path(), &["merge-base", "main", "topic"]);

    let id = start_change_review(
        plan.path(),
        src.path(),
        &format!("change/{}", &head[..8]),
        &["--implements", "rdm:plan/design-plan"],
    );
    let j = review_json(plan.path(), src.path(), &id);
    assert_eq!(j["target"]["kind"], "change");
    assert_eq!(j["target"]["head"], head);
    assert_eq!(j["target"]["head"].as_str().unwrap().len(), 40);
    assert_eq!(j["target"]["base"], merge_base);
    assert_eq!(j["change_branch"], "topic");

    // `change/HEAD` from inside the checkout pins the same tip.
    let id2 = start_change_review(
        plan.path(),
        src.path(),
        "change/HEAD",
        &["--implements", "rdm:plan/design-plan"],
    );
    let j2 = review_json(plan.path(), src.path(), &id2);
    assert_eq!(j2["target"]["head"], head);
}

#[test]
fn change_review_start_rejects_an_unknown_rev() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    create_plan(plan.path(), "design-plan", true);
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "start",
            "--on",
            "change/deadbeefdeadbeef",
            "--implements",
            "rdm:plan/design-plan",
            "--no-edit",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .failure()
        .get_output()
        .stderr
        .clone();
    let text = String::from_utf8_lossy(&out);
    assert!(
        text.contains("does not name a commit"),
        "expected an actionable unknown-rev message: {text}"
    );
}

#[test]
fn change_review_start_names_base_when_there_is_no_merge_base() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    create_plan(plan.path(), "design-plan", true);
    // An orphan branch shares no history with `main`.
    git(src.path(), &["checkout", "--orphan", "island"]);
    std::fs::write(src.path().join("other.txt"), "hello\n").unwrap();
    git(src.path(), &["add", "other.txt"]);
    git(src.path(), &["commit", "-m", "orphan"]);

    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "start",
            "--on",
            "change/HEAD",
            "--implements",
            "rdm:plan/design-plan",
            "--no-edit",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .failure()
        .get_output()
        .stderr
        .clone();
    let text = String::from_utf8_lossy(&out);
    assert!(
        text.contains("no merge base") && text.contains("--base"),
        "a missing merge base must name --base: {text}"
    );

    // …and `--base` really is the escape hatch.
    let root = git_out(src.path(), &["rev-list", "--max-parents=0", "HEAD"]);
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "start",
            "--on",
            "change/HEAD",
            "--base",
            &root,
            "--implements",
            "rdm:plan/design-plan",
            "--no-edit",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .success();
}

// --- AC2: in-hunk anchor, out-of-hunk error, drift, unresolved ---

/// Every other test here runs with the cwd at the checkout's top level. That
/// hides an asymmetry between the two git reads a `--path` comment makes:
/// `git show <rev>:<path>` resolves from the repository root, but a bare
/// `git diff -- <path>` pathspec resolves from the CURRENT DIRECTORY. Since
/// `discover_source_repo` roots the source repo at the invoking cwd, running
/// this from a subdirectory used to read the file fine, compute an EMPTY hunk
/// set, and refuse a genuinely-modified file as untouched.
#[test]
fn change_comment_anchors_from_a_subdirectory_of_the_checkout() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    create_plan(plan.path(), "design-plan", true);

    // A sibling directory to invoke from. `init_source_repo` leaves HEAD on
    // `topic`, so this commit lands only there — it just needs to exist in
    // the working tree to `cd` into.
    let sub = src.path().join("other");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(sub.join("keep.txt"), "x\n").unwrap();
    git(src.path(), &["add", "."]);
    git(src.path(), &["commit", "-m", "sibling"]);

    // Start the review from the subdirectory too — `change/HEAD` must still
    // pin this checkout's tip.
    let id = start_change_review(
        plan.path(),
        &sub,
        "change/HEAD",
        &["--implements", "rdm:plan/design-plan"],
    );

    // The repo-relative path is the documented contract, from anywhere.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "comment",
            &id,
            "--path",
            "src/lib.rs",
            "--quote",
            "fn two_renamed() {}",
            "--body",
            "Anchored from a subdirectory.",
            "--no-edit",
            "--project",
            "demo",
        ])
        .current_dir(&sub)
        .assert()
        .success();

    let j = review_json(plan.path(), &sub, &id);
    assert_eq!(j["comments"][0]["resolution"]["state"], "resolved");
    assert_eq!(j["comments"][0]["anchor"]["anchor_type"], "file-quote");
    assert_eq!(j["comments"][0]["anchor"]["start_line"], 2);

    // The out-of-hunk refusal must still be reachable from here — the fix
    // must not turn every quote into a match.
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "comment",
            &id,
            "--path",
            "src/lib.rs",
            "--quote",
            "fn three() {}",
            "--body",
            "Untouched.",
            "--no-edit",
            "--project",
            "demo",
        ])
        .current_dir(&sub)
        .assert()
        .failure()
        .get_output()
        .stderr
        .clone();
    let text = String::from_utf8_lossy(&out);
    assert!(
        text.contains("nearest: lines 2-2"),
        "an out-of-hunk quote must still be refused from a subdirectory: {text}"
    );
}

#[test]
fn change_comment_anchor_lifecycle() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    create_plan(plan.path(), "design-plan", true);
    let id = start_change_review(
        plan.path(),
        src.path(),
        "change/HEAD",
        &["--implements", "rdm:plan/design-plan"],
    );

    // (a) a quote inside the touched hunk anchors and resolves.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "comment",
            &id,
            "--path",
            "src/lib.rs",
            "--quote",
            "fn two_renamed() {}",
            "--body",
            "Name is unclear.",
            "--no-edit",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .success();
    let j = review_json(plan.path(), src.path(), &id);
    assert_eq!(j["comments"][0]["resolution"]["state"], "resolved");
    assert_eq!(j["comments"][0]["anchor"]["anchor_type"], "file-quote");
    assert_eq!(j["comments"][0]["anchor"]["start_line"], 2);

    // (b) a quote outside every touched hunk is refused, naming the nearest.
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "comment",
            &id,
            "--path",
            "src/lib.rs",
            "--quote",
            "fn three() {}",
            "--body",
            "Untouched.",
            "--no-edit",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .failure()
        .get_output()
        .stderr
        .clone();
    let text = String::from_utf8_lossy(&out);
    assert!(
        text.contains("nearest: lines 2-2"),
        "an out-of-hunk quote must name the nearest touched hunk: {text}"
    );

    // (c) a later commit editing the anchored lines reports `drifted`.
    std::fs::write(
        src.path().join("src/lib.rs"),
        "fn one() {}\nfn two_renamed_again() {}\nfn three() {}\n",
    )
    .unwrap();
    git(src.path(), &["add", "."]);
    git(src.path(), &["commit", "-m", "rename again"]);
    let j = review_json(plan.path(), src.path(), &id);
    assert_eq!(j["comments"][0]["resolution"]["state"], "drifted");

    // (d) deleting the file reports `unresolved`.
    git(src.path(), &["rm", "src/lib.rs"]);
    git(src.path(), &["commit", "-m", "drop lib"]);
    let j = review_json(plan.path(), src.path(), &id);
    assert_eq!(j["comments"][0]["resolution"]["state"], "unresolved");
}

#[test]
fn change_comment_requires_path_and_rejects_a_missing_file() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    create_plan(plan.path(), "design-plan", true);
    let id = start_change_review(
        plan.path(),
        src.path(),
        "change/HEAD",
        &["--implements", "rdm:plan/design-plan"],
    );

    // `--quote` with no `--path` names `--path`.
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "comment",
            &id,
            "--quote",
            "fn two_renamed() {}",
            "--body",
            "x",
            "--no-edit",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .failure()
        .get_output()
        .stderr
        .clone();
    assert!(String::from_utf8_lossy(&out).contains("--path"));

    // A path absent at head is refused at comment time, not silently anchored.
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "comment",
            &id,
            "--path",
            "src/gone.rs",
            "--quote",
            "anything",
            "--body",
            "x",
            "--no-edit",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .failure()
        .get_output()
        .stderr
        .clone();
    assert!(String::from_utf8_lossy(&out).contains("does not exist at"));

    // A whole-change comment (no --path/--quote) still works.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "comment",
            &id,
            "--body",
            "Overall: fine.",
            "--no-edit",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .success();
    let j = review_json(plan.path(), src.path(), &id);
    assert_eq!(j["comments"][0]["resolution"]["state"], "unresolved");
    assert!(j["comments"][0]["anchor"].is_null());
}

// --- AC3: pinned rdm:src permalink accepted by `rdm link resolve` ---

#[test]
fn change_comment_emits_resolvable_permalink() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    create_plan(plan.path(), "design-plan", true);
    let head = git_out(src.path(), &["rev-parse", "HEAD"]);
    let id = start_change_review(
        plan.path(),
        src.path(),
        "change/HEAD",
        &["--implements", "rdm:plan/design-plan"],
    );
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "comment",
            &id,
            "--path",
            "src/lib.rs",
            "--quote",
            "fn two_renamed() {}",
            "--body",
            "Name is unclear.",
            "--no-edit",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .success();

    let j = review_json(plan.path(), src.path(), &id);
    let link = j["comments"][0]["source_link"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(link, format!("rdm:src/src/lib.rs@{head}#L2"));

    // The emitted string is exactly what `rdm link resolve` accepts.
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "link",
            "resolve",
            &link,
            "--format",
            "json",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let r: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(r["kind"], "code");
    assert_eq!(r["path"], "src/lib.rs");
    assert_eq!(r["rev"], head);
    assert_eq!(r["line"], 2);
}

// --- AC4: --implements recorded / inferred / errored, plan show, backlinks ---

#[test]
fn implements_is_recorded_and_surfaced_on_the_plan() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    create_plan(plan.path(), "design-plan", true);
    let head = git_out(src.path(), &["rev-parse", "HEAD"]);
    let id = start_change_review(
        plan.path(),
        src.path(),
        "change/HEAD",
        &["--implements", "rdm:plan/design-plan"],
    );
    let j = review_json(plan.path(), src.path(), &id);
    assert_eq!(j["implements"], "rdm:plan/design-plan");

    // `plan show` lists it under `change_reviews`, separate from `reviews`.
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "plan",
            "show",
            "design-plan",
            "--format",
            "json",
            "--project",
            "demo",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let p: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(p["change_reviews"][0]["id"], id.as_str());
    assert_eq!(p["change_reviews"][0]["head"], head);
    // The approving review ON the plan is still in the other list.
    assert_eq!(p["reviews"].as_array().unwrap().len(), 1);
    assert_ne!(p["reviews"][0]["id"], id.as_str());
}

#[test]
fn implements_rejects_a_non_plan_reference_and_a_missing_plan() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    create_plan(plan.path(), "design-plan", true);

    for (arg, needle) in [
        ("rdm:task/nope", "is not an implementation plan"),
        ("rdm:plan/absent", "not found"),
    ] {
        let out = rdm()
            .arg("--root")
            .arg(plan.path())
            .args([
                "review",
                "start",
                "--on",
                "change/HEAD",
                "--implements",
                arg,
                "--no-edit",
                "--project",
                "demo",
            ])
            .current_dir(src.path())
            .assert()
            .failure()
            .get_output()
            .stderr
            .clone();
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains(needle), "for {arg}, got: {text}");
    }
}

#[test]
fn implements_is_inferred_from_a_worktrees_single_approved_plan() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    create_plan(plan.path(), "design-plan", true);
    // Put the checkout on the branch-name convention rdm's worktree
    // detection inverts: `phase/<roadmap>/<stem>`.
    git(src.path(), &["checkout", "-b", "phase/auth/phase-1-design"]);

    let id = start_change_review(plan.path(), src.path(), "change/HEAD", &[]);
    let j = review_json(plan.path(), src.path(), &id);
    assert_eq!(j["implements"], "rdm:plan/design-plan");
}

#[test]
fn implements_inference_errors_on_zero_and_on_many_approved_plans() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    git(src.path(), &["checkout", "-b", "phase/auth/phase-1-design"]);

    // Zero approved plans (a draft plan does not count).
    create_plan(plan.path(), "draft-plan", false);
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "start",
            "--on",
            "change/HEAD",
            "--no-edit",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .failure()
        .get_output()
        .stderr
        .clone();
    let text = String::from_utf8_lossy(&out);
    assert!(
        text.contains("no approved plan") && text.contains("--implements"),
        "zero approved plans must name --implements: {text}"
    );

    // Two approved plans: the error lists both candidates.
    create_plan(plan.path(), "plan-a", true);
    create_plan(plan.path(), "plan-b", true);
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "start",
            "--on",
            "change/HEAD",
            "--no-edit",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .failure()
        .get_output()
        .stderr
        .clone();
    let text = String::from_utf8_lossy(&out);
    assert!(
        text.contains("rdm:plan/plan-a") && text.contains("rdm:plan/plan-b"),
        "an ambiguous inference must list every candidate: {text}"
    );
}

#[test]
fn backlinks_reach_change_reviews_through_the_plan() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    create_plan(plan.path(), "design-plan", true);
    let id = start_change_review(
        plan.path(),
        src.path(),
        "change/HEAD",
        &["--implements", "rdm:plan/design-plan"],
    );

    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "backlinks",
            "phase/auth/phase-1-design",
            "--format",
            "json",
            "--project",
            "demo",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let entries: Vec<Value> = serde_json::from_slice(&out).unwrap();

    let plan_entry = entries
        .iter()
        .find(|e| e["kind"] == "plan")
        .unwrap_or_else(|| panic!("no plan backlink in {entries:?}"));
    assert_eq!(plan_entry["slug"], "design-plan");
    assert_eq!(plan_entry["field"], "implements");
    assert!(plan_entry["via"].is_null());

    let review_entry = entries
        .iter()
        .find(|e| e["kind"] == "review" && e["id"] == id.as_str())
        .unwrap_or_else(|| panic!("no change-review backlink in {entries:?}"));
    assert_eq!(review_entry["field"], "implements");
    assert_eq!(review_entry["via"], "rdm:plan/design-plan");
}

// --- list / requests / search treat the kind like any other ---

#[test]
fn list_and_requests_and_search_handle_a_change_target() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    create_plan(plan.path(), "design-plan", true);
    let head = git_out(src.path(), &["rev-parse", "HEAD"]);
    let id = start_change_review(
        plan.path(),
        src.path(),
        "change/HEAD",
        &["--implements", "rdm:plan/design-plan"],
    );
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "comment",
            &id,
            "--body",
            "Distinctive comment prose.",
            "--no-edit",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "submit",
            &id,
            "--verdict",
            "request-changes",
            "--body",
            "Needs work.",
            "--no-edit",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .success();

    // `list --on change/<sha>` must match even though the stored target
    // also carries a `base` the typed reference does not.
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "list",
            "--on",
            &format!("change/{head}"),
            "--format",
            "json",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let arr: Vec<Value> = serde_json::from_slice(&out).unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], id.as_str());

    // It is in the change-request queue like any other submitted review.
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "requests",
            "--format",
            "json",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let arr: Vec<Value> = serde_json::from_slice(&out).unwrap();
    assert!(arr.iter().any(|r| r["id"] == id.as_str()));

    // Its comment bodies are searchable.
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "search",
            "Distinctive",
            "--type",
            "review",
            "--format",
            "json",
            "--project",
            "demo",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8_lossy(&out);
    assert!(
        text.contains(&id),
        "change review must be searchable: {text}"
    );
}

#[test]
fn review_pending_is_unaffected_by_the_new_target_kind() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    create_plan(plan.path(), "design-plan", true);
    start_change_review(
        plan.path(),
        src.path(),
        "change/HEAD",
        &["--implements", "rdm:plan/design-plan"],
    );
    // `review pending` is the needs-review ITEM queue, not the document-review
    // list: a change review must not appear in it.
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["review", "pending", "--format", "json", "--project", "demo"])
        .current_dir(src.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let arr: Vec<Value> = serde_json::from_slice(&out).unwrap();
    assert!(
        arr.is_empty(),
        "expected an empty pending queue, got {arr:?}"
    );
}

// --- AC6: nothing beyond the quote is persisted; the source is never written ---

#[test]
fn review_file_holds_only_the_quote_and_never_writes_the_source_repo() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    create_plan(plan.path(), "design-plan", true);

    let head_before = git_out(src.path(), &["rev-parse", "HEAD"]);
    let status_before = git_out(src.path(), &["status", "--porcelain"]);
    let tree_before = git_out(src.path(), &["rev-parse", "HEAD^{tree}"]);

    let id = start_change_review(
        plan.path(),
        src.path(),
        "change/HEAD",
        &["--implements", "rdm:plan/design-plan"],
    );
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "comment",
            &id,
            "--path",
            "src/lib.rs",
            "--quote",
            "fn two_renamed() {}",
            "--body",
            "Name is unclear.",
            "--no-edit",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .success();
    let _ = review_json(plan.path(), src.path(), &id);

    // The raw review file carries the quote and nothing else from the file.
    let raw = std::fs::read_to_string(
        plan.path()
            .join("projects/demo/reviews")
            .join(format!("{id}.md")),
    )
    .unwrap();
    assert!(raw.contains("anchor_type: file-quote"));
    assert!(raw.contains("fn two_renamed() {}"));
    for neighbour in ["fn one() {}", "fn three() {}"] {
        assert!(
            !raw.contains(neighbour),
            "review file must not embed the unquoted neighbouring line {neighbour:?}:\n{raw}"
        );
    }
    assert!(
        !raw.contains("prefix:") && !raw.contains("suffix:"),
        "a file-quote anchor must store no surrounding context:\n{raw}"
    );

    // The source repository is byte-identical before and after.
    assert_eq!(git_out(src.path(), &["rev-parse", "HEAD"]), head_before);
    assert_eq!(
        git_out(src.path(), &["status", "--porcelain"]),
        status_before
    );
    assert_eq!(
        git_out(src.path(), &["rev-parse", "HEAD^{tree}"]),
        tree_before
    );
}

// --- the read path degrades, the write path does not ---

#[test]
fn review_show_degrades_with_a_note_when_no_source_repo_is_reachable() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    create_plan(plan.path(), "design-plan", true);
    let id = start_change_review(
        plan.path(),
        src.path(),
        "change/HEAD",
        &["--implements", "rdm:plan/design-plan"],
    );
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "comment",
            &id,
            "--path",
            "src/lib.rs",
            "--quote",
            "fn two_renamed() {}",
            "--body",
            "Name is unclear.",
            "--no-edit",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .success();

    // Point the project at a source repo that no longer exists, and run from
    // a non-git directory: `review show` must still print the review.
    let nowhere = TempDir::new().unwrap();
    set_project_source(plan.path(), "demo", "/nonexistent/source/repo");
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "show",
            &id,
            "--format",
            "json",
            "--project",
            "demo",
        ])
        .current_dir(nowhere.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let j: Value = serde_json::from_slice(&out).unwrap();
    assert!(
        j["source_verification_skipped"].is_string(),
        "a degraded read must say why: {j}"
    );
    assert_eq!(j["comments"][0]["resolution"]["state"], "unresolved");
    // …but the permalink still renders, because it needs no checkout.
    assert!(j["comments"][0]["source_link"].is_string());

    // The write path fails loudly instead of degrading.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "comment",
            &id,
            "--path",
            "src/lib.rs",
            "--quote",
            "fn one() {}",
            "--body",
            "x",
            "--no-edit",
            "--project",
            "demo",
        ])
        .current_dir(nowhere.path())
        .assert()
        .failure();
}

// --- the `change` link-grammar divergence ---

#[test]
fn change_is_a_review_target_but_never_a_link() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "link",
            "resolve",
            "rdm:change/deadbeef",
            "--format",
            "json",
            "--project",
            "demo",
        ])
        .assert()
        .failure()
        .get_output()
        .stderr
        .clone();
    let text = String::from_utf8_lossy(&out);
    assert!(
        text.contains("rdm:src/<path>@<sha>"),
        "rejecting an rdm:change link must point at the code-link form: {text}"
    );
    let _ = src;
}

// --- the guards that keep --base/--implements change-only ---

/// `--base` and `--implements` describe a *change* review. Passing either on
/// any other target kind must be refused, by a message naming the flag and how
/// to use it, rather than silently ignored — otherwise a caller that mistyped
/// `--on` would get a review that quietly dropped what it asked for.
#[test]
fn base_and_implements_are_refused_on_every_non_change_target() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    create_plan(plan.path(), "design-plan", true);
    let head = git_out(src.path(), &["rev-parse", "HEAD"]);
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "task",
            "create",
            "fix-login",
            "--title",
            "Fix login",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .success();

    for on in [
        "task/fix-login",
        "phase/auth/phase-1-design",
        "roadmap/auth",
        "plan/design-plan",
    ] {
        for (flag, value, needle) in [
            (
                "--base",
                head.as_str(),
                "--base only applies to a change review",
            ),
            (
                "--implements",
                "rdm:plan/design-plan",
                "--implements records which plan a reviewed *change* implements",
            ),
        ] {
            let out = rdm()
                .arg("--root")
                .arg(plan.path())
                .args([
                    "review",
                    "start",
                    "--on",
                    on,
                    flag,
                    value,
                    "--no-edit",
                    "--project",
                    "demo",
                ])
                .current_dir(src.path())
                .assert()
                .failure()
                .get_output()
                .stderr
                .clone();
            let text = String::from_utf8_lossy(&out);
            assert!(
                text.contains(needle),
                "`review start --on {on} {flag}` must be refused by a message naming the flag, got: {text}"
            );
            assert!(
                text.contains("change/<sha>"),
                "`review start --on {on} {flag}` must say how to use the flag, got: {text}"
            );
        }
    }
}

// --- source-repo discovery: which checkout a change review reads from ---
//
// `discover_source_repo`'s whole reason to exist is refusing to answer
// "which repository is this?" with "whichever one the cwd happens to be
// in". Every other test in this file runs from a checkout that *is* the
// project's configured `source.repo`, so these four cover the branches
// where the cwd and the configuration disagree — or where one of them is
// absent entirely.

/// cwd is a git checkout, but a different one from the project's source,
/// and `source.repo` names a local directory: the configured source wins,
/// so the pinned head is the *source* repo's tip and never the cwd's.
#[test]
fn change_review_prefers_the_configured_source_over_an_unrelated_checkout() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    create_plan(plan.path(), "design-plan", true);
    let other = init_unrelated_repo();

    let source_head = git_out(src.path(), &["rev-parse", "HEAD"]);
    let other_head = git_out(other.path(), &["rev-parse", "HEAD"]);
    assert_ne!(source_head, other_head, "the fixture repos must differ");

    let id = start_change_review(
        plan.path(),
        other.path(),
        "change/HEAD",
        &["--implements", "rdm:plan/design-plan"],
    );
    let j = review_json(plan.path(), src.path(), &id);
    assert_eq!(
        j["target"]["head"], source_head,
        "change/HEAD must pin the configured source repo's tip, not the cwd's"
    );
    assert_eq!(j["change_branch"], "topic");
}

/// Same disagreement, but `source.repo` is a URL rather than a local path,
/// so there is nothing to fall back to: refuse, and say which repository
/// was expected.
#[test]
fn change_review_refuses_an_unrelated_checkout_when_the_source_is_not_local() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    create_plan(plan.path(), "design-plan", true);
    set_project_source(plan.path(), "demo", "https://example.com/org/repo");
    let other = init_unrelated_repo();

    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "start",
            "--on",
            "change/HEAD",
            "--implements",
            "rdm:plan/design-plan",
            "--no-edit",
            "--project",
            "demo",
        ])
        .current_dir(other.path())
        .assert()
        .failure()
        .get_output()
        .stderr
        .clone();
    let text = String::from_utf8_lossy(&out);
    assert!(
        text.contains("inside a git checkout, but not project 'demo''s configured source repo"),
        "expected the repo-confusion refusal: {text}"
    );
    assert!(
        text.contains("https://example.com/org/repo"),
        "the refusal must name the configured repo: {text}"
    );
}

/// No `source.repo` at all: there is nothing to contradict the cwd, so the
/// checkout the operator is standing in is the one that gets read.
#[test]
fn change_review_uses_the_cwd_checkout_when_the_project_configures_no_source() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    create_plan(plan.path(), "design-plan", true);
    clear_project_source(plan.path(), "demo");
    let other = init_unrelated_repo();
    let other_head = git_out(other.path(), &["rev-parse", "HEAD"]);

    let id = start_change_review(
        plan.path(),
        other.path(),
        "change/HEAD",
        &["--implements", "rdm:plan/design-plan"],
    );
    let j = review_json(plan.path(), other.path(), &id);
    assert_eq!(j["target"]["head"], other_head);
    assert_eq!(j["change_branch"], "main");
}

/// Outside any checkout: the two no-cwd-repo branches each name what to do,
/// and they say different things depending on whether a source is
/// configured at all.
#[test]
fn change_review_start_names_what_to_do_when_no_checkout_is_reachable() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    create_plan(plan.path(), "design-plan", true);
    let nowhere = TempDir::new().unwrap();

    let start_from_nowhere = |plan: &Path| -> String {
        let out = rdm()
            .arg("--root")
            .arg(plan)
            .args([
                "review",
                "start",
                "--on",
                "change/HEAD",
                "--implements",
                "rdm:plan/design-plan",
                "--no-edit",
                "--project",
                "demo",
            ])
            .current_dir(nowhere.path())
            .assert()
            .failure()
            .get_output()
            .stderr
            .clone();
        String::from_utf8_lossy(&out).to_string()
    };

    // A configured-but-unreachable source names that source.
    set_project_source(plan.path(), "demo", "https://example.com/org/repo");
    let text = start_from_nowhere(plan.path());
    assert!(
        text.contains("not inside a git checkout, and project 'demo''s configured source repo"),
        "expected the unreachable-source message: {text}"
    );
    assert!(text.contains("https://example.com/org/repo"), "{text}");

    // No configured source at all points at `source.repo` in project.md.
    clear_project_source(plan.path(), "demo");
    let text = start_from_nowhere(plan.path());
    assert!(
        text.contains("configures no source repo"),
        "expected the no-source message: {text}"
    );
    assert!(
        text.contains("source.repo"),
        "the no-source message must name what to set: {text}"
    );
}
