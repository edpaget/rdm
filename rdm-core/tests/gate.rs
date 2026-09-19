//! Core-level coverage of the write-time `reviewed` transition gate.
//!
//! Everything here runs against [`MemoryStore`] plus
//! [`MemoryWorktreeProbe`] — no git, no filesystem — which is the whole point
//! of pushing the worktree question behind a port: the three preconditions,
//! their fixed evaluation order, and the fail-closed branches are all
//! exercisable as pure logic.

use chrono::{NaiveDate, Utc};
use rdm_core::document::Document;
use rdm_core::error::Error;
use rdm_core::link::ItemRef;
use rdm_core::model::*;
use rdm_core::ops::gate::{GateDecision, ReviewedGate, check_reviewed_gate};
use rdm_core::ops::{BodyUpdate, TagsUpdate, TitleUpdate};
use rdm_core::store::MemoryStore;
use rdm_core::worktree::MemoryWorktreeProbe;

const PROJECT: &str = "demo";
const ROADMAP: &str = "gates";
const STEM: &str = "phase-1-enforce";

fn day() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 9, 13).unwrap()
}

fn phase_item() -> ItemRef {
    ItemRef::Phase {
        roadmap: ROADMAP.to_string(),
        stem: STEM.to_string(),
    }
}

fn task_item(slug: &str) -> ItemRef {
    ItemRef::Task {
        slug: slug.to_string(),
    }
}

/// A project with one roadmap, one phase, and one task.
fn seed() -> MemoryStore {
    let mut store = MemoryStore::new();
    rdm_core::ops::project::create_project(&mut store, PROJECT, "Demo").unwrap();
    rdm_core::io::write_roadmap(
        &mut store,
        PROJECT,
        ROADMAP,
        &Document {
            frontmatter: Roadmap {
                project: PROJECT.to_string(),
                roadmap: ROADMAP.to_string(),
                title: "Gates".to_string(),
                phases: vec![STEM.to_string()],
                dependencies: None,
                priority: None,
                tags: None,
            },
            body: String::new(),
        },
    )
    .unwrap();
    rdm_core::io::write_phase(
        &mut store,
        PROJECT,
        ROADMAP,
        STEM,
        &Document {
            frontmatter: Phase {
                phase: 1,
                title: "Enforce".to_string(),
                status: PhaseStatus::NeedsReview,
                tags: None,
                completed: None,
                commit: None,
                review_sha: None,
                review_branch: None,
                difficulty: None,
                model: None,
                blocked_reason: None,
                gate_override: None,
            },
            body: String::new(),
        },
    )
    .unwrap();
    rdm_core::io::write_task(
        &mut store,
        PROJECT,
        "solo",
        &Document {
            frontmatter: Task {
                project: PROJECT.to_string(),
                title: "Solo".to_string(),
                status: TaskStatus::NeedsReview,
                priority: Priority::Medium,
                created: day(),
                tags: None,
                completed: None,
                commit: None,
                review_sha: None,
                review_branch: None,
                close_reason: None,
                gate_override: None,
            },
            body: String::new(),
        },
    )
    .unwrap();
    store
}

/// Writes a plan implementing `item` with the given status.
fn add_plan(store: &mut MemoryStore, slug: &str, item: &ItemRef, status: PlanStatus) {
    rdm_core::io::write_plan(
        store,
        PROJECT,
        slug,
        &Document {
            frontmatter: Plan {
                project: PROJECT.to_string(),
                plan: slug.to_string(),
                title: format!("Plan {slug}"),
                implements: item.clone(),
                supersedes: None,
                status,
                created: day(),
                updated: day(),
            },
            body: String::new(),
        },
    )
    .unwrap();
}

/// Writes a `change/<sha>` review whose `implements` names `plan_slug`.
fn add_change_review(
    store: &mut MemoryStore,
    id: &str,
    plan_slug: &str,
    verdict: Option<Verdict>,
    state: ReviewState,
) {
    rdm_core::io::write_review(
        store,
        PROJECT,
        id,
        &Document {
            frontmatter: Review {
                id: id.to_string(),
                author: "reviewer".to_string(),
                target: ReviewTarget::Change {
                    head: "a".repeat(40),
                    base: None,
                },
                state,
                verdict,
                created: Utc::now(),
                submitted: None,
                created_commit: None,
                implements: Some(ReviewTarget::Plan {
                    slug: plan_slug.to_string(),
                }),
                change_branch: None,
                comments: Vec::new(),
            },
            body: "Looks right.".to_string(),
        },
    )
    .unwrap();
}

// ---------------------------------------------------------------------------
// AC1 — the three refusals, their order, and the success path
// ---------------------------------------------------------------------------

#[test]
fn a_disabled_gate_never_fires() {
    let store = seed();
    let d = check_reviewed_gate(&store, PROJECT, &phase_item(), &ReviewedGate::disabled()).unwrap();
    assert_eq!(d, GateDecision::NotApplicable);
}

#[test]
fn a_disabled_gate_refuses_an_override_rather_than_dropping_it() {
    // An override against a gate that is not enforcing has nothing to bypass.
    // Honoring it as a no-op would discard the reason and actor the operator
    // supplied and leave `phase show` silently disagreeing with the request,
    // so it is refused — the same reasoning that rejects an override on a
    // transition the gate never guards.
    let store = seed();
    let err = check_reviewed_gate(
        &store,
        PROJECT,
        &phase_item(),
        &ReviewedGate::disabled().with_override("operator: hotfix", "alice"),
    )
    .unwrap_err();
    assert!(matches!(err, Error::GateOverrideGateDisabled), "{err:?}");
    let msg = err.to_string();
    assert!(
        msg.contains("rdm config set gates.reviewed true"),
        "remediation missing: {msg}"
    );
    assert!(msg.contains("not enforcing"), "cause missing: {msg}");
}

