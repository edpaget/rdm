use std::str::FromStr;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;

/// The response format negotiated from the `Accept` header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseFormat {
    /// `application/hal+json`
    HalJson,
    /// `text/html`
    Html,
}

impl<S: Send + Sync> FromRequestParts<S> for ResponseFormat {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let accept_header = parts
            .headers
            .get(axum::http::header::ACCEPT)
            .and_then(|v| v.to_str().ok());

        let Some(accept_str) = accept_header else {
            return Ok(ResponseFormat::Html);
        };

        let Ok(accept) = headers_accept::Accept::from_str(accept_str) else {
            return Ok(ResponseFormat::Html);
        };

        let hal_json: mediatype::MediaType =
            mediatype::MediaType::parse("application/hal+json").expect("valid media type");
        let text_html: mediatype::MediaType =
            mediatype::MediaType::parse("text/html").expect("valid media type");
        // text/html listed first so it wins on wildcard ties (browser-friendly default)
        let available = [text_html, hal_json.clone()];

        match accept.negotiate(&available) {
            Some(matched) if *matched == hal_json => Ok(ResponseFormat::HalJson),
            _ => Ok(ResponseFormat::Html),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;

    async fn negotiate(accept: Option<&str>) -> ResponseFormat {
        let mut builder = Request::builder().method("GET").uri("/");
        if let Some(val) = accept {
            builder = builder.header("accept", val);
        }
        let req = builder.body(()).unwrap();
        let (mut parts, _) = req.into_parts();
        ResponseFormat::from_request_parts(&mut parts, &())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn accept_hal_json() {
        assert_eq!(
            negotiate(Some("application/hal+json")).await,
            ResponseFormat::HalJson
        );
    }

    #[tokio::test]
    async fn accept_text_html() {
        assert_eq!(negotiate(Some("text/html")).await, ResponseFormat::Html);
    }

    #[tokio::test]
    async fn accept_missing() {
        assert_eq!(negotiate(None).await, ResponseFormat::Html);
    }

    #[tokio::test]
    async fn accept_wildcard() {
        assert_eq!(negotiate(Some("*/*")).await, ResponseFormat::Html);
    }

    #[tokio::test]
    async fn accept_hal_json_with_quality() {
        assert_eq!(
            negotiate(Some("application/hal+json;q=1, text/html;q=0.9")).await,
            ResponseFormat::HalJson
        );
    }

    #[tokio::test]
    async fn accept_html_preferred() {
        assert_eq!(
            negotiate(Some("text/html, application/hal+json;q=0.5")).await,
            ResponseFormat::Html
        );
    }
}
