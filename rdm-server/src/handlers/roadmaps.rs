use std::time::SystemTime;

use askama::Template;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use crate::hal::{HalLink, HalResource};
use rdm_core::document::Document;
use rdm_core::model::{Phase, PhaseStatus, Priority, RoadmapSort};
use rdm_core::ops::{BodyUpdate, PriorityUpdate, TagsUpdate};

use axum::extract::Query;

use crate::problem::ProblemDetail;

use crate::content_type::ResponseFormat;
use crate::error::{
    error_response, json_rejection_response, problem_detail_into_response, validation_error,
};
use crate::extract::{hal_created_response, hal_response, see_other_response};
use crate::state::AppState;
use crate::templates::{
    PhaseRow, RoadmapDetailPage, RoadmapSummaryView, RoadmapsPage, computed_roadmap_status,
    phase_status_class, priority_class,
};

/// Query parameters for filtering the roadmap list.
#[derive(Debug, Deserialize, Default)]
pub struct RoadmapFilters {
    /// When true, include completed roadmaps (all phases done) in the list.
    pub show_completed: Option<bool>,
    /// Filter by priority level (low, medium, high, critical).
    pub priority: Option<String>,
    /// Sort order (alphabetical or priority).
    pub sort: Option<String>,
    /// Filter to roadmaps carrying this tag.
    pub tag: Option<String>,
}

/// Format a `SystemTime` as a `YYYY-MM-DD` date string.
fn format_system_time(t: SystemTime) -> String {
    let dt: chrono::DateTime<chrono::Utc> = t.into();
    dt.format("%Y-%m-%d").to_string()
}

/// Compute the most recent modification date across the roadmap and phase files.
fn last_changed_date(
    store: &rdm_store_fs::FsStore,
    project: &str,
    roadmap: &str,
    phases: &[(String, Document<Phase>)],
) -> Option<String> {
    rdm_core::ops::roadmap::last_modified(store, project, roadmap, phases).map(format_system_time)
}

/// Summary data for a roadmap in a collection.
#[derive(Serialize)]
struct RoadmapSummary {
    slug: String,
    title: String,
    total_phases: usize,
    done_phases: usize,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_changed: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    priority: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tags: Option<Vec<String>>,
}

/// Empty data for the roadmaps collection wrapper.
#[derive(Serialize)]
struct RoadmapsCollection {}

/// Detail data for a single roadmap.
#[derive(Serialize)]
struct RoadmapDetail {
    slug: String,
    title: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_changed: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dependencies: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    priority: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    revision: Option<String>,
}

