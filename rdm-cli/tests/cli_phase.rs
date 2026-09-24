use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

#[path = "git_test_support.rs"]
mod git_test_support;
use git_test_support::git;

fn rdm() -> Command {
    let mut cmd = Command::cargo_bin("rdm").unwrap();
    // Isolate from host global config (e.g. default_format = "json").
    cmd.env("XDG_CONFIG_HOME", "/dev/null/nonexistent");
    cmd
}

fn create_phase(dir: &TempDir, slug: &str, title: &str) {
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            slug,
            "--title",
            title,
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success();
}

fn init_with_roadmap(dir: &TempDir) {
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
            "roadmap",
            "create",
            "two-way",
            "--title",
            "Two-Way Players",
            "--project",
            "fbm",
        ])
        .assert()
        .success();
}

/// A temp source repo whose git root is a **subdirectory** of the `TempDir`.
///
/// `rdm worktree add` places a new worktree at
/// `repo_root.parent()/<name>__worktrees/<item>` — a sibling of the repo
/// root. If the repo root were the `TempDir` itself that sibling would land
/// in the system temp directory, where `TempDir::drop` never reaches it.
/// Rooting the repo one level down keeps the sibling inside the `TempDir`.
struct SourceRepo {
    _dir: TempDir,
    root: std::path::PathBuf,
}

impl SourceRepo {
    fn path(&self) -> &Path {
        &self.root
    }
}

/// A source repo on `main` with one commit — enough for `git rev-parse
/// --show-toplevel` and `rdm worktree add` to work, with nothing else
/// configured (no project `source` binding; resolution goes through the
/// caller's cwd, exactly as `cli_gate.rs`'s fixtures do).
fn init_source_repo() -> SourceRepo {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("repo");
    fs::create_dir_all(&root).unwrap();
    let p = root.as_path();
    git(p, &["init", "-b", "main"]);
    fs::write(p.join("README.md"), "# source\n").unwrap();
    git(p, &["add", "."]);
    git(p, &["commit", "-m", "initial"]);
    SourceRepo { _dir: dir, root }
}

fn phase_estimate_snapshot(dir: &TempDir) -> String {
    let output = rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "phase-1-core",
            "--roadmap",
            "two-way",
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
    serde_json::from_slice::<serde_json::Value>(&output).unwrap()["estimate_snapshot"]
        .as_str()
        .expect("phase show exposes conditional estimate snapshot")
        .to_owned()
}

#[test]
fn conditional_estimate_rejects_stale_snapshot_without_overwriting_concurrent_edit() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core");
    let snapshot = phase_estimate_snapshot(&dir);
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--body",
            "Concurrent body",
            "--tags",
            "concurrent",
            "--difficulty",
            "hard",
        ])
        .assert()
        .success();
    let phase = dir
        .path()
        .join("projects/fbm/roadmaps/two-way/phase-1-core.md");
    let before = fs::read_to_string(&phase).unwrap();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--body",
            "Stale body and estimate",
            "--difficulty",
            "easy",
            "--expected-estimate-snapshot",
            &snapshot,
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("conditional estimate refused"));
    assert_eq!(fs::read_to_string(phase).unwrap(), before);
}

#[test]
fn conditional_estimate_applies_once_and_requires_unset_estimate() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core");
    let snapshot = phase_estimate_snapshot(&dir);
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--body",
            "Estimate body",
            "--difficulty",
            "easy",
            "--expected-estimate-snapshot",
            &snapshot,
        ])
        .assert()
        .success();
    let fresh = phase_estimate_snapshot(&dir);
    assert_ne!(snapshot, fresh);
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--difficulty",
            "hard",
            "--expected-estimate-snapshot",
            &fresh,
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("conditional estimate refused"));
}

#[test]
fn phase_create_auto_number() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            "core",
            "--title",
            "Core Valuation",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Created phase 'phase-1-core'"));

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            "service",
            "--title",
            "Keeper Service",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Created phase 'phase-2-service'"));
}

#[test]
fn phase_create_explicit_number() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            "core",
            "--title",
            "Core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--number",
            "5",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("phase-5-core"));
}

