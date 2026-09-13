//! Implementation-plan operations.

use chrono::Local;

use crate::document::Document;
use crate::error::{Error, Result};
use crate::link::ItemRef;
use crate::model::{Plan, PlanStatus};
use crate::ops::update::{BodyUpdate, TitleUpdate};
use crate::store::{DirEntryKind, Store};

/// Request describing a new implementation plan to create.
///
/// There is no [`Default`] impl: `implements` has no sensible default (a
/// plan that implements nothing is meaningless), so every field is supplied
/// explicitly.
#[derive(Debug, Clone)]
pub struct CreatePlan<'a> {
    /// Project the plan belongs to.
    pub project: &'a str,
    /// Slug (file name) for the new plan.
    pub slug: &'a str,
    /// Human-readable title.
    pub title: &'a str,
    /// The phase or task this plan implements. Must exist at creation time,
    /// and must be an [`ItemRef::Phase`] or [`ItemRef::Task`].
    pub implements: ItemRef,
    /// The earlier plan this one replaces. Must exist at creation time, and
    /// must be an [`ItemRef::Plan`] naming a different plan. Creating the new
    /// plan flips the named one to [`PlanStatus::Superseded`].
    pub supersedes: Option<ItemRef>,
    /// Markdown body below the frontmatter. `None` yields an empty body.
    pub body: Option<&'a str>,
}

/// Criteria for filtering a list of plans.
///
/// Each field narrows the result set independently; a plan is kept only if
/// it satisfies all populated criteria. The default value (all fields empty)
/// keeps every plan.
#[derive(Debug, Clone, Default)]
pub struct PlanFilter {
    /// Implemented-item criterion. `None` keeps plans implementing anything;
    /// `Some(r)` keeps only plans whose `implements` equals `r` after
    /// numeric-phase-stem normalization.
    pub implements: Option<ItemRef>,
    /// Status criterion. `None` keeps plans of any status; `Some(s)` keeps
    /// only plans with exactly status `s`.
    pub status: Option<PlanStatus>,
}

/// Normalizes an [`ItemRef`] for comparison, resolving a numeric phase
/// identifier (`phase/auth/2`) to the roadmap's canonical stem.
///
/// Mirrors [`crate::ops::links::backlinks`]'s own normalization policy so
/// `phase/auth/2` and `phase/auth/phase-2-ship` are one target here too. An
/// unresolvable roadmap or phase number folds to the input unchanged — it
/// simply will not equal a canonical stem.
fn normalize(store: &impl Store, project: &str, item_ref: &ItemRef) -> Result<ItemRef> {
    match item_ref {
        ItemRef::Phase { roadmap, stem } => {
            match crate::ops::phase::resolve_phase_stem(store, project, roadmap, stem) {
                Ok(resolved) => Ok(ItemRef::Phase {
                    roadmap: roadmap.clone(),
                    stem: resolved,
                }),
                Err(Error::RoadmapNotFound(_) | Error::PhaseNotFound(_)) => Ok(item_ref.clone()),
                Err(e) => Err(e),
            }
        }
        other => Ok(other.clone()),
    }
}

