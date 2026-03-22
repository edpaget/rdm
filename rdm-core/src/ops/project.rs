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
        },
        body: String::new(),
    };
    let content = doc.render()?;
    store.write(&md_path, content)?;
    store.commit()?;
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
