/// Serializable JSON output types for CLI and API consumers.
///
/// These structs combine frontmatter fields with contextual identifiers
/// (slug, stem, project, roadmap) and optional body content, producing
/// a stable JSON contract for scripts and agents.
use chrono::NaiveDate;
use serde::Serialize;

use crate::document::Document;
use crate::model::{Phase, PhaseStatus, Priority, Project, Roadmap, Task, TaskStatus};
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
    /// Markdown body content.
    pub body: String,
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
    /// Completion date, if done.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed: Option<NaiveDate>,
    /// Git commit SHA associated with phase completion, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
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
    /// Markdown body content.
    pub body: String,
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
}

// ---------------------------------------------------------------------------
// Conversion helpers
// ---------------------------------------------------------------------------

/// Build a [`RoadmapJson`] from a roadmap document and its loaded phases.
pub fn roadmap_to_json(
    doc: &Document<Roadmap>,
    phases: &[(String, Document<Phase>)],
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
        body: doc.body.clone(),
    }
}

/// Build a [`PhaseJson`] from a phase document, stem, and parent roadmap slug.
///
/// `prev` and `next` are optional stems of adjacent phases.
pub fn phase_to_json(
    stem: &str,
    doc: &Document<Phase>,
    roadmap: &str,
    prev: Option<&str>,
    next: Option<&str>,
) -> PhaseJson {
    let fm = &doc.frontmatter;
    PhaseJson {
        stem: stem.to_string(),
        phase: fm.phase,
        title: fm.title.clone(),
        status: fm.status,
        completed: fm.completed,
        commit: fm.commit.clone(),
        roadmap: roadmap.to_string(),
        prev_phase: prev.map(String::from),
        next_phase: next.map(String::from),
        body: doc.body.clone(),
    }
}

/// Build a [`TaskJson`] from a task document and slug.
pub fn task_to_json(slug: &str, doc: &Document<Task>) -> TaskJson {
    let fm = &doc.frontmatter;
    TaskJson {
        slug: slug.to_string(),
        project: fm.project.clone(),
        title: fm.title.clone(),
        status: fm.status,
        priority: fm.priority,
        created: fm.created,
        tags: fm.tags.clone(),
        body: doc.body.clone(),
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
        .filter(|(_, pd)| pd.frontmatter.status == PhaseStatus::Done)
        .count();
    let progress = if total == 0 {
        "no phases".to_string()
    } else if done == total {
        "complete".to_string()
    } else {
        format!("{done}/{total} done")
    };
    RoadmapSummaryJson {
        slug: rm.roadmap.clone(),
        title: rm.title.clone(),
        total_phases: total,
        done_phases: done,
        progress,
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
                completed: if status == PhaseStatus::Done {
                    Some(NaiveDate::from_ymd_opt(2026, 3, 14).unwrap())
                } else {
                    None
                },
                commit: None,
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
        let json = roadmap_to_json(&doc, &phases);
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
    fn task_to_json_fields() {
        let doc = make_task_doc("fix-bug", "acme");
        let json = task_to_json("fix-bug", &doc);
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
        let json = task_to_json("t", &doc);
        let serialized = serde_json::to_string(&json).unwrap();
        assert!(!serialized.contains("tags"));

        let phase_doc = make_phase_doc(1, "X", PhaseStatus::NotStarted);
        let pj = phase_to_json("phase-1-x", &phase_doc, "rm", None, None);
        let serialized = serde_json::to_string(&pj).unwrap();
        assert!(!serialized.contains("completed"));
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
        };
        let json = search_result_to_json(&result);
        assert_eq!(json.kind, ItemKind::Task);
        assert_eq!(json.identifier, "fix-bug");
        assert_eq!(json.project, "acme");
        assert_eq!(json.title, "Fix Bug");
        assert_eq!(json.snippet, "...fix the bug...");
    }

    #[test]
    fn roadmap_json_phases_are_summaries_without_body() {
        let doc = make_roadmap_doc("acme", "alpha", "Alpha");
        let mut phase_doc = make_phase_doc(1, "Setup", PhaseStatus::InProgress);
        phase_doc.body = "Detailed phase body content.".to_string();
        let phases = vec![("phase-1-setup".to_string(), phase_doc)];
        let json = roadmap_to_json(&doc, &phases);
        let serialized = serde_json::to_string(&json).unwrap();
        // Phase summaries should not contain body content
        assert!(!serialized.contains("Detailed phase body content"));
        // But the roadmap's own body should be present
        assert_eq!(json.phases[0].title, "Setup");
        assert_eq!(json.phases[0].number, 1);
    }
}
