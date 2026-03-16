use axum::Router;
use axum::routing::get;

use crate::handlers;
use crate::state::AppState;

/// Builds the application router with all routes and shared state.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(handlers::health::healthz))
        .with_state(state)
}
