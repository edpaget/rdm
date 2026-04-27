use askama::Template;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use rdm_core::hal::{HalLink, HalResource};

use crate::content_type::ResponseFormat;
use crate::error::error_response;
use crate::extract::hal_response;
use crate::state::AppState;
use crate::templates::{IndexPage, ProjectView};

/// Empty data for the root resource.
#[derive(Serialize)]
struct RootData {}

/// `GET /` — API root with discovery links (HAL+JSON) or project listing (HTML).
pub async fn index(
    format: ResponseFormat,
    State(state): State<AppState>,
) -> Result<Response, Response> {
    let store = state.store();
    let names =
        rdm_core::ops::project::list_projects(&store).map_err(|e| error_response(e, format))?;

    match format {
        ResponseFormat::HalJson => {
            let mut resource =
                HalResource::new(RootData {}, "/").with_link("projects", HalLink::new("/projects"));

            for name in &names {
                resource =
                    resource.with_link(name.as_str(), HalLink::new(format!("/projects/{name}")));
            }

            Ok(hal_response(resource))
        }
        ResponseFormat::Html => {
            let mut projects = Vec::new();
            for name in &names {
                if let Ok(doc) = rdm_core::io::load_project(&store, name) {
                    projects.push(ProjectView {
                        name: doc.frontmatter.name,
                        title: doc.frontmatter.title,
                    });
                }
            }
            let page = IndexPage { projects };
            Ok((
                [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
                page.render().expect("template rendering cannot fail"),
            )
                .into_response())
        }
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
        rdm_core::ops::project::create_project(&mut store, "alpha", "Alpha").unwrap();
        rdm_core::ops::project::create_project(&mut store, "beta", "Beta").unwrap();
        let state = AppState {
            plan_root: dir.path().to_path_buf(),
            quick_filters: Vec::new(),
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
    async fn root_returns_html() {
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
        assert_eq!(response.status(), 200);
        assert!(
            response
                .headers()
                .get("content-type")
                .unwrap()
                .to_str()
                .unwrap()
                .contains("text/html")
        );
        let body = to_bytes(response.into_body(), 16384).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("#main-content"));
        assert!(html.contains("aria-label=\"Breadcrumb\""));
        assert!(html.contains("aria-current=\"page\""));
        assert!(html.contains("<main id=\"main-content\""));
        assert!(html.contains("<footer>"));
        assert!(html.contains("Alpha"));
        assert!(html.contains("Beta"));
    }

    #[tokio::test]
    async fn root_default_accept_returns_html() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(Request::get("/").body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert!(
            response
                .headers()
                .get("content-type")
                .unwrap()
                .to_str()
                .unwrap()
                .contains("text/html")
        );
    }
}
