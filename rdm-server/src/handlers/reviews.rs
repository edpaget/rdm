//! Review REST endpoints: list, create, show, comment, submit, transition,
//! and delete document reviews.
//!
//! Unlike the task/roadmap/phase handlers, these routes are **API-only**
//! (HAL+JSON in, HAL+JSON out; errors as RFC 9457 Problem+JSON) — the web UI
//! over them arrives in a later phase, so there is no `Accept`-negotiated
//! HTML rendering here.
//!
//! All lifecycle rules (draft-only structural edits, submitted-only
//! resolutions, verdict-gated submit, resolved-comments-required
//! `addressed`) are enforced by `rdm_core::ops::reviews` — this layer only
//! maps requests onto core ops and core errors onto Problem+JSON.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use rdm_core::anchor::ResolvedComment;
use rdm_core::document::Document;
use rdm_core::json::ReviewJson;
use rdm_core::model::{
    Anchor, CommentDoc, Review, ReviewCommentStatus, ReviewState, ReviewTarget, Verdict,
};
use rdm_core::ops::BodyUpdate;
use rdm_core::ops::reviews::{
    AddComment, AnchorUpdate, CreateReview, DocUpdate, ReviewFilter, ReviewTransition,
    UpdateComment,
};

use crate::error::{
    AppError, json_rejection_response, problem_detail_into_response, validation_error,
};
use crate::extract::{hal_created_response, hal_response};
use crate::hal::{HalLink, HalResource};
use crate::problem::ProblemDetail;
use crate::state::AppState;

/// Maps a core error to its Problem+JSON response (reviews are API-only, so
/// there is no HTML error-page branch).
fn core_error(err: rdm_core::error::Error) -> Response {
    AppError(err).into_response()
}

/// Builds a `400 Bad Request` Problem+JSON response with the given detail.
fn bad_request(detail: String) -> Response {
    problem_detail_into_response(ProblemDetail {
        problem_type: "about:blank".to_string(),
        title: "Bad Request".to_string(),
        status: 400,
        detail: Some(detail),
        instance: None,
    })
}

/// Deserializes a field that must distinguish "key absent" (keep) from
/// "key present with `null`" (clear) from "key present with a value" (set).
///
/// Pair with `#[serde(default, deserialize_with = "tri_state")]` on an
/// `Option<Option<T>>` field: `default` supplies the outer `None` (keep)
/// when the key is missing; when the key *is* present this fn runs and wraps
/// whatever `Option<T>` comes out (`None` for `null`, `Some(v)` for a value)
/// in an outer `Some`, so `null` becomes `Some(None)` (clear) instead of
/// collapsing into the same `None` a missing key produces.
fn tri_state<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: serde::Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// Builds the `target` link relation for a review: the reviewed roadmap,
/// phase, or task resource.
fn target_link(project: &str, target: &ReviewTarget) -> HalLink {
    HalLink::new(crate::review_views::target_detail_href(project, target))
}

/// Wraps a full review detail (metadata, summary, comments with resolution)
/// as a HAL resource with `self`, `project`, and `target` links.
fn review_resource(
    project: &str,
    id: &str,
    doc: &Document<Review>,
    resolutions: &[ResolvedComment],
) -> HalResource<ReviewJson> {
    HalResource::new(
        rdm_core::json::review_to_json(id, doc, resolutions),
        format!("/projects/{project}/reviews/{id}"),
    )
    .with_link("project", HalLink::new(format!("/projects/{project}")))
    .with_link("target", target_link(project, &doc.frontmatter.target))
}

/// Empty data for the reviews collection wrapper.
#[derive(Serialize)]
struct ReviewsCollection {}

/// One review in the list response: metadata only.
///
/// Deliberately lighter than the detail shape: the summary body, the
/// comments, and each comment's anchor resolution are omitted — resolving
/// anchors costs store reads per comment, so only
/// `GET /projects/:project/reviews/:id` pays for the resolution pass.
/// Clients follow an item's `self` link for the full detail.
#[derive(Serialize)]
struct ReviewListItem {
    /// Review id (also the file stem under `reviews/`).
    id: String,
    /// Who authored the review.
    author: String,
    /// The plan item under review (tagged on `kind`).
    target: ReviewTarget,
    /// Lifecycle state.
    state: ReviewState,
    /// Verdict stamped on submit; absent on drafts.
    #[serde(skip_serializing_if = "Option::is_none")]
    verdict: Option<Verdict>,
    /// When the review was started.
    created: DateTime<Utc>,
    /// When the review was submitted; absent on drafts.
    #[serde(skip_serializing_if = "Option::is_none")]
    submitted: Option<DateTime<Utc>>,
    /// Plan-repo HEAD when the review started, if recorded.
    #[serde(skip_serializing_if = "Option::is_none")]
    created_commit: Option<String>,
    /// Number of comments on the review.
    comment_count: usize,
}

impl ReviewListItem {
    /// Builds a list item from a loaded review document.
    fn from_doc(id: &str, doc: &Document<Review>) -> Self {
        let fm = &doc.frontmatter;
        ReviewListItem {
            id: id.to_string(),
            author: fm.author.clone(),
            target: fm.target.clone(),
            state: fm.state,
            verdict: fm.verdict,
            created: fm.created,
            submitted: fm.submitted,
            created_commit: fm.created_commit.clone(),
            comment_count: fm.comments.len(),
        }
    }
}

/// Query parameters for filtering the review list.
#[derive(Debug, Deserialize, Default)]
pub struct ReviewFilters {
    /// Keep only reviews of this target: `roadmap/<slug>`,
    /// `phase/<roadmap-slug>/<stem-or-number>`, or `task/<slug>`.
    pub on: Option<String>,
    /// Keep only reviews in this state (draft, submitted, addressed,
    /// dismissed).
    pub state: Option<String>,
    /// Keep only reviews with this verdict (approve, request-changes,
    /// comment).
    pub verdict: Option<String>,
    /// Keep only reviews by this author.
    pub author: Option<String>,
}

/// `GET /projects/:project/reviews` — list reviews with optional filters.
pub async fn list_reviews(
    State(state): State<AppState>,
    Path(project): Path<String>,
    Query(filters): Query<ReviewFilters>,
) -> Result<Response, Response> {
    let store = state.store();
    let reviews = rdm_core::ops::reviews::list_reviews(&store, &project).map_err(core_error)?;

    let target = match &filters.on {
        Some(reference) => Some(
            rdm_core::ops::reviews::parse_review_target_ref(&store, &project, reference)
                .map_err(core_error)?,
        ),
        None => None,
    };
    let state_filter = match &filters.state {
        Some(s) => Some(s.parse::<ReviewState>().map_err(|_| {
            bad_request(format!(
                "invalid state filter: '{s}' (expected draft, submitted, addressed, or dismissed)"
            ))
        })?),
        None => None,
    };
    let verdict_filter = match &filters.verdict {
        Some(v) => Some(v.parse::<Verdict>().map_err(|_| {
            bad_request(format!(
                "invalid verdict filter: '{v}' (expected approve, request-changes, or comment)"
            ))
        })?),
        None => None,
    };

    let filtered = rdm_core::ops::reviews::filter_reviews(
        reviews,
        &ReviewFilter {
            target,
            state: state_filter,
            verdict: verdict_filter,
            author: filters.author.clone(),
            ..Default::default()
        },
    );

    // Deliberately no anchor-resolution pass here: the list shape is
    // metadata-only (see `ReviewListItem`); only the detail route resolves.
    let mut embedded = Vec::new();
    for (id, doc) in &filtered {
        let item = HalResource::new(
            ReviewListItem::from_doc(id, doc),
            format!("/projects/{project}/reviews/{id}"),
        )
        .with_link("project", HalLink::new(format!("/projects/{project}")))
        .with_link("target", target_link(&project, &doc.frontmatter.target));
        embedded.push(serde_json::to_value(&item).expect("list item serialization cannot fail"));
    }

    let resource = HalResource::new(ReviewsCollection {}, format!("/projects/{project}/reviews"))
        .with_link("project", HalLink::new(format!("/projects/{project}")))
        .with_embedded("reviews", embedded);
    Ok(hal_response(resource))
}

