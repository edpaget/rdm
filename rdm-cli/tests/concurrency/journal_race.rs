//! The changeset journal under concurrent writers, across real processes that
//! share one `RDM_SESSION`: a 40-way create fan-out racing six commits, and
//! the four barrier interleavings — a commit parked inside `truncate`, a
//! `session gc` arriving while an append is parked, an append arriving while
//! a compaction is parked, and an append during a parked `session discard`.
//! Each scenario function returns what it observed, so the fixed test here
//! and its negative control in [`crate::mutants`] drive identical steps and
//! differ only in what they assert. Ported from the retired
//! `scripts/verify-journal-truncation-race.sh`; the case map is
//! `docs/test-migration-inventory.md` § 9.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use crate::support::{EXIT_DEADLINE, Finished, Plan, Proc, must, real_bin, wait_until};

/// The one changeset every writer in these scenarios shares.
pub const SHARED: &str = "shared-changeset";
/// The fan-out width.
pub const N_MUTATIONS: usize = 40;
/// Commits interleaved into the fan-out.
pub const M_COMMITS: usize = 6;

const JOURNAL_BARRIER: &str = "RDM_HARNESS_JOURNAL_BARRIER";
const APPEND_BARRIER: &str = "RDM_HARNESS_APPEND_BARRIER";
const COMPACT_BARRIER: &str = "RDM_HARNESS_COMPACT_BARRIER";

/// Two projects (`alpha` default, `beta`), each with one committed task.
pub fn seed(bin: &Path) -> Plan {
    Plan::seeded(
        bin,
        "alpha",
        &[
            &["project", "create", "beta", "--title", "Beta"],
            &[
                "task",
                "create",
                "seed-alpha",
                "--title",
                "Seed alpha",
                "--body",
                "Body.",
                "--no-edit",
                "--project",
                "alpha",
            ],
            &[
                "task",
                "create",
                "seed-beta",
                "--title",
                "Seed beta",
                "--body",
                "Body.",
                "--no-edit",
                "--project",
                "beta",
            ],
        ],
    )
}

/// `task create <slug>` in the shared changeset.
pub fn create(plan: &Plan, slug: &str, project: &str) -> Command {
    plan.cmd(
        Some(SHARED),
        &[
            "task",
            "create",
            slug,
            "--title",
            slug,
            "--body",
            "Body.",
            "--no-edit",
            "--project",
            project,
        ],
    )
}

/// The project stress task `i` lives in.
pub fn stress_project(i: usize) -> &'static str {
    if i.is_multiple_of(2) { "alpha" } else { "beta" }
}

/// The path of stress task `i`.
pub fn stress_path(i: usize) -> String {
    format!("projects/{}/tasks/stress-{i}.md", stress_project(i))
}

/// Waits for every child in `procs` against one overall bound.
fn finish_all(procs: Vec<Proc>, bound: Duration) -> Vec<Finished> {
    let until = std::time::Instant::now() + bound;
    procs
        .into_iter()
        .map(|mut p| {
            let left = until.saturating_duration_since(std::time::Instant::now());
            must(p.wait_bounded(left))
        })
        .collect()
}

/// A settling commit in the shared changeset, required to succeed.
pub fn settle(plan: &Plan, message: &str) -> Finished {
    let out = plan.run(Some(SHARED), &["commit", "-m", message]);
    assert!(
        out.success(),
        "the settling commit failed: {}",
        out.describe()
    );
    out
}

/// Section 1b's scenario against `bin`: a commit parked inside `truncate`
/// while the whole create fan-out runs to completion, then a settling commit.
/// Returns the repo.
pub fn parked_fanout(bin: &Path) -> Plan {
    let plan = seed(bin);
    assert!(plan.output(create(&plan, "parked-item", "alpha")).success());
    let commit = plan.cmd(Some(SHARED), &["commit", "-m", "P: land the staged item"]);
    let p = must(plan.park(JOURNAL_BARRIER, "commit-p", commit));
    let creates: Vec<Proc> = (1..=N_MUTATIONS)
        .map(|i| {
            let slug = format!("stress-{i}");
            must(plan.spawn(&slug, create(&plan, &slug, stress_project(i))))
        })
        .collect();
    for f in finish_all(creates, Duration::from_secs(180)) {
        assert!(f.success(), "a fan-out create failed: {}", f.describe());
    }
    let parked = must(p.release_and_finish());
    assert!(
        parked.success(),
        "the parked commit failed: {}",
        parked.describe()
    );
    settle(
        &plan,
        "stress: land whatever the parked fan-out left staged",
    );
    plan
}

