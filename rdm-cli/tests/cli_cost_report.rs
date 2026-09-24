//! `rdm cost report --roadmap <slug>` over the checked-in fixture plan repo
//! `tests/fixtures/cost-report/plan/`, joined to Phase 2's session fixture
//! `tests/fixtures/cost-session/home/.claude/`, which is read, never written.
//!
//! Each test copies the fixture plan repo into a temp dir and initialises it
//! as a git repository, so no read touches the checked-in tree. Every
//! invocation points `HOME` and `CLAUDE_CONFIG_DIR` at the session fixture,
//! clears `CLAUDE_CODE_SESSION_ID`, `RDM_ROOT`, `RDM_PROJECT` and the git
//! variables that would redirect the store to another repository, because
//! the suite often runs inside a Claude Code session or a git hook. The
//! goldens under `tests/fixtures/cost-report/expected/` are written only by
//! the ignored [`bless_cost_report_goldens`] case, which runs the binary; no
//! token figure is typed into this file.

use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

const S1: &str = "11111111-2222-4333-8444-555555555555";
const S2: &str = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";
const BLESS: &str = "cargo nextest run -p rdm-cli --test cli_cost_report --run-ignored only -E 'test(=bless_cost_report_goldens)'";

/// Variables that would point the store at a different repository or inject
/// git configuration.
const GIT_REDIRECT_VARS: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_COMMON_DIR",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_PREFIX",
    "GIT_NAMESPACE",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
];

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("rdm-cli has a parent directory")
        .join("tests/fixtures")
}

fn config_dir() -> PathBuf {
    fixtures().join("cost-session/home/.claude")
}

fn expected_dir() -> PathBuf {
    fixtures().join("cost-report/expected")
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("create dir");
    for entry in fs::read_dir(from).expect("read fixture dir") {
        let entry = entry.expect("fixture entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).expect("copy fixture file");
        }
    }
}

/// A private copy of the fixture plan repo, initialised as a git repository.
struct PlanRepo {
    dir: TempDir,
    state: TempDir,
}

impl PlanRepo {
    fn new() -> Self {
        let repo = PlanRepo {
            dir: TempDir::new().expect("tempdir"),
            state: TempDir::new().expect("tempdir"),
        };
        copy_tree(&fixtures().join("cost-report/plan"), &repo.root());
        gix::init(repo.root()).expect("git init the plan repo copy");
        repo
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join("plan")
    }

    /// `rdm` with an isolated environment and no `--root`.
    fn bare(&self) -> Command {
        let mut cmd = Command::cargo_bin("rdm").expect("rdm binary");
        for var in GIT_REDIRECT_VARS {
            cmd.env_remove(var);
        }
        cmd.env_remove("CLAUDE_CODE_SESSION_ID")
            .env_remove("RDM_ROOT")
            .env_remove("RDM_PROJECT")
            .env("RDM_SESSION", "cli-cost-report-test")
            .env("XDG_STATE_HOME", self.state.path())
            .env("XDG_CONFIG_HOME", "/dev/null/nonexistent")
            .env("HOME", fixtures().join("cost-session/home"))
            .env("CLAUDE_CONFIG_DIR", config_dir());
        cmd
    }

    /// `rdm` rooted at this plan repo.
    fn rdm(&self) -> Command {
        let mut cmd = self.bare();
        cmd.arg("--root").arg(self.root());
        cmd
    }
}

struct Out {
    ok: bool,
    stdout: String,
    stderr: String,
}

