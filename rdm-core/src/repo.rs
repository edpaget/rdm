/// Plan repo operations: path resolution, file I/O, and initialization.
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::document::Document;
use crate::error::{Error, Result};
use crate::model::{Phase, Roadmap, Task};

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

    /// Loads and parses a roadmap document from disk.
    pub fn load_roadmap(&self, project: &str, roadmap: &str) -> Result<Document<Roadmap>> {
        let content = fs::read_to_string(self.roadmap_path(project, roadmap))?;
        Document::parse(&content)
    }

    /// Loads and parses a phase document from disk.
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
    pub fn load_task(&self, project: &str, task_slug: &str) -> Result<Document<Task>> {
        let content = fs::read_to_string(self.task_path(project, task_slug))?;
        Document::parse(&content)
    }

    // -- Write operations --

    /// Writes a roadmap document to disk, creating parent directories as needed.
    pub fn write_roadmap(
        &self,
        project: &str,
        roadmap: &str,
        doc: &Document<Roadmap>,
    ) -> Result<()> {
        let path = self.roadmap_path(project, roadmap);
        fs::create_dir_all(path.parent().unwrap())?;
        let content = doc.render()?;
        fs::write(path, content)?;
        Ok(())
    }

    /// Writes a phase document to disk, creating parent directories as needed.
    pub fn write_phase(
        &self,
        project: &str,
        roadmap: &str,
        phase_stem: &str,
        doc: &Document<Phase>,
    ) -> Result<()> {
        let path = self.phase_path(project, roadmap, phase_stem);
        fs::create_dir_all(path.parent().unwrap())?;
        let content = doc.render()?;
        fs::write(path, content)?;
        Ok(())
    }

    /// Writes a task document to disk, creating parent directories as needed.
    pub fn write_task(&self, project: &str, task_slug: &str, doc: &Document<Task>) -> Result<()> {
        let path = self.task_path(project, task_slug);
        fs::create_dir_all(path.parent().unwrap())?;
        let content = doc.render()?;
        fs::write(path, content)?;
        Ok(())
    }

    // -- Init --

    /// Initializes a new plan repo at the configured root.
    ///
    /// Creates `rdm.toml`, `projects/`, and `INDEX.md`.
    /// Returns an error if the repo is already initialized.
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
    fn init_already_initialized() {
        let dir = TempDir::new().unwrap();
        PlanRepo::init(dir.path()).unwrap();
        let result = PlanRepo::init(dir.path());
        assert!(matches!(result, Err(Error::AlreadyInitialized)));
    }
}
