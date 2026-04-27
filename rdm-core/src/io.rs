//! Document I/O primitives for plan repo data.
//!
//! These functions read and write plan repo documents (configs, roadmaps,
//! phases, tasks) through a [`Store`].  They have no dependency on
//! a [`Store`] and can be used standalone.

use crate::config::Config;
use crate::document::Document;
use crate::error::{Error, Result};
use crate::model::{Phase, Project, Roadmap, Task};
use crate::store::Store;

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
}
