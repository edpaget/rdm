use crate::document::Document;
use crate::error::Result;
use crate::model::{Phase, Roadmap};
use crate::store::Store;

use super::PlanRepo;

impl<S: Store> PlanRepo<S> {
    // -- Roadmap operations (delegates to crate::ops::roadmap) --

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
        crate::ops::roadmap::create_roadmap(&mut self.store, project, slug, title, body)
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
        crate::ops::roadmap::update_roadmap(&mut self.store, project, slug, body)
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
        crate::ops::roadmap::list_roadmaps(&self.store, project)
    }

    /// Adds a dependency from one roadmap to another.
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
        crate::ops::roadmap::add_dependency(&mut self.store, project, slug, depends_on)
    }

    /// Removes a dependency from a roadmap.
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
        crate::ops::roadmap::remove_dependency(&mut self.store, project, slug, depends_on)
    }

    /// Returns the dependency graph for all roadmaps in a project.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ProjectNotFound`] if the project doesn't exist,
    /// [`Error::Io`] if directory reads fail, or frontmatter errors if
    /// any roadmap file is malformed.
    pub fn dependency_graph(&self, project: &str) -> Result<Vec<(String, Vec<String>)>> {
        crate::ops::roadmap::dependency_graph(&self.store, project)
    }

    /// Deletes a roadmap and all its phase files.
    ///
    /// # Errors
    ///
    /// Returns [`Error::RoadmapNotFound`] if the roadmap doesn't exist,
    /// [`Error::Io`] if file removal or writes fail, or
    /// frontmatter errors if any roadmap file is malformed.
    pub fn delete_roadmap(&mut self, project: &str, slug: &str) -> Result<()> {
        crate::ops::roadmap::delete_roadmap(&mut self.store, project, slug)
    }

    /// Archives a completed roadmap, moving it from active to archive.
    ///
    /// # Errors
    ///
    /// Returns [`Error::RoadmapNotFound`] if the roadmap doesn't exist,
    /// [`Error::RoadmapHasIncompletePhases`] if any phase is not done and
    /// `force` is false, or [`Error::Io`] on file I/O failures.
    pub fn archive_roadmap(&mut self, project: &str, slug: &str, force: bool) -> Result<()> {
        crate::ops::roadmap::archive_roadmap(&mut self.store, project, slug, force)
    }

    /// Lists archived roadmaps in a project.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] on read failure, or frontmatter errors
    /// if any archived roadmap file is malformed.
    pub fn list_archived_roadmaps(&self, project: &str) -> Result<Vec<Document<Roadmap>>> {
        crate::ops::roadmap::list_archived_roadmaps(&self.store, project)
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
        crate::ops::roadmap::list_archived_phases(&self.store, project, roadmap)
    }

    /// Restores an archived roadmap back to active status.
    ///
    /// # Errors
    ///
    /// Returns [`Error::RoadmapNotFound`] if the archived roadmap doesn't exist,
    /// [`Error::DuplicateSlug`] if an active roadmap with the same slug exists,
    /// or [`Error::Io`] on file I/O failures.
    pub fn unarchive_roadmap(&mut self, project: &str, slug: &str) -> Result<()> {
        crate::ops::roadmap::unarchive_roadmap(&mut self.store, project, slug)
    }

    /// Splits a roadmap by extracting selected phases into a new roadmap.
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
        crate::ops::roadmap::split_roadmap(
            &mut self.store,
            project,
            source_slug,
            target_slug,
            target_title,
            phase_stems,
            depends_on,
        )
    }
}
