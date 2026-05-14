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
        rdm_core::model::PhaseStatus::WontFix => "wont-fix",
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

/// One `<option>` in a status `<select>`: `value` is the canonical
/// kebab-case status string, `label` is the human-readable form, and
/// `selected` is true when it matches the current status.
pub struct StatusOption {
    /// Canonical kebab-case status string used as the `<option value="…">`.
    pub value: &'static str,
    /// Human-readable text rendered as the `<option>`'s content.
    pub label: &'static str,
    /// `true` when this option matches the current status.
    pub selected: bool,
}

/// Build the option list for a phase status `<select>`, in lifecycle order.
///
/// Returns all five phase statuses (`not-started`, `in-progress`, `done`,
/// `blocked`, `wont-fix`), marking the entry matching `current` as selected.
/// Values are the canonical kebab-case strings parsed by `PhaseStatus::from_str`;
/// labels are sentence-case for display.
pub fn phase_status_options(current: &rdm_core::model::PhaseStatus) -> Vec<StatusOption> {
    use rdm_core::model::PhaseStatus;
    [
        ("not-started", "Not started", PhaseStatus::NotStarted),
        ("in-progress", "In progress", PhaseStatus::InProgress),
        ("done", "Done", PhaseStatus::Done),
        ("blocked", "Blocked", PhaseStatus::Blocked),
        ("wont-fix", "Won't fix", PhaseStatus::WontFix),
    ]
    .into_iter()
    .map(|(value, label, variant)| StatusOption {
        value,
        label,
        selected: *current == variant,
    })
    .collect()
}

