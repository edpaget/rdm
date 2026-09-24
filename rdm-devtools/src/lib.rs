//! Repository-only developer tooling for rdm.
//!
//! This crate is never published or packaged with the rdm release: it holds
//! reusable support for development-time checks that should not live in the
//! shipped `rdm` binary. Today that is [`process`], a bounded process runner
//! used by live smoke checks (for example the opt-in Codex coexistence check)
//! to guarantee that a child process group is terminated and reaped, and that a
//! private file copy is removed, on every catchable exit path;
//! [`workflow`], a repository-only binding (tests and measurement tools) that
//! executes the real Claude Workflow JavaScript sources under Node so Rust can
//! drive them; and [`measure`], the token/refuter measurement and corpus tools
//! behind the `rdm-measure` binary.

#![warn(missing_docs)]

#[cfg(not(unix))]
compile_error!(
    "rdm-devtools process support requires a unix target (process groups and SIGINT/SIGTERM)"
);

pub mod measure;
pub mod process;
pub mod workflow;
