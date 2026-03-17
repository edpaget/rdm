use axum::extract::State;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use rdm_core::hal::{HalLink, HalResource};

use crate::content_type::ResponseFormat;
use crate::error::AppError;
use crate::extract::{hal_response, require_hal_json};
use crate::state::AppState;

/// Empty data for the root resource.
#[derive(Serialize)]
struct RootData {}

/// `GET /` — API root with discovery links to all projects.
pub async fn index(
    format: ResponseFormat,
    State(state): State<AppState>,
) -> Result<Response, Response> {
    require_hal_json(format)?;

    let repo = state.plan_repo();
    let names = repo
        .list_projects()
        .map_err(|e| AppError(e).into_response())?;

    let mut resource =
        HalResource::new(RootData {}, "/").with_link("projects", HalLink::new("/projects"));

    for name in &names {
        resource = resource.with_link(name.as_str(), HalLink::new(format!("/projects/{name}")));
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
        repo.create_project("alpha", "Alpha").unwrap();
        repo.create_project("beta", "Beta").unwrap();
        let state = AppState {
            plan_root: dir.path().to_path_buf(),
        };
        (dir, state)
    }

    #[tokio::test]
    async fn root_returns_hal_with_links() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/")
                    .header("accept", "application/hal+json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/hal+json"
        );
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["_links"]["self"]["href"], "/");
        assert_eq!(json["_links"]["projects"]["href"], "/projects");
        assert_eq!(json["_links"]["alpha"]["href"], "/projects/alpha");
        assert_eq!(json["_links"]["beta"]["href"], "/projects/beta");
    }

    #[tokio::test]
    async fn root_returns_406_for_html() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/")
                    .header("accept", "text/html")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 406);
    }
}
