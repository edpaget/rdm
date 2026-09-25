//! HTTP-level coverage of the core-enforced `reviewed` transition gate.
//!
//! `rdm-server` is one of the surfaces this phase deliberately routed through
//! the gated update entries, because a `PATCH` body carries an arbitrary
//! caller-supplied status and can therefore reach `reviewed`. Using the
//! gated call site proves nothing about whether the gate actually refuses
//! over HTTP, that the config key is read from the right file, or that the
//! `Gate*` error variants really map to 409. That is what this file does.
//!
//! Everything is seeded through `rdm-core` against a temp plan repo, exactly
//! as `tests/integration.rs` does, and driven through a real TCP server.

use std::net::SocketAddr;
use std::path::Path;

use chrono::Utc;
use rdm_core::document::Document;
use rdm_core::link::ItemRef;
use rdm_core::model::{
    PhaseStatus, Plan, PlanStatus, Priority, Review, ReviewState, ReviewTarget, TaskStatus, Verdict,
};
use rdm_store_fs::FsStore;
use reqwest::Client;
use tempfile::TempDir;

const PROJECT: &str = "demo";
const ROADMAP: &str = "auth";
const STEM: &str = "phase-1-design";
const TASK: &str = "solo";

fn store_at(dir: &Path) -> FsStore {
    FsStore::new(dir)
}

fn flush(store: &mut FsStore) {
    rdm_core::store::Store::commit(store).unwrap();
}

/// A plan repo with one project, one roadmap + phase, and one task — each
/// parked at `needs-review`, the status a `reviewed` write follows.
fn seed_plan_repo(gate: Option<bool>) -> TempDir {
    let dir = TempDir::new().unwrap();
    let mut store = store_at(dir.path());
    rdm_core::ops::init::init(&mut store).unwrap();
    rdm_core::ops::project::create_project(&mut store, PROJECT, "Demo").unwrap();
    rdm_core::ops::roadmap::create_roadmap(
        &mut store,
        rdm_core::ops::roadmap::CreateRoadmap {
            project: PROJECT,
            slug: ROADMAP,
            title: "Auth",
            ..Default::default()
        },
    )
    .unwrap();
    rdm_core::ops::phase::create_phase(
        &mut store,
        rdm_core::ops::phase::CreatePhase {
            project: PROJECT,
            roadmap: ROADMAP,
            slug: "design",
            title: "Design",
            number: Some(1),
            ..Default::default()
        },
    )
    .unwrap();
    rdm_core::ops::task::create_task(
        &mut store,
        rdm_core::ops::task::CreateTask {
            project: PROJECT,
            slug: TASK,
            title: "Solo",
            priority: Priority::Medium,
            ..Default::default()
        },
    )
    .unwrap();
    rdm_core::ops::phase::update_phase(
        &mut store,
        PROJECT,
        ROADMAP,
        STEM,
        Some(PhaseStatus::NeedsReview),
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        None,
        None,
        None,
        rdm_core::ops::TitleUpdate::Keep,
    )
    .unwrap();
    rdm_core::ops::task::update_task(
        &mut store,
        PROJECT,
        TASK,
        Some(TaskStatus::NeedsReview),
        None,
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        None,
        None,
        None,
        rdm_core::ops::TitleUpdate::Keep,
    )
    .unwrap();
    flush(&mut store);

    // The repo-only opt-in. `None` writes no `[gates]` table at all, which is
    // the state every pre-existing plan repo is in.
    if let Some(on) = gate {
        std::fs::write(
            dir.path().join("rdm.toml"),
            format!("[gates]\nreviewed = {on}\n"),
        )
        .unwrap();
    }
    dir
}

fn phase_item() -> ItemRef {
    ItemRef::Phase {
        roadmap: ROADMAP.to_string(),
        stem: STEM.to_string(),
    }
}

fn task_item() -> ItemRef {
    ItemRef::Task {
        slug: TASK.to_string(),
    }
}

