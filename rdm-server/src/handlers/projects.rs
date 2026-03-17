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

/// Summary data for a project in a collection.
#[derive(Serialize)]
struct ProjectData {
    name: String,
    title: String,
}

/// Empty data for the projects collection wrapper.
#[derive(Serialize)]
struct ProjectsCollection {}

/// `GET /projects` — list all projects.
pub async fn list_projects(
    format: ResponseFormat,
    State(state): State<AppState>,
) -> Result<Response, Response> {
    let repo = state.plan_repo();
    let names = repo
        .list_projects()
        .map_err(|e| error_response(e, format))?;

    match format {
        ResponseFormat::HalJson => {
            let mut embedded = Vec::new();
            for name in &names {
                let doc = repo
                    .load_project(name)
                    .map_err(|e| error_response(e, format))?;
                let project_resource = HalResource::new(
                    ProjectData {
                        name: doc.frontmatter.name.clone(),
                        title: doc.frontmatter.title.clone(),
                    },
                    format!("/projects/{}", doc.frontmatter.name),
                )
                .with_link(
                    "roadmaps",
                    HalLink::new(format!("/projects/{}/roadmaps", doc.frontmatter.name)),
                )
                .with_link(
                    "tasks",
                    HalLink::new(format!("/projects/{}/tasks", doc.frontmatter.name)),
                );
                embedded.push(serde_json::to_value(&project_resource).unwrap());
            }

            let resource = HalResource::new(ProjectsCollection {}, "/projects")
                .with_embedded("projects", embedded);

            Ok(hal_response(resource))
        }
        ResponseFormat::Html => {
            let mut projects = Vec::new();
            for name in &names {
                let doc = repo
                    .load_project(name)
                    .map_err(|e| error_response(e, format))?;
                projects.push(ProjectView {
                    name: doc.frontmatter.name,
                    title: doc.frontmatter.title,
                });
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

    use rdm_core::repo::PlanRepo;

    use crate::router::build_router;
    use crate::state::AppState;

    fn setup() -> (TempDir, AppState) {
        let dir = TempDir::new().unwrap();
        let repo = PlanRepo::init(dir.path()).unwrap();
        repo.create_project("alpha", "Alpha Project").unwrap();
        repo.create_project("beta", "Beta Project").unwrap();
        let state = AppState {
            plan_root: dir.path().to_path_buf(),
        };
        (dir, state)
    }

    #[tokio::test]
    async fn list_projects_returns_embedded() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects")
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
        let body = to_bytes(response.into_body(), 8192).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let projects = json["_embedded"]["projects"].as_array().unwrap();
        assert_eq!(projects.len(), 2);
        assert_eq!(projects[0]["name"], "alpha");
        assert_eq!(projects[1]["name"], "beta");
        assert_eq!(projects[0]["_links"]["self"]["href"], "/projects/alpha");
        assert_eq!(
            projects[0]["_links"]["roadmaps"]["href"],
            "/projects/alpha/roadmaps"
        );
    }

    #[tokio::test]
    async fn list_projects_empty() {
        let dir = TempDir::new().unwrap();
        PlanRepo::init(dir.path()).unwrap();
        let state = AppState {
            plan_root: dir.path().to_path_buf(),
        };
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects")
                    .header("accept", "application/hal+json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 8192).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["_links"]["self"]["href"], "/projects");
    }

    #[tokio::test]
    async fn list_projects_returns_html() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects")
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
        let body = to_bytes(response.into_body(), 8192).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("Alpha Project"));
        assert!(html.contains("Beta Project"));
    }
}
