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
        rdm_core::model::PhaseStatus::NeedsReview => "needs-review",
        rdm_core::model::PhaseStatus::Reviewed => "reviewed",
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
        rdm_core::model::TaskStatus::NeedsReview => "needs-review",
        rdm_core::model::TaskStatus::Reviewed => "reviewed",
        rdm_core::model::TaskStatus::Done => "done",
        rdm_core::model::TaskStatus::Blocked => "blocked",
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
/// Returns all phase statuses (`not-started`, `in-progress`, `needs-review`,
/// `reviewed`, `done`, `blocked`, `wont-fix`), marking the entry matching
/// `current` as selected.
/// Values are the canonical kebab-case strings parsed by `PhaseStatus::from_str`;
/// labels are sentence-case for display.
pub fn phase_status_options(current: &rdm_core::model::PhaseStatus) -> Vec<StatusOption> {
    use rdm_core::model::PhaseStatus;
    [
        ("not-started", "Not started", PhaseStatus::NotStarted),
        ("in-progress", "In progress", PhaseStatus::InProgress),
        ("needs-review", "Needs review", PhaseStatus::NeedsReview),
        ("reviewed", "Reviewed", PhaseStatus::Reviewed),
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
/// Returns all task statuses (`open`, `in-progress`, `needs-review`,
/// `reviewed`, `done`, `blocked`, `wont-fix`), marking the entry matching
/// `current` as selected.
/// Values are the canonical kebab-case strings parsed by `TaskStatus::from_str`;
/// labels are sentence-case for display.
pub fn task_status_options(current: &rdm_core::model::TaskStatus) -> Vec<StatusOption> {
    use rdm_core::model::TaskStatus;
    [
        ("open", "Open", TaskStatus::Open),
        ("in-progress", "In progress", TaskStatus::InProgress),
        ("needs-review", "Needs review", TaskStatus::NeedsReview),
        ("reviewed", "Reviewed", TaskStatus::Reviewed),
        ("done", "Done", TaskStatus::Done),
        ("blocked", "Blocked", TaskStatus::Blocked),
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

/// Helper to map a review lifecycle state to its display label.
///
/// `Draft` is included for totality but never reaches a template: the web
/// pages render only non-draft reviews.
pub fn review_state_label(state: &rdm_core::model::ReviewState) -> &'static str {
    match state {
        rdm_core::model::ReviewState::Draft => "Draft",
        rdm_core::model::ReviewState::Submitted => "Submitted",
        rdm_core::model::ReviewState::Addressed => "Addressed",
        rdm_core::model::ReviewState::Dismissed => "Dismissed",
    }
}

/// Helper to map a review lifecycle state to its CSS badge class.
pub fn review_state_class(state: &rdm_core::model::ReviewState) -> &'static str {
    match state {
        rdm_core::model::ReviewState::Draft => "draft",
        rdm_core::model::ReviewState::Submitted => "submitted",
        rdm_core::model::ReviewState::Addressed => "addressed",
        rdm_core::model::ReviewState::Dismissed => "dismissed",
    }
}

/// Helper to map a review verdict to its display label.
pub fn verdict_label(verdict: &rdm_core::model::Verdict) -> &'static str {
    match verdict {
        rdm_core::model::Verdict::Approve => "Approve",
        rdm_core::model::Verdict::RequestChanges => "Request changes",
        rdm_core::model::Verdict::Comment => "Comment",
    }
}

/// Helper to map a review verdict to its CSS badge class.
pub fn verdict_class(verdict: &rdm_core::model::Verdict) -> &'static str {
    match verdict {
        rdm_core::model::Verdict::Approve => "approve",
        rdm_core::model::Verdict::RequestChanges => "request-changes",
        rdm_core::model::Verdict::Comment => "verdict-comment",
    }
}

/// Helper to map a review comment status to its display label.
pub fn comment_status_label(status: &rdm_core::model::ReviewCommentStatus) -> &'static str {
    match status {
        rdm_core::model::ReviewCommentStatus::Open => "Open",
        rdm_core::model::ReviewCommentStatus::Addressed => "Addressed",
        rdm_core::model::ReviewCommentStatus::WontFix => "Won't fix",
    }
}