/// Writes an `approved` plan implementing `item` — precondition (a).
fn add_approved_plan(dir: &Path, slug: &str, item: &ItemRef) {
    let mut store = store_at(dir);
    let today = Utc::now().date_naive();
    rdm_core::io::write_plan(
        &mut store,
        PROJECT,
        slug,
        &Document {
            frontmatter: Plan {
                project: PROJECT.to_string(),
                plan: slug.to_string(),
                title: format!("Plan {slug}"),
                implements: item.clone(),
                supersedes: None,
                status: PlanStatus::Approved,
                created: today,
                updated: today,
            },
            body: "Plan body.".to_string(),
        },
    )
    .unwrap();
    flush(&mut store);
}

/// Writes a submitted, approving `change/<sha>` review whose `implements`
/// names `plan_slug` — precondition (b).
fn add_approving_change_review(dir: &Path, id: &str, plan_slug: &str) {
    let mut store = store_at(dir);
    rdm_core::io::write_review(
        &mut store,
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
                state: ReviewState::Submitted,
                verdict: Some(Verdict::Approve),
                created: Utc::now(),
                submitted: Some(Utc::now()),
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
    flush(&mut store);
}

async fn spawn(dir: &Path) -> (SocketAddr, Client) {
    let state = rdm_server::state::AppState {
        plan_root: dir.to_path_buf(),
        quick_filters: Vec::new(),
        ..Default::default()
    };
    let app = rdm_server::router::build_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, Client::new())
}

async fn patch_status(
    client: &Client,
    addr: SocketAddr,
    path: &str,
    status: &str,
) -> (u16, serde_json::Value) {
    let resp = client
        .patch(format!("http://{addr}{path}"))
        .header("accept", "application/hal+json")
        .json(&serde_json::json!({ "status": status }))
        .send()
        .await
        .unwrap();
    let code = resp.status().as_u16();
    (code, resp.json().await.unwrap())
}

fn phase_path() -> String {
    format!("/projects/{PROJECT}/roadmaps/{ROADMAP}/phases/{STEM}")
}

fn task_path() -> String {
    format!("/projects/{PROJECT}/tasks/{TASK}")
}

fn detail(body: &serde_json::Value) -> String {
    body["detail"].as_str().unwrap_or("").to_string()
}

// ---------------------------------------------------------------------------
// The gate really refuses over HTTP, with a distinct 409 per precondition
// ---------------------------------------------------------------------------

#[tokio::test]
async fn phase_patch_to_reviewed_walks_the_gate_ladder() {
    let dir = seed_plan_repo(Some(true));
    let (addr, client) = spawn(dir.path()).await;
    let path = phase_path();

    // Rung 1 — no plan implements the phase.
    let (code, body) = patch_status(&client, addr, &path, "reviewed").await;
    assert_eq!(code, 409, "rung 1 body: {body}");
    assert_eq!(body["status"], 409, "problem detail status: {body}");
    let d = detail(&body);
    assert!(d.contains("rdm plan create"), "rung 1 remediation: {d}");
    assert!(
        d.contains("phase/auth/phase-1-design"),
        "rung 1 item label: {d}"
    );
    assert!(
        !d.contains("rdm review start --on change/"),
        "rung 1 must not report rung 2's remediation: {d}"
    );

    // Rung 2 — an approved plan, but no approving change review.
    add_approved_plan(dir.path(), "design-plan", &phase_item());
    let (code, body) = patch_status(&client, addr, &path, "reviewed").await;
    assert_eq!(code, 409, "rung 2 body: {body}");
    let d = detail(&body);
    assert!(
        d.contains("rdm review start --on change/"),
        "rung 2 remediation: {d}"
    );
    assert!(d.contains("plan/design-plan"), "rung 2 names the plan: {d}");

    // Rung 3 — both records present. The server passes no worktree probe, so
    // precondition (c) is a documented skip and the write lands.
    add_approving_change_review(dir.path(), "2026-09-13-reviewer-1", "design-plan");
    let (code, body) = patch_status(&client, addr, &path, "reviewed").await;
    assert_eq!(code, 200, "rung 3 body: {body}");
    assert_eq!(body["status"], "reviewed");

    let store = store_at(dir.path());
    let doc = rdm_core::io::load_phase(&store, PROJECT, ROADMAP, STEM).unwrap();
    assert_eq!(doc.frontmatter.status, PhaseStatus::Reviewed);
}

