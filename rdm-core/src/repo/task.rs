use crate::document::Document;
use crate::error::Result;
use crate::model::{Priority, Roadmap, Task, TaskStatus};
use crate::store::Store;

use super::PlanRepo;

impl<S: Store> PlanRepo<S> {
    // -- Task operations (delegates to crate::ops::task) --

    /// Creates a new task within a project.
    ///
    /// `body` sets the markdown body below the frontmatter. Pass `None` for
    /// an empty body.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ProjectNotFound`] if the project doesn't exist,
    /// [`Error::DuplicateSlug`] if a task with the same slug already exists,
    /// [`Error::Io`] if file creation fails, or
    /// [`Error::FrontmatterParse`] if frontmatter serialization fails.
    pub fn create_task(
        &mut self,
        project: &str,
        slug: &str,
        title: &str,
        priority: Priority,
        tags: Option<Vec<String>>,
        body: Option<&str>,
    ) -> Result<Document<Task>> {
        crate::ops::task::create_task(&mut self.store, project, slug, title, priority, tags, body)
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
    pub fn list_tasks(&self, project: &str) -> Result<Vec<(String, Document<Task>)>> {
        crate::ops::task::list_tasks(&self.store, project)
    }

    /// Updates a task's status, priority, tags, and/or body.
    ///
    /// Only fields that are `Some(...)` are updated; others are left unchanged.
    ///
    /// # Errors
    ///
    /// Returns [`Error::TaskNotFound`] if the task file doesn't exist,
    /// [`Error::Io`] if reading or writing fails, or
    /// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] if the
    /// existing task file has invalid frontmatter.
    #[allow(clippy::too_many_arguments)]
    pub fn update_task(
        &mut self,
        project: &str,
        slug: &str,
        status: Option<TaskStatus>,
        priority: Option<Priority>,
        tags: Option<Vec<String>>,
        body: Option<&str>,
        commit: Option<String>,
    ) -> Result<Document<Task>> {
        crate::ops::task::update_task(
            &mut self.store,
            project,
            slug,
            status,
            priority,
            tags,
            body,
            commit,
        )
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
        &mut self,
        project: &str,
        task_slug: &str,
        roadmap_slug: &str,
    ) -> Result<Document<Roadmap>> {
        crate::ops::task::promote_task(&mut self.store, project, task_slug, roadmap_slug)
    }
}