/// Creates a new implementation plan within a project.
///
/// Validation runs in a fixed order so the first failure is the most
/// specific one: the project must exist, the slug must be free, `implements`
/// must name a phase or a task (never a roadmap or another plan), a numeric
/// phase stem is normalized to the roadmap's canonical stem, and the named
/// item must exist on disk. A `supersedes` reference must name a *different*,
/// existing plan.
///
/// The predecessor's flip to [`PlanStatus::Superseded`] is written **before**
/// the new plan, so a validation failure leaves nothing half-written. Both
/// writes land in the same session changeset, so one `rdm commit` captures
/// the pair.
///
/// The new plan is always created [`PlanStatus::Draft`]: a plan's status is
/// derived from reviews (see [`set_plan_status`]), never chosen at creation.
///
/// # Errors
///
/// Returns [`Error::ProjectNotFound`] if the project doesn't exist,
/// [`Error::PlanExists`] if a plan with the same slug already exists,
/// [`Error::PlanImplementsInvalidKind`] if `implements` names a reference
/// kind that cannot be implemented by a plan,
/// [`Error::PlanImplementsMissing`] if the implemented phase or task does
/// not exist, [`Error::PlanSupersedesInvalidKind`] if `supersedes` names
/// anything other than a `plan/<slug>` reference,
/// [`Error::PlanSupersedesSelf`] if the plan names itself,
/// [`Error::PlanSupersedesMissing`] if the superseded plan does not exist,
/// [`Error::Io`] if file creation fails, or [`Error::FrontmatterParse`] if
/// frontmatter serialization fails.
pub fn create_plan(store: &mut impl Store, req: CreatePlan<'_>) -> Result<Document<Plan>> {
    let CreatePlan {
        project,
        slug,
        title,
        implements,
        supersedes,
        body,
    } = req;

    if !store.exists(&crate::paths::project_md_path(project)) {
        return Err(Error::ProjectNotFound(project.to_string()));
    }
    let path = crate::paths::plan_path(project, slug);
    if store.exists(&path) {
        return Err(Error::PlanExists(slug.to_string()));
    }

    // (iii) `implements` must be a phase or a task, (iv) normalized, and
    // (v) present on disk.
    let implements = match implements {
        ItemRef::Phase { .. } | ItemRef::Task { .. } => normalize(store, project, &implements)?,
        other => return Err(Error::PlanImplementsInvalidKind(other.label())),
    };
    let implements_path = match &implements {
        ItemRef::Phase { roadmap, stem } => crate::paths::phase_path(project, roadmap, stem),
        ItemRef::Task { slug } => crate::paths::task_path(project, slug),
        // Unreachable: the match above admits only Phase and Task.
        other => return Err(Error::PlanImplementsInvalidKind(other.label())),
    };
    if !store.exists(&implements_path) {
        return Err(Error::PlanImplementsMissing(implements.label()));
    }

    // `supersedes` must name a different, existing plan.
    if let Some(target) = &supersedes {
        let ItemRef::Plan {
            slug: predecessor, ..
        } = target
        else {
            return Err(Error::PlanSupersedesInvalidKind(target.label()));
        };
        if predecessor == slug {
            return Err(Error::PlanSupersedesSelf(slug.to_string()));
        }
        if !store.exists(&crate::paths::plan_path(project, predecessor)) {
            return Err(Error::PlanSupersedesMissing(predecessor.clone()));
        }
        // Predecessor first: every validation above has passed, so this
        // write can no longer be followed by a rejection.
        set_plan_status(store, project, predecessor, PlanStatus::Superseded)?;
    }

    let today = Local::now().date_naive();
    let doc = Document {
        frontmatter: Plan {
            project: project.to_string(),
            plan: slug.to_string(),
            title: title.to_string(),
            implements,
            supersedes,
            status: PlanStatus::Draft,
            created: today,
            updated: today,
        },
        body: body.unwrap_or_default().to_string(),
    };
    crate::io::write_plan(store, project, slug, &doc)?;
    Ok(doc)
}

/// Lists all implementation plans for a project, sorted by slug.
///
/// Returns `(slug, Document<Plan>)` tuples. Returns an empty vec when the
/// project has no `plans/` directory yet, mirroring
/// [`list_tasks`](crate::ops::task::list_tasks) — a project that has never
/// had a plan is not an error.
///
/// # Errors
///
/// Returns [`Error::ProjectNotFound`] if the project doesn't exist,
/// [`Error::Io`] if the directory cannot be read, or
/// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] if a plan file
/// has invalid frontmatter.
pub fn list_plans(store: &impl Store, project: &str) -> Result<Vec<(String, Document<Plan>)>> {
    if !store.exists(&crate::paths::project_md_path(project)) {
        return Err(Error::ProjectNotFound(project.to_string()));
    }
    let dir = crate::paths::plans_dir(project);
    let entries = store.list(&dir)?;

    let mut plans: Vec<(String, Document<Plan>)> = Vec::new();
    for entry in entries {
        if entry.kind != DirEntryKind::File {
            continue;
        }
        if !entry.name.ends_with(".md") {
            continue;
        }
        let slug = entry.name.trim_end_matches(".md").to_string();
        let doc = crate::io::load_plan(store, project, &slug)?;
        plans.push((slug, doc));
    }
    plans.sort_by(|(a, _), (b, _)| a.cmp(b));
    Ok(plans)
}

