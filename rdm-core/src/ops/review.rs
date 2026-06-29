//! Review operations: enumerate items awaiting review.

use serde::Serialize;

use crate::error::Result;
use crate::model::{PhaseStatus, TaskStatus};
use crate::store::Store;

/// The kind of plan item awaiting review.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PendingReviewKind {
    /// A roadmap phase.
    Phase,
    /// A standalone task.
    Task,
}

/// A single plan item in the `needs-review` state.
///
/// `identifier` is the canonical reference used by other rdm commands:
/// `roadmap/stem` for a phase, or the task slug for a task. `review_sha` is the
/// source-repo HEAD SHA stamped when the item entered `needs-review`, or `None`
/// for legacy / pre-stamp items (which callers should treat as "always in
/// scope").
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PendingReviewItem {
    /// Whether this item is a phase or a task.
    pub kind: PendingReviewKind,
    /// Canonical reference: `roadmap/stem` for phases, slug for tasks.
    pub identifier: String,
    /// Project the item belongs to.
    pub project: String,
    /// Human-readable title.
    pub title: String,
    /// Source-repo HEAD SHA stamped at the `needs-review` transition, if any.
    pub review_sha: Option<String>,
    /// Branch name of the checkout that produced the review, stamped at the
    /// `needs-review` transition, or `None` for legacy / pre-stamp items.
    /// Callers scope by identity (matching this against the firing checkout's
    /// current branch); legacy items fall back to `review_sha` reachability.
    pub review_branch: Option<String>,
}

/// Lists every phase and task in `project` whose status is `needs-review`.
///
/// Phases are reported with identifier `roadmap/stem`; tasks with their slug.
/// Each item carries its stamped `review_branch` and `review_sha` (if any) so
/// callers can scope the list to the checkout that produced it — by branch
/// identity, falling back to SHA reachability for legacy items. Git is
/// intentionally not consulted here — that filtering belongs to the caller
/// (the CLI).
///
/// # Errors
///
/// Returns [`Error::ProjectNotFound`](crate::error::Error::ProjectNotFound) if
/// the project doesn't exist, [`Error::Io`](crate::error::Error::Io) if a
/// directory cannot be read, or
/// [`Error::FrontmatterMissing`](crate::error::Error::FrontmatterMissing) /
/// [`Error::FrontmatterParse`](crate::error::Error::FrontmatterParse) if any
/// item file has invalid frontmatter.
pub fn pending_review_items(store: &impl Store, project: &str) -> Result<Vec<PendingReviewItem>> {
    let mut items = Vec::new();

    let roadmaps = crate::ops::roadmap::list_roadmaps(store, project, None, None)?;
    for roadmap_doc in roadmaps {
        let roadmap = &roadmap_doc.frontmatter.roadmap;
        let phases = crate::ops::phase::list_phases(store, project, roadmap)?;
        for (stem, doc) in phases {
            if doc.frontmatter.status == PhaseStatus::NeedsReview {
                items.push(PendingReviewItem {
                    kind: PendingReviewKind::Phase,
                    identifier: format!("{roadmap}/{stem}"),
                    project: project.to_string(),
                    title: doc.frontmatter.title,
                    review_sha: doc.frontmatter.review_sha,
                    review_branch: doc.frontmatter.review_branch,
                });
            }
        }
    }

    let tasks = crate::ops::task::list_tasks(store, project)?;
    for (slug, doc) in tasks {
        if doc.frontmatter.status == TaskStatus::NeedsReview {
            items.push(PendingReviewItem {
                kind: PendingReviewKind::Task,
                identifier: slug,
                project: project.to_string(),
                title: doc.frontmatter.title,
                review_sha: doc.frontmatter.review_sha,
                review_branch: doc.frontmatter.review_branch,
            });
        }
    }

    Ok(items)
}

/// A phase parked as `blocked` — an escalation awaiting a human decision.
///
/// `identifier` is the canonical `roadmap/stem` reference used by other rdm
/// commands. `reason` is the escalation note recorded when the phase was
/// blocked, or `None` if it was blocked without one (e.g. a legacy item or a
/// status set directly).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BlockedPhaseItem {
    /// Canonical reference: `roadmap/stem`.
    pub identifier: String,
    /// Project the phase belongs to.
    pub project: String,
    /// Human-readable title.
    pub title: String,
    /// The recorded escalation reason, if any.
    pub reason: Option<String>,
}

/// Lists every phase in `project` whose status is `blocked`, with its recorded
/// escalation reason.
///
/// This is the batch escalation queue: a single command surfaces every parked
/// decision/blocker so a human can answer them together rather than being
/// interrupted mid-run. Tasks are excluded — only phases (the unit of dispatch)
/// carry a `blocked` status.
///
/// # Errors
///
/// Returns [`Error::ProjectNotFound`](crate::error::Error::ProjectNotFound) if
/// the project doesn't exist, [`Error::Io`](crate::error::Error::Io) if a
/// directory cannot be read, or
/// [`Error::FrontmatterMissing`](crate::error::Error::FrontmatterMissing) /
/// [`Error::FrontmatterParse`](crate::error::Error::FrontmatterParse) if any
/// phase file has invalid frontmatter.
pub fn blocked_phases(store: &impl Store, project: &str) -> Result<Vec<BlockedPhaseItem>> {
    let mut items = Vec::new();

    let roadmaps = crate::ops::roadmap::list_roadmaps(store, project, None, None)?;
    for roadmap_doc in roadmaps {
        let roadmap = &roadmap_doc.frontmatter.roadmap;
        let phases = crate::ops::phase::list_phases(store, project, roadmap)?;
        for (stem, doc) in phases {
            if doc.frontmatter.status == PhaseStatus::Blocked {
                items.push(BlockedPhaseItem {
                    identifier: format!("{roadmap}/{stem}"),
                    project: project.to_string(),
                    title: doc.frontmatter.title,
                    reason: doc.frontmatter.blocked_reason,
                });
            }
        }
    }

    Ok(items)
}
