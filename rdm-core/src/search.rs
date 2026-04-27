//! Fuzzy search across plan repo content.
//!
//! Provides a [`search()`] function that finds roadmaps, phases, and tasks by
//! fuzzy-matching their titles and body content using the `nucleo-matcher` crate.

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use serde::Serialize;

use crate::error::Result;
use crate::model::{PhaseStatus, TaskStatus};
use crate::store::Store;

/// The kind of plan item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ItemKind {
    /// A roadmap.
    Roadmap,
    /// A roadmap phase.
    Phase,
    /// A standalone task.
    Task,
}

impl std::fmt::Display for ItemKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ItemKind::Roadmap => write!(f, "roadmap"),
            ItemKind::Phase => write!(f, "phase"),
            ItemKind::Task => write!(f, "task"),
        }
    }
}

/// Unified status across phases and tasks for filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemStatus {
    /// A phase status.
    Phase(PhaseStatus),
    /// A task status.
    Task(TaskStatus),
}

/// Default minimum score ratio — results below this fraction of the top score
/// are dropped.
const DEFAULT_MIN_SCORE_RATIO: f64 = 0.25;

/// Filters to narrow search results.
#[derive(Debug, Clone, Default)]
pub struct SearchFilter {
    /// Restrict results to a specific item kind.
    pub kind: Option<ItemKind>,
    /// Restrict results to a specific project.
    pub project: Option<String>,
    /// Restrict results to items matching a specific status.
    pub status: Option<ItemStatus>,
    /// Restrict results to items carrying every listed tag (AND semantics).
    ///
    /// An item with no tags never matches a non-empty tag filter. Tag
    /// comparison is exact and case-sensitive — tags are not part of the
    /// fuzzy score, only a hard pre-filter. `None` and `Some(empty)` apply
    /// no tag constraint.
    pub tags: Option<Vec<String>>,
    /// Minimum score as a fraction of the top result's score (0.0–1.0).
    ///
    /// Results scoring below `top_score * ratio` are dropped. Defaults to
    /// [`DEFAULT_MIN_SCORE_RATIO`] (0.25). Set to `Some(0.0)` to disable.
    pub min_score_ratio: Option<f64>,
}

/// Returns `true` if `item_tags` contains every tag in `required`.
///
/// `required` empty → always matches. An item with `None` tags never
/// matches a non-empty `required`.
fn matches_required_tags(item_tags: Option<&Vec<String>>, required: &[String]) -> bool {
    if required.is_empty() {
        return true;
    }
    match item_tags {
        None => false,
        Some(tags) => required.iter().all(|t| tags.iter().any(|it| it == t)),
    }
}

/// A single search result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchResult {
    /// The kind of item matched.
    pub kind: ItemKind,
    /// Identifier for the item (slug for tasks/roadmaps, `roadmap-slug/phase-stem` for phases).
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

