use std::net::SocketAddr;

use rdm_core::model::{PhaseStatus, Priority};
use rdm_store_fs::FsStore;
use reqwest::Client;
use tempfile::TempDir;

/// Spawn a real TCP server with seeded data. Returns TempDir (must outlive the test),
/// the bound address, and a reqwest client.
async fn spawn_server() -> (TempDir, SocketAddr, Client) {
    let dir = TempDir::new().unwrap();
    let mut store = FsStore::new(dir.path());
    rdm_core::ops::init::init(&mut store).unwrap();

    // Seed data
    rdm_core::ops::project::create_project(&mut store, "demo", "Demo Project").unwrap();
    rdm_core::ops::roadmap::create_roadmap(
        &mut store,
        "demo",
        "api",
        "API Roadmap",
        Some("API roadmap body."),
        None,
        None,
    )
    .unwrap();
    rdm_core::ops::phase::create_phase(
        &mut store,
        "demo",
        "api",
        "design",
        "Design Phase",
        Some(1),
        Some("Design details."),
        None,
    )
    .unwrap();
    rdm_core::ops::phase::create_phase(
        &mut store,
        "demo",
        "api",
        "build",
        "Build Phase",
        Some(2),
        None,
        None,
    )
    .unwrap();
    rdm_core::ops::phase::update_phase(
        &mut store,
        "demo",
        "api",
        "phase-1-design",
        Some(PhaseStatus::Done),
        None,
        None,
        None,
    )
    .unwrap();
    rdm_core::ops::task::create_task(
        &mut store,
        "demo",
        "bug-1",
        "Fix Bug One",
        Priority::High,
        Some(vec!["bug".to_string()]),
        Some("Bug details."),
    )
    .unwrap();
    rdm_core::ops::task::create_task(
        &mut store,
        "demo",
        "feature-1",
        "Add Feature One",
        Priority::Low,
        None,
        None,
    )
    .unwrap();

    let state = rdm_server::state::AppState {
        plan_root: dir.path().to_path_buf(),
    };
    let app = rdm_server::router::build_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = Client::new();
    (dir, addr, client)
}

fn url(addr: SocketAddr, path: &str) -> String {
    format!("http://{addr}{path}")
}

// ── Step 3: Read endpoint tests ───────────────────────────────────────────────

#[tokio::test]
async fn healthz_returns_200() {
    let (_dir, addr, client) = spawn_server().await;
    let resp = client.get(url(addr, "/healthz")).send().await.unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn root_hal_json_has_links() {
    let (_dir, addr, client) = spawn_server().await;
    let resp = client
        .get(url(addr, "/"))
        .header("accept", "application/hal+json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: serde_json::Value = resp.json().await.unwrap();
    assert!(json["_links"]["self"]["href"].is_string());
}

#[tokio::test]
async fn root_html_returns_doctype() {
    let (_dir, addr, client) = spawn_server().await;
    let resp = client
        .get(url(addr, "/"))
        .header("accept", "text/html")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("<!DOCTYPE html>"));
}

#[tokio::test]
async fn projects_hal_json() {
    let (_dir, addr, client) = spawn_server().await;
    let resp = client
        .get(url(addr, "/projects"))
        .header("accept", "application/hal+json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: serde_json::Value = resp.json().await.unwrap();
    let projects = json["_embedded"]["projects"].as_array().unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0]["name"], "demo");
}

#[tokio::test]
async fn projects_html() {
    let (_dir, addr, client) = spawn_server().await;
    let resp = client
        .get(url(addr, "/projects"))
        .header("accept", "text/html")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("<!DOCTYPE html>"));
    assert!(body.contains("Demo Project"));
}