/// Build the option list for a task status `<select>`, in lifecycle order.
///
/// Returns all four task statuses (`open`, `in-progress`, `done`,
/// `wont-fix`), marking the entry matching `current` as selected.
/// Values are the canonical kebab-case strings parsed by `TaskStatus::from_str`;
/// labels are sentence-case for display.
pub fn task_status_options(current: &rdm_core::model::TaskStatus) -> Vec<StatusOption> {
    use rdm_core::model::TaskStatus;
    [
        ("open", "Open", TaskStatus::Open),
        ("in-progress", "In progress", TaskStatus::InProgress),
        ("done", "Done", TaskStatus::Done),
        ("wont-fix", "Won't fix", TaskStatus::WontFix),
    ]
    .into_iter()
    .map(|(value, label, variant)| StatusOption {
        value,
        label,
        selected: *current == variant,
    })
    .collect()
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
/// - All phases terminal (`done` or `wont-fix`) → `("done", "done")`
/// - Any phase in-progress, or any terminal phase mixed with non-terminal phases → `("in-progress", "in-progress")`
/// - Otherwise (all not-started, all blocked, or no phases) → `("not-started", "not-started")`
pub fn computed_roadmap_status(phases: &[PhaseStatus]) -> (&'static str, &'static str) {
    if phases.is_empty() {
        return ("not-started", "not-started");
    }
    if phases.iter().all(PhaseStatus::is_terminal) {
        return ("done", "done");
    }
    let has_terminal = phases.iter().any(PhaseStatus::is_terminal);
    let has_in_progress = phases.contains(&PhaseStatus::InProgress);
    if has_in_progress || has_terminal {
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
    /// Raw markdown source of the body, used to back the inline editor textarea.
    pub body_md: String,
    /// Phases in this roadmap (filtered by `active_tag` if set).
    pub phases: Vec<PhaseRow>,
    /// Quick-filter chips for tag presets.
    pub quick_filters: Vec<QuickFilterView>,
    /// Currently active `?tag=` filter, if any.
    pub active_tag: Option<String>,
    /// Git revision the body is sourced from, when viewing at a historical SHA.
    pub revision: Option<String>,
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
    /// Raw markdown source of the body, used to back the inline editor textarea.
    pub body_md: String,
    /// Optional phase tags.
    pub tags: Option<Vec<String>>,
    /// URL for the previous phase, if any.
    pub prev_href: Option<String>,
    /// URL for the next phase, if any.
    pub next_href: Option<String>,
    /// Options for the inline status `<select>`. Lifecycle-ordered; the entry
    /// matching the current status is flagged `selected`.
    pub status_options: Vec<StatusOption>,
    /// Git revision the body is sourced from, when viewing at a historical SHA.
    pub revision: Option<String>,
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
    /// Raw markdown source of the body, used to back the inline editor textarea.
    pub body_md: String,
    /// Options for the inline status `<select>`. Lifecycle-ordered; the entry
    /// matching the current status is flagged `selected`.
    pub status_options: Vec<StatusOption>,
    /// Git revision the body is sourced from, when viewing at a historical SHA.
    pub revision: Option<String>,
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
    fn phase_status_options_marks_current_selected() {
        let opts = phase_status_options(&rdm_core::model::PhaseStatus::InProgress);
        let selected: Vec<&str> = opts
            .iter()
            .filter(|o| o.selected)
            .map(|o| o.value)
            .collect();
        assert_eq!(selected, vec!["in-progress"]);
    }

    #[test]
    fn phase_status_options_includes_wont_fix() {
        let opts = phase_status_options(&rdm_core::model::PhaseStatus::NotStarted);
        let values: Vec<&str> = opts.iter().map(|o| o.value).collect();
        assert_eq!(
            values,
            vec!["not-started", "in-progress", "done", "blocked", "wont-fix"]
        );
    }

    #[test]
    fn task_status_options_marks_current_selected() {
        let opts = task_status_options(&rdm_core::model::TaskStatus::Done);
        let selected: Vec<&str> = opts
            .iter()
            .filter(|o| o.selected)
            .map(|o| o.value)
            .collect();
        assert_eq!(selected, vec!["done"]);
        let values: Vec<&str> = opts.iter().map(|o| o.value).collect();
        assert_eq!(values, vec!["open", "in-progress", "done", "wont-fix"]);
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

    // -- Revision badge a11y tests --

    fn revision_page_html(revision: Option<String>) -> String {
        let page = TaskDetailPage {
            project: "demo".to_string(),
            slug: "fix-bug".to_string(),
            title: "Fix the bug".to_string(),
            status: "open".to_string(),
            status_class: "open".to_string(),
            priority: "medium".to_string(),
            priority_class: "medium".to_string(),
            created: "2026-05-01".to_string(),
            tags: None,
            body_html: String::new(),
            body_md: String::new(),
            status_options: task_status_options(&rdm_core::model::TaskStatus::Open),
            revision,
        };
        page.render().unwrap()
    }

    #[test]
    fn task_detail_renders_revision_badge_with_aria_live() {
        let html = revision_page_html(Some("abcdef1234".to_string()));
        assert!(
            html.contains(r#"class="revision-badge""#),
            "expected revision-badge container class, got:\n{html}"
        );
        assert!(
            html.contains(r#"aria-live="polite""#),
            "expected aria-live=polite on revision badge"
        );
        assert!(
            html.contains(r#"aria-label="Viewing historical revision""#),
            "expected aria-label on revision badge"
        );
        assert!(
            html.contains("Viewing revision abcdef1234"),
            "expected revision text to include sha"
        );
    }

    #[test]
    fn task_detail_omits_revision_badge_when_none() {
        let html = revision_page_html(None);
        assert!(
            !html.contains(r#"class="revision-badge""#),
            "revision badge must not render without revision"
        );
    }

    fn roadmap_detail_html(revision: Option<String>) -> String {
        let page = RoadmapDetailPage {
            project: "demo".to_string(),
            slug: "alpha".to_string(),
            title: "Alpha Roadmap".to_string(),
            status: "in-progress".to_string(),
            status_class: "in-progress".to_string(),
            last_changed: None,
            priority: None,
            priority_class: None,
            dependencies: None,
            tags: None,
            body_html: String::new(),
            body_md: String::new(),
            phases: Vec::new(),
            quick_filters: Vec::new(),
            active_tag: None,
            revision,
        };
        page.render().unwrap()
    }

    #[test]
    fn roadmap_detail_renders_revision_badge_with_aria_live() {
        let html = roadmap_detail_html(Some("abcdef1234".to_string()));
        assert!(html.contains(r#"class="revision-badge""#));
        assert!(html.contains(r#"aria-live="polite""#));
        assert!(html.contains(r#"aria-label="Viewing historical revision""#));
        assert!(html.contains("Viewing revision abcdef1234"));
    }

    #[test]
    fn roadmap_detail_omits_revision_badge_when_none() {
        let html = roadmap_detail_html(None);
        assert!(!html.contains(r#"class="revision-badge""#));
    }

    fn phase_detail_html(revision: Option<String>) -> String {
        let page = PhaseDetailPage {
            project: "demo".to_string(),
            roadmap: "alpha".to_string(),
            stem: "phase-1-core".to_string(),
            phase_number: 1,
            title: "Core".to_string(),
            status: "in-progress".to_string(),
            status_class: "in-progress".to_string(),
            completed: None,
            body_html: String::new(),
            body_md: String::new(),
            tags: None,
            prev_href: None,
            next_href: None,
            status_options: phase_status_options(&rdm_core::model::PhaseStatus::InProgress),
            revision,
        };
        page.render().unwrap()
    }

    #[test]
    fn phase_detail_renders_revision_badge_with_aria_live() {
        let html = phase_detail_html(Some("abcdef1234".to_string()));
        assert!(html.contains(r#"class="revision-badge""#));
        assert!(html.contains(r#"aria-live="polite""#));
        assert!(html.contains(r#"aria-label="Viewing historical revision""#));
        assert!(html.contains("Viewing revision abcdef1234"));
    }

    #[test]
    fn phase_detail_omits_revision_badge_when_none() {
        let html = phase_detail_html(None);
        assert!(!html.contains(r#"class="revision-badge""#));
    }

    #[test]
    fn task_detail_renders_revision_badge_class_is_styled() {
        // Render a page with a revision and confirm the badge class is on the
        // element so the external `styles.css` rule applies. The CSS rule
        // itself is verified by `styles_css_defines_badge_revision_rule`.
        let html = revision_page_html(Some("deadbeef".to_string()));
        assert!(
            html.contains(r#"class="badge badge-revision""#),
            "badge element must carry the badge-revision class"
        );
    }

    #[test]
    fn styles_css_defines_badge_revision_rule() {
        // The historical-view badge needs a distinctive style. The external
        // stylesheet must define a `.badge-revision` rule so the class on
        // the badge element actually picks up colors.
        let css = include_str!("../assets/styles.css");
        assert!(
            css.contains(".badge-revision {"),
            "styles.css must define a .badge-revision rule"
        );
        assert!(
            css.contains(".revision-badge {"),
            "styles.css must define a .revision-badge rule for the wrapper"
        );
        assert!(
            css.contains("--badge-revision-bg"),
            "styles.css must define --badge-revision-bg CSS variable"
        );
    }
}