#[test]
fn phase_show() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            "core",
            "--title",
            "Core Valuation",
            "--roadmap",
            "two-way",
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
            "show",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Phase 1: Core Valuation")
                .and(predicate::str::contains("Status: not-started")),
        );
}

#[test]
fn phase_update_to_done() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            "core",
            "--title",
            "Core",
            "--roadmap",
            "two-way",
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
            "update",
            "phase-1-core",
            "--status",
            "done",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Updated 'phase-1-core' → done"));

    // Verify completed date is set
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Status: done").and(predicate::str::contains("Completed:")),
        );
}

#[test]
fn phase_update_done_then_back() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            "core",
            "--title",
            "Core",
            "--roadmap",
            "two-way",
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
            "update",
            "phase-1-core",
            "--status",
            "done",
            "--roadmap",
            "two-way",
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
            "update",
            "phase-1-core",
            "--status",
            "in-progress",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("in-progress"));

    // Verify completed date is cleared
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Status: in-progress")
                .and(predicate::str::contains("Completed:").not()),
        );
}

#[test]
fn phase_list() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");
    create_phase(&dir, "service", "Keeper Service");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["phase", "list", "--roadmap", "two-way", "--project", "fbm"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("| # | Phase | Status | Difficulty | Model | Stem |")
                .and(predicate::str::contains(
                    "| 1 | Core Valuation | not-started | - | - | phase-1-core |",
                ))
                .and(predicate::str::contains(
                    "| 2 | Keeper Service | not-started | - | - | phase-2-service |",
                )),
        );
}

#[test]
fn phase_create_with_difficulty_and_model() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            "core",
            "--title",
            "Core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--difficulty",
            "hard",
            "--model",
            "large",
            "--no-edit",
        ])
        .assert()
        .success();

    // Human show reflects both
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Difficulty: hard")
                .and(predicate::str::contains("Model: large")),
        );

    // JSON show reflects both
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--format",
            "json",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("\"difficulty\": \"hard\"")
                .and(predicate::str::contains("\"model\": \"large\"")),
        );
}

#[test]
fn phase_update_sets_and_clears_difficulty_and_model() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");

    // Set both
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--difficulty",
            "easy",
            "--model",
            "small",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--no-edit",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Difficulty: easy")
                .and(predicate::str::contains("Model: small")),
        );

    // Clear both
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--clear-difficulty",
            "--clear-model",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--no-edit",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Difficulty:")
                .not()
                .and(predicate::str::contains("Model:").not()),
        );
}

#[test]
fn phase_update_repeated_difficulty_change_rederives_stale_model() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");

    // Difficulty-only update derives "small" from "easy".
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--difficulty",
            "easy",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--no-edit",
        ])
        .assert()
        .success();

    // A later difficulty-only update (still no --model) must re-derive the
    // stale "small" tier to "large" rather than stranding it.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--difficulty",
            "hard",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--no-edit",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--format",
            "json",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"model\": \"large\""));
}

#[test]
fn phase_update_repeated_difficulty_change_preserves_explicit_model() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");

    // Explicit model, no difficulty set yet.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--model",
            "small",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--no-edit",
        ])
        .assert()
        .success();

    // Difficulty-only update must not clobber the explicitly chosen model.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--difficulty",
            "hard",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--no-edit",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--format",
            "json",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"model\": \"small\""));
}

#[test]
fn phase_update_status_and_estimate_in_one_invocation() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");

    // Status + estimate in a single invocation (the consolidated single-write
    // path). stdout still reports the status transition unchanged.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--status",
            "in-progress",
            "--difficulty",
            "hard",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--no-edit",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Updated 'phase-1-core' → in-progress",
        ));

    // Both the status change and the estimate (hard → derived large) landed.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Status: in-progress")
                .and(predicate::str::contains("Difficulty: hard"))
                .and(predicate::str::contains("Model: large")),
        );
}

#[test]
fn phase_update_difficulty_conflicts_with_clear() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--difficulty",
            "hard",
            "--clear-difficulty",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--no-edit",
        ])
        .assert()
        .failure();
}

