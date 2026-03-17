use axum::response::{IntoResponse, Response};
use serde::Serialize;

use rdm_core::hal::HalResource;

/// Content type for HAL+JSON responses.
const HAL_JSON: &str = "application/hal+json";

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
