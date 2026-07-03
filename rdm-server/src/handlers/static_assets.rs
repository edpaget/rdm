use axum::http::header;
use axum::response::IntoResponse;

const EDIT_JS: &str = include_str!("../../assets/edit.js");
const REVIEW_HIGHLIGHT_JS: &str = include_str!("../../assets/review-highlight.js");
const REVIEW_ANCHOR_JS: &str = include_str!("../../assets/review-anchor.js");
const STYLES_CSS: &str = include_str!("../../assets/styles.css");
const FAVICON_SVG: &str = include_str!("../../assets/logo.svg");

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

/// `GET /static/review-highlight.js` — serves the embedded
/// hover-to-highlight client script for review-comment anchors.
pub async fn review_highlight_js() -> impl IntoResponse {
    (
        [
            (
                header::CONTENT_TYPE,
                "application/javascript; charset=utf-8",
            ),
            (header::CACHE_CONTROL, "no-store"),
        ],
        REVIEW_HIGHLIGHT_JS,
    )
}

/// `GET /static/review-anchor.js` — serves the embedded select-to-anchor
/// client script (selection gesture plus no-reload draft-panel updates).
pub async fn review_anchor_js() -> impl IntoResponse {
    (
        [
            (
                header::CONTENT_TYPE,
                "application/javascript; charset=utf-8",
            ),
            (header::CACHE_CONTROL, "no-store"),
        ],
        REVIEW_ANCHOR_JS,
    )
}

/// `GET /static/styles.css` — serves the embedded stylesheet.
pub async fn styles_css() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        STYLES_CSS,
    )
}

/// `GET /favicon.ico` — serves the project logo as an SVG favicon.
///
/// The `.ico` path is the legacy browser convention; the payload is SVG,
/// which modern browsers handle via the `image/svg+xml` content type.
pub async fn favicon() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/svg+xml; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        FAVICON_SVG,
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
    async fn styles_css_returns_200_with_css_content_type() {
        let app = build_router(test_state());
        let response = app
            .oneshot(
                Request::get("/static/styles.css")
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
            ctype.contains("text/css"),
            "expected text/css content-type, got: {ctype}"
        );
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(!text.is_empty(), "styles.css body should not be empty");
        assert!(
            text.contains(".tag-edit"),
            "styles.css should contain the tag-edit rule"
        );
    }

    #[tokio::test]
    async fn favicon_returns_200_with_svg_content_type() {
        let app = build_router(test_state());
        let response = app
            .oneshot(
                Request::get("/favicon.ico")
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
            ctype.contains("image/svg+xml"),
            "expected image/svg+xml content-type, got: {ctype}"
        );
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(!text.is_empty(), "favicon body should not be empty");
        assert!(text.contains("<svg"), "favicon should be valid SVG");
    }

    #[tokio::test]
    async fn favicon_is_cacheable() {
        let app = build_router(test_state());
        let response = app
            .oneshot(
                Request::get("/favicon.ico")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let cache_control = response
            .headers()
            .get("cache-control")
            .expect("favicon should set cache-control")
            .to_str()
            .unwrap();
        assert!(
            cache_control.contains("max-age="),
            "favicon should be cacheable, got: {cache_control}"
        );
        assert!(
            !cache_control.contains("no-store"),
            "favicon should not be no-store, got: {cache_control}"
        );
    }

    #[tokio::test]
    async fn review_highlight_js_returns_200_with_javascript_content_type() {
        let app = build_router(test_state());
        let response = app
            .oneshot(
                Request::get("/static/review-highlight.js")
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
        let body = to_bytes(response.into_body(), 16384).await.unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(
            text.contains("data-rdm-anchor-ref"),
            "review-highlight.js should bind quote previews"
        );
        assert!(
            text.contains("rdm-anchor"),
            "review-highlight.js should toggle inline marks"
        );
    }

    #[tokio::test]
    async fn review_anchor_js_returns_200_with_javascript_content_type() {
        let app = build_router(test_state());
        let response = app
            .oneshot(
                Request::get("/static/review-anchor.js")
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
        let body = to_bytes(response.into_body(), 32768).await.unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(
            text.contains("data-rdm-annotated"),
            "review-anchor.js should target annotated bodies"
        );
        assert!(
            text.contains("data-rdm-anchor-action"),
            "review-anchor.js should read the panel's anchor endpoint"
        );
        assert!(
            text.contains("rendered_text"),
            "review-anchor.js should ship the selected text for the cross-check"
        );
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