#[test]
fn a_disabled_gate_refuses_an_override_before_validating_its_reason() {
    // Ordering: the disabled-gate refusal is the more fundamental condition,
    // so it is reported ahead of the empty-reason complaint. Telling an
    // operator to write a better reason for a bypass that would never be
    // recorded is the less actionable of the two.
    let store = seed();
    let err = check_reviewed_gate(
        &store,
        PROJECT,
        &phase_item(),
        &ReviewedGate::disabled().with_override("   ", "alice"),
    )
    .unwrap_err();
    assert!(matches!(err, Error::GateOverrideGateDisabled), "{err:?}");
}

#[test]
fn precondition_a_refuses_with_no_approved_plan() {
    let mut store = seed();
    // No plan at all.
    let err = check_reviewed_gate(
        &store,
        PROJECT,
        &phase_item(),
        &ReviewedGate::enforcing(None),
    )
    .unwrap_err();
    assert!(matches!(err, Error::GateNoApprovedPlan(ref i) if i == "phase/gates/phase-1-enforce"));
    let msg = err.to_string();
    assert!(
        msg.contains("rdm plan create"),
        "remediation missing: {msg}"
    );
    assert!(
        msg.contains("phase/gates/phase-1-enforce"),
        "item missing: {msg}"
    );

    // A plan that is merely a draft does not satisfy (a).
    add_plan(&mut store, "draft-plan", &phase_item(), PlanStatus::Draft);
    let err = check_reviewed_gate(
        &store,
        PROJECT,
        &phase_item(),
        &ReviewedGate::enforcing(None),
    )
    .unwrap_err();
    assert!(matches!(err, Error::GateNoApprovedPlan(_)));

    // Nor does a superseded one — (a) fails first, so the operator is told to
    // create a new plan rather than chasing a change review on a dead one.
    add_plan(
        &mut store,
        "dead-plan",
        &phase_item(),
        PlanStatus::Superseded,
    );
    let err = check_reviewed_gate(
        &store,
        PROJECT,
        &phase_item(),
        &ReviewedGate::enforcing(None),
    )
    .unwrap_err();
    assert!(matches!(err, Error::GateNoApprovedPlan(_)));
}

#[test]
fn precondition_b_refuses_with_no_approving_change_review_and_lists_every_plan() {
    let mut store = seed();
    add_plan(&mut store, "plan-one", &phase_item(), PlanStatus::Approved);
    add_plan(&mut store, "plan-two", &phase_item(), PlanStatus::Approved);

    // No change review at all.
    let err = check_reviewed_gate(
        &store,
        PROJECT,
        &phase_item(),
        &ReviewedGate::enforcing(None),
    )
    .unwrap_err();
    let Error::GateNoApprovedChangeReview { ref plans, .. } = err else {
        panic!("expected GateNoApprovedChangeReview, got {err:?}");
    };
    assert_eq!(plans, &vec!["plan-one".to_string(), "plan-two".to_string()]);
    let msg = err.to_string();
    assert!(
        msg.contains("rdm review start --on change/"),
        "remediation missing: {msg}"
    );
    assert!(
        msg.contains("plan/plan-one") && msg.contains("plan/plan-two"),
        "candidates missing: {msg}"
    );

    // A change review with the WRONG verdict does not count.
    add_change_review(
        &mut store,
        "2026-09-13-1000-aaaa",
        "plan-one",
        Some(Verdict::RequestChanges),
        ReviewState::Submitted,
    );
    assert!(matches!(
        check_reviewed_gate(
            &store,
            PROJECT,
            &phase_item(),
            &ReviewedGate::enforcing(None)
        ),
        Err(Error::GateNoApprovedChangeReview { .. })
    ));

    // Neither does a DRAFT that somehow carries a verdict.
    add_change_review(
        &mut store,
        "2026-09-13-1001-bbbb",
        "plan-one",
        Some(Verdict::Approve),
        ReviewState::Draft,
    );
    assert!(matches!(
        check_reviewed_gate(
            &store,
            PROJECT,
            &phase_item(),
            &ReviewedGate::enforcing(None)
        ),
        Err(Error::GateNoApprovedChangeReview { .. })
    ));

    // Nor one whose `implements` names a DIFFERENT (unapproved) plan.
    add_plan(&mut store, "other-plan", &phase_item(), PlanStatus::Draft);
    add_change_review(
        &mut store,
        "2026-09-13-1002-cccc",
        "other-plan",
        Some(Verdict::Approve),
        ReviewState::Submitted,
    );
    assert!(matches!(
        check_reviewed_gate(
            &store,
            PROJECT,
            &phase_item(),
            &ReviewedGate::enforcing(None)
        ),
        Err(Error::GateNoApprovedChangeReview { .. })
    ));
}

#[test]
fn an_addressed_review_submitted_with_approve_still_counts() {
    let mut store = seed();
    add_plan(&mut store, "plan-one", &phase_item(), PlanStatus::Approved);
    add_change_review(
        &mut store,
        "2026-09-13-1000-aaaa",
        "plan-one",
        Some(Verdict::Approve),
        ReviewState::Addressed,
    );
    let d = check_reviewed_gate(
        &store,
        PROJECT,
        &phase_item(),
        &ReviewedGate::enforcing(None),
    )
    .unwrap();
    assert!(matches!(d, GateDecision::Satisfied { ref plan, .. } if plan == "plan-one"));
}

