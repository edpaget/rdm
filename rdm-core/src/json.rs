/// Serializable JSON output types for CLI and API consumers.
///
/// These structs combine frontmatter fields with contextual identifiers
/// (slug, stem, project, roadmap) and optional body content, producing
/// a stable JSON contract for scripts and agents.
use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;

use crate::anchor::{Resolution, ResolvedComment};
use crate::document::Document;
use crate::link::{BacklinkEntry, DocRef, Resolved};
use crate::model::{
    Anchor, CommentDoc, Difficulty, GateOverride, ModelTier, Phase, PhaseStatus, Plan, PlanStatus,
    Priority, Project, Review, ReviewCommentStatus, ReviewState, ReviewTarget, Roadmap, Task,
    TaskStatus, Verdict,
};
use crate::search::{ItemKind, SearchResult};

// ---------------------------------------------------------------------------
// Show types (single item with body)
// ---------------------------------------------------------------------------

/// Full roadmap detail, including nested phase summaries and body.
#[derive(Debug, Clone, Serialize)]
pub struct RoadmapJson {
    /// Project the roadmap belongs to.
    pub project: String,
    /// Roadmap slug identifier.
    pub slug: String,
    /// Human-readable title.
    pub title: String,
    /// Phase summaries in order (without body content — use `phase show` for full details).
    pub phases: Vec<PhaseSummaryJson>,
    /// Roadmap slugs this depends on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependencies: Option<Vec<String>>,
    /// Priority level, if set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<Priority>,
    /// Tags for categorization.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    /// Markdown body content.
    pub body: String,
    /// Git revision the body was read from (only set when this view was
    /// requested at a specific historical SHA).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
}

/// Full phase detail with body.
#[derive(Debug, Clone, Serialize)]
pub struct PhaseJson {
    /// File-stem (e.g. `phase-1-design`).
    pub stem: String,
    /// Phase number.
    pub phase: u32,
    /// Human-readable title.
    pub title: String,
    /// Current status.
    pub status: PhaseStatus,
    /// Tags for categorization.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    /// Completion date, if done.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed: Option<NaiveDate>,
    /// Git commit SHA associated with phase completion, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// Estimated difficulty of the phase, if assessed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub difficulty: Option<Difficulty>,
    /// Model tier that should run the phase, if assigned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelTier>,
    /// Reason the phase was parked as `blocked` (an escalation note), if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    /// An operator's recorded bypass of the `reviewed` transition gate, if
    /// one authorized this phase's current status. Omitted when absent, so a
    /// never-overridden phase serializes exactly as it did before the gate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gate_override: Option<GateOverride>,
    /// Git revision the body was read from (only set when this view was
    /// requested at a specific historical SHA).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    /// Parent roadmap slug.
    pub roadmap: String,
    /// Stem of the previous phase, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prev_phase: Option<String>,
    /// Stem of the next phase, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_phase: Option<String>,
    /// Implementation plans that implement this phase. Omitted entirely when
    /// empty, so a phase with no plan serializes exactly as it did before
    /// plans existed.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub plans: Vec<PlanRefJson>,
    /// Markdown body content.
    pub body: String,
}

impl PhaseJson {
    /// Attaches the plans implementing this phase.
    ///
    /// A builder rather than a `phase_to_json` parameter so every existing
    /// caller stays untouched and the common no-plan case keeps the exact
    /// bytes it had.
    #[must_use]
    pub fn with_plans(mut self, plans: Vec<PlanRefJson>) -> Self {
        self.plans = plans;
        self
    }
}

/// Full task detail with body.
#[derive(Debug, Clone, Serialize)]
pub struct TaskJson {
    /// Task slug.
    pub slug: String,
    /// Project the task belongs to.
    pub project: String,
    /// Human-readable title.
    pub title: String,
    /// Current status.
    pub status: TaskStatus,
    /// Priority level.
    pub priority: Priority,
    /// Creation date.
    pub created: NaiveDate,
    /// Tags for categorization.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    /// Date the task was completed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed: Option<NaiveDate>,
    /// Git commit SHA that completed this task.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// Reason the task was closed (a retire/supersede note), if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub close_reason: Option<String>,
    /// An operator's recorded bypass of the `reviewed` transition gate, if
    /// one authorized this task's current status. Omitted when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gate_override: Option<GateOverride>,
    /// Implementation plans that implement this task. Omitted entirely when
    /// empty, so a task with no plan serializes exactly as it did before
    /// plans existed.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub plans: Vec<PlanRefJson>,
    /// Markdown body content.
    pub body: String,
    /// Git revision the body was read from (only set when this view was
    /// requested at a specific historical SHA).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
}

impl TaskJson {
    /// Attaches the plans implementing this task.
    ///
    /// A builder rather than a `task_to_json` parameter so every existing
    /// caller stays untouched and the common no-plan case keeps the exact
    /// bytes it had.
    #[must_use]
    pub fn with_plans(mut self, plans: Vec<PlanRefJson>) -> Self {
        self.plans = plans;
        self
    }
}

/// Full implementation-plan detail with body and the reviews on it.
#[derive(Debug, Clone, Serialize)]
pub struct PlanJson {
    /// Plan slug.
    pub slug: String,
    /// Project the plan belongs to.
    pub project: String,
    /// Human-readable title.
    pub title: String,
    /// Current status (derived from reviews).
    pub status: PlanStatus,
    /// The phase or task this plan implements, as the canonical `rdm:` URI.
    pub implements: String,
    /// The earlier plan this one replaces, as the canonical `rdm:` URI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<String>,
    /// Creation date.
    pub created: NaiveDate,
    /// Last-modified date.
    pub updated: NaiveDate,
    /// Markdown body content.
    pub body: String,
    /// Reviews targeting this plan, in id order. Omitted when empty.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reviews: Vec<PlanReviewRefJson>,
    /// `change/<sha>` reviews whose `implements` names this plan, in id
    /// order. Omitted when empty.
    ///
    /// Deliberately distinct from `reviews`: that list is reviews **on**
    /// this plan document (feedback about the plan), while this one is
    /// reviews of the **code** written against it. Conflating them would
    /// make `rdm plan show`'s verdict summary meaningless.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub change_reviews: Vec<PlanChangeReviewJson>,
}

impl PlanJson {
    /// Attaches the change reviews implementing this plan.
    ///
    /// A builder rather than a [`plan_to_json`] parameter so every existing
    /// caller stays untouched and a plan with no change reviews keeps the
    /// exact bytes it had.
    #[must_use]
    pub fn with_change_reviews(mut self, change_reviews: Vec<PlanChangeReviewJson>) -> Self {
        self.change_reviews = change_reviews;
        self
    }
}

/// Plan summary for list output (no body, no reviews).
#[derive(Debug, Clone, Serialize)]
pub struct PlanSummaryJson {
    /// Plan slug.
    pub slug: String,
    /// Human-readable title.
    pub title: String,
    /// Current status.
    pub status: PlanStatus,
    /// The phase or task this plan implements, as the canonical `rdm:` URI.
    pub implements: String,
    /// The earlier plan this one replaces, as the canonical `rdm:` URI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<String>,
    /// Creation date.
    pub created: NaiveDate,
    /// Last-modified date.
    pub updated: NaiveDate,
}

