use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use rdm_core::problem::ProblemDetail;

/// Content type for RFC 9457 Problem Details responses.
const PROBLEM_JSON: &str = "application/problem+json";

/// Converts a [`ProblemDetail`] into an axum [`Response`].
pub(crate) fn problem_detail_into_response(pd: ProblemDetail) -> Response {
    let status = StatusCode::from_u16(pd.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let body = serde_json::to_string(&pd).expect("ProblemDetail serialization cannot fail");

    (
        status,
        [(axum::http::header::CONTENT_TYPE, PROBLEM_JSON)],
        body,
    )
        .into_response()
}

/// Wrapper around [`rdm_core::error::Error`] that implements [`IntoResponse`].
pub struct AppError(pub rdm_core::error::Error);

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let problem = ProblemDetail::from(&self.0);
        problem_detail_into_response(problem)
    }
}

impl From<rdm_core::error::Error> for AppError {
    fn from(err: rdm_core::error::Error) -> Self {
        AppError(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    #[tokio::test]
    async fn problem_detail_response_status() {
        let pd = ProblemDetail {
            problem_type: "about:blank".to_string(),
            title: "Not Found".to_string(),
            status: 404,
            detail: Some("gone".to_string()),
            instance: None,
        };
        let response = problem_detail_into_response(pd);
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn problem_detail_response_content_type() {
        let pd = ProblemDetail {
            problem_type: "about:blank".to_string(),
            title: "Bad Request".to_string(),
            status: 400,
            detail: None,
            instance: None,
        };
        let response = problem_detail_into_response(pd);
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/problem+json"
        );
    }

    #[tokio::test]
    async fn app_error_from_core_error() {
        let err = rdm_core::error::Error::ProjectNotFound("demo".to_string());
        let app_err = AppError::from(err);
        let response = app_err.into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], 404);
        assert!(json["detail"].as_str().unwrap().contains("demo"));
    }
}
