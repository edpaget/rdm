use crate::document::Document;
use crate::error::Result;
use crate::model::{Phase, PhaseStatus};
use crate::store::Store;

use super::PlanRepo;

impl<S: Store> PlanRepo<S> {
    // -- Phase operations (delegates to crate::ops::phase) --

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
        crate::ops::phase::list_phases(&self.store, project, roadmap)
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
        crate::ops::phase::create_phase(
            &mut self.store,
            project,
            roadmap,
            slug,
            title,
            phase_number,
            body,
        )
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
        crate::ops::phase::update_phase(
            &mut self.store,
            project,
            roadmap,
            phase_stem,
            status,
            body,
            commit,
        )
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
        crate::ops::phase::remove_phase(&mut self.store, project, roadmap, phase_stem)
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
        crate::ops::phase::resolve_phase_stem(&self.store, project, roadmap, identifier)
    }
}
