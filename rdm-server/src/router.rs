use axum::Router;
use axum::routing::{get, patch, post};

use crate::handlers;
use crate::state::AppState;

/// Header naming the changeset every mutation this server makes belongs to.
///
/// Set on **every** response from one place, so a client that just POSTed a
/// mutation can always name the changeset to reconcile — even under the
/// staging-only default, where the server never commits on its own.
pub const CHANGESET_HEADER: &str = "x-rdm-changeset";

/// Header carrying the "this was staged, not committed" warning and the
/// exact command that lands it.
///
/// Set on **mutating** responses only (anything that is not a safe method),
/// and only under the staging-only default: under `--autocommit` the write
/// did reach a commit, so there is nothing pending to report. Together with
/// the per-mutation stderr warning this is what makes staging loud per
/// *mutation* rather than per *process* — a boot line has long scrolled away
/// by the time a week-old server stages a write.
pub const STAGED_HEADER: &str = "x-rdm-staged";

/// Whether a request method can have mutated the plan repo.
///
/// The safe methods (RFC 9110 § 9.2.1) never reach a handler that calls
/// [`AppState::post_mutate`], so tagging their responses as "staged" would
/// be noise on every page load.
fn is_mutating(method: &axum::http::Method) -> bool {
    !matches!(
        *method,
        axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS
    )
}

/// Builds the application router with all routes and shared state.
pub fn build_router(state: AppState) -> Router {
    let changeset = state.changeset.clone();
    let staged_notice = state.staged_notice();
    Router::new()
        .route("/", get(handlers::root::index))
        .route("/healthz", get(handlers::health::healthz))
        .route(
            "/projects",
            get(handlers::projects::list_projects).post(handlers::projects::create_project),
        )
        .route(
            "/projects/{project}/roadmaps",
            get(handlers::roadmaps::list_roadmaps).post(handlers::roadmaps::create_roadmap),
        )
        .route(
            "/projects/{project}/roadmaps/{roadmap}",
            get(handlers::roadmaps::get_roadmap).patch(handlers::roadmaps::update_roadmap),
        )
        .route(
            "/projects/{project}/roadmaps/{roadmap}/phases",
            get(handlers::phases::list_phases).post(handlers::phases::create_phase),
        )
        .route(
            "/projects/{project}/roadmaps/{roadmap}/phases/{phase}",
            get(handlers::phases::get_phase).patch(handlers::phases::update_phase),
        )
        .route(
            "/projects/{project}/reviews",
            get(handlers::reviews::list_reviews).post(handlers::reviews::create_review),
        )
        .route(
            "/projects/{project}/reviews/form",
            post(handlers::review_forms::start_review_form),
        )
        .route(
            "/projects/{project}/reviews/{review_id}/form/comments",
            post(handlers::review_forms::add_comment_form),
        )
        .route(
            "/projects/{project}/reviews/{review_id}/form/comments/anchor",
            post(handlers::review_forms::anchor_comment_form),
        )
        .route(
            "/projects/{project}/reviews/{review_id}/form/comments/{comment_id}/edit",
            post(handlers::review_forms::edit_comment_form),
        )
        .route(
            "/projects/{project}/reviews/{review_id}/form/comments/{comment_id}/remove",
            post(handlers::review_forms::remove_comment_form),
        )
        .route(
            "/projects/{project}/reviews/{review_id}/form/submit",
            post(handlers::review_forms::submit_review_form),
        )
        .route(
            "/projects/{project}/reviews/{review_id}/form/dismiss",
            post(handlers::review_forms::dismiss_review_form),
        )
        .route(
            "/projects/{project}/reviews/{review_id}/form/delete",
            post(handlers::review_forms::delete_review_form),
        )
        .route(
            "/projects/{project}/reviews/{review_id}",
            get(handlers::reviews::get_review)
                .patch(handlers::reviews::update_review)
                .delete(handlers::reviews::delete_review),
        )
        .route(
            "/projects/{project}/reviews/{review_id}/comments",
            post(handlers::reviews::add_comment),
        )
        .route(
            "/projects/{project}/reviews/{review_id}/comments/{comment_id}",
            patch(handlers::reviews::update_comment),
        )
        .route(
            "/projects/{project}/reviews/{review_id}/submit",
            post(handlers::reviews::submit_review),
        )
        .route(
            "/projects/{project}/search",
            get(handlers::search::search_items),
        )
        .route(
            "/projects/{project}/tasks",
            get(handlers::tasks::list_tasks).post(handlers::tasks::create_task),
        )
        .route(
            "/projects/{project}/tasks/{task}",
            get(handlers::tasks::get_task).patch(handlers::tasks::update_task),
        )
        .route(
            "/projects/{project}/tasks/{task}/promote",
            post(handlers::tasks::promote_task),
        )
        .route("/static/edit.js", get(handlers::static_assets::edit_js))
        .route(
            "/static/review-highlight.js",
            get(handlers::static_assets::review_highlight_js),
        )
        .route(
            "/static/review-anchor.js",
            get(handlers::static_assets::review_anchor_js),
        )
        .route(
            "/static/styles.css",
            get(handlers::static_assets::styles_css),
        )
        .route("/favicon.ico", get(handlers::static_assets::favicon))
        .with_state(state)
        // Surfacing the pending changeset happens in exactly one place, so a
        // handler can never forget it.
        .layer(axum::middleware::from_fn(
            move |req: axum::extract::Request, next: axum::middleware::Next| {
                let changeset = changeset.clone();
                let staged_notice = staged_notice.clone();
                async move {
                    let mutating = is_mutating(req.method());
                    let mut response = next.run(req).await;
                    if let Some(id) = changeset
                        && let Ok(value) = axum::http::HeaderValue::from_str(&id)
                    {
                        response.headers_mut().insert(CHANGESET_HEADER, value);
                    }
                    if mutating
                        && let Some(notice) = staged_notice
                        && let Ok(value) = axum::http::HeaderValue::from_str(&notice)
                    {
                        response.headers_mut().insert(STAGED_HEADER, value);
                    }
                    response
                }
            },
        ))
}