#[test]
fn precondition_c_refuses_a_dirty_worktree_and_names_the_paths() {
    let mut store = seed();
    add_plan(&mut store, "plan-one", &phase_item(), PlanStatus::Approved);
    add_change_review(
        &mut store,
        "2026-09-13-1000-aaaa",
        "plan-one",
        Some(Verdict::Approve),
        ReviewState::Submitted,
    );
    // The recorded head must match the observed one, or (b)'s HEAD rule —
    // which applies to every observed checkout, dirty or clean — would
    // refuse before (c) ever runs and this test would stop isolating (c).
    let probe = MemoryWorktreeProbe::new()
        .with_worktree(&phase_item(), "/wt/gates", &["src/leftover.rs", "notes.md"])
        .with_head(&phase_item(), &"a".repeat(40));
    let err = check_reviewed_gate(
        &store,
        PROJECT,
        &phase_item(),
        &ReviewedGate::enforcing(Some(&probe)),
    )
    .unwrap_err();
    let Error::GateWorktreeDirty {
        ref path,
        ref paths,
        ..
    } = err
    else {
        panic!("expected GateWorktreeDirty, got {err:?}");
    };
    assert_eq!(path, "/wt/gates");
    assert_eq!(
        paths,
        &vec!["src/leftover.rs".to_string(), "notes.md".to_string()]
    );
    let msg = err.to_string();
    assert!(msg.contains("/wt/gates"), "path missing: {msg}");
    assert!(msg.contains("src/leftover.rs"), "dirty path missing: {msg}");
    assert!(
        msg.contains("--override-gate` does NOT bypass"),
        "the message must say an override will not help: {msg}"
    );
}

#[test]
fn precondition_c_is_skipped_when_no_worktree_is_known() {
    let mut store = seed();
    add_plan(&mut store, "plan-one", &phase_item(), PlanStatus::Approved);
    add_change_review(
        &mut store,
        "2026-09-13-1000-aaaa",
        "plan-one",
        Some(Verdict::Approve),
        ReviewState::Submitted,
    );
    // A probe that knows nothing about this item: `Ok(None)` is a benign miss.
    let probe = MemoryWorktreeProbe::new();
    let d = check_reviewed_gate(
        &store,
        PROJECT,
        &phase_item(),
        &ReviewedGate::enforcing(Some(&probe)),
    )
    .unwrap();
    assert!(matches!(d, GateDecision::Satisfied { .. }));
}

#[test]
fn an_unobservable_worktree_refuses_rather_than_reading_as_clean() {
    let mut store = seed();
    add_plan(&mut store, "plan-one", &phase_item(), PlanStatus::Approved);
    add_change_review(
        &mut store,
        "2026-09-13-1000-aaaa",
        "plan-one",
        Some(Verdict::Approve),
        ReviewState::Submitted,
    );
    let probe = MemoryWorktreeProbe::new().with_error(&phase_item());
    let err = check_reviewed_gate(
        &store,
        PROJECT,
        &phase_item(),
        &ReviewedGate::enforcing(Some(&probe)),
    )
    .unwrap_err();
    assert!(
        matches!(err, Error::GateWorktreeUnobservable { .. }),
        "got {err:?}"
    );
}

#[test]
fn the_evaluation_order_is_fixed_a_then_b_then_c() {
    let mut store = seed();
    // Everything is wrong at once: no plan AND a dirty worktree. The most
    // specific, earliest failure must be the one reported.
    let probe = MemoryWorktreeProbe::new().with_worktree(&phase_item(), "/wt/gates", &["dirty.rs"]);
    let err = check_reviewed_gate(
        &store,
        PROJECT,
        &phase_item(),
        &ReviewedGate::enforcing(Some(&probe)),
    )
    .unwrap_err();
    assert!(
        matches!(err, Error::GateNoApprovedPlan(_)),
        "(a) must be reported first, got {err:?}"
    );

    // With (a) satisfied, (b) is reported before (c).
    add_plan(&mut store, "plan-one", &phase_item(), PlanStatus::Approved);
    let err = check_reviewed_gate(
        &store,
        PROJECT,
        &phase_item(),
        &ReviewedGate::enforcing(Some(&probe)),
    )
    .unwrap_err();
    assert!(
        matches!(err, Error::GateNoApprovedChangeReview { .. }),
        "(b) must be reported before (c), got {err:?}"
    );
}

#[test]
fn all_three_preconditions_holding_yields_satisfied() {
    let mut store = seed();
    add_plan(&mut store, "plan-one", &phase_item(), PlanStatus::Approved);
    add_change_review(
        &mut store,
        "2026-09-13-1000-aaaa",
        "plan-one",
        Some(Verdict::Approve),
        ReviewState::Submitted,
    );
    let probe = MemoryWorktreeProbe::new()
        .with_worktree(&phase_item(), "/wt/gates", &[])
        .with_head(&phase_item(), &"a".repeat(40));
    let d = check_reviewed_gate(
        &store,
        PROJECT,
        &phase_item(),
        &ReviewedGate::enforcing(Some(&probe)),
    )
    .unwrap();
    assert_eq!(
        d,
        GateDecision::Satisfied {
            plan: "plan-one".to_string(),
            review_id: "2026-09-13-1000-aaaa".to_string(),
        }
    );
}