/// Section 2's scenario: A stages `a-item` and `a-control` and parks inside
/// `truncate`; B creates `b-item` and rewrites `a-item` to completion; A is
/// released.
pub struct Truncate {
    /// The parked commit.
    pub a: Finished,
    /// B's create.
    pub b: Finished,
    /// B's rewrite of the path A is landing.
    pub b2: Finished,
    /// `a-item.md`'s bytes before and after B's rewrite.
    pub a_item: (Vec<u8>, Vec<u8>),
    /// What the shared changeset claims once A has finished.
    pub journal: BTreeMap<String, String>,
    /// The repo.
    pub plan: Plan,
}

/// Runs section 2's scenario against `bin`.
pub fn parked_truncate(bin: &Path) -> Truncate {
    let plan = seed(bin);
    for slug in ["a-item", "a-control"] {
        assert!(plan.output(create(&plan, slug, "alpha")).success());
    }
    let commit = plan.cmd(Some(SHARED), &["commit", "-m", "A: land the staged items"]);
    let a = must(plan.park(JOURNAL_BARRIER, "commit-a", commit));
    let b = plan.output(create(&plan, "b-item", "beta"));
    let a_item = plan.path("projects/alpha/tasks/a-item.md");
    let before = std::fs::read(&a_item).unwrap();
    let b2 = plan.run(
        Some(SHARED),
        &[
            "task",
            "update",
            "a-item",
            "--status",
            "in-progress",
            "--no-edit",
            "--project",
            "alpha",
        ],
    );
    let after = std::fs::read(&a_item).unwrap();
    let a = must(a.release_and_finish());
    let journal = plan.journal(SHARED);
    Truncate {
        a,
        b,
        b2,
        a_item: (before, after),
        journal,
        plan,
    }
}

/// Section 5's scenario: two pre-claims; B parks inside its append; a
/// `session gc` from an unrelated session runs to completion; B is released.
pub struct GcDuringAppend {
    /// The parked appender.
    pub b: Finished,
    /// The sweep.
    pub gc: Finished,
    /// The shared journal's raw line count before and after the sweep.
    pub lines: (Option<usize>, Option<usize>),
    /// What the shared changeset claims once B has finished.
    pub journal: BTreeMap<String, String>,
    /// The repo.
    pub plan: Plan,
}

/// Runs section 5's scenario against `bin`.
pub fn gc_during_append(bin: &Path) -> GcDuringAppend {
    let plan = seed(bin);
    for slug in ["pre-item", "pre-item-two"] {
        assert!(plan.output(create(&plan, slug, "alpha")).success());
    }
    let b = must(plan.park(APPEND_BARRIER, "append-b", create(&plan, "b-item", "beta")));
    let before = plan.journal_lines(SHARED);
    let gc = plan.run(Some("gc-runner"), &["session", "gc"]);
    let after = plan.journal_lines(SHARED);
    let b = must(b.release_and_finish());
    let journal = plan.journal(SHARED);
    GcDuringAppend {
        b,
        gc,
        lines: (before, after),
        journal,
        plan,
    }
}

/// When section 6's scenario releases the parked compaction.
#[derive(Clone, Copy)]
pub enum Release {
    /// Once B's task file exists (its flush is done, so its append is next)
    /// and B is still running — it cannot finish while the sweep holds the
    /// lock exclusively.
    WhileAppendWaits,
    /// Once B has exited: without the lock it writes into the doomed inode
    /// while the sweep is parked, which is the old interleaving made
    /// deterministic.
    AfterAppendExits,
}

/// Section 6's scenario: a pre-sweep empties the changesets directory; two
/// pre-claims; `session gc` parks after its compare-and-swap; `task create
/// b-item` arrives; the sweep is released as `release` says.
pub struct AppendDuringCompaction {
    /// The appender.
    pub b: Finished,
    /// The parked sweep.
    pub gc: Finished,
    /// The shared journal's raw line count before the sweep and after both.
    pub lines: (Option<usize>, Option<usize>),
    /// What the shared changeset claims afterwards.
    pub journal: BTreeMap<String, String>,
    /// The repo.
    pub plan: Plan,
}