#[tokio::test]
async fn roadmaps_hal_json() {
    let (_dir, addr, client) = spawn_server().await;
    let resp = client
        .get(url(addr, "/projects/demo/roadmaps"))
        .header("accept", "application/hal+json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: serde_json::Value = resp.json().await.unwrap();
    let roadmaps = json["_embedded"]["roadmaps"].as_array().unwrap();
    assert_eq!(roadmaps.len(), 1);
    assert_eq!(roadmaps[0]["slug"], "api");
}

#[tokio::test]
async fn roadmaps_html() {
    let (_dir, addr, client) = spawn_server().await;
    let resp = client
        .get(url(addr, "/projects/demo/roadmaps"))
        .header("accept", "text/html")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("<!DOCTYPE html>"));
    assert!(body.contains("API Roadmap"));
}

#[tokio::test]
async fn roadmap_detail_with_embedded_phases() {
    let (_dir, addr, client) = spawn_server().await;
    let resp = client
        .get(url(addr, "/projects/demo/roadmaps/api"))
        .header("accept", "application/hal+json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(json["slug"], "api");
    let phases = json["_embedded"]["phases"].as_array().unwrap();
    assert_eq!(phases.len(), 2);
}

#[tokio::test]
async fn phase_detail_hal_json() {
    let (_dir, addr, client) = spawn_server().await;
    let resp = client
        .get(url(
            addr,
            "/projects/demo/roadmaps/api/phases/phase-1-design",
        ))
        .header("accept", "application/hal+json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(json["phase"], 1);
    assert_eq!(json["title"], "Design Phase");
    assert_eq!(json["status"], "done");
}

#[tokio::test]
async fn phase_detail_html() {
    let (_dir, addr, client) = spawn_server().await;
    let resp = client
        .get(url(
            addr,
            "/projects/demo/roadmaps/api/phases/phase-1-design",
        ))
        .header("accept", "text/html")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("<!DOCTYPE html>"));
    assert!(body.contains("Design Phase"));
}

#[tokio::test]
async fn tasks_hal_json() {
    let (_dir, addr, client) = spawn_server().await;
    let resp = client
        .get(url(addr, "/projects/demo/tasks"))
        .header("accept", "application/hal+json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: serde_json::Value = resp.json().await.unwrap();
    let tasks = json["_embedded"]["tasks"].as_array().unwrap();
    assert_eq!(tasks.len(), 2);
}

#[tokio::test]
async fn tasks_html() {
    let (_dir, addr, client) = spawn_server().await;
    let resp = client
        .get(url(addr, "/projects/demo/tasks"))
        .header("accept", "text/html")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("<!DOCTYPE html>"));
    assert!(body.contains("Fix Bug One"));
}

#[tokio::test]
async fn tasks_filter_by_priority() {
    let (_dir, addr, client) = spawn_server().await;
    let resp = client
        .get(url(addr, "/projects/demo/tasks?priority=high"))
        .header("accept", "application/hal+json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: serde_json::Value = resp.json().await.unwrap();
    let tasks = json["_embedded"]["tasks"].as_array().unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0]["priority"], "high");
}

#[tokio::test]
async fn task_detail_hal_json() {
    let (_dir, addr, client) = spawn_server().await;
    let resp = client
        .get(url(addr, "/projects/demo/tasks/bug-1"))
        .header("accept", "application/hal+json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(json["slug"], "bug-1");
    assert_eq!(json["title"], "Fix Bug One");
}

#[tokio::test]
async fn task_detail_html() {
    let (_dir, addr, client) = spawn_server().await;
    let resp = client
        .get(url(addr, "/projects/demo/tasks/bug-1"))
        .header("accept", "text/html")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("<!DOCTYPE html>"));
    assert!(body.contains("Fix Bug One"));
}

#[tokio::test]
async fn content_negotiation_no_accept_defaults_to_html() {
    let (_dir, addr, client) = spawn_server().await;
    let resp = client.get(url(addr, "/")).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(ct.contains("text/html"));
}