#[test]
fn the_gate_reads_through_the_same_store_the_write_goes_through() {
    // An approved plan and its approving review created earlier in the SAME
    // uncommitted session must count — otherwise an agent has to `rdm commit`
    // mid-dispatch just to satisfy its own gate.
    let mut store = seed();
    add_plan(&mut store, "plan-one", &phase_item(), PlanStatus::Approved);
    add_change_review(
        &mut store,
        "2026-09-13-1000-aaaa",
        "plan-one",
        Some(Verdict::Approve),
        ReviewState::Submitted,
    );
    // Nothing committed; the reads still see it.
    let doc = rdm_core::ops::phase::update_phase_gated(
        &mut store,
        PROJECT,
        ROADMAP,
        STEM,
        Some(PhaseStatus::Reviewed),
        TagsUpdate::Keep,
        BodyUpdate::Keep,
        None,
        None,
        None,
        TitleUpdate::Keep,
        &ReviewedGate::enforcing(None),
    )
    .unwrap();
    assert_eq!(doc.frontmatter.status, PhaseStatus::Reviewed);
}

// ---------------------------------------------------------------------------
// AC1 (threading) — the gated entries refuse where the primitives still write
// ---------------------------------------------------------------------------

#[test]
fn gated_entry_refuses_where_ungated_primitive_writes() {
    let mut store = seed();

    // The gated phase entry refuses: no records exist.
    let err = rdm_core::ops::phase::update_phase_gated(
        &mut store,
        PROJECT,
        ROADMAP,
        STEM,
        Some(PhaseStatus::Reviewed),
        TagsUpdate::Keep,
        BodyUpdate::Keep,
        None,
        None,
        None,
        TitleUpdate::Keep,
        &ReviewedGate::enforcing(None),
    )
    .unwrap_err();
    assert!(matches!(err, Error::GateNoApprovedPlan(_)));
    // The refusal left the file untouched.
    let doc = rdm_core::io::load_phase(&store, PROJECT, ROADMAP, STEM).unwrap();
    assert_eq!(doc.frontmatter.status, PhaseStatus::NeedsReview);

    // The UNGATED primitive is deliberately unchecked and still writes. The
    // allowlist harness (`scripts/verify-reviewed-gate.sh`) is what bounds its
    // callers, not a runtime check.
    let doc = rdm_core::ops::phase::update_phase(
        &mut store,
        PROJECT,
        ROADMAP,
        STEM,
        Some(PhaseStatus::Reviewed),
        TagsUpdate::Keep,
        BodyUpdate::Keep,
        None,
        None,
        None,
        TitleUpdate::Keep,
    )
    .unwrap();
    assert_eq!(doc.frontmatter.status, PhaseStatus::Reviewed);

    // Same for tasks.
    let err = rdm_core::ops::task::update_task_gated(
        &mut store,
        PROJECT,
        "solo",
        Some(TaskStatus::Reviewed),
        None,
        TagsUpdate::Keep,
        BodyUpdate::Keep,
        None,
        None,
        None,
        TitleUpdate::Keep,
        &ReviewedGate::enforcing(None),
    )
    .unwrap_err();
    assert!(matches!(err, Error::GateNoApprovedPlan(_)));
    let doc = rdm_core::ops::task::update_task(
        &mut store,
        PROJECT,
        "solo",
        Some(TaskStatus::Reviewed),
        None,
        TagsUpdate::Keep,
        BodyUpdate::Keep,
        None,
        None,
        None,
        TitleUpdate::Keep,
    )
    .unwrap();
    assert_eq!(doc.frontmatter.status, TaskStatus::Reviewed);
}

#[test]
fn the_gate_only_fires_on_a_transition_to_reviewed() {
    let mut store = seed();
    // No records exist, but these transitions are not gated.
    for status in [
        PhaseStatus::InProgress,
        PhaseStatus::NeedsReview,
        PhaseStatus::Done,
        PhaseStatus::Blocked,
        PhaseStatus::WontFix,
    ] {
        rdm_core::ops::phase::update_phase_gated(
            &mut store,
            PROJECT,
            ROADMAP,
            STEM,
            Some(status),
            TagsUpdate::Keep,
            BodyUpdate::Keep,
            None,
            None,
            None,
            TitleUpdate::Keep,
            &ReviewedGate::enforcing(None),
        )
        .unwrap_or_else(|e| panic!("{status} must not be gated, got {e}"));
    }
}

#[test]
fn rewriting_reviewed_onto_an_already_reviewed_item_re_evaluates_the_gate() {
    // `reviewed` is non-terminal, so `apply_phase_update`'s terminal-status
    // short-circuit does not apply here — but assert it, because that
    // short-circuit is the obvious place for a laundering hole.
    let mut store = seed();
    rdm_core::ops::phase::update_phase(
        &mut store,
        PROJECT,
        ROADMAP,
        STEM,
        Some(PhaseStatus::Reviewed),
        TagsUpdate::Keep,
        BodyUpdate::Keep,
        None,
        None,
        None,
        TitleUpdate::Keep,
    )
    .unwrap();
    let err = rdm_core::ops::phase::update_phase_gated(
        &mut store,
        PROJECT,
        ROADMAP,
        STEM,
        Some(PhaseStatus::Reviewed),
        TagsUpdate::Keep,
        BodyUpdate::Keep,
        None,
        None,
        None,
        TitleUpdate::Keep,
        &ReviewedGate::enforcing(None),
    )
    .unwrap_err();
    assert!(matches!(err, Error::GateNoApprovedPlan(_)));
}

