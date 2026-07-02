//! Document I/O primitives for plan repo data.
//!
//! These functions read and write plan repo documents (configs, roadmaps,
//! phases, tasks) through a [`Store`].  They have no dependency on
//! a [`Store`] and can be used standalone.

use crate::config::Config;
use crate::document::Document;
use crate::error::{Error, Result};
use crate::model::{Phase, Project, Review, Roadmap, Task};
use crate::store::{Store, VersionedStore};

/// Loads and parses `rdm.toml` from the plan repo root.
///
/// # Errors
///
/// Returns [`Error::ConfigNotFound`] if `rdm.toml` does not exist,
/// [`Error::Io`] on read failure, or [`Error::ConfigParse`] if the file
/// is not valid TOML.
pub fn load_config(store: &impl Store) -> Result<Config> {
    let path = crate::paths::config_path();
    if !store.exists(&path) {
        return Err(Error::ConfigNotFound);
    }
    let content = store.read(&path)?;
    Config::from_toml(&content)
}

/// Loads and parses a project document from the store.
///
/// # Errors
///
/// Returns [`Error::ProjectNotFound`] if the project directory or
/// `project.md` does not exist, [`Error::Io`] on read failure, or
/// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] if the
/// YAML is invalid.
pub fn load_project(store: &impl Store, name: &str) -> Result<Document<Project>> {
    let path = crate::paths::project_md_path(name);
    if !store.exists(&path) {
        return Err(Error::ProjectNotFound(name.to_string()));
    }
    let content = store.read(&path)?;
    Document::parse(&content)
}

/// Loads and parses a roadmap document from the store.
///
/// # Errors
///
/// Returns [`Error::RoadmapNotFound`] if the roadmap file does not exist,
/// [`Error::Io`] on read failure, or
/// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] if the
/// YAML is invalid.
pub fn load_roadmap(store: &impl Store, project: &str, roadmap: &str) -> Result<Document<Roadmap>> {
    let path = crate::paths::roadmap_path(project, roadmap);
    if !store.exists(&path) {
        return Err(Error::RoadmapNotFound(roadmap.to_string()));
    }
    let content = store.read(&path)?;
    Document::parse(&content)
}

/// Loads and parses a phase document from the store.
///
/// # Errors
///
/// Returns [`Error::PhaseNotFound`] if the phase file does not exist,
/// [`Error::Io`] on read failure, or
/// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] if the
/// YAML is invalid.
pub fn load_phase(
    store: &impl Store,
    project: &str,
    roadmap: &str,
    phase_stem: &str,
) -> Result<Document<Phase>> {
    let path = crate::paths::phase_path(project, roadmap, phase_stem);
    if !store.exists(&path) {
        return Err(Error::PhaseNotFound(phase_stem.to_string()));
    }
    let content = store.read(&path)?;
    Document::parse(&content)
}

/// Loads and parses a task document from the store.
///
/// # Errors
///
/// Returns [`Error::TaskNotFound`] if the task file does not exist,
/// [`Error::Io`] on read failure, or
/// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] if the
/// YAML is invalid.
pub fn load_task(store: &impl Store, project: &str, task_slug: &str) -> Result<Document<Task>> {
    let path = crate::paths::task_path(project, task_slug);
    if !store.exists(&path) {
        return Err(Error::TaskNotFound(task_slug.to_string()));
    }
    let content = store.read(&path)?;
    Document::parse(&content)
}

/// Loads and parses a review document from the store.
///
/// Parsing never validates the review's target: a review whose target
/// roadmap/phase/task has been renamed or deleted (a dangling target) still
/// loads successfully.
///
/// # Errors
///
/// Returns [`Error::ReviewNotFound`] if the review file does not exist,
/// [`Error::Io`] on read failure, or
/// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] if the
/// YAML is invalid.
pub fn load_review(store: &impl Store, project: &str, review_id: &str) -> Result<Document<Review>> {
    let path = crate::paths::review_path(project, review_id);
    if !store.exists(&path) {
        return Err(Error::ReviewNotFound(review_id.to_string()));
    }
    let content = store.read(&path)?;
    Document::parse(&content)
}

