//! `rdm run` — run records, driven through the real binary against a temp
//! plan repo.
//!
//! Every invocation clears `RDM_ROOT`/`RDM_PROJECT` (the plan repo is always
//! the test's own `--root`), pins the changeset session with an explicit
//! `RDM_SESSION`, keeps session state in a temp `XDG_STATE_HOME`, and sets or
//! removes `CLAUDE_CODE_SESSION_ID` explicitly — the suite itself often runs
//! inside a Claude Code session, whose id would otherwise leak into
//! `session_uuid`.

use std::process::Output;

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

/// A seeded, committed plan repo: project `test`, roadmaps `alpha` (phases 1
/// and 2) and `beta` (phase 1), and task `fix-bug`.
struct Repo {
    dir: TempDir,
    state: TempDir,
}

/// What `CLAUDE_CODE_SESSION_ID` is for one invocation.
#[derive(Clone, Copy)]
enum Session<'a> {
    Unset,
    Set(&'a str),
}

impl Repo {
    fn new() -> Self {
        let repo = Repo {
            dir: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
        };
        repo.ok(&["init"]);
        repo.ok(&["project", "create", "test", "--title", "Test"]);
        for slug in ["alpha", "beta"] {
            repo.ok(&[
                "roadmap",
                "create",
                slug,
                "--title",
                slug,
                "--no-edit",
                "--project",
                "test",
            ]);
        }
        for (roadmap, n, slug) in [
            ("alpha", "1", "one"),
            ("alpha", "2", "two"),
            ("beta", "1", "uno"),
        ] {
            repo.ok(&[
                "phase",
                "create",
                slug,
                "--title",
                slug,
                "--number",
                n,
                "--no-edit",
                "--roadmap",
                roadmap,
                "--project",
                "test",
            ]);
        }
        repo.ok(&[
            "task",
            "create",
            "fix-bug",
            "--title",
            "Fix bug",
            "--no-edit",
            "--project",
            "test",
        ]);
        repo.ok(&["commit", "-m", "seed"]);
        repo
    }

    fn cmd(&self, session: Session<'_>) -> Command {
        let mut cmd = Command::cargo_bin("rdm").unwrap();
        cmd.env_remove("RDM_ROOT")
            .env_remove("RDM_PROJECT")
            .env("RDM_SESSION", "cli-run-test")
            .env("XDG_STATE_HOME", self.state.path())
            .env("XDG_CONFIG_HOME", "/dev/null/nonexistent");
        match session {
            Session::Unset => {
                cmd.env_remove("CLAUDE_CODE_SESSION_ID");
            }
            Session::Set(v) => {
                cmd.env("CLAUDE_CODE_SESSION_ID", v);
            }
        }
        cmd.arg("--root").arg(self.dir.path());
        cmd
    }

    fn run_with(&self, session: Session<'_>, args: &[&str]) -> Output {
        self.cmd(session).args(args).output().unwrap()
    }

    /// Runs `args` with `CLAUDE_CODE_SESSION_ID` removed and asserts success,
    /// returning stdout.
    fn ok(&self, args: &[&str]) -> String {
        self.ok_with(Session::Unset, args)
    }

