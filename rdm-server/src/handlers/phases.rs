use askama::Template;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use rdm_core::hal::{HalLink, HalResource};
use rdm_core::model::{Phase, PhaseStatus};
use rdm_core::store::Store;

use crate::content_type::ResponseFormat;
use crate::error::{error_response, json_rejection_response, validation_error};
use crate::extract::{hal_created_response, hal_response, see_other_response};
use crate::markdown::render_markdown;
use crate::state::AppState;
use crate::templates::{PhaseDetailPage, phase_status_class};

/// Detail data for a single phase.
#[derive(Serialize)]
struct PhaseDetail {
    #[serde(flatten)]
    phase: Phase,
    stem: String,
    body: String,
}

/// Empty data for the phases collection wrapper.
#[derive(Serialize)]
struct PhasesCollection {}

/// Query parameters for filtering the phase list.
#[derive(Debug, Deserialize, Default)]
pub struct PhaseFilters {
    /// Filter to phases carrying this tag.
    pub tag: Option<String>,
    /// Filter by phase status.
    pub status: Option<String>,
}

/// `GET /projects/:project/roadmaps/:roadmap/phases` — list phases for a
/// roadmap with optional tag/status filtering.
pub async fn list_phases(
    format: ResponseFormat,
    State(state): State<AppState>,
    Path((project, roadmap)): Path<(String, String)>,
    Query(filters): Query<PhaseFilters>,
) -> Result<Response, Response> {
    let status_filter: Option<PhaseStatus> = match &filters.status {
        Some(s) => Some(s.parse::<PhaseStatus>().map_err(|_| {
            validation_error(format!(
                "invalid status filter: '{s}' (expected not-started, in-progress, done, or blocked)"
            ))
        })?),
        None => None,
    };

    let store = state.store();
    // Verify the roadmap exists so we 404 cleanly instead of returning [].
    if !store.exists(&rdm_core::paths::roadmap_path(&project, &roadmap)) {
        return Err(error_response(
            rdm_core::error::Error::RoadmapNotFound(roadmap.clone()),
            format,
        ));
    }
    let phases = rdm_core::ops::phase::list_phases(&store, &project, &roadmap)
        .map_err(|e| error_response(e, format))?;

    let filtered: Vec<_> = phases
        .into_iter()
        .filter(|(_, doc)| {
            if let Some(ref sf) = status_filter
                && doc.frontmatter.status != *sf
            {
                return false;
            }
            if let Some(ref tag) = filters.tag {
                let has_tag = doc
                    .frontmatter
                    .tags
                    .as_ref()
                    .is_some_and(|tags| tags.contains(tag));
                if !has_tag {
                    return false;
                }
            }
            true
        })
        .collect();

    let self_href = format!("/projects/{project}/roadmaps/{roadmap}/phases");
    let mut embedded = Vec::new();
    for (stem, doc) in &filtered {
        let phase_resource = HalResource::new(
            PhaseDetail {
                phase: doc.frontmatter.clone(),
                stem: stem.clone(),
                body: doc.body.clone(),
            },
            format!("/projects/{project}/roadmaps/{roadmap}/phases/{stem}"),
        )
        .with_link(
            "roadmap",
            HalLink::new(format!("/projects/{project}/roadmaps/{roadmap}")),
        );
        embedded.push(serde_json::to_value(&phase_resource).unwrap());
    }

    match format {
        ResponseFormat::HalJson => {
            let resource = HalResource::new(PhasesCollection {}, self_href)
                .with_link(
                    "roadmap",
                    HalLink::new(format!("/projects/{project}/roadmaps/{roadmap}")),
                )
                .with_embedded("phases", embedded);
            Ok(hal_response(resource))
        }
        ResponseFormat::Html => {
            // No dedicated HTML phase-list view: redirect to the roadmap
            // detail page, which renders the phase table with the same
            // ?tag=<tag> filter applied.
            let mut redirect = format!("/projects/{project}/roadmaps/{roadmap}");
            if let Some(ref tag) = filters.tag {
                redirect.push_str(&format!("?tag={}", crate::templates::encode_tag_value(tag)));
            }
            Ok(see_other_response(&redirect))
        }
    }
}

