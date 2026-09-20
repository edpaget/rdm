//! Project operations.

use crate::document::Document;
use crate::error::{Error, Result};
use crate::model::Project;
use crate::store::{DirEntryKind, RelPath, Store};

/// Creates a new project with `roadmaps/` and `tasks/` subdirectories.
///
/// # Errors
///
/// Returns [`Error::DuplicateSlug`] if the project already exists,
/// [`Error::Io`] if file creation fails, or
/// [`Error::FrontmatterParse`] if frontmatter serialization fails.
pub fn create_project(
    store: &mut impl Store,
    name: &str,
    title: &str,
) -> Result<Document<Project>> {
    let md_path = crate::paths::project_md_path(name);
    if store.exists(&md_path) {
        return Err(Error::DuplicateSlug(name.to_string()));
    }

    let doc = Document {
        frontmatter: Project {
            name: name.to_string(),
            title: title.to_string(),
            source: None,
        },
        body: String::new(),
    };
    let content = doc.render()?;
    store.write(&md_path, content)?;
    Ok(doc)
}

/// Lists all projects in the plan repo, sorted alphabetically.
///
/// # Errors
///
/// Returns [`Error::Io`] if the projects directory cannot be read.
pub fn list_projects(store: &impl Store) -> Result<Vec<String>> {
    let projects_dir = RelPath::new("projects").expect("valid path");
    let entries = store.list(&projects_dir)?;
    let mut names: Vec<String> = entries
        .into_iter()
        .filter(|e| e.kind == DirEntryKind::Dir)
        .map(|e| e.name)
        .collect();
    names.sort();
    Ok(names)
}

/// How an update should treat a project's configured [`Source`].
///
/// This is the `from_args`/`apply` protocol [`crate::ops::update`] uses for
/// bodies, titles and tags, specialized for the two-field
/// repo-plus-default-branch shape: every frontend (CLI, server, a future TUI)
/// hands its raw flags to [`SourceUpdate::from_args`] and the merge rules —
/// "keep the configured repo when only a branch is given", "keep the configured
/// branch when only a repo is given", "a branch needs a repo to belong to" —
/// live here in core rather than in each frontend's command handler.
///
/// [`Source`]: crate::model::Source
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceUpdate {
    /// Merge these fields into the project's existing source, keeping whichever
    /// half is `None`. At least one half is always `Some` — [`from_args`]
    /// rejects an update that names nothing.
    ///
    /// [`from_args`]: SourceUpdate::from_args
    Merge {
        /// Repository locator (a clone URL or a filesystem path). `None` keeps
        /// the configured repo, and requires that one is already configured.
        repo: Option<String>,
        /// Branch `rdm:src/` links resolve against when no `@<rev>` is given.
        /// `None` keeps the configured branch.
        default_branch: Option<String>,
    },
    /// Remove any configured source, confirming the clobber.
    Clear,
}

impl SourceUpdate {
    /// Builds a [`SourceUpdate`] from a frontend's raw `--source-repo` /
    /// `--source-branch` / `--clear-source` arguments.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ConflictingUpdate`] if `clear` is set alongside either
    /// value, or [`Error::ProjectSourceUpdateEmpty`] if the update names
    /// nothing at all — an empty update is a mistake to report, not a silent
    /// no-op write.
    pub fn from_args(
        project: &str,
        repo: Option<String>,
        default_branch: Option<String>,
        clear: bool,
    ) -> Result<Self> {
        match (repo, default_branch, clear) {
            (repo, default_branch, true) if repo.is_some() || default_branch.is_some() => {
                Err(Error::ConflictingUpdate {
                    field: "source".to_string(),
                })
            }
            (_, _, true) => Ok(SourceUpdate::Clear),
            (None, None, false) => Err(Error::ProjectSourceUpdateEmpty(project.to_string())),
            (repo, default_branch, false) => Ok(SourceUpdate::Merge {
                repo,
                default_branch,
            }),
        }
    }

    /// Resolves this update against a project's currently configured source.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ProjectSourceRepoMissing`] when a [`Merge`] supplies
    /// only a default branch and the project has no configured repository for
    /// that branch to belong to.
    ///
    /// [`Merge`]: SourceUpdate::Merge
    fn apply(
        self,
        project: &str,
        existing: Option<crate::model::Source>,
    ) -> Result<Option<crate::model::Source>> {
        match self {
            SourceUpdate::Clear => Ok(None),
            SourceUpdate::Merge {
                repo,
                default_branch,
            } => {
                let existing_repo = existing.as_ref().map(|s| s.repo.clone());
                let repo = repo
                    .or(existing_repo)
                    .ok_or_else(|| Error::ProjectSourceRepoMissing(project.to_string()))?;
                let default_branch =
                    default_branch.or_else(|| existing.and_then(|s| s.default_branch));
                Ok(Some(crate::model::Source {
                    repo,
                    default_branch,
                }))
            }
        }
    }
}

