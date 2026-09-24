//! Session identity across real, separate processes: rung-2 lease sharing
//! within a long-lived parent and distinctness between parents, rung 1 and
//! rung 3 precedence, stale-lease rejection, invisibility of session state to
//! `rdm status` and a whole-tree commit, the resolution cost, the `Done:` hook
//! path, a harness id beating an inherited ancestor lease (§ J), and the
//! per-call wrapper topology (§ K). Ported from the retired
//! `scripts/verify-session-identity.sh`; the case map is
//! `docs/test-migration-inventory.md` § 9.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::support::{DriverRun, Plan, SessionOut, ShellDriver, must, real_bin};

const SESSION_ID: &[&str] = &["session", "id", "--format", "json"];

/// Section J's scenario: one bare invocation from the test process plants an
/// ancestor lease at the test's pid; two drivers under it, each with its own
/// `CLAUDE_CODE_SESSION_ID`, then run `session id` `repeats` times.
pub struct AncestorLease {
    /// The id the plant invocation resolved.
    pub planted: SessionOut,
    /// Lease files right after the plant.
    pub leases_after_plant: usize,
    /// Child one's resolutions.
    pub one: Vec<SessionOut>,
    /// Child two's resolutions.
    pub two: Vec<SessionOut>,
    /// Lease files after both children ran.
    pub leases_after: usize,
}

/// Runs section J's scenario against `bin`.
pub fn ancestor_lease(bin: &Path, repeats: usize) -> AncestorLease {
    let plan = Plan::demo(bin);
    let plant = plan.run_bare(SESSION_ID);
    assert!(
        plant.success(),
        "the plant invocation failed: {}",
        plant.describe()
    );
    let planted = must(SessionOut::parse(&plant.stdout));
    let leases_after_plant = plan.leases().len();

    let child = |value: &str| {
        let mut d = ShellDriver::new().env("CLAUDE_CODE_SESSION_ID", value);
        for _ in 0..repeats {
            d = d.direct(SESSION_ID);
        }
        must(d.spawn(&plan, value))
    };
    let (one, two) = (child("child-one"), child("child-two"));
    let (one, two) = (must(one.finish()), must(two.finish()));
    let ids = |run: &DriverRun| {
        assert!(
            run.steps.iter().all(|s| s.status == 0),
            "a child invocation failed: {run:?}"
        );
        must(run.sessions())
    };
    AncestorLease {
        planted,
        leases_after_plant,
        one: ids(&one),
        two: ids(&two),
        leases_after: plan.leases().len(),
    }
}

/// Section K2's scenario: a task create and a commit, each in its own
/// wrapper shell under one driver, with no session variable anywhere.
pub fn fragmented_commit(bin: &Path) -> (Plan, DriverRun) {
    let plan = Plan::demo(bin);
    let run = must(
        ShellDriver::new()
            .wrapped(&[
                "task",
                "create",
                "k-item",
                "--title",
                "K item",
                "--body",
                "Body.",
                "--no-edit",
                "--project",
                "demo",
            ])
            .wrapped(&["commit", "-m", "k: land the wrapper batch"])
            .run(&plan, "k2"),
    );
    (plan, run)
}

/// `n` wrapped `session id` calls under one fresh driver.
pub fn wrapped_ids(plan: &Plan, n: usize, label: &str) -> DriverRun {
    let mut d = ShellDriver::new();
    for _ in 0..n {
        d = d.wrapped(SESSION_ID);
    }
    must(d.run(plan, label))
}

fn distinct(ids: &[SessionOut]) -> BTreeSet<&str> {
    ids.iter().map(|s| s.id.as_str()).collect()
}