/// `GET /projects/:project/roadmaps` — list roadmaps with progress summaries.
pub async fn list_roadmaps(
    format: ResponseFormat,
    State(state): State<AppState>,
    Path(project): Path<String>,
    Query(filters): Query<RoadmapFilters>,
) -> Result<Response, Response> {
    let store = state.store();

    let priority_filter = match &filters.priority {
        Some(p) => Some(p.parse::<Priority>().map_err(|_| {
            problem_detail_into_response(ProblemDetail {
                problem_type: "about:blank".to_string(),
                title: "Bad Request".to_string(),
                status: 400,
                detail: Some(format!(
                    "invalid priority filter: '{p}' (expected low, medium, high, or critical)"
                )),
                instance: None,
            })
        })?),
        None => None,
    };
    let sort = match &filters.sort {
        Some(s) => Some(s.parse::<RoadmapSort>().map_err(|_| {
            problem_detail_into_response(ProblemDetail {
                problem_type: "about:blank".to_string(),
                title: "Bad Request".to_string(),
                status: 400,
                detail: Some(format!(
                    "invalid sort: '{s}' (expected alphabetical or priority)"
                )),
                instance: None,
            })
        })?),
        None => None,
    };

    let roadmaps = rdm_core::ops::roadmap::list_roadmaps(&store, &project, sort, priority_filter)
        .map_err(|e| error_response(e, format))?;

    struct Summary {
        slug: String,
        title: String,
        total_phases: usize,
        done_phases: usize,
        status: String,
        status_class: String,
        last_changed: Option<String>,
        priority: Option<String>,
        priority_class: Option<String>,
        tags: Option<Vec<String>>,
    }

    let mut summaries = Vec::new();
    for roadmap_doc in &roadmaps {
        let slug = &roadmap_doc.frontmatter.roadmap;
        if let Some(ref tag) = filters.tag
            && !roadmap_doc
                .frontmatter
                .tags
                .as_ref()
                .is_some_and(|tags| tags.iter().any(|t| t == tag))
        {
            continue;
        }

        let phases = rdm_core::ops::phase::list_phases(&store, &project, slug)
            .map_err(|e| error_response(e, format))?;
        let done_count = phases
            .iter()
            .filter(|(_, doc)| doc.frontmatter.status.is_terminal())
            .count();

        let phase_statuses: Vec<PhaseStatus> = phases
            .iter()
            .map(|(_, doc)| doc.frontmatter.status)
            .collect();
        let (status_text, status_cls) = computed_roadmap_status(&phase_statuses);

        let last_changed = last_changed_date(&store, &project, slug, &phases);
        let priority = roadmap_doc.frontmatter.priority.map(|p| p.to_string());
        let pri_class = roadmap_doc
            .frontmatter
            .priority
            .map(|p| priority_class(&p).to_string());

        summaries.push(Summary {
            slug: slug.clone(),
            title: roadmap_doc.frontmatter.title.clone(),
            total_phases: phases.len(),
            done_phases: done_count,
            status: status_text.to_string(),
            status_class: status_cls.to_string(),
            last_changed,
            priority,
            priority_class: pri_class,
            tags: roadmap_doc.frontmatter.tags.clone(),
        });
    }

    let show_completed = filters.show_completed.unwrap_or(false);
    if !show_completed {
        summaries.retain(|s| s.status != "done");
    }

    match format {
        ResponseFormat::HalJson => {
            let mut embedded = Vec::new();
            for s in &summaries {
                let summary = HalResource::new(
                    RoadmapSummary {
                        slug: s.slug.clone(),
                        title: s.title.clone(),
                        total_phases: s.total_phases,
                        done_phases: s.done_phases,
                        status: s.status.clone(),
                        last_changed: s.last_changed.clone(),
                        priority: s.priority.clone(),
                        tags: s.tags.clone(),
                    },
                    format!("/projects/{project}/roadmaps/{}", s.slug),
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
        ResponseFormat::Html => {
            // Same counting pass as INDEX.md generation (phase-targeted
            // reviews roll up into their parent roadmap), so the two
            // surfaces always report the same numbers.
            let review_counts = rdm_core::ops::reviews::count_open_reviews(&store, &project)
                .map_err(|e| error_response(e, format))?;
            let views: Vec<RoadmapSummaryView> = summaries
                .into_iter()
                .map(|s| {
                    let counts = review_counts
                        .roadmaps
                        .get(&s.slug)
                        .copied()
                        .unwrap_or_default();
                    RoadmapSummaryView {
                        slug: s.slug,
                        title: s.title,
                        total_phases: s.total_phases,
                        done_phases: s.done_phases,
                        status: s.status,
                        status_class: s.status_class,
                        last_changed: s.last_changed,
                        priority: s.priority,
                        priority_class: s.priority_class,
                        open_reviews: counts.open_reviews,
                        open_comments: counts.open_comments,
                    }
                })
                .collect();
            let quick_filters = state.quick_filter_views_for_path(
                &format!("/projects/{project}/roadmaps"),
                filters.tag.as_deref(),
            );
            let page = RoadmapsPage {
                project,
                roadmaps: views,
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

/// Query parameters for the roadmap detail page (filters the embedded phases).
#[derive(Debug, Deserialize, Default)]
pub struct RoadmapDetailFilters {
    /// Filter the embedded phases section to phases carrying this tag.
    pub tag: Option<String>,
    /// Read the body as it was at a specific git revision.
    pub at: Option<String>,
}

/// `GET /projects/:project/roadmaps/:roadmap` — roadmap detail with embedded phases.
pub async fn get_roadmap(
    format: ResponseFormat,
    State(state): State<AppState>,
    Path((project, roadmap)): Path<(String, String)>,
    Query(filters): Query<RoadmapDetailFilters>,
) -> Result<Response, Response> {
    let store = state.store();
    let roadmap_doc = match filters.at.as_deref() {
        Some(sha) => rdm_core::io::load_roadmap_at(&store, &project, &roadmap, sha)
            .map_err(|e| error_response(e, format))?,
        None => rdm_core::io::load_roadmap(&store, &project, &roadmap)
            .map_err(|e| error_response(e, format))?,
    };

    let mut phases = rdm_core::ops::phase::list_phases(&store, &project, &roadmap)
        .map_err(|e| error_response(e, format))?;

    if let Some(ref tag) = filters.tag {
        phases.retain(|(_, doc)| {
            doc.frontmatter
                .tags
                .as_ref()
                .is_some_and(|tags| tags.iter().any(|t| t == tag))
        });
    }

    let phase_statuses: Vec<PhaseStatus> = phases
        .iter()
        .map(|(_, doc)| doc.frontmatter.status)
        .collect();
    let (status_text, status_cls) = computed_roadmap_status(&phase_statuses);
    let last_changed = last_changed_date(&store, &project, &roadmap, &phases);

    match format {
        ResponseFormat::HalJson => {
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
                    status: status_text.to_string(),
                    last_changed: last_changed.clone(),
                    dependencies: roadmap_doc.frontmatter.dependencies,
                    priority: roadmap_doc.frontmatter.priority.map(|p| p.to_string()),
                    tags: roadmap_doc.frontmatter.tags,
                    revision: filters.at.clone(),
                },
                self_href,
            )
            .with_link("project", HalLink::new(format!("/projects/{project}")))
            .with_embedded("phases", phase_embedded);

            Ok(hal_response(resource))
        }
        ResponseFormat::Html => {
            let phase_rows: Vec<PhaseRow> = phases
                .iter()
                .map(|(stem, doc)| {
                    let status_cls = phase_status_class(&doc.frontmatter.status).to_string();
                    PhaseRow {
                        phase: doc.frontmatter.phase,
                        stem: stem.clone(),
                        title: doc.frontmatter.title.clone(),
                        status: doc.frontmatter.status.to_string(),
                        status_class: status_cls,
                    }
                })
                .collect();
            let priority = roadmap_doc.frontmatter.priority.map(|p| p.to_string());
            let pri_class = roadmap_doc
                .frontmatter
                .priority
                .map(|p| priority_class(&p).to_string());
            let detail_path = format!(
                "/projects/{project}/roadmaps/{}",
                roadmap_doc.frontmatter.roadmap
            );
            let quick_filters =
                state.quick_filter_views_for_path(&detail_path, filters.tag.as_deref());
            // Inline highlights index the *current* body; a pinned `?at=`
            // view disables them (quote previews still render). Doc-scoped
            // comments never highlight here — they link through to their
            // phase instead. Note: `phases` may be tag-filtered; a comment
            // scoped to a filtered-out phase degrades to a stem-only link
            // label.
            let page_reviews = crate::review_views::page_reviews(
                &store,
                &project,
                &crate::review_views::PageDoc::Roadmap {
                    roadmap: &roadmap,
                    phases: &phases,
                },
                filters.at.is_none(),
            )
            .map_err(|e| error_response(e, format))?;
            let body_html = page_reviews.render_body(&roadmap_doc.body);
            let page = RoadmapDetailPage {
                project,
                slug: roadmap_doc.frontmatter.roadmap,
                title: roadmap_doc.frontmatter.title,
                status: status_text.to_string(),
                status_class: status_cls.to_string(),
                last_changed,
                priority,
                priority_class: pri_class,
                dependencies: roadmap_doc.frontmatter.dependencies,
                tags: roadmap_doc.frontmatter.tags,
                body_html,
                body_md: roadmap_doc.body,
                phases: phase_rows,
                quick_filters,
                active_tag: filters.tag,
                revision: filters.at,
                reviews: page_reviews.reviews,
            };
            Ok((
                [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
                page.render().expect("template rendering cannot fail"),
            )
                .into_response())
        }
    }
}

/// Request body for `POST /projects/:project/roadmaps`.
#[derive(Deserialize)]
pub struct CreateRoadmapRequest {
    slug: String,
    title: String,
    body: Option<String>,
    priority: Option<String>,
    tags: Option<Vec<String>>,
}

/// Parse a priority string into a `Priority`, returning a 422 on invalid values.
#[allow(clippy::result_large_err)]
fn parse_priority(s: &str) -> Result<Priority, Response> {
    s.parse::<Priority>().map_err(|_| {
        validation_error(format!(
            "invalid priority: '{s}' (expected low, medium, high, or critical)"
        ))
    })
}

/// `POST /projects/:project/roadmaps` — create a new roadmap.
pub async fn create_roadmap(
    format: ResponseFormat,
    State(state): State<AppState>,
    Path(project): Path<String>,
    payload: Result<axum::Json<CreateRoadmapRequest>, JsonRejection>,
) -> Result<Response, Response> {
    let axum::Json(req) = payload.map_err(json_rejection_response)?;
    let priority = req.priority.as_deref().map(parse_priority).transpose()?;
    let mut store = state.store();
    let doc = rdm_core::ops::mutate(&mut store, &project, |s| {
        rdm_core::ops::roadmap::create_roadmap(
            s,
            rdm_core::ops::roadmap::CreateRoadmap {
                project: &project,
                slug: &req.slug,
                title: &req.title,
                body: req.body.as_deref(),
                priority,
                tags: req.tags.clone(),
            },
        )
    })
    .map_err(|e| error_response(e, format))?;

    let location = format!("/projects/{project}/roadmaps/{}", doc.frontmatter.roadmap);
    match format {
        ResponseFormat::HalJson => {
            let resource = HalResource::new(
                RoadmapDetail {
                    slug: doc.frontmatter.roadmap.clone(),
                    title: doc.frontmatter.title.clone(),
                    status: "not-started".to_string(),
                    last_changed: None,
                    dependencies: doc.frontmatter.dependencies,
                    priority: doc.frontmatter.priority.map(|p| p.to_string()),
                    tags: doc.frontmatter.tags,
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

/// Request body for `PATCH /projects/:project/roadmaps/:roadmap`.
#[derive(Deserialize)]
pub struct UpdateRoadmapRequest {
    priority: Option<String>,
    clear_priority: Option<bool>,
    body: Option<String>,
    clear_body: Option<bool>,
    tags: Option<Vec<String>>,
    clear_tags: Option<bool>,
}

/// `PATCH /projects/:project/roadmaps/:roadmap` — update a roadmap.
pub async fn update_roadmap(
    format: ResponseFormat,
    State(state): State<AppState>,
    Path((project, roadmap)): Path<(String, String)>,
    payload: Result<axum::Json<UpdateRoadmapRequest>, JsonRejection>,
) -> Result<Response, Response> {
    let axum::Json(req) = payload.map_err(json_rejection_response)?;

    let body = BodyUpdate::from_args(req.body, req.clear_body.unwrap_or(false))
        .map_err(|e| error_response(e, format))?;
    let priority = PriorityUpdate::from_args(
        req.priority.as_deref().map(parse_priority).transpose()?,
        req.clear_priority.unwrap_or(false),
    )
    .map_err(|e| error_response(e, format))?;
    let tags = TagsUpdate::from_args(req.tags, req.clear_tags.unwrap_or(false))
        .map_err(|e| error_response(e, format))?;

    let mut store = state.store();
    let doc = rdm_core::ops::mutate(&mut store, &project, |s| {
        rdm_core::ops::roadmap::update_roadmap(
            s,
            &project,
            &roadmap,
            body,
            priority,
            tags,
            rdm_core::ops::TitleUpdate::Keep,
        )
    })
    .map_err(|e| error_response(e, format))?;

    let phases = rdm_core::ops::phase::list_phases(&store, &project, &roadmap)
        .map_err(|e| error_response(e, format))?;
    let phase_statuses: Vec<PhaseStatus> =
        phases.iter().map(|(_, d)| d.frontmatter.status).collect();
    let (status_text, _) = computed_roadmap_status(&phase_statuses);
    let last_changed = last_changed_date(&store, &project, &roadmap, &phases);

    let self_href = format!("/projects/{project}/roadmaps/{roadmap}");
    match format {
        ResponseFormat::HalJson => {
            let resource = HalResource::new(
                RoadmapDetail {
                    slug: doc.frontmatter.roadmap,
                    title: doc.frontmatter.title,
                    status: status_text.to_string(),
                    last_changed,
                    dependencies: doc.frontmatter.dependencies,
                    priority: doc.frontmatter.priority.map(|p| p.to_string()),
                    tags: doc.frontmatter.tags,
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

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;
    use axum::http::Request;
    use tempfile::TempDir;
    use tower::ServiceExt;

    use rdm_core::model::PhaseStatus;

    use crate::router::build_router;
    use crate::state::AppState;

    fn setup() -> (TempDir, AppState) {
        let dir = TempDir::new().unwrap();
        let mut store = rdm_store_fs::FsStore::new(dir.path());
        rdm_core::ops::init::init(&mut store).unwrap();
        rdm_core::ops::project::create_project(&mut store, "demo", "Demo Project").unwrap();
        rdm_core::ops::roadmap::create_roadmap(
            &mut store,
            rdm_core::ops::roadmap::CreateRoadmap {
                project: "demo",
                slug: "alpha",
                title: "Alpha Roadmap",
                ..Default::default()
            },
        )
        .unwrap();
        rdm_core::ops::phase::create_phase(
            &mut store,
            rdm_core::ops::phase::CreatePhase {
                project: "demo",
                roadmap: "alpha",
                slug: "first",
                title: "First Phase",
                number: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        rdm_core::ops::phase::create_phase(
            &mut store,
            rdm_core::ops::phase::CreatePhase {
                project: "demo",
                roadmap: "alpha",
                slug: "second",
                title: "Second Phase",
                number: Some(2),
                ..Default::default()
            },
        )
        .unwrap();
        rdm_core::ops::phase::update_phase(
            &mut store,
            "demo",
            "alpha",
            "phase-1-first",
            Some(PhaseStatus::Done),
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

    /// Create a setup with an additional completed roadmap ("beta") for filter tests.
    fn setup_with_completed() -> (TempDir, AppState) {
        let (dir, state) = setup();
        let mut store = rdm_store_fs::FsStore::new(dir.path());
        rdm_core::ops::roadmap::create_roadmap(
            &mut store,
            rdm_core::ops::roadmap::CreateRoadmap {
                project: "demo",
                slug: "beta",
                title: "Beta Roadmap",
                ..Default::default()
            },
        )
        .unwrap();
        rdm_core::ops::phase::create_phase(
            &mut store,
            rdm_core::ops::phase::CreatePhase {
                project: "demo",
                roadmap: "beta",
                slug: "only",
                title: "Only Phase",
                number: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        rdm_core::ops::phase::update_phase(
            &mut store,
            "demo",
            "beta",
            "phase-1-only",
            Some(PhaseStatus::Done),
            rdm_core::ops::TagsUpdate::Keep,
            rdm_core::ops::BodyUpdate::Keep,
            None,
            None,
            None,
            rdm_core::ops::TitleUpdate::Keep,
        )
        .unwrap();
        rdm_core::store::Store::commit(&mut store).unwrap();
        (dir, state)
    }

    #[tokio::test]
    async fn list_roadmaps_hides_completed_by_default_html() {
        let (_dir, state) = setup_with_completed();
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
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("Alpha Roadmap"));
        assert!(!html.contains("Beta Roadmap"));
        assert!(html.contains("Show completed roadmaps"));
    }

    #[tokio::test]
    async fn list_roadmaps_shows_completed_when_requested_html() {
        let (_dir, state) = setup_with_completed();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/roadmaps?show_completed=true")
                    .header("accept", "text/html")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("Alpha Roadmap"));
        assert!(html.contains("Beta Roadmap"));
        assert!(html.contains("Hide completed roadmaps"));
    }

    #[tokio::test]
    async fn list_roadmaps_hides_completed_by_default_hal_json() {
        let (_dir, state) = setup_with_completed();
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
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let roadmaps = json["_embedded"]["roadmaps"].as_array().unwrap();
        assert_eq!(roadmaps.len(), 1);
        assert_eq!(roadmaps[0]["slug"], "alpha");
    }

    #[tokio::test]
    async fn list_roadmaps_shows_completed_when_requested_hal_json() {
        let (_dir, state) = setup_with_completed();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/roadmaps?show_completed=true")
                    .header("accept", "application/hal+json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let roadmaps = json["_embedded"]["roadmaps"].as_array().unwrap();
        assert_eq!(roadmaps.len(), 2);
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
        let body = to_bytes(response.into_body(), 16384).await.unwrap();
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
        let body = to_bytes(response.into_body(), 16384).await.unwrap();
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
    async fn list_roadmaps_returns_html() {
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
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 16384).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("Alpha Roadmap"));
        assert!(html.contains("1/2 phases done"));
    }

    #[tokio::test]
    async fn get_roadmap_returns_html() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/roadmaps/alpha")
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
        assert!(html.contains("Alpha Roadmap"));
        assert!(html.contains("First Phase"));
        assert!(html.contains("badge-done"));
    }

    fn post_json(uri: &str, body: &str) -> Request<axum::body::Body> {
        Request::post(uri)
            .header("accept", "application/hal+json")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap()
    }

    #[tokio::test]
    async fn create_roadmap_returns_201() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(post_json(
                "/projects/demo/roadmaps",
                r#"{"slug":"beta","title":"Beta Roadmap"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 201);
        assert_eq!(
            response.headers().get("location").unwrap(),
            "/projects/demo/roadmaps/beta"
        );
        let body = to_bytes(response.into_body(), 16384).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["slug"], "beta");
        assert_eq!(json["title"], "Beta Roadmap");
    }

    #[tokio::test]
    async fn create_roadmap_missing_project_returns_404() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(post_json(
                "/projects/nonexistent/roadmaps",
                r#"{"slug":"beta","title":"Beta"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 404);
    }

    #[tokio::test]
    async fn create_roadmap_duplicate_returns_409() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(post_json(
                "/projects/demo/roadmaps",
                r#"{"slug":"alpha","title":"Alpha Again"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 409);
    }

    #[tokio::test]
    async fn create_roadmap_html_returns_303() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::post("/projects/demo/roadmaps")
                    .header("accept", "text/html")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(r#"{"slug":"beta","title":"Beta"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 303);
        assert_eq!(
            response.headers().get("location").unwrap(),
            "/projects/demo/roadmaps/beta"
        );
    }

    #[test]
    fn computed_status_all_done() {
        use crate::templates::computed_roadmap_status;
        let statuses = vec![PhaseStatus::Done, PhaseStatus::Done];
        assert_eq!(computed_roadmap_status(&statuses), ("done", "done"));
    }

    #[test]
    fn computed_status_some_in_progress() {
        use crate::templates::computed_roadmap_status;
        let statuses = vec![PhaseStatus::Done, PhaseStatus::InProgress];
        assert_eq!(
            computed_roadmap_status(&statuses),
            ("in-progress", "in-progress")
        );
    }

    #[test]
    fn computed_status_none_started() {
        use crate::templates::computed_roadmap_status;
        let statuses = vec![PhaseStatus::NotStarted, PhaseStatus::NotStarted];
        assert_eq!(
            computed_roadmap_status(&statuses),
            ("not-started", "not-started")
        );
    }

    #[test]
    fn computed_status_empty_phases() {
        use crate::templates::computed_roadmap_status;
        assert_eq!(computed_roadmap_status(&[]), ("not-started", "not-started"));
    }

    #[test]
    fn computed_status_done_and_wont_fix_is_done() {
        use crate::templates::computed_roadmap_status;
        let statuses = vec![PhaseStatus::Done, PhaseStatus::WontFix];
        assert_eq!(computed_roadmap_status(&statuses), ("done", "done"));
    }

    #[test]
    fn computed_status_in_progress_and_wont_fix_is_in_progress() {
        use crate::templates::computed_roadmap_status;
        let statuses = vec![PhaseStatus::InProgress, PhaseStatus::WontFix];
        assert_eq!(
            computed_roadmap_status(&statuses),
            ("in-progress", "in-progress")
        );
    }

    #[test]
    fn computed_status_only_wont_fix_is_done() {
        use crate::templates::computed_roadmap_status;
        let statuses = vec![PhaseStatus::WontFix];
        assert_eq!(computed_roadmap_status(&statuses), ("done", "done"));
    }

    #[tokio::test]
    async fn list_roadmaps_hal_json_includes_status() {
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
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let roadmaps = json["_embedded"]["roadmaps"].as_array().unwrap();
        assert_eq!(roadmaps[0]["status"], "in-progress");
        assert!(roadmaps[0]["last_changed"].is_string());
    }

    #[tokio::test]
    async fn list_roadmaps_html_includes_status_badge() {
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
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("badge-in-progress"));
        assert!(html.contains("Last Changed"));
    }

    #[tokio::test]
    async fn get_roadmap_hal_json_includes_status_and_last_changed() {
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
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "in-progress");
        assert!(json["last_changed"].is_string());
    }

    /// Create a setup with a roadmap that has priority set.
    fn setup_with_priority() -> (TempDir, AppState) {
        let dir = TempDir::new().unwrap();
        let mut store = rdm_store_fs::FsStore::new(dir.path());
        rdm_core::ops::init::init(&mut store).unwrap();
        rdm_core::ops::project::create_project(&mut store, "demo", "Demo Project").unwrap();
        rdm_core::ops::roadmap::create_roadmap(
            &mut store,
            rdm_core::ops::roadmap::CreateRoadmap {
                project: "demo",
                slug: "alpha",
                title: "Alpha Roadmap",
                body: None,
                priority: Some(rdm_core::model::Priority::High),
                ..Default::default()
            },
        )
        .unwrap();
        rdm_core::ops::phase::create_phase(
            &mut store,
            rdm_core::ops::phase::CreatePhase {
                project: "demo",
                roadmap: "alpha",
                slug: "first",
                title: "First Phase",
                number: Some(1),
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
    async fn list_roadmaps_html_includes_priority_badge() {
        let (_dir, state) = setup_with_priority();
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
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(
            html.contains("Priority"),
            "should have Priority column header"
        );
        assert!(
            html.contains("badge-high"),
            "should have priority badge class"
        );
        assert!(html.contains(">high<"), "should display priority text");
    }

    #[tokio::test]
    async fn list_roadmaps_html_no_priority_shows_dash() {
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
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(
            html.contains("Priority"),
            "should have Priority column header"
        );
        assert!(
            html.contains("\u{2014}"),
            "should show em-dash when no priority set"
        );
        // Verify no priority badge markup is rendered (CSS definitions contain
        // "badge-low" etc. so we check for the actual badge span pattern).
        assert!(
            !html.contains(r#"class="badge badge-low"#)
                && !html.contains(r#"class="badge badge-high"#),
            "should not render any priority badge span"
        );
    }

    #[tokio::test]
    async fn get_roadmap_html_no_priority_omits_badge() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/roadmaps/alpha")
                    .header("accept", "text/html")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        // Verify no priority badge markup is rendered (CSS definitions contain
        // "badge-low" etc. so we check for the actual badge span pattern).
        assert!(
            !html.contains(r#"class="badge badge-low"#)
                && !html.contains(r#"class="badge badge-medium"#)
                && !html.contains(r#"class="badge badge-high"#)
                && !html.contains(r#"class="badge badge-critical"#),
            "should not render any priority badge span"
        );
    }

    #[tokio::test]
    async fn get_roadmap_html_includes_priority_badge() {
        let (_dir, state) = setup_with_priority();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/roadmaps/alpha")
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
            html.contains("badge-high"),
            "should have priority badge class"
        );
        assert!(html.contains(">high<"), "should display priority text");
    }

    #[tokio::test]
    async fn get_roadmap_html_includes_status_badge_and_last_changed() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/roadmaps/alpha")
                    .header("accept", "text/html")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("badge-in-progress"));
        assert!(html.contains("<dt>Last changed</dt>"));
    }

    /// Setup with two roadmaps: `tagged` (tags: foo, bar) and `untagged-rm`.
    /// `tagged` has phase 1 with tag foo, phase 2 with no tag.
    fn setup_with_tags() -> (TempDir, AppState) {
        let dir = TempDir::new().unwrap();
        let mut store = rdm_store_fs::FsStore::new(dir.path());
        rdm_core::ops::init::init(&mut store).unwrap();
        rdm_core::ops::project::create_project(&mut store, "demo", "Demo").unwrap();
        rdm_core::ops::roadmap::create_roadmap(
            &mut store,
            rdm_core::ops::roadmap::CreateRoadmap {
                project: "demo",
                slug: "tagged",
                title: "Tagged Roadmap",
                body: None,
                priority: None,
                tags: Some(vec!["foo".to_string(), "bar".to_string()]),
            },
        )
        .unwrap();
        rdm_core::ops::phase::create_phase(
            &mut store,
            rdm_core::ops::phase::CreatePhase {
                project: "demo",
                roadmap: "tagged",
                slug: "first",
                title: "First",
                number: Some(1),
                body: None,
                tags: Some(vec!["foo".to_string()]),
                ..Default::default()
            },
        )
        .unwrap();
        rdm_core::ops::phase::create_phase(
            &mut store,
            rdm_core::ops::phase::CreatePhase {
                project: "demo",
                roadmap: "tagged",
                slug: "second",
                title: "Second",
                number: Some(2),
                ..Default::default()
            },
        )
        .unwrap();
        rdm_core::ops::roadmap::create_roadmap(
            &mut store,
            rdm_core::ops::roadmap::CreateRoadmap {
                project: "demo",
                slug: "untagged-rm",
                title: "Untagged Roadmap",
                ..Default::default()
            },
        )
        .unwrap();
        rdm_core::ops::phase::create_phase(
            &mut store,
            rdm_core::ops::phase::CreatePhase {
                project: "demo",
                roadmap: "untagged-rm",
                slug: "only",
                title: "Only",
                number: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        rdm_core::store::Store::commit(&mut store).unwrap();
        let state = AppState {
            plan_root: dir.path().to_path_buf(),
            quick_filters: vec![rdm_core::config::QuickFilter {
                label: "Foo".to_string(),
                tag: "foo".to_string(),
            }],
        };
        (dir, state)
    }

    #[tokio::test]
    async fn list_roadmaps_filter_by_tag_hal() {
        let (_dir, state) = setup_with_tags();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/roadmaps?tag=foo")
                    .header("accept", "application/hal+json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let roadmaps = json["_embedded"]["roadmaps"].as_array().unwrap();
        assert_eq!(roadmaps.len(), 1);
        assert_eq!(roadmaps[0]["slug"], "tagged");
        // tags appear in the summary
        let tags = roadmaps[0]["tags"].as_array().unwrap();
        assert_eq!(tags.len(), 2);
    }

    #[tokio::test]
    async fn list_roadmaps_unknown_tag_returns_empty() {
        let (_dir, state) = setup_with_tags();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/roadmaps?tag=does-not-exist")
                    .header("accept", "application/hal+json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["_embedded"]["roadmaps"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn get_roadmap_returns_tags_in_detail() {
        let (_dir, state) = setup_with_tags();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/roadmaps/tagged")
                    .header("accept", "application/hal+json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let tags = json["tags"].as_array().unwrap();
        assert!(tags.iter().any(|t| t == "foo"));
    }

    #[tokio::test]
    async fn create_roadmap_persists_tags() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::post("/projects/demo/roadmaps")
                    .header("accept", "application/hal+json")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(
                        r#"{"slug":"new","title":"New","tags":["x","y"]}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 201);
        let body = to_bytes(response.into_body(), 16384).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let tags = json["tags"].as_array().unwrap();
        assert_eq!(tags.len(), 2);
    }

    #[tokio::test]
    async fn update_roadmap_replaces_tags() {
        let (_dir, state) = setup_with_tags();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::patch("/projects/demo/roadmaps/tagged")
                    .header("accept", "application/hal+json")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(r#"{"tags":["only"]}"#))
                    .unwrap(),
            )
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
    async fn update_roadmap_clear_tags() {
        let (_dir, state) = setup_with_tags();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::patch("/projects/demo/roadmaps/tagged")
                    .header("accept", "application/hal+json")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(r#"{"clear_tags":true}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 16384).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.get("tags").is_none() || json["tags"].is_null());
    }

    #[tokio::test]
    async fn get_roadmap_html_tag_edit_form_renders_when_no_tags() {
        // `untagged-rm` in setup_with_tags has no tags; the editor should still
        // render with an empty input value and the Clear button.
        let (_dir, state) = setup_with_tags();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/roadmaps/untagged-rm")
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
            !html.contains("class=\"tags\""),
            "tags paragraph should be absent in:\n{html}"
        );
        assert!(html.contains("id=\"roadmap-tags-edit\""));
        assert!(
            html.contains("value=\"\""),
            "empty tag input value missing in:\n{html}"
        );
        assert!(html.contains("name=\"clear_tags\""));
    }

    #[tokio::test]
    async fn get_roadmap_html_includes_tag_edit_form() {
        let (_dir, state) = setup_with_tags();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/roadmaps/tagged")
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
        assert!(html.contains("action=\"/projects/demo/roadmaps/tagged\""));
        assert!(html.contains("name=\"tags\""));
        assert!(
            html.contains("value=\"foo, bar\""),
            "missing pre-populated tag input value in:\n{html}"
        );
        assert!(
            html.contains("name=\"clear_tags\"") && html.contains("value=\"true\""),
            "missing clear_tags submitter in:\n{html}"
        );
    }

    #[tokio::test]
    async fn update_roadmap_conflicting_tag_fields_returns_422() {
        let (_dir, state) = setup_with_tags();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::patch("/projects/demo/roadmaps/tagged")
                    .header("accept", "application/hal+json")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(
                        r#"{"tags":["x"],"clear_tags":true}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 422);
    }

    #[tokio::test]
    async fn patch_roadmap_clears_body_with_empty_string() {
        let (_dir, state) = setup();
        // First seed a body so we have something to clear.
        let app = build_router(state.clone());
        let response = app
            .oneshot(
                Request::patch("/projects/demo/roadmaps/alpha")
                    .header("accept", "application/hal+json")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(
                        r#"{"body":"Initial roadmap body."}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);

        // Now clear it.
        let app2 = build_router(state.clone());
        let response = app2
            .oneshot(
                Request::patch("/projects/demo/roadmaps/alpha")
                    .header("accept", "application/hal+json")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(r#"{"clear_body":true}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);

        let app3 = build_router(state);
        let response = app3
            .oneshot(
                Request::get("/projects/demo/roadmaps/alpha")
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
    async fn list_roadmaps_html_renders_quick_filter_chips() {
        let (_dir, state) = setup_with_tags();
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
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(
            html.contains("quick-filter-chips"),
            "should render chip nav"
        );
        assert!(
            html.contains(r#"href="/projects/demo/roadmaps?tag=foo""#),
            "chip href should target the page with ?tag=<tag>"
        );
        assert!(html.contains(">Foo</a>"));
        // No active highlight when no tag selected.
        assert!(!html.contains(r#"class="quick-filter-chip active""#));
        // No "All" link without active filter.
        assert!(!html.contains(r#"class="quick-filter-clear""#));
    }

    #[tokio::test]
    async fn list_roadmaps_html_active_chip_highlighted() {
        let (_dir, state) = setup_with_tags();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/roadmaps?tag=foo")
                    .header("accept", "text/html")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains(r#"class="quick-filter-chip active""#));
        assert!(html.contains(r#"aria-current="true""#));
        assert!(
            html.contains(r#"class="quick-filter-clear""#),
            "should render All link"
        );
    }

    #[tokio::test]
    async fn get_roadmap_html_includes_body_edit_form() {
        let (_dir, state) = setup();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/roadmaps/alpha")
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
        assert!(html.contains("action=\"/projects/demo/roadmaps/alpha\""));
        assert!(html.contains("<textarea"));
        assert!(html.contains("name=\"body\""));
    }

    #[tokio::test]
    async fn get_roadmap_html_filters_phases_by_tag() {
        let (_dir, state) = setup_with_tags();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::get("/projects/demo/roadmaps/tagged?tag=foo")
                    .header("accept", "text/html")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        // First phase has tag foo and should appear; second has no tags and should not.
        assert!(html.contains("phase-1-first"));
        assert!(!html.contains("phase-2-second"));
    }

    // -- reviews section & open-review counts --

    use rdm_core::model::{Anchor, CommentDoc, CommentDocKind, ReviewTarget, Verdict};

    /// Text-quote anchor with empty context (fine for unique quotes).
    fn tq(quote: &str) -> Anchor {
        Anchor::TextQuote {
            quote: quote.to_string(),
            prefix: String::new(),
            suffix: String::new(),
        }
    }

    /// Authors a submitted review with the given comments via core ops;
    /// returns the review id.
    fn author_submitted_review(
        state: &AppState,
        target: ReviewTarget,
        comments: &[(Option<Anchor>, Option<CommentDoc>, &str)],
    ) -> String {
        let mut store = state.store();
        let doc = rdm_core::ops::reviews::create_review(
            &mut store,
            rdm_core::ops::reviews::CreateReview {
                project: "demo",
                author: "reviewer",
                target,
                body: Some("Summary."),
            },
        )
        .unwrap();
        let id = doc.frontmatter.id.clone();
        for (anchor, doc_scope, body) in comments {
            rdm_core::ops::reviews::add_comment(
                &mut store,
                rdm_core::ops::reviews::AddComment {
                    project: "demo",
                    review_id: &id,
                    body,
                    doc: doc_scope.clone(),
                    anchor: anchor.clone(),
                },
            )
            .unwrap();
        }
        rdm_core::ops::reviews::submit_review(&mut store, "demo", &id, Some(Verdict::Comment))
            .unwrap();
        rdm_core::store::Store::commit(&mut store).unwrap();
        id
    }

    async fn get_html(state: &AppState, uri: &str) -> String {
        let response = build_router(state.clone())
            .oneshot(
                Request::get(uri)
                    .header("accept", "text/html")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 262144).await.unwrap();
        String::from_utf8(body.to_vec()).unwrap()
    }

    /// A `setup()` variant whose roadmap has body text to anchor into.
    fn setup_with_roadmap_body() -> (TempDir, AppState) {
        let (dir, state) = setup();
        let mut store = state.store();
        rdm_core::ops::roadmap::update_roadmap(
            &mut store,
            "demo",
            "alpha",
            rdm_core::ops::BodyUpdate::Set(
                "Intro paragraph. The roadmap span here. Outro.\n".to_string(),
            ),
            rdm_core::ops::PriorityUpdate::Keep,
            rdm_core::ops::TagsUpdate::Keep,
            rdm_core::ops::TitleUpdate::Keep,
        )
        .unwrap();
        rdm_core::store::Store::commit(&mut store).unwrap();
        (dir, state)
    }

    #[tokio::test]
    async fn get_roadmap_html_renders_reviews_section_with_highlight() {
        let (_dir, state) = setup_with_roadmap_body();
        let id = author_submitted_review(
            &state,
            ReviewTarget::Roadmap {
                roadmap: "alpha".to_string(),
            },
            &[(Some(tq("roadmap span")), None, "About this span.")],
        );
        let html = get_html(&state, "/projects/demo/roadmaps/alpha").await;
        assert!(html.contains(r#"<section id="reviews""#), "got: {html}");
        assert!(html.contains(r#"badge-verdict-comment">Comment</span>"#));
        assert!(html.contains(&format!(
            r#"<mark class="rdm-anchor" data-rdm-anchor="{id}-c1">roadmap span</mark>"#
        )));
    }

    #[tokio::test]
    async fn get_roadmap_html_doc_scoped_comment_links_through_to_phase() {
        let (_dir, state) = setup_with_roadmap_body();
        let id = author_submitted_review(
            &state,
            ReviewTarget::Roadmap {
                roadmap: "alpha".to_string(),
            },
            &[(
                None,
                Some(CommentDoc {
                    kind: CommentDocKind::Phase,
                    stem: "phase-2-second".to_string(),
                }),
                "Scoped into the phase.",
            )],
        );
        let html = get_html(&state, "/projects/demo/roadmaps/alpha").await;
        assert!(html.contains("Scoped into the phase."), "got: {html}");
        assert!(
            html.contains("View in Phase 2: Second Phase"),
            "cross-link label must name the phase: {html}"
        );
        assert!(html.contains(&format!(
            r#"href="/projects/demo/roadmaps/alpha/phases/phase-2-second#comment-{id}-c1""#
        )));
        // Doc-scoped comments never highlight in the roadmap body.
        assert!(!html.contains("<mark"), "got: {html}");
    }

    #[tokio::test]
    async fn list_roadmaps_html_shows_rolled_up_counts_matching_index() {
        let (_dir, state) = setup();
        // One roadmap-targeted review with 1 open comment, one
        // phase-targeted review with 2 — the roadmap row must roll them up
        // to 2 open reviews / 3 open comments, exactly like INDEX.md.
        author_submitted_review(
            &state,
            ReviewTarget::Roadmap {
                roadmap: "alpha".to_string(),
            },
            &[(None, None, "Roadmap note.")],
        );
        author_submitted_review(
            &state,
            ReviewTarget::Phase {
                roadmap: "alpha".to_string(),
                stem: "phase-1-first".to_string(),
            },
            &[(None, None, "First."), (None, None, "Second.")],
        );

        let html = get_html(&state, "/projects/demo/roadmaps?show_completed=true").await;
        assert!(
            html.contains(
                r#"<a href="/projects/demo/roadmaps/alpha#reviews">2 open (3 comments)</a>"#
            ),
            "got: {html}"
        );

        // INDEX.md, regenerated over the same fixture, must agree.
        let mut store = state.store();
        rdm_core::ops::index::generate_project_index(&mut store, "demo").unwrap();
        let index_md =
            rdm_core::store::Store::read(&store, &rdm_core::paths::project_index_path("demo"))
                .unwrap();
        let alpha_row = index_md
            .lines()
            .find(|l| l.contains("[alpha]"))
            .expect("alpha row present in INDEX.md");
        let cells: Vec<&str> = alpha_row.split('|').map(str::trim).collect();
        assert!(
            cells.contains(&"2") && cells.contains(&"3"),
            "INDEX.md must report the same 2 open reviews / 3 open comments: {alpha_row}"
        );
    }

    #[tokio::test]
    async fn list_roadmaps_html_dash_when_no_open_reviews() {
        let (_dir, state) = setup();
        let html = get_html(&state, "/projects/demo/roadmaps").await;
        assert!(
            html.contains(r#"<th scope="col">Reviews</th>"#),
            "got: {html}"
        );
        assert!(
            !html.contains("#reviews\">"),
            "no count link without open reviews"
        );
    }
}
