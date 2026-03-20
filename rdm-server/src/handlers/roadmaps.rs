use std::time::SystemTime;

use askama::Template;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use rdm_core::document::Document;
use rdm_core::hal::{HalLink, HalResource};
use rdm_core::model::{Phase, PhaseStatus};
use rdm_core::repo::PlanRepo;
use rdm_store_fs::FsStore;

use axum::extract::Query;

use crate::content_type::ResponseFormat;
use crate::error::{error_response, json_rejection_response};
use crate::extract::{hal_created_response, hal_response, see_other_response};
use crate::markdown::render_markdown;
use crate::state::AppState;
use crate::templates::{
    PhaseRow, RoadmapDetailPage, RoadmapSummaryView, RoadmapsPage, computed_roadmap_status,
    phase_status_class,
};

/// Query parameters for filtering the roadmap list.
#[derive(Debug, Deserialize, Default)]
pub struct RoadmapFilters {
    /// When true, include completed roadmaps (all phases done) in the list.
    pub show_completed: Option<bool>,
}

/// Format a `SystemTime` as a `YYYY-MM-DD` date string.
fn format_system_time(t: SystemTime) -> String {
    let dt: chrono::DateTime<chrono::Utc> = t.into();
    dt.format("%Y-%m-%d").to_string()
}

/// Compute the most recent modification date across the roadmap and phase files.
fn last_changed_date(
    repo: &PlanRepo<FsStore>,
    project: &str,
    roadmap: &str,
    phases: &[(String, Document<Phase>)],
) -> Option<String> {
    let mut latest: Option<SystemTime> = None;

    // Check roadmap.md itself
    let root = repo.store().root();
    if let Ok(meta) = std::fs::metadata(root.join(repo.roadmap_path(project, roadmap).as_str()))
        && let Ok(modified) = meta.modified()
    {
        latest = Some(modified);
    }

    // Check each phase file
    for (stem, _) in phases {
        if let Ok(meta) =
            std::fs::metadata(root.join(repo.phase_path(project, roadmap, stem).as_str()))
            && let Ok(modified) = meta.modified()
        {
            latest = Some(match latest {
                Some(prev) if prev >= modified => prev,
                _ => modified,
            });
        }
    }

    latest.map(format_system_time)
}

/// Summary data for a roadmap in a collection.
#[derive(Serialize)]
struct RoadmapSummary {
    slug: String,
    title: String,
    total_phases: usize,
    done_phases: usize,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_changed: Option<String>,
}

/// Empty data for the roadmaps collection wrapper.
#[derive(Serialize)]
struct RoadmapsCollection {}

/// Detail data for a single roadmap.
#[derive(Serialize)]
struct RoadmapDetail {
    slug: String,
    title: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_changed: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dependencies: Option<Vec<String>>,
}

