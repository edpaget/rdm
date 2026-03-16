use axum::http::StatusCode;

/// Health check endpoint. Returns `200 OK` with no body.
pub async fn healthz() -> StatusCode {
    StatusCode::OK
}

#[cfg(test)]
mod tests {
    use axum::http::Request;
    use tower::ServiceExt;

    use crate::router::build_router;
    use crate::state::AppState;

    fn test_state() -> AppState {
        AppState {
            plan_root: std::path::PathBuf::from("/tmp/rdm-test"),
        }
    }

    #[tokio::test]
    async fn healthz_returns_200() {
        let app = build_router(test_state());
        let response = app
            .oneshot(
                Request::get("/healthz")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn unknown_route_returns_404() {
        let app = build_router(test_state());
        let response = app
            .oneshot(
                Request::get("/nonexistent")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    }
}
