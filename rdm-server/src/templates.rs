//! Askama template structs for HTML rendering.

use askama::Template;
use rdm_core::config::QuickFilter;
use rdm_core::model::PhaseStatus;

/// A rendered quick-filter chip, paired with the URL it should navigate to.
pub struct QuickFilterView {
    /// User-facing label.
    pub label: String,
    /// Tag value the chip applies.
    pub tag: String,
    /// Pre-built `?tag=<tag>` href targeting the page the chip is rendered on.
    pub href: String,
    /// `true` when the page's currently active tag equals this chip's `tag`.
    pub is_active: bool,
}

/// Percent-encode a tag value for safe inclusion in a `?tag=` query string.
///
/// Encodes everything outside the unreserved set (alphanumerics, `-`, `.`,
/// `_`, `~`) per RFC 3986. Tag values in practice are alphanumeric+dashes,
/// but we encode defensively so chips don't break on stray characters.
pub fn encode_tag_value(tag: &str) -> String {
    let mut out = String::with_capacity(tag.len());
    for b in tag.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'.' || b == b'_' || b == b'~' {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Build a [`QuickFilterView`] list for a given page path and active tag.
pub fn quick_filter_views(
    quick_filters: &[QuickFilter],
    page_path: &str,
    active_tag: Option<&str>,
) -> Vec<QuickFilterView> {
    quick_filters
        .iter()
        .map(|qf| QuickFilterView {
            label: qf.label.clone(),
            tag: qf.tag.clone(),
            href: format!("{page_path}?tag={}", encode_tag_value(&qf.tag)),
            is_active: active_tag == Some(qf.tag.as_str()),
        })
        .collect()
}

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
    /// Display priority, if set.
    pub priority: Option<String>,
    /// CSS class for the priority badge, if set.
    pub priority_class: Option<String>,
}

/// Roadmaps list page for a project.
#[derive(Template)]
#[template(path = "roadmaps.html")]
pub struct RoadmapsPage {
    /// Project name.
    pub project: String,
    /// All roadmaps with progress.
    pub roadmaps: Vec<RoadmapSummaryView>,
    /// Whether completed roadmaps are currently shown.
    pub show_completed: bool,
    /// Quick-filter chips for tag presets.
    pub quick_filters: Vec<QuickFilterView>,
    /// Currently active `?tag=` filter, if any.
    pub active_tag: Option<String>,
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
    /// Display priority, if set.
    pub priority: Option<String>,
    /// CSS class for the priority badge, if set.
    pub priority_class: Option<String>,
    /// Optional dependencies.
    pub dependencies: Option<Vec<String>>,
    /// Optional roadmap tags.
    pub tags: Option<Vec<String>>,
    /// Rendered HTML body.
    pub body_html: String,
    /// Phases in this roadmap (filtered by `active_tag` if set).
    pub phases: Vec<PhaseRow>,
    /// Quick-filter chips for tag presets.
    pub quick_filters: Vec<QuickFilterView>,
    /// Currently active `?tag=` filter, if any.
    pub active_tag: Option<String>,
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
    /// Whether completed tasks are currently shown.
    pub show_completed: bool,
    /// Quick-filter chips for tag presets.
    pub quick_filters: Vec<QuickFilterView>,
    /// Currently active `?tag=` filter, if any.
    pub active_tag: Option<String>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_tag_value_passes_through_alphanumerics() {
        assert_eq!(encode_tag_value("bug"), "bug");
        assert_eq!(encode_tag_value("ui-work"), "ui-work");
        assert_eq!(encode_tag_value("v1.2_alpha~beta"), "v1.2_alpha~beta");
    }

    #[test]
    fn encode_tag_value_percent_encodes_specials() {
        assert_eq!(encode_tag_value("a b"), "a%20b");
        assert_eq!(encode_tag_value("a&b"), "a%26b");
        assert_eq!(encode_tag_value("a/b"), "a%2Fb");
    }

    #[test]
    fn quick_filter_views_marks_active_chip() {
        let filters = vec![
            QuickFilter {
                label: "Bugs".into(),
                tag: "bug".into(),
            },
            QuickFilter {
                label: "UI".into(),
                tag: "ui".into(),
            },
        ];
        let views = quick_filter_views(&filters, "/projects/p/tasks", Some("ui"));
        assert_eq!(views.len(), 2);
        assert!(!views[0].is_active);
        assert_eq!(views[0].href, "/projects/p/tasks?tag=bug");
        assert!(views[1].is_active);
        assert_eq!(views[1].href, "/projects/p/tasks?tag=ui");
    }

    #[test]
    fn quick_filter_views_no_active_when_tag_unmatched() {
        let filters = vec![QuickFilter {
            label: "Bugs".into(),
            tag: "bug".into(),
        }];
        let views = quick_filter_views(&filters, "/x", Some("other"));
        assert!(!views[0].is_active);
    }
}
