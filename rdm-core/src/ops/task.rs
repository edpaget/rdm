//! Task operations.

use chrono::Local;

use crate::document::Document;
use crate::error::{Error, Result};
use crate::model::{Phase, PhaseStatus, Priority, Roadmap, Task, TaskStatus, TaskStatusFilter};
use crate::ops::update::{BodyUpdate, TagsUpdate};
use crate::store::{DirEntryKind, Store};

/// Criteria for filtering a list of tasks.
///
/// Each field narrows the result set independently; a task is kept only if it
/// satisfies all populated criteria. The default value (all fields empty) keeps
/// the "active work" set — open or in-progress tasks of any priority and tags.
#[derive(Debug, Clone, Default)]
pub struct TaskFilter {
    /// Status criterion. `None` keeps open **or** in-progress tasks (the
    /// "active work" default); `Some(All)` keeps any status; `Some(Status(s))`
    /// keeps only tasks with exactly status `s`.
    pub status: Option<TaskStatusFilter>,
    /// Priority criterion. `None` keeps tasks of any priority; `Some(p)` keeps
    /// only tasks with exactly priority `p`.
    pub priority: Option<Priority>,
    /// Tag criterion. Empty keeps tasks regardless of tags; otherwise a task is
    /// kept only if it carries **all** of these tags (logical AND).
    pub tags: Vec<String>,
}

/// Returns whether `task` satisfies every populated criterion in `filter`.
///
/// Status semantics match the CLI's `task list`: `filter.status` of `None`
/// keeps open or in-progress tasks, `Some(All)` keeps any status, and
/// `Some(Status(s))` keeps an exact match. Priority of `None` matches any.
/// Tags are matched as a logical AND — every tag in `filter.tags` must be
/// present on the task (an empty list imposes no tag constraint).
pub fn task_matches(task: &Task, filter: &TaskFilter) -> bool {
    let status_ok = match filter.status {
        Some(TaskStatusFilter::All) => true,
        Some(TaskStatusFilter::Status(s)) => task.status == s,
        None => task.status == TaskStatus::Open || task.status == TaskStatus::InProgress,
    };
    let priority_ok = filter.priority.is_none_or(|p| task.priority == p);
    let tags_ok = filter
        .tags
        .iter()
        .all(|t| task.tags.as_ref().is_some_and(|tags| tags.contains(t)));
    status_ok && priority_ok && tags_ok
}

/// Filters `tasks` to those satisfying `filter`, preserving order.
///
/// An owned convenience wrapper over [`task_matches`] for callers that hold a
/// `Vec` of `(slug, Document<Task>)` pairs (e.g. [`list_tasks`] output).
pub fn filter_tasks(
    tasks: Vec<(String, Document<Task>)>,
    filter: &TaskFilter,
) -> Vec<(String, Document<Task>)> {
    tasks
        .into_iter()
        .filter(|(_, doc)| task_matches(&doc.frontmatter, filter))
        .collect()
}

/// Request describing a new task to create.
///
/// Only `project`, `slug`, and `title` are required for a minimal task; the
/// remaining fields default via [`Default`]. The default `priority` is
/// [`Priority::Medium`], matching the conventional mid-priority for new work, so
/// callers can write `CreateTask { project, slug, title, ..Default::default() }`.
#[derive(Debug, Clone)]
pub struct CreateTask<'a> {
    /// Project the task belongs to.
    pub project: &'a str,
    /// Slug (file name) for the new task.
    pub slug: &'a str,
    /// Human-readable title.
    pub title: &'a str,
    /// Priority level. Required; defaults to [`Priority::Medium`].
    pub priority: Priority,
    /// Optional tags for categorization.
    pub tags: Option<Vec<String>>,
    /// Markdown body below the frontmatter. `None` yields an empty body.
    pub body: Option<&'a str>,
}

impl Default for CreateTask<'_> {
    fn default() -> Self {
        CreateTask {
            project: "",
            slug: "",
            title: "",
            priority: Priority::Medium,
            tags: None,
            body: None,
        }
    }
}

