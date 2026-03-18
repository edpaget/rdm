//! Fuzzy search across plan repo content.
//!
//! Provides a [`search()`] function that finds roadmaps, phases, and tasks by
//! fuzzy-matching their titles and body content using the `nucleo-matcher` crate.

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use serde::Serialize;

use crate::error::Result;
use crate::model::{PhaseStatus, TaskStatus};
use crate::repo::PlanRepo;
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

/// Filters to narrow search results.
#[derive(Debug, Clone, Default)]
pub struct SearchFilter {
    /// Restrict results to a specific item kind.
    pub kind: Option<ItemKind>,
    /// Restrict results to a specific project.
    pub project: Option<String>,
    /// Restrict results to items matching a specific status.
    pub status: Option<ItemStatus>,
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
/// and tasks. Results are sorted by score in descending order.
///
/// # Errors
///
/// Returns an error if the plan repo cannot be read (e.g., missing projects
/// directory, unreadable files, or invalid frontmatter).
pub fn search<S: Store>(
    repo: &PlanRepo<S>,
    query: &str,
    filter: &SearchFilter,
) -> Result<Vec<SearchResult>> {
    let mut results = Vec::new();
    let mut matcher = Matcher::new(Config::DEFAULT);
    let pattern = Pattern::new(
        query,
        CaseMatching::Ignore,
        Normalization::Smart,
        AtomKind::Fuzzy,
    );
    let mut buf = Vec::new();

    let projects = repo.list_projects()?;

    for project in &projects {
        if let Some(ref fp) = filter.project
            && fp != project
        {
            continue;
        }

        // Search roadmaps (roadmaps have no status; skip when a status filter is active)
        if (filter.kind.is_none() || filter.kind == Some(ItemKind::Roadmap))
            && filter.status.is_none()
            && let Ok(roadmaps) = repo.list_roadmaps(project)
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
            && let Ok(roadmaps) = repo.list_roadmaps(project)
        {
            for roadmap_doc in &roadmaps {
                let roadmap_slug = &roadmap_doc.frontmatter.roadmap;
                if let Ok(phases) = repo.list_phases(project, roadmap_slug) {
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
            && let Ok(tasks) = repo.list_tasks(project)
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
    use crate::repo::PlanRepo;
    use crate::store::MemoryStore;

    /// Sets up a plan repo with sample data for testing.
    fn setup_test_repo() -> PlanRepo<MemoryStore> {
        let mut repo = PlanRepo::init(MemoryStore::new()).unwrap();

        // Create a project
        repo.create_project("acme", "Acme Corp").unwrap();

        // Create a roadmap with body
        repo.create_roadmap(
            "acme",
            "widget-launch",
            "Widget Launch",
            Some("Launch the new widget product line."),
        )
        .unwrap();

        // Create phases
        repo.create_phase(
            "acme",
            "widget-launch",
            "design",
            "Design the Widget",
            Some(1),
            Some("Create mockups and wireframes for the widget."),
        )
        .unwrap();
        repo.create_phase(
            "acme",
            "widget-launch",
            "implementation",
            "Implement the Widget",
            Some(2),
            Some("Build the widget according to the design specifications."),
        )
        .unwrap();

        // Mark phase 1 as done
        repo.update_phase(
            "acme",
            "widget-launch",
            "phase-1-design",
            Some(PhaseStatus::Done),
            None,
        )
        .unwrap();

        // Create tasks
        repo.create_task(
            "acme",
            "fix-login-bug",
            "Fix Login Bug",
            Priority::Medium,
            None,
            Some("Users cannot log in when password contains special characters."),
        )
        .unwrap();
        repo.create_task(
            "acme",
            "add-search",
            "Add Search Feature",
            Priority::Medium,
            None,
            Some("Implement full-text search across all content."),
        )
        .unwrap();

        // Mark one task as done
        repo.update_task(
            "acme",
            "fix-login-bug",
            Some(TaskStatus::Done),
            None,
            None,
            None,
        )
        .unwrap();

        repo
    }

    #[test]
    fn exact_title_match() {
        let repo = setup_test_repo();
        let filter = SearchFilter::default();
        let results = search(&repo, "Fix Login Bug", &filter).unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].title, "Fix Login Bug");
    }

    #[test]
    fn fuzzy_title_match() {
        let repo = setup_test_repo();
        let filter = SearchFilter::default();
        let results = search(&repo, "fx logn bg", &filter).unwrap();
        assert!(
            results.iter().any(|r| r.title == "Fix Login Bug"),
            "Expected fuzzy match for 'Fix Login Bug', got: {results:?}"
        );
    }

    #[test]
    fn body_content_match() {
        let repo = setup_test_repo();
        let filter = SearchFilter::default();
        let results = search(&repo, "special characters", &filter).unwrap();
        assert!(
            results.iter().any(|r| r.identifier == "fix-login-bug"),
            "Expected body match for task with 'special characters', got: {results:?}"
        );
    }

    #[test]
    fn no_results() {
        let repo = setup_test_repo();
        let filter = SearchFilter::default();
        let results = search(&repo, "xyzzy-nonexistent-qqq", &filter).unwrap();
        assert!(results.is_empty(), "Expected no results, got: {results:?}");
    }

    #[test]
    fn filter_by_kind_task() {
        let repo = setup_test_repo();
        let filter = SearchFilter {
            kind: Some(ItemKind::Task),
            ..Default::default()
        };
        let results = search(&repo, "widget", &filter).unwrap();
        for r in &results {
            assert_eq!(r.kind, ItemKind::Task, "Expected only tasks, got: {r:?}");
        }
    }

    #[test]
    fn filter_by_kind_phase() {
        let repo = setup_test_repo();
        let filter = SearchFilter {
            kind: Some(ItemKind::Phase),
            ..Default::default()
        };
        let results = search(&repo, "widget", &filter).unwrap();
        assert!(!results.is_empty(), "Expected phase results for 'widget'");
        for r in &results {
            assert_eq!(r.kind, ItemKind::Phase, "Expected only phases, got: {r:?}");
        }
    }

    #[test]
    fn filter_by_status() {
        let repo = setup_test_repo();
        let filter = SearchFilter {
            status: Some(ItemStatus::Task(TaskStatus::Done)),
            ..Default::default()
        };
        let results = search(&repo, "bug", &filter).unwrap();
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
        let mut repo = setup_test_repo();

        // Create a second project
        repo.create_project("other", "Other Project").unwrap();
        repo.create_task(
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
        let results = search(&repo, "task", &filter).unwrap();
        for r in &results {
            assert_eq!(r.project, "acme", "Expected only acme results, got: {r:?}");
        }
    }

    #[test]
    fn results_ranked_by_score() {
        let repo = setup_test_repo();
        let filter = SearchFilter::default();
        // "search" appears in the title of "Add Search Feature" and in the body
        // The title match should score higher.
        let results = search(&repo, "search", &filter).unwrap();
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
        let repo = setup_test_repo();
        let filter = SearchFilter::default();
        let results = search(&repo, "Widget Launch", &filter).unwrap();
        assert!(
            results
                .iter()
                .any(|r| r.kind == ItemKind::Roadmap && r.identifier == "widget-launch"),
            "Expected roadmap result for 'Widget Launch', got: {results:?}"
        );
    }

    #[test]
    fn phase_identifier_includes_roadmap() {
        let repo = setup_test_repo();
        let filter = SearchFilter {
            kind: Some(ItemKind::Phase),
            ..Default::default()
        };
        let results = search(&repo, "Design", &filter).unwrap();
        assert!(!results.is_empty(), "Expected phase results for 'Design'");
        let phase_result = &results[0];
        assert!(
            phase_result.identifier.starts_with("widget-launch/"),
            "Phase identifier should start with roadmap slug, got: {}",
            phase_result.identifier
        );
    }
}