// ---------------------------------------------------------------------------
// AC2 — the operator override
// ---------------------------------------------------------------------------

#[test]
fn an_override_waives_a_and_b_but_never_c() {
    let store = seed();
    // No plan, no review, clean worktree → allowed, and recorded.
    let clean = MemoryWorktreeProbe::new()
        .with_worktree(&phase_item(), "/wt/gates", &[])
        .with_head(&phase_item(), &"a".repeat(40));
    let d = check_reviewed_gate(
        &store,
        PROJECT,
        &phase_item(),
        &ReviewedGate::enforcing(Some(&clean)).with_override("hotfix", "alice"),
    )
    .unwrap();
    let GateDecision::Overridden(o) = d else {
        panic!("expected Overridden, got {d:?}");
    };
    assert_eq!(o.reason, "hotfix");
    assert_eq!(o.actor, "alice");

    // Same override, DIRTY worktree → still refused. (c) survives an override.
    let dirty = MemoryWorktreeProbe::new().with_worktree(&phase_item(), "/wt/gates", &["oops.rs"]);
    let err = check_reviewed_gate(
        &store,
        PROJECT,
        &phase_item(),
        &ReviewedGate::enforcing(Some(&dirty)).with_override("hotfix", "alice"),
    )
    .unwrap_err();
    assert!(
        matches!(err, Error::GateWorktreeDirty { .. }),
        "got {err:?}"
    );
}

#[test]
fn an_empty_override_reason_is_refused() {
    let store = seed();
    for reason in ["", "   ", "\t\n"] {
        let err = check_reviewed_gate(
            &store,
            PROJECT,
            &phase_item(),
            &ReviewedGate::enforcing(None).with_override(reason, "alice"),
        )
        .unwrap_err();
        assert!(matches!(err, Error::GateOverrideEmptyReason), "got {err:?}");
    }
}

#[test]
fn an_override_is_recorded_then_cleared_on_leaving_reviewed() {
    let mut store = seed();
    let doc = rdm_core::ops::phase::update_phase_gated(
        &mut store,
        PROJECT,
        ROADMAP,
        STEM,
        Some(PhaseStatus::Reviewed),
        TagsUpdate::Keep,
        BodyUpdate::Keep,
        None,
        None,
        None,
        TitleUpdate::Keep,
        &ReviewedGate::enforcing(None).with_override("operator: hotfix", "alice"),
    )
    .unwrap();
    let o = doc.frontmatter.gate_override.as_ref().expect("recorded");
    assert_eq!(o.reason, "operator: hotfix");
    assert_eq!(o.actor, "alice");

    // Leaving `reviewed` drops it...
    let doc = rdm_core::ops::phase::update_phase_gated(
        &mut store,
        PROJECT,
        ROADMAP,
        STEM,
        Some(PhaseStatus::InProgress),
        TagsUpdate::Keep,
        BodyUpdate::Keep,
        None,
        None,
        None,
        TitleUpdate::Keep,
        &ReviewedGate::enforcing(None),
    )
    .unwrap();
    assert!(doc.frontmatter.gate_override.is_none());

    // ...so a later bare `reviewed` write is refused again: a stale override
    // can never authorize a second transition.
    let err = rdm_core::ops::phase::update_phase_gated(
        &mut store,
        PROJECT,
        ROADMAP,
        STEM,
        Some(PhaseStatus::Reviewed),
        TagsUpdate::Keep,
        BodyUpdate::Keep,
        None,
        None,
        None,
        TitleUpdate::Keep,
        &ReviewedGate::enforcing(None),
    )
    .unwrap_err();
    assert!(matches!(err, Error::GateNoApprovedPlan(_)));
}

#[test]
fn a_satisfied_gate_clears_a_previously_recorded_override() {
    let mut store = seed();
    rdm_core::ops::phase::update_phase_gated(
        &mut store,
        PROJECT,
        ROADMAP,
        STEM,
        Some(PhaseStatus::Reviewed),
        TagsUpdate::Keep,
        BodyUpdate::Keep,
        None,
        None,
        None,
        TitleUpdate::Keep,
        &ReviewedGate::enforcing(None).with_override("operator: hotfix", "alice"),
    )
    .unwrap();
    // Now the real records land; re-writing `reviewed` is authorized on its
    // own merits, so the stale bypass must stop claiming otherwise.
    add_plan(&mut store, "plan-one", &phase_item(), PlanStatus::Approved);
    add_change_review(
        &mut store,
        "2026-09-13-1000-aaaa",
        "plan-one",
        Some(Verdict::Approve),
        ReviewState::Submitted,
    );
    let doc = rdm_core::ops::phase::update_phase_gated(
        &mut store,
        PROJECT,
        ROADMAP,
        STEM,
        Some(PhaseStatus::Reviewed),
        TagsUpdate::Keep,
        BodyUpdate::Keep,
        None,
        None,
        None,
        TitleUpdate::Keep,
        &ReviewedGate::enforcing(None),
    )
    .unwrap();
    assert!(doc.frontmatter.gate_override.is_none());
}

