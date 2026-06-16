//! Conflict classification for merge conflict paths.
//!
//! Maps file paths to rdm item types so conflict output can show
//! rdm-aware context (e.g., "Roadmap: my-roadmap" instead of a raw path).

use crate::paths;

/// The kind of rdm item a conflicted path belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictItemKind {
    /// A roadmap file (`projects/<proj>/roadmaps/<slug>/roadmap.md`).
    Roadmap,
    /// A phase file (`projects/<proj>/roadmaps/<slug>/<stem>.md`).
    Phase,
    /// A task file (`projects/<proj>/tasks/<slug>.md`).
    Task,
    /// A file that doesn't match any known rdm pattern.
    Other,
}

/// A classified conflict item with path and optional rdm context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictItem {
    /// The relative file path within the repository.
    pub path: String,
    /// The kind of rdm item this path represents.
    pub kind: ConflictItemKind,
    /// The project name, if the path is inside a `projects/<name>/` directory.
    pub project: Option<String>,
    /// The roadmap slug, if applicable.
    pub roadmap: Option<String>,
    /// The item slug or stem (e.g., task slug or phase stem).
    pub slug: Option<String>,
}

/// Classifies a repository-relative file path into an rdm item type.
///
/// # Examples
///
/// ```
/// use rdm_core::conflict::{classify_path, ConflictItemKind};
///
/// let item = classify_path("projects/myproj/roadmaps/auth/roadmap.md");
/// assert_eq!(item.kind, ConflictItemKind::Roadmap);
/// assert_eq!(item.project.as_deref(), Some("myproj"));
/// assert_eq!(item.roadmap.as_deref(), Some("auth"));
/// ```
pub fn classify_path(path: &str) -> ConflictItem {
    let segments: Vec<&str> = path.split('/').collect();

    // Roadmap: projects/<proj>/roadmaps/<roadmap>/roadmap.md
    //
    // Tried before Phase: `paths::phase_path(p, r, "roadmap")` reconstructs the
    // identical `roadmap.md` path, so a roadmap file would otherwise also satisfy
    // the phase round-trip below.
    if segments.len() == 5 && segments[4] == "roadmap.md" {
        let project = segments[1];
        let roadmap = segments[3];
        if paths::roadmap_path(project, roadmap).as_str() == path {
            return ConflictItem {
                path: path.to_string(),
                kind: ConflictItemKind::Roadmap,
                project: Some(project.to_string()),
                roadmap: Some(roadmap.to_string()),
                slug: Some(roadmap.to_string()),
            };
        }
    }

    // Phase: projects/<proj>/roadmaps/<roadmap>/<stem>.md
    if segments.len() == 5 && segments[4].ends_with(".md") && segments[4] != "roadmap.md" {
        let project = segments[1];
        let roadmap = segments[3];
        let stem = segments[4].strip_suffix(".md").unwrap_or(segments[4]);
        if paths::phase_path(project, roadmap, stem).as_str() == path {
            return ConflictItem {
                path: path.to_string(),
                kind: ConflictItemKind::Phase,
                project: Some(project.to_string()),
                roadmap: Some(roadmap.to_string()),
                slug: Some(stem.to_string()),
            };
        }
    }

    // Task: projects/<proj>/tasks/<slug>.md
    if segments.len() == 4 && segments[3].ends_with(".md") {
        let project = segments[1];
        let slug = segments[3].strip_suffix(".md").unwrap_or(segments[3]);
        if paths::task_path(project, slug).as_str() == path {
            return ConflictItem {
                path: path.to_string(),
                kind: ConflictItemKind::Task,
                project: Some(project.to_string()),
                roadmap: None,
                slug: Some(slug.to_string()),
            };
        }
    }

    ConflictItem {
        path: path.to_string(),
        kind: ConflictItemKind::Other,
        project: None,
        roadmap: None,
        slug: None,
    }
}

impl std::fmt::Display for ConflictItemKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConflictItemKind::Roadmap => write!(f, "Roadmap"),
            ConflictItemKind::Phase => write!(f, "Phase"),
            ConflictItemKind::Task => write!(f, "Task"),
            ConflictItemKind::Other => write!(f, "Other"),
        }
    }
}

