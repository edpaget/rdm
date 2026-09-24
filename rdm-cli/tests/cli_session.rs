//! `rdm session` end-to-end: identity precedence, journal exactness, and the
//! invisibility of session state to `rdm status` and `rdm commit`.
//!
//! Everything here runs the real binary against a temp plan repo. The
//! identity-precedence assertions pin `RDM_SESSION` so they never depend on
//! the test runner's process tree; the rung-chain behavior below rung 1 is
//! covered hermetically by `rdm-core`'s unit tests and end-to-end across real
//! processes by the `concurrency` test binary
//! (`tests/concurrency/session_identity.rs`).

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
fn session_id_json_reports_whether_this_call_bootstrapped_its_lease() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    // Rung 1: an explicitly pinned id reads no lease and creates none, so the
    // flag is false. This is the field's stable meaning — "this invocation
    // minted the changeset it is reporting" — not "a lease exists somewhere".
    let raw = stdout(
        rdm(&dir)
            .env("RDM_SESSION", "explicit-1")
            .args(["session", "id", "--format", "json"]),
    );
    let value: serde_json::Value = serde_json::from_str(raw.lines().next_back().unwrap()).unwrap();
    assert_eq!(value["lease_bootstrapped"], false);

    // Rung 3: a harness variable is derived, never lease-backed.
    let raw = stdout(
        rdm(&dir)
            .env("CLAUDE_CODE_SESSION_ID", "abc123")
            .args(["session", "id", "--format", "json"]),
    );
    let value: serde_json::Value = serde_json::from_str(raw.lines().next_back().unwrap()).unwrap();
    assert_eq!(value["rung"], 3);
    assert_eq!(value["lease_bootstrapped"], false);

    // The field must be present on every rung, so a consumer can read it
    // unconditionally rather than probing for it.
    assert!(value["lease_bootstrapped"].is_boolean());
}

