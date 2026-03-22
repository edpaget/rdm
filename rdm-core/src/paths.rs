//! Path builders for plan repo layout.
//!
//! These are pure functions that produce [`RelPath`] values for the
//! well-known locations inside a plan repo.  They have no dependency on
//! the store and can be used without a [`PlanRepo`](crate::repo::PlanRepo).

use crate::store::RelPath;

/// Returns the path to `rdm.toml`.
pub fn config_path() -> RelPath {
    RelPath::new("rdm.toml").expect("valid path")
}

/// Returns the path to `INDEX.md`.
pub fn index_path() -> RelPath {
    RelPath::new("INDEX.md").expect("valid path")
}

/// Returns the path to a project's directory.
pub fn project_path(project: &str) -> RelPath {
    RelPath::new(&format!("projects/{project}")).expect("valid path")
}

/// Returns the path to a project's `INDEX.md` file.
pub fn project_index_path(project: &str) -> RelPath {
    RelPath::new(&format!("projects/{project}/INDEX.md")).expect("valid path")
}

/// Returns the path to a project's `project.md` file.
pub(crate) fn project_md_path(project: &str) -> RelPath {
    RelPath::new(&format!("projects/{project}/project.md")).expect("valid path")
}

/// Returns the path to a project's roadmaps directory.
pub fn roadmaps_dir(project: &str) -> RelPath {
    RelPath::new(&format!("projects/{project}/roadmaps")).expect("valid path")
}

/// Returns the path to a specific roadmap directory.
pub fn roadmap_dir(project: &str, roadmap: &str) -> RelPath {
    RelPath::new(&format!("projects/{project}/roadmaps/{roadmap}")).expect("valid path")
}

/// Returns the path to a roadmap's `roadmap.md` file.
pub fn roadmap_path(project: &str, roadmap: &str) -> RelPath {
    RelPath::new(&format!("projects/{project}/roadmaps/{roadmap}/roadmap.md")).expect("valid path")
}

/// Returns the path to a phase file within a roadmap directory.
pub fn phase_path(project: &str, roadmap: &str, phase_stem: &str) -> RelPath {
    RelPath::new(&format!(
        "projects/{project}/roadmaps/{roadmap}/{phase_stem}.md"
    ))
    .expect("valid path")
}

/// Returns the path to a project's tasks directory.
pub fn tasks_dir(project: &str) -> RelPath {
    RelPath::new(&format!("projects/{project}/tasks")).expect("valid path")
}

/// Returns the path to a task file.
pub fn task_path(project: &str, task_slug: &str) -> RelPath {
    RelPath::new(&format!("projects/{project}/tasks/{task_slug}.md")).expect("valid path")
}

/// Returns the path to a project's archived roadmaps directory.
pub fn archived_roadmaps_dir(project: &str) -> RelPath {
    RelPath::new(&format!("projects/{project}/archive/roadmaps")).expect("valid path")
}

/// Returns the path to a specific archived roadmap directory.
pub fn archived_roadmap_dir(project: &str, roadmap: &str) -> RelPath {
    RelPath::new(&format!("projects/{project}/archive/roadmaps/{roadmap}")).expect("valid path")
}

/// Returns the path to an archived roadmap's `roadmap.md` file.
pub fn archived_roadmap_path(project: &str, roadmap: &str) -> RelPath {
    RelPath::new(&format!(
        "projects/{project}/archive/roadmaps/{roadmap}/roadmap.md"
    ))
    .expect("valid path")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_path_is_correct() {
        assert_eq!(config_path().as_str(), "rdm.toml");
    }

    #[test]
    fn index_path_is_correct() {
        assert_eq!(index_path().as_str(), "INDEX.md");
    }

    #[test]
    fn project_path_is_correct() {
        assert_eq!(project_path("fbm").as_str(), "projects/fbm");
    }

    #[test]
    fn project_index_path_is_correct() {
        assert_eq!(project_index_path("fbm").as_str(), "projects/fbm/INDEX.md");
    }

    #[test]
    fn project_md_path_is_correct() {
        assert_eq!(project_md_path("fbm").as_str(), "projects/fbm/project.md");
    }

    #[test]
    fn roadmaps_dir_is_correct() {
        assert_eq!(roadmaps_dir("fbm").as_str(), "projects/fbm/roadmaps");
    }

    #[test]
    fn roadmap_dir_is_correct() {
        assert_eq!(
            roadmap_dir("fbm", "two-way-players").as_str(),
            "projects/fbm/roadmaps/two-way-players"
        );
    }

    #[test]
    fn roadmap_path_is_correct() {
        assert_eq!(
            roadmap_path("fbm", "two-way-players").as_str(),
            "projects/fbm/roadmaps/two-way-players/roadmap.md"
        );
    }

    #[test]
    fn phase_path_is_correct() {
        assert_eq!(
            phase_path("fbm", "two-way-players", "phase-1-core-valuation").as_str(),
            "projects/fbm/roadmaps/two-way-players/phase-1-core-valuation.md"
        );
    }

    #[test]
    fn tasks_dir_is_correct() {
        assert_eq!(tasks_dir("fbm").as_str(), "projects/fbm/tasks");
    }

    #[test]
    fn task_path_is_correct() {
        assert_eq!(
            task_path("fbm", "fix-barrel-nulls").as_str(),
            "projects/fbm/tasks/fix-barrel-nulls.md"
        );
    }

    #[test]
    fn archived_roadmaps_dir_is_correct() {
        assert_eq!(
            archived_roadmaps_dir("fbm").as_str(),
            "projects/fbm/archive/roadmaps"
        );
    }

    #[test]
    fn archived_roadmap_dir_is_correct() {
        assert_eq!(
            archived_roadmap_dir("fbm", "alpha").as_str(),
            "projects/fbm/archive/roadmaps/alpha"
        );
    }

    #[test]
    fn archived_roadmap_path_is_correct() {
        assert_eq!(
            archived_roadmap_path("fbm", "alpha").as_str(),
            "projects/fbm/archive/roadmaps/alpha/roadmap.md"
        );
    }
}