fn run(cmd: &mut Command) -> Out {
    let out = cmd.output().expect("run rdm");
    Out {
        ok: out.status.success(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

const REPORT: [&str; 6] = [
    "cost",
    "report",
    "--roadmap",
    "demo",
    "--project",
    "fixture",
];

fn report_json(repo: &PlanRepo) -> Value {
    let r = run(repo.rdm().args(REPORT).args(["--format", "json"]));
    assert!(r.ok, "rdm cost report failed: {}", r.stderr);
    assert!(
        r.stderr.is_empty(),
        "json stderr must be empty: {}",
        r.stderr
    );
    serde_json::from_str(&r.stdout).expect("stdout is JSON")
}

/// Replaces the session fixture's Claude data directory, raw and canonical,
/// with a placeholder so the goldens do not depend on the checkout path.
fn redact(text: &str) -> String {
    let raw = config_dir().display().to_string();
    let canonical = fs::canonicalize(config_dir())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| raw.clone());
    let mut forms = [raw, canonical];
    forms.sort_by_key(|f| std::cmp::Reverse(f.len()));
    forms.iter().fold(text.to_owned(), |t, f| {
        t.replace(f.as_str(), "<CLAUDE_CONFIG_DIR>")
    })
}

/// The two goldens, captured from the binary and redacted.
fn capture() -> Vec<(&'static str, String)> {
    let repo = PlanRepo::new();
    let j = run(repo.rdm().args(REPORT).args(["--format", "json"]));
    assert!(j.ok, "rdm cost report --format json failed: {}", j.stderr);
    let h = run(repo.rdm().args(REPORT));
    assert!(h.ok, "rdm cost report failed: {}", h.stderr);
    vec![
        ("report.json", redact(&j.stdout)),
        (
            "report.txt",
            redact(&format!(
                "--- stdout ---\n{}--- stderr ---\n{}",
                h.stdout, h.stderr
            )),
        ),
    ]
}

#[test]
fn output_matches_committed_goldens() {
    for (name, captured) in capture() {
        let path = expected_dir().join(name);
        let committed = fs::read_to_string(&path).unwrap_or_default();
        assert!(
            captured == committed,
            "rdm cost report output drifted from {}.\n--- captured ---\n{captured}\n--- committed ---\n{committed}\nIf the change is intentional, re-bless with:\n  {BLESS}\nand review `git diff tests/fixtures/cost-report/expected/`.",
            path.display()
        );
    }
}

/// Rewrites the goldens from the binary. Run deliberately (see [`BLESS`]),
/// then review the diff.
#[test]
#[ignore = "writes tests/fixtures/cost-report/expected/; run deliberately to re-bless"]
fn bless_cost_report_goldens() {
    fs::create_dir_all(expected_dir()).expect("create expected dir");
    for (name, text) in capture() {
        fs::write(expected_dir().join(name), text).expect("write golden");
    }
}

const CLASSES: [&str; 7] = [
    "requests",
    "input",
    "output",
    "cache_write_5m",
    "cache_write_1h",
    "cache_read",
    "total",
];

fn class(v: &Value, key: &str) -> u64 {
    v[key]
        .as_u64()
        .unwrap_or_else(|| panic!("{key} is not a number in {v}"))
}

/// Asserts, per token class, that `parts` sum to `whole`.
fn assert_sums(parts: &[&Value], whole: &Value, what: &str) {
    for key in CLASSES {
        let sum: u64 = parts.iter().map(|p| class(p, key)).sum();
        assert_eq!(sum, class(whole, key), "{what}: {key}");
    }
}

fn array<'a>(v: &'a Value, key: &str) -> &'a Vec<Value> {
    v[key]
        .as_array()
        .unwrap_or_else(|| panic!("{key} is not an array"))
}

fn run_by_id<'a>(report: &'a Value, id: &str) -> &'a Value {
    array(report, "runs")
        .iter()
        .find(|r| r["id"] == id)
        .unwrap_or_else(|| panic!("no run {id}"))
}

