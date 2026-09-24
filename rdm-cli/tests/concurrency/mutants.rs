//! Negative controls: each fixed test's scenario, re-run against an isolated
//! build with one regression planted ([`crate::mutant`]), must show the old
//! failure. Every control also asserts its "inconclusive" guards — the
//! processes it relies on exited 0, the mutant's sweep really compacted — so a
//! control can fail as not-run but never pass without exercising its
//! regression.
//!
//! These run in the `rdm-mutant-builds` nextest group (`max-threads = 1`,
//! `.config/nextest.toml`): each first ensures its family's build, a full
//! `rdm-cli` cargo build when cold. See `docs/test-migration-inventory.md`
//! § 9 for the families, the attribution of each control to exactly one
//! planted edit, and the measured costs.

use std::path::PathBuf;

use tempfile::TempDir;

use crate::journal_race::{
    Release, SHARED, append_during_compaction, discard_during_append, gc_during_append,
    parked_fanout, parked_truncate,
};
use crate::lost_update::{delayed_delete, interleave};
use crate::mutant::{Family, build};
use crate::session_identity::{ancestor_lease, fragmented_commit, wrapped_ids};
use crate::support::Plan;

/// A family's mutant binary, in a temp dir the test owns.
fn mutant(family: Family) -> (TempDir, PathBuf) {
    let dir = TempDir::new().expect("tempdir");
    let bin = build(family, dir.path())
        .unwrap_or_else(|f| panic!("negative control not run — the mutant was not built: {f}"));
    (dir, bin)
}

/// Section J's control: with the harness check skipped, both children merge
/// onto the planted ancestor lease — the phase-10 bug.
#[test]
fn session_harness_check_removed_merges_children_onto_the_ancestor_lease() {
    let (_dir, bin) = mutant(Family::Session);
    let j = ancestor_lease(&bin, 1);
    assert_eq!(
        j.planted.rung, 2,
        "inconclusive: the plant did not land on rung 2"
    );
    assert!(
        j.one[0].id == j.planted.id && j.two[0].id == j.planted.id,
        "the mutant did not merge the children onto {}: {:?} {:?}",
        j.planted.id,
        j.one,
        j.two
    );
}

/// § K self-test 1: with the continuity advisory silenced, a fragmented
/// commit no longer names the remedy.
#[test]
fn session_advisory_silenced_drops_the_remedy() {
    let (_dir, bin) = mutant(Family::Session);
    let (_plan, run) = fragmented_commit(&bin);
    assert!(
        run.steps.iter().all(|s| s.status == 0),
        "inconclusive: a wrapped step failed: {run:?}"
    );
    assert!(
        !run.steps[1].all().contains("RDM_HARNESS_SESSION_ID"),
        "the mutant still printed the remedy: {}",
        run.steps[1].all()
    );
}

/// § K self-test 2: with the create-path sweep removed, five wrapped calls
/// leak more than two leases.
#[test]
fn session_create_sweep_removed_leaks_leases() {
    let (_dir, bin) = mutant(Family::Session);
    let plan = Plan::demo(&bin);
    let run = wrapped_ids(&plan, 5, "mk2");
    assert!(
        run.steps.iter().all(|s| s.status == 0),
        "inconclusive: a wrapped call failed: {run:?}"
    );
    let leases = plan.leases();
    assert!(
        leases.len() > 2,
        "the mutant left only {} lease(s): {leases:?}",
        leases.len()
    );
}

/// Section 1b's control: with read-modify-write truncation, the records
/// appended while the commit was parked are destroyed and their task files
/// are stranded with nothing claiming them.
#[test]
fn journal_rmw_truncate_strands_the_parked_fanout() {
    let (_dir, bin) = mutant(Family::Journal);
    let plan = parked_fanout(&bin);
    let dirty = plan.porcelain();
    let stranded: Vec<&str> = dirty
        .lines()
        .filter(|l| l.contains("/tasks/stress-") && l.ends_with(".md"))
        .collect();
    assert!(
        !stranded.is_empty(),
        "the mutant stranded no stress task: {dirty:?}"
    );
    let claimed = plan.journal(SHARED);
    assert!(
        !claimed.keys().any(|p| p.contains("stress-")),
        "the mutant still claims the stranded paths: {claimed:?}"
    );
}

