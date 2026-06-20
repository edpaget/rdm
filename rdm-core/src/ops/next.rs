//! Next-actionable-phase selection.
//!
//! Resolves, deterministically and side-effect-free, the single phase that
//! should be worked on next *within one roadmap*. An autonomous roadmap loop
//! drives off this contract instead of eyeballing `phase list`.
//!
//! Scope is one roadmap at a time: callers name the roadmap. Choosing *which*
//! roadmap to advance stays a human decision — there is no project-wide scan or
//! cross-roadmap traversal here.

use serde::Serialize;

use crate::error::{Error, Result};
use crate::model::{Difficulty, ModelTier, PhaseStatus};
use crate::ops::roadmap::{RoadmapStatus, computed_status};
use crate::store::Store;

/// A phase selected as the next actionable item in a roadmap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NextPhase {
    /// Roadmap slug the phase belongs to.
    pub roadmap: String,
    /// Phase number (1-based ordering).
    pub number: u32,
    /// Phase file stem (e.g. `phase-2-indexing`).
    pub stem: String,
    /// Current status (`not-started` or `in-progress`).
    pub status: PhaseStatus,
    /// Estimated difficulty, if assessed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub difficulty: Option<Difficulty>,
    /// Model tier that should run the phase, if assigned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelTier>,
}

/// The result of resolving the next actionable phase in a roadmap.
///
/// Serializes with an internally-tagged `result` discriminator:
/// - [`NextActionable::Phase`] → `{"result":"phase", ...NextPhase fields}`
/// - [`NextActionable::BlockedOnDependencies`] →
///   `{"result":"blocked-on-dependencies","unmet":[...]}`
/// - [`NextActionable::Nothing`] → `{"result":"nothing"}`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "result", rename_all = "kebab-case")]
pub enum NextActionable {
    /// The next phase to work on.
    Phase(NextPhase),
    /// One or more dependency roadmaps are not yet complete; no phase in this
    /// roadmap should be started. `unmet` lists the blocking dependency slugs
    /// in declaration order.
    BlockedOnDependencies {
        /// Dependency roadmap slugs that are not yet complete (in order).
        unmet: Vec<String>,
    },
    /// Dependencies (if any) are met, but no phase is actionable — every phase
    /// is terminal or otherwise non-actionable.
    Nothing,
}

/// Resolves the next actionable phase in a single roadmap.
///
/// The dependencies gate is checked first: for each slug in the roadmap's
/// `dependencies`, the dependency is *met* iff that roadmap exists and every
/// one of its phases is terminal ([`RoadmapStatus::Done`]). A missing
/// dependency roadmap counts as unmet. If any dependency is unmet, returns
/// [`NextActionable::BlockedOnDependencies`] with the unmet slugs in
/// declaration order.
///
/// Otherwise the phases are scanned in number order and the lowest-numbered
/// phase whose status is `not-started` or `in-progress` is returned (an
/// in-progress phase naturally outranks a later not-started one because it has
/// the lower number). Phases in `needs-review`, `reviewed`, `done`, `blocked`,
/// or `wont-fix` are skipped. If none qualify, returns
/// [`NextActionable::Nothing`].
///
/// # Errors
///
/// Returns [`Error::RoadmapNotFound`] if the named `roadmap` does not exist.
/// Propagates [`Error::Io`] and
/// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] from reading the
/// roadmap or its phase files. A missing *dependency* roadmap is never an
/// error — it is treated as an unmet dependency.
pub fn next_actionable(store: &impl Store, project: &str, roadmap: &str) -> Result<NextActionable> {
    let doc = crate::io::load_roadmap(store, project, roadmap)?;

    // Dependencies gate first.
    if let Some(deps) = &doc.frontmatter.dependencies {
        let mut unmet = Vec::new();
        for dep in deps {
            if !dependency_met(store, project, dep)? {
                unmet.push(dep.clone());
            }
        }
        if !unmet.is_empty() {
            return Ok(NextActionable::BlockedOnDependencies { unmet });
        }
    }

    // Lowest-numbered actionable phase.
    let phases = super::phase::list_phases(store, project, roadmap)?;
    for (stem, phase_doc) in phases {
        let status = phase_doc.frontmatter.status;
        if matches!(status, PhaseStatus::NotStarted | PhaseStatus::InProgress) {
            return Ok(NextActionable::Phase(NextPhase {
                roadmap: roadmap.to_string(),
                number: phase_doc.frontmatter.phase,
                stem,
                status,
                difficulty: phase_doc.frontmatter.difficulty,
                model: phase_doc.frontmatter.model,
            }));
        }
    }

    Ok(NextActionable::Nothing)
}

/// Returns whether a dependency roadmap is complete (all phases terminal).
///
/// A missing dependency roadmap is treated as unmet rather than an error: the
/// `RoadmapNotFound` from `list_phases` is caught and folded into `Ok(false)`.
fn dependency_met(store: &impl Store, project: &str, dep: &str) -> Result<bool> {
    match super::phase::list_phases(store, project, dep) {
        Ok(phases) => {
            let statuses: Vec<PhaseStatus> =
                phases.iter().map(|(_, d)| d.frontmatter.status).collect();
            Ok(computed_status(&statuses) == RoadmapStatus::Done)
        }
        Err(Error::RoadmapNotFound(_)) => Ok(false),
        Err(e) => Err(e),
    }
}