/// Returns whether `plan` satisfies every populated criterion in `filter`.
///
/// `filter.implements` is compared against the plan's own `implements` with
/// both sides normalized (see [`plans_implementing`]), so the numeric and
/// canonical phase forms match each other.
///
/// # Errors
///
/// Returns an error only for a genuine store failure while normalizing a
/// numeric phase stem — see [`crate::ops::phase::resolve_phase_stem`].
pub fn plan_matches(
    store: &impl Store,
    project: &str,
    plan: &Plan,
    filter: &PlanFilter,
) -> Result<bool> {
    let status_ok = filter.status.is_none_or(|s| plan.status == s);
    if !status_ok {
        return Ok(false);
    }
    match &filter.implements {
        None => Ok(true),
        Some(target) => {
            let wanted = normalize(store, project, target)?;
            let actual = normalize(store, project, &plan.implements)?;
            Ok(wanted == actual)
        }
    }
}

/// Filters `plans` to those satisfying `filter`, preserving order.
///
/// An owned convenience wrapper over [`plan_matches`] for callers holding a
/// `Vec` of `(slug, Document<Plan>)` pairs (e.g. [`list_plans`] output).
///
/// # Errors
///
/// Same as [`plan_matches`].
pub fn filter_plans(
    store: &impl Store,
    project: &str,
    plans: Vec<(String, Document<Plan>)>,
    filter: &PlanFilter,
) -> Result<Vec<(String, Document<Plan>)>> {
    let mut kept = Vec::new();
    for (slug, doc) in plans {
        if plan_matches(store, project, &doc.frontmatter, filter)? {
            kept.push((slug, doc));
        }
    }
    Ok(kept)
}

/// Lists the plans implementing `target`, slug-sorted.
///
/// Both the caller's `target` and each plan's stored `implements` are
/// normalized first, so `phase/auth/2` and `phase/auth/phase-2-ship` name the
/// same item here exactly as they do for `rdm backlinks`.
///
/// Returns an empty vec (never an `Io` error) when the project has no
/// `plans/` directory yet.
///
/// # Errors
///
/// Same as [`list_plans`], plus any genuine store failure while normalizing
/// a numeric phase stem.
pub fn plans_implementing(
    store: &impl Store,
    project: &str,
    target: &ItemRef,
) -> Result<Vec<(String, Document<Plan>)>> {
    filter_plans(
        store,
        project,
        list_plans(store, project)?,
        &PlanFilter {
            implements: Some(target.clone()),
            status: None,
        },
    )
}

/// Updates a plan's title and/or body, bumping its `updated` date.
///
/// There is deliberately no status parameter: a plan's status is derived
/// from reviews (see [`set_plan_status`]), never set with a flag.
///
/// # Errors
///
/// Returns [`Error::PlanNotFound`] if the plan doesn't exist,
/// [`Error::EmptyTitle`] if `title` is [`TitleUpdate::Set`] with an empty or
/// whitespace-only value, [`Error::BodyClobberRefused`] if `body` is
/// [`BodyUpdate::Set("")`](BodyUpdate::Set) over a non-empty body (use
/// [`BodyUpdate::Clear`] to confirm), [`Error::Io`] if reading or writing
/// fails, or [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] if
/// the existing plan file has invalid frontmatter.
pub fn update_plan(
    store: &mut impl Store,
    project: &str,
    slug: &str,
    title: TitleUpdate,
    body: BodyUpdate,
) -> Result<Document<Plan>> {
    let mut doc = crate::io::load_plan(store, project, slug)?;
    title.apply(&mut doc.frontmatter.title)?;
    body.apply(&mut doc.body)?;
    doc.frontmatter.updated = Local::now().date_naive();
    crate::io::write_plan(store, project, slug, &doc)?;
    Ok(doc)
}

