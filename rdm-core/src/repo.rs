/// Plan repo operations: path resolution, file I/O, and initialization.
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Local;

use crate::config::Config;
use crate::document::Document;
use crate::error::{Error, Result};
use crate::model::{Phase, PhaseStatus, Project, Roadmap, Task};

/// Represents an rdm plan repository on disk.
#[derive(Debug, Clone)]
pub struct PlanRepo {
    root: PathBuf,
}

impl PlanRepo {
    /// Opens an existing plan repo at the given root path.
    pub fn open(root: impl Into<PathBuf>) -> Self {
        PlanRepo { root: root.into() }
    }

    /// Returns the root path of the plan repo.
    pub fn root(&self) -> &Path {
        &self.root
    }

    // -- Path builders --

    /// Returns the path to `rdm.toml`.
    pub fn config_path(&self) -> PathBuf {
        self.root.join("rdm.toml")
    }

    /// Returns the path to `INDEX.md`.
    pub fn index_path(&self) -> PathBuf {
        self.root.join("INDEX.md")
    }

    /// Returns the path to a project's directory.
    pub fn project_path(&self, project: &str) -> PathBuf {
        self.root.join("projects").join(project)
    }

    /// Returns the path to a project's roadmaps directory.
    pub fn roadmaps_dir(&self, project: &str) -> PathBuf {
        self.project_path(project).join("roadmaps")
    }

    /// Returns the path to a specific roadmap directory.
    pub fn roadmap_dir(&self, project: &str, roadmap: &str) -> PathBuf {
        self.roadmaps_dir(project).join(roadmap)
    }

    /// Returns the path to a roadmap's `roadmap.md` file.
    pub fn roadmap_path(&self, project: &str, roadmap: &str) -> PathBuf {
        self.roadmap_dir(project, roadmap).join("roadmap.md")
    }

    /// Returns the path to a phase file within a roadmap directory.
    pub fn phase_path(&self, project: &str, roadmap: &str, phase_stem: &str) -> PathBuf {
        self.roadmap_dir(project, roadmap)
            .join(format!("{phase_stem}.md"))
    }

    /// Returns the path to a project's tasks directory.
    pub fn tasks_dir(&self, project: &str) -> PathBuf {
        self.project_path(project).join("tasks")
    }

    /// Returns the path to a task file.
    pub fn task_path(&self, project: &str, task_slug: &str) -> PathBuf {
        self.tasks_dir(project).join(format!("{task_slug}.md"))
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
        if !path.exists() {
            return Err(Error::ConfigNotFound);
        }
        let content = fs::read_to_string(path)?;
        Config::from_toml(&content)
    }

    /// Loads and parses a roadmap document from disk.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] if the file cannot be read,
    /// [`Error::FrontmatterMissing`] if delimiters are absent, or
    /// [`Error::FrontmatterParse`] if the YAML is invalid.
    pub fn load_roadmap(&self, project: &str, roadmap: &str) -> Result<Document<Roadmap>> {
        let content = fs::read_to_string(self.roadmap_path(project, roadmap))?;
        Document::parse(&content)
    }

