use axum::http::StatusCode;
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
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, HAL_JSON)],
        body,
    )
        .into_response()
}

/// Serializes a [`HalResource`] into a `201 Created` response with
/// `Content-Type: application/hal+json` and a `Location` header.
pub fn hal_created_response<T: Serialize>(resource: HalResource<T>, location: &str) -> Response {
    let body = serde_json::to_string(&resource).expect("HalResource serialization cannot fail");
    (
        StatusCode::CREATED,
        [
            (axum::http::header::CONTENT_TYPE, HAL_JSON),
            (axum::http::header::LOCATION, location),
        ],
        body,
    )
        .into_response()
}

/// Returns a `303 See Other` response with a `Location` header.
pub fn see_other_response(location: &str) -> Response {
    (
        StatusCode::SEE_OTHER,
        [(axum::http::header::LOCATION, location)],
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
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/hal+json"
        );
    }

    #[tokio::test]
    async fn hal_created_response_sets_status_and_location() {
        #[derive(Serialize)]
        struct Empty {}
        let resource = HalResource::new(Empty {}, "/things/1");
        let response = hal_created_response(resource, "/things/1");
        assert_eq!(response.status(), StatusCode::CREATED);
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/hal+json"
        );
        assert_eq!(response.headers().get("location").unwrap(), "/things/1");
    }

    #[tokio::test]
    async fn see_other_response_sets_status_and_location() {
        let response = see_other_response("/destination");
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(response.headers().get("location").unwrap(), "/destination");
    }
}
