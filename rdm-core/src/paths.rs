//! Path builders for plan repo layout.
//!
//! These are pure functions that produce [`RelPath`] values for the
//! well-known locations inside a plan repo.  They have no dependency on
//! the store and can be used with any [`Store`] implementation.

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

/// Returns whether `path` is a file rdm *generates* rather than one a user
/// authored.
///
/// Membership is exactly the set
/// [`ops::index::generate_index`](crate::ops::index::generate_index) writes,
/// and nothing else:
///
/// - the root index, [`index_path`] — `INDEX.md`
/// - one per-project index per project, [`project_index_path`] —
///   `projects/<name>/INDEX.md`
///
/// The check round-trips through those two builders rather than sniffing the
/// string, so it cannot drift into matching paths the generator never writes.
/// In particular it is **not** a suffix match on `INDEX.md`: the
/// `**/INDEX.md merge=rdm-index` line rdm installs in `.gitattributes` is
/// deliberately broader than the generator's write set, and an `INDEX.md` a
/// user authored anywhere else in the tree (say
/// `projects/demo/roadmaps/auth/INDEX.md`) is a real user change that must
/// keep showing up in `rdm status`.
///
/// `.gitattributes` is deliberately **excluded**. rdm writes it on open, so it
/// looks derived, but it is a real, committable, hand-customizable file that
/// must be landed for the merge-driver mapping to travel with clones — hiding
/// a user's own `*.bin binary` line from `rdm status` would be a worse failure
/// than showing rdm's own one-time write.
///
/// # Invariant
///
/// This function must be kept in lockstep with `generate_index`'s write set.
/// If a future change makes the generator emit a third file (an archive
/// index, say), it must be added here in the same change — otherwise
/// `rdm status` will report generated output as a user change again.
///
/// # Examples
///
/// ```
/// use rdm_core::paths::is_derived_path;
///
/// assert!(is_derived_path("INDEX.md"));
/// assert!(is_derived_path("projects/demo/INDEX.md"));
/// // A user-authored INDEX.md elsewhere in the tree is not derived.
/// assert!(!is_derived_path("projects/demo/roadmaps/auth/INDEX.md"));
/// assert!(!is_derived_path(".gitattributes"));
/// ```
pub fn is_derived_path(path: &str) -> bool {
    if path == index_path().as_str() {
        return true;
    }
    let segments: Vec<&str> = path.split('/').collect();
    if segments.len() == 3 && segments[0] == "projects" && segments[2] == "INDEX.md" {
        let project = segments[1];
        // Guard the middle segment before round-tripping: `project_index_path`
        // panics on a component `RelPath` rejects, and these segments come
        // from an arbitrary caller-supplied string.
        if project.is_empty() || project == "." || project == ".." {
            return false;
        }
        return project_index_path(project).as_str() == path;
    }
    false
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

/// Returns the path to a project's reviews directory.
pub fn reviews_dir(project: &str) -> RelPath {
    RelPath::new(&format!("projects/{project}/reviews")).expect("valid path")
}

/// Returns the path to a review file.
pub fn review_path(project: &str, review_id: &str) -> RelPath {
    RelPath::new(&format!("projects/{project}/reviews/{review_id}.md")).expect("valid path")
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

/// Returns the path to a phase file within an archived roadmap directory.
pub fn archived_phase_path(project: &str, roadmap: &str, phase_stem: &str) -> RelPath {
    RelPath::new(&format!(
        "projects/{project}/archive/roadmaps/{roadmap}/{phase_stem}.md"
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
    fn is_derived_path_matches_only_generated_indexes() {
        // The exact write set of `ops::index::generate_index`.
        assert!(is_derived_path("INDEX.md"));
        assert!(is_derived_path("projects/demo/INDEX.md"));
        assert!(is_derived_path(project_index_path("fbm").as_str()));
        assert!(is_derived_path(index_path().as_str()));

        // Everything else is a user change, including INDEX.md files the
        // `**/INDEX.md` gitattributes line would match.
        assert!(!is_derived_path("projects/demo/roadmaps/r/INDEX.md"));
        assert!(!is_derived_path("projects/demo/tasks/INDEX.md"));
        assert!(!is_derived_path("projects/INDEX.md"));
        assert!(!is_derived_path("notes/INDEX.md"));
        assert!(!is_derived_path("INDEX.md.bak"));
        assert!(!is_derived_path("projects/demo/roadmap.md"));
        assert!(!is_derived_path("projects/demo/INDEX.md/nested.md"));
        assert!(!is_derived_path(".gitattributes"));
        assert!(!is_derived_path("rdm.toml"));
        assert!(!is_derived_path(""));
    }

    #[test]
    fn is_derived_path_does_not_panic_on_traversal_segments() {
        assert!(!is_derived_path("projects/../INDEX.md"));
        assert!(!is_derived_path("projects/./INDEX.md"));
        assert!(!is_derived_path("projects//INDEX.md"));
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
    fn reviews_dir_is_correct() {
        assert_eq!(reviews_dir("fbm").as_str(), "projects/fbm/reviews");
    }

    #[test]
    fn review_path_is_correct() {
        assert_eq!(
            review_path("fbm", "2026-07-01-1430-a1b2").as_str(),
            "projects/fbm/reviews/2026-07-01-1430-a1b2.md"
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

    #[test]
    fn archived_phase_path_is_correct() {
        assert_eq!(
            archived_phase_path("fbm", "alpha", "phase-1-one").as_str(),
            "projects/fbm/archive/roadmaps/alpha/phase-1-one.md"
        );
    }
}
