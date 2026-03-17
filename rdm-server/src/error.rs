//! Error handling: core errors to RFC 9457 Problem Details responses or HTML error pages.

use askama::Template;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use rdm_core::problem::ProblemDetail;

use axum::extract::rejection::JsonRejection;

use crate::content_type::ResponseFormat;
use crate::templates::ErrorPage;

/// Content type for RFC 9457 Problem Details responses.
const PROBLEM_JSON: &str = "application/problem+json";

/// Converts a [`ProblemDetail`] into an axum [`Response`] with JSON body.
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

/// Returns an error response in the appropriate format (JSON Problem Details or HTML error page).
pub fn error_response(err: rdm_core::error::Error, format: ResponseFormat) -> Response {
    let pd = ProblemDetail::from(&err);
    match format {
        ResponseFormat::HalJson => problem_detail_into_response(pd),
        ResponseFormat::Html => {
            let status =
                StatusCode::from_u16(pd.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            let page = ErrorPage {
                status: pd.status,
                title: pd.title,
                detail: pd.detail,
            };
            match page.render() {
                Ok(html) => (
                    status,
                    [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
                    html,
                )
                    .into_response(),
                Err(_) => problem_detail_into_response(ProblemDetail::from(&err)),
            }
        }
    }
}

/// Returns a `422 Unprocessable Content` response with the given detail message.
pub fn validation_error(detail: String) -> Response {
    problem_detail_into_response(ProblemDetail {
        problem_type: "about:blank".to_string(),
        title: "Unprocessable Content".to_string(),
        status: 422,
        detail: Some(detail),
        instance: None,
    })
}

/// Wraps an axum [`JsonRejection`] as a `422 Unprocessable Content` Problem Details response.
pub fn json_rejection_response(rejection: JsonRejection) -> Response {
    validation_error(rejection.body_text())
}

/// Wrapper around [`rdm_core::error::Error`] that implements [`IntoResponse`].
///
/// Defaults to JSON Problem Details. For format-aware errors, use [`error_response`] directly.
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

    #[tokio::test]
    async fn error_response_html_returns_html() {
        let err = rdm_core::error::Error::ProjectNotFound("demo".to_string());
        let response = error_response(err, ResponseFormat::Html);
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert!(
            response
                .headers()
                .get("content-type")
                .unwrap()
                .to_str()
                .unwrap()
                .contains("text/html")
        );
        let body = to_bytes(response.into_body(), 8192).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("404"));
        assert!(html.contains("Not Found"));
    }

    #[tokio::test]
    async fn validation_error_returns_422() {
        let response = validation_error("field is required".to_string());
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/problem+json"
        );
        let body = to_bytes(response.into_body(), 8192).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], 422);
        assert_eq!(json["title"], "Unprocessable Content");
        assert!(
            json["detail"]
                .as_str()
                .unwrap()
                .contains("field is required")
        );
    }

    #[tokio::test]
    async fn error_response_json_returns_problem_details() {
        let err = rdm_core::error::Error::ProjectNotFound("demo".to_string());
        let response = error_response(err, ResponseFormat::HalJson);
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/problem+json"
        );
    }
}
