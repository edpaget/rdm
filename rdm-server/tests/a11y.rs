use std::net::SocketAddr;

use rdm_core::model::{PhaseStatus, Priority};
use rdm_core::repo::PlanRepo;
use rdm_core::store::FsStore;
use reqwest::Client;
use tempfile::TempDir;

/// Spawn a real TCP server with seeded data for a11y tests.
async fn spawn_server() -> (TempDir, SocketAddr, Client) {
    let dir = TempDir::new().unwrap();
    let mut repo = PlanRepo::init(FsStore::new(dir.path())).unwrap();

    repo.create_project("demo", "Demo Project").unwrap();
    repo.create_roadmap("demo", "api", "API Roadmap", None)
        .unwrap();
    repo.create_phase(
        "demo",
        "api",
        "design",
        "Design Phase",
        Some(1),
        Some("Details."),
    )
    .unwrap();
    repo.create_phase("demo", "api", "build", "Build Phase", Some(2), None)
        .unwrap();
    repo.update_phase(
        "demo",
        "api",
        "phase-1-design",
        Some(PhaseStatus::Done),
        None,
    )
    .unwrap();
    repo.create_task(
        "demo",
        "bug-1",
        "Fix Bug One",
        Priority::High,
        Some(vec!["bug".to_string()]),
        Some("Bug body."),
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

/// Fetch an HTML page and return its body text.
async fn fetch_html(client: &Client, addr: SocketAddr, path: &str) -> String {
    let resp = client
        .get(url(addr, path))
        .header("accept", "text/html")
        .send()
        .await
        .unwrap();
    resp.text().await.unwrap()
}

/// Assert common structural a11y properties on an HTML page.
fn assert_a11y_structure(html: &str, page_label: &str) {
    // DOCTYPE and lang attribute
    assert!(
        html.contains("<!DOCTYPE html>"),
        "{page_label}: missing <!DOCTYPE html>"
    );
    assert!(
        html.contains("<html lang=\"en\">"),
        "{page_label}: missing <html lang=\"en\">"
    );

    // Exactly one <main with id="main-content"
    let main_count = html.matches("<main").count();
    assert_eq!(
        main_count, 1,
        "{page_label}: expected exactly one <main>, found {main_count}"
    );
    assert!(
        html.contains("id=\"main-content\""),
        "{page_label}: <main> missing id=\"main-content\""
    );

    // Skip link near the top
    assert!(
        html.contains("<a href=\"#main-content\""),
        "{page_label}: missing skip link to #main-content"
    );
    // Skip link should appear before <main
    let skip_pos = html
        .find("<a href=\"#main-content\"")
        .expect("{page_label}: no skip link");
    let main_pos = html.find("<main").expect("{page_label}: no <main>");
    assert!(
        skip_pos < main_pos,
        "{page_label}: skip link should appear before <main>"
    );

    // Breadcrumb nav with <ol>
    assert!(
        html.contains("<nav aria-label=\"Breadcrumb\">"),
        "{page_label}: missing <nav aria-label=\"Breadcrumb\">"
    );
    assert!(
        html.contains("<ol"),
        "{page_label}: breadcrumb nav missing <ol>"
    );

    // Exactly one <h1
    let h1_count = html.matches("<h1").count();
    assert_eq!(
        h1_count, 1,
        "{page_label}: expected exactly one <h1>, found {h1_count}"
    );

    // All <th> have scope= (skip <thead> which also starts with "<th")
    for (i, _) in html.match_indices("<th") {
        // Find the end of this opening tag
        let tag_end = html[i..].find('>').map(|j| i + j).unwrap();
        let tag = &html[i..=tag_end];
        // Skip non-<th> tags like <thead>
        if tag.starts_with("<thead") {
            continue;
        }
        assert!(
            tag.contains("scope="),
            "{page_label}: <th> without scope= attribute: {tag}"
        );
    }

    // aria-current="page" present in breadcrumb
    assert!(
        html.contains("aria-current=\"page\""),
        "{page_label}: missing aria-current=\"page\" in breadcrumb"
    );
}

/// Pages to test for a11y structure.
const A11Y_PAGES: &[(&str, &str)] = &[
    ("/", "root"),
    ("/projects", "projects"),
    ("/projects/demo/roadmaps", "roadmaps"),
    ("/projects/demo/roadmaps/api", "roadmap detail"),
    (
        "/projects/demo/roadmaps/api/phases/phase-1-design",
        "phase detail",
    ),
    ("/projects/demo/tasks", "task list"),
    ("/projects/demo/tasks/bug-1", "task detail"),
];

#[tokio::test]
async fn all_pages_have_correct_a11y_structure() {
    let (_dir, addr, client) = spawn_server().await;

    for (path, label) in A11Y_PAGES {
        let html = fetch_html(&client, addr, path).await;
        assert_a11y_structure(&html, label);
    }
}

#[tokio::test]
async fn error_page_has_a11y_structure() {
    let (_dir, addr, client) = spawn_server().await;
    let html = fetch_html(&client, addr, "/projects/nonexistent/roadmaps").await;
    assert_a11y_structure(&html, "error page (404)");
}
