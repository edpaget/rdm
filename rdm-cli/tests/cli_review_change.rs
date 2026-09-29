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

#[path = "git_test_support.rs"]
mod git_test_support;
use git_test_support::git;

fn rdm() -> Command {
    let mut cmd = Command::cargo_bin("rdm").unwrap();
    cmd.env("XDG_CONFIG_HOME", "/dev/null/nonexistent");
    cmd
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
    // `ref` is the full, head-only `--on` form, even from an abbreviated rev.
    assert_eq!(j["target"]["ref"], format!("change/{head}"));
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
    // An orphan branch shares no history with `main`. TWO commits, so the
    // `--base <root>` escape hatch below names a genuine ancestor rather than
    // HEAD itself — this test is about the missing merge base, not about the
    // separate empty-reviewed-range rule.
    git(src.path(), &["checkout", "--orphan", "island"]);
    std::fs::write(src.path().join("other.txt"), "hello\n").unwrap();
    git(src.path(), &["add", "other.txt"]);
    git(src.path(), &["commit", "-m", "orphan root"]);
    std::fs::write(src.path().join("other.txt"), "hello again\n").unwrap();
    git(src.path(), &["add", "other.txt"]);
    git(src.path(), &["commit", "-m", "orphan tip"]);

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
    // A change review indexes source-repo content, not a document body:
    // there is no current-body placement.
    assert!(j["comments"][0]["resolution"]["current"].is_null(), "{j}");
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

/// `diff.relative = true` in a real `~/.gitconfig` is the exact hostile
/// setting `2c55784` fixed (`--no-relative` in `unified_diff_argv`) — but
/// that bug only reproduces when the repo is rooted at a SUBdirectory of the
/// checkout (see `unified_diff_argv`'s own doc comment). This test combines
/// both conditions at once, matching the unit-test upgrade in
/// `rdm-git/src/source.rs`
/// (`unified_diff_finds_hunks_from_a_subdirectory_of_the_checkout`): the
/// `rdm` command runs from a sibling subdirectory of the checkout AND the
/// hostile setting reaches its git subprocess through the GLOBAL config
/// layer. The scratch config lives entirely inside this test's own
/// `TempDir` (`write_global_config`) and is wired in via `GIT_CONFIG_GLOBAL`
/// on the `rdm` command itself — never a real global config.
#[test]
fn change_comment_anchors_under_a_hostile_ambient_git_config() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    create_plan(plan.path(), "design-plan", true);

    // A sibling directory to invoke from, exactly as
    // `change_comment_anchors_from_a_subdirectory_of_the_checkout` does.
    // `init_source_repo` leaves HEAD on `topic`, so this commit lands only
    // there — it just needs to exist in the working tree to `cd` into.
    let sub = src.path().join("other");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(sub.join("keep.txt"), "x\n").unwrap();
    git(src.path(), &["add", "."]);
    git(src.path(), &["commit", "-m", "sibling"]);

    let global_dir = TempDir::new().unwrap();
    let hostile =
        git_test_support::write_global_config(global_dir.path(), "[diff]\n\trelative = true\n");

    // Start the review from the subdirectory too — `change/HEAD` must still
    // pin this checkout's tip.
    let id = start_change_review(
        plan.path(),
        &sub,
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
            "Anchored under a hostile global git config.",
            "--no-edit",
            "--project",
            "demo",
        ])
        .env("GIT_CONFIG_GLOBAL", hostile.to_str().unwrap())
        .current_dir(&sub)
        .assert()
        .success();

    let j = review_json(plan.path(), &sub, &id);
    assert_eq!(j["comments"][0]["resolution"]["state"], "resolved");
    assert_eq!(j["comments"][0]["anchor"]["anchor_type"], "file-quote");
    assert_eq!(j["comments"][0]["anchor"]["start_line"], 2);
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
    assert!(j["comments"][0]["resolution"]["current"].is_null(), "{j}");

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

