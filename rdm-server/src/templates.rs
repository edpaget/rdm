//! Askama template structs for HTML rendering.

use askama::Template;
use rdm_core::model::PhaseStatus;

/// Helper to map phase status to CSS badge class.
pub fn phase_status_class(status: &rdm_core::model::PhaseStatus) -> &'static str {
    match status {
        rdm_core::model::PhaseStatus::NotStarted => "not-started",
        rdm_core::model::PhaseStatus::InProgress => "in-progress",
        rdm_core::model::PhaseStatus::Done => "done",
        rdm_core::model::PhaseStatus::Blocked => "blocked",
    }
}

/// Helper to map task status to CSS badge class.
pub fn task_status_class(status: &rdm_core::model::TaskStatus) -> &'static str {
    match status {
        rdm_core::model::TaskStatus::Open => "open",
        rdm_core::model::TaskStatus::InProgress => "in-progress",
        rdm_core::model::TaskStatus::Done => "done",
        rdm_core::model::TaskStatus::WontFix => "wont-fix",
    }
}

/// Helper to map priority to CSS badge class.
pub fn priority_class(priority: &rdm_core::model::Priority) -> &'static str {
    match priority {
        rdm_core::model::Priority::Low => "low",
        rdm_core::model::Priority::Medium => "medium",
        rdm_core::model::Priority::High => "high",
        rdm_core::model::Priority::Critical => "critical",
    }
}

/// A project entry for the index page.
pub struct ProjectView {
    /// Project slug.
    pub name: String,
    /// Human-readable title.
    pub title: String,
}

/// Root index page listing all projects.
#[derive(Template)]
#[template(path = "index.html")]
pub struct IndexPage {
    /// All projects.
    pub projects: Vec<ProjectView>,
}

/// Compute an overall roadmap status from its phase statuses.
///
/// Returns `(display_text, css_class)`:
/// - All phases done → `("done", "done")`
/// - Any phase in-progress, or a mix of done and not-started → `("in-progress", "in-progress")`
/// - Otherwise (all not-started, all blocked, or no phases) → `("not-started", "not-started")`
pub fn computed_roadmap_status(phases: &[PhaseStatus]) -> (&'static str, &'static str) {
    if phases.is_empty() {
        return ("not-started", "not-started");
    }
    if phases.iter().all(|s| *s == PhaseStatus::Done) {
        return ("done", "done");
    }
    let has_done = phases.contains(&PhaseStatus::Done);
    let has_in_progress = phases.contains(&PhaseStatus::InProgress);
    if has_in_progress || has_done {
        return ("in-progress", "in-progress");
    }
    ("not-started", "not-started")
}

/// A roadmap summary for the roadmaps list page.
pub struct RoadmapSummaryView {
    /// Roadmap slug.
    pub slug: String,
    /// Human-readable title.
    pub title: String,
    /// Total number of phases.
    pub total_phases: usize,
    /// Number of completed phases.
    pub done_phases: usize,
    /// Computed overall status display text.
    pub status: String,
    /// CSS class for the status badge.
    pub status_class: String,
    /// Last modification date, if available.
    pub last_changed: Option<String>,
}

/// Roadmaps list page for a project.
#[derive(Template)]
#[template(path = "roadmaps.html")]
pub struct RoadmapsPage {
    /// Project name.
    pub project: String,
    /// All roadmaps with progress.
    pub roadmaps: Vec<RoadmapSummaryView>,
}

/// A phase row for the roadmap detail page.
pub struct PhaseRow {
    /// Phase number.
    pub phase: u32,
    /// Phase stem (file identifier).
    pub stem: String,
    /// Human-readable title.
    pub title: String,
    /// Display status.
    pub status: String,
    /// CSS class for the status badge.
    pub status_class: String,
}

/// Roadmap detail page with phase table.
#[derive(Template)]
#[template(path = "roadmap_detail.html")]
pub struct RoadmapDetailPage {
    /// Project name.
    pub project: String,
    /// Roadmap slug.
    pub slug: String,
    /// Human-readable title.
    pub title: String,
    /// Computed overall status display text.
    pub status: String,
    /// CSS class for the status badge.
    pub status_class: String,
    /// Last modification date, if available.
    pub last_changed: Option<String>,
    /// Optional dependencies.
    pub dependencies: Option<Vec<String>>,
    /// Phases in this roadmap.
    pub phases: Vec<PhaseRow>,
}

/// Phase detail page with rendered markdown body.
#[derive(Template)]
#[template(path = "phase_detail.html")]
pub struct PhaseDetailPage {
    /// Project name.
    pub project: String,
    /// Roadmap slug.
    pub roadmap: String,
    /// Phase stem.
    pub stem: String,
    /// Phase number.
    pub phase_number: u32,
    /// Human-readable title.
    pub title: String,
    /// Display status.
    pub status: String,
    /// CSS class for the status badge.
    pub status_class: String,
    /// Completion date, if any.
    pub completed: Option<String>,
    /// Rendered HTML body.
    pub body_html: String,
    /// URL for the previous phase, if any.
    pub prev_href: Option<String>,
    /// URL for the next phase, if any.
    pub next_href: Option<String>,
}

/// A task row for the task list page.
pub struct TaskRow {
    /// Task slug.
    pub slug: String,
    /// Human-readable title.
    pub title: String,
    /// Display status.
    pub status: String,
    /// CSS class for the status badge.
    pub status_class: String,
    /// Display priority.
    pub priority: String,
    /// CSS class for the priority badge.
    pub priority_class: String,
}

/// Task list page for a project.
#[derive(Template)]
#[template(path = "task_list.html")]
pub struct TaskListPage {
    /// Project name.
    pub project: String,
    /// Filtered tasks.
    pub tasks: Vec<TaskRow>,
}

/// Task detail page with rendered markdown body.
#[derive(Template)]
#[template(path = "task_detail.html")]
pub struct TaskDetailPage {
    /// Project name.
    pub project: String,
    /// Task slug.
    pub slug: String,
    /// Human-readable title.
    pub title: String,
    /// Display status.
    pub status: String,
    /// CSS class for the status badge.
    pub status_class: String,
    /// Display priority.
    pub priority: String,
    /// CSS class for the priority badge.
    pub priority_class: String,
    /// Creation date.
    pub created: String,
    /// Optional tags.
    pub tags: Option<Vec<String>>,
    /// Rendered HTML body.
    pub body_html: String,
}

/// A single search result row for the search results page.
pub struct SearchResultRow {
    /// Item kind ("roadmap", "phase", or "task").
    pub kind: String,
    /// Human-readable title.
    pub title: String,
    /// Item identifier.
    pub identifier: String,
    /// Short text snippet.
    pub snippet: String,
    /// Link to the item detail page.
    pub href: String,
}

/// Search results page.
#[derive(Template)]
#[template(path = "search_results.html")]
pub struct SearchResultsPage {
    /// Project name.
    pub project: String,
    /// The search query.
    pub query: String,
    /// Search results.
    pub results: Vec<SearchResultRow>,
}

/// Error page with status code and message.
#[derive(Template)]
#[template(path = "error.html")]
pub struct ErrorPage {
    /// HTTP status code.
    pub status: u16,
    /// Error title.
    pub title: String,
    /// Optional detail message.
    pub detail: Option<String>,
}
