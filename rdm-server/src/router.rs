use axum::Router;
use axum::routing::get;

use crate::handlers;
use crate::state::AppState;

/// Builds the application router with all routes and shared state.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(handlers::root::index))
        .route("/healthz", get(handlers::health::healthz))
        .route("/projects", get(handlers::projects::list_projects))
        .route(
            "/projects/{project}/roadmaps",
            get(handlers::roadmaps::list_roadmaps),
        )
        .route(
            "/projects/{project}/roadmaps/{roadmap}",
            get(handlers::roadmaps::get_roadmap),
        )
        .route(
            "/projects/{project}/roadmaps/{roadmap}/phases/{phase}",
            get(handlers::phases::get_phase),
        )
        .route(
            "/projects/{project}/tasks",
            get(handlers::tasks::list_tasks),
        )
        .route(
            "/projects/{project}/tasks/{task}",
            get(handlers::tasks::get_task),
        )
        .with_state(state)
}
