//! Fuzzy search across plan repo content.
//!
//! Provides a [`search()`] function that finds roadmaps, phases, and tasks by
//! fuzzy-matching their titles and body content using the `nucleo-matcher` crate.

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use serde::Serialize;

use std::str::FromStr;

use crate::error::Result;
use crate::link::ItemRef;
use crate::model::{ParseError, PhaseStatus, TaskStatus};
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
    /// A document review.
    Review,
}

impl std::fmt::Display for ItemKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ItemKind::Roadmap => write!(f, "roadmap"),
            ItemKind::Phase => write!(f, "phase"),
            ItemKind::Task => write!(f, "task"),
            ItemKind::Review => write!(f, "review"),
        }
    }
}

impl FromStr for ItemKind {
    type Err = ParseError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "roadmap" => Ok(ItemKind::Roadmap),
            "phase" => Ok(ItemKind::Phase),
            "task" => Ok(ItemKind::Task),
            "review" => Ok(ItemKind::Review),
            other => Err(ParseError::new(
                "item kind",
                other,
                "roadmap, phase, task, or review",
            )),
        }
    }
}

/// Parses a comma-separated list of item kinds (e.g. `roadmap,phase,task`).
///
/// Each member is trimmed and parsed with [`ItemKind`]'s `FromStr`; a
/// repeated kind is kept once. An empty string (or one of only commas and
/// whitespace) yields an empty list, which [`SearchFilter::kinds`] reads as
/// "every kind".
///
/// # Errors
///
/// Returns the [`ParseError`] of the first member that is not a kind.
///
/// # Examples
///
/// ```
/// use rdm_core::search::{ItemKind, parse_kinds};
///
/// assert_eq!(
///     parse_kinds("roadmap, phase").unwrap(),
///     vec![ItemKind::Roadmap, ItemKind::Phase]
/// );
/// assert!(parse_kinds("phase,bogus").is_err());
/// ```
pub fn parse_kinds(s: &str) -> std::result::Result<Vec<ItemKind>, ParseError> {
    let mut kinds = Vec::new();
    for member in s.split(',').map(str::trim).filter(|m| !m.is_empty()) {
        let kind: ItemKind = member.parse()?;
        if !kinds.contains(&kind) {
            kinds.push(kind);
        }
    }
    Ok(kinds)
}

/// Why a search status filter could not be built for a set of item kinds.
///
/// Returned by [`ItemStatus::parse_for_kinds`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusFilterError {
    /// The value is not a status of the requested kinds.
    Invalid(ParseError),
    /// None of the requested kinds carries a status a search can filter on:
    /// the set holds only roadmaps and/or reviews.
    NoStatus {
        /// The requested kinds (roadmaps and/or reviews only).
        kinds: Vec<ItemKind>,
    },
}

impl std::fmt::Display for StatusFilterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StatusFilterError::Invalid(e) => write!(f, "{e}"),
            StatusFilterError::NoStatus { kinds } => {
                let roadmap = kinds.contains(&ItemKind::Roadmap);
                let review = kinds.contains(&ItemKind::Review);
                match (roadmap, review) {
                    (true, false) => write!(
                        f,
                        "roadmaps do not have a status — remove --status or change --type"
                    ),
                    (false, true) => write!(
                        f,
                        "reviews do not use --status in search — filter with `rdm review list --state <state>` (or --verdict) instead"
                    ),
                    _ => write!(
                        f,
                        "roadmaps do not have a status and reviews do not use --status in search — remove --status or add --type phase or --type task"
                    ),
                }
            }
        }
    }
}

impl std::error::Error for StatusFilterError {}

impl From<ParseError> for StatusFilterError {
    fn from(e: ParseError) -> Self {
        StatusFilterError::Invalid(e)
    }
}

/// Unified status across phases and tasks for filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemStatus {
    /// A phase status.
    Phase(PhaseStatus),
    /// A task status.
    Task(TaskStatus),
    /// Match a phase or task whose status equals the given value — produced
    /// when a status string is valid for both kinds and no `--type` is given.
    Either(PhaseStatus, TaskStatus),
}

impl ItemStatus {
    /// Returns `true` if this filter matches a phase in `status`.
    ///
    /// `Phase(s)` matches only when `s == status`; `Task(_)` never matches a
    /// phase; `Either(p, _)` matches when `p == status`.
    pub fn matches_phase(self, status: PhaseStatus) -> bool {
        match self {
            ItemStatus::Phase(s) => s == status,
            ItemStatus::Task(_) => false,
            ItemStatus::Either(p, _) => p == status,
        }
    }