/// `GET /projects/:project/roadmaps` — list roadmaps with progress summaries.
pub async fn list_roadmaps(
    format: ResponseFormat,
    State(state): State<AppState>,
    Path(project): Path<String>,
    Query(filters): Query<RoadmapFilters>,
) -> Result<Response, Response> {
    let repo = state.plan_repo();
    let roadmaps = repo
        .list_roadmaps(&project)
        .map_err(|e| error_response(e, format))?;

    let mut summaries = Vec::new();
    for roadmap_doc in &roadmaps {
        let slug = &roadmap_doc.frontmatter.roadmap;
        let phases = repo
            .list_phases(&project, slug)
            .map_err(|e| error_response(e, format))?;
        let done_count = phases
            .iter()
            .filter(|(_, doc)| doc.frontmatter.status == PhaseStatus::Done)
            .count();

        let phase_statuses: Vec<PhaseStatus> = phases
            .iter()
            .map(|(_, doc)| doc.frontmatter.status)
            .collect();
        let (status_text, status_cls) = computed_roadmap_status(&phase_statuses);

        let last_changed = last_changed_date(&repo, &project, slug, &phases);

        summaries.push((
            slug.clone(),
            roadmap_doc.frontmatter.title.clone(),
            phases.len(),
            done_count,
            status_text.to_string(),
            status_cls.to_string(),
            last_changed,
        ));
    }

    let show_completed = filters.show_completed.unwrap_or(false);
    if !show_completed {
        summaries.retain(|(_slug, _title, _total, _done, status, _cls, _changed)| status != "done");
    }

    match format {
        ResponseFormat::HalJson => {
            let mut embedded = Vec::new();
            for (slug, title, total, done, status, _status_cls, last_changed) in &summaries {
                let summary = HalResource::new(
                    RoadmapSummary {
                        slug: slug.clone(),
                        title: title.clone(),
                        total_phases: *total,
                        done_phases: *done,
                        status: status.clone(),
                        last_changed: last_changed.clone(),
                    },
                    format!("/projects/{project}/roadmaps/{slug}"),
                )
                .with_link("project", HalLink::new(format!("/projects/{project}")));
                embedded.push(serde_json::to_value(&summary).unwrap());
            }

            let self_href = format!("/projects/{project}/roadmaps");
            let resource = HalResource::new(RoadmapsCollection {}, self_href)
                .with_link("project", HalLink::new(format!("/projects/{project}")))
                .with_embedded("roadmaps", embedded);

            Ok(hal_response(resource))
        }
        ResponseFormat::Html => {
            let views: Vec<RoadmapSummaryView> = summaries
                .into_iter()
                .map(
                    |(slug, title, total, done, status, status_class, last_changed)| {
                        RoadmapSummaryView {
                            slug,
                            title,
                            total_phases: total,
                            done_phases: done,
                            status,
                            status_class,
                            last_changed,
                        }
                    },
                )
                .collect();
            let page = RoadmapsPage {
                project,
                roadmaps: views,
                show_completed,
            };
            Ok((
                [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
                page.render().expect("template rendering cannot fail"),
            )
                .into_response())
        }
    }
}

/// `GET /projects/:project/roadmaps/:roadmap` — roadmap detail with embedded phases.
pub async fn get_roadmap(
    format: ResponseFormat,
    State(state): State<AppState>,
    Path((project, roadmap)): Path<(String, String)>,
) -> Result<Response, Response> {
    let repo = state.plan_repo();
    let roadmap_doc = repo
        .load_roadmap(&project, &roadmap)
        .map_err(|e| error_response(e, format))?;
    let phases = repo
        .list_phases(&project, &roadmap)
        .map_err(|e| error_response(e, format))?;

    let phase_statuses: Vec<PhaseStatus> = phases
        .iter()
        .map(|(_, doc)| doc.frontmatter.status)
        .collect();
    let (status_text, status_cls) = computed_roadmap_status(&phase_statuses);
    let last_changed = last_changed_date(&repo, &project, &roadmap, &phases);

    match format {
        ResponseFormat::HalJson => {
            let mut phase_embedded = Vec::new();
            for (stem, phase_doc) in &phases {
                let phase_resource = HalResource::new(
                    &phase_doc.frontmatter,
                    format!("/projects/{project}/roadmaps/{roadmap}/phases/{stem}"),
                );
                phase_embedded.push(serde_json::to_value(&phase_resource).unwrap());
            }

            let self_href = format!("/projects/{project}/roadmaps/{roadmap}");
            let resource = HalResource::new(
                RoadmapDetail {
                    slug: roadmap_doc.frontmatter.roadmap,
                    title: roadmap_doc.frontmatter.title,
                    status: status_text.to_string(),
                    last_changed: last_changed.clone(),
                    dependencies: roadmap_doc.frontmatter.dependencies,
                },
                self_href,
            )
            .with_link("project", HalLink::new(format!("/projects/{project}")))
            .with_embedded("phases", phase_embedded);

            Ok(hal_response(resource))
        }
        ResponseFormat::Html => {
            let phase_rows: Vec<PhaseRow> = phases
                .iter()
                .map(|(stem, doc)| {
                    let status_cls = phase_status_class(&doc.frontmatter.status).to_string();
                    PhaseRow {
                        phase: doc.frontmatter.phase,
                        stem: stem.clone(),
                        title: doc.frontmatter.title.clone(),
                        status: doc.frontmatter.status.to_string(),
                        status_class: status_cls,
                    }
                })
                .collect();
            let page = RoadmapDetailPage {
                project,
                slug: roadmap_doc.frontmatter.roadmap,
                title: roadmap_doc.frontmatter.title,
                status: status_text.to_string(),
                status_class: status_cls.to_string(),
                last_changed,
                dependencies: roadmap_doc.frontmatter.dependencies,
                body_html: render_markdown(&roadmap_doc.body),
                phases: phase_rows,
            };
            Ok((
                [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
                page.render().expect("template rendering cannot fail"),
            )
                .into_response())
        }
    }
}

/// Request body for `POST /projects/:project/roadmaps`.
#[derive(Deserialize)]
pub struct CreateRoadmapRequest {
    slug: String,
    title: String,
    body: Option<String>,
}

/// `POST /projects/:project/roadmaps` — create a new roadmap.
pub async fn create_roadmap(
    format: ResponseFormat,
    State(state): State<AppState>,
    Path(project): Path<String>,
    payload: Result<axum::Json<CreateRoadmapRequest>, JsonRejection>,
) -> Result<Response, Response> {
    let axum::Json(req) = payload.map_err(json_rejection_response)?;
    let mut repo = state.plan_repo();
    let doc = repo
        .create_roadmap(&project, &req.slug, &req.title, req.body.as_deref())
        .map_err(|e| error_response(e, format))?;
    repo.generate_index()
        .map_err(|e| error_response(e, format))?;

    let location = format!("/projects/{project}/roadmaps/{}", doc.frontmatter.roadmap);
    match format {
        ResponseFormat::HalJson => {
            let resource = HalResource::new(
                RoadmapDetail {
                    slug: doc.frontmatter.roadmap.clone(),
                    title: doc.frontmatter.title.clone(),
                    status: "not-started".to_string(),
                    last_changed: None,
                    dependencies: doc.frontmatter.dependencies,
                },
                &location,
            )
            .with_link("project", HalLink::new(format!("/projects/{project}")));
            Ok(hal_created_response(resource, &location))
        }
        ResponseFormat::Html => Ok(see_other_response(&location)),
    }
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;
    use axum::http::Request;
    use tempfile::TempDir;
    use tower::ServiceExt;

    use rdm_core::model::PhaseStatus;
    use rdm_core::repo::PlanRepo;

    use crate::router::build_router;
    use crate::state::AppState;

    fn setup() -> (TempDir, AppState) {
        let dir = TempDir::new().unwrap();
        let mut repo = PlanRepo::init(rdm_store_fs::FsStore::new(dir.path())).unwrap();
        repo.create_project("demo", "Demo Project").unwrap();
        repo.create_roadmap("demo", "alpha", "Alpha Roadmap", None)
            .unwrap();
        repo.create_phase("demo", "alpha", "first", "First Phase", Some(1), None)
            .unwrap();
        repo.create_phase("demo", "alpha", "second", "Second Phase", Some(2), None)
            .unwrap();
        repo.update_phase(
            "demo",
            "alpha",
            "phase-1-first",
            Some(PhaseStatus::Done),
            None,
            None,
        )
        .unwrap();
        let state = AppState {
            plan_root: dir.path().to_path_buf(),
        };
        (dir, state)
    }

    /// Create a setup with an additional completed roadmap ("beta") for filter tests.
    fn setup_with_completed() -> (TempDir, AppState) {
        let (dir, state) = setup();
        let mut repo = PlanRepo::new(rdm_store_fs::FsStore::new(dir.path()));
        repo.create_roadmap("demo", "beta", "Beta Roadmap", None)
            .unwrap();
        repo.create_phase("demo", "beta", "only", "Only Phase", Some(1), None)
            .unwrap();
        repo.update_phase(
            "demo",
            "beta",
            "phase-1-only",
            Some(PhaseStatus::Done),
            None,
            None,
        )
        .unwrap();
        (dir, state)
    }

    #[tokio::test]
    async fn list_roadmaps_hides_completed_by_default_html() {
        let (_dir, state) = setup_with_completed();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/roadmaps")
                    .header("accept", "text/html")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("Alpha Roadmap"));
        assert!(!html.contains("Beta Roadmap"));
        assert!(html.contains("Show completed roadmaps"));
    }

    #[tokio::test]
    async fn list_roadmaps_shows_completed_when_requested_html() {
        let (_dir, state) = setup_with_completed();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/roadmaps?show_completed=true")
                    .header("accept", "text/html")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("Alpha Roadmap"));
        assert!(html.contains("Beta Roadmap"));
        assert!(html.contains("Hide completed roadmaps"));
    }

    #[tokio::test]
    async fn list_roadmaps_hides_completed_by_default_hal_json() {
        let (_dir, state) = setup_with_completed();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/roadmaps")
                    .header("accept", "application/hal+json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let roadmaps = json["_embedded"]["roadmaps"].as_array().unwrap();
        assert_eq!(roadmaps.len(), 1);
        assert_eq!(roadmaps[0]["slug"], "alpha");
    }

    #[tokio::test]
    async fn list_roadmaps_shows_completed_when_requested_hal_json() {
        let (_dir, state) = setup_with_completed();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/roadmaps?show_completed=true")
                    .header("accept", "application/hal+json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let roadmaps = json["_embedded"]["roadmaps"].as_array().unwrap();
        assert_eq!(roadmaps.len(), 2);
    }

    #[tokio::test]
    async fn list_roadmaps_returns_summaries() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/roadmaps")
                    .header("accept", "application/hal+json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 16384).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let roadmaps = json["_embedded"]["roadmaps"].as_array().unwrap();
        assert_eq!(roadmaps.len(), 1);
        assert_eq!(roadmaps[0]["slug"], "alpha");
        assert_eq!(roadmaps[0]["total_phases"], 2);
        assert_eq!(roadmaps[0]["done_phases"], 1);
    }

    #[tokio::test]
    async fn get_roadmap_returns_detail_with_phases() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/roadmaps/alpha")
                    .header("accept", "application/hal+json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 16384).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["slug"], "alpha");
        assert_eq!(json["title"], "Alpha Roadmap");
        assert_eq!(json["_links"]["project"]["href"], "/projects/demo");
        let phases = json["_embedded"]["phases"].as_array().unwrap();
        assert_eq!(phases.len(), 2);
        assert_eq!(phases[0]["phase"], 1);
    }

    #[tokio::test]
    async fn get_roadmap_not_found() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/roadmaps/nonexistent")
                    .header("accept", "application/hal+json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 404);
    }

    #[tokio::test]
    async fn list_roadmaps_project_not_found() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/nonexistent/roadmaps")
                    .header("accept", "application/hal+json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 404);
    }

    #[tokio::test]
    async fn list_roadmaps_returns_html() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/roadmaps")
                    .header("accept", "text/html")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 16384).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("Alpha Roadmap"));
        assert!(html.contains("1/2 phases done"));
    }

    #[tokio::test]
    async fn get_roadmap_returns_html() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/roadmaps/alpha")
                    .header("accept", "text/html")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 16384).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("Alpha Roadmap"));
        assert!(html.contains("First Phase"));
        assert!(html.contains("badge-done"));
    }

    fn post_json(uri: &str, body: &str) -> Request<axum::body::Body> {
        Request::post(uri)
            .header("accept", "application/hal+json")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap()
    }

    #[tokio::test]
    async fn create_roadmap_returns_201() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(post_json(
                "/projects/demo/roadmaps",
                r#"{"slug":"beta","title":"Beta Roadmap"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 201);
        assert_eq!(
            response.headers().get("location").unwrap(),
            "/projects/demo/roadmaps/beta"
        );
        let body = to_bytes(response.into_body(), 16384).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["slug"], "beta");
        assert_eq!(json["title"], "Beta Roadmap");
    }

    #[tokio::test]
    async fn create_roadmap_missing_project_returns_404() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(post_json(
                "/projects/nonexistent/roadmaps",
                r#"{"slug":"beta","title":"Beta"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 404);
    }

    #[tokio::test]
    async fn create_roadmap_duplicate_returns_409() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(post_json(
                "/projects/demo/roadmaps",
                r#"{"slug":"alpha","title":"Alpha Again"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 409);
    }

    #[tokio::test]
    async fn create_roadmap_html_returns_303() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::post("/projects/demo/roadmaps")
                    .header("accept", "text/html")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(r#"{"slug":"beta","title":"Beta"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 303);
        assert_eq!(
            response.headers().get("location").unwrap(),
            "/projects/demo/roadmaps/beta"
        );
    }

    #[test]
    fn computed_status_all_done() {
        use crate::templates::computed_roadmap_status;
        let statuses = vec![PhaseStatus::Done, PhaseStatus::Done];
        assert_eq!(computed_roadmap_status(&statuses), ("done", "done"));
    }

    #[test]
    fn computed_status_some_in_progress() {
        use crate::templates::computed_roadmap_status;
        let statuses = vec![PhaseStatus::Done, PhaseStatus::InProgress];
        assert_eq!(
            computed_roadmap_status(&statuses),
            ("in-progress", "in-progress")
        );
    }

    #[test]
    fn computed_status_none_started() {
        use crate::templates::computed_roadmap_status;
        let statuses = vec![PhaseStatus::NotStarted, PhaseStatus::NotStarted];
        assert_eq!(
            computed_roadmap_status(&statuses),
            ("not-started", "not-started")
        );
    }

    #[test]
    fn computed_status_empty_phases() {
        use crate::templates::computed_roadmap_status;
        assert_eq!(computed_roadmap_status(&[]), ("not-started", "not-started"));
    }

    #[tokio::test]
    async fn list_roadmaps_hal_json_includes_status() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/roadmaps")
                    .header("accept", "application/hal+json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let roadmaps = json["_embedded"]["roadmaps"].as_array().unwrap();
        assert_eq!(roadmaps[0]["status"], "in-progress");
        assert!(roadmaps[0]["last_changed"].is_string());
    }

    #[tokio::test]
    async fn list_roadmaps_html_includes_status_badge() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/roadmaps")
                    .header("accept", "text/html")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("badge-in-progress"));
        assert!(html.contains("Last Changed"));
    }

    #[tokio::test]
    async fn get_roadmap_hal_json_includes_status_and_last_changed() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/roadmaps/alpha")
                    .header("accept", "application/hal+json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "in-progress");
        assert!(json["last_changed"].is_string());
    }

    #[tokio::test]
    async fn get_roadmap_html_includes_status_badge_and_last_changed() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/roadmaps/alpha")
                    .header("accept", "text/html")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("badge-in-progress"));
        assert!(html.contains("Last changed:"));
    }
}