    fn ok_with(&self, session: Session<'_>, args: &[&str]) -> String {
        let out = self.run_with(session, args);
        assert!(
            out.status.success(),
            "rdm {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    fn json(&self, args: &[&str]) -> Value {
        let mut full = args.to_vec();
        full.extend(["--format", "json", "--project", "test"]);
        serde_json::from_str(&self.ok(&full)).expect("stdout is JSON")
    }

    /// `rdm run record` (human) with the given session, returning the id.
    fn record(&self, session: Session<'_>, target: &[&str]) -> String {
        let mut args = vec![
            "run",
            "record",
            "--driver",
            "autopilot",
            "--project",
            "test",
        ];
        args.extend(target);
        self.ok_with(session, &args).trim().to_string()
    }

    fn show(&self, id: &str) -> Value {
        self.json(&["run", "show", id])
    }

    fn human(&self, args: &[&str]) -> String {
        let mut full = args.to_vec();
        full.extend(["--project", "test", "--format", "human"]);
        self.ok(&full)
    }

    /// The `added:`/`modified:` lines of `rdm status`.
    fn status_lines(&self) -> Vec<String> {
        self.ok(&["status"])
            .lines()
            .filter(|l| l.starts_with("  added:") || l.starts_with("  modified:"))
            .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect()
    }
}

fn parse_time(v: &Value) -> chrono::DateTime<chrono::FixedOffset> {
    chrono::DateTime::parse_from_rfc3339(v.as_str().expect("timestamp is a string"))
        .expect("timestamp is RFC 3339")
}

#[test]
fn record_prints_the_minted_id_in_human_and_json_form() {
    let repo = Repo::new();

    let out = repo.run_with(
        Session::Unset,
        &[
            "run",
            "record",
            "--driver",
            "autopilot",
            "--roadmap",
            "alpha",
            "--project",
            "test",
        ],
    );
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(
        stdout.lines().count(),
        1,
        "human stdout is only the id: {stdout:?}"
    );
    let id = stdout.trim();
    assert_eq!(repo.show(id)["id"], id);

    let j = repo.json(&[
        "run",
        "record",
        "--driver",
        "dispatch-phase",
        "--task",
        "fix-bug",
    ]);
    let json_id = j["id"].as_str().unwrap();
    assert_eq!(repo.show(json_id)["task"], "fix-bug");
    let listed: Vec<Value> = repo.json(&["run", "list"]).as_array().unwrap().clone();
    let ids: Vec<&str> = listed.iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&id) && ids.contains(&json_id), "{ids:?}");
}

#[test]
fn record_needs_exactly_one_target() {
    let repo = Repo::new();
    for target in [&[][..], &["--roadmap", "alpha", "--task", "fix-bug"][..]] {
        let mut args = vec![
            "run",
            "record",
            "--driver",
            "autopilot",
            "--project",
            "test",
        ];
        args.extend(target);
        assert!(!repo.run_with(Session::Unset, &args).status.success());
    }
}

#[test]
fn record_stores_the_session_env_verbatim_the_flag_wins_and_neither_stores_none() {
    let repo = Repo::new();
    let sentinel = "0f3c9a1e-aaaa-4bbb-8ccc-dddddddddddd";

    let from_env = repo.record(Session::Set(sentinel), &["--roadmap", "alpha"]);
    assert_eq!(repo.show(&from_env)["session_uuid"], sentinel);

    let from_flag = repo.record(
        Session::Set(sentinel),
        &["--roadmap", "alpha", "--session-uuid", "other"],
    );
    assert_eq!(repo.show(&from_flag)["session_uuid"], "other");

    let out = repo.run_with(
        Session::Unset,
        &[
            "run",
            "record",
            "--driver",
            "autopilot",
            "--roadmap",
            "alpha",
            "--project",
            "test",
        ],
    );
    assert!(out.status.success());
    let none = String::from_utf8(out.stdout).unwrap().trim().to_string();
    assert!(repo.show(&none).get("session_uuid").is_none());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("cannot be joined to spend"),
        "a run with no session uuid says so on stderr"
    );
}

#[test]
fn unit_start_then_end_records_one_complete_entry() {
    let repo = Repo::new();
    let id = repo.record(Session::Unset, &["--roadmap", "alpha"]);

    let started = repo.human(&["run", "unit-start", &id, "--unit", "1"]);
    assert!(
        started.contains("Started phase-1-one (attempt 1)"),
        "{started}"
    );
    let ended = repo.human(&["run", "unit-end", &id, "--outcome", "reviewed"]);
    assert!(
        ended.contains("Ended phase-1-one (attempt 1): reviewed"),
        "{ended}"
    );

    let run = repo.show(&id);
    let units = run["units"].as_array().unwrap();
    assert_eq!(units.len(), 1);
    let unit = &units[0];
    assert_eq!(unit["unit"], "phase-1-one");
    assert_eq!(unit["attempt"], 1);
    assert_eq!(unit["outcome"], "reviewed");
    assert_eq!(unit["complete"], true);
    assert!(parse_time(&unit["started"]) <= parse_time(&unit["ended"]));

    let show = repo.human(&["run", "show", &id]);
    assert!(show.contains("phase-1-one (attempt 1)"), "{show}");
    assert!(show.contains("reviewed"), "{show}");
}