    /// Returns `true` if this filter matches a task in `status`.
    ///
    /// `Task(s)` matches only when `s == status`; `Phase(_)` never matches a
    /// task; `Either(_, t)` matches when `t == status`.
    pub fn matches_task(self, status: TaskStatus) -> bool {
        match self {
            ItemStatus::Task(s) => s == status,
            ItemStatus::Phase(_) => false,
            ItemStatus::Either(_, t) => t == status,
        }
    }
}

impl ItemStatus {
    /// Parses a status filter for a search over `kinds` (empty = every kind).
    ///
    /// - Every kind, or a set holding both phases and tasks: parsed as
    ///   [`ItemStatus`]'s `FromStr` does, so a value valid for both kinds
    ///   becomes [`ItemStatus::Either`].
    /// - Phases but not tasks: parsed as a [`PhaseStatus`].
    /// - Tasks but not phases: parsed as a [`TaskStatus`].
    /// - Only roadmaps and/or reviews: refused, since neither carries a
    ///   status a search filters on.
    ///
    /// # Errors
    ///
    /// Returns [`StatusFilterError::Invalid`] when `s` is not a status of the
    /// requested kinds, and [`StatusFilterError::NoStatus`] when no requested
    /// kind has a status.
    ///
    /// # Examples
    ///
    /// ```
    /// use rdm_core::model::TaskStatus;
    /// use rdm_core::search::{ItemKind, ItemStatus};
    ///
    /// assert_eq!(
    ///     ItemStatus::parse_for_kinds("open", &[ItemKind::Roadmap, ItemKind::Task]).unwrap(),
    ///     ItemStatus::Task(TaskStatus::Open)
    /// );
    /// assert!(ItemStatus::parse_for_kinds("open", &[ItemKind::Roadmap]).is_err());
    /// ```
    pub fn parse_for_kinds(
        s: &str,
        kinds: &[ItemKind],
    ) -> std::result::Result<Self, StatusFilterError> {
        if kinds.is_empty() {
            return Ok(s.parse()?);
        }
        match (
            kinds.contains(&ItemKind::Phase),
            kinds.contains(&ItemKind::Task),
        ) {
            (true, true) => Ok(s.parse()?),
            (true, false) => Ok(ItemStatus::Phase(s.parse()?)),
            (false, true) => Ok(ItemStatus::Task(s.parse()?)),
            (false, false) => Err(StatusFilterError::NoStatus {
                kinds: kinds.to_vec(),
            }),
        }
    }
}

impl FromStr for ItemStatus {
    type Err = ParseError;

    /// Parses a status string. A value valid for both phases and tasks yields
    /// [`ItemStatus::Either`]; one valid for only a single kind yields the
    /// corresponding variant.
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match (PhaseStatus::from_str(s).ok(), TaskStatus::from_str(s).ok()) {
            (Some(ps), Some(ts)) => Ok(ItemStatus::Either(ps, ts)),
            (Some(ps), None) => Ok(ItemStatus::Phase(ps)),
            (None, Some(ts)) => Ok(ItemStatus::Task(ts)),
            (None, None) => Err(ParseError::new(
                "status",
                s,
                "a phase status (not-started, in-progress, needs-review, reviewed, done, blocked, or wont-fix) or a task status (open, in-progress, needs-review, reviewed, done, blocked, or wont-fix)",
            )),
        }
    }
}

/// Default minimum score ratio — results below this fraction of the top score
/// are dropped.
const DEFAULT_MIN_SCORE_RATIO: f64 = 0.25;

