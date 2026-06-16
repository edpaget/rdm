//! Index generation operations.

use crate::display::{self, ProjectIndex, RoadmapIndexEntry};
use crate::error::Result;
use crate::store::Store;

/// Builds a [`ProjectIndex`] for a single project.
///
/// Scans roadmaps (with phase progress) and tasks, returning the
/// aggregated index data without performing any I/O beyond reads.
///
/// # Errors
///
/// Returns [`Error::Io`] if directory reads fail, or frontmatter
/// errors if any document file is malformed.
fn build_project_index(store: &impl Store, project: &str) -> Result<ProjectIndex> {
    let roadmap_docs = super::roadmap::list_roadmaps(store, project, None, None)?;
    let mut roadmap_entries = Vec::new();
    for roadmap_doc in &roadmap_docs {
        let slug = &roadmap_doc.frontmatter.roadmap;
        let phases = super::phase::list_phases(store, project, slug)?;
        let done_count = phases
            .iter()
            .filter(|(_, doc)| doc.frontmatter.status.is_terminal())
            .count();
        roadmap_entries.push(RoadmapIndexEntry {
            slug: slug.clone(),
            project: project.to_string(),
            phase_count: phases.len(),
            done_count,
            dependencies: roadmap_doc.frontmatter.dependencies.clone(),
        });
    }

    let mut tasks = super::task::list_tasks(store, project)?;
    tasks.sort_by(|(slug_a, doc_a), (slug_b, doc_b)| {
        doc_b
            .frontmatter
            .priority
            .cmp(&doc_a.frontmatter.priority)
            .then_with(|| slug_a.cmp(slug_b))
    });

    Ok(ProjectIndex {
        name: project.to_string(),
        roadmaps: roadmap_entries,
        tasks,
    })
}

/// Generates `projects/{project}/INDEX.md` for a single project.
///
/// This **commits**. It is a standalone entry point; mutation flows should
/// instead go through [`crate::ops::mutate`], which uses the commit-free
/// [`generate_index_for_project`] and commits once for the whole transaction.
///
/// # Errors
///
/// Returns [`Error::Io`] if directory reads or the write fail,
/// or frontmatter errors if any document file is malformed.
pub fn generate_project_index(store: &mut impl Store, project: &str) -> Result<()> {
    let pi = build_project_index(store, project)?;
    let content = display::format_project_index(&pi);
    let path = crate::paths::project_index_path(project);
    store.write(&path, content)?;
    store.commit()?;
    Ok(())
}

/// Generates index files, but only rewrites the per-project `INDEX.md`
/// for the specified project.
///
/// Builds index data for **all** projects (needed for the top-level
/// summary), writes per-project `INDEX.md` only for `project`, and
/// writes the top-level `INDEX.md`.
///
/// Unlike [`generate_index`], this function does **not** commit: it only
/// stages the `INDEX.md` writes and leaves committing to the caller. It is
/// the index step inside [`crate::ops::mutate`], which writes the entity,
/// regenerates the index, and commits once.
///
/// # Errors
///
/// Returns [`Error::Io`] if directory reads or the final write fail,
/// or frontmatter errors if any document file is malformed.
pub fn generate_index_for_project(store: &mut impl Store, project: &str) -> Result<()> {
    let project_names = super::project::list_projects(store)?;
    let mut project_indices = Vec::new();

    for project_name in &project_names {
        let pi = build_project_index(store, project_name)?;

        // Only write per-project INDEX.md for the targeted project
        if project_name == project {
            let project_content = display::format_project_index(&pi);
            let project_index_path = crate::paths::project_index_path(project_name);
            store.write(&project_index_path, project_content)?;
        }

        project_indices.push(pi);
    }

    let content = display::format_top_level_index(&project_indices);
    let index_path = crate::paths::index_path();
    store.write(&index_path, content)?;
    Ok(())
}

/// Generates `INDEX.md` from the current repo state.
///
/// Scans all projects, roadmaps (with phase progress), and tasks,
/// then writes a formatted root index and per-project index files.
///
/// This **commits** and rebuilds every project's index. It is the standalone
/// full-repo rebuild (the `rdm index` command and post-merge/pull recovery);
/// per-mutation index upkeep happens automatically inside
/// [`crate::ops::mutate`] and need not call this.
///
/// # Errors
///
/// Returns [`Error::Io`] if directory reads or the final write fail,
/// or frontmatter errors if any document file is malformed.
pub fn generate_index(store: &mut impl Store) -> Result<()> {
    let project_names = super::project::list_projects(store)?;
    let mut project_indices = Vec::new();

    for project_name in &project_names {
        let pi = build_project_index(store, project_name)?;

        // Write per-project INDEX.md
        let project_content = display::format_project_index(&pi);
        let project_index_path = crate::paths::project_index_path(project_name);
        store.write(&project_index_path, project_content)?;

        project_indices.push(pi);
    }

    let content = display::format_top_level_index(&project_indices);
    let index_path = crate::paths::index_path();
    store.write(&index_path, content)?;
    store.commit()?;
    Ok(())
}