/// Sections A and B: two concurrent bare drivers, three `session id` calls
/// each. Each is stable across its three processes and distinct from the
/// other; every call is on rung 2; exactly two lease files exist, one per
/// driver pid, each recording its driver's id.
#[test]
fn bare_shells_are_stable_within_and_distinct_between() {
    let plan = Plan::demo(&real_bin());
    let spawn = |label| {
        must(
            ShellDriver::new()
                .direct(SESSION_ID)
                .direct(SESSION_ID)
                .direct(SESSION_ID)
                .spawn(&plan, label),
        )
    };
    let (a, b) = (spawn("seq-a"), spawn("seq-b"));
    let (a, b) = (must(a.finish()), must(b.finish()));
    let (ids_a, ids_b) = (must(a.sessions()), must(b.sessions()));
    assert_eq!(ids_a.len(), 3);
    assert_eq!(
        distinct(&ids_a).len(),
        1,
        "A's ids differ across processes: {ids_a:?}"
    );
    assert_eq!(
        distinct(&ids_b).len(),
        1,
        "B's ids differ across processes: {ids_b:?}"
    );
    assert_ne!(
        ids_a[0].id, ids_b[0].id,
        "two concurrent parents merged onto one id"
    );
    assert!(
        ids_a.iter().chain(&ids_b).all(|s| s.rung == 2),
        "every bare invocation must resolve on rung 2: {ids_a:?} {ids_b:?}"
    );
    let leases = plan.leases();
    let expected: BTreeMap<u32, String> =
        [(a.pid, ids_a[0].id.clone()), (b.pid, ids_b[0].id.clone())]
            .into_iter()
            .collect();
    assert_eq!(
        leases, expected,
        "exactly one lease per driver pid, recording its id"
    );
}

/// Section B's degenerate parent: `rdm` invoked directly by the test process,
/// whose ancestry is arbitrary, still resolves and exits 0.
#[test]
fn a_direct_invocation_still_resolves() {
    let plan = Plan::demo(&real_bin());
    let out = plan.run_bare(&["session", "id"]);
    assert!(out.success(), "{}", out.describe());
    assert!(
        !out.stdout.trim().is_empty(),
        "a direct invocation printed no id"
    );
}

/// Section C: two unrelated drivers sharing `CLAUDE_CODE_SESSION_ID` agree on
/// one rung-3 id; a different value derives a different id; `RDM_SESSION`
/// wins (rung 1); and rung 3 creates no lease.
#[test]
fn harness_variable_is_leaseless_rung3_and_rdm_session_wins() {
    let plan = Plan::demo(&real_bin());
    let resolve = |label: &str, value: &str, explicit: Option<&str>| {
        let mut d = ShellDriver::new().env("CLAUDE_CODE_SESSION_ID", value);
        if let Some(e) = explicit {
            d = d.env("RDM_SESSION", e);
        }
        let run = must(d.direct(SESSION_ID).run(&plan, label));
        must(run.steps[0].session())
    };
    let c1 = resolve("c1", "abc123", None);
    let c2 = resolve("c2", "abc123", None);
    assert_eq!(
        c1.id, c2.id,
        "one harness value from two parents must agree"
    );
    assert_eq!(c1.rung, 3);
    let c3 = resolve("c3", "different-value", None);
    assert_ne!(
        c3.id, c1.id,
        "a different harness value must derive a different id"
    );
    let c4 = resolve("c4", "abc123", Some("explicit-1"));
    assert_eq!((c4.id.as_str(), c4.rung), ("explicit-1", 1));
    assert!(plan.leases().is_empty(), "rung 3 created a lease");
}