/// Filters to narrow search results.
#[derive(Debug, Clone, Default)]
pub struct SearchFilter {
    /// Restrict results to these item kinds; empty keeps every kind.
    ///
    /// Applied while results are collected, so a caller's limit counts only
    /// items of these kinds.
    pub kinds: Vec<ItemKind>,
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

impl SearchFilter {
    /// Whether this filter keeps items of `kind`.
    fn wants(&self, kind: ItemKind) -> bool {
        self.kinds.is_empty() || self.kinds.contains(&kind)
    }
}

/// Returns `true` if `item_tags` contains every tag in `required`.
///
/// `required` empty → always matches. An item with `None` tags never
/// matches a non-empty `required`. Matching is exact and case-sensitive —
/// tags are a hard pre-filter and are never fuzzy-matched like the query.
///
/// A thin wrapper over [`crate::tags::matches_all_tags`], the single source
/// of truth for tag filtering across every rdm surface.
fn matches_required_tags(item_tags: Option<&[String]>, required: &[String]) -> bool {
    crate::tags::matches_all_tags(item_tags, required)
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
    /// The item's status, for a phase or task hit; `None` otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// The matched roadmap, phase, or task as a structured reference;
    /// `None` for a review hit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<ItemRef>,
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
/// Returns [`crate::error::Error::InvalidStoredChangeRevision`] for malformed resolved change
/// identities (head or present base must be 40 lowercase ASCII hex characters).
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
    let required_tags: &[String] = filter.tags.as_deref().unwrap_or(&[]);

    for project in &projects {
        if let Some(ref fp) = filter.project
            && fp != project
        {
            continue;
        }

        // Search roadmaps (roadmaps have no status; skip when a status filter is active)
        if filter.wants(ItemKind::Roadmap)
            && filter.status.is_none()
            && let Ok(roadmaps) = crate::ops::roadmap::list_roadmaps(store, project, None, None)
        {
            for doc in &roadmaps {
                let rm = &doc.frontmatter;
                if !matches_required_tags(rm.tags.as_deref(), required_tags) {
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
                    rm.tags.as_deref(),
                ) {
                    results.push(result.with_item(
                        None,
                        ItemRef::Roadmap {
                            roadmap: rm.roadmap.clone(),
                        },
                    ));
                }
            }
        }

        // Search phases
        if filter.wants(ItemKind::Phase)
            && let Ok(roadmaps) = crate::ops::roadmap::list_roadmaps(store, project, None, None)
        {
            for roadmap_doc in &roadmaps {
                let roadmap_slug = &roadmap_doc.frontmatter.roadmap;
                if let Ok(phases) = crate::ops::phase::list_phases(store, project, roadmap_slug) {
                    for (stem, phase_doc) in &phases {
                        if let Some(fs) = filter.status
                            && !fs.matches_phase(phase_doc.frontmatter.status)
                        {
                            continue;
                        }
                        if !matches_required_tags(
                            phase_doc.frontmatter.tags.as_deref(),
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
                            phase_doc.frontmatter.tags.as_deref(),
                        ) {
                            results.push(result.with_item(
                                Some(phase_doc.frontmatter.status.to_string()),
                                ItemRef::Phase {
                                    roadmap: roadmap_slug.clone(),
                                    stem: stem.clone(),
                                },
                            ));
                        }
                    }
                }
            }
        }

        // Search tasks
        if filter.wants(ItemKind::Task)
            && let Ok(tasks) = crate::ops::task::list_tasks(store, project)
        {
            for (slug, task_doc) in &tasks {
                if let Some(fs) = filter.status
                    && !fs.matches_task(task_doc.frontmatter.status)
                {
                    continue;
                }
                if !matches_required_tags(task_doc.frontmatter.tags.as_deref(), required_tags) {
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
                    task_doc.frontmatter.tags.as_deref(),
                ) {
                    results.push(result.with_item(
                        Some(task_doc.frontmatter.status.to_string()),
                        ItemRef::Task { slug: slug.clone() },
                    ));
                }
            }
        }

        // Search reviews. Their searchable text is the summary body plus
        // every comment body, so a match inside a comment surfaces the
        // review. Reviews carry no tags and their lifecycle states are not
        // part of the phase/task `ItemStatus` vocabulary, so both a status
        // filter and a tag filter skip them (mirroring how roadmaps skip
        // status filters); narrow reviews with `rdm review list
        // --state/--verdict` instead.
        if filter.wants(ItemKind::Review) && filter.status.is_none() && required_tags.is_empty() {
            let reviews = crate::ops::reviews::list_reviews(store, project)?;
            for (id, review_doc) in &reviews {
                let title = format!("review of {}", review_doc.frontmatter.target.label());
                let text: String = std::iter::once(review_doc.body.as_str())
                    .chain(
                        review_doc
                            .frontmatter
                            .comments
                            .iter()
                            .map(|c| c.body.as_str()),
                    )
                    .collect::<Vec<_>>()
                    .join("\n\n");
                if let Some(result) = score_item(
                    &pattern,
                    &mut matcher,
                    &mut buf,
                    ItemKind::Review,
                    id,
                    project,
                    &title,
                    &text,
                    None,
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
    tags: Option<&[String]>,
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
        crate::display::truncate_snippet(body, 80)
    };

    Some(SearchResult {
        kind,
        identifier: identifier.to_string(),
        project: project.to_string(),
        title: title.to_string(),
        snippet,
        score: best_score,
        tags: tags.map(|t| t.to_vec()),
        status: None,
        item: None,
    })
}

impl SearchResult {
    /// Attaches the hit's status and structured item reference.
    fn with_item(mut self, status: Option<String>, item: ItemRef) -> Self {
        self.status = status;
        self.item = Some(item);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{PhaseStatus, Priority, TaskStatus};
    use crate::store::MemoryStore;

    #[test]
    fn item_kind_from_str_round_trips_and_rejects_unknown() {
        for kind in [
            ItemKind::Roadmap,
            ItemKind::Phase,
            ItemKind::Task,
            ItemKind::Review,
        ] {
            assert_eq!(kind.to_string().parse::<ItemKind>().unwrap(), kind);
        }
        let err = "bogus".parse::<ItemKind>().unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid item kind: 'bogus' (expected roadmap, phase, task, or review)"
        );
    }

    #[test]
    fn item_status_from_str_resolves_kind() {
        // "done" is valid for both phases and tasks → kind-agnostic.
        assert_eq!(
            "done".parse::<ItemStatus>().unwrap(),
            ItemStatus::Either(PhaseStatus::Done, TaskStatus::Done)
        );
        // "blocked" is valid for both phases and tasks → kind-agnostic.
        assert_eq!(
            "blocked".parse::<ItemStatus>().unwrap(),
            ItemStatus::Either(PhaseStatus::Blocked, TaskStatus::Blocked)
        );
        // "not-started" is phase-only; "open" is task-only.
        assert_eq!(
            "not-started".parse::<ItemStatus>().unwrap(),
            ItemStatus::Phase(PhaseStatus::NotStarted)
        );
        assert_eq!(
            "open".parse::<ItemStatus>().unwrap(),
            ItemStatus::Task(TaskStatus::Open)
        );
        assert!("nonsense".parse::<ItemStatus>().is_err());
    }

    /// Sets up a store with sample data for testing.
    fn setup_test_store() -> MemoryStore {
        let mut store = MemoryStore::new();
        crate::ops::init::init(&mut store).unwrap();

        // Create a project
        crate::ops::project::create_project(&mut store, "acme", "Acme Corp").unwrap();

        // Create a roadmap with body
        crate::ops::roadmap::create_roadmap(
            &mut store,
            crate::ops::roadmap::CreateRoadmap {
                project: "acme",
                slug: "widget-launch",
                title: "Widget Launch",
                body: Some("Launch the new widget product line."),
                ..Default::default()
            },
        )
        .unwrap();

        // Create phases
        crate::ops::phase::create_phase(
            &mut store,
            crate::ops::phase::CreatePhase {
                project: "acme",
                roadmap: "widget-launch",
                slug: "design",
                title: "Design the Widget",
                number: Some(1),
                body: Some("Create mockups and wireframes for the widget."),
                ..Default::default()
            },
        )
        .unwrap();
        crate::ops::phase::create_phase(
            &mut store,
            crate::ops::phase::CreatePhase {
                project: "acme",
                roadmap: "widget-launch",
                slug: "implementation",
                title: "Implement the Widget",
                number: Some(2),
                body: Some("Build the widget according to the design specifications."),
                ..Default::default()
            },
        )
        .unwrap();

        // Mark phase 1 as done
        crate::ops::phase::update_phase(
            &mut store,
            "acme",
            "widget-launch",
            "phase-1-design",
            Some(PhaseStatus::Done),
            crate::ops::TagsUpdate::Keep,
            crate::ops::BodyUpdate::Keep,
            None,
            None,
            None,
            None,
            crate::ops::TitleUpdate::Keep,
        )
        .unwrap();

        // Create tasks
        crate::ops::task::create_task(
            &mut store,
            crate::ops::task::CreateTask {
                project: "acme",
                slug: "fix-login-bug",
                title: "Fix Login Bug",
                priority: Priority::Medium,
                tags: None,
                body: Some("Users cannot log in when password contains special characters."),
            },
        )
        .unwrap();
        crate::ops::task::create_task(
            &mut store,
            crate::ops::task::CreateTask {
                project: "acme",
                slug: "add-search",
                title: "Add Search Feature",
                priority: Priority::Medium,
                tags: None,
                body: Some("Implement full-text search across all content."),
            },
        )
        .unwrap();

        // Mark one task as done
        crate::ops::task::update_task(
            &mut store,
            "acme",
            "fix-login-bug",
            Some(TaskStatus::Done),
            None,
            crate::ops::TagsUpdate::Keep,
            crate::ops::BodyUpdate::Keep,
            None,
            None,
            None,
            None,
            crate::ops::TitleUpdate::Keep,
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
            kinds: vec![ItemKind::Task],
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
            kinds: vec![ItemKind::Phase],
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
    fn filter_by_needs_review_status() {
        let mut store = setup_test_store();

        // Move the implementation phase to needs-review.
        crate::ops::phase::update_phase(
            &mut store,
            "acme",
            "widget-launch",
            "phase-2-implementation",
            Some(PhaseStatus::NeedsReview),
            crate::ops::TagsUpdate::Keep,
            crate::ops::BodyUpdate::Keep,
            None,
            None,
            None,
            None,
            crate::ops::TitleUpdate::Keep,
        )
        .unwrap();

        let filter = SearchFilter {
            status: Some(ItemStatus::Phase(PhaseStatus::NeedsReview)),
            ..Default::default()
        };
        let results = search(&store, "widget", &filter).unwrap();
        assert!(
            results
                .iter()
                .any(|r| r.identifier == "widget-launch/phase-2-implementation"),
            "Expected needs-review phase, got: {results:?}"
        );
    }

    #[test]
    fn filter_by_reviewed_status_task() {
        let mut store = setup_test_store();

        // Move the add-search task to reviewed.
        crate::ops::task::update_task(
            &mut store,
            "acme",
            "add-search",
            Some(TaskStatus::Reviewed),
            None,
            crate::ops::TagsUpdate::Keep,
            crate::ops::BodyUpdate::Keep,
            None,
            None,
            None,
            None,
            crate::ops::TitleUpdate::Keep,
        )
        .unwrap();

        let filter = SearchFilter {
            status: Some(ItemStatus::Task(TaskStatus::Reviewed)),
            ..Default::default()
        };
        let results = search(&store, "search", &filter).unwrap();
        assert!(
            results.iter().any(|r| r.identifier == "add-search"),
            "Expected reviewed task, got: {results:?}"
        );
    }

    #[test]
    fn filter_by_ambiguous_status_matches_both_kinds() {
        let mut store = setup_test_store();

        // Move a phase and a task both to needs-review.
        crate::ops::phase::update_phase(
            &mut store,
            "acme",
            "widget-launch",
            "phase-2-implementation",
            Some(PhaseStatus::NeedsReview),
            crate::ops::TagsUpdate::Keep,
            crate::ops::BodyUpdate::Keep,
            None,
            None,
            None,
            None,
            crate::ops::TitleUpdate::Keep,
        )
        .unwrap();
        crate::ops::task::update_task(
            &mut store,
            "acme",
            "add-search",
            Some(TaskStatus::NeedsReview),
            None,
            crate::ops::TagsUpdate::Keep,
            crate::ops::BodyUpdate::Keep,
            None,
            None,
            None,
            None,
            crate::ops::TitleUpdate::Keep,
        )
        .unwrap();

        let filter = SearchFilter {
            status: Some(ItemStatus::Either(
                PhaseStatus::NeedsReview,
                TaskStatus::NeedsReview,
            )),
            ..Default::default()
        };
        let results = search(&store, "", &filter).unwrap();
        assert!(
            results
                .iter()
                .any(|r| r.identifier == "widget-launch/phase-2-implementation"),
            "Expected needs-review phase, got: {results:?}"
        );
        assert!(
            results.iter().any(|r| r.identifier == "add-search"),
            "Expected needs-review task, got: {results:?}"
        );
    }

    #[test]
    fn filter_by_either_excludes_other_status() {
        let store = setup_test_store();

        // fix-login-bug is done; an Either(_, NeedsReview) filter must exclude it.
        let filter = SearchFilter {
            status: Some(ItemStatus::Either(
                PhaseStatus::NeedsReview,
                TaskStatus::NeedsReview,
            )),
            ..Default::default()
        };
        let results = search(&store, "", &filter).unwrap();
        assert!(
            !results.iter().any(|r| r.identifier == "fix-login-bug"),
            "Done task should be excluded by needs-review Either filter, got: {results:?}"
        );
    }

    #[test]
    fn filter_by_project() {
        let mut store = setup_test_store();

        // Create a second project
        crate::ops::project::create_project(&mut store, "other", "Other Project").unwrap();
        crate::ops::task::create_task(
            &mut store,
            crate::ops::task::CreateTask {
                project: "other",
                slug: "other-task",
                title: "Other Task",
                priority: Priority::Medium,
                tags: None,
                body: Some("Something else."),
            },
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
            crate::ops::task::CreateTask {
                project: "p",
                slug: "exact-match",
                title: "authentication",
                priority: Priority::Medium,
                ..Default::default()
            },
        )
        .unwrap();
        // Another task whose body vaguely matches (low score)
        crate::ops::task::create_task(
            &mut store,
            crate::ops::task::CreateTask {
                project: "p",
                slug: "vague-match",
                title: "Unrelated Topic",
                priority: Priority::Medium,
                tags: None,
                body: Some("This has nothing to do with anything but mentions auth once."),
            },
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
            kinds: vec![ItemKind::Phase],
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
            crate::ops::roadmap::CreateRoadmap {
                project: "p",
                slug: "rust-cleanup",
                title: "Rust Cleanup",
                body: None,
                priority: None,
                tags: Some(vec!["refactor".to_string(), "rust".to_string()]),
            },
        )
        .unwrap();
        crate::ops::phase::create_phase(
            &mut store,
            crate::ops::phase::CreatePhase {
                project: "p",
                roadmap: "rust-cleanup",
                slug: "fmt",
                title: "Rust Fmt",
                number: Some(1),
                body: None,
                tags: Some(vec!["refactor".to_string()]),
                ..Default::default()
            },
        )
        .unwrap();
        crate::ops::phase::create_phase(
            &mut store,
            crate::ops::phase::CreatePhase {
                project: "p",
                roadmap: "rust-cleanup",
                slug: "clippy",
                title: "Rust Clippy",
                number: Some(2),
                body: None,
                tags: Some(vec!["refactor".to_string(), "lint".to_string()]),
                ..Default::default()
            },
        )
        .unwrap();

        crate::ops::roadmap::create_roadmap(
            &mut store,
            crate::ops::roadmap::CreateRoadmap {
                project: "p",
                slug: "untagged-roadmap",
                title: "Untagged Roadmap",
                ..Default::default()
            },
        )
        .unwrap();
        crate::ops::phase::create_phase(
            &mut store,
            crate::ops::phase::CreatePhase {
                project: "p",
                roadmap: "untagged-roadmap",
                slug: "phase",
                title: "Untagged Phase",
                number: Some(1),
                ..Default::default()
            },
        )
        .unwrap();

        crate::ops::task::create_task(
            &mut store,
            crate::ops::task::CreateTask {
                project: "p",
                slug: "bug-fix",
                title: "Bug Fix Task",
                priority: Priority::Medium,
                tags: Some(vec!["bug".to_string(), "refactor".to_string()]),
                ..Default::default()
            },
        )
        .unwrap();
        crate::ops::task::create_task(
            &mut store,
            crate::ops::task::CreateTask {
                project: "p",
                slug: "untagged-task",
                title: "Untagged Task",
                priority: Priority::Medium,
                ..Default::default()
            },
        )
        .unwrap();

        store
    }

    #[test]
    fn matches_required_tags_helper() {
        let tags = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let tags_slice: &[String] = &tags;
        // Empty required matches anything (including None).
        assert!(matches_required_tags(None, &[]));
        assert!(matches_required_tags(Some(tags_slice), &[]));
        // Non-empty required: None never matches.
        assert!(!matches_required_tags(None, &["a".to_string()]));
        // Subset matches.
        assert!(matches_required_tags(Some(tags_slice), &["a".to_string()]));
        assert!(matches_required_tags(
            Some(tags_slice),
            &["a".to_string(), "c".to_string()]
        ));
        // Missing tag fails AND.
        assert!(!matches_required_tags(
            Some(tags_slice),
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
            kinds: vec![ItemKind::Task],
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
            kinds: vec![ItemKind::Phase],
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

    // -- review search --

    /// Adds a review of `fix-login-bug` with a distinctive summary and one
    /// distinctive comment body, returning its id.
    fn add_review(store: &mut MemoryStore) -> String {
        let doc = crate::ops::reviews::create_review(
            store,
            crate::ops::reviews::CreateReview {
                project: "acme",
                author: "ed",
                target: crate::model::ReviewTarget::Task {
                    slug: "fix-login-bug".to_string(),
                },
                body: Some("Summary mentions zanzibar explicitly."),
                implements: None,
                change_branch: None,
            },
        )
        .unwrap();
        let id = doc.frontmatter.id.clone();
        crate::ops::reviews::add_comment(
            store,
            crate::ops::reviews::AddComment {
                project: "acme",
                review_id: &id,
                body: "Comment mentions quokka explicitly.",
                doc: None,
                anchor: None,
                ..Default::default()
            },
        )
        .unwrap();
        id
    }

    /// A `change/<sha>` review with a distinctive comment body, written
    /// directly (its target needs a source repo the ops layer never sees).
    fn add_change_review(store: &mut MemoryStore) -> String {
        let id = "2026-07-01-1200-cccc".to_string();
        let doc = crate::document::Document {
            frontmatter: crate::model::Review {
                id: id.clone(),
                author: "ed".to_string(),
                target: crate::model::ReviewTarget::Change {
                    head: "a".repeat(40),
                    base: Some("b".repeat(40)),
                },
                state: crate::model::ReviewState::Draft,
                verdict: None,
                created: chrono::Utc::now(),
                submitted: None,
                created_commit: None,
                implements: None,
                change_branch: None,
                comments: vec![crate::model::ReviewComment {
                    anchor_state: None,
                    intended_path: None,
                    id: 1,
                    doc: None,
                    status: crate::model::ReviewCommentStatus::Open,
                    applied_commit: None,
                    anchor: None,
                    body: "Comment mentions wombat explicitly.".to_string(),
                    reply: None,
                }],
            },
            body: "Summary mentions aardvark explicitly.".to_string(),
        };
        crate::io::write_review(store, "acme", &id, &doc).unwrap();
        id
    }

    #[test]
    fn a_change_reviews_summary_and_comments_are_searchable() {
        let mut store = setup_test_store();
        let id = add_change_review(&mut store);
        let filter = SearchFilter {
            kinds: vec![ItemKind::Review],
            ..Default::default()
        };
        for query in ["aardvark", "wombat"] {
            let results = search(&store, query, &filter).unwrap();
            assert!(
                results.iter().any(|r| r.identifier == id),
                "expected the change review in results for {query:?}: {results:?}"
            );
        }
        // The title names the change target like any other kind.
        let results = search(&store, "aardvark", &filter).unwrap();
        assert!(
            results[0].title.starts_with("review of change/"),
            "unexpected title: {}",
            results[0].title
        );
    }

    #[test]
    fn search_matches_review_summary_body() {
        let mut store = setup_test_store();
        let id = add_review(&mut store);
        let filter = SearchFilter {
            kinds: vec![ItemKind::Review],
            ..Default::default()
        };
        let results = search(&store, "zanzibar", &filter).unwrap();
        assert_eq!(results.len(), 1, "expected one review hit: {results:?}");
        assert_eq!(results[0].kind, ItemKind::Review);
        assert_eq!(results[0].identifier, id);
        assert_eq!(results[0].title, "review of task/fix-login-bug");
    }

    #[test]
    fn search_matches_review_comment_body() {
        let mut store = setup_test_store();
        let id = add_review(&mut store);
        let filter = SearchFilter {
            kinds: vec![ItemKind::Review],
            ..Default::default()
        };
        let results = search(&store, "quokka", &filter).unwrap();
        assert_eq!(
            results.len(),
            1,
            "comment bodies must be searchable: {results:?}"
        );
        assert_eq!(results[0].identifier, id);
    }

    #[test]
    fn search_review_kind_filter_excludes_other_kinds() {
        let mut store = setup_test_store();
        add_review(&mut store);
        let filter = SearchFilter {
            kinds: vec![ItemKind::Review],
            min_score_ratio: Some(0.0),
            ..Default::default()
        };
        // "widget" matches roadmap/phase content but no review text.
        let results = search(&store, "widget", &filter).unwrap();
        assert!(
            results.iter().all(|r| r.kind == ItemKind::Review),
            "kind filter must keep only reviews: {results:?}"
        );
    }

    #[test]
    fn search_reviews_appear_in_unfiltered_results() {
        let mut store = setup_test_store();
        add_review(&mut store);
        let results = search(&store, "zanzibar", &SearchFilter::default()).unwrap();
        assert!(
            results.iter().any(|r| r.kind == ItemKind::Review),
            "reviews must be indexed without a kind filter too: {results:?}"
        );
    }

    #[test]
    fn search_reviews_skipped_when_status_or_tag_filter_active() {
        let mut store = setup_test_store();
        add_review(&mut store);
        // A status filter uses phase/task vocabulary — reviews are skipped.
        let filter = SearchFilter {
            status: Some(ItemStatus::Task(TaskStatus::Open)),
            min_score_ratio: Some(0.0),
            ..Default::default()
        };
        let results = search(&store, "zanzibar", &filter).unwrap();
        assert!(
            results.iter().all(|r| r.kind != ItemKind::Review),
            "status filter must skip reviews: {results:?}"
        );
        // Same for a tag filter — reviews carry no tags.
        let filter = SearchFilter {
            tags: Some(vec!["bug".to_string()]),
            min_score_ratio: Some(0.0),
            ..Default::default()
        };
        let results = search(&store, "zanzibar", &filter).unwrap();
        assert!(
            results.iter().all(|r| r.kind != ItemKind::Review),
            "tag filter must skip reviews: {results:?}"
        );
    }

    #[test]
    fn multi_kind_filter_drops_reviews_before_any_limit() {
        let mut store = setup_test_store();
        add_review(&mut store);
        crate::ops::task::create_task(
            &mut store,
            crate::ops::task::CreateTask {
                project: "acme",
                slug: "quokka-habitat",
                title: "Quokka habitat",
                ..Default::default()
            },
        )
        .unwrap();
        let unfiltered = search(&store, "quokka", &SearchFilter::default()).unwrap();
        assert!(
            unfiltered.iter().any(|r| r.kind == ItemKind::Review),
            "precondition: the review matches the query: {unfiltered:?}"
        );

        let filter = SearchFilter {
            kinds: vec![ItemKind::Roadmap, ItemKind::Phase, ItemKind::Task],
            ..Default::default()
        };
        let results = search(&store, "quokka", &filter).unwrap();
        assert!(
            results.iter().all(|r| r.kind != ItemKind::Review),
            "a review is never a hit of roadmap,phase,task: {results:?}"
        );
        let first = results.first().expect("the task still matches");
        assert_eq!(first.identifier, "quokka-habitat");
    }

    #[test]
    fn parse_kinds_splits_trims_and_dedupes() {
        assert_eq!(
            parse_kinds(" roadmap ,phase,task,phase").unwrap(),
            vec![ItemKind::Roadmap, ItemKind::Phase, ItemKind::Task]
        );
        assert_eq!(parse_kinds("").unwrap(), Vec::<ItemKind>::new());
        assert!(parse_kinds("phase,bogus").is_err());
    }

    #[test]
    fn parse_for_kinds_reads_the_status_vocabulary_of_the_kind_set() {
        use ItemKind::*;
        // Every kind, or phases and tasks together: kind-agnostic.
        assert_eq!(
            ItemStatus::parse_for_kinds("done", &[]).unwrap(),
            ItemStatus::Either(PhaseStatus::Done, TaskStatus::Done)
        );
        assert_eq!(
            ItemStatus::parse_for_kinds("done", &[Phase, Task, Roadmap]).unwrap(),
            ItemStatus::Either(PhaseStatus::Done, TaskStatus::Done)
        );
        // One of the two: that kind's vocabulary.
        assert_eq!(
            ItemStatus::parse_for_kinds("done", &[Phase, Roadmap]).unwrap(),
            ItemStatus::Phase(PhaseStatus::Done)
        );
        assert_eq!(
            ItemStatus::parse_for_kinds("done", &[Task]).unwrap(),
            ItemStatus::Task(TaskStatus::Done)
        );
        assert!(matches!(
            ItemStatus::parse_for_kinds("open", &[Phase]),
            Err(StatusFilterError::Invalid(_))
        ));
        // Neither: refused with the kind's own message.
        let err = ItemStatus::parse_for_kinds("done", &[Roadmap]).unwrap_err();
        assert!(err.to_string().starts_with("roadmaps do not have a status"));
        let err = ItemStatus::parse_for_kinds("done", &[Review]).unwrap_err();
        assert!(err.to_string().contains("rdm review list --state"));
        let err = ItemStatus::parse_for_kinds("done", &[Roadmap, Review]).unwrap_err();
        assert!(matches!(err, StatusFilterError::NoStatus { .. }));
    }

    #[test]
    fn hits_carry_status_and_structured_item() {
        let mut store = setup_test_store();
        add_review(&mut store);
        let filter = SearchFilter {
            min_score_ratio: Some(0.0),
            ..Default::default()
        };
        let results = search(&store, "widget", &filter).unwrap();
        let phase = results
            .iter()
            .find(|r| r.identifier == "widget-launch/phase-1-design")
            .expect("phase hit");
        assert_eq!(phase.status.as_deref(), Some("done"));
        assert_eq!(
            phase.item,
            Some(ItemRef::Phase {
                roadmap: "widget-launch".to_string(),
                stem: "phase-1-design".to_string(),
            })
        );
        let roadmap = results
            .iter()
            .find(|r| r.kind == ItemKind::Roadmap)
            .expect("roadmap hit");
        assert_eq!(roadmap.status, None);
        assert_eq!(
            roadmap.item,
            Some(ItemRef::Roadmap {
                roadmap: "widget-launch".to_string(),
            })
        );

        let results = search(&store, "Fix Login Bug", &filter).unwrap();
        let task = results
            .iter()
            .find(|r| r.kind == ItemKind::Task)
            .expect("task hit");
        assert_eq!(task.status.as_deref(), Some("done"));
        assert_eq!(
            task.item,
            Some(ItemRef::Task {
                slug: "fix-login-bug".to_string(),
            })
        );

        let results = search(&store, "zanzibar", &filter).unwrap();
        let review = results
            .iter()
            .find(|r| r.kind == ItemKind::Review)
            .expect("review hit");
        assert_eq!((review.status.as_ref(), review.item.as_ref()), (None, None));
    }
}
