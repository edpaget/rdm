//! View structs that decouple rendering from the core data model.
//!
//! These small, owned structs are built from `rdm-core` documents in
//! [`crate::app`] and consumed by [`crate::ui`]. Keeping them TUI-local makes
//! the render functions pure (no `Store` access) and trivial to unit-test with
//! a `TestBackend`.

use chrono::NaiveDate;
use rdm_core::document::Document;
use rdm_core::model::{Phase, PhaseStatus, Priority, Task, TaskStatus};
use rdm_core::ops::roadmap::RoadmapStatus;

/// A single row in the roadmap-list screen.
pub struct RoadmapRow {
    /// Roadmap slug (its directory name).
    pub slug: String,
    /// Human-readable title.
    pub title: String,
    /// Aggregate status computed from the roadmap's phases.
    pub status: RoadmapStatus,
    /// Optional priority level.
    pub priority: Option<Priority>,
    /// Number of phases in a terminal state.
    pub done: usize,
    /// Total number of phases.
    pub total: usize,
}

/// A single phase row in the roadmap-detail screen.
pub struct PhaseRow {
    /// Phase number (1-based ordering).
    pub number: u32,
    /// Human-readable title.
    pub title: String,
    /// Current phase status.
    pub status: PhaseStatus,
}

/// The roadmap-detail screen: body text plus an ordered phase list.
pub struct RoadmapDetail {
    /// Roadmap slug.
    pub slug: String,
    /// Human-readable title.
    pub title: String,
    /// Markdown body, rendered as plain wrapped text for now.
    pub body: String,
    /// Phases in number order.
    pub phases: Vec<PhaseRow>,
}

/// The phase-detail screen: a phase's metadata plus its full markdown body.
///
/// Built up-front for every phase in a roadmap so prev/next navigation is pure
/// indexing — no re-listing of phases on each move.
pub struct PhaseDetailView {
    /// Roadmap slug this phase belongs to.
    pub roadmap: String,
    /// Phase number (1-based ordering).
    pub number: u32,
    /// Human-readable title.
    pub title: String,
    /// Current phase status.
    pub status: PhaseStatus,
    /// Completion date, if the phase is done.
    pub completed: Option<NaiveDate>,
    /// Git commit SHA recorded at completion, if any.
    pub commit: Option<String>,
    /// Tags for categorization.
    pub tags: Vec<String>,
    /// Markdown body, rendered through [`crate::markdown::render_markdown`].
    pub body: String,
}

/// Builds one [`PhaseDetailView`] per phase from [`list_phases`] output.
///
/// The input is already number-sorted, and that order is preserved so the
/// detail screen's prev/next matches the roadmap-detail phase list.
///
/// [`list_phases`]: rdm_core::ops::phase::list_phases
pub fn build_phase_details(
    roadmap: &str,
    phases: Vec<(String, Document<Phase>)>,
) -> Vec<PhaseDetailView> {
    phases
        .into_iter()
        .map(|(_, doc)| {
            let Document { frontmatter, body } = doc;
            PhaseDetailView {
                roadmap: roadmap.to_string(),
                number: frontmatter.phase,
                title: frontmatter.title,
                status: frontmatter.status,
                completed: frontmatter.completed,
                commit: frontmatter.commit,
                tags: frontmatter.tags.unwrap_or_default(),
                body,
            }
        })
        .collect()
}

/// A single row in the task-list screen.
pub struct TaskRow {
    /// Task slug (its file stem).
    pub slug: String,
    /// Human-readable title.
    pub title: String,
    /// Current task status.
    pub status: TaskStatus,
    /// Priority level.
    pub priority: Priority,
    /// Tags for categorization.
    pub tags: Vec<String>,
}

/// The task-detail screen: a task's metadata plus its full markdown body.
pub struct TaskDetailView {
    /// Task slug (its file stem).
    pub slug: String,
    /// Human-readable title.
    pub title: String,
    /// Current task status.
    pub status: TaskStatus,
    /// Priority level.
    pub priority: Priority,
    /// Date the task was created.
    pub created: NaiveDate,
    /// Completion date, if the task is done.
    pub completed: Option<NaiveDate>,
    /// Git commit SHA recorded at completion, if any.
    pub commit: Option<String>,
    /// Tags for categorization.
    pub tags: Vec<String>,
    /// Markdown body, rendered through [`crate::markdown::render_markdown`].
    pub body: String,
}

/// Builds one [`TaskRow`] per task from [`list_tasks`] output, preserving order.
///
/// [`list_tasks`]: rdm_core::ops::task::list_tasks
pub fn build_task_rows(tasks: &[(String, Document<Task>)]) -> Vec<TaskRow> {
    tasks
        .iter()
        .map(|(slug, doc)| TaskRow {
            slug: slug.clone(),
            title: doc.frontmatter.title.clone(),
            status: doc.frontmatter.status,
            priority: doc.frontmatter.priority,
            tags: doc.frontmatter.tags.clone().unwrap_or_default(),
        })
        .collect()
}