/// Creates a new task within a project.
///
/// See [`CreateTask`] for the field semantics: `body` of `None` yields an empty
/// body and `priority` defaults to [`Priority::Medium`].
///
/// # Errors
///
/// Returns [`Error::ProjectNotFound`] if the project doesn't exist,
/// [`Error::DuplicateSlug`] if a task with the same slug already exists,
/// [`Error::Io`] if file creation fails, or
/// [`Error::FrontmatterParse`] if frontmatter serialization fails.
pub fn create_task(store: &mut impl Store, req: CreateTask<'_>) -> Result<Document<Task>> {
    let CreateTask {
        project,
        slug,
        title,
        priority,
        tags,
        body,
    } = req;
    if !store.exists(&crate::paths::project_md_path(project)) {
        return Err(Error::ProjectNotFound(project.to_string()));
    }
    let path = crate::paths::task_path(project, slug);
    if store.exists(&path) {
        return Err(Error::DuplicateSlug(slug.to_string()));
    }

    let doc = Document {
        frontmatter: Task {
            project: project.to_string(),
            title: title.to_string(),
            status: TaskStatus::Open,
            priority,
            created: Local::now().date_naive(),
            tags,
            completed: None,
            commit: None,
            review_sha: None,
            review_branch: None,
        },
        body: body.unwrap_or_default().to_string(),
    };
    crate::io::write_task(store, project, slug, &doc)?;
    Ok(doc)
}

/// Lists all tasks for a project, sorted by slug.
///
/// Returns `(slug, Document<Task>)` tuples. Returns an empty vec if the
/// tasks directory doesn't exist.
///
/// # Errors
///
/// Returns [`Error::Io`] if the directory cannot be read, or
/// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] if a
/// task file has invalid frontmatter.
pub fn list_tasks(store: &impl Store, project: &str) -> Result<Vec<(String, Document<Task>)>> {
    if !store.exists(&crate::paths::project_md_path(project)) {
        return Err(Error::ProjectNotFound(project.to_string()));
    }
    let dir = crate::paths::tasks_dir(project);
    let entries = store.list(&dir)?;

    let mut tasks: Vec<(String, Document<Task>)> = Vec::new();
    for entry in entries {
        if entry.kind != DirEntryKind::File {
            continue;
        }
        if !entry.name.ends_with(".md") {
            continue;
        }
        let slug = entry.name.trim_end_matches(".md").to_string();
        let doc = crate::io::load_task(store, project, &slug)?;
        tasks.push((slug, doc));
    }
    tasks.sort_by(|(a, _), (b, _)| a.cmp(b));
    Ok(tasks)
}

/// Updates a task's status, priority, tags, and/or body.
///
/// `status`/`priority` of `None` and `tags`/`body` of `Keep` leave those
/// fields unchanged; otherwise see [`TagsUpdate`] and [`BodyUpdate`]. A task's
/// priority is required, so it is set-or-keep (`Option<Priority>`) rather than
/// clearable.
///
/// Setting a terminal status ([`TaskStatus::is_terminal`], i.e. `Done` or
/// `WontFix`) stamps `completed` with today's date and stores the optional
/// `commit` SHA. Re-setting the same terminal status preserves the existing
/// `completed` date and only updates `commit` if a new value is provided.
/// Transitioning to a non-terminal status clears both `completed` and `commit`.
///
/// The `review_sha` parameter stamps the source-repo HEAD SHA that produced
/// the item. When `status` transitions to [`TaskStatus::NeedsReview`], the
/// provided `review_sha` is stored on the task; any other status change clears
/// it to `None`; when `status` is `None`, the existing `review_sha` is
/// preserved.
/// The `review_branch` parameter mirrors `review_sha`: the branch name of the
/// checkout that produced the review, stamped and cleared in lockstep so
/// `review pending` can scope by branch identity.
/// Re-applying [`TaskStatus::NeedsReview`] while the task is already in that
/// status re-stamps `review_sha`/`review_branch` to the newly provided values
/// (rather than preserving the existing ones the way `status: None` does) —
/// this is the refresh path `rdm review restamp` uses to keep a stamp from
/// going stale after a commit is amended or rebased mid-review.
///
/// # Errors
///
/// Returns [`Error::TaskNotFound`] if the task file doesn't exist,
/// [`Error::BodyClobberRefused`] if `body` is [`BodyUpdate::Set("")`](BodyUpdate::Set)
/// over a non-empty body (use [`BodyUpdate::Clear`] to confirm),
/// [`Error::Io`] if reading or writing fails, or
/// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] if the
/// existing task file has invalid frontmatter.
#[allow(clippy::too_many_arguments)]
pub fn update_task(
    store: &mut impl Store,
    project: &str,
    slug: &str,
    status: Option<TaskStatus>,
    priority: Option<Priority>,
    tags: TagsUpdate,
    body: BodyUpdate,
    commit: Option<String>,
    review_sha: Option<String>,
    review_branch: Option<String>,
) -> Result<Document<Task>> {
    let path = crate::paths::task_path(project, slug);
    if !store.exists(&path) {
        return Err(Error::TaskNotFound(slug.to_string()));
    }

    let mut doc = crate::io::load_task(store, project, slug)?;
    if let Some(status) = status {
        if status.is_terminal() && doc.frontmatter.status == status {
            // Already at this terminal state: only update commit if a new one
            // is provided, preserving the existing completed date.
            if let Some(sha) = commit {
                doc.frontmatter.commit = Some(sha);
            }
        } else {
            doc.frontmatter.status = status;
            if status.is_terminal() {
                doc.frontmatter.completed = Some(Local::now().date_naive());
                doc.frontmatter.commit = commit;
            } else {
                doc.frontmatter.completed = None;
                doc.frontmatter.commit = None;
            }
            // Stamp the source-repo SHA on entry to needs-review; clear it on
            // any other transition so a stale discriminator never lingers.
            if status == TaskStatus::NeedsReview {
                doc.frontmatter.review_sha = review_sha;
                doc.frontmatter.review_branch = review_branch;
            } else {
                doc.frontmatter.review_sha = None;
                doc.frontmatter.review_branch = None;
            }
        }
    }
    if let Some(p) = priority {
        doc.frontmatter.priority = p;
    }
    tags.apply(&mut doc.frontmatter.tags);
    body.apply(&mut doc.body)?;
    crate::io::write_task(store, project, slug, &doc)?;
    Ok(doc)
}