#[test]
fn phase_update_model_conflicts_with_clear() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--model",
            "large",
            "--clear-model",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--no-edit",
        ])
        .assert()
        .failure();
}

#[test]
fn phase_list_shows_difficulty_and_model_columns() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            "core",
            "--title",
            "Core Valuation",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--difficulty",
            "moderate",
            "--model",
            "medium",
            "--no-edit",
        ])
        .assert()
        .success();

    // Human list shows the populated columns
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["phase", "list", "--roadmap", "two-way", "--project", "fbm"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("| # | Phase | Status | Difficulty | Model | Stem |").and(
                predicate::str::contains(
                    "| 1 | Core Valuation | not-started | moderate | medium | phase-1-core |",
                ),
            ),
        );

    // JSON list carries the fields
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "list",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--format",
            "json",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("\"difficulty\": \"moderate\"")
                .and(predicate::str::contains("\"model\": \"medium\"")),
        );
}

#[test]
fn phase_list_empty() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["phase", "list", "--roadmap", "two-way", "--project", "fbm"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No phases yet."));
}

#[test]
fn phase_show_by_number() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "1",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Phase 1: Core Valuation")
                .and(predicate::str::contains("Stem: phase-1-core")),
        );
}

#[test]
fn phase_show_by_number_not_found() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "99",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("phase not found: 99"));
}

#[test]
fn phase_update_by_number() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "1",
            "--status",
            "done",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Updated 'phase-1-core' → done"));
}

#[test]
fn phase_create_with_body_flag() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            "core",
            "--title",
            "Core Valuation",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--body",
            "Phase description here.",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Phase description here."));
}

#[test]
fn phase_update_with_body_flag() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--status",
            "in-progress",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--body",
            "Updated body content.",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Updated body content."));
}

#[test]
fn phase_create_with_stdin_pipe() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            "core",
            "--title",
            "Core Valuation",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .write_stdin("piped phase content")
        .assert()
        .success();

    let phase_file = dir
        .path()
        .join("projects/fbm/roadmaps/two-way/phase-1-core.md");
    let content = fs::read_to_string(&phase_file).unwrap();
    assert!(
        content.contains("piped phase content"),
        "expected piped content in file, got: {content}"
    );
}

#[test]
fn phase_remove_by_stem() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");
    create_phase(&dir, "service", "Keeper Service");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "remove",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed phase 'phase-1-core'"));

    // Verify it no longer appears in phase list
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["phase", "list", "--roadmap", "two-way", "--project", "fbm"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Keeper Service")
                .and(predicate::str::contains("Core Valuation").not()),
        );
}

#[test]
fn phase_remove_by_number() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "remove",
            "1",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed phase 'phase-1-core'"));

    // Verify phase list is empty
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["phase", "list", "--roadmap", "two-way", "--project", "fbm"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No phases yet."));
}

#[test]
fn phase_remove_not_found() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "remove",
            "phase-99-nope",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("phase not found"));
}

#[test]
fn phase_show_body_and_no_body() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");

    // Append body text to the phase file
    let phase_file = dir
        .path()
        .join("projects/fbm/roadmaps/two-way/phase-1-core.md");
    let content = fs::read_to_string(&phase_file).unwrap();
    fs::write(
        &phase_file,
        format!("{content}\n## Details\n\nPhase body content.\n"),
    )
    .unwrap();

    // show includes body
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Phase 1: Core Valuation")
                .and(predicate::str::contains("Phase body content.")),
        );

    // show --no-body suppresses body
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--no-body",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Phase 1: Core Valuation")
                .and(predicate::str::contains("Phase body content.").not()),
        );
}

#[test]
fn phase_create_no_edit_skips_editor() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            "no-edit",
            "--title",
            "No Edit Phase",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--no-edit",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Created phase"));
}

#[test]
fn phase_update_no_edit_skips_editor() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "1",
            "--status",
            "in-progress",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--no-edit",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("in-progress"));
}

#[test]
fn phase_update_without_status_preserves_existing() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");

    // First set status to in-progress
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "1",
            "--status",
            "in-progress",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--no-edit",
        ])
        .assert()
        .success();

    // Update body only, without --status
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "1",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--body",
            "New body content.",
            "--no-edit",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("in-progress"));

    // Verify body was updated and status preserved
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "1",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("New body content."))
        .stdout(predicate::str::contains("in-progress"));
}

