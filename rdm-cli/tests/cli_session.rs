//! `rdm session` end-to-end: identity precedence, journal exactness, and the
//! invisibility of session state to `rdm status` and `rdm commit`.
//!
//! Everything here runs the real binary against a temp plan repo. The
//! identity-precedence assertions pin `RDM_SESSION` so they never depend on
//! the test runner's process tree; the rung-chain behavior below rung 1 is
//! covered hermetically by `rdm-core`'s unit tests and end-to-end across real
//! processes by `scripts/verify-session-identity.sh`.

use std::collections::BTreeSet;

use assert_cmd::Command;
use tempfile::TempDir;

fn rdm(dir: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("rdm").unwrap();
    // Isolate from host global config (e.g. default_format = "json") and from
    // any session identity the developer's own shell exports.
    cmd.env("XDG_CONFIG_HOME", "/dev/null/nonexistent")
        .env_remove("RDM_SESSION")
        .env_remove("CLAUDE_CODE_SESSION_ID")
        .env_remove("CLAUDE_SESSION_ID")
        .env_remove("RDM_HARNESS_SESSION_ID")
        .arg("--root")
        .arg(dir.path());
    cmd
}

fn init_repo(dir: &TempDir) {
    rdm(dir).arg("init").assert().success();
    rdm(dir)
        .args(["project", "create", "demo", "--title", "Demo"])
        .assert()
        .success();
    rdm(dir)
        .args(["commit", "-m", "seed: init plan repo and project"])
        .assert()
        .success();
}

/// Runs git against `dir` with the ambient git environment stripped.
///
/// `GIT_DIR` / `GIT_WORK_TREE` / `GIT_INDEX_FILE` are set whenever the suite
/// itself runs under a git hook (the repo's own pre-commit gate), and an
/// inherited `GIT_DIR` would silently point this query at the *source* repo
/// rather than the temp plan repo.
fn git(dir: &std::path::Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stdout(cmd: &mut Command) -> String {
    String::from_utf8(cmd.assert().success().get_output().stdout.clone()).unwrap()
}

fn journal_paths(dir: &TempDir, session: &str) -> BTreeSet<String> {
    let raw = stdout(
        rdm(dir)
            .env("RDM_SESSION", session)
            .args(["session", "journal", "--format", "json"]),
    );
    let value: serde_json::Value = serde_json::from_str(raw.lines().next_back().unwrap()).unwrap();
    value["paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["path"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn explicit_session_env_wins_and_is_reported_as_rung_one() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    let raw = stdout(
        rdm(&dir)
            .env("RDM_SESSION", "explicit-1")
            // A harness variable is also set; rung 1 must still win.
            .env("CLAUDE_CODE_SESSION_ID", "abc123")
            .args(["session", "id", "--format", "json"]),
    );
    let value: serde_json::Value = serde_json::from_str(raw.lines().next_back().unwrap()).unwrap();
    assert_eq!(value["id"], "explicit-1");
    assert_eq!(value["rung"], 1);
    assert!(value["resolve_micros"].is_number());

    // Text output is the bare id, so shells can capture it with `$(...)`.
    let text = stdout(
        rdm(&dir)
            .env("RDM_SESSION", "explicit-1")
            .args(["session", "id"]),
    );
    assert!(
        text.lines().any(|l| l == "explicit-1"),
        "expected a bare id line, got: {text}"
    );
}

#[test]
fn a_blank_explicit_session_falls_through_instead_of_erroring() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);
    let raw = stdout(
        rdm(&dir)
            .env("RDM_SESSION", "   ")
            .args(["session", "id", "--format", "json"]),
    );
    let value: serde_json::Value = serde_json::from_str(raw.lines().next_back().unwrap()).unwrap();
    let rung = value["rung"].as_u64().unwrap();
    assert!((2..=4).contains(&rung), "expected a lower rung, got {rung}");
    assert!(!value["id"].as_str().unwrap().is_empty());
}

