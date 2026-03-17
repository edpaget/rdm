use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use rdm_core::hal::{HalLink, HalResource};
use rdm_core::model::PhaseStatus;

use crate::content_type::ResponseFormat;
use crate::error::AppError;
use crate::extract::{hal_response, require_hal_json};
use crate::state::AppState;

/// Summary data for a roadmap in a collection.
#[derive(Serialize)]
struct RoadmapSummary {
    slug: String,
    title: String,
    total_phases: usize,
    done_phases: usize,
}

/// Empty data for the roadmaps collection wrapper.
#[derive(Serialize)]
struct RoadmapsCollection {}

/// Detail data for a single roadmap.
#[derive(Serialize)]
struct RoadmapDetail {
    slug: String,
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    dependencies: Option<Vec<String>>,
}

/// `GET /projects/:project/roadmaps` — list roadmaps with progress summaries.
pub async fn list_roadmaps(
    format: ResponseFormat,
    State(state): State<AppState>,
    Path(project): Path<String>,
) -> Result<Response, Response> {
    require_hal_json(format)?;

    let repo = state.plan_repo();
    let roadmaps = repo
        .list_roadmaps(&project)
        .map_err(|e| AppError(e).into_response())?;

    let mut embedded = Vec::new();
    for roadmap_doc in &roadmaps {
        let slug = &roadmap_doc.frontmatter.roadmap;
        let phases = repo
            .list_phases(&project, slug)
            .map_err(|e| AppError(e).into_response())?;
        let done_count = phases
            .iter()
            .filter(|(_, doc)| doc.frontmatter.status == PhaseStatus::Done)
            .count();

        let summary = HalResource::new(
            RoadmapSummary {
                slug: slug.clone(),
                title: roadmap_doc.frontmatter.title.clone(),
                total_phases: phases.len(),
                done_phases: done_count,
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

/// `GET /projects/:project/roadmaps/:roadmap` — roadmap detail with embedded phases.
pub async fn get_roadmap(
    format: ResponseFormat,
    State(state): State<AppState>,
    Path((project, roadmap)): Path<(String, String)>,
) -> Result<Response, Response> {
    require_hal_json(format)?;

    let repo = state.plan_repo();
    let roadmap_doc = repo
        .load_roadmap(&project, &roadmap)
        .map_err(|e| AppError(e).into_response())?;
    let phases = repo
        .list_phases(&project, &roadmap)
        .map_err(|e| AppError(e).into_response())?;

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
            dependencies: roadmap_doc.frontmatter.dependencies,
        },
        self_href,
    )
    .with_link("project", HalLink::new(format!("/projects/{project}")))
    .with_embedded("phases", phase_embedded);

    Ok(hal_response(resource))
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
        let repo = PlanRepo::init(dir.path()).unwrap();
        repo.create_project("demo", "Demo Project").unwrap();
        repo.create_roadmap("demo", "alpha", "Alpha Roadmap", None)
            .unwrap();
        repo.create_phase("demo", "alpha", "first", "First Phase", Some(1), None)
            .unwrap();
        repo.create_phase("demo", "alpha", "second", "Second Phase", Some(2), None)
            .unwrap();
        repo.update_phase("demo", "alpha", "phase-1-first", PhaseStatus::Done, None)
            .unwrap();
        let state = AppState {
            plan_root: dir.path().to_path_buf(),
        };
        (dir, state)
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
        let body = to_bytes(response.into_body(), 8192).await.unwrap();
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
        let body = to_bytes(response.into_body(), 8192).await.unwrap();
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
    async fn list_roadmaps_406_for_html() {
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
        assert_eq!(response.status(), 406);
    }
}
