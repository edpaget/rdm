use axum::response::Response;
use serde::Serialize;

use rdm_core::hal::{HalLink, HalResource};

use crate::content_type::ResponseFormat;
use crate::extract::{hal_response, require_hal_json};

/// Empty data for the root resource.
#[derive(Serialize)]
struct RootData {}

/// `GET /` — API root with discovery links.
pub async fn index(format: ResponseFormat) -> Result<Response, Response> {
    require_hal_json(format)?;

    let resource =
        HalResource::new(RootData {}, "/").with_link("projects", HalLink::new("/projects"));

    Ok(hal_response(resource))
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
        }
    }

    #[tokio::test]
    async fn root_returns_hal_with_links() {
        let app = build_router(test_state());
        let response = app
            .oneshot(
                Request::get("/")
                    .header("accept", "application/hal+json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/hal+json"
        );
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["_links"]["self"]["href"], "/");
        assert_eq!(json["_links"]["projects"]["href"], "/projects");
    }

    #[tokio::test]
    async fn root_returns_406_for_html() {
        let app = build_router(test_state());
        let response = app
            .oneshot(
                Request::get("/")
                    .header("accept", "text/html")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 406);
    }
}