/// Helper to map a review comment status to its CSS badge class.
pub fn comment_status_class(status: &rdm_core::model::ReviewCommentStatus) -> &'static str {
    match status {
        rdm_core::model::ReviewCommentStatus::Open => "open",
        rdm_core::model::ReviewCommentStatus::Addressed => "addressed",
        rdm_core::model::ReviewCommentStatus::WontFix => "wont-fix",
    }
}

/// Formats how long ago `dt` was relative to `now`, coarsely
/// ("just now", "5 minutes ago", "3 hours ago", "2 days ago").
///
/// Future timestamps (clock skew between writer and server) clamp to
/// "just now" rather than rendering a negative duration.
pub fn relative_time_at(
    dt: chrono::DateTime<chrono::Utc>,
    now: chrono::DateTime<chrono::Utc>,
) -> String {
    let secs = (now - dt).num_seconds().max(0);
    let (n, unit) = match secs {
        0..60 => return "just now".to_string(),
        60..3600 => (secs / 60, "minute"),
        3600..86400 => (secs / 3600, "hour"),
        _ => (secs / 86400, "day"),
    };
    let s = if n == 1 { "" } else { "s" };
    format!("{n} {unit}{s} ago")
}

/// [`relative_time_at`] against the current wall clock.
pub fn relative_time(dt: chrono::DateTime<chrono::Utc>) -> String {
    relative_time_at(dt, chrono::Utc::now())
}

/// A link from a review comment to a different document than the page it
/// is rendered on — a roadmap-review comment scoped into a phase links
/// forward to that phase, and the same comment shown on the phase page
/// links back to the roadmap review.
pub struct DocLink {
    /// Link text.
    pub label: String,
    /// Link href (detail page plus fragment).
    pub href: String,
}

/// One review comment prepared for HTML rendering.
pub struct ReviewCommentView {
    /// Stable per-comment reference (`<review-id>-c<comment-id>`); used as
    /// the comment's DOM id and, when highlightable, mirrored by the
    /// in-body `<mark data-rdm-anchor>` it controls.
    pub anchor_ref: String,
    /// Display status text.
    pub status: String,
    /// CSS class for the status badge.
    pub status_class: String,
    /// Commit SHA recorded when the comment was addressed, if any.
    pub applied_commit: Option<String>,
    /// Rendered HTML of the comment body.
    pub body_html: String,
    /// Rendered HTML of the agent reply, when present.
    pub reply_html: Option<String>,
    /// Cross-document link, when the comment targets a different document
    /// than the page it is rendered on.
    pub cross_link: Option<DocLink>,
    /// `Some("Whole document")` for un-anchored comments; `None` for
    /// anchored ones (which show `quote_text` instead).
    pub anchor_label: Option<String>,
    /// The anchored text (resolved quote, or the stored quote when the
    /// anchor no longer resolves), when the comment is anchored.
    pub quote_text: Option<String>,
    /// `true` when hovering the quote should light up an in-body highlight
    /// (i.e. an inline `<mark>` with this comment's `anchor_ref` exists in
    /// the rendered body).
    pub quote_highlightable: bool,
    /// `true` when the anchor is drifted or unresolved — rendered as an
    /// "outdated" badge with the original quote shown instead of a
    /// highlight.
    pub outdated: bool,
}

/// One review prepared for HTML rendering in a detail page's Reviews
/// section.
pub struct ReviewView {
    /// Review id (used as the DOM fragment `#review-<id>`).
    pub id: String,
    /// Review author.
    pub author: String,
    /// Relative creation time (e.g. "3 hours ago").
    pub created_relative: String,
    /// Display state text.
    pub state: String,
    /// CSS class for the state badge.
    pub state_class: String,
    /// Display verdict text, when stamped.
    pub verdict: Option<String>,
    /// CSS class for the verdict badge, when stamped.
    pub verdict_class: Option<String>,
    /// Rendered HTML of the review summary.
    pub summary_html: String,
    /// The review's comments, in order.
    pub comments: Vec<ReviewCommentView>,
    /// Form action for the inline Dismiss control; `Some` only while the
    /// review is submitted (drafts never render here, and terminal states
    /// have nothing left to dismiss).
    pub dismiss_href: Option<String>,
}

