#![warn(missing_docs)]
//! rdm-core: data model, parsing, file I/O, and index generation for rdm.

/// Agent configuration generation for AI coding assistants.
pub mod agent_config;
/// Anchor resolution: locating a review comment's span in a body,
/// including after the body has been edited.
pub mod anchor;
/// Internal Markdown AST types for structured document generation.
pub mod ast;
/// Cross-repository change review: hunk-restricted anchoring of a review
/// comment to a quoted span of a source-repository file, and resolving it
/// again against a later tip.
pub mod change;
/// Plan repo configuration (`rdm.toml`).
pub mod config;
/// Conflict classification for merge conflict paths.
pub mod conflict;
/// Joining run records to session spend by time window: tokens per roadmap
/// and per phase.
pub mod cost_report;
/// Model introspection: discover what rdm tracks and the shape of each entity.
pub mod describe;
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
/// The `rdm:` link scheme: parsing item and code references, and
/// extracting them from markdown bodies.
pub mod link;
/// A best-effort, age-bounded advisory file lock, shared by the store's flush
/// precondition and the scoped-commit path.
pub mod lock;

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
/// Session identity and the per-session changeset journal.
pub mod session;
/// The read-only source-repository port a `change/<sha>` review's anchors
/// resolve against, plus an in-memory double for tests.
pub mod source;
/// Which source repository a command reads from, as a pure decision shared
/// by `rdm link check` and `rdm review --on change/…`.
pub mod source_select;
/// Storage abstraction layer for plan repo data.
pub mod store;
/// Reserved-tag primitives (e.g. the `needs-plan-review` sentinel).
pub mod tags;
/// Locating a Claude Code session on disk and building its anchored token
/// ledger, behind the read-only `TranscriptSource` port.
pub mod transcript;
/// Hierarchical tree view of plan repo contents.
pub mod tree;
/// Per-model, per-token-class token accounting over already-read transcript
/// text.
pub mod usage;
/// The read-only worktree port the `reviewed` transition gate's cleanliness
/// precondition reads through, plus a fail-closed `git status --porcelain`
/// parser and an in-memory double for tests.
pub mod worktree;

pub use transcript::{
    EntryKind, MemoryTranscriptSource, ModelRow, ReportScope, ReportWarning, SessionLocation,
    SessionReport, SourceKind, SourceReport, TranscriptEntry, TranscriptError, TranscriptPath,
    TranscriptSource, UnanchoredEntry, UnanchoredKind, UsageSummary, WarningScope,
    WorkflowRunLocation, WorkflowRunReport, locate_session, locate_workflow_run, reap_session,
    reap_workflow_run,
};
pub use usage::{
    ModelUsage, ParsedTranscript, RequestUsage, TokenUsage, UsageLedger, UsageWarning,
    parse_transcript,
};
pub use worktree::{ReviewSource, ReviewSourceRequest, resolve_review_source};