/// Sets a plan's status, bumping its `updated` date.
///
/// This is the one write path for a plan's status, and it enforces two rules
/// the review-derived flips in [`crate::ops::reviews::submit_review`] depend
/// on:
///
/// - **`superseded` is terminal.** A plan already
///   [`PlanStatus::Superseded`] is never downgraded — a later `approve` on a
///   stale plan leaves it superseded. The only exception is re-setting
///   `Superseded` itself, which is a no-op.
/// - **Idempotent.** Setting the status a plan already has rewrites the file
///   (refreshing `updated`) rather than erroring.
///
/// # Errors
///
/// Returns [`Error::PlanNotFound`] if the plan doesn't exist,
/// [`Error::Io`] if reading or writing fails, or
/// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] if the existing
/// plan file has invalid frontmatter.
pub fn set_plan_status(
    store: &mut impl Store,
    project: &str,
    slug: &str,
    status: PlanStatus,
) -> Result<Document<Plan>> {
    let mut doc = crate::io::load_plan(store, project, slug)?;
    if doc.frontmatter.status == PlanStatus::Superseded && status != PlanStatus::Superseded {
        // Terminal: reviewing a stale plan must never resurrect it.
        return Ok(doc);
    }
    doc.frontmatter.status = status;
    doc.frontmatter.updated = Local::now().date_naive();
    crate::io::write_plan(store, project, slug, &doc)?;
    Ok(doc)
}