#[test]
fn a_task_override_is_recorded_and_a_status_none_update_preserves_it() {
    let mut store = seed();
    let item = task_item("solo");
    let d = check_reviewed_gate(
        &store,
        PROJECT,
        &item,
        &ReviewedGate::enforcing(None).with_override("hotfix", "bob"),
    )
    .unwrap();
    assert!(matches!(d, GateDecision::Overridden(_)));

    let doc = rdm_core::ops::task::update_task_gated(
        &mut store,
        PROJECT,
        "solo",
        Some(TaskStatus::Reviewed),
        None,
        TagsUpdate::Keep,
        BodyUpdate::Keep,
        None,
        None,
        None,
        TitleUpdate::Keep,
        &ReviewedGate::enforcing(None).with_override("hotfix", "bob"),
    )
    .unwrap();
    assert_eq!(doc.frontmatter.gate_override.as_ref().unwrap().actor, "bob");

    // A `status: None` update preserves it, like every other status-coupled
    // field.
    let doc = rdm_core::ops::task::update_task(
        &mut store,
        PROJECT,
        "solo",
        None,
        None,
        TagsUpdate::Set(vec!["kept".to_string()]),
        BodyUpdate::Keep,
        None,
        None,
        None,
        TitleUpdate::Keep,
    )
    .unwrap();
    assert!(doc.frontmatter.gate_override.is_some());

    // But a status change away from `reviewed` — even through the UNGATED
    // primitive — drops it.
    let doc = rdm_core::ops::task::update_task(
        &mut store,
        PROJECT,
        "solo",
        Some(TaskStatus::InProgress),
        None,
        TagsUpdate::Keep,
        BodyUpdate::Keep,
        None,
        None,
        None,
        TitleUpdate::Keep,
    )
    .unwrap();
    assert!(doc.frontmatter.gate_override.is_none());
}

#[test]
fn a_plan_implementing_a_different_item_does_not_satisfy_the_gate() {
    let mut store = seed();
    // The plan implements the TASK, not the phase.
    add_plan(
        &mut store,
        "task-plan",
        &task_item("solo"),
        PlanStatus::Approved,
    );
    add_change_review(
        &mut store,
        "2026-09-13-1000-aaaa",
        "task-plan",
        Some(Verdict::Approve),
        ReviewState::Submitted,
    );
    let err = check_reviewed_gate(
        &store,
        PROJECT,
        &phase_item(),
        &ReviewedGate::enforcing(None),
    )
    .unwrap_err();
    assert!(matches!(err, Error::GateNoApprovedPlan(_)));
    // ...while the task itself passes.
    let d = check_reviewed_gate(
        &store,
        PROJECT,
        &task_item("solo"),
        &ReviewedGate::enforcing(None),
    )
    .unwrap();
    assert!(matches!(d, GateDecision::Satisfied { .. }));
}

#[test]
fn gate_rejects_stale_head_and_accepts_matching_later_review() {
    let mut store = seed();
    let item = phase_item();
    add_plan(&mut store, "plan", &item, PlanStatus::Approved);
    add_change_review(
        &mut store,
        "old",
        "plan",
        Some(Verdict::Approve),
        ReviewState::Submitted,
    );
    let probe = MemoryWorktreeProbe::new()
        .with_worktree(&item, "/shared", &[])
        .with_head(&item, &"b".repeat(40));
    let gate = ReviewedGate::enforcing(Some(&probe));
    assert!(check_reviewed_gate(&store, PROJECT, &item, &gate).is_err());
    add_change_review(
        &mut store,
        "new",
        "plan",
        Some(Verdict::Approve),
        ReviewState::Submitted,
    );
    let mut review = rdm_core::io::load_review(&store, PROJECT, "new").unwrap();
    review.frontmatter.target = ReviewTarget::Change {
        head: "b".repeat(40),
        base: None,
    };
    rdm_core::io::write_review(&mut store, PROJECT, "new", &review).unwrap();
    assert!(
        matches!(check_reviewed_gate(&store, PROJECT, &item, &gate).unwrap(), GateDecision::Satisfied { review_id, .. } if review_id == "new")
    );
}

// ---------------------------------------------------------------------------
// AC5 — a dismissed approval is not gate evidence
// ---------------------------------------------------------------------------

#[test]
fn a_dismissed_approving_change_review_does_not_satisfy_precondition_b() {
    // Dismissal closes a review *without it being acted on*, which retracts
    // the approval as gate evidence. It is filtered out at the plan-helper
    // layer, so it is never even a stale candidate.
    let mut store = seed();
    add_plan(&mut store, "plan-one", &phase_item(), PlanStatus::Approved);
    add_change_review(
        &mut store,
        "2026-09-13-1000-aaaa",
        "plan-one",
        Some(Verdict::Approve),
        ReviewState::Dismissed,
    );
    let err = check_reviewed_gate(
        &store,
        PROJECT,
        &phase_item(),
        &ReviewedGate::enforcing(None),
    )
    .unwrap_err();
    assert!(
        matches!(err, Error::GateNoApprovedChangeReview { .. }),
        "a dismissed approval must read as absent, got {err:?}"
    );
    assert!(
        !matches!(err, Error::GateStaleChangeReview { .. }),
        "a dismissed approval is excluded before the head comparison, got {err:?}"
    );

    // And the boundary: `addressed` deliberately still counts, so the rule
    // is a dismissal rule, not a terminal-state rule.
    add_change_review(
        &mut store,
        "2026-09-13-1001-bbbb",
        "plan-one",
        Some(Verdict::Approve),
        ReviewState::Addressed,
    );
    let d = check_reviewed_gate(
        &store,
        PROJECT,
        &phase_item(),
        &ReviewedGate::enforcing(None),
    )
    .unwrap();
    assert!(
        matches!(d, GateDecision::Satisfied { ref review_id, .. } if review_id == "2026-09-13-1001-bbbb")
    );
}