/// AC1 end-to-end: a `--path` carrying one of the `rdm:src/` permalink
/// grammar's own delimiters is refused at the binary boundary, naming the
/// character, and writes no comment — so no anchor can be stored whose
/// permalink parses back to a different file.
#[test]
fn change_comment_rejects_a_path_carrying_a_permalink_delimiter() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    create_plan(plan.path(), "design-plan", true);
    let id = start_change_review(
        plan.path(),
        src.path(),
        "change/HEAD",
        &["--implements", "rdm:plan/design-plan"],
    );

    for (path, delimiter) in [("src/a@b.rs", "@"), ("src/a#L3.rs", "#")] {
        let out = rdm()
            .arg("--root")
            .arg(plan.path())
            .args([
                "review",
                "comment",
                &id,
                "--path",
                path,
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
        let err = String::from_utf8_lossy(&out).to_string();
        assert!(
            err.contains(delimiter),
            "the refusal must name the offending character: {err}"
        );
        assert!(
            err.contains("rdm:src/<path>@<rev>#L<n>"),
            "the refusal must name the grammar that reserves it: {err}"
        );
        assert!(
            !err.contains("does not exist at"),
            "the refusal must not assert the false cause: {err}"
        );
    }

    // Nothing was written.
    let j = review_json(plan.path(), src.path(), &id);
    assert_eq!(
        j["comments"].as_array().map_or(0, Vec::len),
        0,
        "a refused --path must leave the review untouched"
    );
}

// --- directory/submodule anchors are rejected, never silently anchored ---

/// A source repo with `sub/{a,b,c,d,e}.txt`, whose `topic` branch edits line
/// 3 of `sub/e.txt` — the exact repro shape from the
/// `change-review-path-accepts-a-directory` task.
fn init_source_repo_with_subdir() -> TempDir {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    git(p, &["init", "-b", "main"]);
    std::fs::create_dir_all(p.join("sub")).unwrap();
    for name in ["a", "b", "c", "d", "e"] {
        std::fs::write(
            p.join("sub").join(format!("{name}.txt")),
            "one\ntwo\nthree\nfour\n",
        )
        .unwrap();
    }
    git(p, &["add", "."]);
    git(p, &["commit", "-m", "base"]);
    git(p, &["checkout", "-b", "topic"]);
    std::fs::write(p.join("sub/e.txt"), "one\ntwo\nTHREE\nfour\n").unwrap();
    git(p, &["add", "."]);
    git(p, &["commit", "-m", "edit sub/e.txt"]);
    dir
}

#[test]
fn change_comment_rejects_a_directory_path() {
    let src = init_source_repo_with_subdir();
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
            "review",
            "comment",
            &id,
            "--path",
            "sub",
            "--quote",
            "a.txt",
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
    let text = String::from_utf8_lossy(&out);
    assert!(text.contains("sub"), "must name the offending path: {text}");
    assert!(
        text.contains("directory"),
        "must say it is a directory: {text}"
    );

    // No comment, and therefore no anchor, was written.
    let j = review_json(plan.path(), src.path(), &id);
    assert!(
        j["comments"].as_array().unwrap().is_empty(),
        "a rejected directory anchor must persist nothing: {j}"
    );
}

/// Simulates a `--path`/`--quote` anchor stored before this check existed (or
/// a hand-edited review file): `sub` is a real directory in the source repo,
/// so `review show` must report the comment unresolved with an explanation
/// and emit no `source_link`, in every output format.
#[test]
fn change_review_reports_a_stored_tree_anchor_as_unresolved_with_no_permalink() {
    let src = init_source_repo_with_subdir();
    let plan = init_plan_repo(src.path());
    create_plan(plan.path(), "design-plan", true);
    let id = start_change_review(
        plan.path(),
        src.path(),
        "change/HEAD",
        &["--implements", "rdm:plan/design-plan"],
    );

    edit_review_frontmatter(plan.path(), &id, |v| {
        v.comments.push(rdm_core::model::ReviewComment {
            anchor_state: None,
            intended_path: None,
            id: 1,
            doc: None,
            status: rdm_core::model::ReviewCommentStatus::Open,
            applied_commit: None,
            anchor: Some(rdm_core::model::Anchor::FileQuote {
                path: "sub".to_string(),
                quote: "a.txt".to_string(),
                occurrence: 1,
                start_line: 1,
                end_line: 1,
            }),
            body: "This looks wrong.".to_string(),
            reply: None,
        });
    });

    let j = review_json(plan.path(), src.path(), &id);
    assert_eq!(j["comments"][0]["resolution"]["state"], "unresolved");
    let reason = j["comments"][0]["unresolved_reason"]
        .as_str()
        .unwrap_or_else(|| panic!("expected an unresolved_reason: {j}"));
    assert!(reason.contains("directory"), "{reason}");
    assert!(j["comments"][0]["source_link"].is_null(), "{j}");

    for extra_format in ["human", "markdown"] {
        let out = rdm()
            .arg("--root")
            .arg(plan.path())
            .args([
                "review",
                "show",
                &id,
                "--format",
                extra_format,
                "--project",
                "demo",
            ])
            .current_dir(src.path())
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let text = String::from_utf8_lossy(&out);
        assert!(
            text.contains("directory"),
            "{extra_format} output must show the reason: {text}"
        );
        assert!(
            !text.contains("Source:"),
            "{extra_format} output must not emit a permalink: {text}"
        );
    }
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
    assert!(
        text.contains("rdm:src@<sha>"),
        "rejecting an rdm:change link must also name the commit-link form: {text}"
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

/// No `source.repo` at all, and the cwd checkout is not the plan repo (E6a):
/// there is nothing to contradict the cwd, so the checkout the operator is
/// standing in is the one that gets read.
#[test]
fn change_review_uses_the_cwd_checkout_when_the_project_configures_no_source() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    create_plan(plan.path(), "design-plan", true);
    clear_project_source(plan.path(), "demo");
    let other = init_unrelated_repo();
    // A topic branch off `main`, so the DERIVED merge base is a real ancestor
    // rather than HEAD itself. Standing on the default branch would make the
    // reviewed range empty, which `resolve_change_target` now refuses — a
    // different rule from the one under test here.
    git(other.path(), &["checkout", "-b", "work"]);
    std::fs::write(other.path().join("README.md"), "still not the source\n").unwrap();
    git(other.path(), &["add", "."]);
    git(other.path(), &["commit", "-m", "work"]);
    let other_head = git_out(other.path(), &["rev-parse", "HEAD"]);

    let id = start_change_review(
        plan.path(),
        other.path(),
        "change/HEAD",
        &["--implements", "rdm:plan/design-plan"],
    );
    let j = review_json(plan.path(), other.path(), &id);
    assert_eq!(j["target"]["head"], other_head);
    assert_eq!(j["change_branch"], "work");
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

// Malicious values are confined to three retained, independent fixtures.
fn edit_review_frontmatter(plan: &Path, id: &str, edit: impl FnOnce(&mut rdm_core::model::Review)) {
    let path = plan.join("projects/demo/reviews").join(format!("{id}.md"));
    let content = std::fs::read_to_string(&path).unwrap();
    let mut doc = rdm_core::document::Document::<rdm_core::model::Review>::parse(&content).unwrap();
    edit(&mut doc.frontmatter);
    std::fs::write(path, doc.render().unwrap()).unwrap();
}

#[test]
fn malformed_stored_change_revisions_cannot_create_or_overwrite_external_files() {
    for field in ["head", "base"] {
        for preexisting in [false, true] {
            let src = init_source_repo();
            std::fs::write(src.path().join("a.txt"), "quoted attack fixture\n").unwrap();
            git(src.path(), &["add", "."]);
            git(src.path(), &["commit", "-m", "root attack fixture"]);
            let plan = init_plan_repo(src.path());
            let outside = TempDir::new().unwrap();
            create_plan(plan.path(), "attack-plan", true);
            let id = start_change_review(
                plan.path(),
                src.path(),
                "change/HEAD",
                &["--implements", "rdm:plan/attack-plan"],
            );
            rdm()
                .arg("--root")
                .arg(plan.path())
                .args([
                    "review",
                    "comment",
                    &id,
                    "--path",
                    "a.txt",
                    "--quote",
                    "quoted attack fixture",
                    "--body",
                    "check",
                    "--no-edit",
                    "--project",
                    "demo",
                ])
                .current_dir(src.path())
                .assert()
                .success();
            let output = outside.path().join("sentinel");
            let actual = outside.path().join("sentinel:a.txt");
            if preexisting {
                std::fs::write(&actual, b"unique protected bytes").unwrap();
            }
            edit_review_frontmatter(plan.path(), &id, |v| {
                let rdm_core::model::ReviewTarget::Change { head, base } = &mut v.target else {
                    panic!()
                };
                let payload = format!("--output={}", output.display());
                if field == "head" {
                    *head = payload;
                } else {
                    *base = Some(payload);
                }
            });
            let result = rdm()
                .arg("--root")
                .arg(plan.path())
                .args(["review", "show", &id, "--project", "demo"])
                .current_dir(src.path())
                .output()
                .unwrap();
            // Inspect bytes even if the vulnerable command reports success.
            if preexisting {
                assert_eq!(std::fs::read(&actual).unwrap(), b"unique protected bytes");
            } else {
                assert!(!actual.exists(), "show created an external output file");
            }
            assert!(
                !result.status.success(),
                "{field} must fail before resolution"
            );
            rdm()
                .arg("--root")
                .arg(plan.path())
                .args(["review", "list", "--project", "demo"])
                .current_dir(src.path())
                .assert()
                .failure();
        }
    }
}

#[test]
fn malformed_identities_without_comments_are_rejected_offline() {
    for field in ["head", "base"] {
        let src = init_source_repo();
        let plan = init_plan_repo(src.path());
        let outside = TempDir::new().unwrap();
        create_plan(plan.path(), "attack-plan", true);
        let id = start_change_review(
            plan.path(),
            src.path(),
            "change/HEAD",
            &["--implements", "rdm:plan/attack-plan"],
        );
        edit_review_frontmatter(plan.path(), &id, |v| {
            let rdm_core::model::ReviewTarget::Change { head, base } = &mut v.target else {
                panic!()
            };
            if field == "head" {
                *head = "HEAD".into();
            } else {
                *base = Some("HEAD".into());
            }
        });
        set_project_source(plan.path(), "demo", "/nonexistent/source/repo");
        rdm()
            .arg("--root")
            .arg(plan.path())
            .args(["review", "show", &id, "--project", "demo"])
            .current_dir(outside.path())
            .assert()
            .failure();
        rdm()
            .arg("--root")
            .arg(plan.path())
            .args(["review", "list", "--project", "demo"])
            .current_dir(outside.path())
            .assert()
            .failure();
    }
}

/// An option-shaped stored `change_branch` is refused by the adapter's
/// pre-spawn guard ([`rdm_core::error::Error::InvalidChangeRevisionInput`]),
/// and under `rdm_core::change::resolve_drift_tip` that refusal is
/// **propagated, not swallowed**: `rdm review show` degrades to
/// all-unresolved with a note naming the refusal, rather than silently
/// measuring drift against a *different* revision (HEAD).
///
/// This is the one intentional behavior change of the drift-tip move. The
/// security invariant is unchanged and still asserted below: no subprocess
/// is spawned, nothing is written, and a pre-existing file at the
/// option's target is untouched.
#[test]
fn option_shaped_drift_branch_is_refused_rather_than_repointed_at_head() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    let outside = TempDir::new().unwrap();
    create_plan(plan.path(), "branch-plan", true);
    let id = start_change_review(
        plan.path(),
        src.path(),
        "change/HEAD",
        &["--implements", "rdm:plan/branch-plan"],
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
            "check",
            "--no-edit",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .success();
    std::fs::write(src.path().join("src/lib.rs"), "fn changed_again() {}\n").unwrap();
    git(src.path(), &["add", "."]);
    git(src.path(), &["commit", "-m", "branch drift"]);
    let control = review_json(plan.path(), src.path(), &id);
    assert_eq!(control["comments"][0]["resolution"]["state"], "drifted");
    let sentinel = outside.path().join("branch-output");
    for preexisting in [false, true] {
        if preexisting {
            std::fs::write(&sentinel, b"branch protected bytes").unwrap();
        }
        edit_review_frontmatter(plan.path(), &id, |v| {
            v.change_branch = Some(format!("--output={}", sentinel.display()))
        });
        let actual = review_json(plan.path(), src.path(), &id);
        assert_eq!(
            actual["comments"][0]["resolution"]["state"], "unresolved",
            "a refused drift-tip lookup must not silently re-point drift at HEAD"
        );
        assert_ne!(
            actual["comments"][0]["resolution"], control["comments"][0]["resolution"],
            "the control run resolved against the real branch; this one must not"
        );
        let note = actual["source_verification_skipped"]
            .as_str()
            .expect("the refusal is explained, not silent");
        assert!(
            note.contains("anchor resolution skipped"),
            "unexpected note: {note}"
        );
        if preexisting {
            assert_eq!(std::fs::read(&sentinel).unwrap(), b"branch protected bytes");
        } else {
            assert!(!sentinel.exists());
        }
    }
}

#[test]
fn option_shaped_configured_default_branch_is_refused() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    let outside = TempDir::new().unwrap();
    create_plan(plan.path(), "branch-plan", true);
    let project_path = plan.path().join("projects/demo/project.md");
    let mut project = rdm_core::document::Document::<rdm_core::model::Project>::parse(
        &std::fs::read_to_string(&project_path).unwrap(),
    )
    .unwrap();
    let sentinel = outside.path().join("default-output");
    project.frontmatter.source.as_mut().unwrap().default_branch =
        Some(format!("--output={}", sentinel.display()));
    std::fs::write(project_path, project.render().unwrap()).unwrap();
    let output = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "start",
            "--on",
            "change/HEAD",
            "--implements",
            "rdm:plan/branch-plan",
            "--no-edit",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .output()
        .unwrap();
    assert!(!sentinel.exists());
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("must not start"));
}

// --- AC3: identity root vs. read root ---
//
// `rdm_git::worktree::discover_project_repo` deliberately answers with the
// repository's MAIN working tree, which is the right answer for *identity*
// ("is this checkout the project's source?") and the wrong one to *read*
// through: reading it would make `change/HEAD` pin main's HEAD and stamp
// `main` as the change branch. These two tests pin that split, and the
// three-cell disagreement with `rdm link check` that survives it.

/// A linked worktree is still the configured source *repository*, so
/// identity matches — but the revision pinned and the branch stamped must
/// be the linked worktree's own, never the main working tree's.
#[test]
fn change_review_in_a_linked_worktree_pins_that_worktrees_head_not_main() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    create_plan(plan.path(), "design-plan", true);

    // The worktree lives inside the test's own TempDir, never the system
    // temp dir — see the `suite_hygiene::temp_hygiene` test.
    let workspace = TempDir::new().unwrap();
    let linked = workspace.path().join("linked");
    git(
        src.path(),
        &[
            "worktree",
            "add",
            "-b",
            "feature/x",
            &linked.to_string_lossy(),
            "HEAD",
        ],
    );
    std::fs::write(linked.join("src/lib.rs"), "fn only_on_feature_x() {}\n").unwrap();
    git(&linked, &["add", "."]);
    git(&linked, &["commit", "-m", "worktree-only commit"]);

    let main_head = git_out(src.path(), &["rev-parse", "HEAD"]);
    let linked_head = git_out(&linked, &["rev-parse", "HEAD"]);
    assert_ne!(
        main_head, linked_head,
        "the fixture must put the two working trees on different commits"
    );

    let id = start_change_review(
        plan.path(),
        &linked,
        "change/HEAD",
        &["--implements", "rdm:plan/design-plan"],
    );
    let j = review_json(plan.path(), &linked, &id);
    assert_eq!(
        j["target"]["head"], linked_head,
        "change/HEAD must pin the linked worktree's own tip"
    );
    assert_ne!(
        j["target"]["head"], main_head,
        "reading through the identity root would pin the MAIN working tree's HEAD"
    );
    assert_eq!(
        j["change_branch"], "feature/x",
        "the stamped branch must be the worktree's, not 'main'"
    );

    git(
        src.path(),
        &["worktree", "remove", "--force", &linked.to_string_lossy()],
    );
}

