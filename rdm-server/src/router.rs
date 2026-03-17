use axum::Router;
use axum::routing::{get, post};

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
            get(handlers::roadmaps::get_roadmap),
        )
        .route(
            "/projects/{project}/roadmaps/{roadmap}/phases",
            post(handlers::phases::create_phase),
        )
        .route(
            "/projects/{project}/roadmaps/{roadmap}/phases/{phase}",
            get(handlers::phases::get_phase).patch(handlers::phases::update_phase),
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
        .with_state(state)
}
