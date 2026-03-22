use crate::document::Document;
use crate::error::{Error, Result};
use crate::model::{Phase, PhaseStatus, Roadmap};
use crate::store::{DirEntryKind, RelPath, Store};

use super::PlanRepo;

impl<S: Store> PlanRepo<S> {
    // -- Roadmap operations --

    /// Creates a new roadmap within a project.
    ///
    /// `body` sets the markdown body below the frontmatter. Pass `None` for
    /// an empty body.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ProjectNotFound`] if the project doesn't exist,
    /// [`Error::DuplicateSlug`] if the roadmap already exists,
    /// [`Error::Io`] if file creation fails, or
    /// [`Error::FrontmatterParse`] if frontmatter serialization fails.
    pub fn create_roadmap(
        &mut self,
        project: &str,
        slug: &str,
        title: &str,
        body: Option<&str>,
    ) -> Result<Document<Roadmap>> {
        if !self.store.exists(&crate::paths::project_md_path(project)) {
            return Err(Error::ProjectNotFound(project.to_string()));
        }
        let roadmap_file = crate::paths::roadmap_path(project, slug);
        if self.store.exists(&roadmap_file) {
            return Err(Error::DuplicateSlug(slug.to_string()));
        }

        let doc = Document {
            frontmatter: Roadmap {
                project: project.to_string(),
                roadmap: slug.to_string(),
                title: title.to_string(),
                phases: Vec::new(),
                dependencies: None,
            },
            body: body.unwrap_or_default().to_string(),
        };
        self.write_roadmap(project, slug, &doc)?;
        self.store.commit()?;
        Ok(doc)
    }

    /// Updates a roadmap's body.
    ///
    /// When `body` is `Some`, replaces the existing body; `None` preserves it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::RoadmapNotFound`] if the roadmap doesn't exist,
    /// [`Error::Io`] if reading or writing fails, or
    /// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] if the
    /// existing roadmap file has invalid frontmatter.
    pub fn update_roadmap(
        &mut self,
        project: &str,
        slug: &str,
        body: Option<&str>,
    ) -> Result<Document<Roadmap>> {
        let path = crate::paths::roadmap_path(project, slug);
        if !self.store.exists(&path) {
            return Err(Error::RoadmapNotFound(slug.to_string()));
        }

        let mut doc = self.load_roadmap(project, slug)?;
        if let Some(b) = body {
            doc.body = b.to_string();
        }
        self.write_roadmap(project, slug, &doc)?;
        self.store.commit()?;
        Ok(doc)
    }

    /// Lists all roadmaps for a project, sorted by slug.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ProjectNotFound`] if the project doesn't exist,
    /// [`Error::Io`] if the directory cannot be read, or
    /// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] if a
    /// roadmap file has invalid frontmatter.
    pub fn list_roadmaps(&self, project: &str) -> Result<Vec<Document<Roadmap>>> {
        if !self.store.exists(&crate::paths::project_md_path(project)) {
            return Err(Error::ProjectNotFound(project.to_string()));
        }
        let roadmaps_dir = crate::paths::roadmaps_dir(project);
        let entries = self.store.list(&roadmaps_dir)?;
        let mut slugs: Vec<String> = entries
            .into_iter()
            .filter(|e| e.kind == DirEntryKind::Dir)
            .map(|e| e.name)
            .collect();
        slugs.sort();

        let mut roadmaps = Vec::new();
        for slug in slugs {
            // Skip directories without a roadmap.md (e.g., leftover empty dirs)
            if !self
                .store
                .exists(&crate::paths::roadmap_path(project, &slug))
            {
                continue;
            }
            let doc = self.load_roadmap(project, &slug)?;
            roadmaps.push(doc);
        }
        Ok(roadmaps)
    }