#[test]
fn json_report_joins_runs_and_holds_the_accounting_identities() {
    let repo = PlanRepo::new();
    let report = report_json(&repo);
    assert_eq!(report["project"], "fixture");
    assert_eq!(report["roadmap"], "demo");

    // Join states: the missing run is reported while the others join.
    let a = run_by_id(&report, "2026-09-24-1000-aaaa");
    let b = run_by_id(&report, "2026-09-24-1009-bbbb");
    let c = run_by_id(&report, "2026-09-24-1200-cccc");
    let d = run_by_id(&report, "2026-09-24-1300-dddd");
    let e = run_by_id(&report, "2026-09-24-1400-eeee");
    for joined in [a, b, c] {
        assert_eq!(joined["join"]["state"], "joined");
        assert!(class(&joined["totals"], "total") > 0);
    }
    assert_eq!(d["join"]["state"], "missing");
    assert!(
        d["join"]["reason"]
            .as_str()
            .is_some_and(|r| r.contains("99999999-0000-4000-8000-000000000000")),
        "{}",
        d["join"]
    );
    assert_eq!(e["join"]["state"], "unjoinable");
    assert_eq!(c["status_label"], "open (incomplete)");
    assert_eq!(c["complete"], false);

    // Per run: per-unit + overhead + unattributed == run total.
    for r in array(&report, "runs") {
        let mut parts: Vec<&Value> = array(r, "units").iter().map(|u| &u["usage"]).collect();
        parts.push(&r["overhead"]);
        parts.push(&r["unattributed"]);
        assert_sums(&parts, &r["totals"], "run identity");
    }

    // Per session: its runs + its unattributed + its excluded == its total.
    for s in array(&report, "sessions") {
        assert_sums(
            &[
                &s["attributed_to_runs"],
                &s["unattributed"],
                &s["excluded"]["usage"],
            ],
            &s["totals"],
            "session conservation",
        );
    }

    // The buckets and the per-phase rows sum to the roadmap total.
    let buckets = &report["buckets"];
    assert_sums(
        &[
            &buckets["phases"],
            &buckets["overhead"],
            &buckets["unattributed"],
            &buckets["session_unattributed"],
        ],
        &report["totals"],
        "buckets",
    );
    let phases: Vec<&Value> = array(&report, "phases")
        .iter()
        .map(|p| &p["usage"])
        .collect();
    assert_sums(&phases, &buckets["phases"], "phases");

    // The rework pair lands on one stem; the missing run's unit is absent.
    let stems: Vec<&str> = array(&report, "phases")
        .iter()
        .filter_map(|p| p["stem"].as_str())
        .collect();
    assert!(stems.contains(&"phase-1-parser"));
    assert!(!stems.contains(&"phase-4-release"));
    let parser = array(&report, "phases")
        .iter()
        .find(|p| p["stem"] == "phase-1-parser")
        .expect("phase-1-parser row");
    assert_eq!(parser["attempts"], 2);

    // Two runs share S1: each unanchored source is listed once.
    let session_items: Vec<&str> = array(&report, "session_unattributed")
        .iter()
        .filter_map(|i| i["id"].as_str())
        .collect();
    assert_eq!(session_items, ["a4", "x1"]);

    // Nothing is dropped or double counted: the roadmap total plus the
    // excluded spend is everything `rdm cost` reports for the two sessions.
    let mut sessions = Vec::new();
    for id in [S1, S2] {
        let r = run(repo
            .rdm()
            .args(["cost", "--session", id, "--format", "json"]));
        assert!(r.ok, "{}", r.stderr);
        sessions.push(serde_json::from_str::<Value>(&r.stdout).expect("JSON"));
    }
    let whole: Value = serde_json::json!({
        "requests": class(&sessions[0]["totals"], "requests") + class(&sessions[1]["totals"], "requests"),
        "input": class(&sessions[0]["totals"], "input") + class(&sessions[1]["totals"], "input"),
        "output": class(&sessions[0]["totals"], "output") + class(&sessions[1]["totals"], "output"),
        "cache_write_5m": class(&sessions[0]["totals"], "cache_write_5m") + class(&sessions[1]["totals"], "cache_write_5m"),
        "cache_write_1h": class(&sessions[0]["totals"], "cache_write_1h") + class(&sessions[1]["totals"], "cache_write_1h"),
        "cache_read": class(&sessions[0]["totals"], "cache_read") + class(&sessions[1]["totals"], "cache_read"),
        "total": class(&sessions[0]["totals"], "total") + class(&sessions[1]["totals"], "total"),
    });
    assert_sums(
        &[&report["totals"], &report["excluded"]["usage"]],
        &whole,
        "roadmap total + excluded == both sessions",
    );
    assert!(class(&report["excluded"]["usage"], "total") > 0);

    // Reap warnings are inside the JSON, stderr stayed empty.
    assert!(!array(&report, "warnings").is_empty());
}

#[test]
fn human_report_puts_warnings_on_stderr() {
    let repo = PlanRepo::new();
    let r = run(repo.rdm().args(REPORT));
    assert!(r.ok, "{}", r.stderr);
    assert!(
        r.stderr
            .lines()
            .all(|l| l.starts_with(&format!("warning: {S1}: "))),
        "{}",
        r.stderr
    );
    assert!(!r.stderr.is_empty());
}

#[test]
fn a_roadmap_with_no_runs_reports_so_and_succeeds() {
    let repo = PlanRepo::new();
    let r = run(repo.rdm().args([
        "cost",
        "report",
        "--roadmap",
        "elsewhere",
        "--format",
        "json",
    ]));
    assert!(r.ok, "{}", r.stderr);
    let report: Value = serde_json::from_str(&r.stdout).expect("JSON");
    assert_eq!(report["counts"]["runs"], 0);
    assert!(array(&report, "runs").is_empty());

    let human = run(repo
        .rdm()
        .args(["cost", "report", "--roadmap", "elsewhere"]));
    assert!(human.ok, "{}", human.stderr);
    assert!(
        human
            .stdout
            .contains("No runs recorded for this roadmap.\n"),
        "{}",
        human.stdout
    );
}

#[test]
fn the_project_resolves_from_the_plan_repo_config() {
    let repo = PlanRepo::new();
    let explicit = report_json(&repo);
    let r = run(repo
        .rdm()
        .args(["cost", "report", "--roadmap", "demo", "--format", "json"]));
    assert!(r.ok, "{}", r.stderr);
    assert_eq!(
        serde_json::from_str::<Value>(&r.stdout).expect("JSON"),
        explicit
    );
}

#[test]
fn report_needs_a_plan_repo_and_rejects_session_selectors() {
    let repo = PlanRepo::new();
    let empty = TempDir::new().expect("tempdir");
    let r = run(repo.bare().arg("--root").arg(empty.path()).args([
        "cost",
        "report",
        "--roadmap",
        "demo",
    ]));
    assert!(!r.ok);
    assert!(r.stderr.contains("no plan repo"), "{}", r.stderr);

    let r = run(repo
        .rdm()
        .args(["cost", "--session", S1, "report", "--roadmap", "demo"]));
    assert!(!r.ok);
}