/// Runs section 6's scenario against `bin`.
pub fn append_during_compaction(bin: &Path, release: Release) -> AppendDuringCompaction {
    let plan = seed(bin);
    let pre = plan.run(Some("gc-runner"), &["session", "gc"]);
    assert!(pre.success(), "the pre-sweep failed: {}", pre.describe());
    let left: Vec<_> = std::fs::read_dir(plan.changesets_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".jsonl"))
        .map(|e| e.file_name())
        .collect();
    assert!(
        left.is_empty(),
        "the pre-sweep left {left:?}, so the parked sweep may park on the wrong journal"
    );
    for slug in ["pre-item", "pre-item-two"] {
        assert!(plan.output(create(&plan, slug, "alpha")).success());
    }
    let before = plan.journal_lines(SHARED);
    let gc_cmd = plan.cmd(Some("gc-runner"), &["session", "gc"]);
    let g = must(plan.park(COMPACT_BARRIER, "gc-g", gc_cmd));
    let mut b = must(plan.spawn("create-b", create(&plan, "b-item", "beta")));
    let task = plan.path("projects/beta/tasks/b-item.md");
    match release {
        Release::WhileAppendWaits => {
            must(wait_until("b-item.md to be flushed", EXIT_DEADLINE, || {
                Ok(task.exists())
            }));
            assert!(
                must(b.try_status()).is_none(),
                "B finished while the sweep held the journal lock exclusively"
            );
        }
        Release::AfterAppendExits => {
            must(wait_until("B to exit", EXIT_DEADLINE, || {
                Ok(b.try_status()?.is_some())
            }));
        }
    }
    let gc = must(g.release_and_finish());
    let b = must(b.wait_bounded(EXIT_DEADLINE));
    let after = plan.journal_lines(SHARED);
    let journal = plan.journal(SHARED);
    AppendDuringCompaction {
        b,
        gc,
        lines: (before, after),
        journal,
        plan,
    }
}

/// Section 7's scenario: `doomed-item` staged; `session discard --force`
/// parks after reading what it will retire; B creates `b-item`; the discard
/// is released.
pub struct DiscardDuringAppend {
    /// The parked discard.
    pub discard: Finished,
    /// The concurrent appender.
    pub b: Finished,
    /// What the shared changeset claims afterwards.
    pub journal: BTreeMap<String, String>,
    /// The repo.
    pub plan: Plan,
}

/// Runs section 7's scenario against `bin`.
pub fn discard_during_append(bin: &Path) -> DiscardDuringAppend {
    let plan = seed(bin);
    assert!(plan.output(create(&plan, "doomed-item", "alpha")).success());
    let discard = plan.cmd(
        Some("discard-runner"),
        &["session", "discard", SHARED, "--force"],
    );
    let d = must(plan.park(JOURNAL_BARRIER, "discard-d", discard));
    let b = plan.output(create(&plan, "b-item", "beta"));
    let discard = must(d.release_and_finish());
    let journal = plan.journal(SHARED);
    DiscardDuringAppend {
        discard,
        b,
        journal,
        plan,
    }
}

/// Section 1: 40 parallel creates across two projects with six commits
/// interleaved into the spawn stream, all under one `RDM_SESSION`, strand
/// nothing: every task lands, the tree is clean immediately (a mutation
/// writes only paths it journals, so a dirty path could only be a destroyed
/// record), the fold is empty, `session list` omits the changeset, and a final
/// commit is a clean no-op with nothing unattributed.
#[test]
fn stress_fanout_strands_nothing() {
    let plan = seed(&real_bin());
    let mut procs = Vec::new();
    let mut is_create = Vec::new();
    for i in 1..=N_MUTATIONS {
        let slug = format!("stress-{i}");
        procs.push(must(
            plan.spawn(&slug, create(&plan, &slug, stress_project(i))),
        ));
        is_create.push(true);
        if i.is_multiple_of(N_MUTATIONS / M_COMMITS) {
            let commit = plan.cmd(Some(SHARED), &["commit", "-m", &format!("stress: at {i}")]);
            procs.push(must(plan.spawn(&format!("commit-{i}"), commit)));
            is_create.push(false);
        }
    }
    assert_eq!(is_create.iter().filter(|c| !**c).count(), M_COMMITS);
    // The interleaved commits race each other for HEAD and may lose; what is
    // asserted is that every create succeeds and nothing ends up stranded.
    for (f, create) in finish_all(procs, Duration::from_secs(180))
        .iter()
        .zip(is_create)
    {
        if create {
            assert!(f.success(), "a fan-out create failed: {}", f.describe());
        }
    }
    settle(&plan, "stress: land whatever the fan-out left staged");
    let tree = plan.tree("HEAD");
    let missing: Vec<String> = (1..=N_MUTATIONS)
        .map(stress_path)
        .filter(|p| !tree.contains(p))
        .collect();
    assert!(
        missing.is_empty(),
        "stress tasks missing from HEAD: {missing:?}"
    );
    assert_eq!(plan.porcelain(), "", "authored files were stranded");
    assert!(
        plan.journal(SHARED).is_empty(),
        "the changeset still claims paths"
    );
    let listed = plan.json(&["session", "list", "--format", "json"]);
    assert!(
        !listed.to_string().contains(SHARED),
        "session list still reports the committed changeset: {listed}"
    );
    let noop = plan.run(Some(SHARED), &["commit", "-m", "stress: should be a no-op"]);
    assert!(noop.success(), "{}", noop.describe());
    for symptom in ["another changeset", "not attributed to any changeset"] {
        assert!(
            !noop.all().to_lowercase().contains(symptom),
            "{symptom}: {}",
            noop.describe()
        );
    }
}