/// Request body for `POST /projects/:project/reviews`.
#[derive(Deserialize)]
pub struct CreateReviewRequest {
    /// The item under review: `roadmap/<slug>`,
    /// `phase/<roadmap-slug>/<stem-or-number>`, or `task/<slug>`.
    target: String,
    /// Review author. Defaults to `"api"` when omitted or blank — the
    /// server process's OS user does not represent the HTTP caller, so the
    /// CLI's `$USER` fallback is deliberately not mirrored here.
    author: Option<String>,
    /// Initial summary body for the draft (Markdown).
    summary: Option<String>,
}

/// `POST /projects/:project/reviews` — start a new draft review.
pub async fn create_review(
    State(state): State<AppState>,
    Path(project): Path<String>,
    payload: Result<axum::Json<CreateReviewRequest>, JsonRejection>,
) -> Result<Response, Response> {
    let axum::Json(req) = payload.map_err(json_rejection_response)?;
    let mut store = state.store();
    let target = rdm_core::ops::reviews::parse_review_target_ref(&store, &project, &req.target)
        .map_err(core_error)?;
    let author = req
        .author
        .filter(|a| !a.trim().is_empty())
        .unwrap_or_else(|| "api".to_string());

    let doc = rdm_core::ops::mutate(&mut store, &project, |s| {
        rdm_core::ops::reviews::create_review(
            s,
            CreateReview {
                project: &project,
                author: &author,
                target: target.clone(),
                body: req.summary.as_deref(),
            },
        )
    })
    .map_err(core_error)?;

    let id = doc.frontmatter.id.clone();
    let location = format!("/projects/{project}/reviews/{id}");
    // A freshly created review has no comments, so the resolution slice is
    // empty by construction.
    Ok(hal_created_response(
        review_resource(&project, &id, &doc, &[]),
        &location,
    ))
}

/// `GET /projects/:project/reviews/:review_id` — full review detail,
/// including each comment's anchor resolution (resolved / drifted /
/// unresolved, with the quoted text and which body the range indexes) so
/// clients can highlight without extra calls.
pub async fn get_review(
    State(state): State<AppState>,
    Path((project, review_id)): Path<(String, String)>,
) -> Result<Response, Response> {
    let store = state.store();
    let doc =
        rdm_core::ops::reviews::get_review(&store, &project, &review_id).map_err(core_error)?;
    let resolutions = rdm_core::anchor::resolve_comments(&store, &project, &doc.frontmatter);
    Ok(hal_response(review_resource(
        &project,
        &review_id,
        &doc,
        &resolutions,
    )))
}

/// Request body for `POST /projects/:project/reviews/:review_id/comments`.
#[derive(Deserialize)]
pub struct AddCommentRequest {
    /// The comment text (Markdown). Must not be blank.
    body: String,
    /// Optional document scope (roadmap reviews only): points the comment
    /// at one of the roadmap's phases (`{"kind": "phase", "stem": ...}`).
    doc: Option<CommentDoc>,
    /// Optional anchor into the target's body (tagged on `anchor_type`).
    /// Unrecognized anchor types are stored and echoed back untouched.
    anchor: Option<Anchor>,
}

/// `POST /projects/:project/reviews/:review_id/comments` — add a comment to
/// a draft review.
pub async fn add_comment(
    State(state): State<AppState>,
    Path((project, review_id)): Path<(String, String)>,
    payload: Result<axum::Json<AddCommentRequest>, JsonRejection>,
) -> Result<Response, Response> {
    let axum::Json(req) = payload.map_err(json_rejection_response)?;
    if req.body.trim().is_empty() {
        return Err(validation_error(
            "comment body must not be empty".to_string(),
        ));
    }
    let mut store = state.store();
    let doc = rdm_core::ops::mutate(&mut store, &project, |s| {
        rdm_core::ops::reviews::add_comment(
            s,
            AddComment {
                project: &project,
                review_id: &review_id,
                body: &req.body,
                doc: req.doc,
                anchor: req.anchor,
            },
        )
    })
    .map_err(core_error)?;

    let comment_id = doc
        .frontmatter
        .comments
        .last()
        .map(|c| c.id)
        .unwrap_or_default();
    let location = format!("/projects/{project}/reviews/{review_id}/comments/{comment_id}");
    let resolutions = rdm_core::anchor::resolve_comments(&store, &project, &doc.frontmatter);
    Ok(hal_created_response(
        review_resource(&project, &review_id, &doc, &resolutions),
        &location,
    ))
}

/// Request body for
/// `PATCH /projects/:project/reviews/:review_id/comments/:comment_id`.
///
/// Structural fields (`body`, `anchor`, `doc`) are draft-only; resolution
/// fields (`status`, `applied_commit`, `reply`) are submitted-only. Which
/// set is legal given the review's current state is enforced by core, not
/// re-implemented here.
///
/// `anchor` and `doc` are tri-state: omit the key to keep the current
/// value, send `null` to clear it, or send a value to replace it.
#[derive(Deserialize, Default)]
pub struct UpdateCommentRequest {
    /// New comment text; omit to keep the existing body. Draft-only.
    body: Option<String>,
    /// Anchor update (omit = keep, `null` = clear, value = set). Draft-only.
    #[serde(default, deserialize_with = "tri_state")]
    anchor: Option<Option<Anchor>>,
    /// Document-scope update (omit = keep, `null` = clear, value = set).
    /// Draft-only.
    #[serde(default, deserialize_with = "tri_state")]
    doc: Option<Option<CommentDoc>>,
    /// New resolution status (addressed or wont-fix); omit to keep.
    /// Submitted-only.
    status: Option<ReviewCommentStatus>,
    /// Commit SHA recorded when the comment was addressed; omit to keep.
    /// Submitted-only.
    applied_commit: Option<String>,
    /// Agent reply note; omit to keep. Submitted-only.
    reply: Option<String>,
}

