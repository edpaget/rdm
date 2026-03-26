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
    /// Minimum score as a fraction of the top result's score (0.0–1.0).
    ///
    /// Results scoring below `top_score * ratio` are dropped. Defaults to
    /// [`DEFAULT_MIN_SCORE_RATIO`] (0.25). Set to `Some(0.0)` to disable.
    pub min_score_ratio: Option<f64>,
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

        // Search roadmaps (roadmaps have no status; skip when a status filter is active)
        if (filter.kind.is_none() || filter.kind == Some(ItemKind::Roadmap))
            && filter.status.is_none()
            && let Ok(roadmaps) = crate::ops::roadmap::list_roadmaps(store, project, None, None)
        {
            for doc in &roadmaps {
                let rm = &doc.frontmatter;
                if let Some(result) = score_item(
                    &pattern,
                    &mut matcher,
                    &mut buf,
                    ItemKind::Roadmap,
                    &rm.roadmap,
                    project,
                    &rm.title,
                    &doc.body,
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
                if let Some(result) = score_item(
                    &pattern,
                    &mut matcher,
                    &mut buf,
                    ItemKind::Task,
                    slug,
                    project,
                    &task_doc.frontmatter.title,
                    &task_doc.body,
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
        assert!(results.len() >= 1, "Expected at least one result");

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