/// Section 1b, fixed half: every record appended while a commit was parked
/// inside `truncate` survives, so the settling commit leaves the tree clean.
#[test]
fn parked_commit_fanout_keeps_every_record() {
    let plan = parked_fanout(&real_bin());
    assert_eq!(
        plan.porcelain(),
        "",
        "a record appended in the window was lost"
    );
}

/// Section 2: a create and a rewrite of a path the parked commit is landing
/// both survive its tombstone, a landed path nobody re-recorded is dropped,
/// and the survivors commit cleanly.
#[test]
fn records_appended_during_a_parked_truncate_survive() {
    let t = parked_truncate(&real_bin());
    for (what, f) in [("A", &t.a), ("B", &t.b), ("B's update", &t.b2)] {
        assert!(f.success(), "{what} failed: {}", f.describe());
    }
    assert_ne!(
        t.a_item.0, t.a_item.1,
        "B's update produced identical bytes"
    );
    assert!(
        t.a.stdout.contains("Committed 2 file(s)"),
        "A did not land both staged tasks: {}",
        t.a.describe()
    );
    assert!(
        t.journal.contains_key("projects/beta/tasks/b-item.md"),
        "{:?}",
        t.journal
    );
    assert!(
        t.journal.contains_key("projects/alpha/tasks/a-item.md"),
        "{:?}",
        t.journal
    );
    assert!(
        !t.journal.contains_key("projects/alpha/tasks/a-control.md"),
        "a landed path nobody re-recorded is still claimed"
    );
    let follow = settle(&t.plan, "B: land what the interleave left staged");
    assert!(!follow.all().to_lowercase().contains("another changeset"));
    assert_eq!(t.plan.porcelain(), "");
}

/// Section 5: a `session gc` from an unrelated session is excluded by a
/// parked append's shared lock (the journal keeps both lines), and the record
/// lands.
#[test]
fn gc_is_excluded_by_a_parked_append() {
    let g = gc_during_append(&real_bin());
    assert!(g.b.success(), "{}", g.b.describe());
    assert!(g.gc.success(), "{}", g.gc.describe());
    assert_eq!(
        g.lines,
        (Some(2), Some(2)),
        "gc rewrote the journal under the append"
    );
    assert!(
        g.journal.contains_key("projects/beta/tasks/b-item.md"),
        "{:?}",
        g.journal
    );
    assert!(
        g.journal
            .contains_key("projects/alpha/tasks/pre-item-two.md")
    );
    let follow = settle(&g.plan, "land what the gc interleave left staged");
    assert!(!follow.all().to_lowercase().contains("another changeset"));
    assert_eq!(g.plan.porcelain(), "");
}

/// Section 6: an append arriving while a compaction is parked between its
/// compare-and-swap and its rename waits it out and lands in the rewritten
/// journal: the compacted line plus its own.
#[test]
fn append_waits_out_a_parked_compaction() {
    let w = append_during_compaction(&real_bin(), Release::WhileAppendWaits);
    assert!(w.b.success(), "{}", w.b.describe());
    assert!(w.gc.success(), "{}", w.gc.describe());
    assert_eq!(
        w.lines,
        (Some(2), Some(2)),
        "expected the compacted line plus B's"
    );
    assert!(
        w.journal.contains_key("projects/beta/tasks/b-item.md"),
        "{:?}",
        w.journal
    );
    assert!(
        w.journal
            .contains_key("projects/alpha/tasks/pre-item-two.md")
    );
    let follow = settle(&w.plan, "land what the window interleave left staged");
    assert!(!follow.all().to_lowercase().contains("another changeset"));
    assert_eq!(w.plan.porcelain(), "");
}

/// Section 7: a record appended during a parked `session discard --force`
/// survives; the discard still retires what it read; the survivor commits.
#[test]
fn discard_keeps_a_concurrent_append() {
    let d = discard_during_append(&real_bin());
    assert!(d.b.success(), "{}", d.b.describe());
    assert!(d.discard.success(), "{}", d.discard.describe());
    assert!(
        !d.journal
            .contains_key("projects/alpha/tasks/doomed-item.md"),
        "the discard did not retire what it read"
    );
    assert!(
        d.journal.contains_key("projects/beta/tasks/b-item.md"),
        "{:?}",
        d.journal
    );
    settle(&d.plan, "land what survived the discard interleave");
    assert!(
        d.plan
            .tree("HEAD")
            .contains("projects/beta/tasks/b-item.md")
    );
}