#[tokio::test]
async fn task_patch_to_reviewed_walks_the_gate_ladder() {
    let dir = seed_plan_repo(Some(true));
    let (addr, client) = spawn(dir.path()).await;
    let path = task_path();

    let (code, body) = patch_status(&client, addr, &path, "reviewed").await;
    assert_eq!(code, 409, "rung 1 body: {body}");
    let d = detail(&body);
    assert!(d.contains("rdm plan create"), "rung 1 remediation: {d}");
    assert!(d.contains("task/solo"), "rung 1 item label: {d}");

    add_approved_plan(dir.path(), "solo-plan", &task_item());
    let (code, body) = patch_status(&client, addr, &path, "reviewed").await;
    assert_eq!(code, 409, "rung 2 body: {body}");
    assert!(
        detail(&body).contains("rdm review start --on change/"),
        "rung 2 remediation: {}",
        detail(&body)
    );

    add_approving_change_review(dir.path(), "2026-09-13-reviewer-1", "solo-plan");
    let (code, body) = patch_status(&client, addr, &path, "reviewed").await;
    assert_eq!(code, 200, "rung 3 body: {body}");
    assert_eq!(body["status"], "reviewed");

    let store = store_at(dir.path());
    let doc = rdm_core::io::load_task(&store, PROJECT, TASK).unwrap();
    assert_eq!(doc.frontmatter.status, TaskStatus::Reviewed);
}

// ---------------------------------------------------------------------------
// …and only there: the gate is opt-in, and it guards only `reviewed`
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_gate_is_off_unless_the_repo_opts_in() {
    // The property that keeps every pre-existing plan repo working over HTTP:
    // with no `[gates]` table, a `reviewed` PATCH needs no records at all.
    for gate in [None, Some(false)] {
        let dir = seed_plan_repo(gate);
        let (addr, client) = spawn(dir.path()).await;
        let (code, body) = patch_status(&client, addr, &phase_path(), "reviewed").await;
        assert_eq!(code, 200, "gate={gate:?} body: {body}");
        assert_eq!(body["status"], "reviewed");

        let (code, body) = patch_status(&client, addr, &task_path(), "reviewed").await;
        assert_eq!(code, 200, "gate={gate:?} body: {body}");
        assert_eq!(body["status"], "reviewed");
    }
}

#[tokio::test]
async fn an_enforcing_gate_does_not_block_other_transitions() {
    // The gate fires on `reviewed` alone. Every other status a PATCH can name
    // — including the terminal `done` — stays ungated by design.
    let dir = seed_plan_repo(Some(true));
    let (addr, client) = spawn(dir.path()).await;

    for status in ["in-progress", "blocked", "done"] {
        let (code, body) = patch_status(&client, addr, &phase_path(), status).await;
        assert_eq!(code, 200, "status={status} body: {body}");
        assert_eq!(body["status"], status);
    }
    for status in ["in-progress", "done"] {
        let (code, body) = patch_status(&client, addr, &task_path(), status).await;
        assert_eq!(code, 200, "status={status} body: {body}");
        assert_eq!(body["status"], status);
    }
}

// ---------------------------------------------------------------------------
// The config reader itself
// ---------------------------------------------------------------------------

