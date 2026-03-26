//! Roadmap operations.

use crate::document::Document;
use crate::error::{Error, Result};
use crate::model::{Phase, PhaseStatus, Priority, Roadmap, RoadmapSort};
use crate::store::{DirEntryKind, RelPath, Store};

/// Creates a new roadmap within a project.
///
/// `body` sets the markdown body below the frontmatter. Pass `None` for
/// an empty body. `priority` sets an optional priority level.
///
/// # Errors
///
/// Returns [`Error::ProjectNotFound`] if the project doesn't exist,
/// [`Error::DuplicateSlug`] if the roadmap already exists,
/// [`Error::Io`] if file creation fails, or
/// [`Error::FrontmatterParse`] if frontmatter serialization fails.
pub fn create_roadmap(
    store: &mut impl Store,
    project: &str,
    slug: &str,
    title: &str,
    body: Option<&str>,
    priority: Option<Priority>,
) -> Result<Document<Roadmap>> {
    if !store.exists(&crate::paths::project_md_path(project)) {
        return Err(Error::ProjectNotFound(project.to_string()));
    }
    let roadmap_file = crate::paths::roadmap_path(project, slug);
    if store.exists(&roadmap_file) {
        return Err(Error::DuplicateSlug(slug.to_string()));
    }

    let doc = Document {
        frontmatter: Roadmap {
            project: project.to_string(),
            roadmap: slug.to_string(),
            title: title.to_string(),
            phases: Vec::new(),
            dependencies: None,
            priority,
        },
        body: body.unwrap_or_default().to_string(),
    };
    crate::io::write_roadmap(store, project, slug, &doc)?;
    store.commit()?;
    Ok(doc)
}

/// Updates a roadmap's body and/or priority.
///
/// When `body` is `Some`, replaces the existing body; `None` preserves it.
/// When `priority` is `Some(p)`, sets the priority to `p` (use
/// `Some(None)` to clear); `None` preserves the existing value.
///
/// # Errors
///
/// Returns [`Error::RoadmapNotFound`] if the roadmap doesn't exist,
/// [`Error::Io`] if reading or writing fails, or
/// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] if the
/// existing roadmap file has invalid frontmatter.
pub fn update_roadmap(
    store: &mut impl Store,
    project: &str,
    slug: &str,
    body: Option<&str>,
    priority: Option<Option<Priority>>,
) -> Result<Document<Roadmap>> {
    let path = crate::paths::roadmap_path(project, slug);
    if !store.exists(&path) {
        return Err(Error::RoadmapNotFound(slug.to_string()));
    }

    let mut doc = crate::io::load_roadmap(store, project, slug)?;
    if let Some(b) = body {
        doc.body = b.to_string();
    }
    if let Some(p) = priority {
        doc.frontmatter.priority = p;
    }
    crate::io::write_roadmap(store, project, slug, &doc)?;
    store.commit()?;
    Ok(doc)
}

/// Lists all roadmaps for a project.
///
/// Results are sorted by `sort` (defaults to alphabetical by slug).
/// When `priority_filter` is `Some`, only roadmaps with that exact
/// priority are returned.
///
/// # Errors
///
/// Returns [`Error::ProjectNotFound`] if the project doesn't exist,
/// [`Error::Io`] if the directory cannot be read, or
/// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] if a
/// roadmap file has invalid frontmatter.
pub fn list_roadmaps(
    store: &impl Store,
    project: &str,
    sort: Option<RoadmapSort>,
    priority_filter: Option<Priority>,
) -> Result<Vec<Document<Roadmap>>> {
    if !store.exists(&crate::paths::project_md_path(project)) {
        return Err(Error::ProjectNotFound(project.to_string()));
    }
    let roadmaps_dir = crate::paths::roadmaps_dir(project);
    let entries = store.list(&roadmaps_dir)?;
    let mut slugs: Vec<String> = entries
        .into_iter()
        .filter(|e| e.kind == DirEntryKind::Dir)
        .map(|e| e.name)
        .collect();
    slugs.sort();

    let mut roadmaps = Vec::new();
    for slug in slugs {
        // Skip directories without a roadmap.md (e.g., leftover empty dirs)
        if !store.exists(&crate::paths::roadmap_path(project, &slug)) {
            continue;
        }
        let doc = crate::io::load_roadmap(store, project, &slug)?;
        if priority_filter.is_none() || doc.frontmatter.priority == priority_filter {
            roadmaps.push(doc);
        }
    }

    if sort == Some(RoadmapSort::Priority) {
        roadmaps.sort_by(|a, b| {
            // Descending by priority: Critical > High > Medium > Low > None
            let pa = a.frontmatter.priority;
            let pb = b.frontmatter.priority;
            pb.cmp(&pa)
                .then_with(|| a.frontmatter.roadmap.cmp(&b.frontmatter.roadmap))
        });
    }

    Ok(roadmaps)
}