#[test]
fn phase_update_without_status_and_without_body() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");

    // Update with neither --status nor --body should succeed (no-op)
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "1",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--no-edit",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("not-started"));
}

#[test]
fn phase_show_includes_navigation_hints() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");
    create_phase(&dir, "service", "Keeper Service");
    create_phase(&dir, "ui", "UI Layer");

    // Middle phase has both prev and next
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "phase-2-service",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Prev: rdm phase show phase-1-core")
                .and(predicate::str::contains("Next: rdm phase show phase-3-ui")),
        );
}

#[test]
fn phase_show_first_phase_no_prev() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");
    create_phase(&dir, "service", "Keeper Service");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Next: rdm phase show phase-2-service")
                .and(predicate::str::contains("Prev:").not()),
        );
}

#[test]
fn phase_update_done_to_done_updates_commit() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");

    // Mark done with --commit abc
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "1",
            "--status",
            "done",
            "--commit",
            "abc123",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--no-edit",
        ])
        .assert()
        .success();

    // Mark done again with --commit def
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "1",
            "--status",
            "done",
            "--commit",
            "def456",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--no-edit",
        ])
        .assert()
        .success();

    // Verify show output has def456
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "1",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("def456"));
}

#[test]
fn phase_update_done_to_done_no_commit_preserves() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");

    // Mark done with --commit abc
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "1",
            "--status",
            "done",
            "--commit",
            "abc123",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--no-edit",
        ])
        .assert()
        .success();

    // Mark done again without --commit
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "1",
            "--status",
            "done",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--no-edit",
        ])
        .assert()
        .success();

    // Verify abc123 is preserved
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "1",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("abc123"));
}

#[test]
fn phase_show_last_phase_no_next() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");
    create_phase(&dir, "service", "Keeper Service");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "phase-2-service",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Prev: rdm phase show phase-1-core")
                .and(predicate::str::contains("Next:").not()),
        );
}

fn head_sha(dir: &std::path::Path) -> String {
    let repo = gix::open(dir).unwrap();
    let mut head = repo.head().unwrap();
    head.peel_to_commit().unwrap().id.to_string()
}

#[test]
fn phase_show_at_revision_returns_historical_body() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            "core",
            "--title",
            "Core Valuation",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--body",
            "original-phase-body",
            "--no-edit",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["commit", "-m", "seed: create phase with original body"])
        .assert()
        .success();

    let old_sha = head_sha(dir.path());

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--body",
            "new-phase-body",
            "--no-edit",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["commit", "-m", "feat: update phase body"])
        .assert()
        .success();

    let new_sha = head_sha(dir.path());
    assert_ne!(old_sha, new_sha, "the update commit should have moved HEAD");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--at",
            &old_sha,
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("original-phase-body")
                .and(predicate::str::contains(format!("Revision: {old_sha}")))
                .and(predicate::str::contains("new-phase-body").not()),
        );
}

#[test]
fn phase_show_at_unknown_revision_errors() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--at",
            "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("is not known to the store"));
}

#[test]
fn phase_show_at_revision_missing_path_errors() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["commit", "-m", "seed: init plan repo and roadmap"])
        .assert()
        .success();

    // Capture anchor SHA *before* the phase exists.
    let pre_sha = head_sha(dir.path());

    create_phase(&dir, "core", "Core");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--at",
            &pre_sha,
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("is not present at revision"));
}

#[test]
fn phase_update_body_flag_beats_stdin() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--body",
            "inline body wins",
        ])
        .write_stdin("piped body loses")
        .assert()
        .success();

    let phase_file = dir
        .path()
        .join("projects/fbm/roadmaps/two-way/phase-1-core.md");
    let content = fs::read_to_string(&phase_file).unwrap();
    assert!(
        content.contains("inline body wins"),
        "expected inline body in file, got: {content}"
    );
    assert!(
        !content.contains("piped body loses"),
        "stdin must be ignored when --body is provided, got: {content}"
    );
}