/// Loads and parses an archived roadmap document from the store.
///
/// # Errors
///
/// Returns [`Error::RoadmapNotFound`] if the archived roadmap file does not
/// exist, [`Error::Io`] on read failure, or
/// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] if the
/// YAML is invalid.
pub fn load_archived_roadmap(
    store: &impl Store,
    project: &str,
    roadmap: &str,
) -> Result<Document<Roadmap>> {
    let path = crate::paths::archived_roadmap_path(project, roadmap);
    if !store.exists(&path) {
        return Err(Error::RoadmapNotFound(roadmap.to_string()));
    }
    let content = store.read(&path)?;
    Document::parse(&content)
}

/// Loads and parses a phase document within an archived roadmap.
///
/// # Errors
///
/// Returns [`Error::PhaseNotFound`] if the archived phase file does not
/// exist, [`Error::Io`] on read failure, or
/// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] if the
/// YAML is invalid.
pub fn load_archived_phase(
    store: &impl Store,
    project: &str,
    roadmap: &str,
    phase_stem: &str,
) -> Result<Document<Phase>> {
    let path = crate::paths::archived_phase_path(project, roadmap, phase_stem);
    if !store.exists(&path) {
        return Err(Error::PhaseNotFound(phase_stem.to_string()));
    }
    let content = store.read(&path)?;
    Document::parse(&content)
}

/// Loads a roadmap document with its body read at a specific git revision.
///
/// Metadata (frontmatter) reflects the current state, but the body is the
/// content that was present at `sha`. Use this to inspect "what did this
/// look like back then" without rewriting the rest of the document.
///
/// # Errors
///
/// Returns [`Error::RoadmapNotFound`] if the roadmap does not currently
/// exist, [`Error::RevisionUnknown`] if `sha` is not a known revision,
/// [`Error::BodyAtRevisionMissing`] if `sha` exists but the roadmap file
/// is not present at that revision, [`Error::HistoryUnavailable`] if the
/// backend cannot serve history, or
/// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] if the
/// historical content fails to parse.
pub fn load_roadmap_at(
    store: &impl VersionedStore,
    project: &str,
    roadmap: &str,
    sha: &str,
) -> Result<Document<Roadmap>> {
    let mut doc = load_roadmap(store, project, roadmap)?;
    let path = crate::paths::roadmap_path(project, roadmap);
    let historical = store.fetch_body_at(&path, sha)?;
    doc.body = Document::<Roadmap>::parse(&historical)?.body;
    Ok(doc)
}

/// Loads a phase document with its body read at a specific git revision.
///
/// Metadata reflects the current state; only the body is replaced with the
/// version from `sha`.
///
/// # Errors
///
/// Returns [`Error::PhaseNotFound`] if the phase does not currently exist,
/// [`Error::RevisionUnknown`] if `sha` is not a known revision,
/// [`Error::BodyAtRevisionMissing`] if `sha` exists but the phase file is
/// not present at that revision, [`Error::HistoryUnavailable`] if the
/// backend cannot serve history, or
/// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] if the
/// historical content fails to parse.
pub fn load_phase_at(
    store: &impl VersionedStore,
    project: &str,
    roadmap: &str,
    phase_stem: &str,
    sha: &str,
) -> Result<Document<Phase>> {
    let mut doc = load_phase(store, project, roadmap, phase_stem)?;
    let path = crate::paths::phase_path(project, roadmap, phase_stem);
    let historical = store.fetch_body_at(&path, sha)?;
    doc.body = Document::<Phase>::parse(&historical)?.body;
    Ok(doc)
}

