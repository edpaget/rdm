use askama::Template;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use rdm_core::hal::{HalLink, HalResource};
use rdm_core::model::{PhaseStatus, TaskStatus};
use rdm_core::problem::ProblemDetail;
use rdm_core::search::{ItemKind, ItemStatus, SearchFilter, search};

use crate::content_type::ResponseFormat;
use crate::error::{error_response, problem_detail_into_response};
use crate::extract::hal_response;
use crate::state::AppState;
use crate::templates::{SearchResultRow, SearchResultsPage};

/// Query parameters for the search endpoint.
#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    /// The search query string (required).
    pub q: Option<String>,
    /// Filter by item kind: roadmap, phase, or task.
    #[serde(rename = "type")]
    pub kind: Option<String>,
    /// Filter by status.
    pub status: Option<String>,
    /// Maximum number of results (default 20).
    pub limit: Option<usize>,
}

/// HAL data for the search results collection.
#[derive(Serialize)]
struct SearchResultsCollection {
    query: String,
    total: usize,
}

/// HAL data for a single search result item.
#[derive(Serialize)]
struct SearchResultItem {
    kind: String,
    identifier: String,
    title: String,
    snippet: String,
    score: u32,
}

/// Parses a kind string into an `ItemKind`.
fn parse_kind(s: &str) -> Option<ItemKind> {
    match s {
        "roadmap" => Some(ItemKind::Roadmap),
        "phase" => Some(ItemKind::Phase),
        "task" => Some(ItemKind::Task),
        _ => None,
    }
}

/// Parses a status string into an `ItemStatus`.
///
/// Tries both phase and task status variants.
fn parse_status(s: &str) -> Option<ItemStatus> {
    if let Ok(ps) = s.parse::<PhaseStatus>() {
        return Some(ItemStatus::Phase(ps));
    }
    if let Ok(ts) = s.parse::<TaskStatus>() {
        return Some(ItemStatus::Task(ts));
    }
    None
}

/// Builds the detail page href for a search result.
fn result_href(project: &str, kind: ItemKind, identifier: &str) -> String {
    match kind {
        ItemKind::Roadmap => format!("/projects/{project}/roadmaps/{identifier}"),
        ItemKind::Phase => {
            // identifier is "roadmap-slug/phase-stem"
            if let Some((roadmap, phase)) = identifier.split_once('/') {
                format!("/projects/{project}/roadmaps/{roadmap}/phases/{phase}")
            } else {
                format!("/projects/{project}/roadmaps/{identifier}")
            }
        }
        ItemKind::Task => format!("/projects/{project}/tasks/{identifier}"),
    }
}