/// Body content covering the reported hang triggers: backticks, em-dash,
/// curly quotes/ellipsis, shell metacharacters, and a literal `--no-edit`
/// substring embedded in the value (not passed as a separate flag).
const SPECIAL_BODY: &str = r#"backtick `code` em-dash — curly “quotes” ‘single’ ellipsis … shell $!\;|<>*~&& literal --no-edit here"#;

#[test]
fn phase_update_with_special_character_body_content() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--body",
            SPECIAL_BODY,
            "--no-edit",
        ])
        .assert()
        .success();

    let phase_file = dir
        .path()
        .join("projects/fbm/roadmaps/two-way/phase-1-core.md");
    let content = fs::read_to_string(&phase_file).unwrap();
    assert!(
        content.contains(SPECIAL_BODY),
        "expected special-character body to round-trip verbatim, got: {content}"
    );
}

#[test]
fn phase_create_with_special_character_body_content() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            "core",
            "--title",
            "Core Valuation",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--body",
            SPECIAL_BODY,
            "--no-edit",
        ])
        .assert()
        .success();

    let phase_file = dir
        .path()
        .join("projects/fbm/roadmaps/two-way/phase-1-core.md");
    let content = fs::read_to_string(&phase_file).unwrap();
    assert!(
        content.contains(SPECIAL_BODY),
        "expected special-character body to round-trip verbatim, got: {content}"
    );
}

/// Reproduces the reported hang mechanism directly: an orchestrator that
/// spawns the process with `--body` set but holds stdin open via a pipe
/// that is never written to and never closed. Since `--body` is
/// authoritative, `resolve_body` must never read stdin, so the process
/// must exit promptly regardless of the open pipe.
#[test]
fn phase_update_body_flag_no_hang_with_open_stdin_pipe() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");

    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_rdm"))
        .env("XDG_CONFIG_HOME", "/dev/null/nonexistent")
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--body",
            SPECIAL_BODY,
            "--no-edit",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();

    // Keep the child's stdin pipe open (never written, never closed) —
    // dropping it would deliver EOF and defeat the point of this test.
    let _stdin = child.stdin.take();

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let status = child.wait();
        let _ = tx.send(status);
    });

    let status = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("rdm phase update --body must not hang with stdin held open")
        .unwrap();

    assert!(status.success());
}

#[test]
fn phase_update_empty_body_refuses_clobber() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--body",
            "existing content",
            "--no-edit",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--body",
            "",
            "--no-edit",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--clear-body"));

    let phase_file = dir
        .path()
        .join("projects/fbm/roadmaps/two-way/phase-1-core.md");
    let content = fs::read_to_string(&phase_file).unwrap();
    assert!(
        content.contains("existing content"),
        "body should be unchanged after refused clobber, got: {content}"
    );
}

#[test]
fn phase_update_clear_body_succeeds() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--body",
            "existing content",
            "--no-edit",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--clear-body",
            "--no-edit",
        ])
        .assert()
        .success();

    let phase_file = dir
        .path()
        .join("projects/fbm/roadmaps/two-way/phase-1-core.md");
    let content = fs::read_to_string(&phase_file).unwrap();
    assert!(
        !content.contains("existing content"),
        "body should be empty after --clear-body, got: {content}"
    );
}

#[test]
fn phase_update_tags_ignores_stdin() {
    // Regression: a tags-only phase update must not consult stdin — feeding a
    // pipe must not clobber the body (and a never-closing pipe must not hang).
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--body",
            "Original phase body.",
            "--no-edit",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--tags",
            "audit",
            "--no-edit",
        ])
        .write_stdin("SNEAKY STDIN BODY")
        .assert()
        .success();

    let phase_file = dir
        .path()
        .join("projects/fbm/roadmaps/two-way/phase-1-core.md");
    let content = fs::read_to_string(&phase_file).unwrap();
    assert!(
        content.contains("Original phase body."),
        "tags-only update must preserve the existing body, got: {content}"
    );
    assert!(
        !content.contains("SNEAKY STDIN BODY"),
        "tags-only update must not read stdin into the body, got: {content}"
    );
}