#[tokio::test]
async fn content_negotiation_wildcard_defaults_to_html() {
    let (_dir, addr, client) = spawn_server().await;
    let resp = client
        .get(url(addr, "/"))
        .header("accept", "*/*")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(ct.contains("text/html"));
}

#[tokio::test]
async fn content_negotiation_explicit_hal_json() {
    let (_dir, addr, client) = spawn_server().await;
    let resp = client
        .get(url(addr, "/"))
        .header("accept", "application/hal+json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(ct.contains("application/hal+json"));
}

// ── Step 4: Write endpoint tests ──────────────────────────────────────────────

#[tokio::test]
async fn create_project_returns_201_and_exists_on_disk() {
    let (dir, addr, client) = spawn_server().await;
    let resp = client
        .post(url(addr, "/projects"))
        .header("accept", "application/hal+json")
        .json(&serde_json::json!({"name": "new-proj", "title": "New Project"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    assert!(resp.headers().get("location").is_some());

    // Verify on disk
    let store = FsStore::new(dir.path());
    let doc = rdm_core::io::load_project(&store, "new-proj").unwrap();
    assert_eq!(doc.frontmatter.title, "New Project");
}

#[tokio::test]
async fn create_roadmap_returns_201_and_exists_on_disk() {
    let (dir, addr, client) = spawn_server().await;
    let resp = client
        .post(url(addr, "/projects/demo/roadmaps"))
        .header("accept", "application/hal+json")
        .json(&serde_json::json!({"slug": "new-rm", "title": "New Roadmap"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    assert!(resp.headers().get("location").is_some());

    let store = FsStore::new(dir.path());
    let doc = rdm_core::io::load_roadmap(&store, "demo", "new-rm").unwrap();
    assert_eq!(doc.frontmatter.title, "New Roadmap");
}

#[tokio::test]
async fn create_phase_returns_201_and_exists_on_disk() {
    let (dir, addr, client) = spawn_server().await;
    let resp = client
        .post(url(addr, "/projects/demo/roadmaps/api/phases"))
        .header("accept", "application/hal+json")
        .json(&serde_json::json!({"slug": "test-ph", "title": "Test Phase", "number": 3}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    assert!(resp.headers().get("location").is_some());

    let store = FsStore::new(dir.path());
    let doc = rdm_core::io::load_phase(&store, "demo", "api", "phase-3-test-ph").unwrap();
    assert_eq!(doc.frontmatter.title, "Test Phase");
}

#[tokio::test]
async fn create_task_returns_201_and_exists_on_disk() {
    let (dir, addr, client) = spawn_server().await;
    let resp = client
        .post(url(addr, "/projects/demo/tasks"))
        .header("accept", "application/hal+json")
        .json(&serde_json::json!({"slug": "new-task", "title": "New Task"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let location = resp.headers().get("location").unwrap().to_str().unwrap();
    assert_eq!(location, "/projects/demo/tasks/new-task");

    let store = FsStore::new(dir.path());
    let doc = rdm_core::io::load_task(&store, "demo", "new-task").unwrap();
    assert_eq!(doc.frontmatter.title, "New Task");
}

#[tokio::test]
async fn create_task_duplicate_returns_409() {
    let (_dir, addr, client) = spawn_server().await;
    let resp = client
        .post(url(addr, "/projects/demo/tasks"))
        .header("accept", "application/hal+json")
        .json(&serde_json::json!({"slug": "bug-1", "title": "Duplicate"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 409);
    let json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(json["status"], 409);
}

#[tokio::test]
async fn update_task_status_via_patch() {
    let (dir, addr, client) = spawn_server().await;
    let resp = client
        .patch(url(addr, "/projects/demo/tasks/bug-1"))
        .header("accept", "application/hal+json")
        .json(&serde_json::json!({"status": "done"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(json["status"], "done");

    // Verify on disk
    let store = FsStore::new(dir.path());
    let doc = rdm_core::io::load_task(&store, "demo", "bug-1").unwrap();
    assert_eq!(doc.frontmatter.status.to_string(), "done");
}

#[tokio::test]
async fn update_phase_via_patch() {
    let (dir, addr, client) = spawn_server().await;
    let resp = client
        .patch(url(
            addr,
            "/projects/demo/roadmaps/api/phases/phase-2-build",
        ))
        .header("accept", "application/hal+json")
        .json(&serde_json::json!({"status": "in-progress"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(json["status"], "in-progress");

    let store = FsStore::new(dir.path());
    let doc = rdm_core::io::load_phase(&store, "demo", "api", "phase-2-build").unwrap();
    assert_eq!(doc.frontmatter.status, PhaseStatus::InProgress);
}

#[tokio::test]
async fn promote_task_returns_201_and_removes_task() {
    let (dir, addr, client) = spawn_server().await;
    let resp = client
        .post(url(addr, "/projects/demo/tasks/bug-1/promote"))
        .header("accept", "application/hal+json")
        .json(&serde_json::json!({"roadmap_slug": "bug-1-roadmap"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let location = resp.headers().get("location").unwrap().to_str().unwrap();
    assert_eq!(location, "/projects/demo/roadmaps/bug-1-roadmap");

    // Old task should be gone
    let store = FsStore::new(dir.path());
    assert!(rdm_core::io::load_task(&store, "demo", "bug-1").is_err());
    // New roadmap should exist
    assert!(rdm_core::io::load_roadmap(&store, "demo", "bug-1-roadmap").is_ok());
}

#[tokio::test]
async fn malformed_json_returns_422() {
    let (_dir, addr, client) = spawn_server().await;
    let resp = client
        .post(url(addr, "/projects/demo/tasks"))
        .header("accept", "application/hal+json")
        .header("content-type", "application/json")
        .body(r#"{"bad":true}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 422);
    let json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(json["status"], 422);
}

#[tokio::test]
async fn invalid_status_value_returns_422() {
    let (_dir, addr, client) = spawn_server().await;
    let resp = client
        .patch(url(addr, "/projects/demo/tasks/bug-1"))
        .header("accept", "application/hal+json")
        .json(&serde_json::json!({"status": "invalid-status"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 422);
}

#[tokio::test]
async fn post_with_html_accept_returns_303() {
    let (_dir, addr, _client) = spawn_server().await;
    // Use a no-redirect client to verify 303
    let client_no_redirect = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let resp = client_no_redirect
        .post(url(addr, "/projects/demo/tasks"))
        .header("accept", "text/html")
        .json(&serde_json::json!({"slug": "html-task", "title": "HTML Task"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 303);
    assert!(resp.headers().get("location").is_some());
}

// ── Roadmap priority tests ────────────────────────────────────────────────────

#[tokio::test]
async fn create_roadmap_with_priority() {
    let (dir, addr, client) = spawn_server().await;
    let resp = client
        .post(url(addr, "/projects/demo/roadmaps"))
        .header("accept", "application/hal+json")
        .json(&serde_json::json!({"slug": "urgent", "title": "Urgent", "priority": "high"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(json["priority"], "high");

    let store = FsStore::new(dir.path());
    let doc = rdm_core::io::load_roadmap(&store, "demo", "urgent").unwrap();
    assert_eq!(doc.frontmatter.priority, Some(Priority::High));
}

#[tokio::test]
async fn list_roadmaps_includes_priority() {
    let (_dir, addr, client) = spawn_server().await;
    // Create a roadmap with priority
    client
        .post(url(addr, "/projects/demo/roadmaps"))
        .header("accept", "application/hal+json")
        .json(&serde_json::json!({"slug": "urgent", "title": "Urgent", "priority": "critical"}))
        .send()
        .await
        .unwrap();

    let resp = client
        .get(url(addr, "/projects/demo/roadmaps"))
        .header("accept", "application/hal+json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: serde_json::Value = resp.json().await.unwrap();
    let roadmaps = json["_embedded"]["roadmaps"].as_array().unwrap();
    // Find the urgent roadmap
    let urgent = roadmaps.iter().find(|r| r["slug"] == "urgent").unwrap();
    assert_eq!(urgent["priority"], "critical");
    // api roadmap has no priority — field should be absent
    let api = roadmaps.iter().find(|r| r["slug"] == "api").unwrap();
    assert!(api.get("priority").is_none());
}

#[tokio::test]
async fn list_roadmaps_filter_by_priority() {
    let (_dir, addr, client) = spawn_server().await;
    client
        .post(url(addr, "/projects/demo/roadmaps"))
        .header("accept", "application/hal+json")
        .json(&serde_json::json!({"slug": "urgent", "title": "Urgent", "priority": "high"}))
        .send()
        .await
        .unwrap();

    let resp = client
        .get(url(addr, "/projects/demo/roadmaps?priority=high"))
        .header("accept", "application/hal+json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: serde_json::Value = resp.json().await.unwrap();
    let roadmaps = json["_embedded"]["roadmaps"].as_array().unwrap();
    assert_eq!(roadmaps.len(), 1);
    assert_eq!(roadmaps[0]["slug"], "urgent");
}

#[tokio::test]
async fn list_roadmaps_sort_by_priority() {
    let (_dir, addr, client) = spawn_server().await;
    client
        .post(url(addr, "/projects/demo/roadmaps"))
        .header("accept", "application/hal+json")
        .json(&serde_json::json!({"slug": "critical-rm", "title": "Critical", "priority": "critical"}))
        .send()
        .await
        .unwrap();

    let resp = client
        .get(url(
            addr,
            "/projects/demo/roadmaps?sort=priority&show_completed=true",
        ))
        .header("accept", "application/hal+json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: serde_json::Value = resp.json().await.unwrap();
    let roadmaps = json["_embedded"]["roadmaps"].as_array().unwrap();
    assert_eq!(roadmaps[0]["slug"], "critical-rm");
}

#[tokio::test]
async fn update_roadmap_priority_via_patch() {
    let (dir, addr, client) = spawn_server().await;
    let resp = client
        .patch(url(addr, "/projects/demo/roadmaps/api"))
        .header("accept", "application/hal+json")
        .json(&serde_json::json!({"priority": "critical"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(json["priority"], "critical");

    let store = FsStore::new(dir.path());
    let doc = rdm_core::io::load_roadmap(&store, "demo", "api").unwrap();
    assert_eq!(doc.frontmatter.priority, Some(Priority::Critical));
}

#[tokio::test]
async fn update_roadmap_clear_priority_via_patch() {
    let (_dir, addr, client) = spawn_server().await;
    // First set a priority
    client
        .patch(url(addr, "/projects/demo/roadmaps/api"))
        .header("accept", "application/hal+json")
        .json(&serde_json::json!({"priority": "high"}))
        .send()
        .await
        .unwrap();

    // Then clear it
    let resp = client
        .patch(url(addr, "/projects/demo/roadmaps/api"))
        .header("accept", "application/hal+json")
        .json(&serde_json::json!({"clear_priority": true}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: serde_json::Value = resp.json().await.unwrap();
    assert!(json.get("priority").is_none());
}

#[tokio::test]
async fn roadmap_detail_includes_priority() {
    let (_dir, addr, client) = spawn_server().await;
    // Set priority on existing roadmap
    client
        .patch(url(addr, "/projects/demo/roadmaps/api"))
        .header("accept", "application/hal+json")
        .json(&serde_json::json!({"priority": "medium"}))
        .send()
        .await
        .unwrap();

    let resp = client
        .get(url(addr, "/projects/demo/roadmaps/api"))
        .header("accept", "application/hal+json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(json["priority"], "medium");
}

// ── Roadmap priority error case tests ─────────────────────────────────────────

#[tokio::test]
async fn create_roadmap_invalid_priority_returns_422() {
    let (_dir, addr, client) = spawn_server().await;
    let resp = client
        .post(url(addr, "/projects/demo/roadmaps"))
        .header("accept", "application/hal+json")
        .json(&serde_json::json!({"slug": "bad", "title": "Bad", "priority": "bogus"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 422);
}

#[tokio::test]
async fn update_roadmap_invalid_priority_returns_422() {
    let (_dir, addr, client) = spawn_server().await;
    let resp = client
        .patch(url(addr, "/projects/demo/roadmaps/api"))
        .header("accept", "application/hal+json")
        .json(&serde_json::json!({"priority": "bogus"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 422);
}

#[tokio::test]
async fn update_roadmap_conflicting_priority_fields_returns_422() {
    let (_dir, addr, client) = spawn_server().await;
    let resp = client
        .patch(url(addr, "/projects/demo/roadmaps/api"))
        .header("accept", "application/hal+json")
        .json(&serde_json::json!({"priority": "high", "clear_priority": true}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 422);
}

#[tokio::test]
async fn list_roadmaps_invalid_priority_filter_returns_400() {
    let (_dir, addr, client) = spawn_server().await;
    let resp = client
        .get(url(addr, "/projects/demo/roadmaps?priority=bogus"))
        .header("accept", "application/hal+json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn list_roadmaps_invalid_sort_returns_400() {
    let (_dir, addr, client) = spawn_server().await;
    let resp = client
        .get(url(addr, "/projects/demo/roadmaps?sort=bogus"))
        .header("accept", "application/hal+json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

// ── Step 5: Error case tests ──────────────────────────────────────────────────

#[tokio::test]
async fn get_nonexistent_project_roadmaps_returns_404() {
    let (_dir, addr, client) = spawn_server().await;
    let resp = client
        .get(url(addr, "/projects/nonexistent/roadmaps"))
        .header("accept", "application/hal+json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
    let json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(json["status"], 404);
    assert!(json["title"].as_str().is_some());
    assert!(json["detail"].as_str().is_some());
}

#[tokio::test]
async fn get_nonexistent_roadmap_returns_404() {
    let (_dir, addr, client) = spawn_server().await;
    let resp = client
        .get(url(addr, "/projects/demo/roadmaps/nonexistent"))
        .header("accept", "application/hal+json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn get_nonexistent_task_returns_404() {
    let (_dir, addr, client) = spawn_server().await;
    let resp = client
        .get(url(addr, "/projects/demo/tasks/nonexistent"))
        .header("accept", "application/hal+json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn get_nonexistent_phase_returns_404() {
    let (_dir, addr, client) = spawn_server().await;
    let resp = client
        .get(url(addr, "/projects/demo/roadmaps/api/phases/99"))
        .header("accept", "application/hal+json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn hal_json_404_has_problem_json_content_type() {
    let (_dir, addr, client) = spawn_server().await;
    let resp = client
        .get(url(addr, "/projects/nonexistent/roadmaps"))
        .header("accept", "application/hal+json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(ct, "application/problem+json");
}

#[tokio::test]
async fn html_404_returns_styled_error_page() {
    let (_dir, addr, client) = spawn_server().await;
    let resp = client
        .get(url(addr, "/projects/nonexistent/roadmaps"))
        .header("accept", "text/html")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
    let body = resp.text().await.unwrap();
    assert!(body.contains("<!DOCTYPE html>"));
    assert!(body.contains("Not Found"));
}

#[tokio::test]
async fn roadmap_detail_html_renders_body() {
    let (_dir, addr, client) = spawn_server().await;
    let resp = client
        .get(url(addr, "/projects/demo/roadmaps/api"))
        .header("accept", "text/html")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("body-content"));
    assert!(body.contains("API roadmap body."));
}
