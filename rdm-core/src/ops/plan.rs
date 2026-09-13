//! Implementation-plan operations.

use crate::document::Document;
use crate::error::{Error, Result};
use crate::model::Plan;
use crate::store::{DirEntryKind, Store};

/// Lists all implementation plans for a project, sorted by slug.
///
/// Returns `(slug, Document<Plan>)` tuples. Returns an empty vec when the
/// project has no `plans/` directory yet, mirroring
/// [`list_tasks`](crate::ops::task::list_tasks) — a project that has never
/// had a plan is not an error.
///
/// # Errors
///
/// Returns [`Error::ProjectNotFound`] if the project doesn't exist,
/// [`Error::Io`] if the directory cannot be read, or
/// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] if a plan file
/// has invalid frontmatter.
pub fn list_plans(store: &impl Store, project: &str) -> Result<Vec<(String, Document<Plan>)>> {
    if !store.exists(&crate::paths::project_md_path(project)) {
        return Err(Error::ProjectNotFound(project.to_string()));
    }
    let dir = crate::paths::plans_dir(project);
    let entries = store.list(&dir)?;

    let mut plans: Vec<(String, Document<Plan>)> = Vec::new();
    for entry in entries {
        if entry.kind != DirEntryKind::File {
            continue;
        }
        if !entry.name.ends_with(".md") {
            continue;
        }
        let slug = entry.name.trim_end_matches(".md").to_string();
        let doc = crate::io::load_plan(store, project, &slug)?;
        plans.push((slug, doc));
    }
    plans.sort_by(|(a, _), (b, _)| a.cmp(b));
    Ok(plans)
}