/// Environment E2 of the shared decision table — cwd inside an unrelated
/// checkout with a configured LOCAL source — is one of the three cells the
/// two consumers deliberately answer differently: change review leaves the
/// cwd for the configured directory, while `rdm link check` refuses to
/// verify against either repository and says so.
#[test]
fn change_review_falls_back_to_the_configured_local_source_while_link_check_skips() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    create_plan(plan.path(), "design-plan", true);
    let other = init_unrelated_repo();

    let source_head = git_out(src.path(), &["rev-parse", "HEAD"]);
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "task",
            "create",
            "code-link",
            "--title",
            "Code link",
            "--body",
            "Code: [src](rdm:src/does-not-exist.rs).",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .success();

    // Change review (`ConfiguredLocal`): reads the configured directory.
    let id = start_change_review(
        plan.path(),
        other.path(),
        "change/HEAD",
        &["--implements", "rdm:plan/design-plan"],
    );
    let j = review_json(plan.path(), src.path(), &id);
    assert_eq!(j["target"]["head"], source_head);

    // Link check (`CwdOnly`), same cwd: skips entirely, and never verifies
    // against the unrelated checkout it is standing in.
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .current_dir(other.path())
        .args([
            "link",
            "check",
            "--on",
            "task/code-link",
            "--project",
            "demo",
            "--format",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let report: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(report["missing_at_rev"], serde_json::json!([]));
    assert!(
        report["path_verification_skipped"]
            .as_str()
            .is_some_and(|s| s.contains("not the project's configured source")),
        "link check must skip, not follow change review into the configured dir: {report}"
    );
}

// --- Phase 20: source-read and persist defects ---

/// Defect 1 (AC1). With an external diff driver and a `.gitattributes`
/// `textconv` filter both configured in the source repo, `rdm review comment
/// --path --quote` must anchor correctly and NEITHER script may run.
///
/// Without `--no-ext-diff`/`--no-textconv`, git executes the configured program
/// and its output replaces the unified diff, so `derive_change_anchor` computes
/// an empty hunk set and refuses a genuinely-modified file as untouched — while
/// having run an operator-supplied script during a read rdm documents as
/// read-only.
#[test]
fn change_comment_anchors_with_a_hostile_external_diff_driver_and_textconv_filter() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    create_plan(plan.path(), "design-plan", true);

    // Markers live outside the source repo so writing them cannot itself dirty
    // the checkout under review.
    let scratch = TempDir::new().unwrap();
    let ext_marker = scratch.path().join("external-diff-ran");
    let textconv_marker = scratch.path().join("textconv-ran");
    let ext_script = scratch.path().join("ext-diff.sh");
    let textconv_script = scratch.path().join("textconv.sh");
    write_marker_script(&ext_script, &ext_marker, false);
    write_marker_script(&textconv_script, &textconv_marker, true);

    git(
        src.path(),
        &["config", "diff.external", ext_script.to_str().unwrap()],
    );
    git(
        src.path(),
        &[
            "config",
            "diff.hostile.textconv",
            textconv_script.to_str().unwrap(),
        ],
    );
    std::fs::write(
        src.path().join(".gitattributes"),
        "src/lib.rs diff=hostile\n",
    )
    .unwrap();
    git(src.path(), &["add", ".gitattributes"]);
    git(src.path(), &["commit", "-m", "hostile diff attributes"]);

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
            "Anchored despite a hostile diff driver.",
            "--no-edit",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .success();

    let j = review_json(plan.path(), src.path(), &id);
    assert_eq!(j["comments"][0]["resolution"]["state"], "resolved");
    assert_eq!(
        j["comments"][0]["anchor"]["start_line"], 2,
        "the recorded line range must come from the real hunk"
    );
    assert!(
        !ext_marker.exists(),
        "the configured external diff driver EXECUTED during an rdm read"
    );
    assert!(
        !textconv_marker.exists(),
        "the configured textconv filter EXECUTED during an rdm read"
    );
}

/// Writes an executable shell script that touches `marker`. `passthrough` makes
/// it `cat` its first argument too, which is what a `textconv` filter has to do
/// to look like a real one.
fn write_marker_script(path: &Path, marker: &Path, passthrough: bool) {
    let body = if passthrough {
        format!("#!/bin/sh\n: > '{}'\ncat \"$1\"\n", marker.display())
    } else {
        format!("#!/bin/sh\n: > '{}'\nexit 0\n", marker.display())
    };
    std::fs::write(path, body).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// Defect 3 (AC2). A filename holding a glob metacharacter must not anchor into
/// a DIFFERENT file's hunks. `:(top)a?b.txt` matched the modified sibling
/// `axb.txt` and returned its hunks, so this command used to SUCCEED, storing an
/// anchor whose `start_line`/`end_line` and `rdm:src/` permalink described a
/// file the reviewed range never touched.
#[test]
fn change_comment_refuses_a_glob_path_that_the_change_never_touched() {
    let src = init_source_repo();
    let p = src.path();
    // `a?b.txt` exists at both ends of the range and is never modified;
    // `axb.txt` is what the change actually edits.
    std::fs::write(p.join("a?b.txt"), "fn glob_named() {}\n").unwrap();
    assert!(
        p.join("a?b.txt").exists(),
        "this filesystem rejected '?' in a filename — the glob fixture cannot be built"
    );
    std::fs::write(p.join("axb.txt"), "fn sibling() {}\n").unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-m", "add both names"]);
    // Rebase the review range so the base already holds both files: only
    // `axb.txt` changes between base and head.
    let base = git_out(p, &["rev-parse", "HEAD"]);
    std::fs::write(p.join("axb.txt"), "fn sibling_renamed() {}\n").unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-m", "edit axb only"]);

    let plan = init_plan_repo(p);
    create_plan(plan.path(), "design-plan", true);
    let id = start_change_review(
        plan.path(),
        p,
        "change/HEAD",
        &["--base", &base, "--implements", "rdm:plan/design-plan"],
    );

    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "comment",
            &id,
            "--path",
            "a?b.txt",
            "--quote",
            "fn glob_named() {}",
            "--body",
            "Should never anchor.",
            "--no-edit",
            "--project",
            "demo",
        ])
        .current_dir(p)
        .assert()
        .failure()
        .get_output()
        .stderr
        .clone();
    let text = String::from_utf8_lossy(&out);
    assert!(
        text.contains("is not touched by"),
        "the untouched-path refusal must name the real problem: {text}"
    );
    assert!(
        !text.contains("axb.txt"),
        "the message must not be derived from the sibling file at all: {text}"
    );
    // The control: the file that really changed still anchors, so the literal
    // pathspec did not simply break anchoring.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "comment",
            &id,
            "--path",
            "axb.txt",
            "--quote",
            "fn sibling_renamed() {}",
            "--body",
            "The file that really changed.",
            "--no-edit",
            "--project",
            "demo",
        ])
        .current_dir(p)
        .assert()
        .success();
    let j = review_json(plan.path(), p, &id);
    assert_eq!(
        j["comments"].as_array().map(Vec::len),
        Some(1),
        "the refused comment must not have been recorded"
    );
    assert_eq!(j["comments"][0]["anchor"]["path"], "axb.txt");
}

/// Defect 4, first half (AC4). Standing INSIDE the plan repo with the project
/// configuring no `source.repo`, `change/HEAD` used to pin the plan repo's own
/// HEAD and silently mint a review of the plan data. The refusal must name both
/// ways out.
#[test]
fn change_review_start_refuses_the_plan_repo_as_a_source() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    create_plan(plan.path(), "design-plan", true);
    clear_project_source(plan.path(), "demo");
    // Make the plan repo a real checkout with a commit, so the cwd genuinely
    // resolves to a repository (the E6 arm) rather than to no checkout at all.
    git(plan.path(), &["init", "-b", "main"]);
    git(plan.path(), &["add", "-A"]);
    git(plan.path(), &["commit", "-m", "plan repo"]);

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
        .current_dir(plan.path())
        .assert()
        .failure()
        .get_output()
        .stderr
        .clone();
    let text = String::from_utf8_lossy(&out);
    assert!(
        text.contains("the plan repo"),
        "the refusal must say what the current directory is: {text}"
    );
    assert!(
        text.contains("source.repo"),
        "the refusal must name the remedy: {text}"
    );

    // And nothing was recorded.
    let listed = rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["review", "list", "--format", "json", "--project", "demo"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let reviews: Value = serde_json::from_slice(&listed).unwrap();
    assert!(
        reviews
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["target"]["kind"] != "change"),
        "no change review may have been minted: {reviews}"
    );
}

/// Defect 4, second half (AC4). A DERIVED reviewed range with no commits in it
/// is refused; the same command with an explicit `--base` naming a real
/// ancestor still works, which is what proves the guard is scoped to a derived
/// base rather than to `base == head`.
#[test]
fn change_review_start_refuses_a_derived_empty_reviewed_range() {
    let src = init_source_repo();
    let p = src.path();
    // Stand ON the default branch, where merge_base(HEAD, main) == HEAD.
    git(p, &["checkout", "main"]);
    let plan = init_plan_repo(p);
    create_plan(plan.path(), "design-plan", true);

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
        .current_dir(p)
        .assert()
        .failure()
        .get_output()
        .stderr
        .clone();
    let text = String::from_utf8_lossy(&out);
    assert!(
        text.contains("empty reviewed range"),
        "a derived empty range must be refused as such: {text}"
    );
    assert!(
        text.contains("--base"),
        "the refusal must name the escape hatch: {text}"
    );

    // An explicit `--base` naming a real ancestor is unaffected.
    let root = git_out(p, &["rev-list", "--max-parents=0", "HEAD"]);
    git(p, &["checkout", "topic"]);
    start_change_review(
        plan.path(),
        p,
        "change/HEAD",
        &["--base", &root, "--implements", "rdm:plan/design-plan"],
    );
}

/// A second source repository with its OWN independent history: its own `git
/// init`, its own single commit, no objects in common with the real source. A
/// checkout like this is what an operator has when they forgot to fetch, or
/// pointed `source.repo` at the wrong clone.
fn init_independent_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    git(p, &["init", "-b", "main"]);
    std::fs::create_dir_all(p.join("src")).unwrap();
    // The SAME path exists here, so a false "path no longer exists" cannot be
    // excused by the path genuinely being absent.
    std::fs::write(p.join("src/lib.rs"), "fn unrelated() {}\n").unwrap();
    git(p, &["add", "."]);
    git(p, &["commit", "-m", "independent history"]);
    dir
}