#[test]
fn a_dismissed_request_changes_review_stays_excluded_too() {
    let mut store = seed();
    add_plan(&mut store, "plan-one", &phase_item(), PlanStatus::Approved);
    add_change_review(
        &mut store,
        "2026-09-13-1000-aaaa",
        "plan-one",
        Some(Verdict::RequestChanges),
        ReviewState::Dismissed,
    );
    // A draft carrying a hand-written verdict is likewise excluded.
    add_change_review(
        &mut store,
        "2026-09-13-1001-bbbb",
        "plan-one",
        Some(Verdict::Approve),
        ReviewState::Draft,
    );
    assert!(matches!(
        check_reviewed_gate(
            &store,
            PROJECT,
            &phase_item(),
            &ReviewedGate::enforcing(None)
        ),
        Err(Error::GateNoApprovedChangeReview { .. })
    ));
}

// ---------------------------------------------------------------------------
// AC6 — a stale approval is named as stale, on dirty and clean checkouts alike
// ---------------------------------------------------------------------------

/// A store with one approved plan and one approving change review recorded
/// at `add_change_review`'s default head (40 × 'a').
fn seed_with_approval() -> MemoryStore {
    let mut store = seed();
    add_plan(&mut store, "plan-one", &phase_item(), PlanStatus::Approved);
    add_change_review(
        &mut store,
        "2026-09-13-1000-aaaa",
        "plan-one",
        Some(Verdict::Approve),
        ReviewState::Submitted,
    );
    store
}

#[test]
fn a_stale_approving_change_review_is_refused_as_stale_not_absent() {
    let store = seed_with_approval();
    let probe = MemoryWorktreeProbe::new()
        .with_worktree(&phase_item(), "/wt/gates", &[])
        .with_head(&phase_item(), &"b".repeat(40));
    let err = check_reviewed_gate(
        &store,
        PROJECT,
        &phase_item(),
        &ReviewedGate::enforcing(Some(&probe)),
    )
    .unwrap_err();
    let Error::GateStaleChangeReview {
        ref review_id,
        ref reviewed_head,
        ref observed_head,
        ..
    } = err
    else {
        panic!("expected GateStaleChangeReview, got {err:?}");
    };
    assert_eq!(review_id, "2026-09-13-1000-aaaa");
    assert_eq!(reviewed_head, &"a".repeat(40));
    assert_eq!(observed_head.as_deref(), Some("b".repeat(40).as_str()));
    assert!(
        !matches!(err, Error::GateNoApprovedChangeReview { .. }),
        "an existing approval must never be reported as absent"
    );
    let msg = err.to_string();
    assert!(msg.contains("aaaaaaaaaaaa"), "reviewed head missing: {msg}");
    assert!(msg.contains("bbbbbbbbbbbb"), "observed head missing: {msg}");
    assert!(
        msg.contains("rdm review start --on change/HEAD"),
        "the re-review remedy is missing: {msg}"
    );
    assert!(
        msg.contains("--override-gate"),
        "the bypass remedy is missing: {msg}"
    );
}

#[test]
fn a_dirty_worktree_with_a_stale_review_still_reports_the_stale_refusal() {
    // The (a) → (b) → (c) order the module documents only actually holds if
    // the HEAD match is decoupled from cleanliness. With the old
    // `if check.is_clean()` guard this returned GateWorktreeDirty, hiding
    // the real cause.
    let store = seed_with_approval();
    let probe = MemoryWorktreeProbe::new()
        .with_worktree(&phase_item(), "/wt/gates", &["oops.rs"])
        .with_head(&phase_item(), &"b".repeat(40));
    let err = check_reviewed_gate(
        &store,
        PROJECT,
        &phase_item(),
        &ReviewedGate::enforcing(Some(&probe)),
    )
    .unwrap_err();
    assert!(
        matches!(err, Error::GateStaleChangeReview { .. }),
        "got {err:?}"
    );
    assert!(
        !matches!(err, Error::GateWorktreeDirty { .. }),
        "the cleanliness complaint must not hide the stale approval: {err:?}"
    );
}

#[test]
fn an_observed_checkout_with_no_readable_head_cannot_match_an_approval() {
    let store = seed_with_approval();
    for dirty in [&[][..], &["oops.rs"][..]] {
        let probe = MemoryWorktreeProbe::new().with_worktree(&phase_item(), "/wt/gates", dirty);
        let err = check_reviewed_gate(
            &store,
            PROJECT,
            &phase_item(),
            &ReviewedGate::enforcing(Some(&probe)),
        )
        .unwrap_err();
        let Error::GateStaleChangeReview {
            ref observed_head, ..
        } = err
        else {
            panic!("expected GateStaleChangeReview for dirty={dirty:?}, got {err:?}");
        };
        assert_eq!(observed_head, &None);
        let msg = err.to_string();
        assert!(
            msg.contains("could not be observed"),
            "the message must say the HEAD could not be read: {msg}"
        );
        assert!(
            msg.contains("rdm review start --on change/HEAD"),
            "the re-review remedy is missing: {msg}"
        );
    }
}