/// Searches the plan repo for items matching `query` with optional filters.
///
/// Performs fuzzy matching against titles and body content of roadmaps, phases,
/// and tasks. Results are sorted by score in descending order, then trimmed by
/// a relevance cutoff: any result whose score falls below
/// [`SearchFilter::min_score_ratio`] × the top result's score is dropped
/// (default 0.25). Set the ratio to `0.0` to disable the cutoff.
///
/// # Errors
///
/// Returns an error if the plan repo cannot be read (e.g., missing projects
/// directory, unreadable files, or invalid frontmatter).
pub fn search(store: &impl Store, query: &str, filter: &SearchFilter) -> Result<Vec<SearchResult>> {
    let mut results = Vec::new();
    let mut matcher = Matcher::new(Config::DEFAULT);
    let pattern = Pattern::new(
        query,
        CaseMatching::Ignore,
        Normalization::Smart,
        AtomKind::Fuzzy,
    );
    let mut buf = Vec::new();

    let projects = crate::ops::project::list_projects(store)?;

    for project in &projects {
        if let Some(ref fp) = filter.project
            && fp != project
        {
            continue;
        }

        let required_tags: &[String] = filter.tags.as_deref().unwrap_or(&[]);

        // Search roadmaps (roadmaps have no status; skip when a status filter is active)
        if (filter.kind.is_none() || filter.kind == Some(ItemKind::Roadmap))
            && filter.status.is_none()
            && let Ok(roadmaps) = crate::ops::roadmap::list_roadmaps(store, project, None, None)
        {
            for doc in &roadmaps {
                let rm = &doc.frontmatter;
                if !matches_required_tags(rm.tags.as_ref(), required_tags) {
                    continue;
                }
                if let Some(result) = score_item(
                    &pattern,
                    &mut matcher,
                    &mut buf,
                    ItemKind::Roadmap,
                    &rm.roadmap,
                    project,
                    &rm.title,
                    &doc.body,
                    rm.tags.as_ref(),
                ) {
                    results.push(result);
                }
            }
        }

        // Search phases
        if (filter.kind.is_none() || filter.kind == Some(ItemKind::Phase))
            && let Ok(roadmaps) = crate::ops::roadmap::list_roadmaps(store, project, None, None)
        {
            for roadmap_doc in &roadmaps {
                let roadmap_slug = &roadmap_doc.frontmatter.roadmap;
                if let Ok(phases) = crate::ops::phase::list_phases(store, project, roadmap_slug) {
                    for (stem, phase_doc) in &phases {
                        if let Some(ref fs) = filter.status
                            && *fs != ItemStatus::Phase(phase_doc.frontmatter.status)
                        {
                            continue;
                        }
                        if !matches_required_tags(
                            phase_doc.frontmatter.tags.as_ref(),
                            required_tags,
                        ) {
                            continue;
                        }
                        let identifier = format!("{roadmap_slug}/{stem}");
                        if let Some(result) = score_item(
                            &pattern,
                            &mut matcher,
                            &mut buf,
                            ItemKind::Phase,
                            &identifier,
                            project,
                            &phase_doc.frontmatter.title,
                            &phase_doc.body,
                            phase_doc.frontmatter.tags.as_ref(),
                        ) {
                            results.push(result);
                        }
                    }
                }
            }
        }

        // Search tasks
        if (filter.kind.is_none() || filter.kind == Some(ItemKind::Task))
            && let Ok(tasks) = crate::ops::task::list_tasks(store, project)
        {
            for (slug, task_doc) in &tasks {
                if let Some(ref fs) = filter.status
                    && *fs != ItemStatus::Task(task_doc.frontmatter.status)
                {
                    continue;
                }
                if !matches_required_tags(task_doc.frontmatter.tags.as_ref(), required_tags) {
                    continue;
                }
                if let Some(result) = score_item(
                    &pattern,
                    &mut matcher,
                    &mut buf,
                    ItemKind::Task,
                    slug,
                    project,
                    &task_doc.frontmatter.title,
                    &task_doc.body,
                    task_doc.frontmatter.tags.as_ref(),
                ) {
                    results.push(result);
                }
            }
        }
    }

    results.sort_by(|a, b| b.score.cmp(&a.score));

    // Drop results below the relevance cutoff.
    let ratio = filter
        .min_score_ratio
        .unwrap_or(DEFAULT_MIN_SCORE_RATIO)
        .clamp(0.0, 1.0);
    if ratio > 0.0
        && let Some(top) = results.first().map(|r| r.score)
    {
        let threshold = (top as f64 * ratio) as u32;
        results.retain(|r| r.score >= threshold);
    }

    Ok(results)
}

/// Scores a single item against the pattern and returns a `SearchResult` if it matches.
///
/// `tags` are passed through to the result for display/JSON output but are
/// not part of the score — tag filtering is handled before this call as a
/// hard pre-filter.
#[allow(clippy::too_many_arguments)]
fn score_item(
    pattern: &Pattern,
    matcher: &mut Matcher,
    buf: &mut Vec<char>,
    kind: ItemKind,
    identifier: &str,
    project: &str,
    title: &str,
    body: &str,
    tags: Option<&Vec<String>>,
) -> Option<SearchResult> {
    let title_score = pattern.score(Utf32Str::new(title, buf), matcher);
    let body_score = pattern.score(Utf32Str::new(body, buf), matcher);

    let best_score = match (title_score, body_score) {
        (Some(ts), Some(bs)) => Some(ts.max(bs)),
        (Some(ts), None) => Some(ts),
        (None, Some(bs)) => Some(bs),
        (None, None) => None,
    }?;

    let snippet = if title_score >= body_score {
        title.to_string()
    } else {
        extract_snippet(body, 80)
    };

    Some(SearchResult {
        kind,
        identifier: identifier.to_string(),
        project: project.to_string(),
        title: title.to_string(),
        snippet,
        score: best_score,
        tags: tags.cloned(),
    })
}