/// `PATCH /projects/:project/reviews/:review_id/comments/:comment_id` —
/// update one comment (structure while draft, resolution once submitted).
pub async fn update_comment(
    State(state): State<AppState>,
    Path((project, review_id, comment_id)): Path<(String, String, String)>,
    payload: Result<axum::Json<UpdateCommentRequest>, JsonRejection>,
) -> Result<Response, Response> {
    // Parsed by hand (rather than a typed `Path<u32>` segment) so a
    // non-numeric id yields Problem+JSON instead of axum's plain-text
    // rejection.
    let comment_id: u32 = comment_id.parse().map_err(|_| {
        bad_request(format!(
            "invalid comment id '{comment_id}' — expected the comment's numeric id \
             (listed in the review's comments)"
        ))
    })?;
    let axum::Json(req) = payload.map_err(json_rejection_response)?;
    // Same guard as add_comment: a blank replacement body would silently
    // empty the comment.
    if req.body.as_deref().is_some_and(|b| b.trim().is_empty()) {
        return Err(validation_error(
            "comment body must not be empty".to_string(),
        ));
    }

    let anchor = match req.anchor {
        None => AnchorUpdate::Keep,
        Some(None) => AnchorUpdate::Clear,
        Some(Some(a)) => AnchorUpdate::Set(a),
    };
    let doc_update = match req.doc {
        None => DocUpdate::Keep,
        Some(None) => DocUpdate::Clear,
        Some(Some(d)) => DocUpdate::Set(d),
    };

    let mut store = state.store();
    let doc = rdm_core::ops::mutate(&mut store, &project, |s| {
        rdm_core::ops::reviews::update_comment(
            s,
            UpdateComment {
                project: &project,
                review_id: &review_id,
                comment_id,
                body: req.body.as_deref(),
                anchor,
                doc: doc_update,
                status: req.status,
                applied_commit: req.applied_commit.as_deref(),
                reply: req.reply.as_deref(),
            },
        )
    })
    .map_err(core_error)?;

    let resolutions = rdm_core::anchor::resolve_comments(&store, &project, &doc.frontmatter);
    Ok(hal_response(review_resource(
        &project,
        &review_id,
        &doc,
        &resolutions,
    )))
}

/// Request body for `POST /projects/:project/reviews/:review_id/submit`.
#[derive(Deserialize, Default)]
pub struct SubmitReviewRequest {
    /// Overall verdict (approve, request-changes, or comment). Omitting it
    /// surfaces core's "cannot be submitted without a verdict" error.
    verdict: Option<Verdict>,
    /// Replaces the review's summary body before submitting, if given.
    ///
    /// A blank (empty or whitespace-only) value is rejected with a 422
    /// rather than treated as "no summary": silently keeping the old
    /// summary would gaslight the caller, and passing it through would
    /// either clobber a non-empty summary with whitespace or produce a
    /// misleading "no comments and no summary" error. Omit the field to
    /// keep the existing summary. (Mirrors the CLI's blank `--body`
    /// handling in spirit.)
    summary: Option<String>,
}

/// `POST /projects/:project/reviews/:review_id/submit` — submit a draft
/// review with a verdict, locking its comment structure.
pub async fn submit_review(
    State(state): State<AppState>,
    Path((project, review_id)): Path<(String, String)>,
    payload: Result<axum::Json<SubmitReviewRequest>, JsonRejection>,
) -> Result<Response, Response> {
    let axum::Json(req) = payload.map_err(json_rejection_response)?;
    // A blank summary is rejected outright (see `SubmitReviewRequest`):
    // never passed to `set_summary`, so it can neither clobber an existing
    // summary nor fall through to a misleading "no summary" submit error.
    if req.summary.as_deref().is_some_and(|s| s.trim().is_empty()) {
        return Err(validation_error(
            "summary must not be blank — pass non-empty text, or omit the field to keep \
             the existing summary"
                .to_string(),
        ));
    }
    let mut store = state.store();
    let doc = rdm_core::ops::mutate(&mut store, &project, |s| {
        if let Some(summary) = &req.summary {
            rdm_core::ops::reviews::set_summary(
                s,
                &project,
                &review_id,
                BodyUpdate::Set(summary.clone()),
            )?;
        }
        rdm_core::ops::reviews::submit_review(s, &project, &review_id, req.verdict)
    })
    .map_err(core_error)?;

    let resolutions = rdm_core::anchor::resolve_comments(&store, &project, &doc.frontmatter);
    Ok(hal_response(review_resource(
        &project,
        &review_id,
        &doc,
        &resolutions,
    )))
}

/// Request body for `PATCH /projects/:project/reviews/:review_id`.
#[derive(Deserialize)]
pub struct UpdateReviewRequest {
    /// Terminal state to transition to: `addressed` or `dismissed`.
    state: ReviewState,
}

/// `PATCH /projects/:project/reviews/:review_id` — transition a review to a
/// terminal state (`addressed` or `dismissed`).
pub async fn update_review(
    State(state): State<AppState>,
    Path((project, review_id)): Path<(String, String)>,
    payload: Result<axum::Json<UpdateReviewRequest>, JsonRejection>,
) -> Result<Response, Response> {
    let axum::Json(req) = payload.map_err(json_rejection_response)?;
    let transition = match req.state {
        ReviewState::Addressed => ReviewTransition::Addressed,
        ReviewState::Dismissed => ReviewTransition::Dismissed,
        other => {
            return Err(validation_error(format!(
                "'{other}' is not a valid transition — pass 'addressed' or 'dismissed' \
                 (submission goes through POST .../submit, and nothing returns to draft)"
            )));
        }
    };
    let mut store = state.store();
    let doc = rdm_core::ops::mutate(&mut store, &project, |s| {
        rdm_core::ops::reviews::update_review(s, &project, &review_id, transition)
    })
    .map_err(core_error)?;

    let resolutions = rdm_core::anchor::resolve_comments(&store, &project, &doc.frontmatter);
    Ok(hal_response(review_resource(
        &project,
        &review_id,
        &doc,
        &resolutions,
    )))
}

