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

/// Builds the application router with all routes and shared state.
pub fn build_router(state: AppState) -> Router {
    let changeset = state.changeset.clone();
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
                async move {
                    let mut response = next.run(req).await;
                    if let Some(id) = changeset
                        && let Ok(value) = axum::http::HeaderValue::from_str(&id)
                    {
                        response.headers_mut().insert(CHANGESET_HEADER, value);
                    }
                    response
                }
            },
        ))
}
