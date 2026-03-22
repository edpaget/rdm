//! Plan repo operations: path resolution, file I/O, and initialization.

use crate::config::Config;
use crate::document::Document;
use crate::error::{Error, Result};
use crate::model::{Phase, Project, Roadmap, Task};
use crate::store::{DirEntryKind, RelPath, Store};

mod index;
mod init;
mod phase;
mod roadmap;
mod task;

/// Represents an rdm plan repository backed by a [`Store`].
#[derive(Debug, Clone)]
pub struct PlanRepo<S: Store> {
    store: S,
}

impl<S: Store> PlanRepo<S> {
    /// Creates a new `PlanRepo` backed by the given store.
    pub fn new(store: S) -> Self {
        PlanRepo { store }
    }

    /// Returns a reference to the underlying store.
    pub fn store(&self) -> &S {
        &self.store
    }

    /// Returns a mutable reference to the underlying store.
    pub fn store_mut(&mut self) -> &mut S {
        &mut self.store
    }

    /// Commits any staged changes in the underlying store.
    ///
    /// # Errors
    ///
    /// Returns an error if the commit fails.
    pub fn commit(&mut self) -> Result<()> {
        self.store.commit()
    }

    // -- Load operations (delegates to crate::io) --

    /// Loads and parses `rdm.toml` from the plan repo root.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ConfigNotFound`] if `rdm.toml` does not exist,
    /// [`Error::Io`] on read failure, or [`Error::ConfigParse`] if the file
    /// is not valid TOML.
    pub fn load_config(&self) -> Result<Config> {
        crate::io::load_config(&self.store)
    }

    /// Loads and parses a project document from the store.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ProjectNotFound`] if the project directory or
    /// `project.md` does not exist, [`Error::Io`] on read failure, or
    /// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] if the
    /// YAML is invalid.
    pub fn load_project(&self, name: &str) -> Result<Document<Project>> {
        crate::io::load_project(&self.store, name)
    }

    /// Loads and parses a roadmap document from the store.
    ///
    /// # Errors
    ///
    /// Returns [`Error::RoadmapNotFound`] if the roadmap file does not exist,
    /// [`Error::Io`] on read failure, or
    /// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] if the
    /// YAML is invalid.
    pub fn load_roadmap(&self, project: &str, roadmap: &str) -> Result<Document<Roadmap>> {
        crate::io::load_roadmap(&self.store, project, roadmap)
    }

    /// Loads and parses a phase document from the store.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] if the file cannot be read,
    /// [`Error::FrontmatterMissing`] if delimiters are absent, or
    /// [`Error::FrontmatterParse`] if the YAML is invalid.
    pub fn load_phase(
        &self,
        project: &str,
        roadmap: &str,
        phase_stem: &str,
    ) -> Result<Document<Phase>> {
        crate::io::load_phase(&self.store, project, roadmap, phase_stem)
    }

    /// Loads and parses a task document from the store.
    ///
    /// # Errors
    ///
    /// Returns [`Error::TaskNotFound`] if the task file does not exist,
    /// [`Error::Io`] on read failure, or
    /// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] if the
    /// YAML is invalid.
    pub fn load_task(&self, project: &str, task_slug: &str) -> Result<Document<Task>> {
        crate::io::load_task(&self.store, project, task_slug)
    }

    // -- Write operations (delegates to crate::io) --

    /// Writes a roadmap document to the store.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] if writing fails, or
    /// [`Error::FrontmatterParse`] if the frontmatter cannot be serialized.
    pub fn write_roadmap(
        &mut self,
        project: &str,
        roadmap: &str,
        doc: &Document<Roadmap>,
    ) -> Result<()> {
        crate::io::write_roadmap(&mut self.store, project, roadmap, doc)
    }

    /// Writes a phase document to the store.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] if writing fails, or
    /// [`Error::FrontmatterParse`] if the frontmatter cannot be serialized.
    pub fn write_phase(
        &mut self,
        project: &str,
        roadmap: &str,
        phase_stem: &str,
        doc: &Document<Phase>,
    ) -> Result<()> {
        crate::io::write_phase(&mut self.store, project, roadmap, phase_stem, doc)
    }

    /// Writes a task document to the store.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] if writing fails, or
    /// [`Error::FrontmatterParse`] if the frontmatter cannot be serialized.
    pub fn write_task(
        &mut self,
        project: &str,
        task_slug: &str,
        doc: &Document<Task>,
    ) -> Result<()> {
        crate::io::write_task(&mut self.store, project, task_slug, doc)
    }

    // -- Project operations --

    /// Creates a new project with `roadmaps/` and `tasks/` subdirectories.
    ///
    /// # Errors
    ///
    /// Returns [`Error::DuplicateSlug`] if the project already exists,
    /// [`Error::Io`] if file creation fails, or
    /// [`Error::FrontmatterParse`] if frontmatter serialization fails.
    pub fn create_project(&mut self, name: &str, title: &str) -> Result<Document<Project>> {
        let md_path = crate::paths::project_md_path(name);
        if self.store.exists(&md_path) {
            return Err(Error::DuplicateSlug(name.to_string()));
        }

        let doc = Document {
            frontmatter: Project {
                name: name.to_string(),
                title: title.to_string(),
            },
            body: String::new(),
        };
        let content = doc.render()?;
        self.store.write(&md_path, content)?;
        self.store.commit()?;
        Ok(doc)
    }

    /// Lists all projects in the plan repo, sorted alphabetically.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] if the projects directory cannot be read.
    pub fn list_projects(&self) -> Result<Vec<String>> {
        let projects_dir = RelPath::new("projects").expect("valid path");
        let entries = self.store.list(&projects_dir)?;
        let mut names: Vec<String> = entries
            .into_iter()
            .filter(|e| e.kind == DirEntryKind::Dir)
            .map(|e| e.name)
            .collect();
        names.sort();
        Ok(names)
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