/// Loads a task document with its body read at a specific git revision.
///
/// Metadata reflects the current state; only the body is replaced with the
/// version from `sha`.
///
/// # Errors
///
/// Returns [`Error::TaskNotFound`] if the task does not currently exist,
/// [`Error::RevisionUnknown`] if `sha` is not a known revision,
/// [`Error::BodyAtRevisionMissing`] if `sha` exists but the task file is
/// not present at that revision, [`Error::HistoryUnavailable`] if the
/// backend cannot serve history, or
/// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] if the
/// historical content fails to parse.
pub fn load_task_at(
    store: &impl VersionedStore,
    project: &str,
    task_slug: &str,
    sha: &str,
) -> Result<Document<Task>> {
    let mut doc = load_task(store, project, task_slug)?;
    let path = crate::paths::task_path(project, task_slug);
    let historical = store.fetch_body_at(&path, sha)?;
    doc.body = Document::<Task>::parse(&historical)?.body;
    Ok(doc)
}

/// Writes a roadmap document to the store.
///
/// # Errors
///
/// Returns [`Error::Io`] if writing fails, or
/// [`Error::FrontmatterParse`] if the frontmatter cannot be serialized.
pub fn write_roadmap(
    store: &mut impl Store,
    project: &str,
    roadmap: &str,
    doc: &Document<Roadmap>,
) -> Result<()> {
    let path = crate::paths::roadmap_path(project, roadmap);
    let content = doc.render()?;
    store.write(&path, content)?;
    Ok(())
}

/// Writes a phase document to the store.
///
/// # Errors
///
/// Returns [`Error::Io`] if writing fails, or
/// [`Error::FrontmatterParse`] if the frontmatter cannot be serialized.
pub fn write_phase(
    store: &mut impl Store,
    project: &str,
    roadmap: &str,
    phase_stem: &str,
    doc: &Document<Phase>,
) -> Result<()> {
    let path = crate::paths::phase_path(project, roadmap, phase_stem);
    let content = doc.render()?;
    store.write(&path, content)?;
    Ok(())
}

/// Writes a task document to the store.
///
/// # Errors
///
/// Returns [`Error::Io`] if writing fails, or
/// [`Error::FrontmatterParse`] if the frontmatter cannot be serialized.
pub fn write_task(
    store: &mut impl Store,
    project: &str,
    task_slug: &str,
    doc: &Document<Task>,
) -> Result<()> {
    let path = crate::paths::task_path(project, task_slug);
    let content = doc.render()?;
    store.write(&path, content)?;
    Ok(())
}

