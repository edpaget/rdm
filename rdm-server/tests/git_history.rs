//! Happy-path integration tests for `?at=<sha>` against a real git-backed
//! server.
//!
//! `rdm-server/tests/integration.rs` seeds every fixture through a plain
//! `FsStore`, whose `VersionedStore` impl always returns
//! `Error::HistoryUnavailable` — so its `?at=<sha>` tests only prove the
//! wire shape of a *failed* pinned read (404 Problem+JSON). This file
//! backs the server with a real `rdm_store_git::GitStore` (via
//! `AppState::with_store_factory`) and proves the *successful* path: a
//! historical SHA resolves to the historical body and is surfaced in the
//! `revision` field / HTML badge.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use rdm_core::model::Priority;
use rdm_core::ops::{BodyUpdate, PriorityUpdate, TagsUpdate, TitleUpdate};
use rdm_core::store::{Store, VersionedStore};
use rdm_store_git::GitStore;
use reqwest::Client;
use tempfile::TempDir;

/// Spawns a real TCP server backed by a real git repository.
///
/// Returns the `TempDir` (must outlive the test), the bound address, a
/// `reqwest` client, and the SHA of the commit captured right after the
/// "old" body was written for roadmap `api`, phase `phase-1-design`, and
/// task `bug-1`. A second commit then overwrites all three bodies with
/// `"new-body-marker"`, so a plain (non-`?at=`) read would observe
/// different content than the pinned "old" read.
async fn spawn_git_backed_server() -> (TempDir, SocketAddr, Client, String) {
    let dir = TempDir::new().unwrap();
    let mut store = GitStore::init(dir.path()).unwrap();

    rdm_core::ops::init::init(&mut store).unwrap();
    rdm_core::ops::project::create_project(&mut store, "demo", "Demo Project").unwrap();
    rdm_core::ops::roadmap::create_roadmap(
        &mut store,
        rdm_core::ops::roadmap::CreateRoadmap {
            project: "demo",
            slug: "api",
            title: "API Roadmap",
            body: Some("original-body-marker"),
            ..Default::default()
        },
    )
    .unwrap();
    rdm_core::ops::phase::create_phase(
        &mut store,
        rdm_core::ops::phase::CreatePhase {
            project: "demo",
            roadmap: "api",
            slug: "design",
            title: "Design Phase",
            number: Some(1),
            body: Some("original-body-marker"),
            ..Default::default()
        },
    )
    .unwrap();
    rdm_core::ops::task::create_task(
        &mut store,
        rdm_core::ops::task::CreateTask {
            project: "demo",
            slug: "bug-1",
            title: "Fix Bug One",
            priority: Priority::High,
            body: Some("original-body-marker"),
            ..Default::default()
        },
    )
    .unwrap();
    Store::commit(&mut store).unwrap();
    store.commit_now("seed original bodies").unwrap();
    let old_sha = VersionedStore::head_sha(&store).unwrap();

    // Second commit that changes all three bodies, so the "current" read
    // (no ?at=) would show different content than the pinned "old" read.
    rdm_core::ops::roadmap::update_roadmap(
        &mut store,
        "demo",
        "api",
        BodyUpdate::Set("new-body-marker".to_string()),
        PriorityUpdate::Keep,
        TagsUpdate::Keep,
        TitleUpdate::Keep,
    )
    .unwrap();
    rdm_core::ops::phase::update_phase(
        &mut store,
        "demo",
        "api",
        "phase-1-design",
        None,
        TagsUpdate::Keep,
        BodyUpdate::Set("new-body-marker".to_string()),
        None,
        None,
        None,
        TitleUpdate::Keep,
    )
    .unwrap();
    rdm_core::ops::task::update_task(
        &mut store,
        "demo",
        "bug-1",
        None,
        None,
        TagsUpdate::Keep,
        BodyUpdate::Set("new-body-marker".to_string()),
        None,
        None,
        None,
        TitleUpdate::Keep,
    )
    .unwrap();
    Store::commit(&mut store).unwrap();
    store
        .commit_now("overwrite bodies with new markers")
        .unwrap();

    let state = rdm_server::state::AppState {
        plan_root: dir.path().to_path_buf(),
        quick_filters: Vec::new(),
        ..Default::default()
    }
    .with_store_factory(Arc::new(|root: &Path| {
        // Only ever invoked against the git-initialized temp dir created by
        // `GitStore::init` above, so `GitStore::new` cannot realistically
        // fail here.
        Box::new(GitStore::new(root).expect("temp dir must be a git repo"))
            as Box<dyn VersionedStore + Send + Sync>
    }));

    let app = rdm_server::router::build_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (dir, addr, Client::new(), old_sha)
}

fn url(addr: SocketAddr, path: &str) -> String {
    format!("http://{addr}{path}")
}

#[tokio::test]
async fn roadmap_show_at_revision_returns_historical_body_and_revision() {
    let (_dir, addr, client, old_sha) = spawn_git_backed_server().await;

    let resp = client
        .get(url(
            addr,
            &format!("/projects/demo/roadmaps/api?at={old_sha}"),
        ))
        .header("accept", "application/hal+json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(json["revision"], old_sha);

    let resp = client
        .get(url(
            addr,
            &format!("/projects/demo/roadmaps/api?at={old_sha}"),
        ))
        .header("accept", "text/html")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("original-body-marker"));
    assert!(!body.contains("new-body-marker"));
    assert!(body.contains(&format!("Viewing revision {old_sha}")));
}

#[tokio::test]
async fn phase_show_at_revision_returns_historical_body_and_revision() {
    let (_dir, addr, client, old_sha) = spawn_git_backed_server().await;

    let resp = client
        .get(url(
            addr,
            &format!("/projects/demo/roadmaps/api/phases/phase-1-design?at={old_sha}"),
        ))
        .header("accept", "application/hal+json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(json["revision"], old_sha);
    assert_eq!(json["body"], "original-body-marker\n");
    assert_ne!(json["body"], "new-body-marker\n");
}

#[tokio::test]
async fn task_show_at_revision_returns_historical_body_and_revision() {
    let (_dir, addr, client, old_sha) = spawn_git_backed_server().await;

    let resp = client
        .get(url(
            addr,
            &format!("/projects/demo/tasks/bug-1?at={old_sha}"),
        ))
        .header("accept", "application/hal+json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(json["revision"], old_sha);
    assert_eq!(json["body"], "original-body-marker\n");
}
