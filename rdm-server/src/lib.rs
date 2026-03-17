#![warn(missing_docs)]
//! rdm-server: REST API layer over rdm-core.

/// Content negotiation via the `Accept` header.
pub mod content_type;
/// Error handling: core errors to RFC 9457 Problem Details responses.
pub mod error;
/// HAL+JSON response helpers and content negotiation guards.
pub mod extract;
/// HTTP request handlers.
pub mod handlers;
/// Axum router construction.
pub mod router;
/// Shared application state.
pub mod state;