/// Sets or clears the code repository a project's `rdm:src/` links and
/// `change/<ref>` reviews resolve against.
///
/// The `update` carries the frontend's intent; the merge against whatever the
/// project already has is resolved here in core, so the CLI, the server and any
/// later interface all get the same partial-update semantics. This is the only
/// writer of [`Project::source`] — it exists so configuring a project's source
/// repository is a CLI act rather than a hand edit of the plan repo.
///
/// # Errors
///
/// Returns [`Error::ProjectNotFound`] if the project does not exist,
/// [`Error::ProjectSourceRepoMissing`] if only a default branch was supplied
/// for a project with no configured repository, [`Error::Io`] if the write
/// fails, or [`Error::FrontmatterParse`] if frontmatter serialization fails.
pub fn update_project_source(
    store: &mut impl Store,
    name: &str,
    update: SourceUpdate,
) -> Result<Document<Project>> {
    let mut doc = crate::io::load_project(store, name)?;
    doc.frontmatter.source = update.apply(name, doc.frontmatter.source.take())?;
    let content = doc.render()?;
    store.write(&crate::paths::project_md_path(name), content)?;
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Source;
    use crate::store::MemoryStore;

    fn store_with_project() -> MemoryStore {
        let mut store = MemoryStore::new();
        create_project(&mut store, "rdm", "rdm").unwrap();
        store
    }

    fn merge(repo: Option<&str>, branch: Option<&str>) -> SourceUpdate {
        SourceUpdate::from_args(
            "rdm",
            repo.map(str::to_string),
            branch.map(str::to_string),
            false,
        )
        .unwrap()
    }

    #[test]
    fn from_args_rejects_clear_alongside_a_value() {
        for (repo, branch) in [(Some("/repo"), None), (None, Some("main"))] {
            let err = SourceUpdate::from_args(
                "rdm",
                repo.map(str::to_string),
                branch.map(str::to_string),
                true,
            )
            .unwrap_err();
            assert!(matches!(err, Error::ConflictingUpdate { field } if field == "source"));
        }
    }

    #[test]
    fn from_args_rejects_an_update_that_names_nothing() {
        let err = SourceUpdate::from_args("rdm", None, None, false).unwrap_err();
        assert!(matches!(err, Error::ProjectSourceUpdateEmpty(name) if name == "rdm"));
    }

    #[test]
    fn from_args_maps_the_three_accepted_shapes() {
        assert_eq!(
            SourceUpdate::from_args("rdm", None, None, true).unwrap(),
            SourceUpdate::Clear
        );
        assert_eq!(
            merge(Some("/repo"), None),
            SourceUpdate::Merge {
                repo: Some("/repo".to_string()),
                default_branch: None,
            }
        );
        assert_eq!(
            merge(Some("/repo"), Some("main")),
            SourceUpdate::Merge {
                repo: Some("/repo".to_string()),
                default_branch: Some("main".to_string()),
            }
        );
    }

    #[test]
    fn update_project_source_sets_repo_and_branch() {
        let mut store = store_with_project();
        let doc = update_project_source(
            &mut store,
            "rdm",
            merge(Some("/Users/ed/Projects/rdm"), Some("main")),
        )
        .unwrap();
        assert_eq!(
            doc.frontmatter.source,
            Some(Source {
                repo: "/Users/ed/Projects/rdm".to_string(),
                default_branch: Some("main".to_string()),
            })
        );
        // Persisted, not just returned.
        let reloaded = crate::io::load_project(&store, "rdm").unwrap();
        assert_eq!(reloaded.frontmatter.source, doc.frontmatter.source);
    }

    #[test]
    fn update_project_source_branch_only_keeps_the_configured_repo() {
        let mut store = store_with_project();
        update_project_source(&mut store, "rdm", merge(Some("/repo"), None)).unwrap();
        let doc = update_project_source(&mut store, "rdm", merge(None, Some("trunk"))).unwrap();
        let source = doc.frontmatter.source.unwrap();
        assert_eq!(source.repo, "/repo");
        assert_eq!(source.default_branch.as_deref(), Some("trunk"));
    }

    #[test]
    fn update_project_source_repo_only_keeps_the_configured_branch() {
        let mut store = store_with_project();
        update_project_source(&mut store, "rdm", merge(Some("/repo"), Some("main"))).unwrap();
        let doc = update_project_source(&mut store, "rdm", merge(Some("/other"), None)).unwrap();
        let source = doc.frontmatter.source.unwrap();
        assert_eq!(source.repo, "/other");
        // The mirror of the branch-only case: the half that was not named survives.
        assert_eq!(source.default_branch.as_deref(), Some("main"));
    }

    #[test]
    fn update_project_source_branch_without_a_repo_is_refused() {
        let mut store = store_with_project();
        let err = update_project_source(&mut store, "rdm", merge(None, Some("main"))).unwrap_err();
        assert!(matches!(err, Error::ProjectSourceRepoMissing(name) if name == "rdm"));
        // Nothing was written.
        assert!(
            crate::io::load_project(&store, "rdm")
                .unwrap()
                .frontmatter
                .source
                .is_none()
        );
    }

    #[test]
    fn update_project_source_clears() {
        let mut store = store_with_project();
        update_project_source(&mut store, "rdm", merge(Some("/repo"), Some("main"))).unwrap();
        let doc = update_project_source(&mut store, "rdm", SourceUpdate::Clear).unwrap();
        assert!(doc.frontmatter.source.is_none());
        assert!(
            crate::io::load_project(&store, "rdm")
                .unwrap()
                .frontmatter
                .source
                .is_none()
        );
    }

    #[test]
    fn update_project_source_unknown_project() {
        let mut store = store_with_project();
        let err = update_project_source(&mut store, "nope", SourceUpdate::Clear).unwrap_err();
        assert!(matches!(err, Error::ProjectNotFound(name) if name == "nope"));
    }

    #[test]
    fn update_project_source_preserves_title_and_body() {
        let mut store = store_with_project();
        let mut doc = crate::io::load_project(&store, "rdm").unwrap();
        doc.frontmatter.title = "The rdm tool".to_string();
        doc.body = "Project notes.".to_string();
        store
            .write(&crate::paths::project_md_path("rdm"), doc.render().unwrap())
            .unwrap();
        let updated = update_project_source(&mut store, "rdm", merge(Some("/repo"), None)).unwrap();
        assert_eq!(updated.frontmatter.title, "The rdm tool");
        assert_eq!(updated.body.trim_end(), "Project notes.");
    }
}
