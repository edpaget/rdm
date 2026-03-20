//! Conflict classification for merge conflict paths.
//!
//! Maps file paths to rdm item types so conflict output can show
//! rdm-aware context (e.g., "Roadmap: my-roadmap" instead of a raw path).

/// The kind of rdm item a conflicted path belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictItemKind {
    /// A roadmap file (`projects/<proj>/roadmaps/<slug>/roadmap.md`).
    Roadmap,
    /// A phase file (`projects/<proj>/roadmaps/<slug>/phases/<stem>.md`).
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

    // projects/<proj>/roadmaps/<slug>/roadmap.md
    if segments.len() == 5
        && segments[0] == "projects"
        && segments[2] == "roadmaps"
        && segments[4] == "roadmap.md"
    {
        return ConflictItem {
            path: path.to_string(),
            kind: ConflictItemKind::Roadmap,
            project: Some(segments[1].to_string()),
            roadmap: Some(segments[3].to_string()),
            slug: Some(segments[3].to_string()),
        };
    }

    // projects/<proj>/roadmaps/<slug>/phases/<stem>.md
    if segments.len() == 6
        && segments[0] == "projects"
        && segments[2] == "roadmaps"
        && segments[4] == "phases"
        && segments[5].ends_with(".md")
    {
        let stem = segments[5].strip_suffix(".md").unwrap_or(segments[5]);
        return ConflictItem {
            path: path.to_string(),
            kind: ConflictItemKind::Phase,
            project: Some(segments[1].to_string()),
            roadmap: Some(segments[3].to_string()),
            slug: Some(stem.to_string()),
        };
    }

    // projects/<proj>/tasks/<slug>.md
    if segments.len() == 4
        && segments[0] == "projects"
        && segments[2] == "tasks"
        && segments[3].ends_with(".md")
    {
        let stem = segments[3].strip_suffix(".md").unwrap_or(segments[3]);
        return ConflictItem {
            path: path.to_string(),
            kind: ConflictItemKind::Task,
            project: Some(segments[1].to_string()),
            roadmap: None,
            slug: Some(stem.to_string()),
        };
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
        let item = classify_path("projects/myproj/roadmaps/auth/roadmap.md");
        assert_eq!(item.kind, ConflictItemKind::Roadmap);
        assert_eq!(item.project.as_deref(), Some("myproj"));
        assert_eq!(item.roadmap.as_deref(), Some("auth"));
        assert_eq!(item.slug.as_deref(), Some("auth"));
    }

    #[test]
    fn classify_phase_path() {
        let item = classify_path("projects/myproj/roadmaps/auth/phases/01-design.md");
        assert_eq!(item.kind, ConflictItemKind::Phase);
        assert_eq!(item.project.as_deref(), Some("myproj"));
        assert_eq!(item.roadmap.as_deref(), Some("auth"));
        assert_eq!(item.slug.as_deref(), Some("01-design"));
    }

    #[test]
    fn classify_task_path() {
        let item = classify_path("projects/myproj/tasks/fix-login.md");
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
    fn classify_index_file() {
        let item = classify_path("projects/myproj/INDEX.md");
        assert_eq!(item.kind, ConflictItemKind::Other);
        assert_eq!(item.project, None);
    }

    #[test]
    fn display_roadmap() {
        let item = classify_path("projects/p/roadmaps/r/roadmap.md");
        assert_eq!(format!("{item}"), "Roadmap (project: p, roadmap: r)");
    }

    #[test]
    fn display_phase() {
        let item = classify_path("projects/p/roadmaps/r/phases/01-foo.md");
        assert_eq!(
            format!("{item}"),
            "Phase (project: p, roadmap: r, phase: 01-foo)"
        );
    }

    #[test]
    fn display_task() {
        let item = classify_path("projects/p/tasks/fix-bug.md");
        assert_eq!(format!("{item}"), "Task (project: p, task: fix-bug)");
    }

    #[test]
    fn display_other() {
        let item = classify_path("README.md");
        assert_eq!(format!("{item}"), "README.md");
    }
}