#[test]
fn two_attempts_of_one_stem_then_close_yields_ordinals_one_and_two() {
    let repo = Repo::new();
    let id = repo.record(Session::Unset, &["--roadmap", "alpha"]);
    for outcome in ["rework", "reviewed"] {
        repo.ok(&[
            "run",
            "unit-start",
            &id,
            "--unit",
            "phase-1-one",
            "--project",
            "test",
        ]);
        repo.ok(&[
            "run",
            "unit-end",
            &id,
            "--outcome",
            outcome,
            "--project",
            "test",
        ]);
    }
    repo.ok(&[
        "run",
        "close",
        &id,
        "--stop-reason",
        "done",
        "--project",
        "test",
    ]);

    let run = repo.show(&id);
    assert_eq!(run["status"], "closed");
    let units = run["units"].as_array().unwrap();
    assert_eq!(units.len(), 2);
    assert!(units.iter().all(|u| u["unit"] == "phase-1-one"));
    let attempts: Vec<u64> = units
        .iter()
        .map(|u| u["attempt"].as_u64().unwrap())
        .collect();
    assert_eq!(attempts, vec![1, 2]);
}

#[test]
fn close_sets_end_time_stop_reason_and_closed_or_abandoned() {
    let repo = Repo::new();
    let closed = repo.record(Session::Unset, &["--roadmap", "alpha"]);
    let out = repo.human(&["run", "close", &closed, "--stop-reason", "done"]);
    assert!(
        out.contains(&format!("Closed run {closed} (closed)")),
        "{out}"
    );
    let run = repo.show(&closed);
    assert_eq!(run["status"], "closed");
    assert_eq!(run["stop_reason"], "done");
    assert!(parse_time(&run["started"]) <= parse_time(&run["ended"]));
    assert_eq!(run["complete"], true);

    let abandoned = repo.record(Session::Unset, &["--task", "fix-bug"]);
    repo.ok(&[
        "run",
        "close",
        &abandoned,
        "--status",
        "abandoned",
        "--stop-reason",
        "interrupted",
        "--project",
        "test",
    ]);
    let run = repo.show(&abandoned);
    assert_eq!(run["status"], "abandoned");
    assert_eq!(run["stop_reason"], "interrupted");
    assert!(run.get("ended").is_some());

    // A terminal run takes no further writes, and says what to do instead.
    let again = repo.run_with(
        Session::Unset,
        &[
            "run",
            "close",
            &closed,
            "--stop-reason",
            "again",
            "--project",
            "test",
        ],
    );
    assert!(!again.status.success());
    assert!(String::from_utf8_lossy(&again.stderr).contains("rdm run record"));
}

#[test]
fn a_never_closed_run_loads_open_and_is_labelled_incomplete() {
    let repo = Repo::new();
    let id = repo.record(Session::Unset, &["--roadmap", "alpha"]);
    repo.ok(&["run", "unit-start", &id, "--unit", "2", "--project", "test"]);

    let run = repo.show(&id);
    assert_eq!(run["status"], "open");
    assert_eq!(run["complete"], false);
    assert_eq!(run["units"][0]["complete"], false);

    let show = repo.human(&["run", "show", &id]);
    assert!(
        show.contains("Status: open — incomplete (never closed)"),
        "{show}"
    );
    assert!(show.contains("phase-2-two (attempt 1)"), "{show}");
    assert!(show.contains("— incomplete"), "{show}");
    let list = repo.human(&["run", "list"]);
    assert!(list.contains("open (incomplete)"), "{list}");
}