/// Defect 5, read path (AC5). Against a checkout that does not contain the
/// reviewed commit, every comment is `unresolved` with the skip stated ONCE in
/// `source_verification_skipped` and NO per-comment `unresolved_reason` — an
/// environmental miss is never dressed up as a verdict about the reviewer's
/// quote.
#[test]
fn change_review_show_reports_a_missing_reviewed_commit_as_a_skip() {
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
            "Anchored against the real source.",
            "--no-edit",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .success();

    // Repoint the project at a checkout with independent history.
    let other = init_independent_repo();
    set_project_source(plan.path(), "demo", &other.path().to_string_lossy());

    let j = review_json(plan.path(), other.path(), &id);
    let note = j["source_verification_skipped"]
        .as_str()
        .unwrap_or_else(|| panic!("the skip must be reported: {j}"));
    assert!(
        note.contains(&head),
        "the note must name the reviewed commit: {note}"
    );
    assert!(
        note.contains("source.repo") || note.contains("fetch"),
        "the note must name a remedy: {note}"
    );
    for comment in j["comments"].as_array().unwrap() {
        assert_eq!(comment["resolution"]["state"], "unresolved", "{comment}");
        assert!(
            comment["unresolved_reason"].is_null(),
            "an environmental skip must set no per-comment reason: {comment}"
        );
    }
}

/// Defect 5, write path (AC5). `review comment --path` against a checkout
/// lacking the reviewed head reports the missing COMMIT, not a missing path.
#[test]
fn change_comment_reports_a_missing_head_not_a_missing_path() {
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

    let other = init_independent_repo();
    set_project_source(plan.path(), "demo", &other.path().to_string_lossy());

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
            "fn two_renamed() {}",
            "--body",
            "Should report the commit, not the path.",
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
        text.contains("does not contain the reviewed commit") && text.contains(&head),
        "the missing head must be reported as a missing commit: {text}"
    );
    assert!(
        !text.contains("does not exist at"),
        "the path must not be blamed for a missing commit: {text}"
    );
}

// --- AC5/AC7: `--applied-commit` on a change/<sha> review resolves against
// the project's configured source repo, not the plan repo, and stores the
// resolved full SHA rather than the operator's abbreviated input ---

#[test]
fn change_review_applied_commit_resolves_short_sha_against_source_repo() {
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
            "Needs a follow-up.",
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

    // A short prefix of the SOURCE repo's real HEAD resolves.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "update",
            &id,
            "--comment",
            "1",
            "--status",
            "addressed",
            "--applied-commit",
            &head[..8],
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .success();

    let j = review_json(plan.path(), src.path(), &id);
    assert_eq!(j["comments"][0]["applied_commit"], head.as_str());
}

#[test]
fn change_review_applied_commit_head_resolves_to_full_source_repo_sha() {
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
            "Needs a follow-up.",
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

    // Give the plan repo a HEAD of its own, distinct from the source repo's,
    // so resolving `HEAD` in the wrong repository would be caught.
    git(
        plan.path(),
        &["commit", "--allow-empty", "--no-verify", "-m", "seed plan"],
    );
    let src_head = git_out(src.path(), &["rev-parse", "HEAD"]);
    let plan_head = git_out(plan.path(), &["rev-parse", "HEAD"]);
    assert_eq!(src_head.len(), 40);
    assert_ne!(
        src_head, plan_head,
        "fixture must give the two repos different HEADs"
    );

    // The literal `HEAD` expands in the SOURCE repo for a change review.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "update",
            &id,
            "--comment",
            "1",
            "--status",
            "addressed",
            "--applied-commit",
            "HEAD",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .success();

    let j = review_json(plan.path(), src.path(), &id);
    assert_eq!(j["comments"][0]["applied_commit"], src_head.as_str());
    assert_ne!(j["comments"][0]["applied_commit"], plan_head.as_str());
}

#[test]
fn change_review_applied_commit_refuses_a_sha_only_in_an_unrelated_repo() {
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
            "Needs a follow-up.",
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

    // This SHA is a real commit, but only in a repo unrelated to the
    // project's configured source — proving the check reads the SOURCE
    // repo, not the plan repo (which would also fail to resolve it, for the
    // wrong reason).
    let unrelated = init_unrelated_repo();
    let unrelated_head = git_out(unrelated.path(), &["rev-parse", "HEAD"]);

    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "update",
            &id,
            "--comment",
            "1",
            "--applied-commit",
            &unrelated_head,
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
        text.contains("does not resolve to a commit"),
        "must refuse the unresolvable sha: {text}"
    );
    assert!(
        text.contains(&src.path().to_string_lossy().to_string())
            || text.contains(
                &src.path()
                    .canonicalize()
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            ),
        "must name the source repo checked, not the plan repo: {text}"
    );

    let j = review_json(plan.path(), src.path(), &id);
    assert!(j["comments"][0]["applied_commit"].is_null());
}

/// Rework round 1, item 2 (code review 2026-09-23-1828-5aca, finding
/// `unreachable-repo-reported-as-missing-commit`). When the checked path
/// exists but is not a git checkout at all, `--applied-commit` must say so —
/// not report the SHA as "does not resolve", which would send the operator
/// off to recheck a SHA that was never the problem.
#[test]
fn change_review_applied_commit_refuses_when_source_is_not_a_git_checkout() {
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
            "Needs a follow-up.",
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

    // Reconfigure the project's source to a plain directory: it exists, but
    // it is not a git repository at all.
    let not_a_repo = TempDir::new().unwrap();
    std::fs::write(not_a_repo.path().join("README.md"), "plain dir\n").unwrap();
    set_project_source(plan.path(), "demo", &not_a_repo.path().to_string_lossy());

    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "update",
            &id,
            "--comment",
            "1",
            "--applied-commit",
            "0123456",
            "--project",
            "demo",
        ])
        .current_dir(not_a_repo.path())
        .assert()
        .failure()
        .get_output()
        .stderr
        .clone();
    let text = String::from_utf8_lossy(&out);
    assert!(
        text.contains("not a git checkout"),
        "must name the real cause, not \"does not resolve\": {text}"
    );
    assert!(
        !text.contains("does not resolve to a commit"),
        "an unreachable repo must not be reported as a missing commit: {text}"
    );
}

/// Rework round 1, item 2. A well-formed but unknown abbreviated SHA still
/// gets the "does not resolve" refusal (the checkout IS a real git repo, so
/// this is genuinely a missing-commit case) — but now with a hint that an
/// abbreviated SHA may be ambiguous, since the operator cannot tell "no such
/// commit" apart from "this prefix matches more than one" from the same
/// underlying `Ok(None)`.
#[test]
fn change_review_applied_commit_unknown_short_sha_hints_at_ambiguity() {
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
            "Needs a follow-up.",
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

    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "update",
            &id,
            "--comment",
            "1",
            "--applied-commit",
            "0123456",
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
        text.contains("does not resolve to a commit"),
        "a real git repo with no matching commit is genuinely unresolvable: {text}"
    );
    assert!(
        text.contains("ambiguous") && text.contains("full SHA"),
        "must hint that an abbreviated SHA may be ambiguous: {text}"
    );
}

// --- per-project default_branch feeds the change/ merge-base ---

/// A source repo whose `develop` and `main` diverge: `develop` carries one
/// commit beyond the shared root, `topic` branches from `develop`, and `main`
/// then gains its own commit. The merge-bases of `topic` with `main` and with
/// `develop` therefore differ.
fn init_diverged_source_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    git(p, &["init", "-b", "main"]);
    std::fs::create_dir_all(p.join("src")).unwrap();
    std::fs::write(p.join("src/lib.rs"), BASE_FILE).unwrap();
    git(p, &["add", "."]);
    git(p, &["commit", "-m", "base"]);
    git(p, &["checkout", "-b", "develop"]);
    std::fs::write(p.join("develop.txt"), "develop\n").unwrap();
    git(p, &["add", "."]);
    git(p, &["commit", "-m", "develop work"]);
    git(p, &["checkout", "-b", "topic"]);
    std::fs::write(p.join("src/lib.rs"), HEAD_FILE).unwrap();
    git(p, &["add", "."]);
    git(p, &["commit", "-m", "rename two"]);
    git(p, &["checkout", "main"]);
    std::fs::write(p.join("main.txt"), "main\n").unwrap();
    git(p, &["add", "."]);
    git(p, &["commit", "-m", "main work"]);
    git(p, &["checkout", "topic"]);
    dir
}

fn set_project_default_branch(plan: &Path, project: &str, branch: &str) {
    let path = plan.join("rdm.toml");
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    std::fs::write(
        &path,
        format!("{existing}\n[projects.{project}]\ndefault_branch = \"{branch}\"\n"),
    )
    .unwrap();
}

#[test]
fn change_review_base_follows_the_projects_default_branch() {
    let src = init_diverged_source_repo();
    let plan = init_plan_repo(src.path());
    set_project_default_branch(plan.path(), "demo", "develop");
    create_plan(plan.path(), "design-plan", true);
    let develop_base = git_out(src.path(), &["merge-base", "develop", "topic"]);
    let main_base = git_out(src.path(), &["merge-base", "main", "topic"]);
    assert_ne!(develop_base, main_base, "the fixture branches must diverge");

    let id = start_change_review(
        plan.path(),
        src.path(),
        "change/HEAD",
        &["--implements", "rdm:plan/design-plan"],
    );
    let j = review_json(plan.path(), src.path(), &id);
    assert_eq!(j["target"]["base"], develop_base);
}