/// A back-reference to a plan, embedded in the phase or task it implements.
#[derive(Debug, Clone, Serialize)]
pub struct PlanRefJson {
    /// Plan slug.
    pub slug: String,
    /// Human-readable title.
    pub title: String,
    /// Current status.
    pub status: PlanStatus,
}

/// A back-reference to a review, embedded in the plan it targets.
#[derive(Debug, Clone, Serialize)]
pub struct PlanReviewRefJson {
    /// Review id.
    pub id: String,
    /// Who authored the review.
    pub author: String,
    /// Lifecycle state.
    pub state: ReviewState,
    /// Verdict stamped on submit; absent on drafts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verdict: Option<Verdict>,
    /// When the review was submitted; absent on drafts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub submitted: Option<DateTime<Utc>>,
}

/// A reference to a `change/<sha>` review implementing a plan, embedded in
/// that plan's detail output.
#[derive(Debug, Clone, Serialize)]
pub struct PlanChangeReviewJson {
    /// Review id.
    pub id: String,
    /// The reviewed change's head SHA.
    pub head: String,
    /// Lifecycle state.
    pub state: ReviewState,
    /// Verdict stamped on submit; absent on drafts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verdict: Option<Verdict>,
    /// When the review was submitted; absent on drafts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub submitted: Option<DateTime<Utc>>,
}

/// Maps a `(id, Document<Review>)` pair into a [`PlanChangeReviewJson`].
///
/// Returns `None` for a review whose target is not a
/// [`ReviewTarget::Change`] — a caller that fed it a plain document review
/// gets nothing rather than a row with an invented head.
#[must_use]
pub fn plan_change_review_to_json(
    id: &str,
    doc: &Document<Review>,
) -> Option<PlanChangeReviewJson> {
    let ReviewTarget::Change { head, .. } = &doc.frontmatter.target else {
        return None;
    };
    Some(PlanChangeReviewJson {
        id: id.to_string(),
        head: head.clone(),
        state: doc.frontmatter.state,
        verdict: doc.frontmatter.verdict,
        submitted: doc.frontmatter.submitted,
    })
}

// ---------------------------------------------------------------------------
// List types (summaries without body)
// ---------------------------------------------------------------------------