/// Section D: a lease naming the live parent with a stale start time (what a
/// recycled pid looks like) is neither adopted nor left on disk.
#[test]
fn stale_lease_on_a_live_parent_is_neither_adopted_nor_kept() {
    const SENTINEL: &str = "s-deadbeefdeadbeef";
    let plan = Plan::demo(&real_bin());
    let mut driver = must(
        ShellDriver::new()
            .gate()
            .direct(SESSION_ID)
            .spawn(&plan, "recycle"),
    );
    let pid = driver.pid();
    std::fs::create_dir_all(plan.leases_dir()).unwrap();
    std::fs::write(
        plan.leases_dir().join(format!("{pid}.lease")),
        format!(
            r#"{{"id":"{SENTINEL}","start_time":"not-the-live-start-time","created_utc":"2026-01-01T00:00:00Z"}}"#
        ),
    )
    .unwrap();
    must(driver.release_gate());
    let run = must(driver.finish());
    let resolved = must(run.steps[0].session());
    assert_ne!(resolved.id, SENTINEL, "a stale lease was adopted");
    assert_ne!(
        plan.leases().get(&pid).map(String::as_str),
        Some(SENTINEL),
        "the stale lease survived on disk"
    );
}

/// Section G: a bare driver's mutation leaves `rdm status` naming no session
/// state, and even a whole-tree commit sweeps up no lease or journal.
#[test]
fn whole_tree_commit_never_sweeps_session_state() {
    let plan = Plan::demo(&real_bin());
    let run = must(
        ShellDriver::new()
            .direct(&[
                "task",
                "create",
                "gamma-one",
                "--title",
                "Gamma",
                "--body",
                "Body.",
                "--no-edit",
                "--project",
                "demo",
            ])
            .direct(&["status"])
            .run(&plan, "g"),
    );
    assert!(run.steps.iter().all(|s| s.status == 0), "{run:?}");
    assert!(
        plan.changesets_dir().is_dir(),
        "no journal was written, so this test is vacuous"
    );
    let status = run.steps[1].all();
    for needle in ["leases", "changesets", ".git"] {
        assert!(
            !status.contains(needle),
            "rdm status named {needle}: {status}"
        );
    }
    plan.ok(Some("seed"), &["commit", "--all", "-m", "add gamma-one"]);
    let tree = plan.tree("HEAD");
    assert!(
        tree.contains("projects/demo/tasks/gamma-one.md"),
        "{tree:?}"
    );
    let swept: Vec<_> = tree
        .iter()
        .filter(|p| {
            p.contains("rdm/leases")
                || p.contains("rdm/changesets")
                || p.ends_with(".lease")
                || p.ends_with(".jsonl")
        })
        .collect();
    assert!(
        swept.is_empty(),
        "a whole-tree commit swept session state: {swept:?}"
    );
    assert_eq!(plan.porcelain(), "", "git status is dirty after the commit");
}

/// Section H: 20 fresh `session id` resolutions each stay far inside the
/// 30 s default hook deadline (bound: 250 ms).
#[test]
fn resolution_cost_stays_far_under_the_hook_deadline() {
    let plan = Plan::demo(&real_bin());
    let micros: Vec<u64> = (0..20)
        .map(|_| {
            let out = plan.run_bare(SESSION_ID);
            assert!(out.success(), "{}", out.describe());
            must(SessionOut::parse(&out.stdout)).resolve_micros
        })
        .collect();
    let max = micros.iter().copied().max().unwrap();
    eprintln!("max resolve_micros over 20 fresh invocations: {max} µs");
    assert!(
        max < 250_000,
        "resolution cost {max} µs exceeds the 250000 µs bound"
    );
}