/// Writes a review document to the store.
///
/// # Errors
///
/// Returns [`Error::Io`] if writing fails, or
/// [`Error::FrontmatterParse`] if the frontmatter cannot be serialized.
pub fn write_review(
    store: &mut impl Store,
    project: &str,
    review_id: &str,
    doc: &Document<Review>,
) -> Result<()> {
    let path = crate::paths::review_path(project, review_id);
    let content = doc.render()?;
    store.write(&path, content)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{PhaseStatus, TaskStatus};
    use crate::store::MemoryStore;

    fn setup_store() -> MemoryStore {
        let mut store = MemoryStore::new();
        // Write a minimal config
        store
            .write(
                &crate::paths::config_path(),
                "default_project = \"test\"\n".to_string(),
            )
            .unwrap();
        // Write a project file
        let project_doc = Document {
            frontmatter: Project {
                name: "test".to_string(),
                title: "Test Project".to_string(),
            },
            body: String::new(),
        };
        store
            .write(
                &crate::paths::project_md_path("test"),
                project_doc.render().unwrap(),
            )
            .unwrap();
        store
    }

    #[test]
    fn load_config_returns_parsed_config() {
        let store = setup_store();
        let config = load_config(&store).unwrap();
        assert_eq!(config.default_project.as_deref(), Some("test"));
    }

    #[test]
    fn load_config_not_found() {
        let store = MemoryStore::new();
        assert!(matches!(load_config(&store), Err(Error::ConfigNotFound)));
    }

    #[test]
    fn load_project_returns_parsed_project() {
        let store = setup_store();
        let doc = load_project(&store, "test").unwrap();
        assert_eq!(doc.frontmatter.title, "Test Project");
    }

    #[test]
    fn load_project_not_found() {
        let store = setup_store();
        assert!(matches!(
            load_project(&store, "nonexistent"),
            Err(Error::ProjectNotFound(_))
        ));
    }

    #[test]
    fn write_and_load_roadmap_round_trip() {
        let mut store = setup_store();
        let doc = Document {
            frontmatter: Roadmap {
                project: "test".to_string(),
                roadmap: "alpha".to_string(),
                title: "Alpha".to_string(),
                phases: vec![],
                dependencies: None,
                priority: None,
                tags: None,
            },
            body: "Roadmap body.".to_string(),
        };
        write_roadmap(&mut store, "test", "alpha", &doc).unwrap();
        let loaded = load_roadmap(&store, "test", "alpha").unwrap();
        assert_eq!(loaded.frontmatter.title, "Alpha");
        assert_eq!(loaded.body, "Roadmap body.\n");
    }

    #[test]
    fn load_roadmap_not_found() {
        let store = setup_store();
        assert!(matches!(
            load_roadmap(&store, "test", "nonexistent"),
            Err(Error::RoadmapNotFound(_))
        ));
    }

    #[test]
    fn write_and_load_phase_round_trip() {
        let mut store = setup_store();
        let doc = Document {
            frontmatter: Phase {
                phase: 1,
                title: "Phase One".to_string(),
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
            body: "Phase body.".to_string(),
        };
        write_phase(&mut store, "test", "alpha", "phase-1-one", &doc).unwrap();
        let loaded = load_phase(&store, "test", "alpha", "phase-1-one").unwrap();
        assert_eq!(loaded.frontmatter.title, "Phase One");
        assert_eq!(loaded.frontmatter.phase, 1);
        assert_eq!(loaded.body, "Phase body.\n");
    }

    #[test]
    fn load_phase_not_found() {
        let store = setup_store();
        assert!(matches!(
            load_phase(&store, "test", "alpha", "phase-99-nope"),
            Err(Error::PhaseNotFound(_))
        ));
    }

    #[test]
    fn write_and_load_task_round_trip() {
        let mut store = setup_store();
        let doc = Document {
            frontmatter: Task {
                project: "test".to_string(),
                title: "Fix bug".to_string(),
                status: TaskStatus::Open,
                priority: crate::model::Priority::Medium,
                created: chrono::Local::now().date_naive(),
                tags: None,
                completed: None,
                commit: None,
                review_sha: None,
                review_branch: None,
            },
            body: "Task body.".to_string(),
        };
        write_task(&mut store, "test", "fix-bug", &doc).unwrap();
        let loaded = load_task(&store, "test", "fix-bug").unwrap();
        assert_eq!(loaded.frontmatter.title, "Fix bug");
        assert_eq!(loaded.body, "Task body.\n");
    }

    #[test]
    fn load_task_not_found() {
        let store = setup_store();
        assert!(matches!(
            load_task(&store, "test", "nonexistent"),
            Err(Error::TaskNotFound(_))
        ));
    }

    fn sample_review(id: &str) -> Document<crate::model::Review> {
        use crate::model::{
            Anchor, Review, ReviewComment, ReviewCommentStatus, ReviewState, ReviewTarget, Verdict,
        };
        use chrono::TimeZone;
        Document {
            frontmatter: Review {
                id: id.to_string(),
                author: "ed".to_string(),
                target: ReviewTarget::Phase {
                    roadmap: "alpha".to_string(),
                    stem: "phase-1-one".to_string(),
                },
                state: ReviewState::Submitted,
                verdict: Some(Verdict::RequestChanges),
                created: chrono::Utc.with_ymd_and_hms(2026, 7, 1, 14, 30, 0).unwrap(),
                submitted: Some(chrono::Utc.with_ymd_and_hms(2026, 7, 1, 14, 55, 0).unwrap()),
                created_commit: Some("a1b2c3d".to_string()),
                comments: vec![ReviewComment {
                    id: 1,
                    doc: None,
                    status: ReviewCommentStatus::Open,
                    applied_commit: None,
                    anchor: Some(Anchor::TextQuote {
                        quote: "quoted".to_string(),
                        prefix: "before ".to_string(),
                        suffix: " after".to_string(),
                    }),
                    body: "Tighten this.".to_string(),
                    reply: None,
                }],
            },
            body: "Review summary.".to_string(),
        }
    }

    #[test]
    fn write_and_load_review_round_trip() {
        let mut store = setup_store();
        let doc = sample_review("2026-07-01-1430-a1b2");
        write_review(&mut store, "test", "2026-07-01-1430-a1b2", &doc).unwrap();
        let loaded = load_review(&store, "test", "2026-07-01-1430-a1b2").unwrap();
        assert_eq!(loaded.frontmatter, doc.frontmatter);
        assert_eq!(loaded.body, "Review summary.\n");
    }

    #[test]
    fn load_review_not_found() {
        let store = setup_store();
        assert!(matches!(
            load_review(&store, "test", "nonexistent"),
            Err(Error::ReviewNotFound(_))
        ));
    }

    #[test]
    fn load_archived_roadmap_returns_parsed_roadmap() {
        let mut store = setup_store();
        let doc = Document {
            frontmatter: Roadmap {
                project: "test".to_string(),
                roadmap: "alpha".to_string(),
                title: "Alpha".to_string(),
                phases: vec![],
                dependencies: None,
                priority: None,
                tags: None,
            },
            body: "Archived roadmap body.".to_string(),
        };
        let path = crate::paths::archived_roadmap_path("test", "alpha");
        store.write(&path, doc.render().unwrap()).unwrap();

        let loaded = load_archived_roadmap(&store, "test", "alpha").unwrap();
        assert_eq!(loaded.frontmatter.title, "Alpha");
        assert_eq!(loaded.body, "Archived roadmap body.\n");
    }

    #[test]
    fn load_archived_roadmap_not_found() {
        let store = setup_store();
        assert!(matches!(
            load_archived_roadmap(&store, "test", "nonexistent"),
            Err(Error::RoadmapNotFound(_))
        ));
    }

    #[test]
    fn load_archived_phase_returns_parsed_phase() {
        let mut store = setup_store();
        let doc = Document {
            frontmatter: Phase {
                phase: 1,
                title: "Phase One".to_string(),
                status: PhaseStatus::Done,
                tags: None,
                completed: None,
                commit: None,
                review_sha: None,
                review_branch: None,
                difficulty: None,
                model: None,
                blocked_reason: None,
            },
            body: "Archived phase body.".to_string(),
        };
        let path = crate::paths::archived_phase_path("test", "alpha", "phase-1-one");
        store.write(&path, doc.render().unwrap()).unwrap();

        let loaded = load_archived_phase(&store, "test", "alpha", "phase-1-one").unwrap();
        assert_eq!(loaded.frontmatter.title, "Phase One");
        assert_eq!(loaded.frontmatter.phase, 1);
        assert_eq!(loaded.body, "Archived phase body.\n");
    }

    #[test]
    fn load_archived_phase_not_found() {
        let store = setup_store();
        assert!(matches!(
            load_archived_phase(&store, "test", "alpha", "phase-99-nope"),
            Err(Error::PhaseNotFound(_))
        ));
    }

    // -- load_*_at history-aware loaders --

    fn seed_roadmap(store: &mut MemoryStore, body: &str) {
        let doc = Document {
            frontmatter: Roadmap {
                project: "test".to_string(),
                roadmap: "alpha".to_string(),
                title: "Alpha".to_string(),
                phases: Vec::new(),
                dependencies: None,
                priority: None,
                tags: None,
            },
            body: body.to_string(),
        };
        write_roadmap(store, "test", "alpha", &doc).unwrap();
    }

    fn seed_phase(store: &mut MemoryStore, body: &str) {
        let doc = Document {
            frontmatter: Phase {
                phase: 1,
                title: "One".to_string(),
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
            body: body.to_string(),
        };
        write_phase(store, "test", "alpha", "phase-1-one", &doc).unwrap();
    }

    fn seed_task(store: &mut MemoryStore, body: &str) {
        let doc = Document {
            frontmatter: Task {
                project: "test".to_string(),
                title: "Fix".to_string(),
                status: TaskStatus::Open,
                priority: crate::model::Priority::Medium,
                created: chrono::NaiveDate::from_ymd_opt(2026, 5, 1).unwrap(),
                tags: None,
                completed: None,
                commit: None,
                review_sha: None,
                review_branch: None,
            },
            body: body.to_string(),
        };
        write_task(store, "test", "fix", &doc).unwrap();
    }

    #[test]
    fn load_roadmap_at_returns_historical_body() {
        let mut store = setup_store();
        seed_roadmap(&mut store, "v1-body");
        store.commit().unwrap();
        let old = store.head_sha().unwrap();
        seed_roadmap(&mut store, "v2-body");
        store.commit().unwrap();

        let loaded = load_roadmap_at(&store, "test", "alpha", &old).unwrap();
        assert_eq!(loaded.frontmatter.title, "Alpha");
        assert_eq!(loaded.body, "v1-body\n");
    }

    #[test]
    fn load_roadmap_at_revision_unknown() {
        let mut store = setup_store();
        seed_roadmap(&mut store, "body");
        store.commit().unwrap();
        let err = load_roadmap_at(&store, "test", "alpha", "mem-nope").unwrap_err();
        assert!(matches!(err, Error::RevisionUnknown { .. }), "got {err:?}");
    }

    #[test]
    fn load_roadmap_at_missing_at_revision() {
        let mut store = setup_store();
        store.commit().unwrap();
        let pre = store.head_sha().unwrap();
        seed_roadmap(&mut store, "body");
        store.commit().unwrap();
        let err = load_roadmap_at(&store, "test", "alpha", &pre).unwrap_err();
        assert!(
            matches!(err, Error::BodyAtRevisionMissing { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn load_phase_at_returns_historical_body() {
        let mut store = setup_store();
        seed_phase(&mut store, "v1-phase");
        store.commit().unwrap();
        let old = store.head_sha().unwrap();
        seed_phase(&mut store, "v2-phase");
        store.commit().unwrap();
        let loaded = load_phase_at(&store, "test", "alpha", "phase-1-one", &old).unwrap();
        assert_eq!(loaded.body, "v1-phase\n");
    }

    #[test]
    fn load_phase_at_revision_unknown() {
        let mut store = setup_store();
        seed_phase(&mut store, "body");
        store.commit().unwrap();
        let err = load_phase_at(&store, "test", "alpha", "phase-1-one", "mem-nope").unwrap_err();
        assert!(matches!(err, Error::RevisionUnknown { .. }));
    }

    #[test]
    fn load_phase_at_missing_at_revision() {
        let mut store = setup_store();
        store.commit().unwrap();
        let pre = store.head_sha().unwrap();
        seed_phase(&mut store, "body");
        store.commit().unwrap();
        let err = load_phase_at(&store, "test", "alpha", "phase-1-one", &pre).unwrap_err();
        assert!(matches!(err, Error::BodyAtRevisionMissing { .. }));
    }

    #[test]
    fn load_task_at_returns_historical_body() {
        let mut store = setup_store();
        seed_task(&mut store, "v1-task");
        store.commit().unwrap();
        let old = store.head_sha().unwrap();
        seed_task(&mut store, "v2-task");
        store.commit().unwrap();
        let loaded = load_task_at(&store, "test", "fix", &old).unwrap();
        assert_eq!(loaded.body, "v1-task\n");
    }

    #[test]
    fn load_task_at_revision_unknown() {
        let mut store = setup_store();
        seed_task(&mut store, "body");
        store.commit().unwrap();
        let err = load_task_at(&store, "test", "fix", "mem-nope").unwrap_err();
        assert!(matches!(err, Error::RevisionUnknown { .. }));
    }

    #[test]
    fn load_task_at_missing_at_revision() {
        let mut store = setup_store();
        store.commit().unwrap();
        let pre = store.head_sha().unwrap();
        seed_task(&mut store, "body");
        store.commit().unwrap();
        let err = load_task_at(&store, "test", "fix", &pre).unwrap_err();
        assert!(matches!(err, Error::BodyAtRevisionMissing { .. }));
    }
}
