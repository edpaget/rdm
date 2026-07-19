//! Task operations.

use chrono::Local;

use crate::document::Document;
use crate::error::{Error, Result};
use crate::model::{Phase, PhaseStatus, Priority, Roadmap, Task, TaskStatus, TaskStatusFilter};
use crate::ops::update::{BodyUpdate, ReasonUpdate, TagsUpdate, TitleUpdate};
use crate::store::{DirEntryKind, Store};

/// Criteria for filtering a list of tasks.
///
/// Each field narrows the result set independently; a task is kept only if it
/// satisfies all populated criteria. The default value (all fields empty) keeps
/// the "active work" set — every non-terminal task (open, in-progress,
/// needs-review, reviewed, or blocked) of any priority and tags.
#[derive(Debug, Clone, Default)]
pub struct TaskFilter {
    /// Status criterion. `None` keeps active tasks — every non-terminal task
    /// (open, in-progress, needs-review, reviewed, or blocked; the "active work"
    /// default); `Some(All)` keeps any status; `Some(Status(s))` keeps only
    /// tasks with exactly status `s`.
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
/// keeps active tasks (every non-terminal task — open, in-progress, needs-review,
/// reviewed, or blocked), `Some(All)` keeps any status, and `Some(Status(s))`
/// keeps an exact match. Priority of
/// `None` matches any. Tags are matched as a logical AND — every tag in
/// `filter.tags` must be present on the task (an empty list imposes no tag constraint).
pub fn task_matches(task: &Task, filter: &TaskFilter) -> bool {
    let status_ok = match filter.status {
        Some(TaskStatusFilter::All) => true,
        Some(TaskStatusFilter::Status(s)) => task.status == s,
        None => !task.status.is_terminal(),
    };
    let priority_ok = filter.priority.is_none_or(|p| task.priority == p);
    let tags_ok = crate::tags::matches_all_tags(task.tags.as_deref(), &filter.tags);
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
            close_reason: None,
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
/// A [`TitleUpdate::Set`] renames the task in place — the `slug` that
/// identifies it is never changed; [`TitleUpdate::Keep`] leaves it unchanged.
///
/// # Errors
///
/// Returns [`Error::TaskNotFound`] if the task file doesn't exist,
/// [`Error::EmptyTitle`] if `title` is [`TitleUpdate::Set`] with an empty or
/// whitespace-only value,
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
    title: TitleUpdate,
) -> Result<Document<Task>> {
    let path = crate::paths::task_path(project, slug);
    if !store.exists(&path) {
        return Err(Error::TaskNotFound(slug.to_string()));
    }

    let mut doc = crate::io::load_task(store, project, slug)?;
    title.apply(&mut doc.frontmatter.title)?;
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

/// Consolidates a task into an existing roadmap as a new trailing phase.
///
/// Unlike [`promote_task`] (which creates a brand-new 1:1 roadmap), this folds
/// a task into an already-existing roadmap: a new phase is appended (auto-
/// numbered after the roadmap's current last phase) carrying the task's title,
/// tags, and body — prefixed with a provenance line naming the source task —
/// via [`super::phase::create_phase`]. The source task is **not** deleted;
/// instead it is marked [`TaskStatus::Done`] and its body gets a pointer note
/// naming the roadmap and the new phase stem, so it reads as "folded" rather
/// than abandoned and drops out of the active `task list`.
///
/// `body_override`, when `Some`, replaces the task's own body as the phase
/// content (still prefixed with the provenance line); when `None`, the task's
/// existing body is used.
///
/// A task can only be consolidated once: if it is already in a terminal
/// status ([`TaskStatus::is_terminal`]), this returns
/// [`Error::TaskAlreadyConsolidated`] before any phase is created or the task
/// is touched, so a rejected call never leaves partial mutation behind.
///
/// # Errors
///
/// Returns [`Error::TaskNotFound`] if the task doesn't exist,
/// [`Error::TaskAlreadyConsolidated`] if the task is already `Done` or
/// `WontFix`,
/// [`Error::RoadmapNotFound`] if the target roadmap doesn't exist,
/// [`Error::DuplicateSlug`] if a phase with the task's slug already exists in
/// the roadmap,
/// [`Error::Io`] if file operations fail, or
/// [`Error::FrontmatterParse`] if frontmatter serialization fails.
pub fn consolidate_task_into_roadmap(
    store: &mut impl Store,
    project: &str,
    task_slug: &str,
    roadmap_slug: &str,
    body_override: Option<&str>,
) -> Result<(Document<Phase>, Document<Task>)> {
    let task_path = crate::paths::task_path(project, task_slug);
    if !store.exists(&task_path) {
        return Err(Error::TaskNotFound(task_slug.to_string()));
    }

    let task_doc = crate::io::load_task(store, project, task_slug)?;
    if task_doc.frontmatter.status.is_terminal() {
        return Err(Error::TaskAlreadyConsolidated(task_slug.to_string()));
    }

    let source_body = body_override.unwrap_or(&task_doc.body);
    let phase_body = format!("Consolidated from task `{task_slug}`.\n\n{source_body}");

    let phase_doc = super::phase::create_phase(
        store,
        super::phase::CreatePhase {
            project,
            roadmap: roadmap_slug,
            slug: task_slug,
            title: &task_doc.frontmatter.title,
            body: Some(&phase_body),
            tags: task_doc.frontmatter.tags.clone(),
            ..Default::default()
        },
    )?;

    let phase_stem = crate::model::phase_stem(phase_doc.frontmatter.phase, task_slug);
    let pointer_body = format!(
        "{}\n\nConsolidated into roadmap `{roadmap_slug}` as `{phase_stem}`.",
        task_doc.body
    );

    let updated_task = update_task(
        store,
        project,
        task_slug,
        Some(TaskStatus::Done),
        None,
        TagsUpdate::Keep,
        BodyUpdate::Set(pointer_body),
        None,
        None,
        None,
        TitleUpdate::Keep,
    )?;

    Ok((phase_doc, updated_task))
}

/// Sets (or clears) a task's close reason — the retire/supersede note recorded
/// when a task is closed.
///
/// Kept separate from [`update_task`] (like [`set_phase_blocked_reason`]) so the
/// status/priority/tags/body signature stays untouched. The reason follows the
/// keep/set/clear protocol via [`ReasonUpdate`]: it is preserved across status
/// transitions unless explicitly cleared, so reopening a retired task never
/// loses the record of why it was closed. This function does not itself change
/// the task status — callers set the status via [`update_task`] and record the
/// reason here in the same mutation.
///
/// [`set_phase_blocked_reason`]: super::phase::set_phase_blocked_reason
///
/// # Errors
///
/// Returns [`Error::TaskNotFound`] if the task file doesn't exist,
/// [`Error::Io`] if reading or writing fails, or
/// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] if the existing
/// task file has invalid frontmatter.
pub fn set_task_close_reason(
    store: &mut impl Store,
    project: &str,
    slug: &str,
    reason: ReasonUpdate,
) -> Result<Document<Task>> {
    let path = crate::paths::task_path(project, slug);
    if !store.exists(&path) {
        return Err(Error::TaskNotFound(slug.to_string()));
    }

    let mut doc = crate::io::load_task(store, project, slug)?;
    reason.apply(&mut doc.frontmatter.close_reason);
    crate::io::write_task(store, project, slug, &doc)?;
    Ok(doc)
}

/// Merges one or more source tasks into a survivor task, then retires the
/// sources with a superseded-by pointer.
///
/// This folds duplicate work items into a single canonical task: the survivor
/// unions in each source's tags, gains an attributed copy of each source body
/// under a `## Merged from task <slug>` heading (in `--from` order), and each
/// source is closed [`TaskStatus::WontFix`] with its `close_reason` set to the
/// machine string `superseded by task/<survivor>` and a
/// `Superseded by task <survivor>.` note appended to its body.
///
/// The operation validates before mutating (like
/// [`consolidate_task_into_roadmap`]): sources are de-duplicated preserving
/// first-seen order, and the survivor plus every source must exist before any
/// write happens, so a rejected call never leaves partial mutation behind.
///
/// It is idempotent. A source that is already terminal **and** already carries
/// `close_reason == "superseded by task/<survivor>"` is skipped entirely — no
/// tag re-union, no survivor body re-append, no re-close — so re-running the
/// same merge is a no-op. A source superseded by a *different* survivor is not
/// skipped: it is merged and re-closed with the new pointer.
///
/// Returns the updated survivor document and the closed source documents (only
/// those actually folded this call, in encounter order).
///
/// # Errors
///
/// Returns [`Error::TaskMergeNoSources`] if `sources` is empty (after dedupe),
/// [`Error::TaskMergeIntoSelf`] if `survivor` appears among `sources`,
/// [`Error::TaskNotFound`] if the survivor or any source doesn't exist,
/// [`Error::Io`] if reading or writing fails, or
/// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] if a task file has
/// invalid frontmatter.
pub fn merge_tasks(
    store: &mut impl Store,
    project: &str,
    survivor: &str,
    sources: &[String],
) -> Result<(Document<Task>, Vec<Document<Task>>)> {
    // 1. Dedupe sources, preserving first-seen order.
    let mut deduped: Vec<String> = Vec::new();
    for s in sources {
        if !deduped.iter().any(|d| d == s) {
            deduped.push(s.clone());
        }
    }

    // 2. Empty → error.
    if deduped.is_empty() {
        return Err(Error::TaskMergeNoSources);
    }

    // 3. Self-merge → error.
    if deduped.iter().any(|s| s == survivor) {
        return Err(Error::TaskMergeIntoSelf(survivor.to_string()));
    }

    // 4. Pre-flight existence: survivor first, then every source. No writes
    //    until all pass.
    if !store.exists(&crate::paths::task_path(project, survivor)) {
        return Err(Error::TaskNotFound(survivor.to_string()));
    }
    for s in &deduped {
        if !store.exists(&crate::paths::task_path(project, s)) {
            return Err(Error::TaskNotFound(s.clone()));
        }
    }

    // 5. Load survivor; seed the tag and body accumulators from it.
    let survivor_doc = crate::io::load_task(store, project, survivor)?;
    let mut tags: Vec<String> = survivor_doc.frontmatter.tags.clone().unwrap_or_default();
    let mut body = survivor_doc.body.clone();

    // The machine string that marks a source as already folded into *this*
    // survivor — both the close_reason value and the idempotency key.
    let superseded_reason = format!("superseded by task/{survivor}");

    let mut closed: Vec<Document<Task>> = Vec::new();

    // 6. Fold each source in encounter order.
    for source in &deduped {
        let source_doc = crate::io::load_task(store, project, source)?;

        // Idempotency: a source already terminal and already superseded by this
        // exact survivor is skipped entirely.
        if source_doc.frontmatter.status.is_terminal()
            && source_doc.frontmatter.close_reason.as_deref() == Some(superseded_reason.as_str())
        {
            continue;
        }

        // Union the source's tags (append not-already-present, preserve order).
        if let Some(source_tags) = &source_doc.frontmatter.tags {
            for t in source_tags {
                if !tags.iter().any(|existing| existing == t) {
                    tags.push(t.clone());
                }
            }
        }

        // Append the attributed source body under a heading.
        if source_doc.body.trim().is_empty() {
            body.push_str(&format!("\n\n## Merged from task `{source}`"));
        } else {
            body.push_str(&format!(
                "\n\n## Merged from task `{source}`\n\n{}",
                source_doc.body
            ));
        }

        // Close the source: WontFix, keep its tags, append the superseded note
        // to its body, then stamp the close_reason pointer.
        let source_pointer_body =
            format!("{}\n\nSuperseded by task `{survivor}`.", source_doc.body);
        update_task(
            store,
            project,
            source,
            Some(TaskStatus::WontFix),
            None,
            TagsUpdate::Keep,
            BodyUpdate::Set(source_pointer_body),
            None,
            None,
            None,
            TitleUpdate::Keep,
        )?;
        let closed_doc = set_task_close_reason(
            store,
            project,
            source,
            ReasonUpdate::Set(superseded_reason.clone()),
        )?;
        closed.push(closed_doc);
    }

    // 7. Write the survivor once with the unioned tags and accumulated body.
    let survivor_updated = update_task(
        store,
        project,
        survivor,
        None,
        None,
        TagsUpdate::Set(tags),
        BodyUpdate::Set(body),
        None,
        None,
        None,
        TitleUpdate::Keep,
    )?;

    // 8. Return survivor + the sources folded this call.
    Ok((survivor_updated, closed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemoryStore;
    use chrono::NaiveDate;

    fn setup() -> MemoryStore {
        let mut store = MemoryStore::new();
        crate::ops::init::init(&mut store).unwrap();
        crate::ops::project::create_project(&mut store, "demo", "Demo").unwrap();
        store
    }

    fn seed(store: &mut MemoryStore, slug: &str, tags: &[&str], body: &str) {
        create_task(
            store,
            CreateTask {
                project: "demo",
                slug,
                title: slug,
                priority: Priority::Medium,
                tags: if tags.is_empty() {
                    None
                } else {
                    Some(tags.iter().map(|s| s.to_string()).collect())
                },
                body: Some(body),
            },
        )
        .unwrap();
    }

    #[test]
    fn merge_dedups_overlapping_tags() {
        let mut store = setup();
        seed(&mut store, "survivor", &["a", "b"], "S.");
        seed(&mut store, "dup", &["b", "c"], "D.");
        let (survivor, _) =
            merge_tasks(&mut store, "demo", "survivor", &["dup".to_string()]).unwrap();
        // Union preserves survivor order first, appends only new tags.
        assert_eq!(
            survivor.frontmatter.tags,
            Some(vec!["a".to_string(), "b".to_string(), "c".to_string()])
        );
    }

    #[test]
    fn merge_dedups_repeated_from_slugs() {
        let mut store = setup();
        seed(&mut store, "survivor", &[], "S.");
        seed(&mut store, "dup", &["x"], "D.");
        // The same source named twice folds in exactly once.
        let (survivor, closed) = merge_tasks(
            &mut store,
            "demo",
            "survivor",
            &["dup".to_string(), "dup".to_string()],
        )
        .unwrap();
        assert_eq!(closed.len(), 1);
        assert_eq!(
            survivor.body.matches("## Merged from task `dup`").count(),
            1
        );
    }

    #[test]
    fn set_close_reason_arms() {
        let mut store = setup();
        seed(&mut store, "t", &[], "Body.");

        // Set records a reason.
        let doc = set_task_close_reason(
            &mut store,
            "demo",
            "t",
            ReasonUpdate::Set("retired".to_string()),
        )
        .unwrap();
        assert_eq!(doc.frontmatter.close_reason.as_deref(), Some("retired"));

        // Keep leaves it untouched.
        let doc = set_task_close_reason(&mut store, "demo", "t", ReasonUpdate::Keep).unwrap();
        assert_eq!(doc.frontmatter.close_reason.as_deref(), Some("retired"));

        // Clear drops it.
        let doc = set_task_close_reason(&mut store, "demo", "t", ReasonUpdate::Clear).unwrap();
        assert_eq!(doc.frontmatter.close_reason, None);
    }

    #[test]
    fn set_close_reason_unknown_task_errors() {
        let mut store = setup();
        let err =
            set_task_close_reason(&mut store, "demo", "ghost", ReasonUpdate::Clear).unwrap_err();
        assert!(matches!(err, Error::TaskNotFound(ref s) if s == "ghost"));
    }

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
            close_reason: None,
        }
    }

    #[test]
    fn default_filter_keeps_active_tasks() {
        let filter = TaskFilter::default();
        assert!(task_matches(
            &task(TaskStatus::Open, Priority::Medium, &[]),
            &filter
        ));
        assert!(task_matches(
            &task(TaskStatus::InProgress, Priority::Medium, &[]),
            &filter
        ));
        assert!(task_matches(
            &task(TaskStatus::NeedsReview, Priority::Medium, &[]),
            &filter
        ));
        assert!(task_matches(
            &task(TaskStatus::Reviewed, Priority::Medium, &[]),
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