#[test]
fn change_review_frontmatter_default_branch_wins_over_the_projects_config() {
    let src = init_diverged_source_repo();
    let plan = init_plan_repo(src.path());
    set_project_default_branch(plan.path(), "demo", "develop");
    create_plan(plan.path(), "design-plan", true);
    let project_path = plan.path().join("projects/demo/project.md");
    let mut project = rdm_core::document::Document::<rdm_core::model::Project>::parse(
        &std::fs::read_to_string(&project_path).unwrap(),
    )
    .unwrap();
    project.frontmatter.source.as_mut().unwrap().default_branch = Some("main".to_string());
    std::fs::write(project_path, project.render().unwrap()).unwrap();
    let main_base = git_out(src.path(), &["merge-base", "main", "topic"]);

    let id = start_change_review(
        plan.path(),
        src.path(),
        "change/HEAD",
        &["--implements", "rdm:plan/design-plan"],
    );
    let j = review_json(plan.path(), src.path(), &id);
    assert_eq!(j["target"]["base"], main_base);
}

// --- A review's commit references become real links (derive on read) ---

/// Runs `rdm <args> --project demo` against `plan` from `cwd`, returning
/// the exit success flag, stdout, and stderr.
fn run_rdm(plan: &Path, cwd: &Path, args: &[&str]) -> (bool, String, String) {
    let out = rdm()
        .arg("--root")
        .arg(plan)
        .args(args)
        .args(["--project", "demo"])
        .current_dir(cwd)
        .output()
        .unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

/// Like [`run_rdm`] with `--format json`, asserting success and parsing
/// stdout.
fn rdm_json(plan: &Path, cwd: &Path, args: &[&str]) -> Value {
    let mut full: Vec<&str> = args.to_vec();
    full.extend(["--format", "json"]);
    let (ok, stdout, stderr) = run_rdm(plan, cwd, &full);
    assert!(ok, "rdm {args:?} failed: {stderr}");
    serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{e}: {stdout}"))
}

/// Writes a review file byte-for-byte into the temp plan repo, in the
/// on-disk format `docs/file-formats.md` documents — for stored values the
/// CLI would refuse to write (a SHA absent from the source repo, a legacy
/// unvalidated `applied_commit`).
fn write_raw_review(plan: &Path, id: &str, yaml: &str) {
    let dir = plan.join("projects/demo/reviews");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{id}.md")), yaml).unwrap();
}

/// A change review started from `topic` (head = topic tip, base = `main`)
/// with one comment, submitted, and that comment addressed with
/// `--applied-commit <main sha>`. Returns the review id.
fn seed_addressed_change_review(plan: &Path, src: &Path, main_sha: &str) -> String {
    create_plan(plan, "design-plan", true);
    let id = start_change_review(
        plan,
        src,
        "change/HEAD",
        &["--implements", "rdm:plan/design-plan"],
    );
    for args in [
        vec![
            "review",
            "comment",
            &id,
            "--path",
            "src/lib.rs",
            "--quote",
            "fn two_renamed() {}",
            "--body",
            "Rename back.",
            "--no-edit",
        ],
        vec![
            "review",
            "submit",
            &id,
            "--verdict",
            "request-changes",
            "--body",
            "Needs work.",
            "--no-edit",
        ],
        vec![
            "review",
            "update",
            &id,
            "--comment",
            "1",
            "--status",
            "addressed",
            "--applied-commit",
            main_sha,
        ],
    ] {
        let (ok, _, stderr) = run_rdm(plan, src, &args);
        assert!(ok, "rdm {args:?} failed: {stderr}");
    }
    id
}

#[test]
fn change_review_commit_refs_are_emitted_as_resolvable_commit_links() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    let topic = git_out(src.path(), &["rev-parse", "topic"]);
    let main = git_out(src.path(), &["rev-parse", "main"]);
    let merge_base = git_out(src.path(), &["merge-base", "main", "topic"]);
    let id = seed_addressed_change_review(plan.path(), src.path(), &main);

    let j = review_json(plan.path(), src.path(), &id);
    assert_eq!(j["head_link"], format!("rdm:src@{topic}"));
    assert_eq!(j["base_link"], format!("rdm:src@{merge_base}"));
    assert_eq!(
        j["comments"][0]["applied_commit_link"],
        format!("rdm:src@{main}")
    );

    for (link, rev) in [
        (j["head_link"].as_str().unwrap(), &topic),
        (j["base_link"].as_str().unwrap(), &merge_base),
        (
            j["comments"][0]["applied_commit_link"].as_str().unwrap(),
            &main,
        ),
    ] {
        let resolved = rdm_json(plan.path(), src.path(), &["link", "resolve", link]);
        assert_eq!(resolved["kind"], "commit", "{resolved}");
        assert_eq!(resolved["rev"], rev.as_str(), "{resolved}");
    }

    // Verified against the seeded source checkout: every one resolves.
    let (ok, stdout, stderr) = run_rdm(
        plan.path(),
        src.path(),
        &["link", "check", "--format", "json"],
    );
    assert!(ok, "link check failed: {stdout}{stderr}");
    let report: Value = serde_json::from_str(&stdout).unwrap();
    assert!(report["path_verification_skipped"].is_null(), "{report}");
    assert_eq!(report["unresolved_commits"], serde_json::json!([]));
    assert!(
        report["commit_links_checked"].as_u64().unwrap() >= 3,
        "{report}"
    );
}

#[test]
fn link_check_reports_a_dangling_review_commit_ref_by_review_and_field_without_failing() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    let head = git_out(src.path(), &["rev-parse", "topic"]);
    let missing_base = "0123456789abcdef0123456789abcdef01234567";
    let missing_fix = "fedcba9876543210fedcba9876543210fedcba98";
    let id = "2026-07-01-1430-dead";
    write_raw_review(
        plan.path(),
        id,
        &format!(
            "---
id: {id}
author: ed
target:
  kind: change
  head: {head}
  base: {missing_base}
state: submitted
verdict: request-changes
created: 2026-07-01T14:30:00Z
comments:
  - id: 1
    status: open
    body: Still open.
  - id: 2
    status: addressed
    applied_commit: {missing_fix}
    body: Fixed elsewhere.
---
Summary.
"
        ),
    );

    let (ok, stdout, stderr) = run_rdm(
        plan.path(),
        src.path(),
        &["link", "check", "--format", "json"],
    );
    assert!(
        ok,
        "an unresolved commit must not fail link check: {stderr}"
    );
    let report: Value = serde_json::from_str(&stdout).unwrap();
    let unresolved = report["unresolved_commits"].as_array().unwrap();
    assert_eq!(
        unresolved,
        &vec![
            serde_json::json!({
                "kind": "review",
                "id": id,
                "comment": null,
                "field": "base",
                "uri": format!("rdm:src@{missing_base}"),
                "rev": missing_base,
            }),
            serde_json::json!({
                "kind": "review",
                "id": id,
                "comment": 2,
                "field": "applied_commit",
                "uri": format!("rdm:src@{missing_fix}"),
                "rev": missing_fix,
            }),
        ],
        "{report}"
    );
    assert!(unresolved.iter().all(|u| u["rev"] != head.as_str()));

    let (ok, stdout, stderr) = run_rdm(plan.path(), src.path(), &["link", "check"]);
    assert!(ok, "human link check must also pass: {stderr}");
    assert!(stdout.contains(&format!("review/{id} [base]")), "{stdout}");
    assert!(
        stdout.contains(&format!("review/{id}#comment-2 [applied_commit]")),
        "{stdout}"
    );
}

#[test]
fn backlinks_to_a_commit_find_the_reviews_recorded_against_it() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    let topic = git_out(src.path(), &["rev-parse", "topic"]);
    let main = git_out(src.path(), &["rev-parse", "main"]);
    let id = seed_addressed_change_review(plan.path(), src.path(), &main);
    let abbrev = &main[..7];
    let task_body = format!("Landed in [base](rdm:src@{abbrev}).");
    let (ok, _, stderr) = run_rdm(
        plan.path(),
        src.path(),
        &[
            "task",
            "create",
            "landed",
            "--title",
            "Landed",
            "--body",
            &task_body,
            "--no-edit",
        ],
    );
    assert!(ok, "{stderr}");

    let head_ref = format!("rdm:src@{topic}");
    let entries = rdm_json(plan.path(), src.path(), &["backlinks", &head_ref]);
    assert_eq!(
        entries,
        serde_json::json!([{
            "kind": "review",
            "id": id,
            "comment": null,
            "found_in": "field",
            "field": "head",
            "link": head_ref,
            "rev": topic,
            "level": "commit",
            "unverified": false,
        }])
    );

    let main_ref = format!("src@{abbrev}");
    let entries = rdm_json(plan.path(), src.path(), &["backlinks", &main_ref]);
    let entries = entries.as_array().unwrap();
    assert!(
        entries
            .iter()
            .any(|e| e["kind"] == "task" && e["slug"] == "landed" && e["range_start"].is_u64()),
        "{entries:?}"
    );
    assert!(
        entries.iter().any(|e| e["kind"] == "review"
            && e["id"] == id.as_str()
            && e["comment"].is_null()
            && e["field"] == "base"),
        "{entries:?}"
    );
    assert!(
        entries.iter().any(|e| e["kind"] == "review"
            && e["id"] == id.as_str()
            && e["comment"] == 1
            && e["field"] == "applied_commit"),
        "{entries:?}"
    );
    assert_eq!(entries.len(), 3, "{entries:?}");

    let (ok, stdout, stderr) = run_rdm(plan.path(), src.path(), &["backlinks", &head_ref]);
    assert!(ok, "{stderr}");
    assert!(stdout.contains(&format!("review/{id}  [head]")), "{stdout}");

    // A file is a backlinks target too: the seeded range comment is on it.
    let entries = rdm_json(
        plan.path(),
        src.path(),
        &["backlinks", "rdm:src/src/lib.rs"],
    );
    assert!(
        entries
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["id"] == id.as_str()
                && e["comment"] == 1
                && e["found_in"] == "anchor"
                && e["level"] == "range"),
        "{entries}"
    );
}

