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

/// Sets or clears the code repository a project's `rdm:src/` links and
/// `change/<ref>` reviews resolve against.
///
/// Passing `Some(source)` records it verbatim; passing `None` clears any
/// configured source. This is the only writer of [`Project::source`] — it
/// exists so configuring a project's source repository is a CLI act rather
/// than a hand edit of the plan repo.
///
/// # Errors
///
/// Returns [`Error::ProjectNotFound`] if the project does not exist,
/// [`Error::Io`] if the write fails, or [`Error::FrontmatterParse`] if
/// frontmatter serialization fails.
pub fn update_project_source(
    store: &mut impl Store,
    name: &str,
    source: Option<crate::model::Source>,
) -> Result<Document<Project>> {
    let mut doc = crate::io::load_project(store, name)?;
    doc.frontmatter.source = source;
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

    #[test]
    fn update_project_source_sets_repo_and_branch() {
        let mut store = store_with_project();
        let doc = update_project_source(
            &mut store,
            "rdm",
            Some(Source {
                repo: "/Users/ed/Projects/rdm".to_string(),
                default_branch: Some("main".to_string()),
            }),
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
    fn update_project_source_can_replace_just_the_branch() {
        let mut store = store_with_project();
        update_project_source(
            &mut store,
            "rdm",
            Some(Source {
                repo: "/repo".to_string(),
                default_branch: None,
            }),
        )
        .unwrap();
        let existing = crate::io::load_project(&store, "rdm")
            .unwrap()
            .frontmatter
            .source
            .unwrap();
        let doc = update_project_source(
            &mut store,
            "rdm",
            Some(Source {
                repo: existing.repo,
                default_branch: Some("trunk".to_string()),
            }),
        )
        .unwrap();
        let source = doc.frontmatter.source.unwrap();
        assert_eq!(source.repo, "/repo");
        assert_eq!(source.default_branch.as_deref(), Some("trunk"));
    }

    #[test]
    fn update_project_source_clears() {
        let mut store = store_with_project();
        update_project_source(
            &mut store,
            "rdm",
            Some(Source {
                repo: "/repo".to_string(),
                default_branch: None,
            }),
        )
        .unwrap();
        let doc = update_project_source(&mut store, "rdm", None).unwrap();
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
        let err = update_project_source(&mut store, "nope", None).unwrap_err();
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
        let updated = update_project_source(
            &mut store,
            "rdm",
            Some(Source {
                repo: "/repo".to_string(),
                default_branch: None,
            }),
        )
        .unwrap();
        assert_eq!(updated.frontmatter.title, "The rdm tool");
        assert_eq!(updated.body.trim_end(), "Project notes.");
    }
}