/// Adds a dependency from one roadmap to another.
///
/// Appends `depends_on` to the `dependencies` list of the roadmap
/// identified by `slug`. Validates that both roadmaps exist, the
/// dependency is not already present, and adding it would not create
/// a cycle.
///
/// # Errors
///
/// Returns [`Error::RoadmapNotFound`] if either roadmap doesn't exist,
/// [`Error::CyclicDependency`] if adding the dependency would create a cycle,
/// [`Error::Io`] if reading or writing fails, or
/// [`Error::FrontmatterParse`] if frontmatter is invalid.
pub fn add_dependency(
    store: &mut impl Store,
    project: &str,
    slug: &str,
    depends_on: &str,
) -> Result<Document<Roadmap>> {
    // Verify both roadmaps exist
    let mut doc = crate::io::load_roadmap(store, project, slug)?;
    let _target = crate::io::load_roadmap(store, project, depends_on)?;

    // Check for self-dependency
    if slug == depends_on {
        return Err(Error::CyclicDependency(format!(
            "{slug} cannot depend on itself"
        )));
    }

    // Check for duplicate
    let deps = doc.frontmatter.dependencies.get_or_insert_with(Vec::new);
    if deps.contains(&depends_on.to_string()) {
        return Ok(doc);
    }

    // Check for cycles: build adjacency list, add proposed edge, then DFS
    let all_roadmaps = list_roadmaps(store, project, None, None)?;
    let mut adj: std::collections::HashMap<&str, Vec<&str>> = std::collections::HashMap::new();
    for rm in &all_roadmaps {
        let s = rm.frontmatter.roadmap.as_str();
        if let Some(ref d) = rm.frontmatter.dependencies {
            for dep in d {
                adj.entry(s).or_default().push(dep.as_str());
            }
        }
    }
    // Add the proposed edge
    adj.entry(slug).or_default().push(depends_on);

    if has_cycle(&adj, slug) {
        return Err(Error::CyclicDependency(format!(
            "adding {slug} → {depends_on} would create a cycle"
        )));
    }

    deps.push(depends_on.to_string());
    crate::io::write_roadmap(store, project, slug, &doc)?;
    store.commit()?;
    Ok(doc)
}

/// Removes a dependency from a roadmap.
///
/// Removes `depends_on` from the `dependencies` list. If the dependency
/// is not present, this is a no-op.
///
/// # Errors
///
/// Returns [`Error::RoadmapNotFound`] if the roadmap doesn't exist,
/// [`Error::Io`] if reading or writing fails, or
/// [`Error::FrontmatterParse`] if frontmatter is invalid.
pub fn remove_dependency(
    store: &mut impl Store,
    project: &str,
    slug: &str,
    depends_on: &str,
) -> Result<Document<Roadmap>> {
    let mut doc = crate::io::load_roadmap(store, project, slug)?;

    if let Some(ref mut deps) = doc.frontmatter.dependencies {
        deps.retain(|d| d != depends_on);
        if deps.is_empty() {
            doc.frontmatter.dependencies = None;
        }
    }

    crate::io::write_roadmap(store, project, slug, &doc)?;
    store.commit()?;
    Ok(doc)
}

/// Returns the dependency graph for all roadmaps in a project.
///
/// Each entry is `(roadmap_slug, vec_of_dependency_slugs)`.
/// Only roadmaps with at least one dependency are included.
///
/// # Errors
///
/// Returns [`Error::ProjectNotFound`] if the project doesn't exist,
/// [`Error::Io`] if directory reads fail, or frontmatter errors if
/// any roadmap file is malformed.
pub fn dependency_graph(store: &impl Store, project: &str) -> Result<Vec<(String, Vec<String>)>> {
    let roadmaps = list_roadmaps(store, project, None, None)?;
    let mut graph = Vec::new();
    for rm in roadmaps {
        if let Some(deps) = rm.frontmatter.dependencies
            && !deps.is_empty()
        {
            graph.push((rm.frontmatter.roadmap, deps));
        }
    }
    Ok(graph)
}

