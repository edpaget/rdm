use askama::Template;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use crate::hal::{HalLink, HalResource};
use crate::problem::ProblemDetail;
use rdm_core::model::{Priority, Task, TaskStatus};
use rdm_core::ops::{BodyUpdate, TagsUpdate};

use crate::content_type::ResponseFormat;
use crate::error::{
    error_response, json_rejection_response, problem_detail_into_response, validation_error,
};
use crate::extract::{hal_created_response, hal_response, see_other_response};
use crate::markdown::render_markdown;
use crate::state::AppState;
use crate::templates::{
    TaskDetailPage, TaskListPage, TaskRow, priority_class, task_status_class, task_status_options,
};

/// Query parameters for filtering the task list.
#[derive(Debug, Deserialize, Default)]
pub struct TaskFilters {
    /// Filter by task status.
    pub status: Option<String>,
    /// Filter by priority.
    pub priority: Option<String>,
    /// Filter by tag.
    pub tag: Option<String>,
    /// When true, include completed (done/wont-fix) tasks in the list.
    pub show_completed: Option<bool>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    revision: Option<String>,
}

/// Query parameters for the task detail route.
#[derive(Debug, Deserialize, Default)]
pub struct TaskDetailFilters {
    /// Read the body as it was at a specific git revision.
    pub at: Option<String>,
}

