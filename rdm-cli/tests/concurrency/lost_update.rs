//! Content-digest optimistic concurrency at the store's flush and commit
//! boundaries, across real processes: two processes interleaved mid-flush at
//! `RDM_HARNESS_FLUSH_BARRIER` (the loser is refused, with or without a
//! session id), a session's own sequential writes, the commit-time overwrite
//! refusal, the hook path staying exit-0, and the delayed delete of a path
//! another session recreated. An in-process test with two `Store` handles
//! cannot stand in for these: the staging overlay does not span invocations.
//! Ported from the retired `scripts/verify-lost-update.sh`; the case map is
//! `docs/test-migration-inventory.md` § 9.

use std::collections::BTreeMap;
use std::path::Path;

use crate::support::{Finished, Plan, must, real_bin};

const FLUSH_BARRIER: &str = "RDM_HARNESS_FLUSH_BARRIER";
const TASK: &str = "projects/demo/tasks/fix-bug.md";

/// Project `demo` with task `fix-bug` tagged `alpha`, committed.
pub fn seed(bin: &Path) -> Plan {
    Plan::seeded(
        bin,
        "demo",
        &[&[
            "task",
            "create",
            "fix-bug",
            "--title",
            "Fix bug",
            "--body",
            "Body.",
            "--tags",
            "alpha",
            "--no-edit",
            "--project",
            "demo",
        ]],
    )
}

fn tag_update(tags: &str) -> [&str; 8] {
    [
        "task",
        "update",
        "fix-bug",
        "--tags",
        tags,
        "--no-edit",
        "--project",
        "demo",
    ]
}

/// Section 2's scenario: A (`--tags alpha,from-a`) parks at the flush
/// barrier, B (`--tags alpha,from-b`) runs to completion, A is released.
pub struct Interleave {
    /// The parked process.
    pub a: Finished,
    /// The process that ran while A was parked.
    pub b: Finished,
    /// The task's tags afterwards.
    pub tags: Vec<String>,
}

/// Runs section 2's scenario against `bin`; `None` sessions run bare from
/// the test process, so both share its rung-2 lease.
pub fn interleave(bin: &Path, a_session: Option<&str>, b_session: Option<&str>) -> Interleave {
    let plan = seed(bin);
    let a = must(plan.park(
        FLUSH_BARRIER,
        "update-a",
        plan.cmd(a_session, &tag_update("alpha,from-a")),
    ));
    let b = plan.run(b_session, &tag_update("alpha,from-b"));
    let a = must(a.release_and_finish());
    let tags = plan.tags("demo", "fix-bug");
    Interleave { a, b, tags }
}

/// Section 6's scenario: A stages a delete (`promote fix-bug`) and does not
/// commit; optionally B recreates the path and lands it; then A commits.
pub struct DelayedDelete {
    /// What A's changeset journaled.
    pub journal: BTreeMap<String, String>,
    /// A's delayed commit.
    pub a: Finished,
    /// The repo.
    pub plan: Plan,
}

/// Runs section 6's scenario against `bin`.
pub fn delayed_delete(bin: &Path, recreate: bool) -> DelayedDelete {
    let plan = seed(bin);
    plan.ok(
        Some("del-a"),
        &[
            "promote",
            "fix-bug",
            "--roadmap-slug",
            "promoted",
            "--project",
            "demo",
        ],
    );
    let journal = plan.journal("del-a");
    if recreate {
        plan.ok(
            Some("del-b"),
            &[
                "task",
                "create",
                "fix-bug",
                "--title",
                "B's task",
                "--body",
                "B's bytes.",
                "--no-edit",
                "--project",
                "demo",
            ],
        );
        plan.ok(Some("del-b"), &["commit", "-m", "B's message"]);
    }
    let a = plan.run(Some("del-a"), &["commit", "-m", "A's message"]);
    DelayedDelete { journal, a, plan }
}

fn assert_loser_refused(run: &Interleave) {
    assert!(
        run.b.success(),
        "B (the winner) failed: {}",
        run.b.describe()
    );
    assert!(
        !run.a.success(),
        "A wrote over B's change and exited 0: the lost update is open"
    );
    assert!(
        run.a.stderr.contains("task/fix-bug"),
        "{}",
        run.a.describe()
    );
    assert!(
        run.a.stderr.to_lowercase().contains("nothing was written"),
        "{}",
        run.a.describe()
    );
    assert!(
        run.tags.iter().any(|t| t == "from-b"),
        "B's tag is gone: {:?}",
        run.tags
    );
    assert!(
        !run.tags.iter().any(|t| t == "from-a"),
        "A's refused write landed: {:?}",
        run.tags
    );
}

/// Section 2: the loser of a concurrent read-modify-write is refused with an
/// error naming the item, and the winner's write survives.
#[test]
fn concurrent_flush_loser_is_refused() {
    assert_loser_refused(&interleave(&real_bin(), Some("sess-a"), Some("sess-b")));
}

/// Section 2b: the same with no session id at all — both processes share the
/// test process's rung-2 lease, so the check is content-keyed, not
/// identity-keyed.
#[test]
fn concurrent_flush_loser_is_refused_without_a_session_id() {
    assert_loser_refused(&interleave(&real_bin(), None, None));
}

