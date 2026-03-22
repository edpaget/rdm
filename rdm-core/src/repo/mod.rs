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

    // -- Load operations --

    /// Loads and parses `rdm.toml` from the plan repo root.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ConfigNotFound`] if `rdm.toml` does not exist,
    /// [`Error::Io`] on read failure, or [`Error::ConfigParse`] if the file
    /// is not valid TOML.
    pub fn load_config(&self) -> Result<Config> {
        let path = crate::paths::config_path();
        if !self.store.exists(&path) {
            return Err(Error::ConfigNotFound);
        }
        let content = self.store.read(&path)?;
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
    pub fn load_project(&self, name: &str) -> Result<Document<Project>> {
        let path = crate::paths::project_md_path(name);
        if !self.store.exists(&path) {
            return Err(Error::ProjectNotFound(name.to_string()));
        }
        let content = self.store.read(&path)?;
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
    pub fn load_roadmap(&self, project: &str, roadmap: &str) -> Result<Document<Roadmap>> {
        let path = crate::paths::roadmap_path(project, roadmap);
        if !self.store.exists(&path) {
            return Err(Error::RoadmapNotFound(roadmap.to_string()));
        }
        let content = self.store.read(&path)?;
        Document::parse(&content)
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
        let content = self
            .store
            .read(&crate::paths::phase_path(project, roadmap, phase_stem))?;
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
    pub fn load_task(&self, project: &str, task_slug: &str) -> Result<Document<Task>> {
        let path = crate::paths::task_path(project, task_slug);
        if !self.store.exists(&path) {
            return Err(Error::TaskNotFound(task_slug.to_string()));
        }
        let content = self.store.read(&path)?;
        Document::parse(&content)
    }

    // -- Write operations --

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
        let path = crate::paths::roadmap_path(project, roadmap);
        let content = doc.render()?;
        self.store.write(&path, content)?;
        Ok(())
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
        let path = crate::paths::phase_path(project, roadmap, phase_stem);
        let content = doc.render()?;
        self.store.write(&path, content)?;
        Ok(())
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
        let path = crate::paths::task_path(project, task_slug);
        let content = doc.render()?;
        self.store.write(&path, content)?;
        Ok(())
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
