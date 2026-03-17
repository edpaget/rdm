use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use rdm_core::hal::{HalLink, HalResource};
use rdm_core::model::Phase;

use crate::content_type::ResponseFormat;
use crate::error::AppError;
use crate::extract::{hal_response, require_hal_json};
use crate::state::AppState;

/// Detail data for a single phase.
#[derive(Serialize)]
struct PhaseDetail {
    #[serde(flatten)]
    phase: Phase,
    stem: String,
    body: String,
}

/// `GET /projects/:project/roadmaps/:roadmap/phases/:phase` — phase detail
/// with sibling links.
pub async fn get_phase(
    format: ResponseFormat,
    State(state): State<AppState>,
    Path((project, roadmap, phase_id)): Path<(String, String, String)>,
) -> Result<Response, Response> {
    require_hal_json(format)?;

    let repo = state.plan_repo();
    let stem = repo
        .resolve_phase_stem(&project, &roadmap, &phase_id)
        .map_err(|e| AppError(e).into_response())?;
    let doc = repo
        .load_phase(&project, &roadmap, &stem)
        .map_err(|e| AppError(e).into_response())?;

    // Load all phases to compute sibling links.
    let all_phases = repo
        .list_phases(&project, &roadmap)
        .map_err(|e| AppError(e).into_response())?;

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

    // Find current index and add prev/next links.
    if let Some(idx) = all_phases.iter().position(|(s, _)| *s == stem) {
        if idx > 0 {
            let prev_stem = &all_phases[idx - 1].0;
            resource = resource.with_link(
                "prev",
                HalLink::new(format!(
                    "/projects/{project}/roadmaps/{roadmap}/phases/{prev_stem}"
                )),
            );
        }
        if idx + 1 < all_phases.len() {
            let next_stem = &all_phases[idx + 1].0;
            resource = resource.with_link(
                "next",
                HalLink::new(format!(
                    "/projects/{project}/roadmaps/{roadmap}/phases/{next_stem}"
                )),
            );
        }
    }

    Ok(hal_response(resource))
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;
    use axum::http::Request;
    use tempfile::TempDir;
    use tower::ServiceExt;

    use rdm_core::repo::PlanRepo;

    use crate::router::build_router;
    use crate::state::AppState;

    fn setup() -> (TempDir, AppState) {
        let dir = TempDir::new().unwrap();
        let repo = PlanRepo::init(dir.path()).unwrap();
        repo.create_project("demo", "Demo").unwrap();
        repo.create_roadmap("demo", "alpha", "Alpha", None).unwrap();
        repo.create_phase("demo", "alpha", "first", "First", Some(1), None)
            .unwrap();
        repo.create_phase("demo", "alpha", "second", "Second", Some(2), None)
            .unwrap();
        repo.create_phase("demo", "alpha", "third", "Third", Some(3), None)
            .unwrap();
        let state = AppState {
            plan_root: dir.path().to_path_buf(),
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
        let body = to_bytes(response.into_body(), 8192).await.unwrap();
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
        let body = to_bytes(response.into_body(), 8192).await.unwrap();
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
        let body = to_bytes(response.into_body(), 8192).await.unwrap();
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
        let body = to_bytes(response.into_body(), 8192).await.unwrap();
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
    async fn get_phase_406_for_html() {
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
        assert_eq!(response.status(), 406);
    }
}