    /// Loads and parses a phase document from disk.
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
        let content = fs::read_to_string(self.phase_path(project, roadmap, phase_stem))?;
        Document::parse(&content)
    }

    /// Loads and parses a task document from disk.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] if the file cannot be read,
    /// [`Error::FrontmatterMissing`] if delimiters are absent, or
    /// [`Error::FrontmatterParse`] if the YAML is invalid.
    pub fn load_task(&self, project: &str, task_slug: &str) -> Result<Document<Task>> {
        let content = fs::read_to_string(self.task_path(project, task_slug))?;
        Document::parse(&content)
    }

    // -- Write operations --

    /// Ensures parent directories exist for the given path.
    fn ensure_parent_dir(path: &Path) -> Result<()> {
        let parent = path.parent().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "path has no parent directory",
            )
        })?;
        fs::create_dir_all(parent)?;
        Ok(())
    }

    /// Writes a roadmap document to disk, creating parent directories as needed.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] if directory creation or file writing fails, or
    /// [`Error::FrontmatterParse`] if the frontmatter cannot be serialized.
    pub fn write_roadmap(
        &self,
        project: &str,
        roadmap: &str,
        doc: &Document<Roadmap>,
    ) -> Result<()> {
        let path = self.roadmap_path(project, roadmap);
        Self::ensure_parent_dir(&path)?;
        let content = doc.render()?;
        fs::write(path, content)?;
        Ok(())
    }

    /// Writes a phase document to disk, creating parent directories as needed.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] if directory creation or file writing fails, or
    /// [`Error::FrontmatterParse`] if the frontmatter cannot be serialized.
    pub fn write_phase(
        &self,
        project: &str,
        roadmap: &str,
        phase_stem: &str,
        doc: &Document<Phase>,
    ) -> Result<()> {
        let path = self.phase_path(project, roadmap, phase_stem);
        Self::ensure_parent_dir(&path)?;
        let content = doc.render()?;
        fs::write(path, content)?;
        Ok(())
    }

    /// Writes a task document to disk, creating parent directories as needed.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] if directory creation or file writing fails, or
    /// [`Error::FrontmatterParse`] if the frontmatter cannot be serialized.
    pub fn write_task(&self, project: &str, task_slug: &str, doc: &Document<Task>) -> Result<()> {
        let path = self.task_path(project, task_slug);
        Self::ensure_parent_dir(&path)?;
        let content = doc.render()?;
        fs::write(path, content)?;
        Ok(())
    }

    // -- Project operations --

    /// Creates a new project directory with `roadmaps/` and `tasks/` subdirectories.
    ///
    /// Returns `DuplicateSlug` if the project directory already exists.
    pub fn create_project(&self, name: &str, title: &str) -> Result<()> {
        let path = self.project_path(name);
        if path.exists() {
            return Err(Error::DuplicateSlug(name.to_string()));
        }
        fs::create_dir_all(self.roadmaps_dir(name))?;
        fs::create_dir_all(self.tasks_dir(name))?;

        let doc = Document {
            frontmatter: Project {
                name: name.to_string(),
                title: title.to_string(),
            },
            body: String::new(),
        };
        let content = doc.render()?;
        fs::write(path.join("project.md"), content)?;
        Ok(())
    }

    /// Lists all projects in the plan repo, sorted alphabetically.
    pub fn list_projects(&self) -> Result<Vec<String>> {
        let projects_dir = self.root.join("projects");
        if !projects_dir.exists() {
            return Ok(Vec::new());
        }
        let mut names: Vec<String> = fs::read_dir(projects_dir)?
            .filter_map(|entry| {
                let entry = entry.ok()?;
                if entry.file_type().ok()?.is_dir() {
                    entry.file_name().into_string().ok()
                } else {
                    None
                }
            })
            .collect();
        names.sort();
        Ok(names)
    }

    // -- Roadmap operations --

    /// Creates a new roadmap within a project.
    ///
    /// Returns `ProjectNotFound` if the project doesn't exist, or
    /// `DuplicateSlug` if the roadmap directory already exists.
    pub fn create_roadmap(
        &self,
        project: &str,
        slug: &str,
        title: &str,
    ) -> Result<Document<Roadmap>> {
        if !self.project_path(project).exists() {
            return Err(Error::ProjectNotFound(project.to_string()));
        }
        let dir = self.roadmap_dir(project, slug);
        if dir.exists() {
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
            body: String::new(),
        };
        self.write_roadmap(project, slug, &doc)?;
        Ok(doc)
    }

    /// Lists all roadmaps for a project, sorted by slug.
    ///
    /// Returns `ProjectNotFound` if the project doesn't exist.
    pub fn list_roadmaps(&self, project: &str) -> Result<Vec<Document<Roadmap>>> {
        if !self.project_path(project).exists() {
            return Err(Error::ProjectNotFound(project.to_string()));
        }
        let roadmaps_dir = self.roadmaps_dir(project);
        if !roadmaps_dir.exists() {
            return Ok(Vec::new());
        }
        let mut entries: Vec<String> = fs::read_dir(&roadmaps_dir)?
            .filter_map(|entry| {
                let entry = entry.ok()?;
                if entry.file_type().ok()?.is_dir() {
                    entry.file_name().into_string().ok()
                } else {
                    None
                }
            })
            .collect();
        entries.sort();

        let mut roadmaps = Vec::new();
        for slug in entries {
            let doc = self.load_roadmap(project, &slug)?;
            roadmaps.push(doc);
        }
        Ok(roadmaps)
    }

    // -- Phase operations --

    /// Lists all phases in a roadmap, sorted by phase number.
    ///
    /// Returns `(stem, Document<Phase>)` tuples.
    pub fn list_phases(
        &self,
        project: &str,
        roadmap: &str,
    ) -> Result<Vec<(String, Document<Phase>)>> {
        let dir = self.roadmap_dir(project, roadmap);
        if !dir.exists() {
            return Err(Error::RoadmapNotFound(roadmap.to_string()));
        }

        let mut phases: Vec<(String, Document<Phase>)> = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let name = entry.file_name().into_string().unwrap_or_default();
            if name == "roadmap.md" || !name.ends_with(".md") {
                continue;
            }
            let stem = name.trim_end_matches(".md").to_string();
            let doc = self.load_phase(project, roadmap, &stem)?;
            phases.push((stem, doc));
        }
        phases.sort_by_key(|(_, doc)| doc.frontmatter.phase);
        Ok(phases)
    }

    /// Creates a new phase within a roadmap.
    ///
    /// If `phase_number` is `None`, auto-assigns the next number.
    /// Returns `RoadmapNotFound` if the roadmap doesn't exist, or
    /// `DuplicateSlug` if a phase with the same stem already exists.
    pub fn create_phase(
        &self,
        project: &str,
        roadmap: &str,
        slug: &str,
        title: &str,
        phase_number: Option<u32>,
    ) -> Result<Document<Phase>> {
        let roadmap_dir = self.roadmap_dir(project, roadmap);
        if !roadmap_dir.exists() {
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

        let stem = format!("phase-{number}-{slug}");
        let path = self.phase_path(project, roadmap, &stem);
        if path.exists() {
            return Err(Error::DuplicateSlug(stem));
        }

        let doc = Document {
            frontmatter: Phase {
                phase: number,
                title: title.to_string(),
                status: PhaseStatus::NotStarted,
                completed: None,
            },
            body: String::new(),
        };
        self.write_phase(project, roadmap, &stem, &doc)?;

        // Update roadmap's phases list
        let mut roadmap_doc = self.load_roadmap(project, roadmap)?;
        roadmap_doc.frontmatter.phases.push(stem);
        self.write_roadmap(project, roadmap, &roadmap_doc)?;

        Ok(doc)
    }

    /// Updates a phase's status.
    ///
    /// When status is `Done`, auto-sets `completed` to today.
    /// When status is not `Done`, clears `completed`.
    pub fn update_phase(
        &self,
        project: &str,
        roadmap: &str,
        phase_stem: &str,
        status: PhaseStatus,
    ) -> Result<Document<Phase>> {
        let path = self.phase_path(project, roadmap, phase_stem);
        if !path.exists() {
            return Err(Error::PhaseNotFound(phase_stem.to_string()));
        }

        let mut doc = self.load_phase(project, roadmap, phase_stem)?;
        doc.frontmatter.status = status;
        doc.frontmatter.completed = if status == PhaseStatus::Done {
            Some(Local::now().date_naive())
        } else {
            None
        };
        self.write_phase(project, roadmap, phase_stem, &doc)?;
        Ok(doc)
    }

    // -- Init --

    /// Initializes a new plan repo at the configured root.
    ///
    /// Creates `rdm.toml`, `projects/`, and `INDEX.md`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::AlreadyInitialized`] if `rdm.toml` already exists, or
    /// [`Error::Io`] if directory or file creation fails.
    pub fn init(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        let repo = PlanRepo { root };

        if repo.config_path().exists() {
            return Err(Error::AlreadyInitialized);
        }

        fs::create_dir_all(repo.root())?;

        let config = Config::default();
        let toml_str = config
            .to_toml()
            .expect("default config serialization should not fail");
        fs::write(repo.config_path(), toml_str)?;

        fs::create_dir_all(repo.root().join("projects"))?;

        fs::write(
            repo.index_path(),
            "<!-- This file is auto-generated by rdm. Do not edit by hand. -->\n\n# Index\n",
        )?;

        Ok(repo)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;
    use chrono::NaiveDate;
    use tempfile::TempDir;

    fn make_repo() -> (TempDir, PlanRepo) {
        let dir = TempDir::new().unwrap();
        let repo = PlanRepo::open(dir.path());
        (dir, repo)
    }

    // -- Path builder tests --

    #[test]
    fn roadmap_path_is_correct() {
        let (_dir, repo) = make_repo();
        let path = repo.roadmap_path("fbm", "two-way-players");
        assert!(path.ends_with("projects/fbm/roadmaps/two-way-players/roadmap.md"));
    }

    #[test]
    fn phase_path_is_correct() {
        let (_dir, repo) = make_repo();
        let path = repo.phase_path("fbm", "two-way-players", "phase-1-core-valuation");
        assert!(path.ends_with("projects/fbm/roadmaps/two-way-players/phase-1-core-valuation.md"));
    }

    #[test]
    fn task_path_is_correct() {
        let (_dir, repo) = make_repo();
        let path = repo.task_path("fbm", "fix-barrel-nulls");
        assert!(path.ends_with("projects/fbm/tasks/fix-barrel-nulls.md"));
    }

    // -- Write + Load round-trip tests --

    #[test]
    fn write_and_load_roadmap() {
        let (_dir, repo) = make_repo();
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
        let (_dir, repo) = make_repo();
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
        let (_dir, repo) = make_repo();
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
    fn load_nonexistent_file_is_io_error() {
        let (_dir, repo) = make_repo();
        let result = repo.load_task("fbm", "does-not-exist");
        assert!(matches!(result, Err(Error::Io(_))));
    }

    // -- Init tests --

    #[test]
    fn init_creates_structure() {
        let dir = TempDir::new().unwrap();
        let repo = PlanRepo::init(dir.path()).unwrap();

        assert!(repo.config_path().exists());
        assert!(repo.root().join("projects").is_dir());
        assert!(repo.index_path().exists());

        // Config should be parseable
        let toml_str = fs::read_to_string(repo.config_path()).unwrap();
        Config::from_toml(&toml_str).unwrap();
    }

    #[test]
    fn load_config_after_init() {
        let dir = TempDir::new().unwrap();
        let repo = PlanRepo::init(dir.path()).unwrap();
        let config = repo.load_config().unwrap();
        assert_eq!(config.default_project, None);
    }

    #[test]
    fn load_config_not_found() {
        let dir = TempDir::new().unwrap();
        let repo = PlanRepo::open(dir.path());
        let result = repo.load_config();
        assert!(matches!(result, Err(Error::ConfigNotFound)));
    }

    // -- Project tests --

    #[test]
    fn create_project_success() {
        let dir = TempDir::new().unwrap();
        let repo = PlanRepo::init(dir.path()).unwrap();
        repo.create_project("fbm", "Fantasy Baseball Manager")
            .unwrap();

        assert!(repo.project_path("fbm").is_dir());
        assert!(repo.roadmaps_dir("fbm").is_dir());
        assert!(repo.tasks_dir("fbm").is_dir());
        assert!(repo.project_path("fbm").join("project.md").exists());
    }

    #[test]
    fn create_project_duplicate() {
        let dir = TempDir::new().unwrap();
        let repo = PlanRepo::init(dir.path()).unwrap();
        repo.create_project("fbm", "Fantasy Baseball Manager")
            .unwrap();
        let result = repo.create_project("fbm", "Duplicate");
        assert!(matches!(result, Err(Error::DuplicateSlug(ref s)) if s == "fbm"));
    }

    #[test]
    fn list_projects_empty() {
        let dir = TempDir::new().unwrap();
        let repo = PlanRepo::init(dir.path()).unwrap();
        assert_eq!(repo.list_projects().unwrap(), Vec::<String>::new());
    }

    #[test]
    fn list_projects_sorted() {
        let dir = TempDir::new().unwrap();
        let repo = PlanRepo::init(dir.path()).unwrap();
        repo.create_project("zzz", "Last").unwrap();
        repo.create_project("aaa", "First").unwrap();
        repo.create_project("mmm", "Middle").unwrap();
        let projects = repo.list_projects().unwrap();
        assert_eq!(projects, vec!["aaa", "mmm", "zzz"]);
    }

    // -- Roadmap tests --

    #[test]
    fn create_roadmap_success() {
        let dir = TempDir::new().unwrap();
        let repo = PlanRepo::init(dir.path()).unwrap();
        repo.create_project("fbm", "FBM").unwrap();
        let doc = repo
            .create_roadmap("fbm", "two-way", "Two-Way Players")
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
    fn create_roadmap_project_not_found() {
        let dir = TempDir::new().unwrap();
        let repo = PlanRepo::init(dir.path()).unwrap();
        let result = repo.create_roadmap("nope", "slug", "Title");
        assert!(matches!(result, Err(Error::ProjectNotFound(_))));
    }

    #[test]
    fn create_roadmap_duplicate() {
        let dir = TempDir::new().unwrap();
        let repo = PlanRepo::init(dir.path()).unwrap();
        repo.create_project("fbm", "FBM").unwrap();
        repo.create_roadmap("fbm", "two-way", "Two-Way Players")
            .unwrap();
        let result = repo.create_roadmap("fbm", "two-way", "Dup");
        assert!(matches!(result, Err(Error::DuplicateSlug(_))));
    }

    #[test]
    fn list_roadmaps_sorted() {
        let dir = TempDir::new().unwrap();
        let repo = PlanRepo::init(dir.path()).unwrap();
        repo.create_project("fbm", "FBM").unwrap();
        repo.create_roadmap("fbm", "zzz-road", "Z").unwrap();
        repo.create_roadmap("fbm", "aaa-road", "A").unwrap();
        let roadmaps = repo.list_roadmaps("fbm").unwrap();
        assert_eq!(roadmaps.len(), 2);
        assert_eq!(roadmaps[0].frontmatter.roadmap, "aaa-road");
        assert_eq!(roadmaps[1].frontmatter.roadmap, "zzz-road");
    }

    #[test]
    fn list_roadmaps_empty() {
        let dir = TempDir::new().unwrap();
        let repo = PlanRepo::init(dir.path()).unwrap();
        repo.create_project("fbm", "FBM").unwrap();
        let roadmaps = repo.list_roadmaps("fbm").unwrap();
        assert!(roadmaps.is_empty());
    }

    #[test]
    fn list_roadmaps_project_not_found() {
        let dir = TempDir::new().unwrap();
        let repo = PlanRepo::init(dir.path()).unwrap();
        let result = repo.list_roadmaps("nope");
        assert!(matches!(result, Err(Error::ProjectNotFound(_))));
    }

    // -- Phase tests --

    fn setup_with_roadmap() -> (TempDir, PlanRepo) {
        let dir = TempDir::new().unwrap();
        let repo = PlanRepo::init(dir.path()).unwrap();
        repo.create_project("fbm", "FBM").unwrap();
        repo.create_roadmap("fbm", "two-way", "Two-Way Players")
            .unwrap();
        (dir, repo)
    }

    #[test]
    fn create_phase_auto_number() {
        let (_dir, repo) = setup_with_roadmap();
        let doc = repo
            .create_phase("fbm", "two-way", "core", "Core Valuation", None)
            .unwrap();
        assert_eq!(doc.frontmatter.phase, 1);
        assert_eq!(doc.frontmatter.status, PhaseStatus::NotStarted);

        let doc2 = repo
            .create_phase("fbm", "two-way", "service", "Keeper Service", None)
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
        let (_dir, repo) = setup_with_roadmap();
        let doc = repo
            .create_phase("fbm", "two-way", "core", "Core", Some(5))
            .unwrap();
        assert_eq!(doc.frontmatter.phase, 5);

        // Stem should be phase-5-core
        let loaded = repo.load_phase("fbm", "two-way", "phase-5-core").unwrap();
        assert_eq!(loaded.frontmatter, doc.frontmatter);
    }

    #[test]
    fn create_phase_roadmap_not_found() {
        let dir = TempDir::new().unwrap();
        let repo = PlanRepo::init(dir.path()).unwrap();
        repo.create_project("fbm", "FBM").unwrap();
        let result = repo.create_phase("fbm", "nope", "s", "T", None);
        assert!(matches!(result, Err(Error::RoadmapNotFound(_))));
    }

    #[test]
    fn list_phases_sorted() {
        let (_dir, repo) = setup_with_roadmap();
        repo.create_phase("fbm", "two-way", "core", "Core", Some(2))
            .unwrap();
        repo.create_phase("fbm", "two-way", "service", "Service", Some(1))
            .unwrap();
        let phases = repo.list_phases("fbm", "two-way").unwrap();
        assert_eq!(phases.len(), 2);
        assert_eq!(phases[0].1.frontmatter.phase, 1);
        assert_eq!(phases[1].1.frontmatter.phase, 2);
    }

    #[test]
    fn update_phase_to_done_sets_completed() {
        let (_dir, repo) = setup_with_roadmap();
        repo.create_phase("fbm", "two-way", "core", "Core", None)
            .unwrap();
        let updated = repo
            .update_phase("fbm", "two-way", "phase-1-core", PhaseStatus::Done)
            .unwrap();
        assert_eq!(updated.frontmatter.status, PhaseStatus::Done);
        assert!(updated.frontmatter.completed.is_some());
    }

    #[test]
    fn update_phase_from_done_clears_completed() {
        let (_dir, repo) = setup_with_roadmap();
        repo.create_phase("fbm", "two-way", "core", "Core", None)
            .unwrap();
        repo.update_phase("fbm", "two-way", "phase-1-core", PhaseStatus::Done)
            .unwrap();
        let updated = repo
            .update_phase("fbm", "two-way", "phase-1-core", PhaseStatus::InProgress)
            .unwrap();
        assert_eq!(updated.frontmatter.status, PhaseStatus::InProgress);
        assert_eq!(updated.frontmatter.completed, None);
    }

    #[test]
    fn update_phase_not_found() {
        let (_dir, repo) = setup_with_roadmap();
        let result = repo.update_phase("fbm", "two-way", "phase-99-nope", PhaseStatus::Done);
        assert!(matches!(result, Err(Error::PhaseNotFound(_))));
    }

    #[test]
    fn init_already_initialized() {
        let dir = TempDir::new().unwrap();
        PlanRepo::init(dir.path()).unwrap();
        let result = PlanRepo::init(dir.path());
        assert!(matches!(result, Err(Error::AlreadyInitialized)));
    }
}