#[test]
fn phase_update_empty_body_ok_when_already_empty() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--body",
            "",
            "--no-edit",
        ])
        .assert()
        .success();
}

#[test]
fn phase_update_clear_body_conflicts_with_body_flag() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--body",
            "x",
            "--clear-body",
            "--no-edit",
        ])
        .assert()
        .failure();
}

#[test]
fn phase_update_title_renames_in_place() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Old Phase Title");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--title",
            "New Phase Title",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success();

    // New title reflected via `show`; stem/number unchanged.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("New Phase Title")
                .and(predicate::str::contains("phase-1-core")),
        );
}

#[test]
fn phase_update_empty_title_rejected() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Keep This Title");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--title",
            "   ",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("title cannot be empty"));

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Keep This Title"));
}

fn phase_show_json(dir: &TempDir, cwd: Option<&Path>) -> serde_json::Value {
    let mut cmd = rdm();
    cmd.arg("--root").arg(dir.path()).args([
        "phase",
        "show",
        "phase-1-core",
        "--roadmap",
        "two-way",
        "--project",
        "fbm",
        "--format",
        "json",
    ]);
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    let output = cmd.assert().success().get_output().stdout.clone();
    serde_json::from_slice(&output).unwrap()
}

fn stamp_in_progress(dir: &TempDir, cwd: &Path) -> assert_cmd::assert::Assert {
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--status",
            "in-progress",
            "--no-edit",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .current_dir(cwd)
        .assert()
}

/// Under `explicit-start-commit`, no status transition records `started_head`
/// on its own any more — rebuilt from the old AC1/C1 coverage (review
/// 2026-09-23-0309-996c), which tested the retired automatic in-progress
/// resolution. A bare `--status in-progress` stamp (no `--start-commit`)
/// succeeds with no warning and no recording, whether or not a worktree is
/// registered, and repeating it never fills the field in either. `review
/// source` on the still-unstamped phase then falls back to the merge-base.
///
/// The explicit `--start-commit` write path (recording the worktree's HEAD
/// from a cwd outside it, and the write-once refusal once a value IS
/// recorded) is covered end to end against a real source-repo worktree in
/// `cli_gate.rs`'s `started_head_scopes_the_second_phase_review_and_satisfies_the_gate`.
#[test]
fn in_progress_status_alone_never_records_started_head() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core");
    let src = init_source_repo();

    // No worktree registered for `two-way` at all: the stamp still succeeds,
    // with nothing recorded and no warning — there is no automatic
    // resolution left to miss.
    let first_stamp = stamp_in_progress(&dir, src.path()).success();
    let first_stderr = String::from_utf8_lossy(&first_stamp.get_output().stderr).to_string();
    assert!(
        !first_stderr.contains("started_head"),
        "a bare in-progress stamp must not warn about started_head: {first_stderr}"
    );
    let json = phase_show_json(&dir, None);
    assert!(
        json.get("started_head").is_none(),
        "a bare --status in-progress must never record started_head: {json}"
    );

    // Register the worktree now, then re-stamp `in-progress` (still with no
    // `--start-commit`) — the field must STAY absent.
    let out = rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["worktree", "add", "two-way", "--project", "fbm"])
        .current_dir(src.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let wt = std::path::PathBuf::from(String::from_utf8_lossy(&out).trim().to_string());
    stamp_in_progress(&dir, src.path()).success();
    let json = phase_show_json(&dir, None);
    assert!(
        json.get("started_head").is_none(),
        "a repeat in-progress stamp must never fill in started_head, even once a worktree resolves: {json}"
    );

    // A real committed change on the worktree branch, so the reviewed range
    // is non-empty and `review source` has something to resolve a base for.
    let merge_base = String::from_utf8_lossy(&git(src.path(), &["rev-parse", "HEAD"]).stdout)
        .trim()
        .to_string();
    fs::write(wt.join("extra.txt"), "phase work\n").unwrap();
    git(&wt, &["add", "extra.txt"]);
    git(&wt, &["commit", "-m", "phase work"]);

    // With no started_head recorded, `review source` falls back to the
    // merge-base with the default branch.
    let out = rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "review",
            "source",
            "--on",
            "phase/two-way/phase-1-core",
            "--project",
            "fbm",
        ])
        .current_dir(src.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let source: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(source["base"], merge_base);
    assert!(
        source["baseNote"]
            .as_str()
            .is_some_and(|n| n.contains("main")),
        "review source must fall back to the merge-base with a note naming 'main': {source}"
    );
}

