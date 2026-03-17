use axum::response::{IntoResponse, Response};
use serde::Serialize;

use rdm_core::hal::HalResource;
use rdm_core::problem::ProblemDetail;

use crate::content_type::ResponseFormat;
use crate::error::problem_detail_into_response;

/// Content type for HAL+JSON responses.
const HAL_JSON: &str = "application/hal+json";

/// Returns a 406 Not Acceptable response if the client explicitly requests HTML.
///
/// Phase 2 only serves HAL+JSON. Call this at the top of each handler.
///
/// # Errors
///
/// Returns a [`Response`] with a Problem Details body when `format` is
/// [`ResponseFormat::Html`].
#[allow(clippy::result_large_err)]
pub fn require_hal_json(format: ResponseFormat) -> Result<(), Response> {
    if format == ResponseFormat::Html {
        let pd = ProblemDetail {
            problem_type: "about:blank".to_string(),
            title: "Not Acceptable".to_string(),
            status: 406,
            detail: Some("this endpoint only supports application/hal+json".to_string()),
            instance: None,
        };
        return Err(problem_detail_into_response(pd));
    }
    Ok(())
}

/// Serializes a [`HalResource`] into a `200 OK` response with
/// `Content-Type: application/hal+json`.
pub fn hal_response<T: Serialize>(resource: HalResource<T>) -> Response {
    let body = serde_json::to_string(&resource).expect("HalResource serialization cannot fail");
    (
        axum::http::StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, HAL_JSON)],
        body,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    #[test]
    fn require_hal_json_accepts_hal() {
        assert!(require_hal_json(ResponseFormat::HalJson).is_ok());
    }

    #[tokio::test]
    async fn require_hal_json_rejects_html() {
        let err = require_hal_json(ResponseFormat::Html).unwrap_err();
        assert_eq!(err.status(), axum::http::StatusCode::NOT_ACCEPTABLE);
        let body = to_bytes(err.into_body(), 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], 406);
    }

    #[tokio::test]
    async fn hal_response_sets_content_type() {
        #[derive(Serialize)]
        struct Empty {}
        let resource = HalResource::new(Empty {}, "/");
        let response = hal_response(resource);
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/hal+json"
        );
    }
}
