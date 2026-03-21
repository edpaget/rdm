use chrono::Local;

use crate::document::Document;
use crate::error::{Error, Result};
use crate::model::{Phase, PhaseStatus, Priority, Roadmap, Task, TaskStatus};
use crate::store::{DirEntryKind, Store};

use super::PlanRepo;

impl<S: Store> PlanRepo<S> {
    // -- Task operations --

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
        if !self.store.exists(&self.project_md_path(project)) {
            return Err(Error::ProjectNotFound(project.to_string()));
        }
        let path = self.task_path(project, slug);
        if self.store.exists(&path) {
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
            },
            body: body.unwrap_or_default().to_string(),
        };
        self.write_task(project, slug, &doc)?;
        self.store.commit()?;
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
    pub fn list_tasks(&self, project: &str) -> Result<Vec<(String, Document<Task>)>> {
        if !self.store.exists(&self.project_md_path(project)) {
            return Err(Error::ProjectNotFound(project.to_string()));
        }
        let dir = self.tasks_dir(project);
        let entries = self.store.list(&dir)?;

        let mut tasks: Vec<(String, Document<Task>)> = Vec::new();
        for entry in entries {
            if entry.kind != DirEntryKind::File {
                continue;
            }
            if !entry.name.ends_with(".md") {
                continue;
            }
            let slug = entry.name.trim_end_matches(".md").to_string();
            let doc = self.load_task(project, &slug)?;
            tasks.push((slug, doc));
        }
        tasks.sort_by(|(a, _), (b, _)| a.cmp(b));
        Ok(tasks)
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
        let path = self.task_path(project, slug);
        if !self.store.exists(&path) {
            return Err(Error::TaskNotFound(slug.to_string()));
        }

        let mut doc = self.load_task(project, slug)?;
        if let Some(status) = status {
            if status == TaskStatus::Done && doc.frontmatter.status == TaskStatus::Done {
                // Already done: only update commit if a new one is provided
                if let Some(sha) = commit {
                    doc.frontmatter.commit = Some(sha);
                }
            } else {
                doc.frontmatter.status = status;
                if status == TaskStatus::Done {
                    doc.frontmatter.completed = Some(Local::now().date_naive());
                    doc.frontmatter.commit = commit;
                } else {
                    doc.frontmatter.completed = None;
                    doc.frontmatter.commit = None;
                }
            }
        }
        if let Some(p) = priority {
            doc.frontmatter.priority = p;
        }
        if let Some(t) = tags {
            doc.frontmatter.tags = if t.is_empty() { None } else { Some(t) };
        }
        if let Some(b) = body {
            doc.body = b.to_string();
        }
        self.write_task(project, slug, &doc)?;
        self.store.commit()?;
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
        &mut self,
        project: &str,
        task_slug: &str,
        roadmap_slug: &str,
    ) -> Result<Document<Roadmap>> {
        let task_path = self.task_path(project, task_slug);
        if !self.store.exists(&task_path) {
            return Err(Error::TaskNotFound(task_slug.to_string()));
        }

        let task_doc = self.load_task(project, task_slug)?;

        let roadmap_file = self.roadmap_path(project, roadmap_slug);
        if self.store.exists(&roadmap_file) {
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
            },
            body: roadmap_body,
        };
        self.write_roadmap(project, roadmap_slug, &roadmap_doc)?;

        let phase_doc = Document {
            frontmatter: Phase {
                phase: 1,
                title: task_doc.frontmatter.title,
                status: PhaseStatus::NotStarted,
                completed: None,
                commit: None,
            },
            body: task_doc.body,
        };
        self.write_phase(project, roadmap_slug, &phase_slug, &phase_doc)?;

        self.store.delete(&task_path)?;
        self.store.commit()?;

        Ok(roadmap_doc)
    }
}
