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

/// Returns the path to a project's directory.
pub fn project_path(project: &str) -> RelPath {
    RelPath::new(&format!("projects/{project}")).expect("valid path")
}

/// Names the plan item a store path holds, in the `<kind>/<id>` vocabulary the
/// CLI already uses everywhere else.
///
/// The inverse of the builders below, for the one situation that needs it: an
/// error message about a path. Users address items as `task/fix-bug`, not as
/// `projects/rdm/tasks/fix-bug.md`, and an error naming only the raw path
/// makes them translate it themselves.
///
/// The spellings are deliberately identical to
/// [`ReviewTarget::label`](crate::model::ReviewTarget::label) so the two
/// surfaces cannot drift into two vocabularies:
///
/// - `projects/<p>/tasks/<slug>.md` → `task/<slug>`
/// - `projects/<p>/roadmaps/<r>/roadmap.md` → `roadmap/<r>`
/// - `projects/<p>/roadmaps/<r>/<stem>.md` → `phase/<r>/<stem>`
/// - `projects/<p>/reviews/<id>.md` → `review/<id>`
///
/// Archived variants (`projects/<p>/archive/roadmaps/…`) map to the same
/// roadmap/phase spellings — an archived roadmap is still that roadmap.
/// Anything else returns the path unchanged, which is the honest answer for
/// `rdm.toml` or a file rdm does not own.
///
/// # Panics
///
/// Never. This function is **total** over arbitrary `&str` input, and that
/// totality is load-bearing: it is applied to paths that arrive from a
/// filesystem walk or a journal line — where a segment can be empty, `.`,
/// `..`, or contain a literal `\` (an ordinary filename character on Unix
/// that [`RelPath`] rejects) — while building the `ChangesetPathOverwritten`,
/// `ChangesetDeletePathRecreated` and `StaleWrite` error messages, so a panic
/// here would take out the very commands reporting the problem. It is total
/// by construction rather than by assertion: it only splits and slices the
/// input and never routes through an `expect`-ing [`RelPath`] builder, so an
/// unrecognized shape falls back to the raw path instead of panicking.
///
/// # Examples
///
/// ```
/// use rdm_core::paths::describe_path;
///
/// assert_eq!(describe_path("projects/rdm/tasks/fix-bug.md"), "task/fix-bug");
/// assert_eq!(describe_path("projects/rdm/roadmaps/auth/roadmap.md"), "roadmap/auth");
/// assert_eq!(
///     describe_path("projects/rdm/roadmaps/auth/phase-1-design.md"),
///     "phase/auth/phase-1-design"
/// );
/// assert_eq!(describe_path("rdm.toml"), "rdm.toml");
/// ```
#[must_use]
pub fn describe_path(path: &str) -> String {
    let segments: Vec<&str> = path.split('/').collect();
    // Every recognized shape is `projects/<project>/…`.
    if segments.len() < 4 || segments[0] != "projects" {
        return path.to_string();
    }
    // Skip an `archive` segment so an archived roadmap keeps its own name.
    let rest: &[&str] = if segments[2] == "archive" {
        &segments[3..]
    } else {
        &segments[2..]
    };
    let described = match rest {
        ["tasks", file] => file.strip_suffix(".md").map(|s| format!("task/{s}")),
        ["reviews", file] => file.strip_suffix(".md").map(|s| format!("review/{s}")),
        ["roadmaps", roadmap, "roadmap.md"] => Some(format!("roadmap/{roadmap}")),
        ["roadmaps", roadmap, file] => file
            .strip_suffix(".md")
            .map(|stem| format!("phase/{roadmap}/{stem}")),
        _ => None,
    };
    described.unwrap_or_else(|| path.to_string())
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
    fn project_path_is_correct() {
        assert_eq!(project_path("fbm").as_str(), "projects/fbm");
    }

    #[test]
    fn describe_path_uses_the_cli_item_vocabulary() {
        use crate::model::ReviewTarget;

        assert_eq!(
            describe_path(task_path("rdm", "fix-bug").as_str()),
            "task/fix-bug"
        );
        assert_eq!(
            describe_path(roadmap_path("rdm", "auth").as_str()),
            "roadmap/auth"
        );
        assert_eq!(
            describe_path(phase_path("rdm", "auth", "phase-1-design").as_str()),
            "phase/auth/phase-1-design"
        );
        assert_eq!(
            describe_path(review_path("rdm", "rv-abc123").as_str()),
            "review/rv-abc123"
        );

        // The spellings must match `ReviewTarget::label` exactly, or the CLI
        // ends up with two vocabularies for the same items.
        assert_eq!(
            describe_path(task_path("rdm", "fix-bug").as_str()),
            ReviewTarget::Task {
                slug: "fix-bug".to_string()
            }
            .label()
        );
        assert_eq!(
            describe_path(roadmap_path("rdm", "auth").as_str()),
            ReviewTarget::Roadmap {
                roadmap: "auth".to_string()
            }
            .label()
        );
        assert_eq!(
            describe_path(phase_path("rdm", "auth", "phase-1-design").as_str()),
            ReviewTarget::Phase {
                roadmap: "auth".to_string(),
                stem: "phase-1-design".to_string()
            }
            .label()
        );
    }

    #[test]
    fn describe_path_maps_archived_items_to_their_own_names() {
        assert_eq!(
            describe_path(archived_roadmap_path("rdm", "auth").as_str()),
            "roadmap/auth"
        );
        assert_eq!(
            describe_path(archived_phase_path("rdm", "auth", "phase-2-ship").as_str()),
            "phase/auth/phase-2-ship"
        );
    }

    #[test]
    fn describe_path_is_total_and_falls_back_to_the_raw_path() {
        // Files rdm does not own, and shapes it does not recognize, answer
        // with the path itself rather than panicking or inventing an item.
        for path in [
            "",
            "rdm.toml",
            ".gitattributes",
            "INDEX.md",
            "projects",
            "projects/rdm",
            "projects/rdm/INDEX.md",
            "projects/rdm/tasks/no-extension",
            "projects/rdm/tasks/nested/deep.md",
            "projects/rdm/roadmaps/auth/nested/deep.md",
            "projects/../rdm/tasks/x.md",
        ] {
            assert_eq!(describe_path(path), path, "unrecognized shape: {path:?}");
        }
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
