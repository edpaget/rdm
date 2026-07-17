/// Serializable JSON output types for CLI and API consumers.
///
/// These structs combine frontmatter fields with contextual identifiers
/// (slug, stem, project, roadmap) and optional body content, producing
/// a stable JSON contract for scripts and agents.
use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;

use crate::anchor::{Resolution, ResolvedComment};
use crate::document::Document;
use crate::model::{
    Anchor, CommentDoc, Difficulty, ModelTier, Phase, PhaseStatus, Priority, Project, Review,
    ReviewCommentStatus, ReviewState, ReviewTarget, Roadmap, Task, TaskStatus, Verdict,
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
    /// Markdown body content.
    pub body: String,
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
    /// Markdown body content.
    pub body: String,
    /// Git revision the body was read from (only set when this view was
    /// requested at a specific historical SHA).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
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
        revision: revision.map(String::from),
        roadmap: roadmap.to_string(),
        prev_phase: prev.map(String::from),
        next_phase: next.map(String::from),
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
    /// The overall review summary (Markdown body).
    pub body: String,
    /// The inline comments with their resolution states.
    pub comments: Vec<ReviewCommentJson>,
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
pub fn review_to_json(
    id: &str,
    doc: &Document<Review>,
    resolutions: &[ResolvedComment],
) -> ReviewJson {
    let fm = &doc.frontmatter;
    let unresolved = ResolvedComment {
        resolution: Resolution::Unresolved,
        quote: None,
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
        body: doc.body.clone(),
        comments: fm
            .comments
            .iter()
            .enumerate()
            .map(|(i, c)| ReviewCommentJson {
                id: c.id,
                doc: c.doc.clone(),
                status: c.status,
                applied_commit: c.applied_commit.clone(),
                anchor: c.anchor.clone(),
                body: c.body.clone(),
                reply: c.reply.clone(),
                resolution: resolution_to_json(resolutions.get(i).unwrap_or(&unresolved)),
            })
            .collect(),
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
        rm_doc.frontmatter.tags = Some(vec!["api".to_string(), "mcp".to_string()]);
        let rj = roadmap_to_json(&rm_doc, &[], None);
        assert_eq!(rj.tags, Some(vec!["api".to_string(), "mcp".to_string()]));
        let rsj = roadmap_summary_to_json(&rm_doc, &[]);
        assert_eq!(rsj.tags, Some(vec!["api".to_string(), "mcp".to_string()]));

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
        let json = review_to_json("2026-07-01-1430-a1b2", &doc, &resolutions);
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
        let json = review_to_json("id", &doc, &resolutions);
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
        let json = review_to_json("id", &doc, &resolutions);
        let v: serde_json::Value = serde_json::to_value(&json).unwrap();
        let r = &v["comments"][0]["resolution"];
        assert_eq!(r["state"], "resolved");
        assert_eq!(r["body"], "current");
    }

    #[test]
    fn review_json_missing_resolution_defaults_to_unresolved() {
        let doc = make_review_doc();
        let json = review_to_json("id", &doc, &[]);
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
        let json = review_to_json("id", &doc, &[]);
        let v: serde_json::Value = serde_json::to_value(&json).unwrap();
        assert!(v.get("verdict").is_none());
        assert!(v.get("submitted").is_none());
        assert!(v.get("created_commit").is_none());
        assert_eq!(v["state"], "draft");
    }
}
