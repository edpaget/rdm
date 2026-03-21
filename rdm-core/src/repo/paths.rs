use crate::store::{RelPath, Store};

use super::PlanRepo;

impl<S: Store> PlanRepo<S> {
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

    /// Returns the path to a project's `INDEX.md` file.
    pub fn project_index_path(&self, project: &str) -> RelPath {
        RelPath::new(&format!("projects/{project}/INDEX.md")).expect("valid path")
    }

    /// Returns the path to a project's `project.md` file.
    pub(super) fn project_md_path(&self, project: &str) -> RelPath {
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

    /// Returns the path to a project's archived roadmaps directory.
    pub fn archived_roadmaps_dir(&self, project: &str) -> RelPath {
        RelPath::new(&format!("projects/{project}/archive/roadmaps")).expect("valid path")
    }

    /// Returns the path to a specific archived roadmap directory.
    pub fn archived_roadmap_dir(&self, project: &str, roadmap: &str) -> RelPath {
        RelPath::new(&format!("projects/{project}/archive/roadmaps/{roadmap}")).expect("valid path")
    }

    /// Returns the path to an archived roadmap's `roadmap.md` file.
    pub fn archived_roadmap_path(&self, project: &str, roadmap: &str) -> RelPath {
        RelPath::new(&format!(
            "projects/{project}/archive/roadmaps/{roadmap}/roadmap.md"
        ))
        .expect("valid path")
    }
}