#[test]
fn journal_lists_exactly_the_mutations_paths() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    rdm(&dir)
        .env("RDM_SESSION", "alpha")
        .args([
            "task",
            "create",
            "alpha-one",
            "--title",
            "Alpha",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .success();

    // Exactly the task file plus the two indexes `ops::mutate` regenerates —
    // asserted as whole-set equality, so a superset fails.
    let expected: BTreeSet<String> = [
        "INDEX.md",
        "projects/demo/INDEX.md",
        "projects/demo/tasks/alpha-one.md",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let actual = journal_paths(&dir, "alpha");
    assert!(!actual.is_empty(), "the journal must not be empty");
    assert_eq!(actual, expected);
}

#[test]
fn concurrent_journals_never_contain_each_others_paths() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    for (session, slug) in [("alpha", "alpha-one"), ("beta", "beta-one")] {
        rdm(&dir)
            .env("RDM_SESSION", session)
            .args([
                "task",
                "create",
                slug,
                "--title",
                "T",
                "--no-edit",
                "--project",
                "demo",
            ])
            .assert()
            .success();
    }

    let alpha = journal_paths(&dir, "alpha");
    let beta = journal_paths(&dir, "beta");
    assert!(alpha.contains("projects/demo/tasks/alpha-one.md"));
    assert!(!alpha.contains("projects/demo/tasks/beta-one.md"));
    assert!(beta.contains("projects/demo/tasks/beta-one.md"));
    assert!(!beta.contains("projects/demo/tasks/alpha-one.md"));
    // The shared derived index lands in BOTH journals. That is correct here:
    // `ops::mutate` regenerates it on every mutation, so every session really
    // did write it. Reconciling it at commit time is a later phase's problem.
    assert!(alpha.contains("INDEX.md") && beta.contains("INDEX.md"));
}

#[test]
fn session_state_is_invisible_to_status_and_to_a_whole_tree_commit() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);
    rdm(&dir)
        .env("RDM_SESSION", "alpha")
        .args([
            "task",
            "create",
            "alpha-one",
            "--title",
            "Alpha",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .success();

    let status = stdout(rdm(&dir).env("RDM_SESSION", "alpha").arg("status"));
    for needle in ["leases", "changesets", ".git"] {
        assert!(
            !status.contains(needle),
            "rdm status named session state ({needle}): {status}"
        );
    }

    // Captured BEFORE the commit: a successful scoped commit truncates the
    // paths it landed out of the journal, so reading it afterwards would be
    // empty for the right reason and make the assertion vacuous.
    assert!(!journal_paths(&dir, "alpha").is_empty());

    rdm(&dir)
        .env("RDM_SESSION", "alpha")
        .args(["commit", "-m", "add alpha-one"])
        .assert()
        .success();

    let tracked = git(dir.path(), &["ls-tree", "-r", "--name-only", "HEAD"]);
    assert!(
        !tracked.is_empty(),
        "git ls-tree returned nothing, so the assertions below would be vacuous"
    );
    assert!(tracked.contains("projects/demo/tasks/alpha-one.md"));
    for needle in ["rdm/leases", "rdm/changesets", ".lease", ".jsonl"] {
        assert!(
            !tracked.contains(needle),
            "a whole-tree commit swept up session state ({needle}): {tracked}"
        );
    }
    // …and the landed paths were truncated out of the journal, so a second
    // commit cannot re-commit them over another session's later edit.
    assert!(journal_paths(&dir, "alpha").is_empty());
}

#[test]
fn list_flags_orphans_and_adopt_repoints_the_caller() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);
    rdm(&dir)
        .env("RDM_SESSION", "orphaned-one")
        .args([
            "task",
            "create",
            "alpha-one",
            "--title",
            "Alpha",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .success();

    // Seen from a different session, `orphaned-one` has no live lease.
    let raw = stdout(
        rdm(&dir)
            .env("RDM_SESSION", "someone-else")
            .args(["session", "list", "--format", "json"]),
    );
    let listed: serde_json::Value = serde_json::from_str(raw.lines().next_back().unwrap()).unwrap();
    let orphan = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "orphaned-one")
        .expect("the changeset should be listed");
    assert_eq!(orphan["orphaned"], true);
    assert_eq!(orphan["paths"], 3);

    // Adopting re-points the caller's lease, so a later bare invocation from
    // the same parent resolves the orphaned changeset instead of its own.
    rdm(&dir)
        .args(["session", "adopt", "orphaned-one"])
        .assert()
        .success();
    let id = stdout(rdm(&dir).args(["session", "id"]));
    assert!(
        id.lines().any(|l| l == "orphaned-one"),
        "expected the adopted id, got: {id}"
    );

    // Discarding is gated on --force.
    rdm(&dir)
        .args(["session", "discard", "orphaned-one"])
        .assert()
        .failure();
    rdm(&dir)
        .args(["session", "discard", "orphaned-one", "--force"])
        .assert()
        .success();
    assert!(journal_paths(&dir, "orphaned-one").is_empty());
}

#[test]
fn an_unusable_changeset_id_reports_the_rule() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);
    let output = rdm(&dir)
        .args(["session", "journal", "--id", "///"])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&output.get_output().stderr).to_string();
    assert!(
        stderr.contains("not a usable changeset id"),
        "expected an actionable message, got: {stderr}"
    );
}

#[test]
fn gc_runs_and_reports_without_touching_journals() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);
    rdm(&dir)
        .env("RDM_SESSION", "alpha")
        .args([
            "task",
            "create",
            "alpha-one",
            "--title",
            "Alpha",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .success();

    let out = stdout(rdm(&dir).args(["session", "gc"]));
    assert!(out.contains("stale lease(s)"), "got: {out}");
    // GC removes dead leases, never journals: no work is silently destroyed.
    assert_eq!(journal_paths(&dir, "alpha").len(), 3);
}
