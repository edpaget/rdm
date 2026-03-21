//! Display formatting and index generation for roadmaps, phases, and projects.
//!
//! - [`format`] — terminal formatting functions for human-readable output
//! - [`index`] — INDEX.md generation from pre-aggregated project data
mod format;
mod index;

pub use format::*;
pub use index::*;