#[test]
fn the_decoupling_never_swallows_precondition_c() {
    let store = seed_with_approval();
    // Matching head, clean → satisfied.
    let clean = MemoryWorktreeProbe::new()
        .with_worktree(&phase_item(), "/wt/gates", &[])
        .with_head(&phase_item(), &"a".repeat(40));
    assert!(matches!(
        check_reviewed_gate(
            &store,
            PROJECT,
            &phase_item(),
            &ReviewedGate::enforcing(Some(&clean))
        )
        .unwrap(),
        GateDecision::Satisfied { .. }
    ));
    // Matching head, DIRTY → still the cleanliness refusal.
    let dirty = MemoryWorktreeProbe::new()
        .with_worktree(&phase_item(), "/wt/gates", &["oops.rs"])
        .with_head(&phase_item(), &"a".repeat(40));
    let err = check_reviewed_gate(
        &store,
        PROJECT,
        &phase_item(),
        &ReviewedGate::enforcing(Some(&dirty)),
    )
    .unwrap_err();
    assert!(
        matches!(err, Error::GateWorktreeDirty { .. }),
        "got {err:?}"
    );
}

#[test]
fn an_override_never_produces_a_stale_refusal() {
    // (b) is waived entirely, so a stale approval is unreachable — including
    // on a dirty checkout, where (c) still refuses.
    let store = seed_with_approval();
    let clean = MemoryWorktreeProbe::new()
        .with_worktree(&phase_item(), "/wt/gates", &[])
        .with_head(&phase_item(), &"b".repeat(40));
    assert!(matches!(
        check_reviewed_gate(
            &store,
            PROJECT,
            &phase_item(),
            &ReviewedGate::enforcing(Some(&clean)).with_override("hotfix", "alice"),
        )
        .unwrap(),
        GateDecision::Overridden(_)
    ));
    let dirty = MemoryWorktreeProbe::new()
        .with_worktree(&phase_item(), "/wt/gates", &["oops.rs"])
        .with_head(&phase_item(), &"b".repeat(40));
    let err = check_reviewed_gate(
        &store,
        PROJECT,
        &phase_item(),
        &ReviewedGate::enforcing(Some(&dirty)).with_override("hotfix", "alice"),
    )
    .unwrap_err();
    assert!(
        matches!(err, Error::GateWorktreeDirty { .. }),
        "got {err:?}"
    );
}

#[test]
fn a_probe_error_never_surfaces_as_a_stale_refusal() {
    // A probe `Err` observes nothing, so (b) compares no heads and (c)
    // reports the fail-closed unobservable refusal — AC3 and AC6 do not
    // interact.
    let store = seed_with_approval();
    let probe = MemoryWorktreeProbe::new().with_error(&phase_item());
    let err = check_reviewed_gate(
        &store,
        PROJECT,
        &phase_item(),
        &ReviewedGate::enforcing(Some(&probe)),
    )
    .unwrap_err();
    assert!(
        matches!(err, Error::GateWorktreeUnobservable { .. }),
        "got {err:?}"
    );
}

#[test]
fn a_fresh_approval_wins_over_a_stale_one_across_plans() {
    let mut store = seed();
    add_plan(&mut store, "plan-a", &phase_item(), PlanStatus::Approved);
    add_plan(&mut store, "plan-b", &phase_item(), PlanStatus::Approved);
    // Stale under plan-a.
    add_change_review(
        &mut store,
        "2026-09-13-1000-aaaa",
        "plan-a",
        Some(Verdict::Approve),
        ReviewState::Submitted,
    );
    // Fresh under plan-b.
    add_change_review(
        &mut store,
        "2026-09-13-1001-bbbb",
        "plan-b",
        Some(Verdict::Approve),
        ReviewState::Submitted,
    );
    let mut fresh = rdm_core::io::load_review(&store, PROJECT, "2026-09-13-1001-bbbb").unwrap();
    fresh.frontmatter.target = ReviewTarget::Change {
        head: "b".repeat(40),
        base: None,
    };
    rdm_core::io::write_review(&mut store, PROJECT, "2026-09-13-1001-bbbb", &fresh).unwrap();

    let probe = MemoryWorktreeProbe::new()
        .with_worktree(&phase_item(), "/wt/gates", &[])
        .with_head(&phase_item(), &"b".repeat(40));
    let d = check_reviewed_gate(
        &store,
        PROJECT,
        &phase_item(),
        &ReviewedGate::enforcing(Some(&probe)),
    )
    .unwrap();
    assert!(
        matches!(d, GateDecision::Satisfied { ref review_id, .. } if review_id == "2026-09-13-1001-bbbb"),
        "the fresh approval must win and raise no stale error: {d:?}"
    );
}

#[test]
fn a_task_stale_approval_reports_the_same_refusal() {
    let mut store = seed();
    let item = task_item("solo");
    add_plan(&mut store, "plan-task", &item, PlanStatus::Approved);
    add_change_review(
        &mut store,
        "2026-09-13-1000-aaaa",
        "plan-task",
        Some(Verdict::Approve),
        ReviewState::Submitted,
    );
    let probe = MemoryWorktreeProbe::new()
        .with_worktree(&item, "/wt/solo", &[])
        .with_head(&item, &"c".repeat(40));
    let err = check_reviewed_gate(
        &store,
        PROJECT,
        &item,
        &ReviewedGate::enforcing(Some(&probe)),
    )
    .unwrap_err();
    let Error::GateStaleChangeReview { ref item, .. } = err else {
        panic!("expected GateStaleChangeReview, got {err:?}");
    };
    assert!(
        item.contains("solo"),
        "the refusal must name the item: {err}"
    );
}