#[test]
fn reviewed_gate_enabled_reads_the_repo_only_key() {
    let dir = TempDir::new().unwrap();
    // No rdm.toml at all: opt-in means off, and never an error.
    assert!(!rdm_server::state::reviewed_gate_enabled(dir.path(), PROJECT).unwrap());

    std::fs::write(dir.path().join("rdm.toml"), "[gates]\nreviewed = true\n").unwrap();
    assert!(rdm_server::state::reviewed_gate_enabled(dir.path(), PROJECT).unwrap());

    std::fs::write(dir.path().join("rdm.toml"), "[gates]\nreviewed = false\n").unwrap();
    assert!(!rdm_server::state::reviewed_gate_enabled(dir.path(), PROJECT).unwrap());

    // A neighbouring key must not be mistaken for it, and a malformed config
    // must never be the thing that turns a gate on.
    std::fs::write(dir.path().join("rdm.toml"), "plan_review = true\n").unwrap();
    assert!(!rdm_server::state::reviewed_gate_enabled(dir.path(), PROJECT).unwrap());
    std::fs::write(dir.path().join("rdm.toml"), "not = = toml [[[").unwrap();
    assert!(!rdm_server::state::reviewed_gate_enabled(dir.path(), PROJECT).unwrap());

    // A `[projects.<p>]` override is read for that project only.
    std::fs::write(
        dir.path().join("rdm.toml"),
        "[gates]\nreviewed = false\n\n[projects.a.gates]\nreviewed = true\n",
    )
    .unwrap();
    assert!(rdm_server::state::reviewed_gate_enabled(dir.path(), "a").unwrap());
    assert!(!rdm_server::state::reviewed_gate_enabled(dir.path(), "b").unwrap());
}

// ---------------------------------------------------------------------------
// The gate resolves per project: `[projects.<p>] gates.reviewed`
// ---------------------------------------------------------------------------