#[test]
fn plan_document_review_applied_commit_is_never_a_source_commit_link() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    let (ok, _, stderr) = run_rdm(
        plan.path(),
        plan.path(),
        &[
            "task",
            "create",
            "fix-login",
            "--title",
            "Fix login",
            "--body",
            "Fix the login flow.",
            "--no-edit",
        ],
    );
    assert!(ok, "{stderr}");
    let (ok, stdout, stderr) = run_rdm(
        plan.path(),
        plan.path(),
        &["review", "start", "--on", "task/fix-login", "--no-edit"],
    );
    assert!(ok, "{stderr}");
    let id = stdout.split('\'').nth(1).unwrap().to_string();
    for args in [
        vec![
            "review",
            "comment",
            &id,
            "--body",
            "Tighten this.",
            "--no-edit",
        ],
        vec![
            "review",
            "submit",
            &id,
            "--verdict",
            "request-changes",
            "--no-edit",
        ],
    ] {
        let (ok, _, stderr) = run_rdm(plan.path(), plan.path(), &args);
        assert!(ok, "rdm {args:?} failed: {stderr}");
    }
    // `--applied-commit HEAD` resolves in the *plan* repo, which needs a
    // commit — a SHA the source repo never has.
    git(plan.path(), &["add", "-A"]);
    git(plan.path(), &["commit", "-m", "seed plan repo"]);
    let (ok, _, stderr) = run_rdm(
        plan.path(),
        plan.path(),
        &[
            "review",
            "update",
            &id,
            "--comment",
            "1",
            "--status",
            "addressed",
            "--applied-commit",
            "HEAD",
        ],
    );
    assert!(ok, "review update failed: {stderr}");

    let j = rdm_json(plan.path(), src.path(), &["review", "show", &id]);
    let sha = j["comments"][0]["applied_commit"]
        .as_str()
        .unwrap_or_else(|| panic!("no applied_commit in {j}"))
        .to_string();
    assert!(j.get("head_link").is_none(), "{j}");
    assert!(j.get("base_link").is_none(), "{j}");
    assert!(j["comments"][0].get("applied_commit_link").is_none(), "{j}");

    let (ok, stdout, stderr) = run_rdm(
        plan.path(),
        src.path(),
        &["link", "check", "--format", "json"],
    );
    assert!(ok, "{stdout}{stderr}");
    let report: Value = serde_json::from_str(&stdout).unwrap();
    assert!(
        report["unresolved_commits"]
            .as_array()
            .unwrap()
            .iter()
            .all(|u| u["rev"] != sha.as_str()),
        "{report}"
    );

    // A commit query resolves its SHA in the source checkout first, and a
    // plan-repo SHA is not a commit there: the query is refused rather than
    // matched against the plan-document review.
    let commit_ref = format!("rdm:src@{sha}");
    let (ok, stdout, stderr) = run_rdm(plan.path(), src.path(), &["backlinks", &commit_ref]);
    assert!(
        !ok,
        "a plan-repo SHA must not answer as a source commit: {stdout}"
    );
    assert!(stderr.contains("does not name a commit"), "{stderr}");
    assert!(!stdout.contains(id.as_str()), "{stdout}");
}

#[test]
fn a_review_written_before_this_phase_still_loads_and_checks() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    let head = git_out(src.path(), &["rev-parse", "topic"]);
    let main = git_out(src.path(), &["rev-parse", "main"]);
    let abbrev = &main[..12];
    let id = "2026-07-01-1430-0ld1";
    // The pre-change format, verbatim: no `base` key, an abbreviated
    // `applied_commit`, and a legacy unvalidated one.
    write_raw_review(
        plan.path(),
        id,
        &format!(
            "---
id: {id}
author: ed
target:
  kind: change
  head: {head}
state: submitted
verdict: request-changes
created: 2026-07-01T14:30:00Z
comments:
  - id: 1
    status: addressed
    applied_commit: {abbrev}
    body: Fixed.
  - id: 2
    status: addressed
    applied_commit: HEAD
    body: Fixed too.
---
Summary.
"
        ),
    );

    let j = review_json(plan.path(), src.path(), id);
    assert_eq!(j["head_link"], format!("rdm:src@{head}"));
    assert!(j.get("base_link").is_none(), "{j}");
    assert_eq!(
        j["comments"][0]["applied_commit_link"],
        format!("rdm:src@{abbrev}")
    );
    assert_eq!(j["comments"][1]["applied_commit"], "HEAD");
    assert!(j["comments"][1].get("applied_commit_link").is_none(), "{j}");

    let (ok, stdout, stderr) = run_rdm(
        plan.path(),
        src.path(),
        &["link", "check", "--format", "json"],
    );
    assert!(ok, "{stdout}{stderr}");
    let report: Value = serde_json::from_str(&stdout).unwrap();
    assert!(report["path_verification_skipped"].is_null(), "{report}");
    assert_eq!(report["unresolved_commits"], serde_json::json!([]));
}

// --- anchor state and intended path (code-as-linkable-target phase 3) ---

/// Runs `rdm review comment <id> <args…> --no-edit --project demo` from the
/// source checkout and returns the raw output, success or not.
fn review_comment(plan: &Path, source: &Path, id: &str, args: &[&str]) -> std::process::Output {
    rdm()
        .arg("--root")
        .arg(plan)
        .args(["review", "comment", id])
        .args(args)
        .args(["--no-edit", "--project", "demo"])
        .current_dir(source)
        .output()
        .unwrap()
}

/// A change review started on the fixture's `topic` tip.
fn fixture_change_review(src: &Path, plan: &Path) -> String {
    create_plan(plan, "design-plan", true);
    start_change_review(
        plan,
        src,
        "change/HEAD",
        &["--implements", "rdm:plan/design-plan"],
    )
}

/// AC1: a comment written with a degradation reason and an intended path
/// round-trips both unchanged through `review show --format json`.
#[test]
fn anchor_state_and_intended_path_round_trip_through_review_show() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    let id = fixture_change_review(src.path(), plan.path());
    let out = review_comment(
        plan.path(),
        src.path(),
        &id,
        &[
            "--anchor-state",
            "path-missing",
            "--intended-path",
            "src/x.rs",
            "--body",
            "The finding named a file this change does not have.",
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let j = review_json(plan.path(), src.path(), &id);
    let c = &j["comments"][0];
    assert_eq!(c["anchor_state"], "path-missing");
    assert_eq!(c["intended_path"], "src/x.rs");
    assert!(c["anchor"].is_null());
}

/// A review file in the format every review had before `anchor_state`
/// existed: one anchorless comment, and neither new key anywhere.
const LEGACY_CHANGE_REVIEW: &str = "---
id: 2026-01-01-0000-lgcy
author: legacy
target:
  kind: change
  head: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
  base: bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
state: submitted
verdict: comment
created: 2026-01-01T00:00:00Z
submitted: 2026-01-01T00:05:00Z
comments:
- id: 1
  status: open
  body: A comment written before anchor_state existed.
---
Legacy review.
";

/// Writes [`LEGACY_CHANGE_REVIEW`] into the plan repo and returns its id.
fn write_legacy_review(plan: &Path) -> String {
    let dir = plan.join("projects/demo/reviews");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("2026-01-01-0000-lgcy.md"), LEGACY_CHANGE_REVIEW).unwrap();
    "2026-01-01-0000-lgcy".to_string()
}

/// AC3: a pre-change review still loads, and its anchorless comment reports
/// `not-recorded` — never `whole-document`.
#[test]
fn a_pre_anchor_state_review_loads_and_reports_not_recorded() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    let id = write_legacy_review(plan.path());
    let j = review_json(plan.path(), src.path(), &id);
    let c = &j["comments"][0];
    assert_eq!(c["anchor_state"], "not-recorded");
    assert!(c.get("intended_path").is_none(), "{c}");
    // Reading never rewrites the file.
    let on_disk = std::fs::read_to_string(
        plan.path()
            .join("projects/demo/reviews")
            .join(format!("{id}.md")),
    )
    .unwrap();
    assert_eq!(on_disk, LEGACY_CHANGE_REVIEW);
}

