#![warn(missing_docs)]
//! rdm-core: data model, parsing, file I/O, and index generation for rdm.

/// Internal Markdown AST types for structured document generation.
pub mod ast;
/// Plan repo configuration (`rdm.toml`).
pub mod config;
/// Display formatting functions for roadmaps, phases, and projects.
pub mod display;
/// Generic document wrapper combining frontmatter with a markdown body.
pub mod document;
/// Error types for rdm-core.
pub mod error;
/// HAL (Hypertext Application Language) response types.
pub mod hal;

/// Markdown frontmatter splitting and joining utilities.
pub mod markdown;
/// Data model types for roadmaps, phases, and tasks.
pub mod model;
/// RFC 9457 Problem Details for HTTP APIs.
pub mod problem;
/// Plan repo operations: path resolution, file I/O, and initialization.
pub mod repo;
/// Fuzzy search across plan repo content (roadmaps, phases, and tasks).
pub mod search;
