use chrono::Local;

use crate::document::Document;
use crate::error::{Error, Result};
use crate::model::{Phase, PhaseStatus};
use crate::store::{DirEntryKind, Store};

use super::PlanRepo;

impl<S: Store> PlanRepo<S> {
    // -- Phase operations --

    /// Lists all phases in a roadmap, sorted by phase number.
    ///
    /// Returns `(stem, Document<Phase>)` tuples.
    ///
    /// # Errors
    ///
    /// Returns [`Error::RoadmapNotFound`] if the roadmap doesn't exist,
    /// [`Error::Io`] if the directory cannot be read, or
    /// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] if a
    /// phase file has invalid frontmatter.
    pub fn list_phases(
        &self,
        project: &str,
        roadmap: &str,
    ) -> Result<Vec<(String, Document<Phase>)>> {
        let roadmap_file = crate::paths::roadmap_path(project, roadmap);
        if !self.store.exists(&roadmap_file) {
            return Err(Error::RoadmapNotFound(roadmap.to_string()));
        }

        let dir = crate::paths::roadmap_dir(project, roadmap);
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
            let doc = crate::io::load_phase(&self.store, project, roadmap, &stem)?;
            phases.push((stem, doc));
        }
        phases.sort_by_key(|(_, doc)| doc.frontmatter.phase);
        Ok(phases)
    }

    /// Creates a new phase within a roadmap.
    ///
    /// If `phase_number` is `None`, auto-assigns the next number.
    /// `body` sets the markdown body below the frontmatter. Pass `None` for
    /// an empty body.
    ///
    /// # Errors
    ///
    /// Returns [`Error::RoadmapNotFound`] if the roadmap doesn't exist,
    /// [`Error::DuplicateSlug`] if a phase with the same stem already exists,
    /// [`Error::Io`] if file creation fails, or
    /// [`Error::FrontmatterParse`] if frontmatter serialization fails.
    pub fn create_phase(
        &mut self,
        project: &str,
        roadmap: &str,
        slug: &str,
        title: &str,
        phase_number: Option<u32>,
        body: Option<&str>,
    ) -> Result<Document<Phase>> {
        let roadmap_file = crate::paths::roadmap_path(project, roadmap);
        if !self.store.exists(&roadmap_file) {
            return Err(Error::RoadmapNotFound(roadmap.to_string()));
        }

        let number = match phase_number {
            Some(n) => n,
            None => {
                let existing = self.list_phases(project, roadmap)?;
                existing
                    .last()
                    .map(|(_, doc)| doc.frontmatter.phase + 1)
                    .unwrap_or(1)
            }
        };

        let stem = crate::model::phase_stem(number, slug);
        let path = crate::paths::phase_path(project, roadmap, &stem);
        if self.store.exists(&path) {
            return Err(Error::DuplicateSlug(stem));
        }

        let doc = Document {
            frontmatter: Phase {
                phase: number,
                title: title.to_string(),
                status: PhaseStatus::NotStarted,
                completed: None,
                commit: None,
            },
            body: body.unwrap_or_default().to_string(),
        };
        crate::io::write_phase(&mut self.store, project, roadmap, &stem, &doc)?;

        // Update roadmap's phases list
        let mut roadmap_doc = crate::io::load_roadmap(&self.store, project, roadmap)?;
        roadmap_doc.frontmatter.phases.push(stem);
        crate::io::write_roadmap(&mut self.store, project, roadmap, &roadmap_doc)?;
        self.store.commit()?;

        Ok(doc)
    }

    /// Updates a phase's status, body, and/or commit SHA.
    ///
    /// When `status` is `Some(Done)`, auto-sets `completed` to today and stores
    /// the optional `commit` SHA. When `status` is `Some` but not `Done`,
    /// clears both `completed` and `commit`. When `status` is `None`, the
    /// existing status, `completed`, and `commit` are preserved.
    /// When `body` is `Some`, replaces the existing body; `None` preserves it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::PhaseNotFound`] if the phase file doesn't exist,
    /// [`Error::Io`] if reading or writing fails, or
    /// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] if the
    /// existing phase file has invalid frontmatter.
    pub fn update_phase(
        &mut self,
        project: &str,
        roadmap: &str,
        phase_stem: &str,
        status: Option<PhaseStatus>,
        body: Option<&str>,
        commit: Option<String>,
    ) -> Result<Document<Phase>> {
        let path = crate::paths::phase_path(project, roadmap, phase_stem);
        if !self.store.exists(&path) {
            return Err(Error::PhaseNotFound(phase_stem.to_string()));
        }

        let mut doc = crate::io::load_phase(&self.store, project, roadmap, phase_stem)?;
        if let Some(status) = status {
            if status == PhaseStatus::Done && doc.frontmatter.status == PhaseStatus::Done {
                // Already done: only update commit if a new one is provided
                if let Some(sha) = commit {
                    doc.frontmatter.commit = Some(sha);
                }
            } else {
                doc.frontmatter.status = status;
                if status == PhaseStatus::Done {
                    doc.frontmatter.completed = Some(Local::now().date_naive());
                    doc.frontmatter.commit = commit;
                } else {
                    doc.frontmatter.completed = None;
                    doc.frontmatter.commit = None;
                }
            }
        }
        if let Some(b) = body {
            doc.body = b.to_string();
        }
        crate::io::write_phase(&mut self.store, project, roadmap, phase_stem, &doc)?;
        self.store.commit()?;
        Ok(doc)
    }

    /// Removes a phase from a roadmap.
    ///
    /// Deletes the phase file and removes its stem from the roadmap's `phases`
    /// list.
    ///
    /// # Errors
    ///
    /// Returns [`Error::PhaseNotFound`] if the phase file doesn't exist,
    /// [`Error::Io`] if the file cannot be deleted or the roadmap cannot be
    /// updated, or [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`]
    /// if the roadmap file has invalid frontmatter.
    pub fn remove_phase(&mut self, project: &str, roadmap: &str, phase_stem: &str) -> Result<()> {
        let path = crate::paths::phase_path(project, roadmap, phase_stem);
        if !self.store.exists(&path) {
            return Err(Error::PhaseNotFound(phase_stem.to_string()));
        }
        self.store.delete(&path)?;

        // Remove stem from roadmap's phases list
        let mut roadmap_doc = crate::io::load_roadmap(&self.store, project, roadmap)?;
        roadmap_doc.frontmatter.phases.retain(|s| s != phase_stem);
        crate::io::write_roadmap(&mut self.store, project, roadmap, &roadmap_doc)?;
        self.store.commit()?;
        Ok(())
    }

    /// Resolves a phase identifier to a file stem.
    ///
    /// If `identifier` parses as a `u32`, looks up the phase by number.
    /// Otherwise, returns `identifier` as-is for downstream validation.
    ///
    /// # Errors
    ///
    /// Returns [`Error::PhaseNotFound`] if `identifier` is numeric but no
    /// phase with that number exists. Also propagates errors from
    /// [`list_phases`].
    pub fn resolve_phase_stem(
        &self,
        project: &str,
        roadmap: &str,
        identifier: &str,
    ) -> Result<String> {
        if let Ok(num) = identifier.parse::<u32>() {
            let phases = self.list_phases(project, roadmap)?;
            for (stem, doc) in phases {
                if doc.frontmatter.phase == num {
                    return Ok(stem);
                }
            }
            return Err(Error::PhaseNotFound(identifier.to_string()));
        }
        Ok(identifier.to_string())
    }
}