#[test]
fn commit_stays_quiet_for_a_caller_that_has_continuity() {
    // The phase-11 advisory must not fire at a caller who already has a stable
    // session id — the nag would land on exactly the people who did the right
    // thing. Asserted on both the successful-commit and the nothing-to-commit
    // paths, since the advisory is wired into the latter's neighbourhood.
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    rdm(&dir)
        .env("RDM_SESSION", "quiet-1")
        .args([
            "task",
            "create",
            "quiet-item",
            "--title",
            "Quiet",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .success();
    let landed =
        stdout(
            rdm(&dir)
                .env("RDM_SESSION", "quiet-1")
                .args(["commit", "-m", "add quiet-item"]),
        );
    assert!(
        !landed.contains("RDM_HARNESS_SESSION_ID"),
        "a successful commit must not print the continuity advisory: {landed}"
    );

    let empty =
        stdout(
            rdm(&dir)
                .env("RDM_SESSION", "quiet-1")
                .args(["commit", "-m", "nothing left"]),
        );
    assert!(
        !empty.contains("RDM_HARNESS_SESSION_ID"),
        "a clean-tree no-op commit must not print the continuity advisory: {empty}"
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

    // Exactly the one authored task file — a mutation regenerates no index —
    // asserted as whole-set equality, so a superset fails.
    let expected: BTreeSet<String> = ["projects/demo/tasks/alpha-one.md"]
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
    // And neither journal names an INDEX.md at all: a mutation authors
    // only its own entity file, so there is no shared index path for two
    // sessions to contend over in the first place.
    assert!(
        !alpha.iter().any(|p| p.ends_with("INDEX.md")),
        "alpha's journal named an INDEX.md: {alpha:?}"
    );
    assert!(
        !beta.iter().any(|p| p.ends_with("INDEX.md")),
        "beta's journal named an INDEX.md: {beta:?}"
    );
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

    // Seen from a different session, `orphaned-one` has no live lease (unleased).
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
    assert_eq!(orphan["liveness"], "unleased");
    assert_eq!(orphan["paths"], 1);

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
fn adopt_with_a_harness_var_set_refuses_instead_of_reporting_false_success() {
    // Since phase 10 of plan-repo-concurrency, a harness variable (rung 3) is
    // checked before an inherited lease (rung 2), so `session adopt` — which
    // works by repointing the caller's parent lease — would silently have no
    // effect for a caller carrying a harness variable: the next `session id`
    // call would keep resolving the harness id, never the adopted changeset.
    let dir = TempDir::new().unwrap();
    init_repo(&dir);
    rdm(&dir)
        .env("RDM_SESSION", "orphaned-two")
        .args([
            "task",
            "create",
            "alpha-two",
            "--title",
            "Alpha",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .success();

    let output = rdm(&dir)
        .env("CLAUDE_CODE_SESSION_ID", "child-one")
        .args(["session", "adopt", "orphaned-two"])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&output.get_output().stderr).to_string();
    assert!(
        stderr.contains("CLAUDE_CODE_SESSION_ID"),
        "expected the offending harness var named in the error, got: {stderr}"
    );
    assert!(
        stderr.contains("RDM_SESSION"),
        "expected the actionable RDM_SESSION alternative, got: {stderr}"
    );

    // Non-vacuousness: the caller's own next resolution still reports its
    // harness id, never the orphaned changeset — the refusal really did
    // nothing, rather than reporting failure while repointing anyway.
    let id = stdout(
        rdm(&dir)
            .env("CLAUDE_CODE_SESSION_ID", "child-one")
            .args(["session", "id"]),
    );
    assert!(
        !id.lines().any(|l| l == "orphaned-two"),
        "adoption should not have taken effect, got: {id}"
    );
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
    assert_eq!(journal_paths(&dir, "alpha").len(), 1);
}

/// The sibling of the test above, covering the *other* half of the sweep.
///
/// `gc_runs_and_reports_without_touching_journals` pins a journal that still
/// claims paths: gc must leave it alone. This one pins the journal that only
/// exists because truncation is now an append-only tombstone rather than an
/// inline delete — fully committed, folding to zero entries, and swept by
/// nothing except `rdm session gc`.
#[test]
fn gc_sweeps_a_journal_whose_changeset_is_fully_committed() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);
    rdm(&dir)
        .env("RDM_SESSION", "beta")
        .args([
            "task",
            "create",
            "beta-one",
            "--title",
            "Beta",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .success();
    assert!(
        !journal_paths(&dir, "beta").is_empty(),
        "the create should have journaled its writes"
    );

    let journal = dir.path().join(".git/rdm/changesets/beta.jsonl");
    assert!(
        journal.exists(),
        "expected a journal at {}",
        journal.display()
    );

    rdm(&dir)
        .env("RDM_SESSION", "beta")
        .args(["commit", "-m", "land beta"])
        .assert()
        .success();
    assert!(
        journal_paths(&dir, "beta").is_empty(),
        "a fully-committed changeset claims nothing"
    );
    assert!(
        journal.exists(),
        "truncation appends a tombstone; it must not remove the file inline"
    );
    // `session list` already hides it, since it claims no path.
    let listed = stdout(rdm(&dir).args(["session", "list"]));
    assert!(
        !listed.contains("beta"),
        "a changeset claiming nothing has no work to recover, got: {listed}"
    );

    let out = stdout(rdm(&dir).args(["session", "gc"]));
    let swept = out
        .lines()
        .find_map(|l| {
            l.strip_prefix("Removed ")
                .and_then(|rest| rest.strip_suffix(" fully-committed changeset journal(s)."))
        })
        .unwrap_or_else(|| panic!("expected the second sweep to report, got: {out}"))
        .parse::<usize>()
        .unwrap();
    assert!(
        swept >= 1,
        "expected beta's journal to be swept, got: {out}"
    );
    assert!(
        !journal.exists(),
        "gc is the only thing that clears a fully-committed journal"
    );
}

#[test]
fn two_harness_sessions_view_each_other_unleased_not_orphaned() {
    // AC1: Two distinct harness sessions with CLAUDE_CODE_SESSION_ID should
    // verify that session B does not see session A's still-active unleased
    // changeset labeled as "orphaned".
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    // Session A (harness-derived id via CLAUDE_CODE_SESSION_ID): create a changeset.
    // First, get session A's actual id, since harness vars are hashed to session ids.
    let session_a_id = stdout(
        rdm(&dir)
            .env("CLAUDE_CODE_SESSION_ID", "harness-session-a")
            .args(["session", "id"]),
    )
    .lines()
    .next()
    .unwrap()
    .to_string();

    rdm(&dir)
        .env("CLAUDE_CODE_SESSION_ID", "harness-session-a")
        .args([
            "task",
            "create",
            "task-from-a",
            "--title",
            "Task from session A",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .success();

    // Session B (different harness-derived id): list changesets and verify A's
    // changeset is not labeled "orphaned", but as "unleased" instead.
    let raw = stdout(
        rdm(&dir)
            .env("CLAUDE_CODE_SESSION_ID", "harness-session-b")
            .args(["session", "list", "--format", "json"]),
    );
    let listed: serde_json::Value = serde_json::from_str(raw.lines().next_back().unwrap()).unwrap();
    let a_changeset = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == session_a_id)
        .unwrap_or_else(|| {
            panic!(
                "session A's changeset {} should be listed from session B, but found: {}",
                session_a_id,
                serde_json::to_string_pretty(&listed).unwrap_or_default()
            )
        });

    // The critical assertion: A's unleased changeset is labeled "unleased", never "orphaned".
    assert_eq!(
        a_changeset["liveness"], "unleased",
        "session A's unleased changeset must not be labeled 'orphaned'"
    );
    assert_eq!(a_changeset["paths"], 1);

    // Session A's own row should have liveness "current".
    let a_own = stdout(
        rdm(&dir)
            .env("CLAUDE_CODE_SESSION_ID", "harness-session-a")
            .args(["session", "list", "--format", "json"]),
    );
    let a_listed: serde_json::Value =
        serde_json::from_str(a_own.lines().next_back().unwrap()).unwrap();
    let a_self = a_listed
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == session_a_id)
        .expect("session A's own changeset should be listed");
    assert_eq!(
        a_self["liveness"], "current",
        "caller's own changeset must be current, not unleased"
    );
}

#[test]
fn session_list_text_output_shows_unleased_label() {
    // Tests AC3: The label used for unleased changesets does not say "orphaned"
    // in human (non-JSON) output.
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    rdm(&dir)
        .env("RDM_SESSION", "unleased-test")
        .args([
            "task",
            "create",
            "task-for-text",
            "--title",
            "Task",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .success();

    // List from a different session in text (non-JSON) format.
    let output = stdout(
        rdm(&dir)
            .env("RDM_SESSION", "viewer")
            .args(["session", "list"]),
    );

    // The text output should show "(unleased)", never "(orphaned)".
    assert!(
        output.contains("unleased-test"),
        "unleased changeset should appear in listing: {output}"
    );
    assert!(
        output.contains("(unleased)"),
        "text output should show '(unleased)' for unleased changesets: {output}"
    );
    assert!(
        !output.contains("orphaned"),
        "text output should never show 'orphaned' for unleased changesets: {output}"
    );
}

#[test]
fn caller_own_changeset_remains_unflagged() {
    // AC4: The caller's own row remains unflagged, even if their changeset is unleased.
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    rdm(&dir)
        .env("RDM_SESSION", "caller-own")
        .args([
            "task",
            "create",
            "caller-task",
            "--title",
            "Caller's task",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .success();

    // Caller lists their own session: should show "current" liveness, not "unleased".
    let json_out = stdout(
        rdm(&dir)
            .env("RDM_SESSION", "caller-own")
            .args(["session", "list", "--format", "json"]),
    );
    let listed: serde_json::Value =
        serde_json::from_str(json_out.lines().next_back().unwrap()).unwrap();
    let own = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "caller-own")
        .expect("caller's own changeset should be in listing");
    assert_eq!(
        own["liveness"], "current",
        "caller's own changeset must be 'current', not 'unleased'"
    );

    // In text output, the caller's row should have no flag at all.
    let text_out = stdout(
        rdm(&dir)
            .env("RDM_SESSION", "caller-own")
            .args(["session", "list"]),
    );
    assert!(
        text_out.contains("caller-own"),
        "caller's changeset should appear in text listing: {text_out}"
    );
    // Find the line with caller-own and ensure it has no (unleased) or (orphaned) flag.
    let caller_line = text_out
        .lines()
        .find(|l| l.contains("caller-own"))
        .expect("caller-own should be in text output");
    assert!(
        !caller_line.contains("(unleased)") && !caller_line.contains("(orphaned)"),
        "caller's own row must have no liveness flag: {caller_line}"
    );
}

#[test]
fn discard_all_warning_correctly_labels_unleased_and_orphaned() {
    // Tests AC3 for the discard warning path: verify both text and liveness
    // labels render correctly when rdm discard --all warns about other changesets.
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    // Create an unleased changeset (rung 1, no lease created).
    rdm(&dir)
        .env("RDM_SESSION", "unleased-for-warning")
        .args([
            "task",
            "create",
            "task-unleased",
            "--title",
            "Task",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .success();

    // Create an orphaned changeset: a session with a dead lease file.
    // Create it in the plan repo's .git/rdm directory so rdm will discover it.
    let git_dir = dir.path().join(".git");
    let rdm_dir = git_dir.join("rdm");
    let leases_dir = rdm_dir.join("leases");
    let changesets_dir = rdm_dir.join("changesets");

    std::fs::create_dir_all(&leases_dir).unwrap();
    std::fs::create_dir_all(&changesets_dir).unwrap();

    // Create a lease file for a dead PID (999999999 is unlikely to exist).
    // The .lease file contains the session metadata.
    let lease_path = leases_dir.join("999999999.lease");
    let lease_content = serde_json::json!({
        "id": "orphaned-for-warning",
        "start_time": "0x0102030405060708",
        "created_utc": "2026-09-09T00:00:00Z"
    });
    std::fs::write(&lease_path, lease_content.to_string()).unwrap();

    // Create a journal file for the orphaned changeset so it has an entry in the changesets.
    let changeset_path = changesets_dir.join("orphaned-for-warning.jsonl");
    let journal_line = serde_json::json!({
        "paths": [
            {
                "path": "task-orphaned.md",
                "kind": "write",
                "digest": "abc123def456"
            }
        ]
    });
    std::fs::write(&changeset_path, format!("{}\n", journal_line)).unwrap();

    // Now try to discard --all from a different session; the warning should
    // show both the unleased changeset with "(unleased)" and the orphaned
    // changeset with "(orphaned)" labels.
    let output = rdm(&dir)
        .env("RDM_SESSION", "another-session")
        .args(["discard", "--all", "--force"])
        .assert()
        .success();

    let stderr = String::from_utf8_lossy(&output.get_output().stderr).to_string();

    // Verify unleased changeset is labeled correctly
    assert!(
        stderr.contains("unleased-for-warning"),
        "warning should name the unleased changeset: {stderr}"
    );
    assert!(
        stderr.contains("(unleased)"),
        "warning should label the changeset as unleased: {stderr}"
    );

    // Verify orphaned changeset is labeled correctly
    assert!(
        stderr.contains("orphaned-for-warning"),
        "warning should name the orphaned changeset: {stderr}"
    );
    assert!(
        stderr.contains("(orphaned)"),
        "warning should label the changeset as orphaned: {stderr}"
    );
}

#[test]
fn session_list_renders_orphaned_label_for_dead_leases() {
    // Tests that orphaned changesets (with dead leases) are correctly labeled
    // as "orphaned" in both text and JSON output of `rdm session list`.
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    // Create an orphaned changeset in the plan repo's .git/rdm directory.
    let git_dir = dir.path().join(".git");
    let rdm_dir = git_dir.join("rdm");
    let leases_dir = rdm_dir.join("leases");
    let changesets_dir = rdm_dir.join("changesets");

    std::fs::create_dir_all(&leases_dir).unwrap();
    std::fs::create_dir_all(&changesets_dir).unwrap();

    // Create a lease file for a dead PID.
    let lease_path = leases_dir.join("888888888.lease");
    let lease_content = serde_json::json!({
        "id": "dead-lease-test",
        "start_time": "0x0102030405060708",
        "created_utc": "2026-09-09T00:00:00Z"
    });
    std::fs::write(&lease_path, lease_content.to_string()).unwrap();

    // Create a journal file for the orphaned changeset.
    let changeset_path = changesets_dir.join("dead-lease-test.jsonl");
    let journal_line = serde_json::json!({
        "paths": [
            {
                "path": "test-file.md",
                "kind": "write",
                "digest": "abc123def456"
            }
        ]
    });
    std::fs::write(&changeset_path, format!("{}\n", journal_line)).unwrap();

    // Test JSON output: `rdm session list --format json` should show
    // the literal string "orphaned" for the liveness field.
    let raw = stdout(
        rdm(&dir)
            .env("RDM_SESSION", "some-other-session")
            .args(["session", "list", "--format", "json"]),
    );
    let listed: serde_json::Value = serde_json::from_str(raw.lines().next_back().unwrap()).unwrap();
    let orphan = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "dead-lease-test")
        .expect("the orphaned changeset should be listed");
    assert_eq!(
        orphan["liveness"], "orphaned",
        "JSON output should show liveness as 'orphaned' (not 'unleased')"
    );

    // Test text output: `rdm session list` should show "(orphaned)" in the output.
    let raw_text = stdout(
        rdm(&dir)
            .env("RDM_SESSION", "another-other-session")
            .args(["session", "list"]),
    );
    assert!(
        raw_text.contains("(orphaned)"),
        "text output should show '(orphaned)' label for dead-lease-test: {raw_text}"
    );
    assert!(
        raw_text.contains("dead-lease-test"),
        "text output should name the orphaned changeset: {raw_text}"
    );
}