/// Section H2: a real `Done:` commit, then `rdm hook post-commit` from inside
/// the repo, applies the directive and logs a `batch-commit` naming its
/// changeset and at least one landed path.
#[test]
fn hook_post_commit_resolves_identity_and_lands_its_batch() {
    let plan = Plan::demo(&real_bin());
    plan.ok(
        Some("seed"),
        &[
            "task",
            "create",
            "hooked",
            "--title",
            "Hooked",
            "--body",
            "Body.",
            "--no-edit",
            "--project",
            "demo",
        ],
    );
    plan.ok(Some("seed"), &["commit", "-m", "add hooked"]);
    plan.git_commit_empty("chore: land\n\nDone: task/hooked");
    let mut hook = plan.cmd(None, &["hook", "post-commit"]);
    hook.current_dir(&plan.root);
    let out = plan.output(hook);
    assert!(out.success(), "{}", out.describe());
    assert_eq!(plan.task("demo", "hooked")["status"], "done");
    let log = std::fs::read_to_string(plan.root.join(".git/rdm-hook.log")).unwrap_or_default();
    let line = log
        .lines()
        .find(|l| l.contains("post-commit batch-commit"))
        .unwrap_or_else(|| panic!("the hook logged no batch-commit event: {log}"));
    let field = |key: &str| {
        line.split_whitespace()
            .find_map(|w| w.strip_prefix(key))
            .unwrap_or_default()
            .to_owned()
    };
    assert!(!field("changeset=").is_empty(), "no changeset in {line}");
    assert!(
        field("paths=").parse::<u32>().is_ok_and(|n| n >= 1),
        "the batch landed no path: {line}"
    );
}

/// Section J: children of an already-leased ancestor, each with its own
/// harness id, resolve their own stable rung-3 ids and never the ancestor's
/// lease; no lease is created for them.
#[test]
fn harness_id_beats_an_inherited_ancestor_lease() {
    let j = ancestor_lease(&real_bin(), 2);
    assert_eq!(j.planted.rung, 2, "the plant must land on rung 2");
    assert_eq!(j.leases_after_plant, 1);
    assert_eq!(
        distinct(&j.one).len(),
        1,
        "child one is unstable: {:?}",
        j.one
    );
    assert_eq!(
        distinct(&j.two).len(),
        1,
        "child two is unstable: {:?}",
        j.two
    );
    assert_ne!(j.one[0].id, j.two[0].id, "two distinct harness ids merged");
    assert!(j.one.iter().chain(&j.two).all(|s| s.rung == 3));
    assert!(
        j.one[0].id != j.planted.id && j.two[0].id != j.planted.id,
        "a child adopted the ancestor lease"
    );
    assert_eq!(j.leases_after, 1, "rung 3 created a lease");
}

/// Sections K0 and K1: per-call wrapper shells are distinct live processes
/// under one driver, and each call bootstraps its own rung-2 changeset (the
/// recorded, kept fragmentation).
#[test]
fn wrapper_calls_fragment_into_bootstrapped_rung2_changesets() {
    let plan = Plan::demo(&real_bin());
    let run = wrapped_ids(&plan, 5, "k1");
    let wrappers: Vec<(u32, u32)> = run.steps.iter().map(|s| s.wrapper.unwrap()).collect();
    let pids: BTreeSet<u32> = wrappers.iter().map(|w| w.0).collect();
    assert_eq!(pids.len(), 5, "wrapper pids are not distinct: {wrappers:?}");
    assert!(
        wrappers.iter().all(|w| w.1 == run.pid),
        "every wrapper's parent must be the one driver ({}): {wrappers:?}",
        run.pid
    );
    let ids = must(run.sessions());
    assert_eq!(
        distinct(&ids).len(),
        5,
        "expected 5 fragmented ids: {ids:?}"
    );
    assert!(
        ids.iter().all(|s| s.rung == 2 && s.lease_bootstrapped),
        "{ids:?}"
    );
}

/// Section K2: a fragmented commit exits 0, names the cause and the remedy,
/// lands nothing — and the work stays recoverable by `commit --changeset`.
#[test]
fn fragmented_commit_names_cause_and_remedy_and_stays_recoverable() {
    let (plan, run) = fragmented_commit(&real_bin());
    assert!(run.steps.iter().all(|s| s.status == 0), "{run:?}");
    let commit = run.steps[1].all();
    for needle in [
        "CLAUDE_CODE_SESSION_ID",
        "RDM_HARNESS_SESSION_ID",
        "Cause:",
        "Remedy:",
    ] {
        assert!(
            commit.contains(needle),
            "the commit output lacks {needle}: {commit}"
        );
    }
    assert!(
        !plan
            .commit_files("HEAD")
            .contains("projects/demo/tasks/k-item.md"),
        "the wrapper commit landed the task, so continuity now works"
    );
    let listed = plan.json(&["session", "list", "--format", "json"]);
    let orphan = listed[0]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("no changeset was recorded: {listed}"))
        .to_owned();
    plan.ok(
        Some("recover"),
        &["commit", "--changeset", &orphan, "-m", "k: recover"],
    );
    assert!(
        plan.commit_files("HEAD")
            .contains("projects/demo/tasks/k-item.md")
    );
}