/// `GET /projects/:project/search` — search across all items in a project.
pub async fn search_items(
    format: ResponseFormat,
    State(state): State<AppState>,
    Path(project): Path<String>,
    Query(params): Query<SearchQuery>,
) -> Result<Response, Response> {
    let query = match params.q {
        Some(ref q) if !q.trim().is_empty() => q.trim().to_string(),
        _ => {
            return Err(problem_detail_into_response(ProblemDetail {
                problem_type: "about:blank".to_string(),
                title: "Bad Request".to_string(),
                status: 400,
                detail: Some("missing required query parameter: q".to_string()),
                instance: None,
            }));
        }
    };

    let kind_filter = match &params.kind {
        Some(k) => match parse_kind(k) {
            Some(kind) => Some(kind),
            None => {
                return Err(problem_detail_into_response(ProblemDetail {
                    problem_type: "about:blank".to_string(),
                    title: "Bad Request".to_string(),
                    status: 400,
                    detail: Some(format!(
                        "invalid type filter: '{k}' (expected roadmap, phase, or task)"
                    )),
                    instance: None,
                }));
            }
        },
        None => None,
    };

    let status_filter = match &params.status {
        Some(s) => match parse_status(s) {
            Some(status) => Some(status),
            None => {
                return Err(problem_detail_into_response(ProblemDetail {
                    problem_type: "about:blank".to_string(),
                    title: "Bad Request".to_string(),
                    status: 400,
                    detail: Some(format!("invalid status filter: '{s}'")),
                    instance: None,
                }));
            }
        },
        None => None,
    };

    let limit = params.limit.unwrap_or(20);

    let repo = state.plan_repo();
    let filter = SearchFilter {
        kind: kind_filter,
        project: Some(project.clone()),
        status: status_filter,
    };

    let results = search(&repo, &query, &filter).map_err(|e| error_response(e, format))?;
    let truncated: Vec<_> = results.into_iter().take(limit).collect();

    match format {
        ResponseFormat::HalJson => {
            let embedded: Vec<_> = truncated
                .iter()
                .map(|r| {
                    let href = result_href(&project, r.kind, &r.identifier);
                    let item = SearchResultItem {
                        kind: r.kind.to_string(),
                        identifier: r.identifier.clone(),
                        title: r.title.clone(),
                        snippet: r.snippet.clone(),
                        score: r.score,
                    };
                    let resource = HalResource::new(item, &href);
                    serde_json::to_value(&resource).unwrap()
                })
                .collect();

            let self_href = format!("/projects/{project}/search?q={query}");
            let resource = HalResource::new(
                SearchResultsCollection {
                    query,
                    total: truncated.len(),
                },
                self_href,
            )
            .with_link("project", HalLink::new(format!("/projects/{project}")))
            .with_embedded("results", embedded);

            Ok(hal_response(resource))
        }
        ResponseFormat::Html => {
            let rows: Vec<SearchResultRow> = truncated
                .iter()
                .map(|r| SearchResultRow {
                    kind: r.kind.to_string(),
                    title: r.title.clone(),
                    identifier: r.identifier.clone(),
                    snippet: r.snippet.clone(),
                    href: result_href(&project, r.kind, &r.identifier),
                })
                .collect();
            let page = SearchResultsPage {
                project,
                query,
                results: rows,
            };
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

    use rdm_core::model::Priority;
    use rdm_core::repo::PlanRepo;

    use crate::router::build_router;
    use crate::state::AppState;

    fn setup() -> (TempDir, AppState) {
        let dir = TempDir::new().unwrap();
        let repo = PlanRepo::init(dir.path()).unwrap();
        repo.create_project("demo", "Demo").unwrap();
        repo.create_roadmap(
            "demo",
            "widget-launch",
            "Widget Launch",
            Some("Launch widgets."),
        )
        .unwrap();
        repo.create_phase(
            "demo",
            "widget-launch",
            "design",
            "Design the Widget",
            Some(1),
            Some("Create mockups and wireframes."),
        )
        .unwrap();
        repo.create_task(
            "demo",
            "fix-login",
            "Fix Login Bug",
            Priority::High,
            None,
            Some("Users cannot log in with special characters."),
        )
        .unwrap();
        repo.create_task(
            "demo",
            "add-search",
            "Add Search Feature",
            Priority::Medium,
            None,
            None,
        )
        .unwrap();
        let state = AppState {
            plan_root: dir.path().to_path_buf(),
        };
        (dir, state)
    }

    #[tokio::test]
    async fn search_returns_results_json() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/search?q=widget")
                    .header("accept", "application/hal+json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 16384).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["query"], "widget");
        let results = json["_embedded"]["results"].as_array().unwrap();
        assert!(!results.is_empty());
    }

    #[tokio::test]
    async fn search_returns_results_html() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/search?q=widget")
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
        assert!(html.contains("widget"));
        assert!(html.contains("Widget Launch"));
        // Should contain links to detail pages
        assert!(html.contains("/projects/demo/roadmaps/widget-launch"));
    }

    #[tokio::test]
    async fn search_missing_query_returns_400() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/search")
                    .header("accept", "application/hal+json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 400);
    }

    #[tokio::test]
    async fn search_empty_query_returns_400() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/search?q=")
                    .header("accept", "application/hal+json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 400);
    }

    #[tokio::test]
    async fn search_with_type_filter() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/search?q=widget&type=roadmap")
                    .header("accept", "application/hal+json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 16384).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let results = json["_embedded"]["results"].as_array().unwrap();
        for r in results {
            assert_eq!(r["kind"], "roadmap");
        }
    }

    #[tokio::test]
    async fn search_no_matches_returns_empty() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/search?q=xyzzy-nonexistent-qqq")
                    .header("accept", "application/hal+json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 16384).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["total"], 0);
    }

    #[tokio::test]
    async fn search_html_contains_form_and_links() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/search?q=login")
                    .header("accept", "text/html")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 16384).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        // Should contain search form
        assert!(html.contains("<form"));
        assert!(html.contains("name=\"q\""));
        // Should contain task link
        assert!(html.contains("/projects/demo/tasks/fix-login"));
        assert!(html.contains("Fix Login Bug"));
    }

    #[tokio::test]
    async fn search_invalid_type_returns_400() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/search?q=test&type=bogus")
                    .header("accept", "application/hal+json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 400);
    }
}
