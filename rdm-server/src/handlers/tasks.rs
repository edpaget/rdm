use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use rdm_core::hal::{HalLink, HalResource};
use rdm_core::model::{Priority, Task, TaskStatus};
use rdm_core::problem::ProblemDetail;

use crate::content_type::ResponseFormat;
use crate::error::{AppError, problem_detail_into_response};
use crate::extract::{hal_response, require_hal_json};
use crate::state::AppState;

/// Query parameters for filtering the task list.
#[derive(Debug, Deserialize, Default)]
pub struct TaskFilters {
    /// Filter by task status.
    pub status: Option<String>,
    /// Filter by priority.
    pub priority: Option<String>,
    /// Filter by tag.
    pub tag: Option<String>,
}

/// Empty data for the tasks collection wrapper.
#[derive(Serialize)]
struct TasksCollection {}

/// Detail data for a single task.
#[derive(Serialize)]
struct TaskDetail {
    slug: String,
    #[serde(flatten)]
    task: Task,
    body: String,
}

/// `GET /projects/:project/tasks` — list tasks with optional filters.
pub async fn list_tasks(
    format: ResponseFormat,
    State(state): State<AppState>,
    Path(project): Path<String>,
    Query(filters): Query<TaskFilters>,
) -> Result<Response, Response> {
    require_hal_json(format)?;

    // Validate filter values up front.
    let status_filter = match &filters.status {
        Some(s) => match s.parse::<TaskStatus>() {
            Ok(ts) => Some(ts),
            Err(_) => {
                return Err(problem_detail_into_response(ProblemDetail {
                    problem_type: "about:blank".to_string(),
                    title: "Bad Request".to_string(),
                    status: 400,
                    detail: Some(format!(
                        "invalid status filter: '{s}' (expected open, in-progress, done, or wont-fix)"
                    )),
                    instance: None,
                }));
            }
        },
        None => None,
    };

    let priority_filter = match &filters.priority {
        Some(p) => match p.parse::<Priority>() {
            Ok(pr) => Some(pr),
            Err(_) => {
                return Err(problem_detail_into_response(ProblemDetail {
                    problem_type: "about:blank".to_string(),
                    title: "Bad Request".to_string(),
                    status: 400,
                    detail: Some(format!(
                        "invalid priority filter: '{p}' (expected low, medium, high, or critical)"
                    )),
                    instance: None,
                }));
            }
        },
        None => None,
    };

    let repo = state.plan_repo();
    let tasks = repo
        .list_tasks(&project)
        .map_err(|e| AppError(e).into_response())?;

    let mut embedded = Vec::new();
    for (slug, doc) in &tasks {
        // Apply filters.
        if let Some(ref sf) = status_filter {
            if doc.frontmatter.status != *sf {
                continue;
            }
        }
        if let Some(ref pf) = priority_filter {
            if doc.frontmatter.priority != *pf {
                continue;
            }
        }
        if let Some(ref tag) = filters.tag {
            let has_tag = doc
                .frontmatter
                .tags
                .as_ref()
                .is_some_and(|tags| tags.contains(tag));
            if !has_tag {
                continue;
            }
        }

        let task_resource = HalResource::new(
            &doc.frontmatter,
            format!("/projects/{project}/tasks/{slug}"),
        )
        .with_link("project", HalLink::new(format!("/projects/{project}")));
        embedded.push(serde_json::to_value(&task_resource).unwrap());
    }

    let self_href = format!("/projects/{project}/tasks");
    let resource = HalResource::new(TasksCollection {}, self_href)
        .with_link("project", HalLink::new(format!("/projects/{project}")))
        .with_embedded("tasks", embedded);

    Ok(hal_response(resource))
}

/// `GET /projects/:project/tasks/:task` — task detail.
pub async fn get_task(
    format: ResponseFormat,
    State(state): State<AppState>,
    Path((project, task_slug)): Path<(String, String)>,
) -> Result<Response, Response> {
    require_hal_json(format)?;

    let repo = state.plan_repo();
    let doc = repo
        .load_task(&project, &task_slug)
        .map_err(|e| AppError(e).into_response())?;

    let self_href = format!("/projects/{project}/tasks/{task_slug}");
    let resource = HalResource::new(
        TaskDetail {
            slug: task_slug,
            task: doc.frontmatter,
            body: doc.body,
        },
        self_href,
    )
    .with_link("project", HalLink::new(format!("/projects/{project}")));

    Ok(hal_response(resource))
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;
    use axum::http::Request;
    use tempfile::TempDir;
    use tower::ServiceExt;

    use rdm_core::model::Priority;
    use rdm_core::repo::PlanRepo;

    use crate::router::build_router;
    use crate::state::AppState;

    fn setup() -> (TempDir, AppState) {
        let dir = TempDir::new().unwrap();
        let repo = PlanRepo::init(dir.path()).unwrap();
        repo.create_project("demo", "Demo").unwrap();
        repo.create_task(
            "demo",
            "bug-fix",
            "Fix the Bug",
            Priority::High,
            Some(vec!["bug".to_string()]),
            Some("Bug details.\n"),
        )
        .unwrap();
        repo.create_task("demo", "feature", "New Feature", Priority::Low, None, None)
            .unwrap();
        let state = AppState {
            plan_root: dir.path().to_path_buf(),
        };
        (dir, state)
    }

    #[tokio::test]
    async fn list_tasks_returns_all() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/tasks")
                    .header("accept", "application/hal+json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 8192).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let tasks = json["_embedded"]["tasks"].as_array().unwrap();
        assert_eq!(tasks.len(), 2);
    }

    #[tokio::test]
    async fn list_tasks_filter_by_priority() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/tasks?priority=high")
                    .header("accept", "application/hal+json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 8192).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let tasks = json["_embedded"]["tasks"].as_array().unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0]["priority"], "high");
    }

    #[tokio::test]
    async fn list_tasks_filter_by_tag() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/tasks?tag=bug")
                    .header("accept", "application/hal+json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 8192).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let tasks = json["_embedded"]["tasks"].as_array().unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0]["title"], "Fix the Bug");
    }

    #[tokio::test]
    async fn list_tasks_invalid_status_returns_400() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/tasks?status=bogus")
                    .header("accept", "application/hal+json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 400);
        let body = to_bytes(response.into_body(), 8192).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["detail"].as_str().unwrap().contains("bogus"));
    }

    #[tokio::test]
    async fn list_tasks_invalid_priority_returns_400() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/tasks?priority=bogus")
                    .header("accept", "application/hal+json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 400);
    }

    #[tokio::test]
    async fn get_task_returns_detail() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/tasks/bug-fix")
                    .header("accept", "application/hal+json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 8192).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["slug"], "bug-fix");
        assert_eq!(json["title"], "Fix the Bug");
        assert_eq!(json["body"], "Bug details.\n");
        assert_eq!(json["_links"]["project"]["href"], "/projects/demo");
    }

    #[tokio::test]
    async fn get_task_not_found() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/tasks/nonexistent")
                    .header("accept", "application/hal+json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 500);
    }

    #[tokio::test]
    async fn list_tasks_project_not_found() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/nonexistent/tasks")
                    .header("accept", "application/hal+json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 404);
    }

    #[tokio::test]
    async fn list_tasks_406_for_html() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/tasks")
                    .header("accept", "text/html")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 406);
    }
}
