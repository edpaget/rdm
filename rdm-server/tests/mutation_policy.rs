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
