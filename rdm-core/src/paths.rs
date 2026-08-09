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
/// The check round-trips through the same construction those two builders use
/// rather than sniffing the string, so it cannot drift into matching paths the
/// generator never writes. In particular it is **not** a suffix match on
/// `INDEX.md`: the
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
/// # Panics
///
/// Never. This function is **total** over arbitrary `&str` input, which is
/// load-bearing: it is applied to every path a filesystem walk yields, so a
/// panic here would take out `rdm status`, `rdm commit`, `rdm discard`, the
/// post-command uncommitted hint, and the three MCP tools at once — including
/// `rdm discard`, the very command a user would reach for to remove an
/// offending path. It deliberately does **not** call [`project_index_path`],
/// whose `expect` would panic on a middle segment [`RelPath::new`] rejects
/// (empty, `.`, `..`, or a literal `\`, which is an ordinary filename
/// character on Unix); it builds the candidate through the fallible
/// [`RelPath::new`] instead, so every current *and future* `RelPath`
/// restriction is handled by construction rather than by re-enumerating those
/// rules here.
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
/// // Segments RelPath rejects are answered `false`, not a panic.
/// assert!(!is_derived_path(r"projects/a\b/INDEX.md"));
/// ```
pub fn is_derived_path(path: &str) -> bool {
    if path == index_path().as_str() {
        return true;
    }
    let segments: Vec<&str> = path.split('/').collect();
    if segments.len() == 3 && segments[0] == "projects" && segments[2] == "INDEX.md" {
        // Round-trip through the fallible `RelPath::new` rather than through
        // `project_index_path`, which `expect`s. `path` arrives from a
        // filesystem walk, so the middle segment is arbitrary: it can be
        // empty, `.`, `..`, or contain a literal `\` (a legal filename
        // character on Unix that `RelPath` rejects). Constructing the
        // candidate fallibly answers `false` for every such segment — and for
        // any rule `RelPath` gains later — instead of panicking.
        return RelPath::new(&format!("projects/{}/INDEX.md", segments[1]))
            .is_ok_and(|candidate| candidate.as_str() == path);
    }
    false
}

/// Reports whether a store path is a project's `project.md` manifest.
///
/// The manifest is the sentinel every `list_*` op checks before enumerating a
/// project (`list_roadmaps`, `list_tasks`, `list_reviews`): a `projects/<p>/`
/// subtree without one is not a project rdm can read. Exposed so a caller
/// deciding whether a project exists in some *projection* of the tree — such
/// as the commit-time seed in `rdm-store-git` — asks this question in one
/// place rather than re-spelling the `project.md` literal.
///
/// # Panics
///
/// Never. Like [`is_derived_path`], this is **total** over arbitrary `&str`:
/// it is applied to paths a HEAD tree walk yields, whose middle segment can be
/// empty, `.`, `..`, or contain a literal `\` (an ordinary filename character
/// on Unix). It deliberately does not route through the `expect`-ing
/// `project_md_path` builder; it round-trips the candidate through the
/// fallible [`RelPath::new`], so every current *and future* `RelPath`
/// restriction is handled by construction.
///
/// # Examples
///
/// ```
/// use rdm_core::paths::is_project_manifest;
///
/// assert!(is_project_manifest("projects/demo/project.md"));
/// // A derived index is not a manifest.
/// assert!(!is_project_manifest("projects/demo/INDEX.md"));
/// // A `project.md` nested deeper is not a project's manifest.
/// assert!(!is_project_manifest("projects/demo/roadmaps/a/project.md"));
/// // Segments RelPath rejects are answered `false`, not a panic.
/// assert!(!is_project_manifest(r"projects/a\b/project.md"));
/// ```
#[must_use]
pub fn is_project_manifest(path: &str) -> bool {
    let segments: Vec<&str> = path.split('/').collect();
    if segments.len() == 3 && segments[0] == "projects" && segments[2] == "project.md" {
        return RelPath::new(&format!("projects/{}/project.md", segments[1]))
            .is_ok_and(|candidate| candidate.as_str() == path);
    }
    false
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
/// Never. Like [`is_derived_path`], this is applied to arbitrary paths from a
/// filesystem walk or a journal line, so it is **total**: an unrecognized
/// shape falls back to the raw path rather than panicking.
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
    fn is_derived_path_does_not_panic_on_segments_relpath_rejects() {
        // A literal backslash is an ordinary filename character on Unix, so a
        // filesystem walk can hand this to `is_derived_path` — but `RelPath`
        // rejects it. Answering `false` (rather than panicking through
        // `project_index_path`'s `expect`) is what keeps `rdm status` /
        // `commit` / `discard` and the MCP tools alive on such a tree.
        assert!(!is_derived_path(r"projects/a\b/INDEX.md"));
        assert!(!is_derived_path("projects/a\\/INDEX.md"));
        assert!(!is_derived_path(r"projects/\/INDEX.md"));
        // A leading slash makes the whole path absolute, which `RelPath` also
        // rejects; the leading empty segment means this is not even 3
        // segments, but assert it is total here regardless.
        assert!(!is_derived_path("/projects/demo/INDEX.md"));
    }

    #[test]
    fn project_md_path_is_correct() {
        assert_eq!(project_md_path("fbm").as_str(), "projects/fbm/project.md");
    }

    /// Lockstep: the sentinel every `list_*` op checks and the sentinel the
    /// commit-time seed pruner checks must be the same path shape.
    #[test]
    fn is_project_manifest_matches_project_md_path() {
        assert!(is_project_manifest(project_md_path("fbm").as_str()));
        assert!(!is_project_manifest(project_index_path("fbm").as_str()));
        assert!(!is_project_manifest("projects/fbm"));
        assert!(!is_project_manifest("rdm.toml"));
        assert!(!is_project_manifest("project.md"));
        // Total over segments `RelPath` rejects.
        assert!(!is_project_manifest("projects//project.md"));
        assert!(!is_project_manifest("projects/../project.md"));
        assert!(!is_project_manifest(r"projects/a\b/project.md"));
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