/// Section 3: nine back-to-back invocations in one session — two successive
/// `--tags` read-modify-writes of a list the session itself wrote among them —
/// all succeed, under `RDM_SESSION` and again bare, each in its own repo.
#[test]
fn a_sessions_sequential_writes_never_trip_the_check() {
    let sequence: [&[&str]; 9] = [
        &[
            "task",
            "create",
            "seq-item",
            "--title",
            "Seq",
            "--body",
            "B",
            "--tags",
            "one",
            "--no-edit",
            "--project",
            "demo",
        ],
        &[
            "task",
            "update",
            "seq-item",
            "--tags",
            "one,two",
            "--no-edit",
            "--project",
            "demo",
        ],
        &[
            "task",
            "update",
            "seq-item",
            "--tags",
            "one,two,three",
            "--no-edit",
            "--project",
            "demo",
        ],
        &[
            "task",
            "update",
            "seq-item",
            "--status",
            "in-progress",
            "--no-edit",
            "--project",
            "demo",
        ],
        &[
            "task",
            "update",
            "seq-item",
            "--body",
            "Rewritten.",
            "--no-edit",
            "--project",
            "demo",
        ],
        &[
            "roadmap",
            "create",
            "seq-map",
            "--title",
            "Map",
            "--body",
            "B",
            "--no-edit",
            "--project",
            "demo",
        ],
        &[
            "phase",
            "create",
            "build",
            "--title",
            "Build",
            "--number",
            "1",
            "--body",
            "B",
            "--no-edit",
            "--roadmap",
            "seq-map",
            "--project",
            "demo",
        ],
        &[
            "phase",
            "update",
            "1",
            "--status",
            "in-progress",
            "--no-edit",
            "--roadmap",
            "seq-map",
            "--project",
            "demo",
        ],
        &["commit", "-m", "sequential-batch"],
    ];
    for session in [Some("solo"), None] {
        let plan = seed(&real_bin());
        for (i, args) in sequence.iter().enumerate() {
            let out = plan.run(session, args);
            assert!(
                out.success(),
                "invocation {} ({}) under {session:?}: {}",
                i + 1,
                args.join(" "),
                out.describe()
            );
        }
        assert_eq!(plan.tags("demo", "seq-item"), ["one", "two", "three"]);
    }
}

/// Section 4: committing a path another session overwrote since this one
/// flushed it is refused rather than landing the other session's bytes under
/// this message; without the overwrite the same commit lands.
#[test]
fn commit_refuses_a_path_another_session_overwrote() {
    let plan = seed(&real_bin());
    plan.ok(Some("cs-a"), &tag_update("alpha,owned-by-a"));
    plan.ok(Some("cs-b"), &tag_update("alpha,owned-by-b"));
    let head = plan.head();
    let out = plan.run(Some("cs-a"), &["commit", "-m", "A's message"]);
    assert!(!out.success(), "A committed B's bytes: {}", out.describe());
    assert!(out.stderr.contains("task/fix-bug"), "{}", out.describe());
    assert!(
        out.stderr
            .to_lowercase()
            .contains("another session overwrote it"),
        "{}",
        out.describe()
    );
    assert_eq!(plan.head(), head, "a refused commit moved HEAD");

    let control = seed(&real_bin());
    control.ok(Some("cs-a"), &tag_update("alpha,owned-by-a"));
    control.ok(Some("cs-a"), &["commit", "-m", "A's message"]);
}

/// Section 5: a flush refused on the `Done:` hook path is logged, and the hook
/// still exits 0.
#[test]
fn hook_logs_a_refused_flush_and_exits_zero() {
    let plan = seed(&real_bin());
    plan.git_commit_empty("work\n\nDone: task/fix-bug");
    let mut hook = plan.cmd(Some("hook-a"), &["hook", "post-commit"]);
    hook.current_dir(&plan.root);
    let parked = must(plan.park(FLUSH_BARRIER, "hook", hook));
    plan.ok(Some("other"), &tag_update("alpha,from-other"));
    let hook = must(parked.release_and_finish());
    assert!(
        hook.success(),
        "the hook failed the git commit: {}",
        hook.describe()
    );
    let log = std::fs::read_to_string(plan.root.join(".git/rdm-hook.log"))
        .expect("the hook wrote no log");
    assert!(log.contains("error"), "the rejection was not logged: {log}");
}

/// Section 6: a delayed commit whose changeset deletes a path another
/// session has since recreated is refused, naming the item, and the other
/// session's file survives at HEAD.
#[test]
fn delayed_delete_of_a_recreated_path_is_refused() {
    let d = delayed_delete(&real_bin(), true);
    assert_eq!(
        d.journal.get(TASK).map(String::as_str),
        Some("delete"),
        "A must journal a delete of {TASK}: {:?}",
        d.journal
    );
    assert!(
        !d.a.success(),
        "A's stale delete landed: {}",
        d.a.describe()
    );
    let err = d.a.stderr.to_lowercase();
    for needle in ["task/fix-bug", "recreated", "nothing was committed"] {
        assert!(err.contains(needle), "{needle}: {}", d.a.describe());
    }
    let at_head = d.plan.show("HEAD", TASK).unwrap_or_default();
    assert!(at_head.contains("B's bytes"), "B's file is gone at HEAD");
    let subject = d.plan.git(&["log", "-1", "--pretty=%s"]);
    assert_ne!(subject.trim(), "A's message", "a refused commit landed");
}

/// Section 6b: the same delayed delete, with nobody recreating the path,
/// lands and removes it.
#[test]
fn delayed_delete_without_a_recreate_lands() {
    let d = delayed_delete(&real_bin(), false);
    assert!(d.a.success(), "{}", d.a.describe());
    assert!(
        !d.plan.tree("HEAD").contains(TASK),
        "the delete did not land"
    );
}
