use askama::Template;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use rdm_core::hal::{HalLink, HalResource};

use crate::content_type::ResponseFormat;
use crate::error::{error_response, json_rejection_response};
use crate::extract::{hal_created_response, hal_response, see_other_response};
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

/// Request body for `POST /projects`.
#[derive(Deserialize)]
pub struct CreateProjectRequest {
    name: String,
    title: String,
}

/// `POST /projects` — create a new project.
pub async fn create_project(
    format: ResponseFormat,
    State(state): State<AppState>,
    payload: Result<axum::Json<CreateProjectRequest>, JsonRejection>,
) -> Result<Response, Response> {
    let axum::Json(req) = payload.map_err(json_rejection_response)?;
    let repo = state.plan_repo();
    let doc = repo
        .create_project(&req.name, &req.title)
        .map_err(|e| error_response(e, format))?;
    repo.generate_index()
        .map_err(|e| error_response(e, format))?;

    let location = format!("/projects/{}/roadmaps", doc.frontmatter.name);
    match format {
        ResponseFormat::HalJson => {
            let resource = HalResource::new(
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

    fn post_json(uri: &str, body: &str) -> Request<axum::body::Body> {
        Request::post(uri)
            .header("accept", "application/hal+json")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap()
    }

    fn post_json_html(uri: &str, body: &str) -> Request<axum::body::Body> {
        Request::post(uri)
            .header("accept", "text/html")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap()
    }

    #[tokio::test]
    async fn create_project_returns_201() {
        let dir = TempDir::new().unwrap();
        PlanRepo::init(dir.path()).unwrap();
        let state = AppState {
            plan_root: dir.path().to_path_buf(),
        };
        let app = build_router(state);
        let response = app
            .oneshot(post_json(
                "/projects",
                r#"{"name":"gamma","title":"Gamma Project"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 201);
        assert_eq!(
            response.headers().get("location").unwrap(),
            "/projects/gamma/roadmaps"
        );
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/hal+json"
        );
        let body = to_bytes(response.into_body(), 8192).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["name"], "gamma");
        assert_eq!(json["title"], "Gamma Project");
    }

    #[tokio::test]
    async fn create_project_duplicate_returns_409() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(post_json(
                "/projects",
                r#"{"name":"alpha","title":"Alpha Again"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 409);
    }

    #[tokio::test]
    async fn create_project_malformed_json_returns_422() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(post_json("/projects", r#"{"bad":true}"#))
            .await
            .unwrap();
        assert_eq!(response.status(), 422);
    }

    #[tokio::test]
    async fn create_project_html_returns_303() {
        let dir = TempDir::new().unwrap();
        PlanRepo::init(dir.path()).unwrap();
        let state = AppState {
            plan_root: dir.path().to_path_buf(),
        };
        let app = build_router(state);
        let response = app
            .oneshot(post_json_html(
                "/projects",
                r#"{"name":"gamma","title":"Gamma Project"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 303);
        assert_eq!(
            response.headers().get("location").unwrap(),
            "/projects/gamma/roadmaps"
        );
    }
}
