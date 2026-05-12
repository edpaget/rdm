#![warn(missing_docs)]
//! rdm-server: REST API layer over rdm-core.
//!
//! The JSON API (HAL+JSON over `application/json`) is the source of truth for
//! every mutation. The HTML pages are a thin presentation layer; interactive
//! editing is delivered by a small embedded JavaScript client served at
//! `/static/edit.js` that intercepts `<form data-rdm-edit>` submissions and
//! PATCHes the resource as JSON. Users with JavaScript disabled retain
//! read-only access to every page.

/// Content negotiation via the `Accept` header.
pub mod content_type;
/// Error handling: core errors to RFC 9457 Problem Details responses.
pub mod error;
/// HAL+JSON response helpers and content negotiation guards.
pub mod extract;
/// HTTP request handlers.
pub mod handlers;
/// Markdown-to-HTML rendering.
pub mod markdown;
/// Axum router construction.
pub mod router;
/// Shared application state.
pub mod state;
/// Askama template structs for HTML pages.
pub mod templates;