/// Section 2's control: with read-modify-write truncation, both of B's
/// records are lost and the tree is left dirty.
#[test]
fn journal_rmw_truncate_loses_appends_made_while_parked() {
    let (_dir, bin) = mutant(Family::Journal);
    let t = parked_truncate(&bin);
    assert!(t.a.success(), "inconclusive: A failed: {}", t.a.describe());
    assert!(t.b.success() && t.b2.success(), "inconclusive: B failed");
    for path in [
        "projects/beta/tasks/b-item.md",
        "projects/alpha/tasks/a-item.md",
    ] {
        assert!(
            !t.journal.contains_key(path),
            "the mutant kept {path}: {:?}",
            t.journal
        );
    }
    assert_ne!(t.plan.porcelain(), "", "the mutant left a clean tree");
}

/// Section 5's control: without the journal lock, `session gc` rewrites the
/// journal under the parked append and its record is lost.
#[test]
fn journal_lockless_gc_rewrites_under_a_parked_append() {
    let (_dir, bin) = mutant(Family::Journal);
    let g = gc_during_append(&bin);
    assert!(g.b.success(), "inconclusive: B failed: {}", g.b.describe());
    assert_eq!(
        g.lines.1,
        Some(1),
        "inconclusive: the mutant's gc never compacted"
    );
    assert!(
        !g.journal.contains_key("projects/beta/tasks/b-item.md"),
        "the mutant kept B's record: {:?}",
        g.journal
    );
    assert_ne!(g.plan.porcelain(), "", "the mutant left a clean tree");
}

/// Section 6's control: without the journal lock, an append made while the
/// compaction is parked lands in the inode the rename discards.
#[test]
fn journal_lockless_append_lands_in_the_doomed_inode() {
    let (_dir, bin) = mutant(Family::Journal);
    let w = append_during_compaction(&bin, Release::AfterAppendExits);
    assert!(w.b.success(), "inconclusive: B failed: {}", w.b.describe());
    assert_eq!(
        w.lines.1,
        Some(1),
        "inconclusive: the sweep did not rewrite"
    );
    assert!(
        !w.journal.contains_key("projects/beta/tasks/b-item.md"),
        "the mutant kept B's record: {:?}",
        w.journal
    );
    assert_ne!(w.plan.porcelain(), "", "the mutant left a clean tree");
}

/// Section 7's control: with the discard back to a bare unlink, the
/// concurrent record is lost and its file is left as unattributed dirt.
#[test]
fn journal_unlinking_discard_loses_a_concurrent_append() {
    let (_dir, bin) = mutant(Family::Journal);
    let d = discard_during_append(&bin);
    assert!(d.b.success(), "inconclusive: B failed: {}", d.b.describe());
    let path = "projects/beta/tasks/b-item.md";
    assert!(!d.journal.contains_key(path), "the mutant kept B's record");
    let _ = d.plan.run(
        Some(SHARED),
        &["commit", "-m", "try to land what the mutant left"],
    );
    assert!(
        !d.plan.tree("HEAD").contains(path),
        "the mutant committed B's file"
    );
    assert!(
        d.plan.porcelain().contains(path),
        "B's file is not left behind as dirt"
    );
}

/// Section 2c's control: with the flush precondition neutered, A overwrites
/// B's change and exits 0.
#[test]
fn lost_update_neutered_flush_check_loses_the_update() {
    let (_dir, bin) = mutant(Family::LostUpdate);
    let run = interleave(&bin, Some("sess-a"), Some("sess-b"));
    assert!(
        run.a.success(),
        "inconclusive: A failed: {}",
        run.a.describe()
    );
    assert!(
        run.b.success(),
        "inconclusive: B failed: {}",
        run.b.describe()
    );
    assert!(
        run.tags.iter().any(|t| t == "from-a") && !run.tags.iter().any(|t| t == "from-b"),
        "the mutant did not lose B's update: {:?}",
        run.tags
    );
}

/// Section 6c's control: with the delete guard short-circuited, A's delayed
/// commit lands its stale delete over B's recreated file.
#[test]
fn lost_update_short_circuited_delete_guard_destroys_the_recreated_file() {
    let (_dir, bin) = mutant(Family::LostUpdate);
    let d = delayed_delete(&bin, true);
    assert!(d.a.success(), "inconclusive: A failed: {}", d.a.describe());
    assert!(
        !d.plan
            .tree("HEAD")
            .contains("projects/demo/tasks/fix-bug.md"),
        "the mutant kept B's file at HEAD"
    );
}