/// One phase option for the draft panel's `doc` scope dropdowns (roadmap
/// pages only).
pub struct DocOption {
    /// Phase file stem — the `doc_stem` form value.
    pub stem: String,
    /// Display label ("Phase N: Title").
    pub label: String,
    /// Whether this option is the comment's current scope.
    pub selected: bool,
}

/// One pending comment rendered in the draft panel with its edit/remove
/// forms.
pub struct DraftCommentView {
    /// Comment id within the review.
    pub id: u32,
    /// Raw markdown of the comment body, backing the edit textarea.
    pub body_md: String,
    /// Display label of the comment's scope ("Whole document" or the
    /// targeted phase).
    pub doc_label: String,
    /// Phase options for the edit form's scope dropdown, with the current
    /// scope flagged `selected`. Empty on non-roadmap reviews (no dropdown).
    pub doc_options: Vec<DocOption>,
    /// The quoted text of the comment's stored anchor, when it has one —
    /// rendered as a small preview above the pending comment.
    pub anchor_preview: Option<String>,
}

/// The visitor's open draft review, rendered in the draft panel.
pub struct DraftReviewView {
    /// Review id.
    pub id: String,
    /// Raw markdown of the draft summary, backing the submit-form textarea.
    pub summary_md: String,
    /// Pending comments, in order.
    pub comments: Vec<DraftCommentView>,
}

/// The draft-review panel on a detail page: either the visitor's open
/// draft on this document, or a "Start review" form.
pub struct DraftPanelView {
    /// The page document as a review target reference
    /// (`roadmap/<slug>`, `phase/<roadmap>/<stem>`, or `task/<slug>`) —
    /// the start form's hidden `target` field.
    pub target_ref: String,
    /// Prefill for the start form's author input (from the `rdm_author`
    /// cookie; empty when absent).
    pub author_value: String,
    /// The visitor's open draft on this document, if any.
    pub draft: Option<DraftReviewView>,
    /// Phase options for the add-comment form's scope dropdown (roadmap
    /// pages only; empty elsewhere).
    pub doc_options: Vec<DocOption>,
}

/// Standalone render of the draft panel (`_draft_panel.html`), returned as
/// an HTML fragment by the JSON responses of the review form endpoints so
/// the select-to-anchor client can refresh the panel without a full page
/// reload.
#[derive(Template)]
#[template(path = "draft_panel_fragment.html")]
pub struct DraftPanelFragment {
    /// Project name.
    pub project: String,
    /// The assembled panel state.
    pub panel: DraftPanelView,
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
    let status = rdm_core::ops::roadmap::computed_status(phases);
    (status.as_str(), status.as_str())
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
    /// Number of open (submitted) reviews on the roadmap, phase-targeted
    /// reviews rolled up (same numbers as `INDEX.md`).
    pub open_reviews: usize,
    /// Number of open comments across those reviews.
    pub open_comments: usize,
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

/// A phase row for the roadmap detail page's per-phase disclosures.
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
    /// Rendered HTML of the phase body, shown inside the collapsed
    /// disclosure. `None` when the body is empty or the page is pinned to
    /// a historical `?at=` revision (no phase bodies render there).
    pub body_html: Option<String>,
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
    /// Non-draft reviews of this roadmap, oldest first.
    pub reviews: Vec<ReviewView>,
    /// Draft-review panel; `None` when viewing a pinned `?at=` revision.
    pub draft_panel: Option<DraftPanelView>,
    /// Inline error from a redirected review-form action, if any.
    pub draft_error: Option<String>,
    /// `true` when the rendered bodies carry `rdm-src` selection
    /// annotations (the viewer has an open draft on this document).
    pub annotated: bool,
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
    /// Non-draft reviews of this phase (including roadmap-review comments
    /// scoped into it), oldest first.
    pub reviews: Vec<ReviewView>,
    /// Draft-review panel; `None` when viewing a pinned `?at=` revision.
    pub draft_panel: Option<DraftPanelView>,
    /// Inline error from a redirected review-form action, if any.
    pub draft_error: Option<String>,
    /// `true` when the rendered body carries `rdm-src` selection
    /// annotations (the viewer has an open draft on this document).
    pub annotated: bool,
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
    /// Number of open (submitted) reviews on the task (same numbers as
    /// `INDEX.md`).
    pub open_reviews: usize,
    /// Number of open comments across those reviews.
    pub open_comments: usize,
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
    /// Non-draft reviews of this task, oldest first.
    pub reviews: Vec<ReviewView>,
    /// Draft-review panel; `None` when viewing a pinned `?at=` revision.
    pub draft_panel: Option<DraftPanelView>,
    /// Inline error from a redirected review-form action, if any.
    pub draft_error: Option<String>,
    /// `true` when the rendered body carries `rdm-src` selection
    /// annotations (the viewer has an open draft on this document).
    pub annotated: bool,
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
            vec![
                "not-started",
                "in-progress",
                "needs-review",
                "reviewed",
                "done",
                "blocked",
                "wont-fix"
            ]
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
        assert_eq!(
            values,
            vec![
                "open",
                "in-progress",
                "needs-review",
                "reviewed",
                "done",
                "blocked",
                "wont-fix"
            ]
        );
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

