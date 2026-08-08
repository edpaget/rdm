//! Integration coverage for the server's mutation policy — the operator
//! decision recorded in `docs/scoping-model-decision.md`.
//!
//! The acceptance criterion this file answers to is "a server mutation
//! reaches a commit or fails loudly". Both halves are exercised end-to-end
//! against a real git-backed plan repo and a real bound TCP listener:
//!
//! - the shipped `StagingOnly` default does **not** commit, and says so on
//!   every mutating response (`X-Rdm-Staged`) alongside the changeset id
//!   needed to reconcile (`X-Rdm-Changeset`);
//! - `--autocommit` does commit, and the mutation is in git afterwards.
//!
//! Everything here is gated on the `git` feature: without it `post_mutate`
//! compiles to the staging branch only, so there is no autocommit to assert
//! on. Feature unification across the workspace turns it on for
//! `cargo nextest run` (rdm-cli's default features enable `rdm-server/git`).

#![cfg(feature = "git")]

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use rdm_core::session::SessionId;
use rdm_core::store::{Store, VersionedStore};
use rdm_server::router::{CHANGESET_HEADER, STAGED_HEADER};
use rdm_server::state::{AppState, MutationPolicy};
use rdm_store_git::GitStore;
use reqwest::Client;
use tempfile::TempDir;

/// Seeds a committed git-backed plan repo and spawns a server over it under
/// the given policy, pinned to an explicit changeset id.
///
/// The id is explicit so the test does not depend on whatever session the
/// test runner's own environment resolves to — and so the assertion that
/// the advertised id is the one that actually commits is meaningful.
async fn spawn(policy: MutationPolicy, changeset: &str) -> (TempDir, SocketAddr, Client) {
    let dir = TempDir::new().unwrap();
    let mut store = GitStore::init(dir.path()).unwrap();
    rdm_core::ops::init::init(&mut store).unwrap();
    rdm_core::ops::project::create_project(&mut store, "demo", "Demo Project").unwrap();
    Store::commit(&mut store).unwrap();
    store.commit_whole_tree("seed").unwrap();

    let state = AppState {
        plan_root: dir.path().to_path_buf(),
        quick_filters: Vec::new(),
        mutation_policy: policy,
        ..Default::default()
    }
    // The real default factory shape: the store is pinned to the changeset
    // the server advertises, so the writes journal where the response says
    // they did.
    .with_store_factory(Arc::new(|root: &Path, changeset: Option<&SessionId>| {
        let store = GitStore::new(root).expect("temp dir must be a git repo");
        Box::new(match changeset {
            Some(id) => store.with_session_id(id.clone()),
            None => store,
        }) as Box<dyn VersionedStore + Send + Sync>
    }))
    .with_resolved_changeset(Some(changeset.to_string()));

    let app = rdm_server::router::build_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (dir, addr, Client::new())
}

/// POSTs a task create, returning the response.
async fn create_task(client: &Client, addr: SocketAddr, slug: &str) -> reqwest::Response {
    client
        .post(format!("http://{addr}/projects/demo/tasks"))
        .header("accept", "application/hal+json")
        .json(&serde_json::json!({ "slug": slug, "title": "A Task" }))
        .send()
        .await
        .unwrap()
}

fn head_sha(root: &Path) -> String {
    VersionedStore::head_sha(&GitStore::new(root).unwrap()).unwrap()
}

/// The paths in the temp repo's HEAD commit.
///
/// Scrubs the ambient git environment before shelling out. Load-bearing, not
/// hygiene: this suite runs under rdm's own pre-commit hook, which exports
/// `GIT_DIR`/`GIT_WORK_TREE`, and an inherited pair points `git` at the
/// developer's repo instead of the temp one — so the assertions below would
/// be made against an unrelated commit.
fn head_paths(root: &Path) -> Vec<String> {
    let out = std::process::Command::new("git")
        .args(["show", "--name-only", "--pretty=format:", "HEAD"])
        .current_dir(root)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git show failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect()
}

