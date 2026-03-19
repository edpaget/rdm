/// Plan repo operations: path resolution, file I/O, and initialization.
use chrono::Local;

use crate::config::Config;
use crate::display::{self, ProjectIndex, RoadmapIndexEntry};
use crate::document::Document;
use crate::error::{Error, Result};
use crate::model::{Phase, PhaseStatus, Priority, Project, Roadmap, Task, TaskStatus};
use crate::store::{DirEntryKind, RelPath, Store};

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

    // -- Path builders --

    /// Returns the path to `rdm.toml`.
    pub fn config_path(&self) -> RelPath {
        RelPath::new("rdm.toml").expect("valid path")
    }

    /// Returns the path to `INDEX.md`.
    pub fn index_path(&self) -> RelPath {
        RelPath::new("INDEX.md").expect("valid path")
    }

    /// Returns the path to a project's directory.
    pub fn project_path(&self, project: &str) -> RelPath {
        RelPath::new(&format!("projects/{project}")).expect("valid path")
    }

    /// Returns the path to a project's `project.md` file.
    fn project_md_path(&self, project: &str) -> RelPath {
        RelPath::new(&format!("projects/{project}/project.md")).expect("valid path")
    }

    /// Returns the path to a project's roadmaps directory.
    pub fn roadmaps_dir(&self, project: &str) -> RelPath {
        RelPath::new(&format!("projects/{project}/roadmaps")).expect("valid path")
    }

    /// Returns the path to a specific roadmap directory.
    pub fn roadmap_dir(&self, project: &str, roadmap: &str) -> RelPath {
        RelPath::new(&format!("projects/{project}/roadmaps/{roadmap}")).expect("valid path")
    }

    /// Returns the path to a roadmap's `roadmap.md` file.
    pub fn roadmap_path(&self, project: &str, roadmap: &str) -> RelPath {
        RelPath::new(&format!("projects/{project}/roadmaps/{roadmap}/roadmap.md"))
            .expect("valid path")
    }

    /// Returns the path to a phase file within a roadmap directory.
    pub fn phase_path(&self, project: &str, roadmap: &str, phase_stem: &str) -> RelPath {
        RelPath::new(&format!(
            "projects/{project}/roadmaps/{roadmap}/{phase_stem}.md"
        ))
        .expect("valid path")
    }

    /// Returns the path to a project's tasks directory.
    pub fn tasks_dir(&self, project: &str) -> RelPath {
        RelPath::new(&format!("projects/{project}/tasks")).expect("valid path")
    }

    /// Returns the path to a task file.
    pub fn task_path(&self, project: &str, task_slug: &str) -> RelPath {
        RelPath::new(&format!("projects/{project}/tasks/{task_slug}.md")).expect("valid path")
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
        let path = self.config_path();
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
        let path = self.project_md_path(name);
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
        let path = self.roadmap_path(project, roadmap);
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
            .read(&self.phase_path(project, roadmap, phase_stem))?;
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
        let path = self.task_path(project, task_slug);
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
        let path = self.roadmap_path(project, roadmap);
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
        let path = self.phase_path(project, roadmap, phase_stem);
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
        let path = self.task_path(project, task_slug);
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
        let md_path = self.project_md_path(name);
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

    // -- Roadmap operations --

    /// Creates a new roadmap within a project.
    ///
    /// `body` sets the markdown body below the frontmatter. Pass `None` for
    /// an empty body.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ProjectNotFound`] if the project doesn't exist,
    /// [`Error::DuplicateSlug`] if the roadmap already exists,
    /// [`Error::Io`] if file creation fails, or
    /// [`Error::FrontmatterParse`] if frontmatter serialization fails.
    pub fn create_roadmap(
        &mut self,
        project: &str,
        slug: &str,
        title: &str,
        body: Option<&str>,
    ) -> Result<Document<Roadmap>> {
        if !self.store.exists(&self.project_md_path(project)) {
            return Err(Error::ProjectNotFound(project.to_string()));
        }
        let roadmap_file = self.roadmap_path(project, slug);
        if self.store.exists(&roadmap_file) {
            return Err(Error::DuplicateSlug(slug.to_string()));
        }

        let doc = Document {
            frontmatter: Roadmap {
                project: project.to_string(),
                roadmap: slug.to_string(),
                title: title.to_string(),
                phases: Vec::new(),
                dependencies: None,
            },
            body: body.unwrap_or_default().to_string(),
        };
        self.write_roadmap(project, slug, &doc)?;
        self.store.commit()?;
        Ok(doc)
    }

    /// Updates a roadmap's body.
    ///
    /// When `body` is `Some`, replaces the existing body; `None` preserves it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::RoadmapNotFound`] if the roadmap doesn't exist,
    /// [`Error::Io`] if reading or writing fails, or
    /// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] if the
    /// existing roadmap file has invalid frontmatter.
    pub fn update_roadmap(
        &mut self,
        project: &str,
        slug: &str,
        body: Option<&str>,
    ) -> Result<Document<Roadmap>> {
        let path = self.roadmap_path(project, slug);
        if !self.store.exists(&path) {
            return Err(Error::RoadmapNotFound(slug.to_string()));
        }

        let mut doc = self.load_roadmap(project, slug)?;
        if let Some(b) = body {
            doc.body = b.to_string();
        }
        self.write_roadmap(project, slug, &doc)?;
        self.store.commit()?;
        Ok(doc)
    }

    /// Lists all roadmaps for a project, sorted by slug.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ProjectNotFound`] if the project doesn't exist,
    /// [`Error::Io`] if the directory cannot be read, or
    /// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] if a
    /// roadmap file has invalid frontmatter.
    pub fn list_roadmaps(&self, project: &str) -> Result<Vec<Document<Roadmap>>> {
        if !self.store.exists(&self.project_md_path(project)) {
            return Err(Error::ProjectNotFound(project.to_string()));
        }
        let roadmaps_dir = self.roadmaps_dir(project);
        let entries = self.store.list(&roadmaps_dir)?;
        let mut slugs: Vec<String> = entries
            .into_iter()
            .filter(|e| e.kind == DirEntryKind::Dir)
            .map(|e| e.name)
            .collect();
        slugs.sort();

        let mut roadmaps = Vec::new();
        for slug in slugs {
            // Skip directories without a roadmap.md (e.g., leftover empty dirs)
            if !self.store.exists(&self.roadmap_path(project, &slug)) {
                continue;
            }
            let doc = self.load_roadmap(project, &slug)?;
            roadmaps.push(doc);
        }
        Ok(roadmaps)
    }

    // -- Phase operations --

    /// Lists all phases in a roadmap, sorted by phase number.
    ///
    /// Returns `(stem, Document<Phase>)` tuples.
    ///
    /// # Errors
    ///
    /// Returns [`Error::RoadmapNotFound`] if the roadmap doesn't exist,
    /// [`Error::Io`] if the directory cannot be read, or
    /// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] if a
    /// phase file has invalid frontmatter.
    pub fn list_phases(
        &self,
        project: &str,
        roadmap: &str,
    ) -> Result<Vec<(String, Document<Phase>)>> {
        let roadmap_file = self.roadmap_path(project, roadmap);
        if !self.store.exists(&roadmap_file) {
            return Err(Error::RoadmapNotFound(roadmap.to_string()));
        }

        let dir = self.roadmap_dir(project, roadmap);
        let entries = self.store.list(&dir)?;

        let mut phases: Vec<(String, Document<Phase>)> = Vec::new();
        for entry in entries {
            if entry.kind != DirEntryKind::File {
                continue;
            }
            if entry.name == "roadmap.md" || !entry.name.ends_with(".md") {
                continue;
            }
            let stem = entry.name.trim_end_matches(".md").to_string();
            let doc = self.load_phase(project, roadmap, &stem)?;
            phases.push((stem, doc));
        }
        phases.sort_by_key(|(_, doc)| doc.frontmatter.phase);
        Ok(phases)
    }

    /// Creates a new phase within a roadmap.
    ///
    /// If `phase_number` is `None`, auto-assigns the next number.
    /// `body` sets the markdown body below the frontmatter. Pass `None` for
    /// an empty body.
    ///
    /// # Errors
    ///
    /// Returns [`Error::RoadmapNotFound`] if the roadmap doesn't exist,
    /// [`Error::DuplicateSlug`] if a phase with the same stem already exists,
    /// [`Error::Io`] if file creation fails, or
    /// [`Error::FrontmatterParse`] if frontmatter serialization fails.
    pub fn create_phase(
        &mut self,
        project: &str,
        roadmap: &str,
        slug: &str,
        title: &str,
        phase_number: Option<u32>,
        body: Option<&str>,
    ) -> Result<Document<Phase>> {
        let roadmap_file = self.roadmap_path(project, roadmap);
        if !self.store.exists(&roadmap_file) {
            return Err(Error::RoadmapNotFound(roadmap.to_string()));
        }

        let number = match phase_number {
            Some(n) => n,
            None => {
                let existing = self.list_phases(project, roadmap)?;
                existing
                    .last()
                    .map(|(_, doc)| doc.frontmatter.phase + 1)
                    .unwrap_or(1)
            }
        };

        let stem = crate::model::phase_stem(number, slug);
        let path = self.phase_path(project, roadmap, &stem);
        if self.store.exists(&path) {
            return Err(Error::DuplicateSlug(stem));
        }

        let doc = Document {
            frontmatter: Phase {
                phase: number,
                title: title.to_string(),
                status: PhaseStatus::NotStarted,
                completed: None,
            },
            body: body.unwrap_or_default().to_string(),
        };
        self.write_phase(project, roadmap, &stem, &doc)?;

        // Update roadmap's phases list
        let mut roadmap_doc = self.load_roadmap(project, roadmap)?;
        roadmap_doc.frontmatter.phases.push(stem);
        self.write_roadmap(project, roadmap, &roadmap_doc)?;
        self.store.commit()?;

        Ok(doc)
    }

    /// Updates a phase's status and/or body.
    ///
    /// When `status` is `Some(Done)`, auto-sets `completed` to today.
    /// When `status` is `Some` but not `Done`, clears `completed`.
    /// When `status` is `None`, the existing status is preserved.
    /// When `body` is `Some`, replaces the existing body; `None` preserves it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::PhaseNotFound`] if the phase file doesn't exist,
    /// [`Error::Io`] if reading or writing fails, or
    /// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] if the
    /// existing phase file has invalid frontmatter.
    pub fn update_phase(
        &mut self,
        project: &str,
        roadmap: &str,
        phase_stem: &str,
        status: Option<PhaseStatus>,
        body: Option<&str>,
    ) -> Result<Document<Phase>> {
        let path = self.phase_path(project, roadmap, phase_stem);
        if !self.store.exists(&path) {
            return Err(Error::PhaseNotFound(phase_stem.to_string()));
        }

        let mut doc = self.load_phase(project, roadmap, phase_stem)?;
        if let Some(status) = status {
            doc.frontmatter.status = status;
            doc.frontmatter.completed = if status == PhaseStatus::Done {
                Some(Local::now().date_naive())
            } else {
                None
            };
        }
        if let Some(b) = body {
            doc.body = b.to_string();
        }
        self.write_phase(project, roadmap, phase_stem, &doc)?;
        self.store.commit()?;
        Ok(doc)
    }

    /// Removes a phase from a roadmap.
    ///
    /// Deletes the phase file and removes its stem from the roadmap's `phases`
    /// list.
    ///
    /// # Errors
    ///
    /// Returns [`Error::PhaseNotFound`] if the phase file doesn't exist,
    /// [`Error::Io`] if the file cannot be deleted or the roadmap cannot be
    /// updated, or [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`]
    /// if the roadmap file has invalid frontmatter.
    pub fn remove_phase(&mut self, project: &str, roadmap: &str, phase_stem: &str) -> Result<()> {
        let path = self.phase_path(project, roadmap, phase_stem);
        if !self.store.exists(&path) {
            return Err(Error::PhaseNotFound(phase_stem.to_string()));
        }
        self.store.delete(&path)?;

        // Remove stem from roadmap's phases list
        let mut roadmap_doc = self.load_roadmap(project, roadmap)?;
        roadmap_doc.frontmatter.phases.retain(|s| s != phase_stem);
        self.write_roadmap(project, roadmap, &roadmap_doc)?;
        self.store.commit()?;
        Ok(())
    }

    /// Resolves a phase identifier to a file stem.
    ///
    /// If `identifier` parses as a `u32`, looks up the phase by number.
    /// Otherwise, returns `identifier` as-is for downstream validation.
    ///
    /// # Errors
    ///
    /// Returns [`Error::PhaseNotFound`] if `identifier` is numeric but no
    /// phase with that number exists. Also propagates errors from
    /// [`list_phases`].
    pub fn resolve_phase_stem(
        &self,
        project: &str,
        roadmap: &str,
        identifier: &str,
    ) -> Result<String> {
        if let Ok(num) = identifier.parse::<u32>() {
            let phases = self.list_phases(project, roadmap)?;
            for (stem, doc) in phases {
                if doc.frontmatter.phase == num {
                    return Ok(stem);
                }
            }
            return Err(Error::PhaseNotFound(identifier.to_string()));
        }
        Ok(identifier.to_string())
    }

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
    pub fn update_task(
        &mut self,
        project: &str,
        slug: &str,
        status: Option<TaskStatus>,
        priority: Option<Priority>,
        tags: Option<Vec<String>>,
        body: Option<&str>,
    ) -> Result<Document<Task>> {
        let path = self.task_path(project, slug);
        if !self.store.exists(&path) {
            return Err(Error::TaskNotFound(slug.to_string()));
        }

        let mut doc = self.load_task(project, slug)?;
        if let Some(s) = status {
            doc.frontmatter.status = s;
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
            },
            body: task_doc.body,
        };
        self.write_phase(project, roadmap_slug, &phase_slug, &phase_doc)?;

        self.store.delete(&task_path)?;
        self.store.commit()?;

        Ok(roadmap_doc)
    }

    // -- Dependency management --

    /// Adds a dependency from one roadmap to another.
    ///
    /// Appends `depends_on` to the `dependencies` list of the roadmap
    /// identified by `slug`. Validates that both roadmaps exist, the
    /// dependency is not already present, and adding it would not create
    /// a cycle.
    ///
    /// # Errors
    ///
    /// Returns [`Error::RoadmapNotFound`] if either roadmap doesn't exist,
    /// [`Error::CyclicDependency`] if adding the dependency would create a cycle,
    /// [`Error::Io`] if reading or writing fails, or
    /// [`Error::FrontmatterParse`] if frontmatter is invalid.
    pub fn add_dependency(
        &mut self,
        project: &str,
        slug: &str,
        depends_on: &str,
    ) -> Result<Document<Roadmap>> {
        // Verify both roadmaps exist
        let mut doc = self.load_roadmap(project, slug)?;
        let _target = self.load_roadmap(project, depends_on)?;

        // Check for self-dependency
        if slug == depends_on {
            return Err(Error::CyclicDependency(format!(
                "{slug} cannot depend on itself"
            )));
        }

        // Check for duplicate
        let deps = doc.frontmatter.dependencies.get_or_insert_with(Vec::new);
        if deps.contains(&depends_on.to_string()) {
            return Ok(doc);
        }

        // Check for cycles: build adjacency list, add proposed edge, then DFS
        let all_roadmaps = self.list_roadmaps(project)?;
        let mut adj: std::collections::HashMap<&str, Vec<&str>> = std::collections::HashMap::new();
        for rm in &all_roadmaps {
            let s = rm.frontmatter.roadmap.as_str();
            if let Some(ref d) = rm.frontmatter.dependencies {
                for dep in d {
                    adj.entry(s).or_default().push(dep.as_str());
                }
            }
        }
        // Add the proposed edge
        adj.entry(slug).or_default().push(depends_on);

        if Self::has_cycle(&adj, slug) {
            return Err(Error::CyclicDependency(format!(
                "adding {slug} → {depends_on} would create a cycle"
            )));
        }

        deps.push(depends_on.to_string());
        self.write_roadmap(project, slug, &doc)?;
        self.store.commit()?;
        Ok(doc)
    }

    /// Removes a dependency from a roadmap.
    ///
    /// Removes `depends_on` from the `dependencies` list. If the dependency
    /// is not present, this is a no-op.
    ///
    /// # Errors
    ///
    /// Returns [`Error::RoadmapNotFound`] if the roadmap doesn't exist,
    /// [`Error::Io`] if reading or writing fails, or
    /// [`Error::FrontmatterParse`] if frontmatter is invalid.
    pub fn remove_dependency(
        &mut self,
        project: &str,
        slug: &str,
        depends_on: &str,
    ) -> Result<Document<Roadmap>> {
        let mut doc = self.load_roadmap(project, slug)?;

        if let Some(ref mut deps) = doc.frontmatter.dependencies {
            deps.retain(|d| d != depends_on);
            if deps.is_empty() {
                doc.frontmatter.dependencies = None;
            }
        }

        self.write_roadmap(project, slug, &doc)?;
        self.store.commit()?;
        Ok(doc)
    }

    /// Returns the dependency graph for all roadmaps in a project.
    ///
    /// Each entry is `(roadmap_slug, vec_of_dependency_slugs)`.
    /// Only roadmaps with at least one dependency are included.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ProjectNotFound`] if the project doesn't exist,
    /// [`Error::Io`] if directory reads fail, or frontmatter errors if
    /// any roadmap file is malformed.
    pub fn dependency_graph(&self, project: &str) -> Result<Vec<(String, Vec<String>)>> {
        let roadmaps = self.list_roadmaps(project)?;
        let mut graph = Vec::new();
        for rm in roadmaps {
            if let Some(deps) = rm.frontmatter.dependencies
                && !deps.is_empty()
            {
                graph.push((rm.frontmatter.roadmap, deps));
            }
        }
        Ok(graph)
    }

    /// Detects whether `start` participates in a cycle in the adjacency list.
    fn has_cycle(adj: &std::collections::HashMap<&str, Vec<&str>>, start: &str) -> bool {
        let mut visited = std::collections::HashSet::new();
        let mut stack = vec![start];
        // Skip the start node on first visit — we want to detect if we
        // can reach `start` again by following edges.
        let mut is_first = true;
        while let Some(node) = stack.pop() {
            if !is_first && node == start {
                return true;
            }
            is_first = false;
            if visited.contains(node) {
                continue;
            }
            visited.insert(node);
            if let Some(neighbors) = adj.get(node) {
                for &n in neighbors {
                    stack.push(n);
                }
            }
        }
        false
    }

    /// Recursively deletes all files under a directory path in the store.
    fn delete_tree(&mut self, path: &RelPath) -> Result<()> {
        let entries = self.store.list(path)?;
        for entry in entries {
            let child = path.join(&entry.name).expect("valid path");
            match entry.kind {
                DirEntryKind::File => self.store.delete(&child)?,
                DirEntryKind::Dir => self.delete_tree(&child)?,
            }
        }
        Ok(())
    }

    /// Deletes a roadmap and all its phase files.
    ///
    /// Also removes this roadmap from the dependency lists of any other
    /// roadmaps in the same project.
    ///
    /// # Errors
    ///
    /// Returns [`Error::RoadmapNotFound`] if the roadmap doesn't exist,
    /// [`Error::Io`] if file removal or writes fail, or
    /// frontmatter errors if any roadmap file is malformed.
    pub fn delete_roadmap(&mut self, project: &str, slug: &str) -> Result<()> {
        let roadmap_file = self.roadmap_path(project, slug);
        if !self.store.exists(&roadmap_file) {
            return Err(Error::RoadmapNotFound(slug.to_string()));
        }

        // Remove this slug from dependency lists of all other roadmaps
        let roadmaps = self.list_roadmaps(project)?;
        for rm in roadmaps {
            if rm.frontmatter.roadmap == slug {
                continue;
            }
            if let Some(ref deps) = rm.frontmatter.dependencies
                && deps.contains(&slug.to_string())
            {
                self.remove_dependency(project, &rm.frontmatter.roadmap, slug)?;
            }
        }

        // Remove all files in the roadmap directory
        let dir = self.roadmap_dir(project, slug);
        self.delete_tree(&dir)?;
        self.store.commit()?;
        Ok(())
    }

    // -- Index generation --

    /// Generates `INDEX.md` from the current repo state.
    ///
    /// Scans all projects, roadmaps (with phase progress), and tasks,
    /// then writes a formatted index to the root `INDEX.md`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] if directory reads or the final write fail,
    /// or frontmatter errors if any document file is malformed.
    pub fn generate_index(&mut self) -> Result<()> {
        let project_names = self.list_projects()?;
        let mut project_indices = Vec::new();

        for project_name in &project_names {
            // Roadmaps
            let roadmap_docs = self.list_roadmaps(project_name)?;
            let mut roadmap_entries = Vec::new();
            for roadmap_doc in &roadmap_docs {
                let slug = &roadmap_doc.frontmatter.roadmap;
                let phases = self.list_phases(project_name, slug)?;
                let done_count = phases
                    .iter()
                    .filter(|(_, doc)| doc.frontmatter.status == PhaseStatus::Done)
                    .count();
                roadmap_entries.push(RoadmapIndexEntry {
                    slug: slug.clone(),
                    project: project_name.clone(),
                    phase_count: phases.len(),
                    done_count,
                    dependencies: roadmap_doc.frontmatter.dependencies.clone(),
                });
            }

            // Tasks — sorted by priority desc, then slug asc
            let mut tasks = self.list_tasks(project_name)?;
            tasks.sort_by(|(slug_a, doc_a), (slug_b, doc_b)| {
                doc_b
                    .frontmatter
                    .priority
                    .cmp(&doc_a.frontmatter.priority)
                    .then_with(|| slug_a.cmp(slug_b))
            });

            project_indices.push(ProjectIndex {
                name: project_name.clone(),
                roadmaps: roadmap_entries,
                tasks,
            });
        }

        let content = display::format_index(&project_indices);
        let index_path = self.index_path();
        self.store.write(&index_path, content)?;
        self.store.commit()?;
        Ok(())
    }

    // -- Init --

    /// Initializes a new plan repo with the given store.
    ///
    /// Creates `rdm.toml` and `INDEX.md`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::AlreadyInitialized`] if `rdm.toml` already exists, or
    /// [`Error::Io`] if file creation fails.
    pub fn init(store: S) -> Result<Self> {
        let mut repo = PlanRepo { store };

        if repo.store.exists(&repo.config_path()) {
            return Err(Error::AlreadyInitialized);
        }

        let config = Config::default();
        let toml_str = config.to_toml()?;
        let config_path = repo.config_path();
        repo.store.write(&config_path, toml_str)?;

        let index_path = repo.index_path();
        repo.store.write(
            &index_path,
            "# Plan Index\n\n<!-- This file is auto-generated by rdm. Do not edit by hand. -->\n"
                .to_string(),
        )?;

        repo.store.commit()?;
        Ok(repo)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;
    use crate::store::MemoryStore;
    use chrono::NaiveDate;

    fn make_repo() -> PlanRepo<MemoryStore> {
        PlanRepo::new(MemoryStore::new())
    }

    // -- Path builder tests --

    #[test]
    fn roadmap_path_is_correct() {
        let repo = make_repo();
        let path = repo.roadmap_path("fbm", "two-way-players");
        assert_eq!(
            path.as_str(),
            "projects/fbm/roadmaps/two-way-players/roadmap.md"
        );
    }

    #[test]
    fn phase_path_is_correct() {
        let repo = make_repo();
        let path = repo.phase_path("fbm", "two-way-players", "phase-1-core-valuation");
        assert_eq!(
            path.as_str(),
            "projects/fbm/roadmaps/two-way-players/phase-1-core-valuation.md"
        );
    }

    #[test]
    fn task_path_is_correct() {
        let repo = make_repo();
        let path = repo.task_path("fbm", "fix-barrel-nulls");
        assert_eq!(path.as_str(), "projects/fbm/tasks/fix-barrel-nulls.md");
    }

    // -- Write + Load round-trip tests --

    #[test]
    fn write_and_load_roadmap() {
        let mut repo = make_repo();
        let doc = Document {
            frontmatter: Roadmap {
                project: "fbm".to_string(),
                roadmap: "two-way-players".to_string(),
                title: "Two-Way Player Identity".to_string(),
                phases: vec![
                    "phase-1-core-valuation".to_string(),
                    "phase-2-keeper-service".to_string(),
                ],
                dependencies: Some(vec!["keeper-surplus-value".to_string()]),
            },
            body: "Summary here.\n".to_string(),
        };
        repo.write_roadmap("fbm", "two-way-players", &doc).unwrap();
        let loaded = repo.load_roadmap("fbm", "two-way-players").unwrap();
        assert_eq!(loaded.frontmatter, doc.frontmatter);
        assert_eq!(loaded.body, doc.body);
    }

    #[test]
    fn write_and_load_phase() {
        let mut repo = make_repo();
        let doc = Document {
            frontmatter: Phase {
                phase: 1,
                title: "Core valuation layer".to_string(),
                status: PhaseStatus::Done,
                completed: Some(NaiveDate::from_ymd_opt(2026, 3, 13).unwrap()),
            },
            body: "## Steps\n\n1. Do things.\n".to_string(),
        };
        repo.write_phase("fbm", "two-way-players", "phase-1-core-valuation", &doc)
            .unwrap();
        let loaded = repo
            .load_phase("fbm", "two-way-players", "phase-1-core-valuation")
            .unwrap();
        assert_eq!(loaded.frontmatter, doc.frontmatter);
        assert_eq!(loaded.body, doc.body);
    }

    #[test]
    fn write_and_load_task() {
        let mut repo = make_repo();
        let doc = Document {
            frontmatter: Task {
                project: "fbm".to_string(),
                title: "Fix barrel column".to_string(),
                status: TaskStatus::Open,
                priority: Priority::High,
                created: NaiveDate::from_ymd_opt(2026, 3, 14).unwrap(),
                tags: Some(vec!["data".to_string()]),
            },
            body: "Details.\n".to_string(),
        };
        repo.write_task("fbm", "fix-barrel-nulls", &doc).unwrap();
        let loaded = repo.load_task("fbm", "fix-barrel-nulls").unwrap();
        assert_eq!(loaded.frontmatter, doc.frontmatter);
        assert_eq!(loaded.body, doc.body);
    }

    #[test]
    fn load_project_success() {
        let mut repo = PlanRepo::init(MemoryStore::new()).unwrap();
        repo.create_project("fbm", "Fantasy Baseball Manager")
            .unwrap();
        let doc = repo.load_project("fbm").unwrap();
        assert_eq!(doc.frontmatter.name, "fbm");
        assert_eq!(doc.frontmatter.title, "Fantasy Baseball Manager");
    }

    #[test]
    fn load_project_not_found() {
        let repo = PlanRepo::init(MemoryStore::new()).unwrap();
        let result = repo.load_project("nonexistent");
        assert!(matches!(result, Err(Error::ProjectNotFound(ref s)) if s == "nonexistent"));
    }

    #[test]
    fn load_roadmap_not_found() {
        let repo = make_repo();
        let result = repo.load_roadmap("fbm", "nonexistent");
        assert!(matches!(result, Err(Error::RoadmapNotFound(ref s)) if s == "nonexistent"));
    }

    #[test]
    fn load_task_not_found() {
        let repo = make_repo();
        let result = repo.load_task("fbm", "does-not-exist");
        assert!(matches!(result, Err(Error::TaskNotFound(ref s)) if s == "does-not-exist"));
    }

    // -- Init tests --

    #[test]
    fn init_creates_structure() {
        let repo = PlanRepo::init(MemoryStore::new()).unwrap();

        assert!(repo.store().exists(&repo.config_path()));
        assert!(repo.store().exists(&repo.index_path()));

        // Config should be parseable
        let toml_str = repo.store().read(&repo.config_path()).unwrap();
        Config::from_toml(&toml_str).unwrap();
    }

    #[test]
    fn load_config_after_init() {
        let repo = PlanRepo::init(MemoryStore::new()).unwrap();
        let config = repo.load_config().unwrap();
        assert_eq!(config.default_project, None);
    }

    #[test]
    fn load_config_not_found() {
        let repo = make_repo();
        let result = repo.load_config();
        assert!(matches!(result, Err(Error::ConfigNotFound)));
    }

    // -- Project tests --

    #[test]
    fn create_project_success() {
        let mut repo = PlanRepo::init(MemoryStore::new()).unwrap();
        repo.create_project("fbm", "Fantasy Baseball Manager")
            .unwrap();

        assert!(repo.store().exists(&repo.project_md_path("fbm")));
    }

    #[test]
    fn create_project_duplicate() {
        let mut repo = PlanRepo::init(MemoryStore::new()).unwrap();
        repo.create_project("fbm", "Fantasy Baseball Manager")
            .unwrap();
        let result = repo.create_project("fbm", "Duplicate");
        assert!(matches!(result, Err(Error::DuplicateSlug(ref s)) if s == "fbm"));
    }

    #[test]
    fn list_projects_empty() {
        let repo = PlanRepo::init(MemoryStore::new()).unwrap();
        assert_eq!(repo.list_projects().unwrap(), Vec::<String>::new());
    }

    #[test]
    fn list_projects_sorted() {
        let mut repo = PlanRepo::init(MemoryStore::new()).unwrap();
        repo.create_project("zzz", "Last").unwrap();
        repo.create_project("aaa", "First").unwrap();
        repo.create_project("mmm", "Middle").unwrap();
        let projects = repo.list_projects().unwrap();
        assert_eq!(projects, vec!["aaa", "mmm", "zzz"]);
    }

    // -- Roadmap tests --

    #[test]
    fn create_roadmap_success() {
        let mut repo = PlanRepo::init(MemoryStore::new()).unwrap();
        repo.create_project("fbm", "FBM").unwrap();
        let doc = repo
            .create_roadmap("fbm", "two-way", "Two-Way Players", None)
            .unwrap();
        assert_eq!(doc.frontmatter.project, "fbm");
        assert_eq!(doc.frontmatter.roadmap, "two-way");
        assert_eq!(doc.frontmatter.title, "Two-Way Players");
        assert!(doc.frontmatter.phases.is_empty());

        // Should be loadable
        let loaded = repo.load_roadmap("fbm", "two-way").unwrap();
        assert_eq!(loaded.frontmatter, doc.frontmatter);
    }

    #[test]
    fn create_roadmap_with_body() {
        let mut repo = PlanRepo::init(MemoryStore::new()).unwrap();
        repo.create_project("fbm", "FBM").unwrap();
        let body = "# Description\n\nA roadmap for two-way players.\n";
        let doc = repo
            .create_roadmap("fbm", "two-way", "Two-Way Players", Some(body))
            .unwrap();
        assert_eq!(doc.body, body);

        let loaded = repo.load_roadmap("fbm", "two-way").unwrap();
        assert_eq!(loaded.body, body);
    }

    #[test]
    fn create_roadmap_project_not_found() {
        let mut repo = PlanRepo::init(MemoryStore::new()).unwrap();
        let result = repo.create_roadmap("nope", "slug", "Title", None);
        assert!(matches!(result, Err(Error::ProjectNotFound(_))));
    }

    #[test]
    fn create_roadmap_duplicate() {
        let mut repo = PlanRepo::init(MemoryStore::new()).unwrap();
        repo.create_project("fbm", "FBM").unwrap();
        repo.create_roadmap("fbm", "two-way", "Two-Way Players", None)
            .unwrap();
        let result = repo.create_roadmap("fbm", "two-way", "Dup", None);
        assert!(matches!(result, Err(Error::DuplicateSlug(_))));
    }

    #[test]
    fn update_roadmap_body_replaces_existing() {
        let mut repo = PlanRepo::init(MemoryStore::new()).unwrap();
        repo.create_project("fbm", "FBM").unwrap();
        repo.create_roadmap("fbm", "two-way", "Two-Way", Some("Original.\n"))
            .unwrap();
        let updated = repo
            .update_roadmap("fbm", "two-way", Some("Replaced.\n"))
            .unwrap();
        assert_eq!(updated.body, "Replaced.\n");

        let loaded = repo.load_roadmap("fbm", "two-way").unwrap();
        assert_eq!(loaded.body, "Replaced.\n");
    }

    #[test]
    fn update_roadmap_none_body_preserves_existing() {
        let mut repo = PlanRepo::init(MemoryStore::new()).unwrap();
        repo.create_project("fbm", "FBM").unwrap();
        repo.create_roadmap("fbm", "two-way", "Two-Way", Some("Keep this.\n"))
            .unwrap();
        let updated = repo.update_roadmap("fbm", "two-way", None).unwrap();
        assert_eq!(updated.body, "Keep this.\n");
    }

    #[test]
    fn update_roadmap_not_found() {
        let mut repo = PlanRepo::init(MemoryStore::new()).unwrap();
        repo.create_project("fbm", "FBM").unwrap();
        let result = repo.update_roadmap("fbm", "nope", Some("body"));
        assert!(matches!(result, Err(Error::RoadmapNotFound(_))));
    }

    #[test]
    fn list_roadmaps_sorted() {
        let mut repo = PlanRepo::init(MemoryStore::new()).unwrap();
        repo.create_project("fbm", "FBM").unwrap();
        repo.create_roadmap("fbm", "zzz-road", "Z", None).unwrap();
        repo.create_roadmap("fbm", "aaa-road", "A", None).unwrap();
        let roadmaps = repo.list_roadmaps("fbm").unwrap();
        assert_eq!(roadmaps.len(), 2);
        assert_eq!(roadmaps[0].frontmatter.roadmap, "aaa-road");
        assert_eq!(roadmaps[1].frontmatter.roadmap, "zzz-road");
    }

    #[test]
    fn list_roadmaps_empty() {
        let mut repo = PlanRepo::init(MemoryStore::new()).unwrap();
        repo.create_project("fbm", "FBM").unwrap();
        let roadmaps = repo.list_roadmaps("fbm").unwrap();
        assert!(roadmaps.is_empty());
    }

    #[test]
    fn list_roadmaps_project_not_found() {
        let repo = PlanRepo::init(MemoryStore::new()).unwrap();
        let result = repo.list_roadmaps("nope");
        assert!(matches!(result, Err(Error::ProjectNotFound(_))));
    }

    // -- Phase tests --

    fn setup_with_roadmap() -> PlanRepo<MemoryStore> {
        let mut repo = PlanRepo::init(MemoryStore::new()).unwrap();
        repo.create_project("fbm", "FBM").unwrap();
        repo.create_roadmap("fbm", "two-way", "Two-Way Players", None)
            .unwrap();
        repo
    }

    #[test]
    fn create_phase_auto_number() {
        let mut repo = setup_with_roadmap();
        let doc = repo
            .create_phase("fbm", "two-way", "core", "Core Valuation", None, None)
            .unwrap();
        assert_eq!(doc.frontmatter.phase, 1);
        assert_eq!(doc.frontmatter.status, PhaseStatus::NotStarted);

        let doc2 = repo
            .create_phase("fbm", "two-way", "service", "Keeper Service", None, None)
            .unwrap();
        assert_eq!(doc2.frontmatter.phase, 2);

        // Verify roadmap phases list was updated
        let roadmap = repo.load_roadmap("fbm", "two-way").unwrap();
        assert_eq!(
            roadmap.frontmatter.phases,
            vec!["phase-1-core", "phase-2-service"]
        );
    }

    #[test]
    fn create_phase_explicit_number() {
        let mut repo = setup_with_roadmap();
        let doc = repo
            .create_phase("fbm", "two-way", "core", "Core", Some(5), None)
            .unwrap();
        assert_eq!(doc.frontmatter.phase, 5);

        // Stem should be phase-5-core
        let loaded = repo.load_phase("fbm", "two-way", "phase-5-core").unwrap();
        assert_eq!(loaded.frontmatter, doc.frontmatter);
    }

    #[test]
    fn create_phase_with_body() {
        let mut repo = setup_with_roadmap();
        let body = "## Acceptance Criteria\n\n- [ ] Criterion one\n- [ ] Criterion two\n";
        let doc = repo
            .create_phase("fbm", "two-way", "core", "Core", None, Some(body))
            .unwrap();
        assert_eq!(doc.body, body);

        let loaded = repo.load_phase("fbm", "two-way", "phase-1-core").unwrap();
        assert_eq!(loaded.body, body);
    }

    #[test]
    fn create_phase_roadmap_not_found() {
        let mut repo = PlanRepo::init(MemoryStore::new()).unwrap();
        repo.create_project("fbm", "FBM").unwrap();
        let result = repo.create_phase("fbm", "nope", "s", "T", None, None);
        assert!(matches!(result, Err(Error::RoadmapNotFound(_))));
    }

    #[test]
    fn list_phases_sorted() {
        let mut repo = setup_with_roadmap();
        repo.create_phase("fbm", "two-way", "core", "Core", Some(2), None)
            .unwrap();
        repo.create_phase("fbm", "two-way", "service", "Service", Some(1), None)
            .unwrap();
        let phases = repo.list_phases("fbm", "two-way").unwrap();
        assert_eq!(phases.len(), 2);
        assert_eq!(phases[0].1.frontmatter.phase, 1);
        assert_eq!(phases[1].1.frontmatter.phase, 2);
    }

    #[test]
    fn update_phase_to_done_sets_completed() {
        let mut repo = setup_with_roadmap();
        repo.create_phase("fbm", "two-way", "core", "Core", None, None)
            .unwrap();
        let updated = repo
            .update_phase(
                "fbm",
                "two-way",
                "phase-1-core",
                Some(PhaseStatus::Done),
                None,
            )
            .unwrap();
        assert_eq!(updated.frontmatter.status, PhaseStatus::Done);
        assert!(updated.frontmatter.completed.is_some());
    }

    #[test]
    fn update_phase_from_done_clears_completed() {
        let mut repo = setup_with_roadmap();
        repo.create_phase("fbm", "two-way", "core", "Core", None, None)
            .unwrap();
        repo.update_phase(
            "fbm",
            "two-way",
            "phase-1-core",
            Some(PhaseStatus::Done),
            None,
        )
        .unwrap();
        let updated = repo
            .update_phase(
                "fbm",
                "two-way",
                "phase-1-core",
                Some(PhaseStatus::InProgress),
                None,
            )
            .unwrap();
        assert_eq!(updated.frontmatter.status, PhaseStatus::InProgress);
        assert_eq!(updated.frontmatter.completed, None);
    }

    #[test]
    fn update_phase_body_replaces_existing() {
        let mut repo = setup_with_roadmap();
        repo.create_phase(
            "fbm",
            "two-way",
            "core",
            "Core",
            None,
            Some("Original body.\n"),
        )
        .unwrap();
        let updated = repo
            .update_phase(
                "fbm",
                "two-way",
                "phase-1-core",
                Some(PhaseStatus::InProgress),
                Some("Replaced body.\n"),
            )
            .unwrap();
        assert_eq!(updated.body, "Replaced body.\n");

        let loaded = repo.load_phase("fbm", "two-way", "phase-1-core").unwrap();
        assert_eq!(loaded.body, "Replaced body.\n");
    }

    #[test]
    fn update_phase_none_body_preserves_existing() {
        let mut repo = setup_with_roadmap();
        repo.create_phase(
            "fbm",
            "two-way",
            "core",
            "Core",
            None,
            Some("Keep this body.\n"),
        )
        .unwrap();
        let updated = repo
            .update_phase(
                "fbm",
                "two-way",
                "phase-1-core",
                Some(PhaseStatus::InProgress),
                None,
            )
            .unwrap();
        assert_eq!(updated.body, "Keep this body.\n");
    }

    #[test]
    fn update_phase_not_found() {
        let mut repo = setup_with_roadmap();
        let result = repo.update_phase(
            "fbm",
            "two-way",
            "phase-99-nope",
            Some(PhaseStatus::Done),
            None,
        );
        assert!(matches!(result, Err(Error::PhaseNotFound(_))));
    }

    #[test]
    fn resolve_by_number() {
        let mut repo = setup_with_roadmap();
        repo.create_phase("fbm", "two-way", "core", "Core", Some(1), None)
            .unwrap();
        repo.create_phase("fbm", "two-way", "service", "Service", Some(2), None)
            .unwrap();
        let stem = repo.resolve_phase_stem("fbm", "two-way", "2").unwrap();
        assert_eq!(stem, "phase-2-service");
    }

    #[test]
    fn resolve_by_stem_passthrough() {
        let repo = setup_with_roadmap();
        let stem = repo
            .resolve_phase_stem("fbm", "two-way", "phase-1-core")
            .unwrap();
        assert_eq!(stem, "phase-1-core");
    }

    #[test]
    fn resolve_number_not_found() {
        let mut repo = setup_with_roadmap();
        repo.create_phase("fbm", "two-way", "core", "Core", Some(1), None)
            .unwrap();
        let result = repo.resolve_phase_stem("fbm", "two-way", "99");
        assert!(matches!(result, Err(Error::PhaseNotFound(ref s)) if s == "99"));
    }

    // -- Remove phase tests --

    #[test]
    fn remove_phase_deletes_file() {
        let mut repo = setup_with_roadmap();
        repo.create_phase("fbm", "two-way", "core", "Core", None, None)
            .unwrap();
        let path = repo.phase_path("fbm", "two-way", "phase-1-core");
        assert!(repo.store().exists(&path));

        repo.remove_phase("fbm", "two-way", "phase-1-core").unwrap();
        assert!(!repo.store().exists(&path));
    }

    #[test]
    fn remove_phase_updates_roadmap() {
        let mut repo = setup_with_roadmap();
        repo.create_phase("fbm", "two-way", "core", "Core", None, None)
            .unwrap();
        repo.create_phase("fbm", "two-way", "service", "Service", None, None)
            .unwrap();

        repo.remove_phase("fbm", "two-way", "phase-1-core").unwrap();

        let roadmap = repo.load_roadmap("fbm", "two-way").unwrap();
        assert_eq!(roadmap.frontmatter.phases, vec!["phase-2-service"]);
    }

    #[test]
    fn remove_phase_not_found() {
        let mut repo = setup_with_roadmap();
        let result = repo.remove_phase("fbm", "two-way", "phase-99-nope");
        assert!(matches!(result, Err(Error::PhaseNotFound(ref s)) if s == "phase-99-nope"));
    }

    // -- Task tests --

    fn setup_with_project() -> PlanRepo<MemoryStore> {
        let mut repo = PlanRepo::init(MemoryStore::new()).unwrap();
        repo.create_project("fbm", "FBM").unwrap();
        repo
    }

    #[test]
    fn create_task_success() {
        let mut repo = setup_with_project();
        let doc = repo
            .create_task("fbm", "fix-bug", "Fix the bug", Priority::High, None, None)
            .unwrap();
        assert_eq!(doc.frontmatter.title, "Fix the bug");
        assert_eq!(doc.frontmatter.status, TaskStatus::Open);
        assert_eq!(doc.frontmatter.priority, Priority::High);
        assert!(doc.frontmatter.tags.is_none());

        // Should be loadable
        let loaded = repo.load_task("fbm", "fix-bug").unwrap();
        assert_eq!(loaded.frontmatter, doc.frontmatter);
    }

    #[test]
    fn create_task_with_tags() {
        let mut repo = setup_with_project();
        let doc = repo
            .create_task(
                "fbm",
                "fix-bug",
                "Fix the bug",
                Priority::High,
                Some(vec!["bug".to_string(), "urgent".to_string()]),
                None,
            )
            .unwrap();
        assert_eq!(
            doc.frontmatter.tags,
            Some(vec!["bug".to_string(), "urgent".to_string()])
        );
    }

    #[test]
    fn create_task_with_body() {
        let mut repo = setup_with_project();
        let body = "## Notes\n\nSome detailed task notes.\n";
        let doc = repo
            .create_task("fbm", "fix-bug", "Fix", Priority::High, None, Some(body))
            .unwrap();
        assert_eq!(doc.body, body);

        let loaded = repo.load_task("fbm", "fix-bug").unwrap();
        assert_eq!(loaded.body, body);
    }

    #[test]
    fn create_task_project_not_found() {
        let mut repo = PlanRepo::init(MemoryStore::new()).unwrap();
        let result = repo.create_task("nope", "slug", "Title", Priority::Low, None, None);
        assert!(matches!(result, Err(Error::ProjectNotFound(_))));
    }

    #[test]
    fn create_task_duplicate() {
        let mut repo = setup_with_project();
        repo.create_task("fbm", "fix-bug", "Fix", Priority::Low, None, None)
            .unwrap();
        let result = repo.create_task("fbm", "fix-bug", "Dup", Priority::Low, None, None);
        assert!(matches!(result, Err(Error::DuplicateSlug(_))));
    }

    #[test]
    fn list_tasks_sorted() {
        let mut repo = setup_with_project();
        repo.create_task("fbm", "zzz-task", "Z", Priority::Low, None, None)
            .unwrap();
        repo.create_task("fbm", "aaa-task", "A", Priority::High, None, None)
            .unwrap();
        let tasks = repo.list_tasks("fbm").unwrap();
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].0, "aaa-task");
        assert_eq!(tasks[1].0, "zzz-task");
    }

    #[test]
    fn list_tasks_empty() {
        let repo = setup_with_project();
        let tasks = repo.list_tasks("fbm").unwrap();
        assert!(tasks.is_empty());
    }

    #[test]
    fn list_tasks_project_not_found() {
        let repo = PlanRepo::init(MemoryStore::new()).unwrap();
        let result = repo.list_tasks("nonexistent");
        assert!(matches!(result, Err(Error::ProjectNotFound(_))));
    }

    #[test]
    fn update_task_status() {
        let mut repo = setup_with_project();
        repo.create_task("fbm", "fix-bug", "Fix", Priority::Low, None, None)
            .unwrap();
        let updated = repo
            .update_task("fbm", "fix-bug", Some(TaskStatus::Done), None, None, None)
            .unwrap();
        assert_eq!(updated.frontmatter.status, TaskStatus::Done);

        let loaded = repo.load_task("fbm", "fix-bug").unwrap();
        assert_eq!(loaded.frontmatter.status, TaskStatus::Done);
    }

    #[test]
    fn update_task_priority() {
        let mut repo = setup_with_project();
        repo.create_task("fbm", "fix-bug", "Fix", Priority::Low, None, None)
            .unwrap();
        let updated = repo
            .update_task("fbm", "fix-bug", None, Some(Priority::Critical), None, None)
            .unwrap();
        assert_eq!(updated.frontmatter.priority, Priority::Critical);
    }

    #[test]
    fn update_task_tags() {
        let mut repo = setup_with_project();
        repo.create_task("fbm", "fix-bug", "Fix", Priority::Low, None, None)
            .unwrap();
        let updated = repo
            .update_task(
                "fbm",
                "fix-bug",
                None,
                None,
                Some(vec!["new-tag".to_string()]),
                None,
            )
            .unwrap();
        assert_eq!(updated.frontmatter.tags, Some(vec!["new-tag".to_string()]));
    }

    #[test]
    fn update_task_body_replaces_existing() {
        let mut repo = setup_with_project();
        repo.create_task(
            "fbm",
            "fix-bug",
            "Fix",
            Priority::Low,
            None,
            Some("Original.\n"),
        )
        .unwrap();
        let updated = repo
            .update_task("fbm", "fix-bug", None, None, None, Some("Replaced.\n"))
            .unwrap();
        assert_eq!(updated.body, "Replaced.\n");

        let loaded = repo.load_task("fbm", "fix-bug").unwrap();
        assert_eq!(loaded.body, "Replaced.\n");
    }

    #[test]
    fn update_task_none_body_preserves_existing() {
        let mut repo = setup_with_project();
        repo.create_task(
            "fbm",
            "fix-bug",
            "Fix",
            Priority::Low,
            None,
            Some("Keep this.\n"),
        )
        .unwrap();
        let updated = repo
            .update_task("fbm", "fix-bug", Some(TaskStatus::Done), None, None, None)
            .unwrap();
        assert_eq!(updated.body, "Keep this.\n");
    }

    #[test]
    fn update_task_not_found() {
        let mut repo = setup_with_project();
        let result = repo.update_task("fbm", "nope", Some(TaskStatus::Done), None, None, None);
        assert!(matches!(result, Err(Error::TaskNotFound(_))));
    }

    #[test]
    fn promote_task_to_roadmap() {
        let mut repo = setup_with_project();
        let task = Document {
            frontmatter: Task {
                project: "fbm".to_string(),
                title: "Big Feature".to_string(),
                status: TaskStatus::Open,
                priority: Priority::High,
                created: NaiveDate::from_ymd_opt(2026, 3, 15).unwrap(),
                tags: Some(vec!["infra".to_string()]),
            },
            body: "Task body content.\n".to_string(),
        };
        repo.write_task("fbm", "big-feature", &task).unwrap();

        let roadmap_doc = repo
            .promote_task("fbm", "big-feature", "big-feature-rm")
            .unwrap();
        assert_eq!(roadmap_doc.frontmatter.title, "Big Feature");
        assert_eq!(roadmap_doc.frontmatter.roadmap, "big-feature-rm");
        assert_eq!(roadmap_doc.frontmatter.phases, vec!["phase-1-big-feature"]);

        // Task file should be removed
        assert!(!repo.store().exists(&repo.task_path("fbm", "big-feature")));

        // Roadmap should preserve task metadata in body
        let loaded_rm = repo.load_roadmap("fbm", "big-feature-rm").unwrap();
        assert_eq!(loaded_rm.frontmatter.title, "Big Feature");
        assert!(loaded_rm.body.contains("priority: high"));
        assert!(loaded_rm.body.contains("created: 2026-03-15"));
        assert!(loaded_rm.body.contains("tags: infra"));

        let loaded_phase = repo
            .load_phase("fbm", "big-feature-rm", "phase-1-big-feature")
            .unwrap();
        assert_eq!(loaded_phase.frontmatter.title, "Big Feature");
        assert_eq!(loaded_phase.body, "Task body content.\n");
    }

    #[test]
    fn promote_task_not_found() {
        let mut repo = setup_with_project();
        let result = repo.promote_task("fbm", "nope", "rm-slug");
        assert!(matches!(result, Err(Error::TaskNotFound(_))));
    }

    #[test]
    fn promote_task_duplicate_roadmap() {
        let mut repo = setup_with_project();
        repo.create_task("fbm", "my-task", "Task", Priority::Low, None, None)
            .unwrap();
        repo.create_roadmap("fbm", "existing-rm", "Existing", None)
            .unwrap();
        let result = repo.promote_task("fbm", "my-task", "existing-rm");
        assert!(matches!(result, Err(Error::DuplicateSlug(_))));
    }

    // -- Dependency tests --

    #[test]
    fn add_dependency_success() {
        let mut repo = setup_with_project();
        repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();
        repo.create_roadmap("fbm", "beta", "Beta", None).unwrap();

        let doc = repo.add_dependency("fbm", "beta", "alpha").unwrap();
        assert_eq!(
            doc.frontmatter.dependencies,
            Some(vec!["alpha".to_string()])
        );

        // Verify persisted
        let loaded = repo.load_roadmap("fbm", "beta").unwrap();
        assert_eq!(
            loaded.frontmatter.dependencies,
            Some(vec!["alpha".to_string()])
        );
    }

    #[test]
    fn add_dependency_multiple() {
        let mut repo = setup_with_project();
        repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();
        repo.create_roadmap("fbm", "beta", "Beta", None).unwrap();
        repo.create_roadmap("fbm", "gamma", "Gamma", None).unwrap();

        repo.add_dependency("fbm", "gamma", "alpha").unwrap();
        let doc = repo.add_dependency("fbm", "gamma", "beta").unwrap();
        assert_eq!(
            doc.frontmatter.dependencies,
            Some(vec!["alpha".to_string(), "beta".to_string()])
        );
    }

    #[test]
    fn add_dependency_duplicate_is_noop() {
        let mut repo = setup_with_project();
        repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();
        repo.create_roadmap("fbm", "beta", "Beta", None).unwrap();

        repo.add_dependency("fbm", "beta", "alpha").unwrap();
        let doc = repo.add_dependency("fbm", "beta", "alpha").unwrap();
        assert_eq!(
            doc.frontmatter.dependencies,
            Some(vec!["alpha".to_string()])
        );
    }

    #[test]
    fn add_dependency_self_cycle() {
        let mut repo = setup_with_project();
        repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();

        let result = repo.add_dependency("fbm", "alpha", "alpha");
        assert!(matches!(result, Err(Error::CyclicDependency(_))));
    }

    #[test]
    fn add_dependency_direct_cycle() {
        let mut repo = setup_with_project();
        repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();
        repo.create_roadmap("fbm", "beta", "Beta", None).unwrap();

        repo.add_dependency("fbm", "beta", "alpha").unwrap();
        let result = repo.add_dependency("fbm", "alpha", "beta");
        assert!(matches!(result, Err(Error::CyclicDependency(_))));
    }

    #[test]
    fn add_dependency_transitive_cycle() {
        let mut repo = setup_with_project();
        repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();
        repo.create_roadmap("fbm", "beta", "Beta", None).unwrap();
        repo.create_roadmap("fbm", "gamma", "Gamma", None).unwrap();

        repo.add_dependency("fbm", "beta", "alpha").unwrap();
        repo.add_dependency("fbm", "gamma", "beta").unwrap();
        // gamma → beta → alpha, now alpha → gamma would create a cycle
        let result = repo.add_dependency("fbm", "alpha", "gamma");
        assert!(matches!(result, Err(Error::CyclicDependency(_))));
    }

    #[test]
    fn add_dependency_target_not_found() {
        let mut repo = setup_with_project();
        repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();

        let result = repo.add_dependency("fbm", "alpha", "nonexistent");
        assert!(matches!(result, Err(Error::RoadmapNotFound(_))));
    }

    #[test]
    fn add_dependency_source_not_found() {
        let mut repo = setup_with_project();
        repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();

        let result = repo.add_dependency("fbm", "nonexistent", "alpha");
        assert!(matches!(result, Err(Error::RoadmapNotFound(_))));
    }

    #[test]
    fn remove_dependency_success() {
        let mut repo = setup_with_project();
        repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();
        repo.create_roadmap("fbm", "beta", "Beta", None).unwrap();

        repo.add_dependency("fbm", "beta", "alpha").unwrap();
        let doc = repo.remove_dependency("fbm", "beta", "alpha").unwrap();
        assert_eq!(doc.frontmatter.dependencies, None);

        let loaded = repo.load_roadmap("fbm", "beta").unwrap();
        assert_eq!(loaded.frontmatter.dependencies, None);
    }

    #[test]
    fn remove_dependency_not_present_is_noop() {
        let mut repo = setup_with_project();
        repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();

        let doc = repo
            .remove_dependency("fbm", "alpha", "nonexistent")
            .unwrap();
        assert_eq!(doc.frontmatter.dependencies, None);
    }

    #[test]
    fn remove_dependency_preserves_others() {
        let mut repo = setup_with_project();
        repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();
        repo.create_roadmap("fbm", "beta", "Beta", None).unwrap();
        repo.create_roadmap("fbm", "gamma", "Gamma", None).unwrap();

        repo.add_dependency("fbm", "gamma", "alpha").unwrap();
        repo.add_dependency("fbm", "gamma", "beta").unwrap();
        let doc = repo.remove_dependency("fbm", "gamma", "alpha").unwrap();
        assert_eq!(doc.frontmatter.dependencies, Some(vec!["beta".to_string()]));
    }

    #[test]
    fn dependency_graph_returns_entries() {
        let mut repo = setup_with_project();
        repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();
        repo.create_roadmap("fbm", "beta", "Beta", None).unwrap();
        repo.create_roadmap("fbm", "gamma", "Gamma", None).unwrap();

        repo.add_dependency("fbm", "beta", "alpha").unwrap();
        repo.add_dependency("fbm", "gamma", "alpha").unwrap();
        repo.add_dependency("fbm", "gamma", "beta").unwrap();

        let graph = repo.dependency_graph("fbm").unwrap();
        assert_eq!(graph.len(), 2);
        // sorted by slug
        assert_eq!(graph[0].0, "beta");
        assert_eq!(graph[0].1, vec!["alpha"]);
        assert_eq!(graph[1].0, "gamma");
        assert_eq!(graph[1].1, vec!["alpha", "beta"]);
    }

    #[test]
    fn dependency_graph_empty() {
        let mut repo = setup_with_project();
        repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();
        let graph = repo.dependency_graph("fbm").unwrap();
        assert!(graph.is_empty());
    }

    // -- Delete roadmap tests --

    #[test]
    fn delete_roadmap_removes_files() {
        let mut repo = setup_with_project();
        repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();
        repo.create_phase("fbm", "alpha", "core", "Core", None, None)
            .unwrap();

        let roadmap_file = repo.roadmap_path("fbm", "alpha");
        assert!(repo.store().exists(&roadmap_file));

        repo.delete_roadmap("fbm", "alpha").unwrap();
        assert!(!repo.store().exists(&roadmap_file));
    }

    #[test]
    fn delete_roadmap_not_found() {
        let mut repo = setup_with_project();
        let result = repo.delete_roadmap("fbm", "nonexistent");
        assert!(matches!(result, Err(Error::RoadmapNotFound(ref s)) if s == "nonexistent"));
    }

    #[test]
    fn delete_roadmap_cleans_up_dependencies() {
        let mut repo = setup_with_project();
        repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();
        repo.create_roadmap("fbm", "beta", "Beta", None).unwrap();
        repo.create_roadmap("fbm", "gamma", "Gamma", None).unwrap();

        repo.add_dependency("fbm", "beta", "alpha").unwrap();
        repo.add_dependency("fbm", "gamma", "alpha").unwrap();
        repo.add_dependency("fbm", "gamma", "beta").unwrap();

        repo.delete_roadmap("fbm", "alpha").unwrap();

        // beta should have no dependencies left
        let beta = repo.load_roadmap("fbm", "beta").unwrap();
        assert_eq!(beta.frontmatter.dependencies, None);

        // gamma should still depend on beta but not alpha
        let gamma = repo.load_roadmap("fbm", "gamma").unwrap();
        assert_eq!(
            gamma.frontmatter.dependencies,
            Some(vec!["beta".to_string()])
        );
    }

    #[test]
    fn delete_roadmap_not_in_list() {
        let mut repo = setup_with_project();
        repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();
        repo.create_roadmap("fbm", "beta", "Beta", None).unwrap();

        repo.delete_roadmap("fbm", "alpha").unwrap();

        let roadmaps = repo.list_roadmaps("fbm").unwrap();
        let slugs: Vec<_> = roadmaps
            .iter()
            .map(|r| r.frontmatter.roadmap.as_str())
            .collect();
        assert_eq!(slugs, vec!["beta"]);
    }

    #[test]
    fn init_already_initialized() {
        let repo = PlanRepo::init(MemoryStore::new()).unwrap();
        let result = PlanRepo::init(repo.store.clone());
        assert!(matches!(result, Err(Error::AlreadyInitialized)));
    }

    // -- Index generation tests --

    #[test]
    fn generate_index_creates_file() {
        let mut repo = setup_with_project();
        repo.create_roadmap("fbm", "alpha", "Alpha Roadmap", None)
            .unwrap();
        repo.create_phase("fbm", "alpha", "core", "Core", None, None)
            .unwrap();
        repo.generate_index().unwrap();

        let content = repo.store().read(&repo.index_path()).unwrap();
        assert!(content.contains("# Plan Index"));
        assert!(content.contains("## Project: fbm"));
        assert!(content.contains("alpha"));
        assert!(content.contains("not started"));
    }

    #[test]
    fn generate_index_idempotent() {
        let mut repo = setup_with_project();
        repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();
        repo.generate_index().unwrap();
        let first = repo.store().read(&repo.index_path()).unwrap();
        repo.generate_index().unwrap();
        let second = repo.store().read(&repo.index_path()).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn generate_index_empty_repo() {
        let mut repo = PlanRepo::init(MemoryStore::new()).unwrap();
        repo.generate_index().unwrap();
        let content = repo.store().read(&repo.index_path()).unwrap();
        assert!(content.contains("# Plan Index"));
    }

    #[test]
    fn generate_index_task_priority_ordering() {
        let mut repo = setup_with_project();
        repo.create_task("fbm", "low-task", "Low", Priority::Low, None, None)
            .unwrap();
        repo.create_task(
            "fbm",
            "crit-task",
            "Critical",
            Priority::Critical,
            None,
            None,
        )
        .unwrap();
        repo.create_task("fbm", "high-task", "High", Priority::High, None, None)
            .unwrap();
        repo.generate_index().unwrap();

        let content = repo.store().read(&repo.index_path()).unwrap();
        let crit_pos = content.find("crit-task").unwrap();
        let high_pos = content.find("high-task").unwrap();
        let low_pos = content.find("low-task").unwrap();
        assert!(crit_pos < high_pos);
        assert!(high_pos < low_pos);
    }
}