/// `DELETE /projects/:project/reviews/:review_id` — delete a draft review.
///
/// Submitted and terminal reviews are part of the record and return `409
/// Conflict`; there is no force override at the API layer.
pub async fn delete_review(
    State(state): State<AppState>,
    Path((project, review_id)): Path<(String, String)>,
) -> Result<Response, Response> {
    let mut store = state.store();
    rdm_core::ops::mutate(&mut store, &project, |s| {
        rdm_core::ops::reviews::delete_review(s, &project, &review_id, false)
    })
    .map_err(core_error)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;
    use axum::http::Request;
    use tempfile::TempDir;
    use tower::ServiceExt;

    use crate::router::build_router;
    use crate::state::AppState;

    /// A plan repo with project `demo`, roadmap `alpha` (one phase,
    /// `phase-1-one`), and task `fix-login`.
    fn setup() -> (TempDir, AppState) {
        let dir = TempDir::new().unwrap();
        let mut store = rdm_store_fs::FsStore::new(dir.path());
        rdm_core::ops::init::init(&mut store).unwrap();
        rdm_core::ops::project::create_project(&mut store, "demo", "Demo").unwrap();
        rdm_core::ops::roadmap::create_roadmap(
            &mut store,
            rdm_core::ops::roadmap::CreateRoadmap {
                project: "demo",
                slug: "alpha",
                title: "Alpha",
                body: Some("Roadmap body with a roadmap span here.\n"),
                ..Default::default()
            },
        )
        .unwrap();
        rdm_core::ops::phase::create_phase(
            &mut store,
            rdm_core::ops::phase::CreatePhase {
                project: "demo",
                roadmap: "alpha",
                slug: "one",
                title: "One",
                number: Some(1),
                body: Some("Phase body with a phase span here.\n"),
                ..Default::default()
            },
        )
        .unwrap();
        rdm_core::ops::task::create_task(
            &mut store,
            rdm_core::ops::task::CreateTask {
                project: "demo",
                slug: "fix-login",
                title: "Fix login",
                body: Some("Task body with the quoted span here.\n"),
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

    fn get_req(uri: &str) -> Request<axum::body::Body> {
        Request::get(uri)
            .header("accept", "application/hal+json")
            .body(axum::body::Body::empty())
            .unwrap()
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

    fn delete_req(uri: &str) -> Request<axum::body::Body> {
        Request::delete(uri)
            .header("accept", "application/hal+json")
            .body(axum::body::Body::empty())
            .unwrap()
    }

    /// Sends one request through a fresh router over the shared state.
    async fn send(state: &AppState, req: Request<axum::body::Body>) -> axum::response::Response {
        build_router(state.clone()).oneshot(req).await.unwrap()
    }

    async fn json_body(response: axum::response::Response) -> serde_json::Value {
        let body = to_bytes(response.into_body(), 262144).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    /// Creates a draft review via the API and returns its id.
    async fn create_draft(state: &AppState, target: &str, author: &str) -> String {
        let body = serde_json::json!({ "target": target, "author": author }).to_string();
        let response = send(state, post_json("/projects/demo/reviews", &body)).await;
        assert_eq!(response.status(), 201);
        json_body(response).await["id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    /// Adds a comment via the API, asserting success.
    async fn add_comment_ok(state: &AppState, review_id: &str, body: serde_json::Value) {
        let uri = format!("/projects/demo/reviews/{review_id}/comments");
        let response = send(state, post_json(&uri, &body.to_string())).await;
        assert_eq!(response.status(), 201);
    }

    /// Submits a review via the API, asserting success.
    async fn submit_ok(state: &AppState, review_id: &str, verdict: &str) {
        let uri = format!("/projects/demo/reviews/{review_id}/submit");
        let body = serde_json::json!({ "verdict": verdict, "summary": "Overall summary." });
        let response = send(state, post_json(&uri, &body.to_string())).await;
        assert_eq!(response.status(), 200);
    }

    // -- list --

    #[tokio::test]
    async fn list_reviews_empty_returns_empty_collection() {
        let (_dir, state) = setup();
        let response = send(&state, get_req("/projects/demo/reviews")).await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/hal+json"
        );
        let json = json_body(response).await;
        assert_eq!(json["_links"]["self"]["href"], "/projects/demo/reviews");
        assert!(
            json.get("_embedded").is_none()
                || json["_embedded"]["reviews"].as_array().unwrap().is_empty()
        );
    }

    #[tokio::test]
    async fn list_reviews_returns_summaries_without_resolution() {
        let (_dir, state) = setup();
        let id = create_draft(&state, "task/fix-login", "ed").await;
        add_comment_ok(
            &state,
            &id,
            serde_json::json!({
                "body": "Tighten this.",
                "anchor": {
                    "anchor_type": "text-quote",
                    "quote": "quoted span",
                    "prefix": "the ",
                    "suffix": " here"
                }
            }),
        )
        .await;
        create_draft(&state, "roadmap/alpha", "bot").await;

        let response = send(&state, get_req("/projects/demo/reviews")).await;
        assert_eq!(response.status(), 200);
        let json = json_body(response).await;
        let items = json["_embedded"]["reviews"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        let item = items
            .iter()
            .find(|i| i["id"] == serde_json::json!(id))
            .expect("created review present in list");
        // Metadata-only list shape: no body, no comments array, and — the
        // deliberate design point — no per-comment resolution pass.
        assert!(item.get("body").is_none());
        assert!(item.get("comments").is_none());
        assert_eq!(item["comment_count"], 1);
        assert_eq!(item["state"], "draft");
        assert_eq!(item["author"], "ed");
        assert_eq!(item["target"]["kind"], "task");
        assert_eq!(
            item["_links"]["self"]["href"],
            format!("/projects/demo/reviews/{id}")
        );
        assert_eq!(
            item["_links"]["target"]["href"],
            "/projects/demo/tasks/fix-login"
        );
        assert_eq!(item["_links"]["project"]["href"], "/projects/demo");
        assert_eq!(json["_links"]["project"]["href"], "/projects/demo");
        let resolution_leak = serde_json::to_string(item).unwrap();
        assert!(
            !resolution_leak.contains("resolution"),
            "list items must not carry resolution: {resolution_leak}"
        );
    }

    #[tokio::test]
    async fn list_reviews_filter_by_on() {
        let (_dir, state) = setup();
        create_draft(&state, "task/fix-login", "ed").await;
        create_draft(&state, "roadmap/alpha", "ed").await;

        let response = send(&state, get_req("/projects/demo/reviews?on=roadmap/alpha")).await;
        assert_eq!(response.status(), 200);
        let json = json_body(response).await;
        let items = json["_embedded"]["reviews"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["target"]["kind"], "roadmap");
    }

    #[tokio::test]
    async fn list_reviews_filter_by_state_and_verdict() {
        let (_dir, state) = setup();
        let submitted = create_draft(&state, "task/fix-login", "ed").await;
        submit_ok(&state, &submitted, "request-changes").await;
        create_draft(&state, "roadmap/alpha", "ed").await;

        let response = send(&state, get_req("/projects/demo/reviews?state=submitted")).await;
        let json = json_body(response).await;
        let items = json["_embedded"]["reviews"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["id"], serde_json::json!(submitted));

        let response = send(
            &state,
            get_req("/projects/demo/reviews?verdict=request-changes"),
        )
        .await;
        let json = json_body(response).await;
        let items = json["_embedded"]["reviews"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["verdict"], "request-changes");

        // A draft with no verdict never matches a verdict filter.
        let response = send(&state, get_req("/projects/demo/reviews?verdict=approve")).await;
        let json = json_body(response).await;
        assert!(
            json.get("_embedded").is_none()
                || json["_embedded"]["reviews"].as_array().unwrap().is_empty()
        );
    }

    #[tokio::test]
    async fn list_reviews_filter_by_author() {
        let (_dir, state) = setup();
        create_draft(&state, "task/fix-login", "ed").await;
        create_draft(&state, "roadmap/alpha", "bot").await;

        let response = send(&state, get_req("/projects/demo/reviews?author=bot")).await;
        let json = json_body(response).await;
        let items = json["_embedded"]["reviews"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["author"], "bot");
    }

    #[tokio::test]
    async fn list_reviews_invalid_state_returns_400() {
        let (_dir, state) = setup();
        let response = send(&state, get_req("/projects/demo/reviews?state=bogus")).await;
        assert_eq!(response.status(), 400);
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/problem+json"
        );
        let json = json_body(response).await;
        let detail = json["detail"].as_str().unwrap();
        assert!(detail.contains("bogus"));
        assert!(
            detail.contains("draft"),
            "must list accepted values: {detail}"
        );
    }

    #[tokio::test]
    async fn list_reviews_invalid_verdict_returns_400() {
        let (_dir, state) = setup();
        let response = send(&state, get_req("/projects/demo/reviews?verdict=bogus")).await;
        assert_eq!(response.status(), 400);
        let json = json_body(response).await;
        assert!(json["detail"].as_str().unwrap().contains("approve"));
    }

    #[tokio::test]
    async fn list_reviews_invalid_on_returns_400() {
        let (_dir, state) = setup();
        let response = send(&state, get_req("/projects/demo/reviews?on=widget/alpha")).await;
        assert_eq!(response.status(), 400);
        let json = json_body(response).await;
        assert!(
            json["detail"].as_str().unwrap().contains("roadmap/<slug>"),
            "must show accepted forms"
        );
    }

    #[tokio::test]
    async fn list_reviews_project_not_found_returns_404() {
        let (_dir, state) = setup();
        let response = send(&state, get_req("/projects/nonexistent/reviews")).await;
        assert_eq!(response.status(), 404);
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/problem+json"
        );
        let json = json_body(response).await;
        let detail = json["detail"].as_str().unwrap();
        assert!(detail.contains("project not found"), "got: {detail}");
        assert!(detail.contains("nonexistent"), "got: {detail}");
    }

    // -- create --

    #[tokio::test]
    async fn create_review_returns_201_with_location() {
        let (_dir, state) = setup();
        let response = send(
            &state,
            post_json(
                "/projects/demo/reviews",
                r#"{"target":"phase/alpha/1","author":"ed"}"#,
            ),
        )
        .await;
        assert_eq!(response.status(), 201);
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/hal+json"
        );
        let location = response
            .headers()
            .get("location")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let json = json_body(response).await;
        let id = json["id"].as_str().unwrap();
        assert_eq!(location, format!("/projects/demo/reviews/{id}"));
        assert_eq!(json["state"], "draft");
        assert_eq!(json["author"], "ed");
        // The phase-number reference resolved to the stem.
        assert_eq!(json["target"]["kind"], "phase");
        assert_eq!(json["target"]["stem"], "phase-1-one");
        assert_eq!(
            json["_links"]["target"]["href"],
            "/projects/demo/roadmaps/alpha/phases/phase-1-one"
        );
    }

    #[tokio::test]
    async fn create_review_with_summary_and_default_author() {
        let (_dir, state) = setup();
        let response = send(
            &state,
            post_json(
                "/projects/demo/reviews",
                r#"{"target":"task/fix-login","summary":"Initial notes."}"#,
            ),
        )
        .await;
        assert_eq!(response.status(), 201);
        let json = json_body(response).await;
        assert_eq!(json["body"], "Initial notes.");
        assert_eq!(json["author"], "api");
    }

    #[tokio::test]
    async fn create_review_invalid_target_returns_400() {
        let (_dir, state) = setup();
        let response = send(
            &state,
            post_json("/projects/demo/reviews", r#"{"target":"widget/alpha"}"#),
        )
        .await;
        assert_eq!(response.status(), 400);
        let json = json_body(response).await;
        assert!(json["detail"].as_str().unwrap().contains("roadmap/<slug>"));
    }

    #[tokio::test]
    async fn create_review_target_missing_returns_404() {
        let (_dir, state) = setup();
        let response = send(
            &state,
            post_json("/projects/demo/reviews", r#"{"target":"task/nonexistent"}"#),
        )
        .await;
        assert_eq!(response.status(), 404);
        let json = json_body(response).await;
        assert!(json["detail"].as_str().unwrap().contains("nonexistent"));
    }

    // -- get --

    #[tokio::test]
    async fn get_review_returns_detail_with_resolution() {
        let (_dir, state) = setup();
        let id = create_draft(&state, "task/fix-login", "ed").await;
        add_comment_ok(
            &state,
            &id,
            serde_json::json!({
                "body": "Tighten this.",
                "anchor": {
                    "anchor_type": "text-quote",
                    "quote": "quoted span",
                    "prefix": "the ",
                    "suffix": " here"
                }
            }),
        )
        .await;

        let response = send(&state, get_req(&format!("/projects/demo/reviews/{id}"))).await;
        assert_eq!(response.status(), 200);
        let json = json_body(response).await;
        assert_eq!(json["id"], serde_json::json!(id));
        assert_eq!(json["_links"]["project"]["href"], "/projects/demo");
        let c = &json["comments"][0];
        assert_eq!(c["body"], "Tighten this.");
        assert_eq!(c["anchor"]["anchor_type"], "text-quote");
        // FsStore has no history, so resolution lands on the current body.
        assert_eq!(c["resolution"]["state"], "resolved");
        assert_eq!(c["resolution"]["quote"], "quoted span");
        assert_eq!(c["resolution"]["body"], "current");
        assert!(c["resolution"]["range_start"].is_u64());
        assert!(c["resolution"]["range_end"].is_u64());
    }

    #[tokio::test]
    async fn get_review_unknown_anchor_round_trips_untouched() {
        let (_dir, state) = setup();
        let id = create_draft(&state, "task/fix-login", "ed").await;
        add_comment_ok(
            &state,
            &id,
            serde_json::json!({
                "body": "Future anchor kind.",
                "anchor": {
                    "anchor_type": "line-range",
                    "start": 3,
                    "end": 7,
                    "extra": "keep-me"
                }
            }),
        )
        .await;

        let response = send(&state, get_req(&format!("/projects/demo/reviews/{id}"))).await;
        assert_eq!(response.status(), 200);
        let json = json_body(response).await;
        let anchor = &json["comments"][0]["anchor"];
        assert_eq!(anchor["anchor_type"], "line-range");
        assert_eq!(anchor["start"], 3);
        assert_eq!(anchor["end"], 7);
        assert_eq!(anchor["extra"], "keep-me");
        assert_eq!(json["comments"][0]["resolution"]["state"], "unresolved");
    }

    #[tokio::test]
    async fn get_review_not_found_returns_404() {
        let (_dir, state) = setup();
        let response = send(&state, get_req("/projects/demo/reviews/nope")).await;
        assert_eq!(response.status(), 404);
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/problem+json"
        );
    }

    // -- add comment --

    #[tokio::test]
    async fn add_comment_returns_201_with_location() {
        let (_dir, state) = setup();
        let id = create_draft(&state, "task/fix-login", "ed").await;
        let uri = format!("/projects/demo/reviews/{id}/comments");
        let response = send(
            &state,
            post_json(&uri, r#"{"body":"Whole-document note."}"#),
        )
        .await;
        assert_eq!(response.status(), 201);
        assert_eq!(
            response
                .headers()
                .get("location")
                .unwrap()
                .to_str()
                .unwrap(),
            format!("/projects/demo/reviews/{id}/comments/1")
        );
        let json = json_body(response).await;
        assert_eq!(json["comments"][0]["id"], 1);
        assert_eq!(json["comments"][0]["status"], "open");
        assert_eq!(json["comments"][0]["resolution"]["state"], "unresolved");
    }

    #[tokio::test]
    async fn add_comment_empty_body_returns_422() {
        let (_dir, state) = setup();
        let id = create_draft(&state, "task/fix-login", "ed").await;
        let uri = format!("/projects/demo/reviews/{id}/comments");
        let response = send(&state, post_json(&uri, r#"{"body":"   "}"#)).await;
        assert_eq!(response.status(), 422);
        let json = json_body(response).await;
        assert!(
            json["detail"]
                .as_str()
                .unwrap()
                .contains("must not be empty")
        );
    }

    #[tokio::test]
    async fn add_comment_doc_scope_on_roadmap_review_succeeds() {
        let (_dir, state) = setup();
        let id = create_draft(&state, "roadmap/alpha", "ed").await;
        let uri = format!("/projects/demo/reviews/{id}/comments");
        let response = send(
            &state,
            post_json(
                &uri,
                r#"{"body":"Phase-scoped.","doc":{"kind":"phase","stem":"phase-1-one"}}"#,
            ),
        )
        .await;
        assert_eq!(response.status(), 201);
        let json = json_body(response).await;
        assert_eq!(json["comments"][0]["doc"]["stem"], "phase-1-one");
    }

    #[tokio::test]
    async fn add_comment_doc_on_task_review_returns_400() {
        let (_dir, state) = setup();
        let id = create_draft(&state, "task/fix-login", "ed").await;
        let uri = format!("/projects/demo/reviews/{id}/comments");
        let response = send(
            &state,
            post_json(
                &uri,
                r#"{"body":"Bad scope.","doc":{"kind":"phase","stem":"phase-1-one"}}"#,
            ),
        )
        .await;
        assert_eq!(response.status(), 400);
        let json = json_body(response).await;
        assert!(
            json["detail"].as_str().unwrap().contains("roadmap review"),
            "must explain doc scoping"
        );
    }

    #[tokio::test]
    async fn add_comment_doc_out_of_scope_returns_400() {
        let (_dir, state) = setup();
        let id = create_draft(&state, "roadmap/alpha", "ed").await;
        let uri = format!("/projects/demo/reviews/{id}/comments");
        let response = send(
            &state,
            post_json(
                &uri,
                r#"{"body":"Ghost phase.","doc":{"kind":"phase","stem":"ghost"}}"#,
            ),
        )
        .await;
        assert_eq!(response.status(), 400);
        let json = json_body(response).await;
        let detail = json["detail"].as_str().unwrap();
        assert!(detail.contains("out of scope"), "got: {detail}");
        assert!(detail.contains("ghost"), "got: {detail}");
    }

    #[tokio::test]
    async fn add_comment_after_submit_returns_409() {
        let (_dir, state) = setup();
        let id = create_draft(&state, "task/fix-login", "ed").await;
        submit_ok(&state, &id, "comment").await;
        let uri = format!("/projects/demo/reviews/{id}/comments");
        let response = send(&state, post_json(&uri, r#"{"body":"Too late."}"#)).await;
        assert_eq!(response.status(), 409);
        let json = json_body(response).await;
        assert!(json["detail"].as_str().unwrap().contains("not a draft"));
    }

    #[tokio::test]
    async fn add_comment_review_not_found_returns_404() {
        let (_dir, state) = setup();
        let response = send(
            &state,
            post_json("/projects/demo/reviews/nope/comments", r#"{"body":"x"}"#),
        )
        .await;
        assert_eq!(response.status(), 404);
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/problem+json"
        );
        let json = json_body(response).await;
        let detail = json["detail"].as_str().unwrap();
        assert!(detail.contains("review not found"), "got: {detail}");
        assert!(detail.contains("nope"), "got: {detail}");
    }

    // -- update comment --

    /// Creates a draft on the task with one anchored comment; returns the id.
    async fn draft_with_anchored_comment(state: &AppState) -> String {
        let id = create_draft(state, "task/fix-login", "ed").await;
        add_comment_ok(
            state,
            &id,
            serde_json::json!({
                "body": "Tighten this.",
                "anchor": {
                    "anchor_type": "text-quote",
                    "quote": "quoted span",
                    "prefix": "the ",
                    "suffix": " here"
                }
            }),
        )
        .await;
        id
    }

    #[tokio::test]
    async fn update_comment_draft_edits_body() {
        let (_dir, state) = setup();
        let id = draft_with_anchored_comment(&state).await;
        let uri = format!("/projects/demo/reviews/{id}/comments/1");
        let response = send(&state, patch_json(&uri, r#"{"body":"Reworded."}"#)).await;
        assert_eq!(response.status(), 200);
        let json = json_body(response).await;
        assert_eq!(json["comments"][0]["body"], "Reworded.");
        // Anchor untouched: absent key means keep.
        assert_eq!(json["comments"][0]["anchor"]["quote"], "quoted span");
    }

    #[tokio::test]
    async fn update_comment_empty_patch_keeps_everything() {
        let (_dir, state) = setup();
        let id = draft_with_anchored_comment(&state).await;
        let uri = format!("/projects/demo/reviews/{id}/comments/1");
        let response = send(&state, patch_json(&uri, "{}")).await;
        assert_eq!(response.status(), 200);
        let json = json_body(response).await;
        assert_eq!(json["comments"][0]["body"], "Tighten this.");
        assert_eq!(json["comments"][0]["anchor"]["quote"], "quoted span");
    }

    #[tokio::test]
    async fn update_comment_null_anchor_clears() {
        let (_dir, state) = setup();
        let id = draft_with_anchored_comment(&state).await;
        let uri = format!("/projects/demo/reviews/{id}/comments/1");
        let response = send(&state, patch_json(&uri, r#"{"anchor":null}"#)).await;
        assert_eq!(response.status(), 200);
        let json = json_body(response).await;
        assert!(json["comments"][0].get("anchor").is_none());
        assert_eq!(json["comments"][0]["resolution"]["state"], "unresolved");
    }

    #[tokio::test]
    async fn update_comment_anchor_value_sets() {
        let (_dir, state) = setup();
        let id = create_draft(&state, "task/fix-login", "ed").await;
        add_comment_ok(&state, &id, serde_json::json!({ "body": "No anchor yet." })).await;
        let uri = format!("/projects/demo/reviews/{id}/comments/1");
        let body = serde_json::json!({
            "anchor": {
                "anchor_type": "text-quote",
                "quote": "quoted span",
                "prefix": "the ",
                "suffix": " here"
            }
        });
        let response = send(&state, patch_json(&uri, &body.to_string())).await;
        assert_eq!(response.status(), 200);
        let json = json_body(response).await;
        assert_eq!(json["comments"][0]["anchor"]["quote"], "quoted span");
        assert_eq!(json["comments"][0]["resolution"]["state"], "resolved");
    }

    #[tokio::test]
    async fn update_comment_doc_set_and_clear() {
        let (_dir, state) = setup();
        let id = create_draft(&state, "roadmap/alpha", "ed").await;
        add_comment_ok(&state, &id, serde_json::json!({ "body": "Scoped later." })).await;
        let uri = format!("/projects/demo/reviews/{id}/comments/1");

        let response = send(
            &state,
            patch_json(&uri, r#"{"doc":{"kind":"phase","stem":"phase-1-one"}}"#),
        )
        .await;
        assert_eq!(response.status(), 200);
        let json = json_body(response).await;
        assert_eq!(json["comments"][0]["doc"]["stem"], "phase-1-one");

        let response = send(&state, patch_json(&uri, r#"{"doc":null}"#)).await;
        assert_eq!(response.status(), 200);
        let json = json_body(response).await;
        assert!(json["comments"][0].get("doc").is_none());
    }

    #[tokio::test]
    async fn update_comment_blank_body_returns_422() {
        let (_dir, state) = setup();
        let id = draft_with_anchored_comment(&state).await;
        let uri = format!("/projects/demo/reviews/{id}/comments/1");
        let response = send(&state, patch_json(&uri, r#"{"body":"   "}"#)).await;
        assert_eq!(response.status(), 422);
        let json = json_body(response).await;
        assert!(
            json["detail"]
                .as_str()
                .unwrap()
                .contains("must not be empty")
        );

        // The comment survives untouched.
        let response = send(&state, get_req(&format!("/projects/demo/reviews/{id}"))).await;
        let json = json_body(response).await;
        assert_eq!(json["comments"][0]["body"], "Tighten this.");
    }

    #[tokio::test]
    async fn update_comment_absent_doc_keeps_existing() {
        let (_dir, state) = setup();
        let id = create_draft(&state, "roadmap/alpha", "ed").await;
        add_comment_ok(
            &state,
            &id,
            serde_json::json!({
                "body": "Phase-scoped.",
                "doc": { "kind": "phase", "stem": "phase-1-one" }
            }),
        )
        .await;
        let uri = format!("/projects/demo/reviews/{id}/comments/1");
        // The `doc` key is absent from the patch: tri-state Keep, not Clear.
        let response = send(&state, patch_json(&uri, r#"{"body":"Reworded."}"#)).await;
        assert_eq!(response.status(), 200);
        let json = json_body(response).await;
        assert_eq!(json["comments"][0]["body"], "Reworded.");
        assert_eq!(json["comments"][0]["doc"]["stem"], "phase-1-one");
    }

    #[tokio::test]
    async fn update_comment_doc_out_of_scope_returns_400() {
        let (_dir, state) = setup();
        let id = create_draft(&state, "roadmap/alpha", "ed").await;
        add_comment_ok(&state, &id, serde_json::json!({ "body": "Scoped later." })).await;
        let uri = format!("/projects/demo/reviews/{id}/comments/1");
        let response = send(
            &state,
            patch_json(&uri, r#"{"doc":{"kind":"phase","stem":"ghost"}}"#),
        )
        .await;
        assert_eq!(response.status(), 400);
    }

    #[tokio::test]
    async fn update_comment_structural_after_submit_returns_409() {
        let (_dir, state) = setup();
        let id = draft_with_anchored_comment(&state).await;
        submit_ok(&state, &id, "request-changes").await;
        let uri = format!("/projects/demo/reviews/{id}/comments/1");
        let response = send(&state, patch_json(&uri, r#"{"body":"Late edit."}"#)).await;
        assert_eq!(response.status(), 409);
        let json = json_body(response).await;
        assert!(json["detail"].as_str().unwrap().contains("not a draft"));
    }

    #[tokio::test]
    async fn update_comment_resolution_before_submit_returns_409() {
        let (_dir, state) = setup();
        let id = draft_with_anchored_comment(&state).await;
        let uri = format!("/projects/demo/reviews/{id}/comments/1");
        let response = send(&state, patch_json(&uri, r#"{"status":"addressed"}"#)).await;
        assert_eq!(response.status(), 409);
        let json = json_body(response).await;
        assert!(
            json["detail"]
                .as_str()
                .unwrap()
                .contains("not been submitted")
        );
    }

    #[tokio::test]
    async fn update_comment_resolution_after_submit_succeeds() {
        let (_dir, state) = setup();
        let id = draft_with_anchored_comment(&state).await;
        submit_ok(&state, &id, "request-changes").await;
        let uri = format!("/projects/demo/reviews/{id}/comments/1");
        let body = serde_json::json!({
            "status": "addressed",
            "applied_commit": "abc123",
            "reply": "Done in abc123."
        });
        let response = send(&state, patch_json(&uri, &body.to_string())).await;
        assert_eq!(response.status(), 200);
        let json = json_body(response).await;
        assert_eq!(json["comments"][0]["status"], "addressed");
        assert_eq!(json["comments"][0]["applied_commit"], "abc123");
        assert_eq!(json["comments"][0]["reply"], "Done in abc123.");
    }

    #[tokio::test]
    async fn update_comment_unknown_id_returns_404() {
        let (_dir, state) = setup();
        let id = draft_with_anchored_comment(&state).await;
        let uri = format!("/projects/demo/reviews/{id}/comments/99");
        let response = send(&state, patch_json(&uri, r#"{"body":"x"}"#)).await;
        assert_eq!(response.status(), 404);
        let json = json_body(response).await;
        assert!(json["detail"].as_str().unwrap().contains("99"));
    }

    #[tokio::test]
    async fn update_comment_non_numeric_id_returns_problem_json_400() {
        let (_dir, state) = setup();
        let id = draft_with_anchored_comment(&state).await;
        let uri = format!("/projects/demo/reviews/{id}/comments/first");
        let response = send(&state, patch_json(&uri, r#"{"body":"x"}"#)).await;
        assert_eq!(response.status(), 400);
        // Must be Problem+JSON, not axum's plain-text path rejection.
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/problem+json"
        );
        let json = json_body(response).await;
        let detail = json["detail"].as_str().unwrap();
        assert!(detail.contains("'first'"), "must name the bad id: {detail}");
        assert!(
            detail.contains("numeric id"),
            "must be actionable: {detail}"
        );
    }

    // -- submit --

    #[tokio::test]
    async fn submit_review_missing_verdict_returns_400() {
        let (_dir, state) = setup();
        let id = draft_with_anchored_comment(&state).await;
        let uri = format!("/projects/demo/reviews/{id}/submit");
        let response = send(&state, post_json(&uri, "{}")).await;
        assert_eq!(response.status(), 400);
        let json = json_body(response).await;
        assert!(json["detail"].as_str().unwrap().contains("verdict"));
    }

    #[tokio::test]
    async fn submit_empty_review_returns_400() {
        let (_dir, state) = setup();
        let id = create_draft(&state, "task/fix-login", "ed").await;
        let uri = format!("/projects/demo/reviews/{id}/submit");
        let response = send(&state, post_json(&uri, r#"{"verdict":"approve"}"#)).await;
        assert_eq!(response.status(), 400);
        let json = json_body(response).await;
        assert!(
            json["detail"]
                .as_str()
                .unwrap()
                .contains("no comments and no summary")
        );
    }

    #[tokio::test]
    async fn submit_review_succeeds_with_verdict_and_summary() {
        let (_dir, state) = setup();
        let id = create_draft(&state, "task/fix-login", "ed").await;
        let uri = format!("/projects/demo/reviews/{id}/submit");
        let response = send(
            &state,
            post_json(&uri, r#"{"verdict":"approve","summary":"Ship it."}"#),
        )
        .await;
        assert_eq!(response.status(), 200);
        let json = json_body(response).await;
        assert_eq!(json["state"], "submitted");
        assert_eq!(json["verdict"], "approve");
        // Stored bodies are normalized with a trailing newline.
        assert_eq!(json["body"], "Ship it.\n");
        assert!(json["submitted"].is_string());
    }

    #[tokio::test]
    async fn submit_blank_summary_returns_422_and_preserves_existing() {
        let (_dir, state) = setup();
        let response = send(
            &state,
            post_json(
                "/projects/demo/reviews",
                r#"{"target":"task/fix-login","author":"ed","summary":"Initial notes."}"#,
            ),
        )
        .await;
        assert_eq!(response.status(), 201);
        let id = json_body(response).await["id"]
            .as_str()
            .unwrap()
            .to_string();

        let uri = format!("/projects/demo/reviews/{id}/submit");
        let response = send(
            &state,
            post_json(&uri, r#"{"verdict":"approve","summary":"   "}"#),
        )
        .await;
        assert_eq!(response.status(), 422);
        let json = json_body(response).await;
        let detail = json["detail"].as_str().unwrap();
        assert!(detail.contains("must not be blank"), "got: {detail}");
        assert!(detail.contains("omit the field"), "got: {detail}");

        // Neither submitted nor clobbered: still a draft with the original
        // summary.
        let response = send(&state, get_req(&format!("/projects/demo/reviews/{id}"))).await;
        let json = json_body(response).await;
        assert_eq!(json["state"], "draft");
        assert_eq!(json["body"], "Initial notes.\n");
    }

    #[tokio::test]
    async fn submit_blank_summary_without_comments_names_the_blank_summary() {
        // With no comments and no existing summary, a blank summary must
        // fail on the blank value itself — not with core's "no comments and
        // no summary", which would gaslight a caller who *did* pass one.
        let (_dir, state) = setup();
        let id = create_draft(&state, "task/fix-login", "ed").await;
        let uri = format!("/projects/demo/reviews/{id}/submit");
        let response = send(
            &state,
            post_json(&uri, r#"{"verdict":"approve","summary":""}"#),
        )
        .await;
        assert_eq!(response.status(), 422);
        let json = json_body(response).await;
        let detail = json["detail"].as_str().unwrap();
        assert!(
            detail.contains("summary must not be blank"),
            "got: {detail}"
        );
        assert!(
            !detail.contains("no comments and no summary"),
            "must not claim no summary was passed: {detail}"
        );
    }

    #[tokio::test]
    async fn submit_twice_returns_409() {
        let (_dir, state) = setup();
        let id = create_draft(&state, "task/fix-login", "ed").await;
        submit_ok(&state, &id, "approve").await;
        let uri = format!("/projects/demo/reviews/{id}/submit");
        let response = send(&state, post_json(&uri, r#"{"verdict":"approve"}"#)).await;
        assert_eq!(response.status(), 409);
    }

    // -- transition (PATCH review) --

    #[tokio::test]
    async fn patch_review_dismiss_draft_succeeds() {
        let (_dir, state) = setup();
        let id = create_draft(&state, "task/fix-login", "ed").await;
        let uri = format!("/projects/demo/reviews/{id}");
        let response = send(&state, patch_json(&uri, r#"{"state":"dismissed"}"#)).await;
        assert_eq!(response.status(), 200);
        let json = json_body(response).await;
        assert_eq!(json["state"], "dismissed");
    }

    #[tokio::test]
    async fn patch_review_addressed_before_submit_returns_409() {
        let (_dir, state) = setup();
        let id = create_draft(&state, "task/fix-login", "ed").await;
        let uri = format!("/projects/demo/reviews/{id}");
        let response = send(&state, patch_json(&uri, r#"{"state":"addressed"}"#)).await;
        assert_eq!(response.status(), 409);
        let json = json_body(response).await;
        assert!(json["detail"].as_str().unwrap().contains("cannot move"));
    }

    #[tokio::test]
    async fn patch_review_addressed_with_open_comments_returns_409() {
        let (_dir, state) = setup();
        let id = draft_with_anchored_comment(&state).await;
        submit_ok(&state, &id, "request-changes").await;
        let uri = format!("/projects/demo/reviews/{id}");
        let response = send(&state, patch_json(&uri, r#"{"state":"addressed"}"#)).await;
        assert_eq!(response.status(), 409);
        let json = json_body(response).await;
        assert!(json["detail"].as_str().unwrap().contains("open comment"));
    }

    #[tokio::test]
    async fn patch_review_addressed_succeeds_after_comments_resolved() {
        let (_dir, state) = setup();
        let id = draft_with_anchored_comment(&state).await;
        submit_ok(&state, &id, "request-changes").await;
        let comment_uri = format!("/projects/demo/reviews/{id}/comments/1");
        let response = send(
            &state,
            patch_json(&comment_uri, r#"{"status":"addressed"}"#),
        )
        .await;
        assert_eq!(response.status(), 200);

        let uri = format!("/projects/demo/reviews/{id}");
        let response = send(&state, patch_json(&uri, r#"{"state":"addressed"}"#)).await;
        assert_eq!(response.status(), 200);
        let json = json_body(response).await;
        assert_eq!(json["state"], "addressed");
    }

    #[tokio::test]
    async fn patch_review_terminal_rejects_further_transition() {
        let (_dir, state) = setup();
        let id = create_draft(&state, "task/fix-login", "ed").await;
        let uri = format!("/projects/demo/reviews/{id}");
        let response = send(&state, patch_json(&uri, r#"{"state":"dismissed"}"#)).await;
        assert_eq!(response.status(), 200);
        let response = send(&state, patch_json(&uri, r#"{"state":"dismissed"}"#)).await;
        assert_eq!(response.status(), 409);
    }

    #[tokio::test]
    async fn patch_review_draft_state_value_returns_422() {
        let (_dir, state) = setup();
        let id = create_draft(&state, "task/fix-login", "ed").await;
        let uri = format!("/projects/demo/reviews/{id}");
        let response = send(&state, patch_json(&uri, r#"{"state":"draft"}"#)).await;
        assert_eq!(response.status(), 422);
        let json = json_body(response).await;
        assert!(
            json["detail"]
                .as_str()
                .unwrap()
                .contains("'addressed' or 'dismissed'")
        );
    }

    // -- delete --

    #[tokio::test]
    async fn delete_draft_returns_204_and_removes() {
        let (_dir, state) = setup();
        let id = create_draft(&state, "task/fix-login", "ed").await;
        let uri = format!("/projects/demo/reviews/{id}");
        let response = send(&state, delete_req(&uri)).await;
        assert_eq!(response.status(), 204);
        let response = send(&state, get_req(&uri)).await;
        assert_eq!(response.status(), 404);
    }

    #[tokio::test]
    async fn delete_submitted_returns_409() {
        let (_dir, state) = setup();
        let id = create_draft(&state, "task/fix-login", "ed").await;
        submit_ok(&state, &id, "approve").await;
        let uri = format!("/projects/demo/reviews/{id}");
        let response = send(&state, delete_req(&uri)).await;
        assert_eq!(response.status(), 409);
        // Still there.
        let response = send(&state, get_req(&uri)).await;
        assert_eq!(response.status(), 200);
    }

    #[tokio::test]
    async fn delete_not_found_returns_404() {
        let (_dir, state) = setup();
        let response = send(&state, delete_req("/projects/demo/reviews/nope")).await;
        assert_eq!(response.status(), 404);
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/problem+json"
        );
        let json = json_body(response).await;
        let detail = json["detail"].as_str().unwrap();
        assert!(detail.contains("review not found"), "got: {detail}");
        assert!(detail.contains("nope"), "got: {detail}");
    }
}