#[tokio::test]
async fn staging_only_default_does_not_commit_and_says_so_on_every_mutating_response() {
    let (dir, addr, client) = spawn(MutationPolicy::StagingOnly, "server-cs").await;
    let before = head_sha(dir.path());

    let resp = create_task(&client, addr, "staged-task").await;
    assert_eq!(resp.status(), 201, "the mutation itself must succeed");

    // Loud per *mutation*, not merely per process: the response names the
    // changeset and the exact command that lands it.
    assert_eq!(
        resp.headers().get(CHANGESET_HEADER).unwrap(),
        "server-cs",
        "the mutating response must name the changeset to reconcile"
    );
    let staged = resp
        .headers()
        .get(STAGED_HEADER)
        .expect("a staged mutation must be reported on the response")
        .to_str()
        .unwrap()
        .to_string();
    assert!(staged.contains("NOT committed"), "{staged}");
    assert!(
        staged.contains("rdm commit --changeset server-cs"),
        "the header must carry the reconciliation command: {staged}"
    );

    // ...and it genuinely did not commit.
    assert_eq!(
        head_sha(dir.path()),
        before,
        "staging-only must not advance HEAD"
    );
    assert!(
        dir.path()
            .join("projects/demo/tasks/staged-task.md")
            .exists(),
        "the write is still on disk — staged, not lost"
    );
}

