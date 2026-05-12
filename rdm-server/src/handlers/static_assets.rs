use axum::http::header;
use axum::response::IntoResponse;

const EDIT_JS: &str = include_str!("../../assets/edit.js");

/// `GET /static/edit.js` — serves the embedded edit-form client script.
pub async fn edit_js() -> impl IntoResponse {
    (
        [
            (
                header::CONTENT_TYPE,
                "application/javascript; charset=utf-8",
            ),
            (header::CACHE_CONTROL, "no-store"),
        ],
        EDIT_JS,
    )
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;
    use axum::http::Request;
    use tower::ServiceExt;

    use crate::router::build_router;
    use crate::state::AppState;

    fn test_state() -> AppState {
        AppState {
            plan_root: std::path::PathBuf::from("/tmp/rdm-test"),
            quick_filters: Vec::new(),
        }
    }

    #[tokio::test]
    async fn edit_js_returns_200_with_javascript_content_type() {
        let app = build_router(test_state());
        let response = app
            .oneshot(
                Request::get("/static/edit.js")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let ctype = response
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert!(
            ctype.contains("javascript"),
            "expected javascript content-type, got: {ctype}"
        );
        assert_eq!(
            response
                .headers()
                .get("cache-control")
                .unwrap()
                .to_str()
                .unwrap(),
            "no-store"
        );
        let body = to_bytes(response.into_body(), 16384).await.unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(!text.is_empty(), "edit.js body should not be empty");
        assert!(
            text.contains("data-rdm-edit"),
            "edit.js should reference data-rdm-edit selector"
        );
    }
}