/// Roadmap summary for list output.
#[derive(Debug, Clone, Serialize)]
pub struct RoadmapSummaryJson {
    /// Roadmap slug.
    pub slug: String,
    /// Human-readable title.
    pub title: String,
    /// Total number of phases.
    pub total_phases: usize,
    /// Number of completed phases.
    pub done_phases: usize,
    /// Progress as a human-readable string (e.g. "2/5 done").
    pub progress: String,
    /// Priority level, if set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<Priority>,
    /// Tags for categorization.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

/// Phase summary for list output.
#[derive(Debug, Clone, Serialize)]
pub struct PhaseSummaryJson {
    /// Phase number.
    pub number: u32,
    /// File-stem (e.g. `phase-1-design`).
    pub stem: String,
    /// Human-readable title.
    pub title: String,
    /// Current status.
    pub status: PhaseStatus,
    /// Tags for categorization.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    /// Estimated difficulty of the phase, if assessed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub difficulty: Option<Difficulty>,
    /// Model tier that should run the phase, if assigned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelTier>,
}

/// Task summary for list output.
#[derive(Debug, Clone, Serialize)]
pub struct TaskSummaryJson {
    /// Task slug.
    pub slug: String,
    /// Human-readable title.
    pub title: String,
    /// Current status.
    pub status: TaskStatus,
    /// Priority level.
    pub priority: Priority,
    /// Creation date.
    pub created: NaiveDate,
    /// Tags for categorization.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

// ---------------------------------------------------------------------------
// Project types
// ---------------------------------------------------------------------------

/// Full project detail with body.
#[derive(Debug, Clone, Serialize)]
pub struct ProjectJson {
    /// Project slug.
    pub name: String,
    /// Human-readable title.
    pub title: String,
    /// Markdown body content.
    pub body: String,
}

// ---------------------------------------------------------------------------
// Info types
// ---------------------------------------------------------------------------

/// What `rdm` actually resolved for the current environment: the plan repo
/// root, the selected project (if any), and the defaults that would govern
/// an unqualified command. Powers `rdm info --format json`, the single-call
/// discovery contract an editor or plugin integration resolves
/// `{root, project, default_branch, default_format}` against.
#[derive(Debug, Clone, Serialize)]
pub struct InfoJson {
    /// The resolved plan repo root path.
    pub root: String,
    /// The resolved project, if one could be determined. Omitted (not
    /// `null`) when no project resolves, so a plugin can detect "no project
    /// selected here" by the key's absence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// The default branch that would govern hook/land-time behavior.
    pub default_branch: String,
    /// The default output format that would govern an unqualified command.
    pub default_format: String,
}

// ---------------------------------------------------------------------------
// Search types
// ---------------------------------------------------------------------------

/// A single search result in JSON format.
#[derive(Debug, Clone, Serialize)]
pub struct SearchResultJson {
    /// The kind of item matched.
    pub kind: ItemKind,
    /// Identifier for the item.
    pub identifier: String,
    /// The project this item belongs to.
    pub project: String,
    /// The item's title.
    pub title: String,
    /// A short text snippet showing the match context.
    pub snippet: String,
    /// Match score (higher is better).
    pub score: u32,
    /// Tags carried by the matched item, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

// ---------------------------------------------------------------------------
// Conversion helpers
// ---------------------------------------------------------------------------

/// Build a [`RoadmapJson`] from a roadmap document and its loaded phases.
///
/// When `revision` is `Some`, the resulting JSON surfaces the historical
/// SHA in the `revision` field so callers can tell that `body` was read
/// from history rather than HEAD.
pub fn roadmap_to_json(
    doc: &Document<Roadmap>,
    phases: &[(String, Document<Phase>)],
    revision: Option<&str>,
) -> RoadmapJson {
    let rm = &doc.frontmatter;
    RoadmapJson {
        project: rm.project.clone(),
        slug: rm.roadmap.clone(),
        title: rm.title.clone(),
        phases: phases
            .iter()
            .map(|(stem, pd)| phase_summary_to_json(stem, pd))
            .collect(),
        dependencies: rm.dependencies.clone(),
        priority: rm.priority,
        tags: rm.tags.clone(),
        body: doc.body.clone(),
        revision: revision.map(String::from),
    }
}

/// Build a [`PhaseJson`] from a phase document, stem, and parent roadmap slug.
///
/// `prev` and `next` are optional stems of adjacent phases. When `revision`
/// is `Some`, the resulting JSON surfaces the historical SHA in the
/// `revision` field.
pub fn phase_to_json(
    stem: &str,
    doc: &Document<Phase>,
    roadmap: &str,
    prev: Option<&str>,
    next: Option<&str>,
    revision: Option<&str>,
) -> PhaseJson {
    let fm = &doc.frontmatter;
    PhaseJson {
        stem: stem.to_string(),
        phase: fm.phase,
        title: fm.title.clone(),
        status: fm.status,
        tags: fm.tags.clone(),
        completed: fm.completed,
        commit: fm.commit.clone(),
        difficulty: fm.difficulty,
        model: fm.model,
        blocked_reason: fm.blocked_reason.clone(),
        gate_override: fm.gate_override.clone(),
        revision: revision.map(String::from),
        roadmap: roadmap.to_string(),
        prev_phase: prev.map(String::from),
        next_phase: next.map(String::from),
        plans: Vec::new(),
        body: doc.body.clone(),
    }
}

/// Build a [`TaskJson`] from a task document and slug.
///
/// When `revision` is `Some`, the resulting JSON surfaces the historical
/// SHA in the `revision` field.
pub fn task_to_json(slug: &str, doc: &Document<Task>, revision: Option<&str>) -> TaskJson {
    let fm = &doc.frontmatter;
    TaskJson {
        slug: slug.to_string(),
        project: fm.project.clone(),
        title: fm.title.clone(),
        status: fm.status,
        priority: fm.priority,
        created: fm.created,
        tags: fm.tags.clone(),
        completed: fm.completed,
        commit: fm.commit.clone(),
        close_reason: fm.close_reason.clone(),
        gate_override: fm.gate_override.clone(),
        plans: Vec::new(),
        body: doc.body.clone(),
        revision: revision.map(String::from),
    }
}

/// Build a [`RoadmapSummaryJson`] from a roadmap document and its phases.
pub fn roadmap_summary_to_json(
    doc: &Document<Roadmap>,
    phases: &[(String, Document<Phase>)],
) -> RoadmapSummaryJson {
    let rm = &doc.frontmatter;
    let total = phases.len();
    let done = phases
        .iter()
        .filter(|(_, pd)| pd.frontmatter.status.is_terminal())
        .count();
    let progress = crate::display::roadmap_progress_label(done, total);
    RoadmapSummaryJson {
        slug: rm.roadmap.clone(),
        title: rm.title.clone(),
        total_phases: total,
        done_phases: done,
        progress,
        priority: rm.priority,
        tags: rm.tags.clone(),
    }
}

/// Build a [`PhaseSummaryJson`] from a phase document and its stem.
pub fn phase_summary_to_json(stem: &str, doc: &Document<Phase>) -> PhaseSummaryJson {
    let fm = &doc.frontmatter;
    PhaseSummaryJson {
        number: fm.phase,
        stem: stem.to_string(),
        title: fm.title.clone(),
        status: fm.status,
        tags: fm.tags.clone(),
        difficulty: fm.difficulty,
        model: fm.model,
    }
}

/// Build a [`TaskSummaryJson`] from a task document and slug.
pub fn task_summary_to_json(slug: &str, doc: &Document<Task>) -> TaskSummaryJson {
    let fm = &doc.frontmatter;
    TaskSummaryJson {
        slug: slug.to_string(),
        title: fm.title.clone(),
        status: fm.status,
        priority: fm.priority,
        created: fm.created,
        tags: fm.tags.clone(),
    }
}

/// Build a [`PlanJson`] from a plan document, its slug, and the reviews
/// targeting it.
///
/// `reviews` is what the caller already loaded (see
/// [`crate::ops::reviews::filter_reviews`]); a pure mapper, like every other
/// `*_to_json` here.
pub fn plan_to_json(
    slug: &str,
    doc: &Document<Plan>,
    reviews: &[(String, Document<Review>)],
) -> PlanJson {
    let fm = &doc.frontmatter;
    PlanJson {
        slug: slug.to_string(),
        project: fm.project.clone(),
        title: fm.title.clone(),
        status: fm.status,
        implements: format!("rdm:{}", fm.implements.label()),
        supersedes: fm.supersedes.as_ref().map(|r| format!("rdm:{}", r.label())),
        created: fm.created,
        updated: fm.updated,
        body: doc.body.clone(),
        reviews: reviews
            .iter()
            .map(|(id, rd)| PlanReviewRefJson {
                id: id.clone(),
                author: rd.frontmatter.author.clone(),
                state: rd.frontmatter.state,
                verdict: rd.frontmatter.verdict,
                submitted: rd.frontmatter.submitted,
            })
            .collect(),
        change_reviews: Vec::new(),
    }
}

/// Build a [`PlanSummaryJson`] from a plan document and its slug.
pub fn plan_summary_to_json(slug: &str, doc: &Document<Plan>) -> PlanSummaryJson {
    let fm = &doc.frontmatter;
    PlanSummaryJson {
        slug: slug.to_string(),
        title: fm.title.clone(),
        status: fm.status,
        implements: format!("rdm:{}", fm.implements.label()),
        supersedes: fm.supersedes.as_ref().map(|r| format!("rdm:{}", r.label())),
        created: fm.created,
        updated: fm.updated,
    }
}

/// Build a [`PlanRefJson`] back-reference from a plan document and its slug,
/// for embedding in the phase or task the plan implements.
pub fn plan_ref_to_json(slug: &str, doc: &Document<Plan>) -> PlanRefJson {
    PlanRefJson {
        slug: slug.to_string(),
        title: doc.frontmatter.title.clone(),
        status: doc.frontmatter.status,
    }
}

/// Build a [`ProjectJson`] from a project document.
pub fn project_to_json(doc: &Document<Project>) -> ProjectJson {
    let fm = &doc.frontmatter;
    ProjectJson {
        name: fm.name.clone(),
        title: fm.title.clone(),
        body: doc.body.clone(),
    }
}

/// Build a [`SearchResultJson`] from a [`SearchResult`].
pub fn search_result_to_json(result: &SearchResult) -> SearchResultJson {
    SearchResultJson {
        kind: result.kind,
        identifier: result.identifier.clone(),
        project: result.project.clone(),
        title: result.title.clone(),
        snippet: result.snippet.clone(),
        score: result.score,
        tags: result.tags.clone(),
    }
}

// ---------------------------------------------------------------------------
// Review types
// ---------------------------------------------------------------------------

/// Which body a resolved anchor range indexes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResolvedBodyJson {
    /// The target's body as it existed at the review's `created_commit` —
    /// re-read it with `--at <created_commit>` (or
    /// [`VersionedStore::fetch_body_at`](crate::store::VersionedStore::fetch_body_at)).
    Original,
    /// The target's current body.
    Current,
}

/// A comment anchor's resolution outcome, for JSON output.
///
/// Tagged on `state`:
///
/// - `resolved` — the anchor located its span; `quote` is the text at the
///   span and `body` says which version of the document
///   (`original`/`current`) the byte range indexes.
/// - `drifted` — the anchor resolved in the body the reviewer saw, but the
///   current body no longer matches. `quote` is the text the reviewer saw;
///   `body` is always `original` (the range indexes the `created_commit`
///   body, never the current one).
/// - `unresolved` — the span could not be located in any available body (or
///   the comment has no anchor).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum ResolutionJson {
    /// The anchor resolved and still matches.
    Resolved {
        /// The text at the resolved range.
        quote: String,
        /// Byte offset of the span start within the indexed body.
        range_start: usize,
        /// Byte offset of the span end within the indexed body.
        range_end: usize,
        /// Which body the range indexes.
        body: ResolvedBodyJson,
    },
    /// The anchor resolved in the reviewer's body but the current body has
    /// drifted from it.
    Drifted {
        /// The text the reviewer saw (from the `created_commit` body).
        quote: String,
        /// Byte offset of the span start within the *original* body.
        range_start: usize,
        /// Byte offset of the span end within the *original* body.
        range_end: usize,
        /// Which body the range indexes — always
        /// [`ResolvedBodyJson::Original`] for drifted anchors.
        body: ResolvedBodyJson,
    },
    /// The anchor (or a whole-document comment) has no locatable span.
    Unresolved,
}

/// A single review comment with its anchor and resolution state.
#[derive(Debug, Clone, Serialize)]
pub struct ReviewCommentJson {
    /// Ordinal identifier, unique within the review.
    pub id: u32,
    /// Document scope (roadmap reviews only), if the comment points at one
    /// of the roadmap's phases.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<CommentDoc>,
    /// Resolution status of the comment.
    pub status: ReviewCommentStatus,
    /// Commit SHA recorded when the comment was addressed, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applied_commit: Option<String>,
    /// The stored anchor (tagged on `anchor_type`), if the comment is
    /// anchored.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<Anchor>,
    /// The comment text (Markdown).
    pub body: String,
    /// Agent reply note, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply: Option<String>,
    /// Where the anchor currently resolves (see [`ResolutionJson`]).
    pub resolution: ResolutionJson,
    /// For a `change/<sha>` review's anchored comment: the head-pinned
    /// `rdm:src/<path>@<head>#L<start>[-L<end>]` permalink, exactly as
    /// [`crate::link::parse`] accepts it. Absent on every other comment, and
    /// on a comment whose anchor is not (or no longer) eligible to anchor at
    /// all (see [`Self::unresolved_reason`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_link: Option<String>,
    /// Why a `change/<sha>` review's anchor cannot resolve, when the reason
    /// is more specific than "drifted" or "not found" — currently, a stored
    /// [`Anchor::FileQuote`](crate::model::Anchor::FileQuote) whose path
    /// names a directory or a submodule rather than a file at the review's
    /// head. Absent whenever there is nothing more specific to say (Unlike
    /// [`ReviewJson::source_verification_skipped`], this is per-comment: it
    /// explains why *this* anchor itself is invalid, not why verification
    /// could not run at all).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unresolved_reason: Option<String>,
}

/// Full review detail: metadata, summary body, and every comment with its
/// anchor and resolution — everything an agent needs in one call.
#[derive(Debug, Clone, Serialize)]
pub struct ReviewJson {
    /// Review id (also the file stem under `reviews/`).
    pub id: String,
    /// Who authored the review.
    pub author: String,
    /// The plan item under review (tagged on `kind`).
    pub target: ReviewTarget,
    /// Lifecycle state.
    pub state: ReviewState,
    /// Verdict stamped on submit; absent on drafts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verdict: Option<Verdict>,
    /// When the review was started.
    pub created: DateTime<Utc>,
    /// When the review was submitted; absent on drafts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub submitted: Option<DateTime<Utc>>,
    /// Plan-repo HEAD when the review started, if recorded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_commit: Option<String>,
    /// The implementation plan a reviewed change implements, as the
    /// canonical `rdm:plan/<slug>` URI. Absent on every other review kind.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implements: Option<String>,
    /// Source-repository branch the reviewed change was on, if recorded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change_branch: Option<String>,
    /// Why source-repository anchor verification was skipped, when it was
    /// (no checkout reachable, no resolvable HEAD, a build without git).
    /// Absent when resolution really ran — the same degrade-with-a-reason
    /// contract as `link check`'s `path_verification_skipped`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_verification_skipped: Option<String>,
    /// The overall review summary (Markdown body).
    pub body: String,
    /// The inline comments with their resolution states.
    pub comments: Vec<ReviewCommentJson>,
}

impl ReviewJson {
    /// Attaches the "source verification skipped" note.
    ///
    /// A builder rather than a [`review_to_json`] parameter so every
    /// existing caller stays untouched and a review with nothing skipped
    /// keeps the exact bytes it had — the same pattern
    /// [`TaskJson::with_plans`] uses.
    #[must_use]
    pub fn with_source_note(mut self, note: Option<String>) -> Self {
        self.source_verification_skipped = note;
        self
    }
}

/// Maps one [`ResolvedComment`] into its JSON shape.
fn resolution_to_json(resolved: &ResolvedComment) -> ResolutionJson {
    match (&resolved.resolution, &resolved.quote) {
        (Resolution::Original { range, drifted }, Some(quote)) => {
            if *drifted {
                ResolutionJson::Drifted {
                    quote: quote.clone(),
                    range_start: range.start,
                    range_end: range.end,
                    body: ResolvedBodyJson::Original,
                }
            } else {
                ResolutionJson::Resolved {
                    quote: quote.clone(),
                    range_start: range.start,
                    range_end: range.end,
                    body: ResolvedBodyJson::Original,
                }
            }
        }
        (Resolution::Current { range }, Some(quote)) => ResolutionJson::Resolved {
            quote: quote.clone(),
            range_start: range.start,
            range_end: range.end,
            body: ResolvedBodyJson::Current,
        },
        _ => ResolutionJson::Unresolved,
    }
}

/// Build a [`ReviewJson`] from a review document and its comments'
/// pre-computed resolutions.
///
/// A pure mapper, like every other `*_to_json` here: the caller performs the
/// single resolution pass (one
/// [`resolve_comment`](crate::anchor::resolve_comment) per comment, in
/// comment order) and feeds the same `resolutions` slice to the JSON, human,
/// and markdown renderers. `resolutions` is parallel to
/// `doc.frontmatter.comments`; a comment without a corresponding entry
/// renders as unresolved.
///
/// `comment_notes` is likewise parallel to `doc.frontmatter.comments`: a
/// per-comment ineligibility explanation (from
/// [`crate::change::change_anchor_ineligibility`]), non-empty only for a
/// `change/<sha>` review. A shorter or empty slice — every non-change caller
/// passes `&[]` — degrades every comment to `None`. A comment with a note
/// emits no `source_link`: an anchor that cannot even name a valid object
/// has no line to permalink to.
pub fn review_to_json(
    id: &str,
    doc: &Document<Review>,
    resolutions: &[ResolvedComment],
    comment_notes: &[Option<String>],
) -> ReviewJson {
    let fm = &doc.frontmatter;
    let unresolved = ResolvedComment {
        resolution: Resolution::Unresolved,
        quote: None,
    };
    // Permalinks are head-pinned, so they only exist for a change target.
    let change_head = match &fm.target {
        ReviewTarget::Change { head, .. } => Some(head.as_str()),
        _ => None,
    };
    ReviewJson {
        id: id.to_string(),
        author: fm.author.clone(),
        target: fm.target.clone(),
        state: fm.state,
        verdict: fm.verdict,
        created: fm.created,
        submitted: fm.submitted,
        created_commit: fm.created_commit.clone(),
        implements: fm.implements.as_ref().map(|r| format!("rdm:{}", r.label())),
        change_branch: fm.change_branch.clone(),
        source_verification_skipped: None,
        body: doc.body.clone(),
        comments: fm
            .comments
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let note = comment_notes.get(i).cloned().flatten();
                ReviewCommentJson {
                    id: c.id,
                    doc: c.doc.clone(),
                    status: c.status,
                    applied_commit: c.applied_commit.clone(),
                    anchor: c.anchor.clone(),
                    body: c.body.clone(),
                    reply: c.reply.clone(),
                    resolution: resolution_to_json(resolutions.get(i).unwrap_or(&unresolved)),
                    source_link: if note.is_some() {
                        None
                    } else {
                        change_head.and_then(|head| {
                            c.anchor
                                .as_ref()
                                .and_then(|a| crate::change::permalink_for(head, a))
                                .map(|link| link.to_string())
                        })
                    },
                    unresolved_reason: note,
                }
            })
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// Link types
// ---------------------------------------------------------------------------

/// The documented JSON shape for a resolved `rdm:` link: `kind` is always
/// present, every other field is present only when it applies to that kind
/// (`path`/`exists` for `"item"`; `path`/`rev`/`line`/`end_line`/`web_url`
/// for `"code"`) — see [`resolved_to_json`].
#[derive(Debug, Clone, Serialize)]
pub struct ResolvedLinkJson {
    /// `"item"`, `"code"`, or `"broken"`.
    pub kind: &'static str,
    /// Item kind: the plan-repo-relative path to the target document.
    /// Code kind: the source path, relative to the source repository root.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Code kind only: the resolved revision.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rev: Option<String>,
    /// Code kind only: the start line, if the link named one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// Code kind only: the end line, if the link named a range.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u32>,
    /// Code kind only: the GitHub-style web URL, if the project has a
    /// `source` configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_url: Option<String>,
    /// Item kind only: whether the target currently exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exists: Option<bool>,
    /// Broken kind only: why resolution could not proceed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Builds the documented flat JSON shape for a [`Resolved`] link.
///
/// `item_path` is supplied by the caller for [`Resolved::Item`] — core's
/// [`Resolved`] type doesn't carry a path (see `resolve_item_link`'s doc
/// comment), so a caller that wants one (`rdm-cli`'s `link resolve`/`link
/// list`) computes it via [`crate::paths::roadmap_path`]/`phase_path`/
/// `task_path`, resolving a numeric phase stem first, and passes it through
/// here. Ignored for `Resolved::Code`/`Resolved::Broken`.
#[must_use]
pub fn resolved_to_json(resolved: &Resolved, item_path: Option<&str>) -> ResolvedLinkJson {
    match resolved {
        Resolved::Item { exists, .. } => ResolvedLinkJson {
            kind: "item",
            path: item_path.map(str::to_string),
            rev: None,
            line: None,
            end_line: None,
            web_url: None,
            exists: Some(*exists),
            reason: None,
        },
        Resolved::Code {
            path,
            rev,
            lines,
            web_url,
        } => ResolvedLinkJson {
            kind: "code",
            path: Some(path.clone()),
            rev: rev.clone(),
            line: lines.map(|(start, _)| start),
            end_line: lines.and_then(|(_, end)| end),
            web_url: web_url.clone(),
            exists: None,
            reason: None,
        },
        Resolved::Broken { reason } => ResolvedLinkJson {
            kind: "broken",
            path: None,
            rev: None,
            line: None,
            end_line: None,
            web_url: None,
            exists: None,
            reason: Some(reason.clone()),
        },
    }
}

/// One outgoing link from a document, resolved — the JSON shape `rdm-cli`'s
/// `link list` emits per entry.
#[derive(Debug, Clone, Serialize)]
pub struct OutgoingLinkJson {
    /// The raw `rdm:` URI text, as written in the document.
    pub uri: String,
    /// The resolved link.
    #[serde(flatten)]
    pub resolved: ResolvedLinkJson,
}

/// Builds an [`OutgoingLinkJson`] from a link's raw URI text and its already
/// [`resolved_to_json`]-mapped resolution.
#[must_use]
pub fn outgoing_link_to_json(uri: &str, resolved: ResolvedLinkJson) -> OutgoingLinkJson {
    OutgoingLinkJson {
        uri: uri.to_string(),
        resolved,
    }
}

/// One document referencing a backlink target, in JSON.
///
/// `range_start`/`range_end` are present for a body link and omitted for a
/// frontmatter reference, which carries `field` (and `via`, for a
/// transitive one) instead — see [`crate::link::BacklinkRef`].
#[derive(Debug, Clone, Serialize)]
pub struct BacklinkEntryJson {
    /// The referencing document.
    #[serde(flatten)]
    pub document: DocRef,
    /// Byte offset of the link's start within that document's body.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range_start: Option<usize>,
    /// Byte offset of the link's end within that document's body.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range_end: Option<usize>,
    /// The frontmatter field the reference lives in, for a structural
    /// backlink.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<&'static str>,
    /// The intermediate document a transitive reference was reached
    /// through, as a `rdm:` URI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
}

/// Builds a [`BacklinkEntryJson`] from a [`BacklinkEntry`].
#[must_use]
pub fn backlink_entry_to_json(entry: &BacklinkEntry) -> BacklinkEntryJson {
    let (range_start, range_end) = match entry.byte_range() {
        Some(r) => (Some(r.start), Some(r.end)),
        None => (None, None),
    };
    let (field, via) = match &entry.reference {
        crate::link::BacklinkRef::Body { .. } => (None, None),
        crate::link::BacklinkRef::Field { field, via } => (
            Some(*field),
            via.as_ref().map(|r| format!("rdm:{}", r.label())),
        ),
    };
    BacklinkEntryJson {
        document: entry.document.clone(),
        range_start,
        range_end,
        field,
        via,
    }
}

/// One dangling item link, in JSON.
#[derive(Debug, Clone, Serialize)]
pub struct DanglingLinkJson {
    /// The document the link was found in.
    #[serde(flatten)]
    pub document: DocRef,
    /// Byte offset of the link's start within that document's body.
    pub range_start: usize,
    /// Byte offset of the link's end within that document's body.
    pub range_end: usize,
    /// The raw `rdm:` URI text, as written in the document.
    pub uri: String,
    /// The missing target, in `<kind>/<id>` reference syntax.
    pub target: String,
}

/// One malformed `rdm:` link destination, in JSON.
#[derive(Debug, Clone, Serialize)]
pub struct LinkDiagnosticJson {
    /// The document the malformed link was found in.
    #[serde(flatten)]
    pub document: DocRef,
    /// Byte offset of the link's start within that document's body.
    pub range_start: usize,
    /// Byte offset of the link's end within that document's body.
    pub range_end: usize,
    /// The raw, unparsed `rdm:` destination text.
    pub uri: String,
    /// Why the destination failed to parse.
    pub error: String,
}

/// One code link found missing at its pinned revision, in JSON.
#[derive(Debug, Clone, Serialize)]
pub struct MissingAtRevJson {
    /// The document the link was found in.
    #[serde(flatten)]
    pub document: DocRef,
    /// Byte offset of the link's start within that document's body.
    pub range_start: usize,
    /// Byte offset of the link's end within that document's body.
    pub range_end: usize,
    /// Path to the file, relative to the source repository root.
    pub path: String,
    /// The revision the path was checked at.
    pub rev: String,
}

/// The full `link check` report, in JSON.
#[derive(Debug, Clone, Serialize)]
pub struct LinkCheckReportJson {
    /// The document reference the check was scoped to, or absent for a
    /// project-wide check.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on: Option<String>,
    /// Count of successfully-parsed links resolved.
    pub links_checked: usize,
    /// Count of code links found (a caller's path-verification worklist
    /// size).
    pub code_links_checked: usize,
    /// Item links whose target does not exist.
    pub dangling: Vec<DanglingLinkJson>,
    /// Malformed `rdm:` destinations found while parsing.
    pub diagnostics: Vec<LinkDiagnosticJson>,
    /// Code links found missing at their pinned revision, distinct from
    /// `dangling` (an item-link concern) and `diagnostics` (a parse-error
    /// concern).
    pub missing_at_rev: Vec<MissingAtRevJson>,
    /// Set when checkout-aware path verification did not run (not inside a
    /// checkout, or built without git support).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path_verification_skipped: Option<String>,
}

/// Builds a [`LinkCheckReportJson`] from a
/// [`LinkCheckReport`](crate::ops::links::LinkCheckReport).
///
/// `on` is the `--on` reference the check was scoped to (as passed on the
/// command line), or `None` for a project-wide check.
#[must_use]
pub fn link_check_report_to_json(
    report: &crate::ops::links::LinkCheckReport,
    on: Option<&str>,
) -> LinkCheckReportJson {
    LinkCheckReportJson {
        on: on.map(str::to_string),
        links_checked: report.links_checked,
        code_links_checked: report.code_links.len(),
        dangling: report
            .dangling
            .iter()
            .map(|d| DanglingLinkJson {
                document: d.document.clone(),
                range_start: d.byte_range.start,
                range_end: d.byte_range.end,
                uri: d.uri.clone(),
                target: d.target.label(),
            })
            .collect(),
        diagnostics: report
            .diagnostics
            .iter()
            .map(|d| LinkDiagnosticJson {
                document: d.document.clone(),
                range_start: d.diagnostic.range.start,
                range_end: d.diagnostic.range.end,
                uri: d.diagnostic.uri.clone(),
                error: d.diagnostic.error.to_string(),
            })
            .collect(),
        missing_at_rev: report
            .missing_at_rev
            .iter()
            .map(|m| MissingAtRevJson {
                document: m.document.clone(),
                range_start: m.byte_range.start,
                range_end: m.byte_range.end,
                path: m.path.clone(),
                rev: m.rev.clone(),
            })
            .collect(),
        path_verification_skipped: report.path_verification_skipped.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{PhaseStatus, Priority, TaskStatus};
    use chrono::NaiveDate;

    fn make_phase_doc(num: u32, title: &str, status: PhaseStatus) -> Document<Phase> {
        Document {
            frontmatter: Phase {
                phase: num,
                title: title.to_string(),
                status,
                tags: None,
                completed: if status == PhaseStatus::Done {
                    Some(NaiveDate::from_ymd_opt(2026, 3, 14).unwrap())
                } else {
                    None
                },
                commit: None,
                review_sha: None,
                review_branch: None,
                difficulty: None,
                model: None,
                blocked_reason: None,
                gate_override: None,
            },
            body: String::new(),
        }
    }

    fn make_roadmap_doc(project: &str, slug: &str, title: &str) -> Document<Roadmap> {
        Document {
            frontmatter: Roadmap {
                project: project.to_string(),
                roadmap: slug.to_string(),
                title: title.to_string(),
                phases: Vec::new(),
                dependencies: None,
                priority: None,
                tags: None,
            },
            body: String::new(),
        }
    }

    fn make_task_doc(slug: &str, project: &str) -> Document<Task> {
        Document {
            frontmatter: Task {
                project: project.to_string(),
                title: format!("Task {slug}"),
                status: TaskStatus::Open,
                priority: Priority::Medium,
                created: NaiveDate::from_ymd_opt(2026, 3, 15).unwrap(),
                tags: None,
                completed: None,
                commit: None,
                review_sha: None,
                review_branch: None,
                close_reason: None,
                gate_override: None,
            },
            body: String::new(),
        }
    }

    #[test]
    fn roadmap_to_json_includes_phases() {
        let doc = make_roadmap_doc("acme", "alpha", "Alpha");
        let phases = vec![
            (
                "phase-1-setup".to_string(),
                make_phase_doc(1, "Setup", PhaseStatus::Done),
            ),
            (
                "phase-2-impl".to_string(),
                make_phase_doc(2, "Impl", PhaseStatus::InProgress),
            ),
        ];
        let json = roadmap_to_json(&doc, &phases, None);
        assert_eq!(json.slug, "alpha");
        assert_eq!(json.phases.len(), 2);
        assert_eq!(json.phases[0].stem, "phase-1-setup");
        assert_eq!(json.phases[1].status, PhaseStatus::InProgress);
    }

    #[test]
    fn roadmap_summary_progress_labels() {
        let doc = make_roadmap_doc("acme", "a", "A");
        // No phases
        let s = roadmap_summary_to_json(&doc, &[]);
        assert_eq!(s.progress, "no phases");

        // All done
        let phases = vec![("p1".to_string(), make_phase_doc(1, "P1", PhaseStatus::Done))];
        let s = roadmap_summary_to_json(&doc, &phases);
        assert_eq!(s.progress, "complete");

        // Partial
        let phases = vec![
            ("p1".to_string(), make_phase_doc(1, "P1", PhaseStatus::Done)),
            (
                "p2".to_string(),
                make_phase_doc(2, "P2", PhaseStatus::InProgress),
            ),
        ];
        let s = roadmap_summary_to_json(&doc, &phases);
        assert_eq!(s.progress, "1/2 done");
    }

    #[test]
    fn roadmap_summary_to_json_counts_wont_fix_as_done() {
        let doc = make_roadmap_doc("acme", "a", "A");
        let phases = vec![
            ("p1".to_string(), make_phase_doc(1, "P1", PhaseStatus::Done)),
            (
                "p2".to_string(),
                make_phase_doc(2, "P2", PhaseStatus::WontFix),
            ),
        ];
        let s = roadmap_summary_to_json(&doc, &phases);
        assert_eq!(s.done_phases, 2);
        assert_eq!(s.progress, "complete");
    }

    #[test]
    fn task_to_json_fields() {
        let doc = make_task_doc("fix-bug", "acme");
        let json = task_to_json("fix-bug", &doc, None);
        assert_eq!(json.slug, "fix-bug");
        assert_eq!(json.project, "acme");
        assert_eq!(json.status, TaskStatus::Open);
    }

    #[test]
    fn task_to_json_carries_close_reason() {
        let mut doc = make_task_doc("dup", "acme");
        doc.frontmatter.status = TaskStatus::WontFix;
        doc.frontmatter.close_reason = Some("superseded by task/survivor".to_string());
        let json = task_to_json("dup", &doc, None);
        assert_eq!(
            json.close_reason.as_deref(),
            Some("superseded by task/survivor")
        );
        let serialized = serde_json::to_string(&json).unwrap();
        assert!(serialized.contains("\"close_reason\":\"superseded by task/survivor\""));
    }

    #[test]
    fn close_reason_skipped_when_none() {
        let doc = make_task_doc("t", "p");
        let json = task_to_json("t", &doc, None);
        assert!(json.close_reason.is_none());
        let serialized = serde_json::to_string(&json).unwrap();
        assert!(!serialized.contains("close_reason"));
    }

    #[test]
    fn phase_summary_fields() {
        let doc = make_phase_doc(3, "Review", PhaseStatus::NotStarted);
        let s = phase_summary_to_json("phase-3-review", &doc);
        assert_eq!(s.number, 3);
        assert_eq!(s.stem, "phase-3-review");
        assert_eq!(s.status, PhaseStatus::NotStarted);
    }

    #[test]
    fn optional_fields_skipped_when_none() {
        let doc = make_task_doc("t", "p");
        let json = task_to_json("t", &doc, None);
        let serialized = serde_json::to_string(&json).unwrap();
        assert!(!serialized.contains("tags"));

        let phase_doc = make_phase_doc(1, "X", PhaseStatus::NotStarted);
        let pj = phase_to_json("phase-1-x", &phase_doc, "rm", None, None, None);
        let serialized = serde_json::to_string(&pj).unwrap();
        assert!(!serialized.contains("completed"));
        assert!(!serialized.contains("tags"));

        let psj = phase_summary_to_json("phase-1-x", &phase_doc);
        let serialized = serde_json::to_string(&psj).unwrap();
        assert!(!serialized.contains("tags"));

        let rm_doc = make_roadmap_doc("acme", "alpha", "Alpha");
        let rj = roadmap_to_json(&rm_doc, &[], None);
        let serialized = serde_json::to_string(&rj).unwrap();
        assert!(!serialized.contains("tags"));

        let rsj = roadmap_summary_to_json(&rm_doc, &[]);
        let serialized = serde_json::to_string(&rsj).unwrap();
        assert!(!serialized.contains("tags"));
    }

    #[test]
    fn roadmap_and_phase_tags_round_trip_through_json() {
        let mut rm_doc = make_roadmap_doc("acme", "alpha", "Alpha");
        rm_doc.frontmatter.tags = Some(vec!["api".to_string(), "cli".to_string()]);
        let rj = roadmap_to_json(&rm_doc, &[], None);
        assert_eq!(rj.tags, Some(vec!["api".to_string(), "cli".to_string()]));
        let rsj = roadmap_summary_to_json(&rm_doc, &[]);
        assert_eq!(rsj.tags, Some(vec!["api".to_string(), "cli".to_string()]));

        let mut p_doc = make_phase_doc(1, "X", PhaseStatus::NotStarted);
        p_doc.frontmatter.tags = Some(vec!["infra".to_string()]);
        let pj = phase_to_json("phase-1-x", &p_doc, "rm", None, None, None);
        assert_eq!(pj.tags, Some(vec!["infra".to_string()]));
        let psj = phase_summary_to_json("phase-1-x", &p_doc);
        assert_eq!(psj.tags, Some(vec!["infra".to_string()]));
    }

    #[test]
    fn project_to_json_fields() {
        let doc = Document {
            frontmatter: Project {
                name: "acme".to_string(),
                title: "Acme Corp".to_string(),
                source: None,
            },
            body: "Project description.".to_string(),
        };
        let json = project_to_json(&doc);
        assert_eq!(json.name, "acme");
        assert_eq!(json.title, "Acme Corp");
        assert_eq!(json.body, "Project description.");
    }

    #[test]
    fn search_result_to_json_fields() {
        let result = SearchResult {
            kind: ItemKind::Task,
            identifier: "fix-bug".to_string(),
            project: "acme".to_string(),
            title: "Fix Bug".to_string(),
            snippet: "...fix the bug...".to_string(),
            score: 42,
            tags: Some(vec!["bug".to_string()]),
        };
        let json = search_result_to_json(&result);
        assert_eq!(json.kind, ItemKind::Task);
        assert_eq!(json.identifier, "fix-bug");
        assert_eq!(json.project, "acme");
        assert_eq!(json.title, "Fix Bug");
        assert_eq!(json.snippet, "...fix the bug...");
        assert_eq!(json.tags, Some(vec!["bug".to_string()]));
    }

    #[test]
    fn roadmap_json_phases_are_summaries_without_body() {
        let doc = make_roadmap_doc("acme", "alpha", "Alpha");
        let mut phase_doc = make_phase_doc(1, "Setup", PhaseStatus::InProgress);
        phase_doc.body = "Detailed phase body content.".to_string();
        let phases = vec![("phase-1-setup".to_string(), phase_doc)];
        let json = roadmap_to_json(&doc, &phases, None);
        let serialized = serde_json::to_string(&json).unwrap();
        // Phase summaries should not contain body content
        assert!(!serialized.contains("Detailed phase body content"));
        // But the roadmap's own body should be present
        assert_eq!(json.phases[0].title, "Setup");
        assert_eq!(json.phases[0].number, 1);
    }

    // -- review JSON --

    use crate::model::{ReviewComment, ReviewState};
    use chrono::TimeZone;

    fn make_review_doc() -> Document<Review> {
        Document {
            frontmatter: Review {
                id: "2026-07-01-1430-a1b2".to_string(),
                author: "ed".to_string(),
                target: ReviewTarget::Task {
                    slug: "fix-login".to_string(),
                },
                state: ReviewState::Submitted,
                verdict: Some(Verdict::RequestChanges),
                created: Utc.with_ymd_and_hms(2026, 7, 1, 14, 30, 0).unwrap(),
                submitted: Some(Utc.with_ymd_and_hms(2026, 7, 1, 14, 55, 0).unwrap()),
                created_commit: Some("abc123".to_string()),
                comments: vec![ReviewComment {
                    id: 1,
                    doc: None,
                    status: ReviewCommentStatus::Open,
                    applied_commit: None,
                    anchor: Some(Anchor::TextQuote {
                        quote: "the span".to_string(),
                        prefix: "before ".to_string(),
                        suffix: " after".to_string(),
                    }),
                    body: "Tighten this.".to_string(),
                    reply: None,
                }],
                implements: None,
                change_branch: None,
            },
            body: "Overall summary.".to_string(),
        }
    }

    #[test]
    fn review_json_includes_metadata_anchor_and_resolution() {
        let doc = make_review_doc();
        let resolutions = vec![ResolvedComment {
            resolution: Resolution::Original {
                range: 7..15,
                drifted: false,
            },
            quote: Some("the span".to_string()),
        }];
        let json = review_to_json("2026-07-01-1430-a1b2", &doc, &resolutions, &[]);
        let v: serde_json::Value = serde_json::to_value(&json).unwrap();
        assert_eq!(v["id"], "2026-07-01-1430-a1b2");
        assert_eq!(v["target"]["kind"], "task");
        assert_eq!(v["target"]["slug"], "fix-login");
        assert_eq!(v["state"], "submitted");
        assert_eq!(v["verdict"], "request-changes");
        assert_eq!(v["created_commit"], "abc123");
        assert_eq!(v["body"], "Overall summary.");
        let c = &v["comments"][0];
        assert_eq!(c["id"], 1);
        assert_eq!(c["status"], "open");
        assert_eq!(c["anchor"]["anchor_type"], "text-quote");
        assert_eq!(c["anchor"]["quote"], "the span");
        assert_eq!(c["anchor"]["prefix"], "before ");
        assert_eq!(c["resolution"]["state"], "resolved");
        assert_eq!(c["resolution"]["quote"], "the span");
        assert_eq!(c["resolution"]["range_start"], 7);
        assert_eq!(c["resolution"]["range_end"], 15);
        assert_eq!(c["resolution"]["body"], "original");
    }

    #[test]
    fn review_json_drifted_resolution_marks_original_body() {
        let doc = make_review_doc();
        let resolutions = vec![ResolvedComment {
            resolution: Resolution::Original {
                range: 7..15,
                drifted: true,
            },
            quote: Some("the span".to_string()),
        }];
        let json = review_to_json("id", &doc, &resolutions, &[]);
        let v: serde_json::Value = serde_json::to_value(&json).unwrap();
        let r = &v["comments"][0]["resolution"];
        assert_eq!(r["state"], "drifted");
        assert_eq!(r["quote"], "the span");
        // Drifted ranges always index the created_commit body — spelled out.
        assert_eq!(r["body"], "original");
    }

    #[test]
    fn review_json_current_resolution_marks_current_body() {
        let doc = make_review_doc();
        let resolutions = vec![ResolvedComment {
            resolution: Resolution::Current { range: 3..11 },
            quote: Some("the span".to_string()),
        }];
        let json = review_to_json("id", &doc, &resolutions, &[]);
        let v: serde_json::Value = serde_json::to_value(&json).unwrap();
        let r = &v["comments"][0]["resolution"];
        assert_eq!(r["state"], "resolved");
        assert_eq!(r["body"], "current");
    }

    #[test]
    fn review_json_missing_resolution_defaults_to_unresolved() {
        let doc = make_review_doc();
        let json = review_to_json("id", &doc, &[], &[]);
        let v: serde_json::Value = serde_json::to_value(&json).unwrap();
        let r = &v["comments"][0]["resolution"];
        assert_eq!(r["state"], "unresolved");
        assert!(r.get("quote").is_none());
    }

    #[test]
    fn review_json_draft_omits_absent_optionals() {
        let mut doc = make_review_doc();
        doc.frontmatter.state = ReviewState::Draft;
        doc.frontmatter.verdict = None;
        doc.frontmatter.submitted = None;
        doc.frontmatter.created_commit = None;
        doc.frontmatter.comments.clear();
        let json = review_to_json("id", &doc, &[], &[]);
        let v: serde_json::Value = serde_json::to_value(&json).unwrap();
        assert!(v.get("verdict").is_none());
        assert!(v.get("submitted").is_none());
        assert!(v.get("created_commit").is_none());
        assert_eq!(v["state"], "draft");
    }

    /// A comment whose `comment_notes` entry is `Some` (a stored anchor
    /// naming a directory or submodule, per
    /// [`crate::change::change_anchor_ineligibility`]) must surface that
    /// reason and emit no `source_link` — even though a permalink could
    /// otherwise be built from the anchor's path and recorded line range.
    #[test]
    fn review_json_suppresses_the_permalink_and_names_the_reason_when_a_note_is_present() {
        let mut doc = make_review_doc();
        doc.frontmatter.target = ReviewTarget::Change {
            head: "a".repeat(40),
            base: Some("b".repeat(40)),
        };
        doc.frontmatter.comments[0].anchor = Some(Anchor::FileQuote {
            path: "sub".to_string(),
            quote: "a.txt".to_string(),
            occurrence: 1,
            start_line: 1,
            end_line: 1,
        });
        let resolutions = vec![ResolvedComment {
            resolution: Resolution::Unresolved,
            quote: None,
        }];
        let notes = vec![Some("'sub' is a directory, not a file".to_string())];
        let json = review_to_json("id", &doc, &resolutions, &notes);
        let v: serde_json::Value = serde_json::to_value(&json).unwrap();
        let c = &v["comments"][0];
        assert_eq!(c["unresolved_reason"], "'sub' is a directory, not a file");
        assert!(c.get("source_link").is_none(), "{c}");
    }

    /// The pre-existing behavior this suite must not regress: with no note
    /// (the common case — including a source repo that could not be reached
    /// at all, which is a *verification* skip, not an anchor defect), the
    /// permalink still renders from the anchor alone, independent of
    /// resolution state.
    #[test]
    fn review_json_still_emits_the_permalink_with_no_note_even_when_unresolved() {
        let mut doc = make_review_doc();
        doc.frontmatter.target = ReviewTarget::Change {
            head: "a".repeat(40),
            base: Some("b".repeat(40)),
        };
        doc.frontmatter.comments[0].anchor = Some(Anchor::FileQuote {
            path: "src/lib.rs".to_string(),
            quote: "fn touched".to_string(),
            occurrence: 1,
            start_line: 2,
            end_line: 2,
        });
        let resolutions = vec![ResolvedComment {
            resolution: Resolution::Unresolved,
            quote: None,
        }];
        let json = review_to_json("id", &doc, &resolutions, &[]);
        let v: serde_json::Value = serde_json::to_value(&json).unwrap();
        let c = &v["comments"][0];
        assert!(c.get("unresolved_reason").is_none(), "{c}");
        assert_eq!(
            c["source_link"],
            format!("rdm:src/src/lib.rs@{}#L2", "a".repeat(40))
        );
    }

    #[test]
    fn info_json_omits_project_when_none() {
        let info = InfoJson {
            root: "/tmp/plan-repo".to_string(),
            project: None,
            default_branch: "main".to_string(),
            default_format: "human".to_string(),
        };
        let v = serde_json::to_value(&info).unwrap();
        assert!(
            v.get("project").is_none(),
            "the 'project' key must be absent entirely when unresolved, not present as null: {v}"
        );
        assert_eq!(v["root"], "/tmp/plan-repo");
        assert_eq!(v["default_branch"], "main");
        assert_eq!(v["default_format"], "human");
    }

    #[test]
    fn info_json_includes_project_when_some() {
        let info = InfoJson {
            root: "/tmp/plan-repo".to_string(),
            project: Some("rdm".to_string()),
            default_branch: "main".to_string(),
            default_format: "json".to_string(),
        };
        let v = serde_json::to_value(&info).unwrap();
        assert_eq!(v["project"], "rdm");
    }

    #[test]
    fn link_check_report_to_json_maps_diagnostics_distinctly_from_dangling() {
        use crate::link::LinkParseError;
        use crate::ops::links::{CheckDiagnostic, LinkCheckReport};

        let report = LinkCheckReport {
            links_checked: 0,
            dangling: Vec::new(),
            code_links: Vec::new(),
            diagnostics: vec![CheckDiagnostic {
                document: DocRef::Task {
                    slug: "malformed".to_string(),
                },
                diagnostic: crate::link::LinkDiagnostic {
                    range: 5..20,
                    uri: "rdm:foo/bar".to_string(),
                    error: LinkParseError::UnknownKind {
                        uri: "rdm:foo/bar".to_string(),
                        kind: "foo".to_string(),
                    },
                },
            }],
            missing_at_rev: Vec::new(),
            path_verification_skipped: None,
        };

        let json = link_check_report_to_json(&report, None);
        assert_eq!(json.diagnostics.len(), 1);
        assert!(json.dangling.is_empty());
        assert!(json.missing_at_rev.is_empty());
        let diag = &json.diagnostics[0];
        assert_eq!(diag.uri, "rdm:foo/bar");
        assert_eq!(diag.range_start, 5);
        assert_eq!(diag.range_end, 20);
        assert!(
            !diag.error.is_empty(),
            "diagnostic error message must not be empty"
        );

        let v = serde_json::to_value(&json).unwrap();
        assert_eq!(v["diagnostics"][0]["uri"], "rdm:foo/bar");
        assert_eq!(v["dangling"], serde_json::json!([]));
        assert_eq!(v["missing_at_rev"], serde_json::json!([]));
    }
}