#[tokio::test]
async fn staging_only_does_not_tag_safe_method_responses_as_staged() {
    let (_dir, addr, client) = spawn(MutationPolicy::StagingOnly, "server-cs").await;
    let resp = client
        .get(format!("http://{addr}/projects/demo/tasks"))
        .header("accept", "application/hal+json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    // The changeset id is always discoverable...
    assert_eq!(resp.headers().get(CHANGESET_HEADER).unwrap(), "server-cs");
    // ...but a read staged nothing, so claiming otherwise would be noise on
    // every page load.
    assert!(
        resp.headers().get(STAGED_HEADER).is_none(),
        "a GET must not be reported as a staged mutation"
    );
}

#[tokio::test]
async fn autocommit_lands_the_mutation_scoped_to_the_advertised_changeset() {
    let (dir, addr, client) = spawn(MutationPolicy::Autocommit, "server-cs").await;
    let before = head_sha(dir.path());

    let resp = create_task(&client, addr, "landed-task").await;
    assert_eq!(resp.status(), 201);
    assert_eq!(resp.headers().get(CHANGESET_HEADER).unwrap(), "server-cs");
    assert!(
        resp.headers().get(STAGED_HEADER).is_none(),
        "under autocommit nothing is pending, so nothing is reported as staged"
    );

    let after = head_sha(dir.path());
    assert_ne!(after, before, "autocommit must land a real commit");
    let paths = head_paths(dir.path());
    assert!(
        paths
            .iter()
            .any(|p| p == "projects/demo/tasks/landed-task.md"),
        "the mutation must be in the commit, got {paths:?}"
    );
    // Still scoped: the commit carries this changeset's own write plus the
    // generated indexes it journaled, and nothing else.
    assert!(
        paths
            .iter()
            .all(|p| p == "projects/demo/tasks/landed-task.md"
                || rdm_core::paths::is_derived_path(p)),
        "the commit must stay inside the changeset's own paths, got {paths:?}"
    );
}

#[tokio::test]
async fn autocommit_lands_a_review_verdict_submitted_with_no_summary() {
    // The regression this covers: `submit_review` used to call
    // `post_mutate()` from *inside* the `ops::mutate` closure, behind the
    // guard that decides whether to set a summary. A verdict submitted with
    // no summary therefore never reached a commit at all.
    let (dir, addr, client) = spawn(MutationPolicy::Autocommit, "server-cs").await;

    let resp = client
        .post(format!("http://{addr}/projects/demo/tasks"))
        .header("accept", "application/hal+json")
        .json(&serde_json::json!({ "slug": "reviewed-task", "title": "Reviewed" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);

    let resp = client
        .post(format!("http://{addr}/projects/demo/reviews"))
        .header("accept", "application/hal+json")
        .json(&serde_json::json!({
            "author": "tester",
            "target": "task/reviewed-task",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201, "review draft must be created");
    let created: serde_json::Value = resp.json().await.unwrap();
    let review_id = created["id"].as_str().expect("review id").to_string();

    let resp = client
        .post(format!(
            "http://{addr}/projects/demo/reviews/{review_id}/comments"
        ))
        .header("accept", "application/hal+json")
        .json(&serde_json::json!({ "body": "Looks fine." }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201, "comment must be added");

    let before = head_sha(dir.path());
    // No `summary` field at all — the case that used to skip post_mutate.
    let resp = client
        .post(format!(
            "http://{addr}/projects/demo/reviews/{review_id}/submit"
        ))
        .header("accept", "application/hal+json")
        .json(&serde_json::json!({ "verdict": "approve" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "verdict submission must succeed");

    assert_ne!(
        head_sha(dir.path()),
        before,
        "a verdict submitted with no summary must still reach a commit"
    );
}

// ==================== Loud-failure branches of post_mutate ====================
//
// These drive `AppState::post_mutate_notices` directly rather than over HTTP.
// The notices are stderr writes from an in-process handler, which a reqwest
// client cannot see and libtest cannot hand back — but they are the entire
// substance of the "or fails loudly" half of the acceptance criterion, so they
// have to be assertable. `post_mutate` is the printing shell over exactly this
// function, so what is asserted here is what an operator reads.

/// Seeds a committed plan repo (no server) for the direct-`post_mutate` tests.
fn seed_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    let mut store = GitStore::init(dir.path()).unwrap();
    rdm_core::ops::init::init(&mut store).unwrap();
    rdm_core::ops::project::create_project(&mut store, "demo", "Demo Project").unwrap();
    Store::commit(&mut store).unwrap();
    store.commit_whole_tree("seed").unwrap();
    dir
}

/// Creates a task the way a handler does — through a store pinned to the
/// server's changeset, so the write is journaled under it.
fn journal_task(root: &Path, changeset: &str, slug: &str) {
    let mut store =
        GitStore::new(root)
            .unwrap()
            .with_session_id(SessionId::new(changeset).unwrap_or_else(|| {
                panic!("'{changeset}' must be a usable changeset id");
            }));
    rdm_core::ops::task::create_task(
        &mut store,
        rdm_core::ops::task::CreateTask {
            project: "demo",
            slug,
            title: "A Task",
            ..Default::default()
        },
    )
    .unwrap();
    Store::commit(&mut store).unwrap();
}

/// An autocommit `AppState` over `root`, pinned to `changeset`.
fn autocommit_state(root: &Path, changeset: &str) -> AppState {
    AppState {
        plan_root: root.to_path_buf(),
        quick_filters: Vec::new(),
        mutation_policy: MutationPolicy::Autocommit,
        ..Default::default()
    }
    .with_resolved_changeset(Some(changeset.to_string()))
}

#[tokio::test]
async fn autocommit_is_silent_when_it_lands_cleanly() {
    // The baseline the two tests below are read against: without this, a
    // notice asserted there could just be unconditional noise.
    let dir = seed_repo();
    journal_task(dir.path(), "server-cs", "clean-task");

    let notices = autocommit_state(dir.path(), "server-cs").post_mutate_notices();
    assert!(
        notices.is_empty(),
        "a clean autocommit has nothing to report: {notices:?}"
    );
}

#[tokio::test]
async fn autocommit_reports_a_vanished_journaled_path_even_when_it_lands_nothing() {
    // The dangerous branch: the changeset's only path is gone from disk, so
    // the scoped tree reconciles straight back to HEAD and the commit
    // correctly produces no SHA. Reporting only "landed nothing" would read as
    // "the changeset was empty" when in fact the server's write disappeared.
    let dir = seed_repo();
    journal_task(dir.path(), "server-cs", "vanishing-task");
    let path = dir.path().join("projects/demo/tasks/vanishing-task.md");
    assert!(path.exists(), "precondition: the task file was written");
    std::fs::remove_file(&path).unwrap();

    let before = head_sha(dir.path());
    let notices = autocommit_state(dir.path(), "server-cs").post_mutate_notices();
    let joined = notices.join("\n");
    assert!(
        joined.contains("ERROR: autocommit landed nothing"),
        "the operator must be told nothing landed: {joined}"
    );
    assert!(
        joined.contains("no longer on disk") && joined.contains("vanishing-task.md"),
        "and must be told WHY — which path vanished: {joined}"
    );
    assert!(
        joined.contains("rdm commit --changeset server-cs"),
        "every loud branch must carry the reconciliation command: {joined}"
    );
    assert_eq!(
        head_sha(dir.path()),
        before,
        "a changeset whose files all vanished must not create an empty commit"
    );
}

#[tokio::test]
async fn autocommit_reports_a_vanished_journaled_path_alongside_what_it_landed() {
    // A partial success is still a success, so this branch used to be silent
    // — the skipped path stays claimed by the changeset (truncation covers
    // only what landed) while the file backing it is gone.
    let dir = seed_repo();
    journal_task(dir.path(), "server-cs", "survivor");
    journal_task(dir.path(), "server-cs", "casualty");
    std::fs::remove_file(dir.path().join("projects/demo/tasks/casualty.md")).unwrap();

    let before = head_sha(dir.path());
    let notices = autocommit_state(dir.path(), "server-cs").post_mutate_notices();
    let joined = notices.join("\n");
    assert!(
        joined.contains("WARN: autocommit") && joined.contains("casualty.md"),
        "a partial success must still name the vanished path: {joined}"
    );
    assert!(
        !joined.contains("landed nothing"),
        "this commit DID land — it must not be reported as a total failure: {joined}"
    );

    assert_ne!(head_sha(dir.path()), before, "the survivor must land");
    let paths = head_paths(dir.path());
    assert!(
        paths.iter().any(|p| p == "projects/demo/tasks/survivor.md"),
        "the surviving path must be in the commit, got {paths:?}"
    );
    assert!(
        !paths.iter().any(|p| p == "projects/demo/tasks/casualty.md"),
        "a vanished path must never be committed, got {paths:?}"
    );
}

#[tokio::test]
async fn autocommit_is_loud_when_it_cannot_reach_the_plan_repo_at_all() {
    // The remaining loud branch: the write is on disk and journaled, but no
    // commit is possible. Silence here would lose the mutation with no trace.
    let dir = TempDir::new().unwrap(); // not a git repo
    let notices = autocommit_state(dir.path(), "server-cs").post_mutate_notices();
    let joined = notices.join("\n");
    assert!(
        joined.starts_with("ERROR: autocommit"),
        "an unusable plan repo must be an ERROR, not a shrug: {joined}"
    );
    assert!(
        joined.contains("rdm commit --changeset server-cs"),
        "and must still name how to reconcile: {joined}"
    );
}

#[tokio::test]
async fn staging_only_notices_are_exactly_the_per_mutation_staged_warning() {
    // `post_mutate` is the printing shell over `post_mutate_notices`, so the
    // staging branch must be represented here too — otherwise a regression
    // that silenced it would only show up in the header assertion above.
    let dir = seed_repo();
    let state = AppState {
        plan_root: dir.path().to_path_buf(),
        quick_filters: Vec::new(),
        mutation_policy: MutationPolicy::StagingOnly,
        ..Default::default()
    }
    .with_resolved_changeset(Some("server-cs".to_string()));

    assert_eq!(
        state.post_mutate_notices(),
        state.staged_notice().into_iter().collect::<Vec<_>>(),
        "staging-only must say the same thing on stderr that it puts on the response"
    );
    assert!(
        state
            .post_mutate_notices()
            .join("\n")
            .contains("NOT committed"),
        "and it must be unmistakable"
    );
}