/// Extracts a snippet of approximately `max_len` characters from the body.
fn extract_snippet(body: &str, max_len: usize) -> String {
    let trimmed = body.trim();
    if trimmed.len() <= max_len {
        return trimmed.to_string();
    }
    // Find a char boundary at or before max_len
    let mut end = max_len;
    while end > 0 && !trimmed.is_char_boundary(end) {
        end -= 1;
    }
    let truncated = &trimmed[..end];
    format!("{truncated}...")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{PhaseStatus, Priority, TaskStatus};
    use crate::store::MemoryStore;

    /// Sets up a store with sample data for testing.
    fn setup_test_store() -> MemoryStore {
        let mut store = MemoryStore::new();
        crate::ops::init::init(&mut store).unwrap();

        // Create a project
        crate::ops::project::create_project(&mut store, "acme", "Acme Corp").unwrap();

        // Create a roadmap with body
        crate::ops::roadmap::create_roadmap(
            &mut store,
            "acme",
            "widget-launch",
            "Widget Launch",
            Some("Launch the new widget product line."),
            None,
            None,
        )
        .unwrap();

        // Create phases
        crate::ops::phase::create_phase(
            &mut store,
            "acme",
            "widget-launch",
            "design",
            "Design the Widget",
            Some(1),
            Some("Create mockups and wireframes for the widget."),
            None,
        )
        .unwrap();
        crate::ops::phase::create_phase(
            &mut store,
            "acme",
            "widget-launch",
            "implementation",
            "Implement the Widget",
            Some(2),
            Some("Build the widget according to the design specifications."),
            None,
        )
        .unwrap();

        // Mark phase 1 as done
        crate::ops::phase::update_phase(
            &mut store,
            "acme",
            "widget-launch",
            "phase-1-design",
            Some(PhaseStatus::Done),
            None,
            None,
            None,
        )
        .unwrap();

        // Create tasks
        crate::ops::task::create_task(
            &mut store,
            "acme",
            "fix-login-bug",
            "Fix Login Bug",
            Priority::Medium,
            None,
            Some("Users cannot log in when password contains special characters."),
        )
        .unwrap();
        crate::ops::task::create_task(
            &mut store,
            "acme",
            "add-search",
            "Add Search Feature",
            Priority::Medium,
            None,
            Some("Implement full-text search across all content."),
        )
        .unwrap();

        // Mark one task as done
        crate::ops::task::update_task(
            &mut store,
            "acme",
            "fix-login-bug",
            Some(TaskStatus::Done),
            None,
            None,
            None,
            None,
        )
        .unwrap();

        store
    }

    #[test]
    fn exact_title_match() {
        let store = setup_test_store();
        let filter = SearchFilter::default();
        let results = search(&store, "Fix Login Bug", &filter).unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].title, "Fix Login Bug");
    }

    #[test]
    fn fuzzy_title_match() {
        let store = setup_test_store();
        let filter = SearchFilter::default();
        let results = search(&store, "fx logn bg", &filter).unwrap();
        assert!(
            results.iter().any(|r| r.title == "Fix Login Bug"),
            "Expected fuzzy match for 'Fix Login Bug', got: {results:?}"
        );
    }

    #[test]
    fn body_content_match() {
        let store = setup_test_store();
        let filter = SearchFilter::default();
        let results = search(&store, "special characters", &filter).unwrap();
        assert!(
            results.iter().any(|r| r.identifier == "fix-login-bug"),
            "Expected body match for task with 'special characters', got: {results:?}"
        );
    }

    #[test]
    fn no_results() {
        let store = setup_test_store();
        let filter = SearchFilter::default();
        let results = search(&store, "xyzzy-nonexistent-qqq", &filter).unwrap();
        assert!(results.is_empty(), "Expected no results, got: {results:?}");
    }

    #[test]
    fn filter_by_kind_task() {
        let store = setup_test_store();
        let filter = SearchFilter {
            kind: Some(ItemKind::Task),
            ..Default::default()
        };
        let results = search(&store, "widget", &filter).unwrap();
        for r in &results {
            assert_eq!(r.kind, ItemKind::Task, "Expected only tasks, got: {r:?}");
        }
    }

    #[test]
    fn filter_by_kind_phase() {
        let store = setup_test_store();
        let filter = SearchFilter {
            kind: Some(ItemKind::Phase),
            ..Default::default()
        };
        let results = search(&store, "widget", &filter).unwrap();
        assert!(!results.is_empty(), "Expected phase results for 'widget'");
        for r in &results {
            assert_eq!(r.kind, ItemKind::Phase, "Expected only phases, got: {r:?}");
        }
    }

    #[test]
    fn filter_by_status() {
        let store = setup_test_store();
        let filter = SearchFilter {
            status: Some(ItemStatus::Task(TaskStatus::Done)),
            ..Default::default()
        };
        let results = search(&store, "bug", &filter).unwrap();
        assert!(
            results.iter().any(|r| r.identifier == "fix-login-bug"),
            "Expected done task 'fix-login-bug', got: {results:?}"
        );
        // "add-search" is open, should not appear
        assert!(
            !results.iter().any(|r| r.identifier == "add-search"),
            "Open task should be excluded by done filter"
        );
    }

    #[test]
    fn filter_by_project() {
        let mut store = setup_test_store();

        // Create a second project
        crate::ops::project::create_project(&mut store, "other", "Other Project").unwrap();
        crate::ops::task::create_task(
            &mut store,
            "other",
            "other-task",
            "Other Task",
            Priority::Medium,
            None,
            Some("Something else."),
        )
        .unwrap();

        let filter = SearchFilter {
            project: Some("acme".to_string()),
            ..Default::default()
        };
        let results = search(&store, "task", &filter).unwrap();
        for r in &results {
            assert_eq!(r.project, "acme", "Expected only acme results, got: {r:?}");
        }
    }

    #[test]
    fn results_ranked_by_score() {
        let store = setup_test_store();
        let filter = SearchFilter::default();
        // "search" appears in the title of "Add Search Feature" and in the body
        // The title match should score higher.
        let results = search(&store, "search", &filter).unwrap();
        assert!(!results.is_empty(), "Expected at least one result");

        // Verify descending score order
        for window in results.windows(2) {
            assert!(
                window[0].score >= window[1].score,
                "Results should be sorted by score descending: {} >= {}",
                window[0].score,
                window[1].score
            );
        }
    }

    #[test]
    fn searches_roadmaps() {
        let store = setup_test_store();
        let filter = SearchFilter::default();
        let results = search(&store, "Widget Launch", &filter).unwrap();
        assert!(
            results
                .iter()
                .any(|r| r.kind == ItemKind::Roadmap && r.identifier == "widget-launch"),
            "Expected roadmap result for 'Widget Launch', got: {results:?}"
        );
    }

    #[test]
    fn relevance_cutoff_drops_low_scoring_results() {
        let mut store = MemoryStore::new();
        crate::ops::init::init(&mut store).unwrap();
        crate::ops::project::create_project(&mut store, "p", "P").unwrap();

        // One task with a title that exactly matches the query
        crate::ops::task::create_task(
            &mut store,
            "p",
            "exact-match",
            "authentication",
            Priority::Medium,
            None,
            None,
        )
        .unwrap();
        // Another task whose body vaguely matches (low score)
        crate::ops::task::create_task(
            &mut store,
            "p",
            "vague-match",
            "Unrelated Topic",
            Priority::Medium,
            None,
            Some("This has nothing to do with anything but mentions auth once."),
        )
        .unwrap();

        // With default cutoff (0.25), the vague result may be dropped
        let filter_default = SearchFilter {
            project: Some("p".to_string()),
            ..Default::default()
        };
        let results_default = search(&store, "authentication", &filter_default).unwrap();

        // With cutoff disabled, all matching results are kept
        let filter_no_cutoff = SearchFilter {
            project: Some("p".to_string()),
            min_score_ratio: Some(0.0),
            ..Default::default()
        };
        let results_all = search(&store, "authentication", &filter_no_cutoff).unwrap();

        // Disabling cutoff should keep at least as many results
        assert!(
            results_all.len() >= results_default.len(),
            "Disabling cutoff should keep at least as many results: all={}, default={}",
            results_all.len(),
            results_default.len()
        );
    }

    #[test]
    fn relevance_cutoff_zero_keeps_all() {
        let store = setup_test_store();
        let filter = SearchFilter {
            min_score_ratio: Some(0.0),
            ..Default::default()
        };
        // A vague query that matches many items with varying scores
        let results = search(&store, "w", &filter).unwrap();
        // With 0.0 ratio, nothing is cut — every scored item remains
        assert!(!results.is_empty(), "Expected results with cutoff disabled");
        // Compare with strict cutoff
        let strict_filter = SearchFilter {
            min_score_ratio: Some(0.9),
            ..Default::default()
        };
        let strict_results = search(&store, "w", &strict_filter).unwrap();
        assert!(
            results.len() >= strict_results.len(),
            "Zero cutoff should keep at least as many results as strict cutoff"
        );
    }

    #[test]
    fn relevance_cutoff_clamps_out_of_range() {
        let store = setup_test_store();

        // Ratio > 1.0 is clamped to 1.0 (only the top scorer survives)
        let filter_high = SearchFilter {
            min_score_ratio: Some(5.0),
            ..Default::default()
        };
        let results = search(&store, "Widget", &filter_high).unwrap();
        // Should not be empty — top result always survives at ratio 1.0
        assert!(
            !results.is_empty(),
            "Ratio > 1.0 should clamp to 1.0, not drop everything"
        );

        // Negative ratio is clamped to 0.0 (cutoff disabled)
        let filter_neg = SearchFilter {
            min_score_ratio: Some(-1.0),
            ..Default::default()
        };
        let results_neg = search(&store, "Widget", &filter_neg).unwrap();
        let filter_zero = SearchFilter {
            min_score_ratio: Some(0.0),
            ..Default::default()
        };
        let results_zero = search(&store, "Widget", &filter_zero).unwrap();
        assert_eq!(
            results_neg.len(),
            results_zero.len(),
            "Negative ratio should behave like 0.0"
        );
    }

    #[test]
    fn relevance_cutoff_keeps_close_scores() {
        let store = setup_test_store();
        // "Widget" appears in multiple titles — scores should be close
        let filter = SearchFilter {
            min_score_ratio: Some(0.25),
            kind: Some(ItemKind::Phase),
            ..Default::default()
        };
        let results = search(&store, "Widget", &filter).unwrap();
        // Both phases have "Widget" in the title, scores should be close enough
        // that neither is dropped at 0.25 ratio
        assert!(
            results.len() >= 2,
            "Expected at least 2 phase results for 'Widget' with 0.25 cutoff, got: {results:?}"
        );
    }

    /// Builds a store seeded with tagged items spanning all three kinds.
    ///
    /// Layout:
    /// - roadmap `rust-cleanup` tagged `[refactor, rust]`
    ///   - phase 1 `rust-fmt` tagged `[refactor]`
    ///   - phase 2 `rust-clippy` tagged `[refactor, lint]`
    /// - roadmap `untagged-roadmap` (no tags)
    ///   - phase 1 `untagged-phase` (no tags)
    /// - task `bug-fix` tagged `[bug, refactor]`
    /// - task `untagged-task` (no tags)
    fn setup_tag_store() -> MemoryStore {
        let mut store = MemoryStore::new();
        crate::ops::init::init(&mut store).unwrap();
        crate::ops::project::create_project(&mut store, "p", "P").unwrap();

        crate::ops::roadmap::create_roadmap(
            &mut store,
            "p",
            "rust-cleanup",
            "Rust Cleanup",
            None,
            None,
            Some(vec!["refactor".to_string(), "rust".to_string()]),
        )
        .unwrap();
        crate::ops::phase::create_phase(
            &mut store,
            "p",
            "rust-cleanup",
            "fmt",
            "Rust Fmt",
            Some(1),
            None,
            Some(vec!["refactor".to_string()]),
        )
        .unwrap();
        crate::ops::phase::create_phase(
            &mut store,
            "p",
            "rust-cleanup",
            "clippy",
            "Rust Clippy",
            Some(2),
            None,
            Some(vec!["refactor".to_string(), "lint".to_string()]),
        )
        .unwrap();

        crate::ops::roadmap::create_roadmap(
            &mut store,
            "p",
            "untagged-roadmap",
            "Untagged Roadmap",
            None,
            None,
            None,
        )
        .unwrap();
        crate::ops::phase::create_phase(
            &mut store,
            "p",
            "untagged-roadmap",
            "phase",
            "Untagged Phase",
            Some(1),
            None,
            None,
        )
        .unwrap();

        crate::ops::task::create_task(
            &mut store,
            "p",
            "bug-fix",
            "Bug Fix Task",
            Priority::Medium,
            Some(vec!["bug".to_string(), "refactor".to_string()]),
            None,
        )
        .unwrap();
        crate::ops::task::create_task(
            &mut store,
            "p",
            "untagged-task",
            "Untagged Task",
            Priority::Medium,
            None,
            None,
        )
        .unwrap();

        store
    }

    #[test]
    fn matches_required_tags_helper() {
        let tags = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        // Empty required matches anything (including None).
        assert!(matches_required_tags(None, &[]));
        assert!(matches_required_tags(Some(&tags), &[]));
        // Non-empty required: None never matches.
        assert!(!matches_required_tags(None, &["a".to_string()]));
        // Subset matches.
        assert!(matches_required_tags(Some(&tags), &["a".to_string()]));
        assert!(matches_required_tags(
            Some(&tags),
            &["a".to_string(), "c".to_string()]
        ));
        // Missing tag fails AND.
        assert!(!matches_required_tags(
            Some(&tags),
            &["a".to_string(), "z".to_string()]
        ));
    }

    #[test]
    fn tag_filter_matches_tasks() {
        let store = setup_tag_store();
        let filter = SearchFilter {
            tags: Some(vec!["bug".to_string()]),
            min_score_ratio: Some(0.0),
            ..Default::default()
        };
        let results = search(&store, "task", &filter).unwrap();
        assert!(
            results.iter().any(|r| r.identifier == "bug-fix"),
            "Expected tagged task 'bug-fix' to match, got: {results:?}"
        );
        assert!(
            !results.iter().any(|r| r.identifier == "untagged-task"),
            "Untagged task must not match a non-empty tag filter"
        );
    }

    #[test]
    fn tag_filter_matches_phases_and_roadmaps() {
        let store = setup_tag_store();
        let filter = SearchFilter {
            tags: Some(vec!["refactor".to_string()]),
            min_score_ratio: Some(0.0),
            ..Default::default()
        };
        let results = search(&store, "rust", &filter).unwrap();

        let has =
            |kind: ItemKind, id: &str| results.iter().any(|r| r.kind == kind && r.identifier == id);

        assert!(
            has(ItemKind::Roadmap, "rust-cleanup"),
            "Expected roadmap rust-cleanup with tag refactor, got: {results:?}"
        );
        assert!(
            has(ItemKind::Phase, "rust-cleanup/phase-1-fmt"),
            "Expected phase fmt with tag refactor"
        );
        assert!(
            has(ItemKind::Phase, "rust-cleanup/phase-2-clippy"),
            "Expected phase clippy with tag refactor"
        );
        // Untagged roadmap and phase must be excluded.
        assert!(
            !has(ItemKind::Roadmap, "untagged-roadmap"),
            "Untagged roadmap must be excluded"
        );
    }

    #[test]
    fn tag_filter_and_semantics() {
        let store = setup_tag_store();
        // Both `refactor` and `lint` — only the clippy phase qualifies.
        let filter = SearchFilter {
            tags: Some(vec!["refactor".to_string(), "lint".to_string()]),
            min_score_ratio: Some(0.0),
            ..Default::default()
        };
        let results = search(&store, "rust", &filter).unwrap();
        assert!(
            results
                .iter()
                .any(|r| r.identifier == "rust-cleanup/phase-2-clippy"),
            "Expected clippy phase to match both tags"
        );
        assert!(
            !results
                .iter()
                .any(|r| r.identifier == "rust-cleanup/phase-1-fmt"),
            "fmt phase has only `refactor`, not `lint`; must be excluded"
        );
        assert!(
            !results.iter().any(|r| r.identifier == "rust-cleanup"),
            "rust-cleanup roadmap has refactor+rust but not lint; must be excluded"
        );
    }

    #[test]
    fn tag_filter_combined_with_kind() {
        let store = setup_tag_store();
        let filter = SearchFilter {
            kind: Some(ItemKind::Task),
            tags: Some(vec!["refactor".to_string()]),
            min_score_ratio: Some(0.0),
            ..Default::default()
        };
        let results = search(&store, "task", &filter).unwrap();
        // Only the bug-fix task carries `refactor`; phases/roadmaps are excluded by kind.
        for r in &results {
            assert_eq!(r.kind, ItemKind::Task);
        }
        assert!(results.iter().any(|r| r.identifier == "bug-fix"));
    }

    #[test]
    fn tag_filter_empty_query_returns_all_tagged() {
        // Empty query + tag filter: every tagged item matches; relevance cutoff
        // at 0.0 ensures none are dropped. Validates the "tag-only" UX.
        let store = setup_tag_store();
        let filter = SearchFilter {
            tags: Some(vec!["refactor".to_string()]),
            min_score_ratio: Some(0.0),
            ..Default::default()
        };
        let results = search(&store, "", &filter).unwrap();
        let has_id = |id: &str| results.iter().any(|r| r.identifier == id);
        assert!(has_id("rust-cleanup"), "got: {results:?}");
        assert!(has_id("rust-cleanup/phase-1-fmt"));
        assert!(has_id("rust-cleanup/phase-2-clippy"));
        assert!(has_id("bug-fix"));
        assert!(!has_id("untagged-task"));
        assert!(!has_id("untagged-roadmap"));
    }

    #[test]
    fn tag_filter_results_carry_tags() {
        let store = setup_tag_store();
        let filter = SearchFilter {
            tags: Some(vec!["bug".to_string()]),
            min_score_ratio: Some(0.0),
            ..Default::default()
        };
        let results = search(&store, "task", &filter).unwrap();
        let bug = results
            .iter()
            .find(|r| r.identifier == "bug-fix")
            .expect("bug-fix should be in results");
        assert_eq!(
            bug.tags.as_ref().expect("bug-fix has tags"),
            &vec!["bug".to_string(), "refactor".to_string()]
        );
    }

    #[test]
    fn empty_tags_vec_treated_as_no_filter() {
        // `Some(vec![])` should behave like `None` — no constraint applied.
        let store = setup_tag_store();
        let filter = SearchFilter {
            tags: Some(vec![]),
            min_score_ratio: Some(0.0),
            ..Default::default()
        };
        let results = search(&store, "task", &filter).unwrap();
        // Should include the untagged task too since no constraint.
        assert!(
            results.iter().any(|r| r.identifier == "untagged-task"),
            "Empty tags vec should not filter out untagged items"
        );
    }

    #[test]
    fn phase_identifier_includes_roadmap() {
        let store = setup_test_store();
        let filter = SearchFilter {
            kind: Some(ItemKind::Phase),
            ..Default::default()
        };
        let results = search(&store, "Design", &filter).unwrap();
        assert!(!results.is_empty(), "Expected phase results for 'Design'");
        let phase_result = &results[0];
        assert!(
            phase_result.identifier.starts_with("widget-launch/"),
            "Phase identifier should start with roadmap slug, got: {}",
            phase_result.identifier
        );
    }
}