/// A plan repo with projects `a` and `b`, each holding roadmap `auth` with
/// `phase-1-design` and task `solo` parked at `needs-review`, and an `rdm.toml` that enables
/// the gate for project `a` only — no plan-repo-wide `[gates]` table.
fn seed_two_project_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    let mut store = store_at(dir.path());
    rdm_core::ops::init::init(&mut store).unwrap();
    for project in ["a", "b"] {
        rdm_core::ops::project::create_project(&mut store, project, project).unwrap();
        rdm_core::ops::roadmap::create_roadmap(
            &mut store,
            rdm_core::ops::roadmap::CreateRoadmap {
                project,
                slug: ROADMAP,
                title: "Auth",
                ..Default::default()
            },
        )
        .unwrap();
        rdm_core::ops::phase::create_phase(
            &mut store,
            rdm_core::ops::phase::CreatePhase {
                project,
                roadmap: ROADMAP,
                slug: "design",
                title: "Design",
                number: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        rdm_core::ops::phase::update_phase(
            &mut store,
            project,
            ROADMAP,
            STEM,
            Some(PhaseStatus::NeedsReview),
            rdm_core::ops::TagsUpdate::Keep,
            rdm_core::ops::BodyUpdate::Keep,
            None,
            None,
            None,
            None,
            rdm_core::ops::TitleUpdate::Keep,
        )
        .unwrap();
        rdm_core::ops::task::create_task(
            &mut store,
            rdm_core::ops::task::CreateTask {
                project,
                slug: TASK,
                title: "Solo",
                priority: Priority::Medium,
                ..Default::default()
            },
        )
        .unwrap();
        rdm_core::ops::task::update_task(
            &mut store,
            project,
            TASK,
            Some(TaskStatus::NeedsReview),
            None,
            rdm_core::ops::TagsUpdate::Keep,
            rdm_core::ops::BodyUpdate::Keep,
            None,
            None,
            None,
            None,
            rdm_core::ops::TitleUpdate::Keep,
        )
        .unwrap();
    }
    flush(&mut store);
    std::fs::write(
        dir.path().join("rdm.toml"),
        "[projects.a.gates]\nreviewed = true\n",
    )
    .unwrap();
    dir
}

#[tokio::test]
async fn a_project_scoped_gate_refuses_only_that_projects_reviewed_write() {
    let dir = seed_two_project_repo();
    let (addr, client) = spawn(dir.path()).await;

    let (code, body) = patch_status(
        &client,
        addr,
        &format!("/projects/a/roadmaps/{ROADMAP}/phases/{STEM}"),
        "reviewed",
    )
    .await;
    assert_eq!(code, 409, "project a is gated: {body}");
    assert!(
        detail(&body).contains("rdm plan create"),
        "a gate refusal, not some other conflict: {body}"
    );

    let (code, body) = patch_status(
        &client,
        addr,
        &format!("/projects/b/roadmaps/{ROADMAP}/phases/{STEM}"),
        "reviewed",
    )
    .await;
    assert_eq!(code, 200, "project b is not gated: {body}");
    assert_eq!(body["status"], "reviewed");
}

#[tokio::test]
async fn a_project_scoped_gate_refuses_only_that_projects_task_reviewed_write() {
    let dir = seed_two_project_repo();
    let (addr, client) = spawn(dir.path()).await;

    let (code, body) = patch_status(
        &client,
        addr,
        &format!("/projects/a/tasks/{TASK}"),
        "reviewed",
    )
    .await;
    assert_eq!(code, 409, "project a is gated: {body}");
    assert!(
        detail(&body).contains("rdm plan create"),
        "a gate refusal, not some other conflict: {body}"
    );

    let (code, body) = patch_status(
        &client,
        addr,
        &format!("/projects/b/tasks/{TASK}"),
        "reviewed",
    )
    .await;
    assert_eq!(code, 200, "project b is not gated: {body}");
    assert_eq!(body["status"], "reviewed");
}

// ---------------------------------------------------------------------------
// The newer refusals map to the same 409 bucket, never the opaque 500
// ---------------------------------------------------------------------------

/// The HTTP surface passes no worktree probe, so a stale-approval refusal is
/// not reachable through a `PATCH` — but the problem-detail mapping is what
/// decides its status code, and an unmapped variant would silently land in
/// the 500 bucket. Asserted directly against the conversion.
#[test]
fn a_stale_change_review_refusal_is_a_409_with_a_detail() {
    let err = rdm_core::error::Error::GateStaleChangeReview {
        item: "phase/auth/phase-1-design".to_string(),
        review_id: "2026-09-13-reviewer-1".to_string(),
        reviewed_head: "a".repeat(40),
        observed_head: Some("b".repeat(40)),
    };
    let p = rdm_server::problem::ProblemDetail::from(&err);
    assert_eq!(p.status, 409, "a stale approval is conflict-shaped");
    assert_eq!(p.title, "Conflict");
    let d = p.detail.expect("a refusal must carry a detail");
    assert!(d.contains("2026-09-13-reviewer-1"), "names the review: {d}");
    assert!(
        d.contains("rdm review start --on change/HEAD"),
        "names the remedy: {d}"
    );
}

/// AC3's two probe refusals are likewise conflict-shaped, not opaque 500s.
#[test]
fn the_probe_mismatch_refusals_are_409s_with_details() {
    for err in [
        rdm_core::error::Error::ReviewSourceItemMismatch {
            expected: "phase/auth/phase-1-design".to_string(),
            found: "task/solo".to_string(),
        },
        rdm_core::error::Error::ReviewSourceBranchChanged {
            path: "/wt/auth".to_string(),
            expected: "roadmap/auth".to_string(),
            found: "scratch".to_string(),
        },
    ] {
        let p = rdm_server::problem::ProblemDetail::from(&err);
        assert_eq!(p.status, 409, "{err:?} must be conflict-shaped");
        assert!(p.detail.is_some(), "{err:?} must carry a detail");
    }
}

/// AC1's path refusal is caller-fixable input, so it belongs in the 400
/// bucket rather than either the 409 or the opaque 500 one.
#[test]
fn an_unlinkable_source_path_is_a_400() {
    let p =
        rdm_server::problem::ProblemDetail::from(&rdm_core::error::Error::ChangePathNotLinkable {
            path: "web/app/@modal/page.tsx".to_string(),
            delimiter: '@',
        });
    assert_eq!(p.status, 400);
    assert!(p.detail.is_some_and(|d| d.contains('@')));
}