/// Deletes a roadmap and all its phase files.
///
/// Also removes this roadmap from the dependency lists of any other
/// roadmaps in the same project.
///
/// # Errors
///
/// Returns [`Error::RoadmapNotFound`] if the roadmap doesn't exist,
/// [`Error::Io`] if file removal or writes fail, or
/// frontmatter errors if any roadmap file is malformed.
pub fn delete_roadmap(store: &mut impl Store, project: &str, slug: &str) -> Result<()> {
    let roadmap_file = crate::paths::roadmap_path(project, slug);
    if !store.exists(&roadmap_file) {
        return Err(Error::RoadmapNotFound(slug.to_string()));
    }

    // Remove this slug from dependency lists of all other roadmaps
    let roadmaps = list_roadmaps(store, project, None, None)?;
    for rm in roadmaps {
        if rm.frontmatter.roadmap == slug {
            continue;
        }
        if let Some(ref deps) = rm.frontmatter.dependencies
            && deps.contains(&slug.to_string())
        {
            remove_dependency(store, project, &rm.frontmatter.roadmap, slug)?;
        }
    }

    // Remove all files in the roadmap directory
    let dir = crate::paths::roadmap_dir(project, slug);
    delete_tree(store, &dir)?;
    store.commit()?;
    Ok(())
}

/// Archives a completed roadmap, moving it from active to archive.
///
/// Unless `force` is true, all phases must have status `Done`.
/// Dependency references from other active roadmaps are cleaned up.
///
/// # Errors
///
/// Returns [`Error::RoadmapNotFound`] if the roadmap doesn't exist,
/// [`Error::RoadmapHasIncompletePhases`] if any phase is not done and
/// `force` is false, or [`Error::Io`] on file I/O failures.
pub fn archive_roadmap(
    store: &mut impl Store,
    project: &str,
    slug: &str,
    force: bool,
) -> Result<()> {
    let roadmap_file = crate::paths::roadmap_path(project, slug);
    if !store.exists(&roadmap_file) {
        return Err(Error::RoadmapNotFound(slug.to_string()));
    }

    if !force {
        let phases = super::phase::list_phases(store, project, slug)?;
        let all_done = phases
            .iter()
            .all(|(_, doc)| doc.frontmatter.status == PhaseStatus::Done);
        if !all_done {
            return Err(Error::RoadmapHasIncompletePhases(slug.to_string()));
        }
    }

    // Clean up dependency refs from other active roadmaps
    let roadmaps = list_roadmaps(store, project, None, None)?;
    for rm in roadmaps {
        if rm.frontmatter.roadmap == slug {
            continue;
        }
        if let Some(ref deps) = rm.frontmatter.dependencies
            && deps.contains(&slug.to_string())
        {
            remove_dependency(store, project, &rm.frontmatter.roadmap, slug)?;
        }
    }

    let src = crate::paths::roadmap_dir(project, slug);
    let dst = crate::paths::archived_roadmap_dir(project, slug);
    copy_tree(store, &src, &dst)?;
    delete_tree(store, &src)?;
    store.commit()?;
    Ok(())
}

/// Lists archived roadmaps in a project.
///
/// Returns an empty vec if the archive directory doesn't exist.
///
/// # Errors
///
/// Returns [`Error::Io`] on read failure, or frontmatter errors
/// if any archived roadmap file is malformed.
pub fn list_archived_roadmaps(store: &impl Store, project: &str) -> Result<Vec<Document<Roadmap>>> {
    let archive_dir = crate::paths::archived_roadmaps_dir(project);
    let entries = store.list(&archive_dir)?;
    let mut slugs: Vec<String> = entries
        .into_iter()
        .filter(|e| e.kind == DirEntryKind::Dir)
        .map(|e| e.name)
        .collect();
    slugs.sort();

    let mut roadmaps = Vec::new();
    for slug in slugs {
        let path = crate::paths::archived_roadmap_path(project, &slug);
        if !store.exists(&path) {
            continue;
        }
        let content = store.read(&path)?;
        let doc: Document<Roadmap> = Document::parse(&content)?;
        roadmaps.push(doc);
    }
    Ok(roadmaps)
}