    #[test]
    fn computed_roadmap_status_lone_needs_review_is_in_progress() {
        assert_eq!(
            computed_roadmap_status(&[PhaseStatus::NeedsReview]),
            ("in-progress", "in-progress")
        );
    }

    #[test]
    fn computed_roadmap_status_lone_reviewed_is_in_progress() {
        assert_eq!(
            computed_roadmap_status(&[PhaseStatus::Reviewed]),
            ("in-progress", "in-progress")
        );
    }

    #[test]
    fn computed_roadmap_status_not_started_and_needs_review_is_in_progress() {
        assert_eq!(
            computed_roadmap_status(&[PhaseStatus::NotStarted, PhaseStatus::NeedsReview]),
            ("in-progress", "in-progress")
        );
    }

    #[test]
    fn computed_roadmap_status_not_started_and_reviewed_is_in_progress() {
        assert_eq!(
            computed_roadmap_status(&[PhaseStatus::NotStarted, PhaseStatus::Reviewed]),
            ("in-progress", "in-progress")
        );
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
            reviews: Vec::new(),
            draft_panel: None,
            draft_error: None,
            annotated: false,
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
            reviews: Vec::new(),
            draft_panel: None,
            draft_error: None,
            annotated: false,
        };
        page.render().unwrap()
    }

    /// The pinned-`?at=` view matches the existing revision gates: phase
    /// disclosures render summary-only (no body markup, no doc attrs, no
    /// annotations in the DOM flow at all).
    #[test]
    fn roadmap_detail_pinned_revision_renders_summary_only_disclosures() {
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
            phases: vec![PhaseRow {
                phase: 1,
                stem: "phase-1-first".to_string(),
                title: "First".to_string(),
                status: "done".to_string(),
                status_class: "done".to_string(),
                // The handler passes None for every phase when ?at= is set.
                body_html: None,
            }],
            quick_filters: Vec::new(),
            active_tag: None,
            revision: Some("abcdef1".to_string()),
            reviews: Vec::new(),
            draft_panel: None,
            draft_error: None,
            annotated: false,
        };
        let html = page.render().unwrap();
        assert!(
            html.contains(r#"<details class="phase-disclosure" id="phase-phase-1-first">"#),
            "summary-only disclosure still renders: {html}"
        );
        assert!(
            !html.contains("data-rdm-doc"),
            "no phase body markup on a pinned revision: {html}"
        );
        assert!(!html.contains("data-rdm-annotated"), "{html}");
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
            reviews: Vec::new(),
            draft_panel: None,
            draft_error: None,
            annotated: false,
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

    // -- review display helpers --

    #[test]
    fn review_state_labels_and_classes_cover_all_variants() {
        use rdm_core::model::ReviewState::*;
        for (state, label, class) in [
            (Draft, "Draft", "draft"),
            (Submitted, "Submitted", "submitted"),
            (Addressed, "Addressed", "addressed"),
            (Dismissed, "Dismissed", "dismissed"),
        ] {
            assert_eq!(review_state_label(&state), label);
            assert_eq!(review_state_class(&state), class);
        }
    }

    #[test]
    fn verdict_labels_and_classes_cover_all_variants() {
        use rdm_core::model::Verdict::*;
        for (verdict, label, class) in [
            (Approve, "Approve", "approve"),
            (RequestChanges, "Request changes", "request-changes"),
            (Comment, "Comment", "verdict-comment"),
        ] {
            assert_eq!(verdict_label(&verdict), label);
            assert_eq!(verdict_class(&verdict), class);
        }
    }

    #[test]
    fn comment_status_labels_and_classes_cover_all_variants() {
        use rdm_core::model::ReviewCommentStatus::*;
        for (status, label, class) in [
            (Open, "Open", "open"),
            (Addressed, "Addressed", "addressed"),
            (WontFix, "Won't fix", "wont-fix"),
        ] {
            assert_eq!(comment_status_label(&status), label);
            assert_eq!(comment_status_class(&status), class);
        }
    }

    #[test]
    fn relative_time_buckets() {
        use chrono::{Duration, Utc};
        let now = Utc::now();
        assert_eq!(relative_time_at(now, now), "just now");
        assert_eq!(
            relative_time_at(now - Duration::seconds(59), now),
            "just now"
        );
        assert_eq!(
            relative_time_at(now - Duration::minutes(1), now),
            "1 minute ago"
        );
        assert_eq!(
            relative_time_at(now - Duration::minutes(5), now),
            "5 minutes ago"
        );
        assert_eq!(
            relative_time_at(now - Duration::hours(3), now),
            "3 hours ago"
        );
        assert_eq!(relative_time_at(now - Duration::days(1), now), "1 day ago");
        assert_eq!(
            relative_time_at(now - Duration::days(40), now),
            "40 days ago"
        );
    }

    #[test]
    fn relative_time_future_clamps_to_just_now() {
        use chrono::{Duration, Utc};
        let now = Utc::now();
        assert_eq!(relative_time_at(now + Duration::hours(2), now), "just now");
    }

    #[test]
    fn styles_css_defines_review_section_rules() {
        // The reviews section introduces new badge classes (review state,
        // verdict, outdated) and the inline-anchor mark styling; all must
        // exist in the stylesheet, in both theme variable blocks.
        let css = include_str!("../assets/styles.css");
        for class in [
            ".badge-submitted {",
            ".badge-addressed {",
            ".badge-dismissed {",
            ".badge-approve {",
            ".badge-request-changes {",
            ".badge-verdict-comment {",
            ".badge-outdated {",
            "mark.rdm-anchor {",
            "mark.rdm-anchor.is-active {",
            ".reviews-section {",
            ".anchor-quote {",
        ] {
            assert!(css.contains(class), "styles.css must define a {class} rule");
        }
        for var in [
            "--badge-submitted-bg",
            "--badge-outdated-bg",
            "--anchor-highlight-bg",
        ] {
            assert_eq!(
                css.matches(&format!("{var}:")).count(),
                2,
                "{var} must be defined in both the light and dark blocks"
            );
        }
    }

    #[test]
    fn styles_css_defines_select_to_anchor_and_disclosure_rules() {
        // The select-to-anchor affordance/composer, the draft panel's
        // anchor-quote preview, the no-anchor fallback note, and the
        // roadmap page's per-phase disclosures all carry classes that
        // must be styled by the shipped stylesheet.
        let css = include_str!("../assets/styles.css");
        for class in [
            ".phase-disclosures {",
            ".phase-disclosure {",
            ".phase-disclosure summary {",
            ".rdm-anchor-affordance {",
            ".rdm-anchor-form {",
            ".rdm-anchor-form-quote,",
            ".draft-anchor-quote {",
            ".rdm-no-anchor-note {",
        ] {
            assert!(css.contains(class), "styles.css must define a {class} rule");
        }
    }

    #[test]
    fn styles_css_defines_review_status_badge_rules() {
        // The `needs-review` and `reviewed` statuses render as
        // `.badge-needs-review` / `.badge-reviewed`; the stylesheet must
        // define matching rules (and their CSS variables) so these badges
        // are styled rather than falling back to the bare `.badge` rule.
        let css = include_str!("../assets/styles.css");
        for class in [".badge-needs-review {", ".badge-reviewed {"] {
            assert!(css.contains(class), "styles.css must define a {class} rule");
        }
        for var in ["--badge-needs-review-bg", "--badge-reviewed-bg"] {
            assert!(
                css.contains(var),
                "styles.css must define the {var} CSS variable"
            );
        }
    }
}