/// Sections K3 and K3b: seven wrapped calls leave at most two leases, and a
/// dead wrapper's lease is swept by the next call.
#[test]
fn dead_wrapper_leases_are_swept_and_bounded() {
    let plan = Plan::demo(&real_bin());
    let run = wrapped_ids(&plan, 7, "k3");
    let leases = plan.leases();
    assert!(
        (1..=2).contains(&leases.len()),
        "7 wrapped calls left {} leases: {leases:?}",
        leases.len()
    );
    let wrappers: BTreeSet<u32> = run.steps.iter().map(|s| s.wrapper.unwrap().0).collect();
    let (&dead, _) = leases.iter().next().unwrap();
    assert!(
        wrappers.contains(&dead),
        "the observed lease ({dead}) is not a finished wrapper's: {wrappers:?}"
    );
    wrapped_ids(&plan, 1, "k3b");
    let after = plan.leases();
    assert!(
        !after.contains_key(&dead),
        "the dead wrapper's lease survived"
    );
    assert!(after.len() <= 2, "leases grew to {after:?}");
}

/// Section K4: two concurrent wrapper drivers never share an id.
#[test]
fn concurrent_wrapper_drivers_never_share_an_id() {
    let plan = Plan::demo(&real_bin());
    let spawn = |label| {
        must(
            ShellDriver::new()
                .wrapped(SESSION_ID)
                .wrapped(SESSION_ID)
                .spawn(&plan, label),
        )
    };
    let (a, b) = (spawn("k4a"), spawn("k4b"));
    let (a, b) = (must(a.finish()), must(b.finish()));
    let (a, b) = (must(a.sessions()), must(b.sessions()));
    let shared: Vec<_> = distinct(&a).intersection(&distinct(&b)).copied().collect();
    assert!(shared.is_empty(), "two drivers shared {shared:?}");
}

/// Section K5: the remedy the advisory prints — `RDM_HARNESS_SESSION_ID` on
/// the driver — gives wrapper calls one rung-3 changeset, a landing commit,
/// no advisory and no lease.
#[test]
fn harness_adoption_var_gives_wrappers_one_changeset() {
    let plan = Plan::demo(&real_bin());
    let run = must(
        ShellDriver::new()
            .env("RDM_HARNESS_SESSION_ID", "k5-agent-run")
            .wrapped(SESSION_ID)
            .wrapped(&[
                "task",
                "create",
                "k5-item",
                "--title",
                "K5 item",
                "--body",
                "Body.",
                "--no-edit",
                "--project",
                "demo",
            ])
            .wrapped(&["commit", "-m", "k5: land under one changeset"])
            .wrapped(SESSION_ID)
            .run(&plan, "k5"),
    );
    assert!(run.steps.iter().all(|s| s.status == 0), "{run:?}");
    let ids = [must(run.steps[0].session()), must(run.steps[3].session())];
    assert_eq!(ids[0].id, ids[1].id);
    assert!(ids.iter().all(|s| s.rung == 3), "{ids:?}");
    assert!(
        plan.commit_files("HEAD")
            .contains("projects/demo/tasks/k5-item.md")
    );
    assert!(
        !run.all().contains("RDM_HARNESS_SESSION_ID=<"),
        "the advisory fired at a caller that has continuity"
    );
    assert!(plan.leases().is_empty(), "the remedy path created a lease");
}