/// Lists phases in an archived roadmap, sorted by phase number.
///
/// # Errors
///
/// Returns [`Error::RoadmapNotFound`] if the archived roadmap doesn't exist,
/// or frontmatter/IO errors if phase files are malformed or unreadable.
pub fn list_archived_phases(
    store: &impl Store,
    project: &str,
    roadmap: &str,
) -> Result<Vec<(String, Document<Phase>)>> {
    let roadmap_file = crate::paths::archived_roadmap_path(project, roadmap);
    if !store.exists(&roadmap_file) {
        return Err(Error::RoadmapNotFound(roadmap.to_string()));
    }

    let dir = crate::paths::archived_roadmap_dir(project, roadmap);
    let entries = store.list(&dir)?;

    let mut phases: Vec<(String, Document<Phase>)> = Vec::new();
    for entry in entries {
        if entry.kind != DirEntryKind::File {
            continue;
        }
        if entry.name == "roadmap.md" || !entry.name.ends_with(".md") {
            continue;
        }
        let stem = entry.name.trim_end_matches(".md").to_string();
        let path = dir.join(&entry.name).expect("valid path");
        let content = store.read(&path)?;
        let doc: Document<Phase> = Document::parse(&content)?;
        phases.push((stem, doc));
    }
    phases.sort_by_key(|(_, doc)| doc.frontmatter.phase);
    Ok(phases)
}

/// Restores an archived roadmap back to active status.
///
/// Does not restore dependency references — the user must re-add them.
///
/// # Errors
///
/// Returns [`Error::RoadmapNotFound`] if the archived roadmap doesn't exist,
/// [`Error::DuplicateSlug`] if an active roadmap with the same slug exists,
/// or [`Error::Io`] on file I/O failures.
pub fn unarchive_roadmap(store: &mut impl Store, project: &str, slug: &str) -> Result<()> {
    let archived_file = crate::paths::archived_roadmap_path(project, slug);
    if !store.exists(&archived_file) {
        return Err(Error::RoadmapNotFound(slug.to_string()));
    }

    let active_file = crate::paths::roadmap_path(project, slug);
    if store.exists(&active_file) {
        return Err(Error::DuplicateSlug(slug.to_string()));
    }

    let src = crate::paths::archived_roadmap_dir(project, slug);
    let dst = crate::paths::roadmap_dir(project, slug);
    copy_tree(store, &src, &dst)?;
    delete_tree(store, &src)?;
    store.commit()?;
    Ok(())
}