/// AC2: all four categories side by side — deliberately whole-document, a
/// persist-time reason, a run-time refusal recorded by the fall-back mode
/// (with its intended path), and not recorded (the legacy fixture).
#[test]
fn every_anchor_state_category_is_distinguishable() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    let id = fixture_change_review(src.path(), plan.path());

    // 1. Body only: whole-document on purpose.
    let out = review_comment(plan.path(), src.path(), &id, &["--body", "Overall: fine."]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // 2. A persist-time reason, recorded by the writer.
    let out = review_comment(
        plan.path(),
        src.path(),
        &id,
        &[
            "--anchor-state",
            "outside-hunk",
            "--intended-path",
            "src/lib.rs",
            "--body",
            "Dropped before asking.",
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // 3. A run-time refusal recorded by the fall-back mode.
    let out = review_comment(
        plan.path(),
        src.path(),
        &id,
        &[
            "--path",
            "src/lib.rs",
            "--quote",
            "absent text",
            "--on-anchor-failure",
            "whole-document",
            "--body",
            "Refused at run time.",
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "exactly one JSON line on stdout: {stdout}");
    let line: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(line["review_id"], id.as_str());
    assert_eq!(line["comment_id"], 3);
    assert_eq!(line["anchored"], false);
    assert_eq!(line["anchor_state"], "quote-not-found");
    assert_eq!(line["intended_path"], "src/lib.rs");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("warning: anchor refused (") && stderr.contains("stored whole-document"),
        "the refusal is reported on stderr: {stderr}"
    );

    let j = review_json(plan.path(), src.path(), &id);
    let states: Vec<&Value> = j["comments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| &c["anchor_state"])
        .collect();
    assert_eq!(
        states,
        ["whole-document", "outside-hunk", "quote-not-found"]
    );
    assert!(j["comments"][0].get("intended_path").is_none());
    assert_eq!(j["comments"][1]["intended_path"], "src/lib.rs");
    assert_eq!(j["comments"][2]["intended_path"], "src/lib.rs");

    // 4. Not recorded — a review in the pre-change format.
    let legacy = write_legacy_review(plan.path());
    let j = review_json(plan.path(), src.path(), &legacy);
    assert_eq!(j["comments"][0]["anchor_state"], "not-recorded");
}

/// The fall-back mode anchors normally when core accepts the anchor, and
/// its JSON line says so.
#[test]
fn fall_back_mode_reports_an_anchored_comment() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    let id = fixture_change_review(src.path(), plan.path());
    let out = review_comment(
        plan.path(),
        src.path(),
        &id,
        &[
            "--path",
            "src/lib.rs",
            "--quote",
            "fn two_renamed() {}",
            "--on-anchor-failure",
            "whole-document",
            "--body",
            "Anchors fine.",
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let line: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        line,
        serde_json::json!({ "review_id": id, "comment_id": 1, "anchored": true })
    );
    let j = review_json(plan.path(), src.path(), &id);
    assert_eq!(j["comments"][0]["anchor"]["anchor_type"], "file-quote");
    assert!(j["comments"][0].get("anchor_state").is_none());
}

/// An unknown `--anchor-state` is rejected, naming every accepted value.
#[test]
fn an_unknown_anchor_state_is_rejected_listing_every_accepted_value() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    let id = fixture_change_review(src.path(), plan.path());
    let out = review_comment(
        plan.path(),
        src.path(),
        &id,
        &["--anchor-state", "bogus", "--body", "x"],
    );
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    for state in rdm_core::model::AnchorState::KNOWN {
        assert!(
            stderr.contains(state.as_str()),
            "stderr omits {}: {stderr}",
            state.as_str()
        );
    }
    let j = review_json(plan.path(), src.path(), &id);
    assert!(j["comments"].as_array().unwrap().is_empty());
}

// --- reverse lookup from a code location (code-as-linkable-target phase 4) ---

/// Task A's body: a floating one-line link to `src/lib.rs`.
const TASK_A_BODY: &str = "Line one: [l1](rdm:src/src/lib.rs#L1).";

/// Creates task `slug` with `body` from the source checkout.
fn create_task_with_body(plan: &Path, src: &Path, slug: &str, body: &str) {
    let (ok, _, stderr) = run_rdm(
        plan,
        src,
        &[
            "task",
            "create",
            slug,
            "--title",
            slug,
            "--body",
            body,
            "--no-edit",
        ],
    );
    assert!(ok, "task create {slug} failed: {stderr}");
}

/// A change review on `topic` with four comments — (1) a range on line 2,
/// (2) a real `outside-hunk` refusal on `src/lib.rs` (the file exists at
/// head), (3) a whole-document comment, (4) an intended path that does not
/// exist — plus task `a-floating` linking `rdm:src/src/lib.rs#L1` and task
/// `b-elsewhere` linking another path. Returns the review id.
fn seed_reverse_lookup(plan: &Path, src: &Path) -> String {
    let id = fixture_change_review(src, plan);
    let comments: [&[&str]; 4] = [
        &[
            "--path",
            "src/lib.rs",
            "--quote",
            "fn two_renamed() {}",
            "--body",
            "Range.",
        ],
        &[
            "--path",
            "src/lib.rs",
            "--quote",
            "fn one() {}",
            "--on-anchor-failure",
            "whole-document",
            "--body",
            "Outside the hunk.",
        ],
        &["--body", "Overall."],
        &[
            "--anchor-state",
            "path-missing",
            "--intended-path",
            "src/gone.rs",
            "--body",
            "Gone.",
        ],
    ];
    for args in comments {
        let out = review_comment(plan, src, &id, args);
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    create_task_with_body(plan, src, "a-floating", TASK_A_BODY);
    create_task_with_body(
        plan,
        src,
        "b-elsewhere",
        "Other: [x](rdm:src/src/other.rs#L1).",
    );
    id
}

/// Each entry's identity: a task's slug, or `comment-<n>` for a review
/// comment.
fn entry_keys(entries: &Value) -> Vec<String> {
    entries
        .as_array()
        .unwrap()
        .iter()
        .map(|e| match e["kind"].as_str().unwrap() {
            "task" => e["slug"].as_str().unwrap().to_string(),
            _ => format!("comment-{}", e["comment"]),
        })
        .collect()
}

/// AC1 + AC2: a path query returns every comment and document anchored to
/// the file — including the failed-anchor comment as a file-level match —
/// each with a resolvable link; the commit-level comments answer only the
/// commit query, although the change's diff touches the file.
#[test]
fn backlinks_on_a_source_path_returns_its_comments_and_documents() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    let topic = git_out(src.path(), &["rev-parse", "topic"]);
    let id = seed_reverse_lookup(plan.path(), src.path());

    let entries = rdm_json(
        plan.path(),
        src.path(),
        &["backlinks", "rdm:src/src/lib.rs"],
    );
    let link_start = TASK_A_BODY.find('[').unwrap();
    let link_end = TASK_A_BODY.rfind(')').unwrap() + 1;
    assert_eq!(
        entries,
        serde_json::json!([
            {
                "kind": "task",
                "slug": "a-floating",
                "found_in": "body",
                "range_start": link_start,
                "range_end": link_end,
                "link": "rdm:src/src/lib.rs#L1",
                "path": "src/lib.rs",
                "lines": { "start": 1, "end": 1 },
                "level": "range",
                "unverified": false,
            },
            {
                "kind": "review",
                "id": id,
                "comment": 1,
                "found_in": "anchor",
                "link": format!("rdm:src/src/lib.rs@{topic}#L2"),
                "path": "src/lib.rs",
                "rev": topic,
                "lines": { "start": 2, "end": 2 },
                "level": "range",
                "unverified": false,
            },
            {
                "kind": "review",
                "id": id,
                "comment": 2,
                "found_in": "anchor",
                "link": format!("rdm:src/src/lib.rs@{topic}"),
                "path": "src/lib.rs",
                "rev": topic,
                "level": "file",
                "reason": "outside-hunk",
                "unverified": false,
            },
        ])
    );
    for entry in entries.as_array().unwrap() {
        let link = entry["link"].as_str().unwrap();
        let (ok, _, stderr) = run_rdm(
            plan.path(),
            src.path(),
            &["link", "resolve", link, "--format", "json"],
        );
        assert!(ok, "link resolve {link} failed: {stderr}");
    }

    let head_ref = format!("src@{topic}");
    let at_commit = rdm_json(plan.path(), src.path(), &["backlinks", &head_ref]);
    assert_eq!(
        at_commit,
        serde_json::json!([
            {
                "kind": "review",
                "id": id,
                "comment": null,
                "found_in": "field",
                "field": "head",
                "link": format!("rdm:src@{topic}"),
                "rev": topic,
                "level": "commit",
                "unverified": false,
            },
            {
                "kind": "review",
                "id": id,
                "comment": 3,
                "found_in": "anchor",
                "link": format!("rdm:src@{topic}"),
                "rev": topic,
                "level": "commit",
                "reason": "whole-document",
                "unverified": false,
            },
            {
                "kind": "review",
                "id": id,
                "comment": 4,
                "found_in": "anchor",
                "link": format!("rdm:src@{topic}"),
                "rev": topic,
                "level": "commit",
                "reason": "path-missing",
                "unverified": false,
            },
        ])
    );

    let (ok, stdout, stderr) = run_rdm(
        plan.path(),
        src.path(),
        &["backlinks", "rdm:src/src/lib.rs"],
    );
    assert!(ok, "{stderr}");
    assert!(
        stdout.contains(&format!(
            "review/{id}#comment-2  [anchor]  rdm:src/src/lib.rs@{topic}  (file: outside-hunk)"
        )),
        "{stdout}"
    );
    assert!(
        stdout.contains("task/a-floating  rdm:src/src/lib.rs#L1  (range)"),
        "{stdout}"
    );
}

/// AC3: narrowing by revision and by line range changes the result set as
/// documented.
#[test]
fn backlinks_narrow_by_revision_and_line_range() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    let topic = git_out(src.path(), &["rev-parse", "topic"]);
    seed_reverse_lookup(plan.path(), src.path());
    create_task_with_body(
        plan.path(),
        src.path(),
        "c-main",
        "At main: [l3](rdm:src/src/lib.rs@main#L3).",
    );

    let keys = |reference: &str| {
        entry_keys(&rdm_json(
            plan.path(),
            src.path(),
            &["backlinks", reference],
        ))
    };
    let all = ["a-floating", "c-main", "comment-1", "comment-2"];
    assert_eq!(keys("rdm:src/src/lib.rs"), all);
    assert_eq!(
        keys(&format!("rdm:src/src/lib.rs@{topic}")),
        ["comment-1", "comment-2"]
    );
    // A branch name is resolved in the source checkout.
    assert_eq!(keys("rdm:src/src/lib.rs@main"), ["c-main"]);
    assert_eq!(keys("rdm:src/src/lib.rs#L2"), ["comment-1", "comment-2"]);
    assert_eq!(keys("rdm:src/src/lib.rs#L1"), ["a-floating", "comment-2"]);
    assert_eq!(keys("rdm:src/src/lib.rs#L1-L3"), all);
}

/// AC5: a file with no annotations is an empty answer and exit 0, distinct
/// from a path that does not exist, a directory, and an unknown revision.
#[test]
fn backlinks_on_a_path_distinguishes_empty_from_missing() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());

    let entries = rdm_json(
        plan.path(),
        src.path(),
        &["backlinks", "rdm:src/src/lib.rs"],
    );
    assert_eq!(entries, serde_json::json!([]));
    let (ok, stdout, stderr) = run_rdm(
        plan.path(),
        src.path(),
        &["backlinks", "rdm:src/src/lib.rs"],
    );
    assert!(ok, "{stderr}");
    assert!(stdout.contains("no annotations found"), "{stdout}");

    for (reference, expected) in [
        ("rdm:src/src/nope.rs", "does not exist"),
        ("rdm:src/src", "is a directory"),
        (
            "rdm:src/src/lib.rs@0123456789abcdef0123456789abcdef01234567",
            "does not name a commit",
        ),
    ] {
        let (ok, _, stderr) = run_rdm(plan.path(), src.path(), &["backlinks", reference]);
        assert!(!ok, "{reference} must fail");
        assert!(stderr.contains(expected), "{reference}: {stderr}");
    }
}

// --- every comment carries a link (code-as-linkable-target phase 6) ---