#[test]
fn list_by_roadmap_in_human_and_json() {
    let repo = Repo::new();
    let a = repo.record(Session::Unset, &["--roadmap", "alpha"]);
    let b = repo.record(Session::Unset, &["--roadmap", "beta"]);
    let t = repo.record(Session::Unset, &["--task", "fix-bug"]);

    let listed = repo.json(&["run", "list", "--roadmap", "alpha"]);
    let ids: Vec<&str> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec![a.as_str()]);

    let human = repo.human(&["run", "list", "--roadmap", "alpha"]);
    assert!(human.contains(&a), "{human}");
    assert!(!human.contains(&b) && !human.contains(&t), "{human}");

    let tasks = repo.json(&["run", "list", "--task", "fix-bug"]);
    assert_eq!(tasks.as_array().unwrap().len(), 1);
    assert_eq!(tasks[0]["id"], t.as_str());
}

#[test]
fn a_run_whose_roadmap_was_deleted_still_loads() {
    let repo = Repo::new();
    let id = repo.record(Session::Unset, &["--roadmap", "alpha"]);
    repo.ok(&["roadmap", "delete", "alpha", "--force", "--project", "test"]);

    assert_eq!(repo.show(&id)["roadmap"], "alpha");
    let listed = repo.json(&["run", "list", "--roadmap", "alpha"]);
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["id"], id.as_str());
}

#[test]
fn a_path_escaping_run_id_is_reported_not_found_without_a_panic() {
    let repo = Repo::new();
    for args in [
        vec!["run", "show", "../x"],
        vec!["run", "unit-start", "../x", "--unit", "1"],
        vec!["run", "unit-end", "../x", "--outcome", "reviewed"],
        vec!["run", "close", "../x", "--stop-reason", "done"],
    ] {
        let mut full = args.clone();
        full.extend(["--project", "test"]);
        let out = repo.run_with(Session::Unset, &full);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "rdm {full:?}: {stderr}");
        assert!(!stderr.contains("panicked"), "rdm {full:?}: {stderr}");
        assert!(
            stderr.contains("run not found: ../x") && stderr.contains("rdm run list"),
            "rdm {full:?}: {stderr}"
        );
    }
}

#[test]
fn unit_errors_name_the_next_command() {
    let repo = Repo::new();
    let id = repo.record(Session::Unset, &["--roadmap", "alpha"]);
    let out = repo.run_with(
        Session::Unset,
        &[
            "run",
            "unit-end",
            &id,
            "--outcome",
            "reviewed",
            "--project",
            "test",
        ],
    );
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("rdm run unit-start"));

    repo.ok(&["run", "unit-start", &id, "--unit", "1", "--project", "test"]);
    let out = repo.run_with(
        Session::Unset,
        &["run", "unit-start", &id, "--unit", "2", "--project", "test"],
    );
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("rdm run unit-end"));
}

#[test]
fn every_run_mutation_is_staged_and_listed_by_status() {
    let repo = Repo::new();
    assert!(repo.status_lines().is_empty(), "seeded repo starts clean");

    let id = repo.record(Session::Unset, &["--roadmap", "alpha"]);
    let path = format!("projects/test/runs/{id}.md");
    assert_eq!(repo.status_lines(), vec![format!("added: {path}")]);
    repo.ok(&["commit", "-m", "record run"]);

    let steps: [&[&str]; 3] = [
        &["run", "unit-start", &id, "--unit", "1", "--project", "test"],
        &[
            "run",
            "unit-end",
            &id,
            "--outcome",
            "reviewed",
            "--project",
            "test",
        ],
        &[
            "run",
            "close",
            &id,
            "--stop-reason",
            "done",
            "--project",
            "test",
        ],
    ];
    for step in steps {
        repo.ok(step);
        assert_eq!(
            repo.status_lines(),
            vec![format!("modified: {path}")],
            "after {step:?}"
        );
        repo.ok(&["commit", "-m", "run step"]);
    }
}
