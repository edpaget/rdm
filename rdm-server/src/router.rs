use axum::Router;
use axum::routing::{get, patch, post};

use crate::handlers;
use crate::state::AppState;

/// Builds the application router with all routes and shared state.
pub fn build_router(state: AppState) -> Router {
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
            "/static/styles.css",
            get(handlers::static_assets::styles_css),
        )
        .route("/favicon.ico", get(handlers::static_assets::favicon))
        .with_state(state)
}