/// Adds a comment through [`review_comment`], asserting it succeeded.
fn add_comment_ok(plan: &Path, source: &Path, id: &str, args: &[&str]) {
    let out = review_comment(plan, source, id, args);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The three anchorless comments the fail-open tests share: whole-document,
/// `path-missing` with no intended path, and `outside-hunk` about
/// `src/lib.rs` (a file that exists at the head).
fn add_degraded_comments(plan: &Path, source: &Path, id: &str) {
    add_comment_ok(plan, source, id, &["--body", "Overall: fine."]);
    add_comment_ok(
        plan,
        source,
        id,
        &[
            "--anchor-state",
            "path-missing",
            "--body",
            "Named a file the change does not have.",
        ],
    );
    add_comment_ok(
        plan,
        source,
        id,
        &[
            "--anchor-state",
            "outside-hunk",
            "--intended-path",
            "src/lib.rs",
            "--body",
            "Outside the diff.",
        ],
    );
}

/// The fail-open targets [`add_degraded_comments`]' comments carry when the
/// source cannot answer.
fn assert_degraded_targets_fail_open(j: &Value, head: &str) {
    assert!(
        j["source_verification_skipped"].is_string(),
        "the skip must be reported: {j}"
    );
    let c = j["comments"].as_array().unwrap();
    let whole = &c[0]["target"];
    assert_eq!(whole["level"], "commit", "{whole}");
    assert_eq!(whole["reason"], "whole-document", "{whole}");
    let missing = &c[1]["target"];
    assert_eq!(missing["level"], "commit", "{missing}");
    assert_eq!(missing["link"], format!("rdm:src@{head}"));
    let file = &c[2]["target"];
    assert_eq!(file["level"], "file", "{file}");
    assert_eq!(file["unverified"], true, "{file}");
    assert_eq!(file["link"], format!("rdm:src/src/lib.rs@{head}"));
}

/// AC1, AC2, AC4. Every comment of a change review carries a `target` whose
/// link `rdm link resolve` accepts — the anchored one, the whole-document one,
/// and each degraded one — and a whole-document comment differs from a lost
/// anchor by `target.reason`. `source_link` keeps its range-only meaning.
#[test]
fn every_comment_carries_a_target_link() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    let id = fixture_change_review(src.path(), plan.path());
    let head = git_out(src.path(), &["rev-parse", "HEAD"]);
    let head_link = format!("rdm:src@{head}");

    add_comment_ok(
        plan.path(),
        src.path(),
        &id,
        &[
            "--path",
            "src/lib.rs",
            "--quote",
            "fn two_renamed() {}",
            "--body",
            "Anchored.",
        ],
    );
    add_comment_ok(plan.path(), src.path(), &id, &["--body", "Overall: fine."]);
    add_comment_ok(
        plan.path(),
        src.path(),
        &id,
        &[
            "--anchor-state",
            "path-missing",
            "--body",
            "Named a file the change does not have.",
        ],
    );
    add_comment_ok(
        plan.path(),
        src.path(),
        &id,
        &[
            "--anchor-state",
            "outside-hunk",
            "--intended-path",
            "src/lib.rs",
            "--body",
            "Outside the diff.",
        ],
    );
    add_comment_ok(
        plan.path(),
        src.path(),
        &id,
        &[
            "--anchor-state",
            "outside-hunk",
            "--intended-path",
            "src/gone.rs",
            "--body",
            "About a file the head does not have.",
        ],
    );

    let j = review_json(plan.path(), src.path(), &id);
    let c = j["comments"].as_array().unwrap();
    assert_eq!(c.len(), 5, "{j}");

    let range = &c[0]["target"];
    assert_eq!(range["level"], "range", "{range}");
    assert_eq!(range["link"], c[0]["source_link"]);
    assert!(range.get("reason").is_none(), "{range}");

    let whole = &c[1]["target"];
    assert_eq!(whole["level"], "commit", "{whole}");
    assert_eq!(whole["link"], head_link);
    assert_eq!(whole["reason"], "whole-document");

    let missing = &c[2]["target"];
    assert_eq!(missing["level"], "commit", "{missing}");
    assert_eq!(missing["link"], head_link);
    assert_eq!(missing["reason"], "path-missing");
    // AC2: an explicit field, not a null, tells them apart.
    assert_ne!(whole["reason"], missing["reason"]);

    let file = &c[3]["target"];
    assert_eq!(file["level"], "file", "{file}");
    assert_eq!(file["link"], format!("rdm:src/src/lib.rs@{head}"));
    assert_eq!(file["unverified"], false, "{file}");
    assert_eq!(file["reason"], "outside-hunk");

    assert_eq!(c[4]["target"]["level"], "commit", "{}", c[4]);

    for comment in c {
        let link = comment["target"]["link"]
            .as_str()
            .unwrap_or_else(|| panic!("every comment carries a link: {comment}"));
        let (ok, _, stderr) = run_rdm(
            plan.path(),
            src.path(),
            &["link", "resolve", link, "--format", "json"],
        );
        assert!(ok, "rdm link resolve {link} failed: {stderr}");
    }
    let with_source_link: Vec<bool> = c.iter().map(|x| x.get("source_link").is_some()).collect();
    assert_eq!(with_source_link, [true, false, false, false, false]);

    // AC4, legacy: an anchorless comment from before anchor_state existed.
    let legacy = write_legacy_review(plan.path());
    let j = review_json(plan.path(), src.path(), &legacy);
    let t = &j["comments"][0]["target"];
    assert_eq!(t["level"], "commit", "{t}");
    assert_eq!(t["link"], format!("rdm:src@{}", "a".repeat(40)));
    assert_eq!(t["reason"], "not-recorded");
}

/// AC3, no source. With the project's `source.repo` pointing nowhere and no
/// checkout to fall back on, a failed-anchor comment with an intended path
/// still links to that file, marked unverified; the others link to the commit.
#[test]
fn comment_targets_fail_open_without_a_source_repo() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    let id = fixture_change_review(src.path(), plan.path());
    let head = git_out(src.path(), &["rev-parse", "HEAD"]);
    add_degraded_comments(plan.path(), src.path(), &id);

    set_project_source(plan.path(), "demo", "/nonexistent/source/repo");
    let elsewhere = TempDir::new().unwrap();
    let j = review_json(plan.path(), elsewhere.path(), &id);
    assert_degraded_targets_fail_open(&j, &head);
}

/// AC3, head missing. The configured source opens but has independent
/// history, so the reviewed head does not resolve: the same fail-open targets,
/// and the note names the head.
#[test]
fn comment_targets_fail_open_when_the_head_does_not_resolve() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    let id = fixture_change_review(src.path(), plan.path());
    let head = git_out(src.path(), &["rev-parse", "HEAD"]);
    add_degraded_comments(plan.path(), src.path(), &id);

    let other = init_independent_repo();
    set_project_source(plan.path(), "demo", &other.path().to_string_lossy());
    let j = review_json(plan.path(), other.path(), &id);
    assert_degraded_targets_fail_open(&j, &head);
    let note = j["source_verification_skipped"].as_str().unwrap();
    assert!(note.contains(&head), "the note must name the head: {note}");
}

/// A quote that occurs exactly once in a file the change never touched is
/// refused as `outside-hunk` — the benign cause — not `quote-not-found`, and
/// the fall-back comment keeps the file as its intended path and file-level
/// target. A quote absent from that file stays `quote-not-found`.
#[test]
fn a_unique_quote_in_a_file_the_change_never_touched_is_outside_hunk() {
    let src = init_source_repo();
    let p = src.path();
    std::fs::write(p.join("src/untouched.rs"), "fn only_here() {}\n").unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-m", "add untouched"]);
    // The base already holds `src/untouched.rs`; only `src/lib.rs` changes.
    let base = git_out(p, &["rev-parse", "HEAD"]);
    std::fs::write(
        p.join("src/lib.rs"),
        "fn one() {}\nfn two_renamed_again() {}\nfn three() {}\n",
    )
    .unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-m", "edit lib only"]);
    let head = git_out(p, &["rev-parse", "HEAD"]);

    let plan = init_plan_repo(p);
    create_plan(plan.path(), "design-plan", true);
    let id = start_change_review(
        plan.path(),
        p,
        "change/HEAD",
        &["--base", &base, "--implements", "rdm:plan/design-plan"],
    );

    let fall_back = |quote: &str| -> Value {
        let out = review_comment(
            plan.path(),
            p,
            &id,
            &[
                "--path",
                "src/untouched.rs",
                "--quote",
                quote,
                "--on-anchor-failure",
                "whole-document",
                "--body",
                "You missed this file.",
            ],
        );
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    };

    let line = fall_back("fn only_here() {}");
    assert_eq!(line["anchored"], false, "{line}");
    assert_eq!(line["anchor_state"], "outside-hunk", "{line}");
    assert_eq!(line["intended_path"], "src/untouched.rs", "{line}");

    let line = fall_back("fn not_here() {}");
    assert_eq!(line["anchored"], false, "{line}");
    assert_eq!(line["anchor_state"], "quote-not-found", "{line}");

    let j = review_json(plan.path(), p, &id);
    let target = &j["comments"][0]["target"];
    assert_eq!(target["level"], "file", "{target}");
    assert_eq!(target["link"], format!("rdm:src/src/untouched.rs@{head}"));
}

/// `--line` anchors to plan-repo document lines, so a change review refuses
/// it in both anchor-failure modes — it is a usage error, never stored
/// whole-document — and writes no comment.
#[test]
fn change_comment_refuses_a_line_anchor_in_both_modes() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    create_plan(plan.path(), "design-plan", true);
    let id = start_change_review(
        plan.path(),
        src.path(),
        "change/HEAD",
        &["--implements", "rdm:plan/design-plan"],
    );
    for extra in [&[][..], &["--on-anchor-failure", "whole-document"][..]] {
        let out = rdm()
            .arg("--root")
            .arg(plan.path())
            .args([
                "review",
                "comment",
                &id,
                "--line",
                "2",
                "--body",
                "x",
                "--no-edit",
                "--project",
                "demo",
            ])
            .args(extra)
            .current_dir(src.path())
            .assert()
            .failure()
            .get_output()
            .stderr
            .clone();
        let stderr = String::from_utf8_lossy(&out);
        assert!(
            stderr.contains("--line only applies to a review of a plan-repo document"),
            "{extra:?}: {stderr}"
        );
    }
    let j = review_json(plan.path(), src.path(), &id);
    assert!(j["comments"].as_array().unwrap().is_empty(), "{j}");
}