/// Splits a roadmap by extracting selected phases into a new roadmap.
///
/// The selected phases are moved to the target roadmap and renumbered
/// starting from 1. Remaining phases in the source roadmap are also
/// renumbered from 1. If `depends_on` is `Some`, a dependency from the
/// target roadmap on the specified slug is added.
///
/// # Errors
///
/// Returns [`Error::RoadmapNotFound`] if the source roadmap doesn't exist,
/// [`Error::DuplicateSlug`] if the target roadmap already exists,
/// [`Error::InvalidPhaseSelection`] if any phase stem is not in the source
/// roadmap or if all phases would be extracted (leaving the source empty),
/// or [`Error::Io`] on file I/O failures.
#[allow(clippy::too_many_arguments)]
pub fn split_roadmap(
    store: &mut impl Store,
    project: &str,
    source_slug: &str,
    target_slug: &str,
    target_title: &str,
    phase_stems: &[String],
    depends_on: Option<&str>,
) -> Result<Document<Roadmap>> {
    // Validate source exists
    let source_doc = crate::io::load_roadmap(store, project, source_slug)?;

    // Validate target doesn't exist
    let target_roadmap_path = crate::paths::roadmap_path(project, target_slug);
    if store.exists(&target_roadmap_path) {
        return Err(Error::DuplicateSlug(target_slug.to_string()));
    }

    let source_phases = &source_doc.frontmatter.phases;

    // Validate all phase_stems exist in source
    for stem in phase_stems {
        if !source_phases.contains(stem) {
            return Err(Error::InvalidPhaseSelection(format!(
                "phase '{stem}' is not in roadmap '{source_slug}'"
            )));
        }
    }

    // Cannot extract all phases
    if phase_stems.len() == source_phases.len() {
        return Err(Error::InvalidPhaseSelection(
            "cannot extract all phases — source roadmap would be empty".to_string(),
        ));
    }

    // Cannot extract zero phases
    if phase_stems.is_empty() {
        return Err(Error::InvalidPhaseSelection(
            "no phases specified to extract".to_string(),
        ));
    }

    // Partition source phases into extracted and remaining, preserving order
    let mut extracted: Vec<String> = Vec::new();
    let mut remaining: Vec<String> = Vec::new();
    for stem in source_phases {
        if phase_stems.contains(stem) {
            extracted.push(stem.clone());
        } else {
            remaining.push(stem.clone());
        }
    }

    // Build target roadmap phases: renumber from 1
    let mut target_phase_stems = Vec::new();
    for (i, old_stem) in extracted.iter().enumerate() {
        let new_number = (i + 1) as u32;
        let phase_doc = crate::io::load_phase(store, project, source_slug, old_stem)?;

        // Derive the slug suffix (everything after "phase-N-")
        let slug_suffix = old_stem.splitn(3, '-').nth(2).unwrap_or(old_stem);
        let new_stem = format!("phase-{new_number}-{slug_suffix}");

        let new_phase_doc = Document {
            frontmatter: Phase {
                phase: new_number,
                ..phase_doc.frontmatter
            },
            body: phase_doc.body,
        };

        crate::io::write_phase(store, project, target_slug, &new_stem, &new_phase_doc)?;
        // Delete from source
        let old_path = crate::paths::phase_path(project, source_slug, old_stem);
        store.delete(&old_path)?;

        target_phase_stems.push(new_stem);
    }

    // Renumber remaining source phases from 1
    let mut new_source_stems = Vec::new();
    for (i, old_stem) in remaining.iter().enumerate() {
        let new_number = (i + 1) as u32;
        let phase_doc = crate::io::load_phase(store, project, source_slug, old_stem)?;

        let slug_suffix = old_stem.splitn(3, '-').nth(2).unwrap_or(old_stem);
        let new_stem = format!("phase-{new_number}-{slug_suffix}");

        let new_phase_doc = Document {
            frontmatter: Phase {
                phase: new_number,
                ..phase_doc.frontmatter
            },
            body: phase_doc.body,
        };

        if new_stem != *old_stem {
            crate::io::write_phase(store, project, source_slug, &new_stem, &new_phase_doc)?;
            let old_path = crate::paths::phase_path(project, source_slug, old_stem);
            store.delete(&old_path)?;
        } else {
            // Same stem, just update the frontmatter number if needed
            crate::io::write_phase(store, project, source_slug, &new_stem, &new_phase_doc)?;
        }

        new_source_stems.push(new_stem);
    }

    // Update source roadmap phases list
    let mut updated_source = source_doc;
    updated_source.frontmatter.phases = new_source_stems;
    crate::io::write_roadmap(store, project, source_slug, &updated_source)?;

    // Create target roadmap
    let target_doc = Document {
        frontmatter: Roadmap {
            project: project.to_string(),
            roadmap: target_slug.to_string(),
            title: target_title.to_string(),
            phases: target_phase_stems,
            dependencies: None,
            priority: None,
        },
        body: String::new(),
    };
    crate::io::write_roadmap(store, project, target_slug, &target_doc)?;

    store.commit()?;

    // Add dependency if requested
    if let Some(dep_slug) = depends_on {
        add_dependency(store, project, target_slug, dep_slug)?;
    }

    // Reload to return the final state
    crate::io::load_roadmap(store, project, target_slug)
}

// -- Private helpers --

/// Recursively copies all files from `src` to `dst` in the store.
fn copy_tree(store: &mut impl Store, src: &RelPath, dst: &RelPath) -> Result<()> {
    let entries = store.list(src)?;
    for entry in entries {
        let src_child = src.join(&entry.name).expect("valid path");
        let dst_child = dst.join(&entry.name).expect("valid path");
        match entry.kind {
            DirEntryKind::File => {
                let content = store.read(&src_child)?;
                store.write(&dst_child, content)?;
            }
            DirEntryKind::Dir => copy_tree(store, &src_child, &dst_child)?,
        }
    }
    Ok(())
}

/// Recursively deletes all files under a directory path in the store.
fn delete_tree(store: &mut impl Store, path: &RelPath) -> Result<()> {
    let entries = store.list(path)?;
    for entry in entries {
        let child = path.join(&entry.name).expect("valid path");
        match entry.kind {
            DirEntryKind::File => store.delete(&child)?,
            DirEntryKind::Dir => delete_tree(store, &child)?,
        }
    }
    Ok(())
}

/// Detects whether `start` participates in a cycle in the adjacency list.
fn has_cycle(adj: &std::collections::HashMap<&str, Vec<&str>>, start: &str) -> bool {
    let mut visited = std::collections::HashSet::new();
    let mut stack = vec![start];
    // Skip the start node on first visit — we want to detect if we
    // can reach `start` again by following edges.
    let mut is_first = true;
    while let Some(node) = stack.pop() {
        if !is_first && node == start {
            return true;
        }
        is_first = false;
        if visited.contains(node) {
            continue;
        }
        visited.insert(node);
        if let Some(neighbors) = adj.get(node) {
            for &n in neighbors {
                stack.push(n);
            }
        }
    }
    false
}