/// Deletes a plan file.
///
/// Reviews targeting the deleted plan are left alone: a review is the
/// durable record of the feedback, and a dangling target is a normal state
/// (see [`crate::model::ReviewTarget`]).
///
/// # Errors
///
/// Returns [`Error::PlanNotFound`] if the plan doesn't exist, or
/// [`Error::Io`] if removal fails.
pub fn delete_plan(store: &mut impl Store, project: &str, slug: &str) -> Result<()> {
    let path = crate::paths::plan_path(project, slug);
    if !store.exists(&path) {
        return Err(Error::PlanNotFound(slug.to_string()));
    }
    store.delete(&path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Phase, PhaseStatus, Priority, Project, Roadmap, Task, TaskStatus};
    use crate::store::MemoryStore;

    /// A project with one roadmap (`auth`, phase 2 `phase-2-ship`) and one
    /// task (`fix-login`) — the two kinds a plan may implement.
    fn setup_store() -> MemoryStore {
        let mut store = MemoryStore::new();
        store
            .write(
                &crate::paths::project_md_path("test"),
                Document {
                    frontmatter: Project {
                        name: "test".to_string(),
                        title: "Test Project".to_string(),
                        source: None,
                    },
                    body: String::new(),
                }
                .render()
                .unwrap(),
            )
            .unwrap();
        crate::io::write_roadmap(
            &mut store,
            "test",
            "auth",
            &Document {
                frontmatter: Roadmap {
                    project: "test".to_string(),
                    roadmap: "auth".to_string(),
                    title: "Auth".to_string(),
                    phases: vec!["phase-2-ship".to_string()],
                    dependencies: None,
                    priority: None,
                    tags: None,
                },
                body: String::new(),
            },
        )
        .unwrap();
        crate::io::write_phase(
            &mut store,
            "test",
            "auth",
            "phase-2-ship",
            &Document {
                frontmatter: Phase {
                    phase: 2,
                    title: "Ship".to_string(),
                    status: PhaseStatus::NotStarted,
                    tags: None,
                    completed: None,
                    commit: None,
                    review_sha: None,
                    review_branch: None,
                    difficulty: None,
                    model: None,
                    blocked_reason: None,
                },
                body: String::new(),
            },
        )
        .unwrap();
        crate::io::write_task(
            &mut store,
            "test",
            "fix-login",
            &Document {
                frontmatter: Task {
                    project: "test".to_string(),
                    title: "Fix login".to_string(),
                    status: TaskStatus::Open,
                    priority: Priority::Medium,
                    created: chrono::NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
                    tags: None,
                    completed: None,
                    commit: None,
                    review_sha: None,
                    review_branch: None,
                    close_reason: None,
                },
                body: String::new(),
            },
        )
        .unwrap();
        store
    }

    fn task_ref() -> ItemRef {
        ItemRef::Task {
            slug: "fix-login".to_string(),
        }
    }

    fn create(store: &mut MemoryStore, slug: &str, implements: ItemRef) -> Result<Document<Plan>> {
        create_plan(
            store,
            CreatePlan {
                project: "test",
                slug,
                title: "A plan",
                implements,
                supersedes: None,
                body: Some("## Approach\n\nDetails.\n"),
            },
        )
    }

    // -- AC1: create-time validation --

    #[test]
    fn create_plan_writes_a_draft() {
        let mut store = setup_store();
        let doc = create(&mut store, "impl-login", task_ref()).unwrap();
        assert_eq!(doc.frontmatter.status, PlanStatus::Draft);
        assert_eq!(doc.frontmatter.plan, "impl-login");
        assert_eq!(doc.frontmatter.created, doc.frontmatter.updated);
        // ...and it round-trips off disk.
        let loaded = crate::io::load_plan(&store, "test", "impl-login").unwrap();
        assert_eq!(loaded, doc);
    }

    #[test]
    fn create_plan_rejects_missing_implements_target() {
        let mut store = setup_store();
        let err = create(
            &mut store,
            "impl-ghost",
            ItemRef::Task {
                slug: "no-such-task".to_string(),
            },
        )
        .unwrap_err();
        assert!(matches!(err, Error::PlanImplementsMissing(_)), "{err:?}");
        // Nothing was written.
        assert!(!store.exists(&crate::paths::plan_path("test", "impl-ghost")));
    }

    #[test]
    fn create_plan_rejects_roadmap_implements_kind() {
        let mut store = setup_store();
        let err = create(
            &mut store,
            "impl-auth",
            ItemRef::Roadmap {
                roadmap: "auth".to_string(),
            },
        )
        .unwrap_err();
        assert!(
            matches!(err, Error::PlanImplementsInvalidKind(_)),
            "{err:?}"
        );
    }

    #[test]
    fn create_plan_rejects_plan_implements_kind() {
        let mut store = setup_store();
        create(&mut store, "impl-login", task_ref()).unwrap();
        let err = create(
            &mut store,
            "impl-meta",
            ItemRef::Plan {
                slug: "impl-login".to_string(),
            },
        )
        .unwrap_err();
        assert!(
            matches!(err, Error::PlanImplementsInvalidKind(_)),
            "{err:?}"
        );
    }

    #[test]
    fn create_plan_normalizes_numeric_phase_stem() {
        let mut store = setup_store();
        let doc = create(
            &mut store,
            "impl-ship",
            ItemRef::Phase {
                roadmap: "auth".to_string(),
                stem: "2".to_string(),
            },
        )
        .unwrap();
        assert_eq!(
            doc.frontmatter.implements,
            ItemRef::Phase {
                roadmap: "auth".to_string(),
                stem: "phase-2-ship".to_string()
            }
        );
    }

    #[test]
    fn create_plan_rejects_a_duplicate_slug() {
        let mut store = setup_store();
        create(&mut store, "impl-login", task_ref()).unwrap();
        let err = create(&mut store, "impl-login", task_ref()).unwrap_err();
        assert!(matches!(err, Error::PlanExists(_)), "{err:?}");
    }

    #[test]
    fn create_plan_rejects_an_unknown_project() {
        let mut store = MemoryStore::new();
        let err = create(&mut store, "impl-login", task_ref()).unwrap_err();
        assert!(matches!(err, Error::ProjectNotFound(_)), "{err:?}");
    }

    // -- AC2: supersede-derived status --

    #[test]
    fn create_plan_with_supersedes_flips_only_the_named_predecessor() {
        let mut store = setup_store();
        create(&mut store, "plan-a", task_ref()).unwrap();
        create(&mut store, "plan-b", task_ref()).unwrap();
        create_plan(
            &mut store,
            CreatePlan {
                project: "test",
                slug: "plan-c",
                title: "Third attempt",
                implements: task_ref(),
                supersedes: Some(ItemRef::Plan {
                    slug: "plan-b".to_string(),
                }),
                body: None,
            },
        )
        .unwrap();

        let a = crate::io::load_plan(&store, "test", "plan-a").unwrap();
        let b = crate::io::load_plan(&store, "test", "plan-b").unwrap();
        let c = crate::io::load_plan(&store, "test", "plan-c").unwrap();
        assert_eq!(a.frontmatter.status, PlanStatus::Draft, "A is untouched");
        assert_eq!(b.frontmatter.status, PlanStatus::Superseded);
        assert_eq!(c.frontmatter.status, PlanStatus::Draft);
    }

    #[test]
    fn create_plan_rejects_self_supersede() {
        let mut store = setup_store();
        let err = create_plan(
            &mut store,
            CreatePlan {
                project: "test",
                slug: "plan-a",
                title: "A",
                implements: task_ref(),
                supersedes: Some(ItemRef::Plan {
                    slug: "plan-a".to_string(),
                }),
                body: None,
            },
        )
        .unwrap_err();
        assert!(matches!(err, Error::PlanSupersedesSelf(_)), "{err:?}");
        assert!(!store.exists(&crate::paths::plan_path("test", "plan-a")));
    }

    #[test]
    fn create_plan_rejects_missing_supersede_target() {
        let mut store = setup_store();
        let err = create_plan(
            &mut store,
            CreatePlan {
                project: "test",
                slug: "plan-b",
                title: "B",
                implements: task_ref(),
                supersedes: Some(ItemRef::Plan {
                    slug: "no-such-plan".to_string(),
                }),
                body: None,
            },
        )
        .unwrap_err();
        assert!(matches!(err, Error::PlanSupersedesMissing(_)), "{err:?}");
        assert!(!store.exists(&crate::paths::plan_path("test", "plan-b")));
    }

    #[test]
    fn create_plan_rejects_a_non_plan_supersede_kind() {
        let mut store = setup_store();
        let err = create_plan(
            &mut store,
            CreatePlan {
                project: "test",
                slug: "plan-b",
                title: "B",
                implements: task_ref(),
                supersedes: Some(task_ref()),
                body: None,
            },
        )
        .unwrap_err();
        assert!(
            matches!(err, Error::PlanSupersedesInvalidKind(_)),
            "{err:?}"
        );
        // The rendered message must name the flag that was actually wrong:
        // `--implements` was fine here, only `--supersedes` was not.
        let rendered = err.to_string();
        assert!(rendered.contains("--supersedes plan/<slug>"), "{rendered}");
        assert!(!rendered.contains("--implements"), "{rendered}");
    }

    #[test]
    fn set_plan_status_never_downgrades_superseded() {
        let mut store = setup_store();
        create(&mut store, "plan-a", task_ref()).unwrap();
        set_plan_status(&mut store, "test", "plan-a", PlanStatus::Superseded).unwrap();
        let doc = set_plan_status(&mut store, "test", "plan-a", PlanStatus::Approved).unwrap();
        assert_eq!(doc.frontmatter.status, PlanStatus::Superseded);
        // ...and the refusal is persisted, not merely returned.
        let loaded = crate::io::load_plan(&store, "test", "plan-a").unwrap();
        assert_eq!(loaded.frontmatter.status, PlanStatus::Superseded);
    }

    #[test]
    fn set_plan_status_is_idempotent() {
        let mut store = setup_store();
        create(&mut store, "plan-a", task_ref()).unwrap();
        set_plan_status(&mut store, "test", "plan-a", PlanStatus::Approved).unwrap();
        let doc = set_plan_status(&mut store, "test", "plan-a", PlanStatus::Approved).unwrap();
        assert_eq!(doc.frontmatter.status, PlanStatus::Approved);
    }

    // -- AC3: listing and implements lookup --

    #[test]
    fn list_plans_missing_dir_is_empty() {
        let store = setup_store();
        assert!(list_plans(&store, "test").unwrap().is_empty());
    }

    #[test]
    fn list_plans_is_slug_sorted() {
        let mut store = setup_store();
        for slug in ["zz-last", "aa-first", "mm-middle"] {
            create(&mut store, slug, task_ref()).unwrap();
        }
        let slugs: Vec<String> = list_plans(&store, "test")
            .unwrap()
            .into_iter()
            .map(|(s, _)| s)
            .collect();
        assert_eq!(slugs, ["aa-first", "mm-middle", "zz-last"]);
    }

    #[test]
    fn plans_implementing_matches_across_numeric_and_canonical_stems() {
        let mut store = setup_store();
        // Stored via the numeric form (normalized at create time)...
        create(
            &mut store,
            "impl-numeric",
            ItemRef::Phase {
                roadmap: "auth".to_string(),
                stem: "2".to_string(),
            },
        )
        .unwrap();
        // ...and via the canonical form.
        create(
            &mut store,
            "impl-canonical",
            ItemRef::Phase {
                roadmap: "auth".to_string(),
                stem: "phase-2-ship".to_string(),
            },
        )
        .unwrap();
        // A plan on an unrelated item must not match.
        create(&mut store, "impl-login", task_ref()).unwrap();

        for query in ["2", "phase-2-ship"] {
            let found = plans_implementing(
                &store,
                "test",
                &ItemRef::Phase {
                    roadmap: "auth".to_string(),
                    stem: query.to_string(),
                },
            )
            .unwrap();
            let slugs: Vec<String> = found.into_iter().map(|(s, _)| s).collect();
            assert_eq!(slugs, ["impl-canonical", "impl-numeric"], "query {query}");
        }
    }

    #[test]
    fn plans_implementing_is_empty_for_an_unimplemented_item() {
        let mut store = setup_store();
        create(&mut store, "impl-login", task_ref()).unwrap();
        let found = plans_implementing(
            &store,
            "test",
            &ItemRef::Phase {
                roadmap: "auth".to_string(),
                stem: "phase-2-ship".to_string(),
            },
        )
        .unwrap();
        assert!(found.is_empty());
    }

    #[test]
    fn filter_plans_narrows_by_status() {
        let mut store = setup_store();
        create(&mut store, "plan-a", task_ref()).unwrap();
        create(&mut store, "plan-b", task_ref()).unwrap();
        set_plan_status(&mut store, "test", "plan-b", PlanStatus::Approved).unwrap();
        let kept = filter_plans(
            &store,
            "test",
            list_plans(&store, "test").unwrap(),
            &PlanFilter {
                implements: None,
                status: Some(PlanStatus::Approved),
            },
        )
        .unwrap();
        let slugs: Vec<String> = kept.into_iter().map(|(s, _)| s).collect();
        assert_eq!(slugs, ["plan-b"]);
    }

    // -- update / delete --

    #[test]
    fn update_plan_sets_title_and_body() {
        let mut store = setup_store();
        create(&mut store, "plan-a", task_ref()).unwrap();
        let doc = update_plan(
            &mut store,
            "test",
            "plan-a",
            TitleUpdate::Set("Renamed".to_string()),
            BodyUpdate::Set("New body.\n".to_string()),
        )
        .unwrap();
        assert_eq!(doc.frontmatter.title, "Renamed");
        assert_eq!(doc.body, "New body.\n");
        // The slug is never renamed by a title change.
        assert_eq!(doc.frontmatter.plan, "plan-a");
    }

    #[test]
    fn update_plan_refuses_an_empty_body_clobber() {
        let mut store = setup_store();
        create(&mut store, "plan-a", task_ref()).unwrap();
        let err = update_plan(
            &mut store,
            "test",
            "plan-a",
            TitleUpdate::Keep,
            BodyUpdate::Set(String::new()),
        )
        .unwrap_err();
        assert!(matches!(err, Error::BodyClobberRefused), "{err:?}");
    }

    #[test]
    fn update_plan_clear_body_succeeds() {
        let mut store = setup_store();
        create(&mut store, "plan-a", task_ref()).unwrap();
        let doc = update_plan(
            &mut store,
            "test",
            "plan-a",
            TitleUpdate::Keep,
            BodyUpdate::Clear,
        )
        .unwrap();
        assert!(doc.body.is_empty());
    }

    #[test]
    fn update_plan_unknown_slug_errors() {
        let mut store = setup_store();
        let err = update_plan(
            &mut store,
            "test",
            "nope",
            TitleUpdate::Keep,
            BodyUpdate::Keep,
        )
        .unwrap_err();
        assert!(matches!(err, Error::PlanNotFound(_)), "{err:?}");
    }

    #[test]
    fn delete_plan_removes_the_file() {
        let mut store = setup_store();
        create(&mut store, "plan-a", task_ref()).unwrap();
        delete_plan(&mut store, "test", "plan-a").unwrap();
        assert!(!store.exists(&crate::paths::plan_path("test", "plan-a")));
        assert!(matches!(
            delete_plan(&mut store, "test", "plan-a"),
            Err(Error::PlanNotFound(_))
        ));
    }
}