impl std::fmt::Display for ConflictItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind {
            ConflictItemKind::Roadmap => {
                write!(
                    f,
                    "{} (project: {}, roadmap: {})",
                    self.kind,
                    self.project.as_deref().unwrap_or("?"),
                    self.slug.as_deref().unwrap_or("?"),
                )
            }
            ConflictItemKind::Phase => {
                write!(
                    f,
                    "{} (project: {}, roadmap: {}, phase: {})",
                    self.kind,
                    self.project.as_deref().unwrap_or("?"),
                    self.roadmap.as_deref().unwrap_or("?"),
                    self.slug.as_deref().unwrap_or("?"),
                )
            }
            ConflictItemKind::Task => {
                write!(
                    f,
                    "{} (project: {}, task: {})",
                    self.kind,
                    self.project.as_deref().unwrap_or("?"),
                    self.slug.as_deref().unwrap_or("?"),
                )
            }
            ConflictItemKind::Other => write!(f, "{}", self.path),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_roadmap_path() {
        let item = classify_path(paths::roadmap_path("myproj", "auth").as_str());
        assert_eq!(item.kind, ConflictItemKind::Roadmap);
        assert_eq!(item.project.as_deref(), Some("myproj"));
        assert_eq!(item.roadmap.as_deref(), Some("auth"));
        assert_eq!(item.slug.as_deref(), Some("auth"));
    }

    #[test]
    fn classify_phase_path() {
        let item = classify_path(paths::phase_path("myproj", "auth", "01-design").as_str());
        assert_eq!(item.kind, ConflictItemKind::Phase);
        assert_eq!(item.project.as_deref(), Some("myproj"));
        assert_eq!(item.roadmap.as_deref(), Some("auth"));
        assert_eq!(item.slug.as_deref(), Some("01-design"));
    }

    #[test]
    fn classify_task_path() {
        let item = classify_path(paths::task_path("myproj", "fix-login").as_str());
        assert_eq!(item.kind, ConflictItemKind::Task);
        assert_eq!(item.project.as_deref(), Some("myproj"));
        assert_eq!(item.roadmap, None);
        assert_eq!(item.slug.as_deref(), Some("fix-login"));
    }

    #[test]
    fn classify_other_path() {
        let item = classify_path("rdm.toml");
        assert_eq!(item.kind, ConflictItemKind::Other);
        assert_eq!(item.project, None);
        assert_eq!(item.roadmap, None);
        assert_eq!(item.slug, None);
    }

    #[test]
    fn roadmap_md_is_roadmap_not_phase() {
        // `paths::phase_path(p, r, "roadmap")` reconstructs the same `roadmap.md`
        // path as `paths::roadmap_path`, so the roadmap arm must win. This pins
        // the roadmap-before-phase ordering invariant.
        let item = classify_path(paths::phase_path("p", "r", "roadmap").as_str());
        assert_eq!(item.kind, ConflictItemKind::Roadmap);
        assert_eq!(item.roadmap.as_deref(), Some("r"));
    }

    #[test]
    fn classify_archived_roadmap_path() {
        // Archived roadmaps live under an `archive/` segment that no classify arm
        // reconstructs, so they fall through to `Other`.
        let item = classify_path(paths::archived_roadmap_path("p", "old").as_str());
        assert_eq!(item.kind, ConflictItemKind::Other);
        assert_eq!(item.project, None);
    }

    #[test]
    fn classify_index_file() {
        let item = classify_path("projects/myproj/INDEX.md");
        assert_eq!(item.kind, ConflictItemKind::Other);
        assert_eq!(item.project, None);
    }

    #[test]
    fn display_roadmap() {
        let item = classify_path(paths::roadmap_path("p", "r").as_str());
        assert_eq!(format!("{item}"), "Roadmap (project: p, roadmap: r)");
    }

    #[test]
    fn display_phase() {
        let item = classify_path(paths::phase_path("p", "r", "01-foo").as_str());
        assert_eq!(
            format!("{item}"),
            "Phase (project: p, roadmap: r, phase: 01-foo)"
        );
    }

    #[test]
    fn display_task() {
        let item = classify_path(paths::task_path("p", "fix-bug").as_str());
        assert_eq!(format!("{item}"), "Task (project: p, task: fix-bug)");
    }

    #[test]
    fn display_other() {
        let item = classify_path("README.md");
        assert_eq!(format!("{item}"), "README.md");
    }
}