/// `--start-commit` is an explicit instruction, independent of `--status`, so
/// every resolution failure is a hard error with NOTHING written: no
/// registered worktree at all, a malformed value, and a well-formed but
/// nonexistent commit.
#[test]
fn start_commit_resolution_failures_are_hard_errors_and_write_nothing() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core");
    let src = init_source_repo();
    let real_sha = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef";

    // No worktree registered for `two-way` at all.
    let assert = rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--start-commit",
            real_sha,
            "--no-edit",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .current_dir(src.path())
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_string();
    assert!(
        stderr.contains("no rdm worktree"),
        "an unregistered item must refuse naming the missing worktree: {stderr}"
    );
    let json = phase_show_json(&dir, None);
    assert!(json.get("started_head").is_none());

    let out = rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["worktree", "add", "two-way", "--project", "fbm"])
        .current_dir(src.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let wt = std::path::PathBuf::from(String::from_utf8_lossy(&out).trim().to_string());

    // A malformed value (not 40 lowercase hex characters) is refused before
    // any existence check.
    let assert = rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--start-commit",
            "not-a-sha",
            "--no-edit",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .current_dir(src.path())
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_string();
    assert!(
        stderr.contains("not-a-sha"),
        "a malformed --start-commit must name the rejected value: {stderr}"
    );
    let json = phase_show_json(&dir, None);
    assert!(json.get("started_head").is_none());

    // A well-formed SHA that does not resolve to a commit in the item's
    // worktree.
    let assert = rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--start-commit",
            real_sha,
            "--no-edit",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .current_dir(src.path())
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_string();
    assert!(
        stderr.contains(real_sha) && stderr.contains(wt.to_str().unwrap()),
        "a nonexistent commit must name both the SHA and the repo it was checked against: {stderr}"
    );
    let json = phase_show_json(&dir, None);
    assert!(
        json.get("started_head").is_none(),
        "no --start-commit resolution failure may write a partial value: {json}"
    );
}

/// The advance/park outcome writes the prose autopilot loop performs: on a
/// plan repo with no `gates.reviewed` key (the gate ships off), `--status
/// reviewed` lands and reads back, and `--status blocked --reason …` records
/// a `blocked_reason` that `phase show --format json` returns. Ported from
/// the retired `scripts/verify-skill-autopilot.sh` § 2.
#[test]
fn reviewed_and_blocked_reason_read_back_as_json() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "a", "Phase A");
    create_phase(&dir, "b", "Phase B");
    let update = |stem: &str, extra: &[&str]| {
        rdm()
            .arg("--root")
            .arg(dir.path())
            .args(["phase", "update", stem])
            .args(extra)
            .args(["--no-edit", "--roadmap", "two-way", "--project", "fbm"])
            .assert()
            .success();
    };
    let show = |stem: &str| -> serde_json::Value {
        let out = rdm()
            .arg("--root")
            .arg(dir.path())
            .args([
                "phase",
                "show",
                stem,
                "--roadmap",
                "two-way",
                "--project",
                "fbm",
                "--format",
                "json",
                "--no-body",
            ])
            .assert()
            .success();
        serde_json::from_slice(&out.get_output().stdout).unwrap()
    };
    for stem in ["phase-1-a", "phase-2-b"] {
        update(stem, &["--status", "in-progress"]);
    }

    update("phase-1-a", &["--status", "reviewed"]);
    assert_eq!(show("phase-1-a")["status"], "reviewed");

    let reason = "[code] rework budget exhausted";
    update("phase-2-b", &["--status", "blocked", "--reason", reason]);
    let parked = show("phase-2-b");
    assert_eq!(parked["status"], "blocked");
    assert_eq!(parked["blocked_reason"], reason);
}