/// `GET /projects/:project/tasks` — list tasks with optional filters.
pub async fn list_tasks(
    format: ResponseFormat,
    State(state): State<AppState>,
    Path(project): Path<String>,
    Query(filters): Query<TaskFilters>,
) -> Result<Response, Response> {
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

    let store = state.store();
    let tasks =
        rdm_core::ops::task::list_tasks(&store, &project).map_err(|e| error_response(e, format))?;

    // Filter tasks.
    let mut filtered: Vec<_> = tasks
        .iter()
        .filter(|(_, doc)| {
            if let Some(ref sf) = status_filter
                && doc.frontmatter.status != *sf
            {
                return false;
            }
            if let Some(ref pf) = priority_filter
                && doc.frontmatter.priority != *pf
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

    let show_completed = filters.show_completed.unwrap_or(false);
    if !show_completed {
        filtered.retain(|(_, doc)| {
            doc.frontmatter.status != TaskStatus::Done
                && doc.frontmatter.status != TaskStatus::WontFix
        });
    }

    match format {
        ResponseFormat::HalJson => {
            let mut embedded = Vec::new();
            for (slug, doc) in &filtered {
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
        ResponseFormat::Html => {
            let rows: Vec<TaskRow> = filtered
                .iter()
                .map(|(slug, doc)| TaskRow {
                    slug: (*slug).clone(),
                    title: doc.frontmatter.title.clone(),
                    status: doc.frontmatter.status.to_string(),
                    status_class: task_status_class(&doc.frontmatter.status).to_string(),
                    priority: doc.frontmatter.priority.to_string(),
                    priority_class: priority_class(&doc.frontmatter.priority).to_string(),
                })
                .collect();
            let task_list_path = format!("/projects/{project}/tasks");
            let quick_filters =
                state.quick_filter_views_for_path(&task_list_path, filters.tag.as_deref());
            let page = TaskListPage {
                project,
                tasks: rows,
                show_completed,
                quick_filters,
                active_tag: filters.tag,
            };
            Ok((
                [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
                page.render().expect("template rendering cannot fail"),
            )
                .into_response())
        }
    }
}

/// `GET /projects/:project/tasks/:task` — task detail.
pub async fn get_task(
    format: ResponseFormat,
    State(state): State<AppState>,
    Path((project, task_slug)): Path<(String, String)>,
    Query(filters): Query<TaskDetailFilters>,
) -> Result<Response, Response> {
    let store = state.store();
    let doc = match filters.at.as_deref() {
        Some(sha) => rdm_core::io::load_task_at(&store, &project, &task_slug, sha)
            .map_err(|e| error_response(e, format))?,
        None => rdm_core::io::load_task(&store, &project, &task_slug)
            .map_err(|e| error_response(e, format))?,
    };

    match format {
        ResponseFormat::HalJson => {
            let self_href = format!("/projects/{project}/tasks/{task_slug}");
            let resource = HalResource::new(
                TaskDetail {
                    slug: task_slug,
                    task: doc.frontmatter,
                    body: doc.body,
                    revision: filters.at.clone(),
                },
                self_href,
            )
            .with_link("project", HalLink::new(format!("/projects/{project}")));

            Ok(hal_response(resource))
        }
        ResponseFormat::Html => {
            let body_html = render_markdown(&doc.body);
            let page = TaskDetailPage {
                project,
                slug: task_slug,
                title: doc.frontmatter.title,
                status: doc.frontmatter.status.to_string(),
                status_class: task_status_class(&doc.frontmatter.status).to_string(),
                status_options: task_status_options(&doc.frontmatter.status),
                priority: doc.frontmatter.priority.to_string(),
                priority_class: priority_class(&doc.frontmatter.priority).to_string(),
                created: doc.frontmatter.created.to_string(),
                tags: doc.frontmatter.tags,
                body_html,
                body_md: doc.body,
                revision: filters.at,
            };
            Ok((
                [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
                page.render().expect("template rendering cannot fail"),
            )
                .into_response())
        }
    }
}

/// Default priority for new tasks.
fn default_priority() -> Priority {
    Priority::Medium
}

/// Request body for `POST /projects/:project/tasks`.
#[derive(Deserialize)]
pub struct CreateTaskRequest {
    slug: String,
    title: String,
    #[serde(default = "default_priority")]
    priority: Priority,
    tags: Option<Vec<String>>,
    body: Option<String>,
}

/// `POST /projects/:project/tasks` — create a new task.
pub async fn create_task(
    format: ResponseFormat,
    State(state): State<AppState>,
    Path(project): Path<String>,
    payload: Result<axum::Json<CreateTaskRequest>, JsonRejection>,
) -> Result<Response, Response> {
    let axum::Json(req) = payload.map_err(json_rejection_response)?;
    let mut store = state.store();
    let doc = rdm_core::ops::mutate(&mut store, &project, |s| {
        rdm_core::ops::task::create_task(
            s,
            rdm_core::ops::task::CreateTask {
                project: &project,
                slug: &req.slug,
                title: &req.title,
                priority: req.priority,
                tags: req.tags,
                body: req.body.as_deref(),
            },
        )
    })
    .map_err(|e| error_response(e, format))?;

    let location = format!("/projects/{project}/tasks/{}", req.slug);
    match format {
        ResponseFormat::HalJson => {
            let resource = HalResource::new(
                TaskDetail {
                    slug: req.slug,
                    task: doc.frontmatter,
                    body: doc.body,
                    revision: None,
                },
                &location,
            )
            .with_link("project", HalLink::new(format!("/projects/{project}")));
            Ok(hal_created_response(resource, &location))
        }
        ResponseFormat::Html => Ok(see_other_response(&location)),
    }
}

/// Request body for `PATCH /projects/:project/tasks/:task`.
#[derive(Deserialize)]
pub struct UpdateTaskRequest {
    status: Option<String>,
    priority: Option<String>,
    tags: Option<Vec<String>>,
    body: Option<String>,
    clear_body: Option<bool>,
}

/// `PATCH /projects/:project/tasks/:task` — update a task.
pub async fn update_task(
    format: ResponseFormat,
    State(state): State<AppState>,
    Path((project, task_slug)): Path<(String, String)>,
    payload: Result<axum::Json<UpdateTaskRequest>, JsonRejection>,
) -> Result<Response, Response> {
    let axum::Json(req) = payload.map_err(json_rejection_response)?;

    let status = match &req.status {
        Some(s) => Some(s.parse::<TaskStatus>().map_err(|_| {
            validation_error(format!(
                "invalid status: '{s}' (expected open, in-progress, done, or wont-fix)"
            ))
        })?),
        None => None,
    };

    let priority = match &req.priority {
        Some(p) => Some(p.parse::<Priority>().map_err(|_| {
            validation_error(format!(
                "invalid priority: '{p}' (expected low, medium, high, or critical)"
            ))
        })?),
        None => None,
    };

    let tags = TagsUpdate::from_args(req.tags, false).map_err(|e| error_response(e, format))?;
    let body = BodyUpdate::from_args(req.body, req.clear_body.unwrap_or(false))
        .map_err(|e| error_response(e, format))?;

    let mut store = state.store();
    let doc = rdm_core::ops::mutate(&mut store, &project, |s| {
        rdm_core::ops::task::update_task(
            s,
            &project,
            &task_slug,
            status,
            priority,
            tags,
            body,
            None,
            None,
            None,
            rdm_core::ops::TitleUpdate::Keep,
        )
    })
    .map_err(|e| error_response(e, format))?;

    let self_href = format!("/projects/{project}/tasks/{task_slug}");
    match format {
        ResponseFormat::HalJson => {
            let resource = HalResource::new(
                TaskDetail {
                    slug: task_slug,
                    task: doc.frontmatter,
                    body: doc.body,
                    revision: None,
                },
                &self_href,
            )
            .with_link("project", HalLink::new(format!("/projects/{project}")));
            Ok(hal_response(resource))
        }
        ResponseFormat::Html => Ok(see_other_response(&self_href)),
    }
}

/// Request body for `POST /projects/:project/tasks/:task/promote`.
#[derive(Deserialize)]
pub struct PromoteTaskRequest {
    roadmap_slug: String,
}

/// `POST /projects/:project/tasks/:task/promote` — promote a task to a roadmap.
pub async fn promote_task(
    format: ResponseFormat,
    State(state): State<AppState>,
    Path((project, task_slug)): Path<(String, String)>,
    payload: Result<axum::Json<PromoteTaskRequest>, JsonRejection>,
) -> Result<Response, Response> {
    let axum::Json(req) = payload.map_err(json_rejection_response)?;
    let mut store = state.store();
    rdm_core::ops::mutate(&mut store, &project, |s| {
        rdm_core::ops::task::promote_task(s, &project, &task_slug, &req.roadmap_slug)
    })
    .map_err(|e| error_response(e, format))?;

    let location = format!("/projects/{project}/roadmaps/{}", req.roadmap_slug);
    match format {
        ResponseFormat::HalJson => {
            // Load the newly created roadmap for the response body
            let roadmap_doc = rdm_core::io::load_roadmap(&store, &project, &req.roadmap_slug)
                .map_err(|e| error_response(e, format))?;
            let resource = HalResource::new(
                serde_json::json!({
                    "slug": roadmap_doc.frontmatter.roadmap,
                    "title": roadmap_doc.frontmatter.title,
                }),
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

    use rdm_core::model::{Priority, TaskStatus};

    use crate::router::build_router;
    use crate::state::AppState;

    fn setup() -> (TempDir, AppState) {
        let dir = TempDir::new().unwrap();
        let mut store = rdm_store_fs::FsStore::new(dir.path());
        rdm_core::ops::init::init(&mut store).unwrap();
        rdm_core::ops::project::create_project(&mut store, "demo", "Demo").unwrap();
        rdm_core::ops::task::create_task(
            &mut store,
            rdm_core::ops::task::CreateTask {
                project: "demo",
                slug: "bug-fix",
                title: "Fix the Bug",
                priority: Priority::High,
                tags: Some(vec!["bug".to_string()]),
                body: Some("Bug details.\n"),
            },
        )
        .unwrap();
        rdm_core::ops::task::create_task(
            &mut store,
            rdm_core::ops::task::CreateTask {
                project: "demo",
                slug: "feature",
                title: "New Feature",
                priority: Priority::Low,
                ..Default::default()
            },
        )
        .unwrap();
        rdm_core::store::Store::commit(&mut store).unwrap();
        let state = AppState {
            plan_root: dir.path().to_path_buf(),
            quick_filters: Vec::new(),
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
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
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
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
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
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
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
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
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
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
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
        assert_eq!(response.status(), 404);
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
    async fn list_tasks_returns_html() {
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
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("Fix the Bug"));
        assert!(html.contains("New Feature"));
        assert!(html.contains("badge-high"));
    }

    #[tokio::test]
    async fn get_task_returns_html() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/tasks/bug-fix")
                    .header("accept", "text/html")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("Fix the Bug"));
        assert!(html.contains("Bug details."));
        assert!(html.contains("badge-high"));
        assert!(html.contains("#main-content"));
        assert!(html.contains("aria-current=\"page\""));
    }

    #[tokio::test]
    async fn get_task_html_includes_status_edit_form() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/tasks/bug-fix")
                    .header("accept", "text/html")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("data-rdm-edit"));
        assert!(html.contains("data-rdm-method=\"PATCH\""));
        assert!(html.contains("action=\"/projects/demo/tasks/bug-fix\""));
        assert!(html.contains("<select id=\"task-status-edit\" name=\"status\">"));
        for s in ["open", "in-progress", "done", "wont-fix"] {
            assert!(
                html.contains(&format!("value=\"{s}\"")),
                "missing option value={s} in:\n{html}"
            );
        }
        // Freshly created task — its status is "open"; verify it is marked selected.
        assert!(html.contains("value=\"open\" selected"));
    }

    #[tokio::test]
    async fn get_task_html_includes_body_edit_form() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/tasks/bug-fix")
                    .header("accept", "text/html")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("<details"));
        assert!(html.contains("<summary"));
        assert!(html.contains("data-rdm-edit"));
        assert!(html.contains("data-rdm-method=\"PATCH\""));
        assert!(html.contains("action=\"/projects/demo/tasks/bug-fix\""));
        assert!(html.contains("<textarea"));
        assert!(html.contains("name=\"body\""));
        // The raw markdown body must appear inside the textarea.
        assert!(
            html.contains("Bug details."),
            "raw markdown body missing in:\n{html}"
        );
    }

    #[tokio::test]
    async fn get_task_html_tag_edit_form_renders_when_no_tags() {
        // `feature` task in setup() has no tags; tag editor should still render
        // with an empty input value and the Clear button still present.
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/tasks/feature")
                    .header("accept", "text/html")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(!html.contains("<dt>Tags</dt>"), "Tags row should be absent");
        assert!(html.contains("id=\"task-tags-edit\""));
        assert!(
            html.contains("value=\"\""),
            "empty tag input value missing in:\n{html}"
        );
        assert!(html.contains("name=\"clear_tags\""));
    }

    #[tokio::test]
    async fn get_task_html_includes_tag_edit_form() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/tasks/bug-fix")
                    .header("accept", "text/html")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("<details"));
        assert!(html.contains("<summary"));
        assert!(html.contains("data-rdm-edit"));
        assert!(html.contains("data-rdm-method=\"PATCH\""));
        assert!(html.contains("action=\"/projects/demo/tasks/bug-fix\""));
        assert!(html.contains("name=\"tags\""));
        assert!(
            html.contains("value=\"bug\""),
            "missing pre-populated tag input value in:\n{html}"
        );
        assert!(
            html.contains("name=\"clear_tags\"") && html.contains("value=\"true\""),
            "missing clear_tags submitter in:\n{html}"
        );
    }

    #[tokio::test]
    async fn get_task_404_returns_html_error() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/tasks/nonexistent")
                    .header("accept", "text/html")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 404);
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
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
    async fn create_task_returns_201() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(post_json(
                "/projects/demo/tasks",
                r#"{"slug":"new-task","title":"New Task","priority":"high","tags":["ui"]}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 201);
        assert_eq!(
            response.headers().get("location").unwrap(),
            "/projects/demo/tasks/new-task"
        );
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["slug"], "new-task");
        assert_eq!(json["title"], "New Task");
        assert_eq!(json["priority"], "high");
    }

    #[tokio::test]
    async fn create_task_defaults_to_medium_priority() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(post_json(
                "/projects/demo/tasks",
                r#"{"slug":"bare","title":"Bare Task"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 201);
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["priority"], "medium");
    }

    #[tokio::test]
    async fn create_task_duplicate_returns_409() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(post_json(
                "/projects/demo/tasks",
                r#"{"slug":"bug-fix","title":"Duplicate"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 409);
    }

    #[tokio::test]
    async fn create_task_missing_project_returns_404() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(post_json(
                "/projects/nonexistent/tasks",
                r#"{"slug":"x","title":"X"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 404);
    }

    #[tokio::test]
    async fn create_task_invalid_priority_returns_422() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(post_json(
                "/projects/demo/tasks",
                r#"{"slug":"x","title":"X","priority":"bogus"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 422);
    }

    #[tokio::test]
    async fn create_task_html_returns_303() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::post("/projects/demo/tasks")
                    .header("accept", "text/html")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(
                        r#"{"slug":"new-task","title":"New Task"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 303);
        assert_eq!(
            response.headers().get("location").unwrap(),
            "/projects/demo/tasks/new-task"
        );
    }

    #[tokio::test]
    async fn update_task_status() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(patch_json(
                "/projects/demo/tasks/bug-fix",
                r#"{"status":"done"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "done");
    }

    #[tokio::test]
    async fn update_task_priority() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(patch_json(
                "/projects/demo/tasks/bug-fix",
                r#"{"priority":"critical"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["priority"], "critical");
    }

    #[tokio::test]
    async fn update_task_multiple_fields() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(patch_json(
                "/projects/demo/tasks/bug-fix",
                r#"{"status":"done","priority":"low","tags":["fixed"]}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "done");
        assert_eq!(json["priority"], "low");
        assert_eq!(json["tags"][0], "fixed");
    }

    #[tokio::test]
    async fn update_task_not_found_returns_404() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(patch_json(
                "/projects/demo/tasks/nonexistent",
                r#"{"status":"done"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 404);
    }

    #[tokio::test]
    async fn update_task_invalid_status_returns_422() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(patch_json(
                "/projects/demo/tasks/bug-fix",
                r#"{"status":"bogus"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 422);
    }

    #[tokio::test]
    async fn patch_task_clears_body_with_empty_string() {
        let (_dir, state) = setup();
        let app = build_router(state.clone());
        let response = app
            .oneshot(patch_json(
                "/projects/demo/tasks/bug-fix",
                r#"{"clear_body":true}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 200);

        let app2 = build_router(state);
        let response = app2
            .oneshot(
                Request::get("/projects/demo/tasks/bug-fix")
                    .header("accept", "text/html")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(
            !html.contains(r#"<div class="body-content">"#),
            "body-content div should be gone after clearing"
        );
    }

    #[tokio::test]
    async fn update_task_html_returns_303() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::patch("/projects/demo/tasks/bug-fix")
                    .header("accept", "text/html")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(r#"{"status":"done"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 303);
        assert_eq!(
            response.headers().get("location").unwrap(),
            "/projects/demo/tasks/bug-fix"
        );
    }

    #[tokio::test]
    async fn promote_task_returns_201() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(post_json(
                "/projects/demo/tasks/bug-fix/promote",
                r#"{"roadmap_slug":"bug-fix-roadmap"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 201);
        assert_eq!(
            response.headers().get("location").unwrap(),
            "/projects/demo/roadmaps/bug-fix-roadmap"
        );
    }

    #[tokio::test]
    async fn promote_task_old_task_gone() {
        let (_dir, state) = setup();
        let app = build_router(state.clone());
        // Promote the task
        let response = app
            .oneshot(post_json(
                "/projects/demo/tasks/bug-fix/promote",
                r#"{"roadmap_slug":"bug-fix-rm"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 201);

        // Original task should be gone
        let app2 = build_router(state);
        let response = app2
            .oneshot(
                Request::get("/projects/demo/tasks/bug-fix")
                    .header("accept", "application/hal+json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 404);
    }

    #[tokio::test]
    async fn promote_task_not_found_returns_404() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(post_json(
                "/projects/demo/tasks/nonexistent/promote",
                r#"{"roadmap_slug":"x"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 404);
    }

    #[tokio::test]
    async fn promote_task_html_returns_303() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::post("/projects/demo/tasks/bug-fix/promote")
                    .header("accept", "text/html")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(r#"{"roadmap_slug":"bug-fix-rm"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 303);
        assert_eq!(
            response.headers().get("location").unwrap(),
            "/projects/demo/roadmaps/bug-fix-rm"
        );
    }

    /// Helper to set up a project with tasks in various statuses (open, done, wont-fix).
    fn setup_with_completed() -> (TempDir, AppState) {
        let dir = TempDir::new().unwrap();
        let mut store = rdm_store_fs::FsStore::new(dir.path());
        rdm_core::ops::init::init(&mut store).unwrap();
        rdm_core::ops::project::create_project(&mut store, "demo", "Demo").unwrap();
        rdm_core::ops::task::create_task(
            &mut store,
            rdm_core::ops::task::CreateTask {
                project: "demo",
                slug: "open-task",
                title: "Open Task",
                priority: Priority::Medium,
                ..Default::default()
            },
        )
        .unwrap();
        rdm_core::ops::task::create_task(
            &mut store,
            rdm_core::ops::task::CreateTask {
                project: "demo",
                slug: "done-task",
                title: "Done Task",
                priority: Priority::Medium,
                ..Default::default()
            },
        )
        .unwrap();
        rdm_core::ops::task::update_task(
            &mut store,
            "demo",
            "done-task",
            Some(TaskStatus::Done),
            None,
            rdm_core::ops::TagsUpdate::Keep,
            rdm_core::ops::BodyUpdate::Keep,
            None,
            None,
            None,
            rdm_core::ops::TitleUpdate::Keep,
        )
        .unwrap();
        rdm_core::ops::task::create_task(
            &mut store,
            rdm_core::ops::task::CreateTask {
                project: "demo",
                slug: "wontfix-task",
                title: "Wont Fix Task",
                priority: Priority::Low,
                ..Default::default()
            },
        )
        .unwrap();
        rdm_core::ops::task::update_task(
            &mut store,
            "demo",
            "wontfix-task",
            Some(TaskStatus::WontFix),
            None,
            rdm_core::ops::TagsUpdate::Keep,
            rdm_core::ops::BodyUpdate::Keep,
            None,
            None,
            None,
            rdm_core::ops::TitleUpdate::Keep,
        )
        .unwrap();
        rdm_core::store::Store::commit(&mut store).unwrap();
        let state = AppState {
            plan_root: dir.path().to_path_buf(),
            quick_filters: Vec::new(),
        };
        (dir, state)
    }

    #[tokio::test]
    async fn list_tasks_hides_completed_by_default_html() {
        let (_dir, state) = setup_with_completed();
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
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("Open Task"));
        assert!(!html.contains("Done Task"));
        assert!(!html.contains("Wont Fix Task"));
        assert!(html.contains("Show completed tasks"));
    }

    #[tokio::test]
    async fn list_tasks_shows_completed_when_requested_html() {
        let (_dir, state) = setup_with_completed();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/tasks?show_completed=true")
                    .header("accept", "text/html")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("Open Task"));
        assert!(html.contains("Done Task"));
        assert!(html.contains("Wont Fix Task"));
        assert!(html.contains("Hide completed tasks"));
    }

    #[tokio::test]
    async fn list_tasks_hides_completed_by_default_hal() {
        let (_dir, state) = setup_with_completed();
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
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let tasks = json["_embedded"]["tasks"].as_array().unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0]["title"], "Open Task");
    }

    #[tokio::test]
    async fn list_tasks_shows_completed_when_requested_hal() {
        let (_dir, state) = setup_with_completed();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/tasks?show_completed=true")
                    .header("accept", "application/hal+json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let tasks = json["_embedded"]["tasks"].as_array().unwrap();
        assert_eq!(tasks.len(), 3);
    }
}