    // -- Dependency management --

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
        &mut self,
        project: &str,
        slug: &str,
        depends_on: &str,
    ) -> Result<Document<Roadmap>> {
        // Verify both roadmaps exist
        let mut doc = self.load_roadmap(project, slug)?;
        let _target = self.load_roadmap(project, depends_on)?;

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
        let all_roadmaps = self.list_roadmaps(project)?;
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

        if Self::has_cycle(&adj, slug) {
            return Err(Error::CyclicDependency(format!(
                "adding {slug} → {depends_on} would create a cycle"
            )));
        }

        deps.push(depends_on.to_string());
        self.write_roadmap(project, slug, &doc)?;
        self.store.commit()?;
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
        &mut self,
        project: &str,
        slug: &str,
        depends_on: &str,
    ) -> Result<Document<Roadmap>> {
        let mut doc = self.load_roadmap(project, slug)?;

        if let Some(ref mut deps) = doc.frontmatter.dependencies {
            deps.retain(|d| d != depends_on);
            if deps.is_empty() {
                doc.frontmatter.dependencies = None;
            }
        }

        self.write_roadmap(project, slug, &doc)?;
        self.store.commit()?;
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
    pub fn dependency_graph(&self, project: &str) -> Result<Vec<(String, Vec<String>)>> {
        let roadmaps = self.list_roadmaps(project)?;
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

    // -- Internal tree helpers --

    /// Recursively copies all files from `src` to `dst` in the store.
    fn copy_tree(&mut self, src: &RelPath, dst: &RelPath) -> Result<()> {
        let entries = self.store.list(src)?;
        for entry in entries {
            let src_child = src.join(&entry.name).expect("valid path");
            let dst_child = dst.join(&entry.name).expect("valid path");
            match entry.kind {
                DirEntryKind::File => {
                    let content = self.store.read(&src_child)?;
                    self.store.write(&dst_child, content)?;
                }
                DirEntryKind::Dir => self.copy_tree(&src_child, &dst_child)?,
            }
        }
        Ok(())
    }

    /// Recursively deletes all files under a directory path in the store.
    fn delete_tree(&mut self, path: &RelPath) -> Result<()> {
        let entries = self.store.list(path)?;
        for entry in entries {
            let child = path.join(&entry.name).expect("valid path");
            match entry.kind {
                DirEntryKind::File => self.store.delete(&child)?,
                DirEntryKind::Dir => self.delete_tree(&child)?,
            }
        }
        Ok(())
    }

    // -- Delete roadmap --

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
    pub fn delete_roadmap(&mut self, project: &str, slug: &str) -> Result<()> {
        let roadmap_file = crate::paths::roadmap_path(project, slug);
        if !self.store.exists(&roadmap_file) {
            return Err(Error::RoadmapNotFound(slug.to_string()));
        }

        // Remove this slug from dependency lists of all other roadmaps
        let roadmaps = self.list_roadmaps(project)?;
        for rm in roadmaps {
            if rm.frontmatter.roadmap == slug {
                continue;
            }
            if let Some(ref deps) = rm.frontmatter.dependencies
                && deps.contains(&slug.to_string())
            {
                self.remove_dependency(project, &rm.frontmatter.roadmap, slug)?;
            }
        }

        // Remove all files in the roadmap directory
        let dir = crate::paths::roadmap_dir(project, slug);
        self.delete_tree(&dir)?;
        self.store.commit()?;
        Ok(())
    }

    // -- Archive roadmap --

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
    pub fn archive_roadmap(&mut self, project: &str, slug: &str, force: bool) -> Result<()> {
        let roadmap_file = crate::paths::roadmap_path(project, slug);
        if !self.store.exists(&roadmap_file) {
            return Err(Error::RoadmapNotFound(slug.to_string()));
        }

        if !force {
            let phases = self.list_phases(project, slug)?;
            let all_done = phases
                .iter()
                .all(|(_, doc)| doc.frontmatter.status == PhaseStatus::Done);
            if !all_done {
                return Err(Error::RoadmapHasIncompletePhases(slug.to_string()));
            }
        }

        // Clean up dependency refs from other active roadmaps
        let roadmaps = self.list_roadmaps(project)?;
        for rm in roadmaps {
            if rm.frontmatter.roadmap == slug {
                continue;
            }
            if let Some(ref deps) = rm.frontmatter.dependencies
                && deps.contains(&slug.to_string())
            {
                self.remove_dependency(project, &rm.frontmatter.roadmap, slug)?;
            }
        }

        let src = crate::paths::roadmap_dir(project, slug);
        let dst = crate::paths::archived_roadmap_dir(project, slug);
        self.copy_tree(&src, &dst)?;
        self.delete_tree(&src)?;
        self.store.commit()?;
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
    pub fn list_archived_roadmaps(&self, project: &str) -> Result<Vec<Document<Roadmap>>> {
        let archive_dir = crate::paths::archived_roadmaps_dir(project);
        let entries = self.store.list(&archive_dir)?;
        let mut slugs: Vec<String> = entries
            .into_iter()
            .filter(|e| e.kind == DirEntryKind::Dir)
            .map(|e| e.name)
            .collect();
        slugs.sort();

        let mut roadmaps = Vec::new();
        for slug in slugs {
            let path = crate::paths::archived_roadmap_path(project, &slug);
            if !self.store.exists(&path) {
                continue;
            }
            let content = self.store.read(&path)?;
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
        &self,
        project: &str,
        roadmap: &str,
    ) -> Result<Vec<(String, Document<Phase>)>> {
        let roadmap_file = crate::paths::archived_roadmap_path(project, roadmap);
        if !self.store.exists(&roadmap_file) {
            return Err(Error::RoadmapNotFound(roadmap.to_string()));
        }

        let dir = crate::paths::archived_roadmap_dir(project, roadmap);
        let entries = self.store.list(&dir)?;

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
            let content = self.store.read(&path)?;
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
    pub fn unarchive_roadmap(&mut self, project: &str, slug: &str) -> Result<()> {
        let archived_file = crate::paths::archived_roadmap_path(project, slug);
        if !self.store.exists(&archived_file) {
            return Err(Error::RoadmapNotFound(slug.to_string()));
        }

        let active_file = crate::paths::roadmap_path(project, slug);
        if self.store.exists(&active_file) {
            return Err(Error::DuplicateSlug(slug.to_string()));
        }

        let src = crate::paths::archived_roadmap_dir(project, slug);
        let dst = crate::paths::roadmap_dir(project, slug);
        self.copy_tree(&src, &dst)?;
        self.delete_tree(&src)?;
        self.store.commit()?;
        Ok(())
    }

    // -- Split roadmap --

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
    pub fn split_roadmap(
        &mut self,
        project: &str,
        source_slug: &str,
        target_slug: &str,
        target_title: &str,
        phase_stems: &[String],
        depends_on: Option<&str>,
    ) -> Result<Document<Roadmap>> {
        // Validate source exists
        let source_doc = self.load_roadmap(project, source_slug)?;

        // Validate target doesn't exist
        let target_roadmap_path = crate::paths::roadmap_path(project, target_slug);
        if self.store.exists(&target_roadmap_path) {
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
            let phase_doc = self.load_phase(project, source_slug, old_stem)?;

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

            self.write_phase(project, target_slug, &new_stem, &new_phase_doc)?;
            // Delete from source
            let old_path = crate::paths::phase_path(project, source_slug, old_stem);
            self.store.delete(&old_path)?;

            target_phase_stems.push(new_stem);
        }

        // Renumber remaining source phases from 1
        let mut new_source_stems = Vec::new();
        for (i, old_stem) in remaining.iter().enumerate() {
            let new_number = (i + 1) as u32;
            let phase_doc = self.load_phase(project, source_slug, old_stem)?;

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
                self.write_phase(project, source_slug, &new_stem, &new_phase_doc)?;
                let old_path = crate::paths::phase_path(project, source_slug, old_stem);
                self.store.delete(&old_path)?;
            } else {
                // Same stem, just update the frontmatter number if needed
                self.write_phase(project, source_slug, &new_stem, &new_phase_doc)?;
            }

            new_source_stems.push(new_stem);
        }

        // Update source roadmap phases list
        let mut updated_source = source_doc;
        updated_source.frontmatter.phases = new_source_stems;
        self.write_roadmap(project, source_slug, &updated_source)?;

        // Create target roadmap
        let target_doc = Document {
            frontmatter: Roadmap {
                project: project.to_string(),
                roadmap: target_slug.to_string(),
                title: target_title.to_string(),
                phases: target_phase_stems,
                dependencies: None,
            },
            body: String::new(),
        };
        self.write_roadmap(project, target_slug, &target_doc)?;

        self.store.commit()?;

        // Add dependency if requested
        if let Some(dep_slug) = depends_on {
            self.add_dependency(project, target_slug, dep_slug)?;
        }

        // Reload to return the final state
        self.load_roadmap(project, target_slug)
    }
}