/// Promotes a task to a roadmap.
///
/// Creates a new roadmap directory, writes `roadmap.md` from task metadata,
/// creates `phase-1-*.md` from the task body, and removes the original task file.
///
/// # Errors
///
/// Returns [`Error::TaskNotFound`] if the task doesn't exist,
/// [`Error::DuplicateSlug`] if the roadmap already exists,
/// [`Error::Io`] if file operations fail, or
/// [`Error::FrontmatterParse`] if frontmatter serialization fails.
pub fn promote_task(
    store: &mut impl Store,
    project: &str,
    task_slug: &str,
    roadmap_slug: &str,
) -> Result<Document<Roadmap>> {
    let task_path = crate::paths::task_path(project, task_slug);
    if !store.exists(&task_path) {
        return Err(Error::TaskNotFound(task_slug.to_string()));
    }

    let task_doc = crate::io::load_task(store, project, task_slug)?;

    let roadmap_file = crate::paths::roadmap_path(project, roadmap_slug);
    if store.exists(&roadmap_file) {
        return Err(Error::DuplicateSlug(roadmap_slug.to_string()));
    }

    let phase_slug = crate::model::phase_stem(1, task_slug);

    let mut roadmap_body = String::new();
    roadmap_body.push_str(&format!(
        "Promoted from task `{task_slug}` (priority: {}, created: {})",
        task_doc.frontmatter.priority, task_doc.frontmatter.created
    ));
    if let Some(ref tags) = task_doc.frontmatter.tags {
        roadmap_body.push_str(&format!(", tags: {}", tags.join(", ")));
    }
    roadmap_body.push('\n');

    let roadmap_doc = Document {
        frontmatter: Roadmap {
            project: project.to_string(),
            roadmap: roadmap_slug.to_string(),
            title: task_doc.frontmatter.title.clone(),
            phases: vec![phase_slug.clone()],
            dependencies: None,
            priority: None,
            tags: None,
        },
        body: roadmap_body,
    };
    crate::io::write_roadmap(store, project, roadmap_slug, &roadmap_doc)?;

    let phase_doc = Document {
        frontmatter: Phase {
            phase: 1,
            title: task_doc.frontmatter.title,
            status: PhaseStatus::NotStarted,
            tags: task_doc.frontmatter.tags.clone(),
            completed: None,
            commit: None,
            review_sha: None,
            review_branch: None,
            difficulty: None,
            model: None,
            blocked_reason: None,
        },
        body: task_doc.body,
    };
    crate::io::write_phase(store, project, roadmap_slug, &phase_slug, &phase_doc)?;

    store.delete(&task_path)?;

    Ok(roadmap_doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn task(status: TaskStatus, priority: Priority, tags: &[&str]) -> Task {
        Task {
            project: "demo".to_string(),
            title: "A task".to_string(),
            status,
            priority,
            created: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            tags: if tags.is_empty() {
                None
            } else {
                Some(tags.iter().map(|s| s.to_string()).collect())
            },
            completed: None,
            commit: None,
            review_sha: None,
            review_branch: None,
        }
    }

    #[test]
    fn default_filter_keeps_open_and_in_progress() {
        let filter = TaskFilter::default();
        assert!(task_matches(
            &task(TaskStatus::Open, Priority::Medium, &[]),
            &filter
        ));
        assert!(task_matches(
            &task(TaskStatus::InProgress, Priority::Medium, &[]),
            &filter
        ));
        assert!(!task_matches(
            &task(TaskStatus::Done, Priority::Medium, &[]),
            &filter
        ));
        assert!(!task_matches(
            &task(TaskStatus::WontFix, Priority::Medium, &[]),
            &filter
        ));
    }

    #[test]
    fn all_status_keeps_every_status() {
        let filter = TaskFilter {
            status: Some(TaskStatusFilter::All),
            ..Default::default()
        };
        for status in [
            TaskStatus::Open,
            TaskStatus::InProgress,
            TaskStatus::NeedsReview,
            TaskStatus::Reviewed,
            TaskStatus::Done,
            TaskStatus::WontFix,
        ] {
            assert!(task_matches(&task(status, Priority::Low, &[]), &filter));
        }
    }

    #[test]
    fn exact_status_matches_only_that_status() {
        let filter = TaskFilter {
            status: Some(TaskStatusFilter::Status(TaskStatus::Done)),
            ..Default::default()
        };
        assert!(task_matches(
            &task(TaskStatus::Done, Priority::Low, &[]),
            &filter
        ));
        assert!(!task_matches(
            &task(TaskStatus::Open, Priority::Low, &[]),
            &filter
        ));
    }

    #[test]
    fn priority_filter_matches_exactly() {
        let filter = TaskFilter {
            status: Some(TaskStatusFilter::All),
            priority: Some(Priority::High),
            ..Default::default()
        };
        assert!(task_matches(
            &task(TaskStatus::Open, Priority::High, &[]),
            &filter
        ));
        assert!(!task_matches(
            &task(TaskStatus::Open, Priority::Low, &[]),
            &filter
        ));
    }

    #[test]
    fn single_tag_filter() {
        let filter = TaskFilter {
            status: Some(TaskStatusFilter::All),
            tags: vec!["bug".to_string()],
            ..Default::default()
        };
        assert!(task_matches(
            &task(TaskStatus::Open, Priority::Low, &["bug", "ui"]),
            &filter
        ));
        assert!(!task_matches(
            &task(TaskStatus::Open, Priority::Low, &["ui"]),
            &filter
        ));
        assert!(!task_matches(
            &task(TaskStatus::Open, Priority::Low, &[]),
            &filter
        ));
    }

    #[test]
    fn multi_tag_filter_is_and() {
        let filter = TaskFilter {
            status: Some(TaskStatusFilter::All),
            tags: vec!["bug".to_string(), "ui".to_string()],
            ..Default::default()
        };
        assert!(task_matches(
            &task(TaskStatus::Open, Priority::Low, &["bug", "ui", "extra"]),
            &filter
        ));
        // Missing one of the required tags fails the AND.
        assert!(!task_matches(
            &task(TaskStatus::Open, Priority::Low, &["bug"]),
            &filter
        ));
    }

    #[test]
    fn empty_tag_filter_imposes_no_constraint() {
        let filter = TaskFilter {
            status: Some(TaskStatusFilter::All),
            ..Default::default()
        };
        assert!(task_matches(
            &task(TaskStatus::Open, Priority::Low, &[]),
            &filter
        ));
    }

    #[test]
    fn filter_tasks_keeps_matching_in_order() {
        let tasks = vec![
            (
                "a".to_string(),
                Document {
                    frontmatter: task(TaskStatus::Open, Priority::Low, &[]),
                    body: String::new(),
                },
            ),
            (
                "b".to_string(),
                Document {
                    frontmatter: task(TaskStatus::Done, Priority::Low, &[]),
                    body: String::new(),
                },
            ),
            (
                "c".to_string(),
                Document {
                    frontmatter: task(TaskStatus::InProgress, Priority::Low, &[]),
                    body: String::new(),
                },
            ),
        ];
        let kept = filter_tasks(tasks, &TaskFilter::default());
        let slugs: Vec<&str> = kept.iter().map(|(s, _)| s.as_str()).collect();
        assert_eq!(slugs, vec!["a", "c"]);
    }
}