/// `GET /projects/:project/roadmaps/:roadmap/phases/:phase` — phase detail
/// with sibling links.
pub async fn get_phase(
    format: ResponseFormat,
    State(state): State<AppState>,
    Path((project, roadmap, phase_id)): Path<(String, String, String)>,
) -> Result<Response, Response> {
    let store = state.store();
    let stem = rdm_core::ops::phase::resolve_phase_stem(&store, &project, &roadmap, &phase_id)
        .map_err(|e| error_response(e, format))?;
    let doc = rdm_core::io::load_phase(&store, &project, &roadmap, &stem)
        .map_err(|e| error_response(e, format))?;

    // Load all phases to compute sibling links.
    let all_phases = rdm_core::ops::phase::list_phases(&store, &project, &roadmap)
        .map_err(|e| error_response(e, format))?;

    let idx = all_phases.iter().position(|(s, _)| *s == stem);
    let prev_href = idx.filter(|&i| i > 0).map(|i| {
        let prev_stem = &all_phases[i - 1].0;
        format!("/projects/{project}/roadmaps/{roadmap}/phases/{prev_stem}")
    });
    let next_href = idx.filter(|&i| i + 1 < all_phases.len()).map(|i| {
        let next_stem = &all_phases[i + 1].0;
        format!("/projects/{project}/roadmaps/{roadmap}/phases/{next_stem}")
    });

    match format {
        ResponseFormat::HalJson => {
            let self_href = format!("/projects/{project}/roadmaps/{roadmap}/phases/{stem}");
            let mut resource = HalResource::new(
                PhaseDetail {
                    phase: doc.frontmatter,
                    stem: stem.clone(),
                    body: doc.body,
                },
                self_href,
            )
            .with_link(
                "roadmap",
                HalLink::new(format!("/projects/{project}/roadmaps/{roadmap}")),
            );

            if let Some(ref prev) = prev_href {
                resource = resource.with_link("prev", HalLink::new(prev.clone()));
            }
            if let Some(ref next) = next_href {
                resource = resource.with_link("next", HalLink::new(next.clone()));
            }

            Ok(hal_response(resource))
        }
        ResponseFormat::Html => {
            let page = PhaseDetailPage {
                project,
                roadmap,
                stem,
                phase_number: doc.frontmatter.phase,
                title: doc.frontmatter.title,
                status: doc.frontmatter.status.to_string(),
                status_class: phase_status_class(&doc.frontmatter.status).to_string(),
                completed: doc.frontmatter.completed.map(|d| d.to_string()),
                body_html: render_markdown(&doc.body),
                prev_href,
                next_href,
            };
            Ok((
                [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
                page.render().expect("template rendering cannot fail"),
            )
                .into_response())
        }
    }
}

/// Request body for `POST /projects/:project/roadmaps/:roadmap/phases`.
#[derive(Deserialize)]
pub struct CreatePhaseRequest {
    slug: String,
    title: String,
    number: Option<u32>,
    body: Option<String>,
    tags: Option<Vec<String>>,
}

/// `POST /projects/:project/roadmaps/:roadmap/phases` — create a new phase.
pub async fn create_phase(
    format: ResponseFormat,
    State(state): State<AppState>,
    Path((project, roadmap)): Path<(String, String)>,
    payload: Result<axum::Json<CreatePhaseRequest>, JsonRejection>,
) -> Result<Response, Response> {
    let axum::Json(req) = payload.map_err(json_rejection_response)?;
    let mut store = state.store();
    let doc = rdm_core::ops::phase::create_phase(
        &mut store,
        &project,
        &roadmap,
        &req.slug,
        &req.title,
        req.number,
        req.body.as_deref(),
        req.tags.clone(),
    )
    .map_err(|e| error_response(e, format))?;
    rdm_core::ops::index::generate_index(&mut store).map_err(|e| error_response(e, format))?;

    let stem = format!("phase-{}-{}", doc.frontmatter.phase, req.slug);
    let location = format!("/projects/{project}/roadmaps/{roadmap}/phases/{stem}");
    match format {
        ResponseFormat::HalJson => {
            let resource = HalResource::new(
                PhaseDetail {
                    phase: doc.frontmatter,
                    stem: stem.clone(),
                    body: doc.body,
                },
                &location,
            )
            .with_link(
                "roadmap",
                HalLink::new(format!("/projects/{project}/roadmaps/{roadmap}")),
            );
            Ok(hal_created_response(resource, &location))
        }
        ResponseFormat::Html => Ok(see_other_response(&location)),
    }
}

/// Request body for `PATCH /projects/:project/roadmaps/:roadmap/phases/:phase`.
#[derive(Deserialize)]
pub struct UpdatePhaseRequest {
    status: Option<String>,
    body: Option<String>,
    tags: Option<Vec<String>>,
    clear_tags: Option<bool>,
}

/// `PATCH /projects/:project/roadmaps/:roadmap/phases/:phase` — update a phase.
pub async fn update_phase(
    format: ResponseFormat,
    State(state): State<AppState>,
    Path((project, roadmap, phase_id)): Path<(String, String, String)>,
    payload: Result<axum::Json<UpdatePhaseRequest>, JsonRejection>,
) -> Result<Response, Response> {
    let axum::Json(req) = payload.map_err(json_rejection_response)?;
    let status: Option<PhaseStatus> = req
        .status
        .map(
            #[allow(clippy::result_large_err)]
            |s| {
                s.parse().map_err(|_| {
                    validation_error(format!(
                        "invalid status: '{s}' (expected not-started, in-progress, done, or blocked)",
                    ))
                })
            },
        )
        .transpose()?;

    if req.clear_tags.unwrap_or(false) && req.tags.is_some() {
        return Err(validation_error(
            "cannot set both 'tags' and 'clear_tags'".to_string(),
        ));
    }
    let tags = if req.clear_tags.unwrap_or(false) {
        Some(Vec::new())
    } else {
        req.tags.clone()
    };

    let mut store = state.store();
    let stem = rdm_core::ops::phase::resolve_phase_stem(&store, &project, &roadmap, &phase_id)
        .map_err(|e| error_response(e, format))?;
    let doc = rdm_core::ops::phase::update_phase(
        &mut store,
        &project,
        &roadmap,
        &stem,
        status,
        tags,
        req.body.as_deref(),
        None,
    )
    .map_err(|e| error_response(e, format))?;
    rdm_core::ops::index::generate_index(&mut store).map_err(|e| error_response(e, format))?;

    let self_href = format!("/projects/{project}/roadmaps/{roadmap}/phases/{stem}");
    match format {
        ResponseFormat::HalJson => {
            let resource = HalResource::new(
                PhaseDetail {
                    phase: doc.frontmatter,
                    stem: stem.clone(),
                    body: doc.body,
                },
                &self_href,
            )
            .with_link(
                "roadmap",
                HalLink::new(format!("/projects/{project}/roadmaps/{roadmap}")),
            );
            Ok(hal_response(resource))
        }
        ResponseFormat::Html => Ok(see_other_response(&self_href)),
    }
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;
    use axum::http::Request;
    use tempfile::TempDir;
    use tower::ServiceExt;

    use crate::router::build_router;
    use crate::state::AppState;

    fn setup() -> (TempDir, AppState) {
        let dir = TempDir::new().unwrap();
        let mut store = rdm_store_fs::FsStore::new(dir.path());
        rdm_core::ops::init::init(&mut store).unwrap();
        rdm_core::ops::project::create_project(&mut store, "demo", "Demo").unwrap();
        rdm_core::ops::roadmap::create_roadmap(
            &mut store, "demo", "alpha", "Alpha", None, None, None,
        )
        .unwrap();
        rdm_core::ops::phase::create_phase(
            &mut store,
            "demo",
            "alpha",
            "first",
            "First",
            Some(1),
            None,
            None,
        )
        .unwrap();
        rdm_core::ops::phase::create_phase(
            &mut store,
            "demo",
            "alpha",
            "second",
            "Second",
            Some(2),
            Some("## Details\n\nSome **bold** text.\n"),
            None,
        )
        .unwrap();
        rdm_core::ops::phase::create_phase(
            &mut store,
            "demo",
            "alpha",
            "third",
            "Third",
            Some(3),
            None,
            None,
        )
        .unwrap();
        let state = AppState {
            plan_root: dir.path().to_path_buf(),
            quick_filters: Vec::new(),
        };
        (dir, state)
    }

    #[tokio::test]
    async fn get_phase_by_stem() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/roadmaps/alpha/phases/phase-2-second")
                    .header("accept", "application/hal+json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 16384).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["phase"], 2);
        assert_eq!(json["title"], "Second");
        assert_eq!(json["stem"], "phase-2-second");
        // Should have prev and next
        assert!(json["_links"]["prev"]["href"].as_str().is_some());
        assert!(json["_links"]["next"]["href"].as_str().is_some());
        assert_eq!(
            json["_links"]["roadmap"]["href"],
            "/projects/demo/roadmaps/alpha"
        );
    }

    #[tokio::test]
    async fn get_phase_by_number() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/roadmaps/alpha/phases/2")
                    .header("accept", "application/hal+json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 16384).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["phase"], 2);
    }

    #[tokio::test]
    async fn get_first_phase_no_prev_link() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/roadmaps/alpha/phases/1")
                    .header("accept", "application/hal+json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 16384).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["_links"]["prev"].is_null());
        assert!(json["_links"]["next"]["href"].as_str().is_some());
    }

    #[tokio::test]
    async fn get_last_phase_no_next_link() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/roadmaps/alpha/phases/3")
                    .header("accept", "application/hal+json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 16384).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["_links"]["prev"]["href"].as_str().is_some());
        assert!(json["_links"]["next"].is_null());
    }

    #[tokio::test]
    async fn get_phase_not_found() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/roadmaps/alpha/phases/99")
                    .header("accept", "application/hal+json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 404);
    }

    #[tokio::test]
    async fn get_phase_returns_html() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/roadmaps/alpha/phases/2")
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
        assert!(html.contains("Phase 2: Second"));
        assert!(html.contains("<strong>bold</strong>"));
        assert!(html.contains("Previous phase"));
        assert!(html.contains("Next phase"));
        assert!(html.contains("#main-content"));
        assert!(html.contains("aria-current=\"page\""));
    }

    #[tokio::test]
    async fn get_phase_html_first_no_prev() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/roadmaps/alpha/phases/1")
                    .header("accept", "text/html")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 16384).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(!html.contains("Previous phase"));
        assert!(html.contains("Next phase"));
    }

    #[tokio::test]
    async fn get_phase_404_returns_html_error() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/roadmaps/alpha/phases/99")
                    .header("accept", "text/html")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 404);
        let body = to_bytes(response.into_body(), 16384).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("Not Found"));
    }

    fn post_json(uri: &str, body: &str) -> Request<axum::body::Body> {
        Request::post(uri)
            .header("accept", "application/hal+json")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap()
    }

    fn patch_json(uri: &str, body: &str) -> Request<axum::body::Body> {
        Request::patch(uri)
            .header("accept", "application/hal+json")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap()
    }

    #[tokio::test]
    async fn create_phase_returns_201() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(post_json(
                "/projects/demo/roadmaps/alpha/phases",
                r#"{"slug":"fourth","title":"Fourth Phase","number":4}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 201);
        assert_eq!(
            response.headers().get("location").unwrap(),
            "/projects/demo/roadmaps/alpha/phases/phase-4-fourth"
        );
        let body = to_bytes(response.into_body(), 16384).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["phase"], 4);
        assert_eq!(json["title"], "Fourth Phase");
        assert_eq!(json["stem"], "phase-4-fourth");
    }

    #[tokio::test]
    async fn create_phase_auto_number() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(post_json(
                "/projects/demo/roadmaps/alpha/phases",
                r#"{"slug":"auto","title":"Auto Phase"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 201);
        // Should auto-assign number 4 (after 1, 2, 3)
        assert_eq!(
            response.headers().get("location").unwrap(),
            "/projects/demo/roadmaps/alpha/phases/phase-4-auto"
        );
    }

    #[tokio::test]
    async fn create_phase_missing_roadmap_returns_404() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(post_json(
                "/projects/demo/roadmaps/nonexistent/phases",
                r#"{"slug":"x","title":"X"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 404);
    }

    #[tokio::test]
    async fn create_phase_duplicate_returns_409() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(post_json(
                "/projects/demo/roadmaps/alpha/phases",
                r#"{"slug":"first","title":"First Again","number":1}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 409);
    }

    #[tokio::test]
    async fn create_phase_html_returns_303() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::post("/projects/demo/roadmaps/alpha/phases")
                    .header("accept", "text/html")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(
                        r#"{"slug":"fourth","title":"Fourth","number":4}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 303);
        assert!(
            response
                .headers()
                .get("location")
                .unwrap()
                .to_str()
                .unwrap()
                .contains("phase-4-fourth")
        );
    }

    #[tokio::test]
    async fn update_phase_returns_200() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(patch_json(
                "/projects/demo/roadmaps/alpha/phases/phase-2-second",
                r#"{"status":"done"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 16384).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "done");
        assert!(json["completed"].as_str().is_some());
    }

    #[tokio::test]
    async fn update_phase_by_number() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(patch_json(
                "/projects/demo/roadmaps/alpha/phases/2",
                r#"{"status":"in-progress"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 16384).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "in-progress");
    }

    #[tokio::test]
    async fn update_phase_not_found_returns_404() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(patch_json(
                "/projects/demo/roadmaps/alpha/phases/99",
                r#"{"status":"done"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 404);
    }

    #[tokio::test]
    async fn update_phase_invalid_status_returns_422() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(patch_json(
                "/projects/demo/roadmaps/alpha/phases/1",
                r#"{"status":"bogus"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 422);
    }

    #[tokio::test]
    async fn update_phase_html_returns_303() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::patch("/projects/demo/roadmaps/alpha/phases/1")
                    .header("accept", "text/html")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(r#"{"status":"done"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 303);
        assert!(
            response
                .headers()
                .get("location")
                .unwrap()
                .to_str()
                .unwrap()
                .contains("phase-1-first")
        );
    }

    fn setup_with_phase_tags() -> (TempDir, AppState) {
        let dir = TempDir::new().unwrap();
        let mut store = rdm_store_fs::FsStore::new(dir.path());
        rdm_core::ops::init::init(&mut store).unwrap();
        rdm_core::ops::project::create_project(&mut store, "demo", "Demo").unwrap();
        rdm_core::ops::roadmap::create_roadmap(
            &mut store, "demo", "alpha", "Alpha", None, None, None,
        )
        .unwrap();
        rdm_core::ops::phase::create_phase(
            &mut store,
            "demo",
            "alpha",
            "tagged",
            "Tagged",
            Some(1),
            None,
            Some(vec!["bug".to_string(), "ui".to_string()]),
        )
        .unwrap();
        rdm_core::ops::phase::create_phase(
            &mut store,
            "demo",
            "alpha",
            "untagged",
            "Untagged",
            Some(2),
            None,
            None,
        )
        .unwrap();
        let state = AppState {
            plan_root: dir.path().to_path_buf(),
            quick_filters: Vec::new(),
        };
        (dir, state)
    }

    #[tokio::test]
    async fn list_phases_filter_by_tag() {
        let (_dir, state) = setup_with_phase_tags();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/roadmaps/alpha/phases?tag=bug")
                    .header("accept", "application/hal+json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let phases = json["_embedded"]["phases"].as_array().unwrap();
        assert_eq!(phases.len(), 1);
        assert_eq!(phases[0]["stem"], "phase-1-tagged");
    }

    #[tokio::test]
    async fn list_phases_no_filter_returns_all() {
        let (_dir, state) = setup_with_phase_tags();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/roadmaps/alpha/phases")
                    .header("accept", "application/hal+json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let phases = json["_embedded"]["phases"].as_array().unwrap();
        assert_eq!(phases.len(), 2);
    }

    #[tokio::test]
    async fn list_phases_unknown_roadmap_returns_404() {
        let (_dir, state) = setup_with_phase_tags();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/roadmaps/nope/phases?tag=bug")
                    .header("accept", "application/hal+json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 404);
    }

    #[tokio::test]
    async fn list_phases_html_redirects_to_roadmap() {
        let (_dir, state) = setup_with_phase_tags();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/roadmaps/alpha/phases?tag=bug")
                    .header("accept", "text/html")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 303);
        let location = response
            .headers()
            .get("location")
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(location, "/projects/demo/roadmaps/alpha?tag=bug");
    }

    #[tokio::test]
    async fn create_phase_persists_tags() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(post_json(
                "/projects/demo/roadmaps/alpha/phases",
                r#"{"slug":"new","title":"New","number":4,"tags":["foo","bar"]}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 201);
        let body = to_bytes(response.into_body(), 16384).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let tags = json["tags"].as_array().unwrap();
        assert_eq!(tags.len(), 2);
    }

    #[tokio::test]
    async fn update_phase_replaces_tags() {
        let (_dir, state) = setup_with_phase_tags();
        let app = build_router(state);
        let response = app
            .oneshot(patch_json(
                "/projects/demo/roadmaps/alpha/phases/1",
                r#"{"tags":["only"]}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 16384).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let tags = json["tags"].as_array().unwrap();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0], "only");
    }

    #[tokio::test]
    async fn update_phase_clear_tags() {
        let (_dir, state) = setup_with_phase_tags();
        let app = build_router(state);
        let response = app
            .oneshot(patch_json(
                "/projects/demo/roadmaps/alpha/phases/1",
                r#"{"clear_tags":true}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 16384).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.get("tags").is_none() || json["tags"].is_null());
    }

    #[tokio::test]
    async fn update_phase_conflicting_tag_fields_returns_422() {
        let (_dir, state) = setup_with_phase_tags();
        let app = build_router(state);
        let response = app
            .oneshot(patch_json(
                "/projects/demo/roadmaps/alpha/phases/1",
                r#"{"tags":["x"],"clear_tags":true}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 422);
    }
}