/// Builds a [`TaskDetailView`] for drill-in from a single `(slug, task)` pair.
pub fn task_detail_view(task: &(String, Document<Task>)) -> TaskDetailView {
    let (slug, doc) = task;
    TaskDetailView {
        slug: slug.clone(),
        title: doc.frontmatter.title.clone(),
        status: doc.frontmatter.status,
        priority: doc.frontmatter.priority,
        created: doc.frontmatter.created,
        completed: doc.frontmatter.completed,
        commit: doc.frontmatter.commit.clone(),
        tags: doc.frontmatter.tags.clone().unwrap_or_default(),
        body: doc.body.clone(),
    }
}

/// Maps a [`TaskStatus`] to a `"<symbol> <label>"` string.
///
/// Mirrors [`status_label`]: the text label is the colorless-terminal signal,
/// and the leading ASCII glyph just aids quick scanning.
pub fn task_status_label(status: TaskStatus) -> String {
    let (symbol, label) = match status {
        TaskStatus::Open => ("[ ]", "open"),
        TaskStatus::InProgress => ("[~]", "in-progress"),
        TaskStatus::NeedsReview => ("[?]", "needs-review"),
        TaskStatus::Reviewed => ("[+]", "reviewed"),
        TaskStatus::Done => ("[x]", "done"),
        TaskStatus::WontFix => ("[-]", "wont-fix"),
    };
    format!("{symbol} {label}")
}

/// Maps a [`PhaseStatus`] to a `"<symbol> <label>"` string.
///
/// The label is the signal that survives a colorless terminal; the leading
/// ASCII glyph just aids quick scanning. Glyphs are intentionally ASCII-safe.
pub fn status_label(status: PhaseStatus) -> String {
    let (symbol, label) = match status {
        PhaseStatus::NotStarted => ("[ ]", "not-started"),
        PhaseStatus::InProgress => ("[~]", "in-progress"),
        PhaseStatus::NeedsReview => ("[?]", "needs-review"),
        PhaseStatus::Reviewed => ("[+]", "reviewed"),
        PhaseStatus::Done => ("[x]", "done"),
        PhaseStatus::Blocked => ("[!]", "blocked"),
        PhaseStatus::WontFix => ("[-]", "wont-fix"),
    };
    format!("{symbol} {label}")
}

/// Maps a [`RoadmapStatus`] to a `"<symbol> <label>"` string.
///
/// Mirrors [`status_label`] using the matching phase glyphs for the three
/// aggregate states.
pub fn roadmap_status_label(status: RoadmapStatus) -> String {
    let symbol = match status {
        RoadmapStatus::NotStarted => "[ ]",
        RoadmapStatus::InProgress => "[~]",
        RoadmapStatus::Done => "[x]",
    };
    format!("{symbol} {}", status.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_label_covers_all_phase_statuses() {
        assert_eq!(status_label(PhaseStatus::NotStarted), "[ ] not-started");
        assert_eq!(status_label(PhaseStatus::InProgress), "[~] in-progress");
        assert_eq!(status_label(PhaseStatus::NeedsReview), "[?] needs-review");
        assert_eq!(status_label(PhaseStatus::Reviewed), "[+] reviewed");
        assert_eq!(status_label(PhaseStatus::Done), "[x] done");
        assert_eq!(status_label(PhaseStatus::Blocked), "[!] blocked");
        assert_eq!(status_label(PhaseStatus::WontFix), "[-] wont-fix");
    }

    #[test]
    fn status_labels_are_ascii() {
        for status in [
            PhaseStatus::NotStarted,
            PhaseStatus::InProgress,
            PhaseStatus::NeedsReview,
            PhaseStatus::Reviewed,
            PhaseStatus::Done,
            PhaseStatus::Blocked,
            PhaseStatus::WontFix,
        ] {
            assert!(status_label(status).is_ascii());
        }
    }

    #[test]
    fn task_status_label_covers_all_task_statuses() {
        assert_eq!(task_status_label(TaskStatus::Open), "[ ] open");
        assert_eq!(task_status_label(TaskStatus::InProgress), "[~] in-progress");
        assert_eq!(
            task_status_label(TaskStatus::NeedsReview),
            "[?] needs-review"
        );
        assert_eq!(task_status_label(TaskStatus::Reviewed), "[+] reviewed");
        assert_eq!(task_status_label(TaskStatus::Done), "[x] done");
        assert_eq!(task_status_label(TaskStatus::WontFix), "[-] wont-fix");
    }

    #[test]
    fn task_status_labels_are_ascii() {
        for status in [
            TaskStatus::Open,
            TaskStatus::InProgress,
            TaskStatus::NeedsReview,
            TaskStatus::Reviewed,
            TaskStatus::Done,
            TaskStatus::WontFix,
        ] {
            assert!(task_status_label(status).is_ascii());
        }
    }

    #[test]
    fn roadmap_status_label_covers_all_states() {
        assert_eq!(
            roadmap_status_label(RoadmapStatus::NotStarted),
            "[ ] not-started"
        );
        assert_eq!(
            roadmap_status_label(RoadmapStatus::InProgress),
            "[~] in-progress"
        );
        assert_eq!(roadmap_status_label(RoadmapStatus::Done), "[x] done");
    }
}
