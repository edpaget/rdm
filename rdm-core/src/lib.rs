#![warn(missing_docs)]
//! rdm-core: data model, parsing, file I/O, and index generation for rdm.

/// Agent configuration generation for AI coding assistants.
pub mod agent_config;
/// Anchor resolution: locating a review comment's span in a body,
/// including after the body has been edited.
pub mod anchor;
/// Internal Markdown AST types for structured document generation.
pub mod ast;
/// Plan repo configuration (`rdm.toml`).
pub mod config;
/// Conflict classification for merge conflict paths.
pub mod conflict;
/// Model introspection: discover what rdm tracks and the shape of each entity.
pub mod describe;
/// Project-authored dispatch directives: discovery, verbatim reading, and bounding.
pub mod directives;
/// Display formatting functions for roadmaps, phases, and projects.
pub mod display;
/// Generic document wrapper combining frontmatter with a markdown body.
pub mod document;
/// Error types for rdm-core.
pub mod error;
/// Git hook helpers for parsing `Done:` directives from commit messages.
pub mod hook;
/// Document I/O primitives for plan repo data.
pub mod io;
/// Serializable JSON output types for CLI and API consumers.
pub mod json;

/// Markdown frontmatter splitting and joining utilities.
pub mod markdown;
/// Data model types for roadmaps, phases, and tasks.
pub mod model;
/// Model-tier sizing policy: resolves a dispatch step (plus an optional
/// caller hint) to a concrete model id via the `[models]` config.
pub mod model_policy;
/// Domain operations for plan repo entities.
pub mod ops;
/// Path builders for plan repo layout.
pub mod paths;
/// Plan repo root resolution: locating the plan repo directory and expanding
/// path shorthand (`~`, `.`, `..`).
pub mod root;
/// Fuzzy search across plan repo content (roadmaps, phases, and tasks).
pub mod search;
/// Storage abstraction layer for plan repo data.
pub mod store;
/// Reserved-tag primitives (e.g. the `needs-plan-review` sentinel).
pub mod tags;
/// Hierarchical tree view of plan repo contents.
pub mod tree;
