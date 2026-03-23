use crate::error::Result;
use crate::store::Store;

use super::PlanRepo;

impl<S: Store> PlanRepo<S> {
    // -- Index generation (delegates to crate::ops::index) --

    /// Generates `projects/{project}/INDEX.md` for a single project.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] if directory reads or the write fail,
    /// or frontmatter errors if any document file is malformed.
    pub fn generate_project_index(&mut self, project: &str) -> Result<()> {
        crate::ops::index::generate_project_index(&mut self.store, project)
    }

    /// Generates index files, but only rewrites the per-project `INDEX.md`
    /// for the specified project.
    ///
    /// Builds index data for **all** projects (needed for the top-level
    /// summary), writes per-project `INDEX.md` only for `project`, and
    /// writes the top-level `INDEX.md`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] if directory reads or the final write fail,
    /// or frontmatter errors if any document file is malformed.
    pub fn generate_index_for_project(&mut self, project: &str) -> Result<()> {
        crate::ops::index::generate_index_for_project(&mut self.store, project)
    }

    /// Generates `INDEX.md` from the current repo state.
    ///
    /// Scans all projects, roadmaps (with phase progress), and tasks,
    /// then writes a formatted root index and per-project index files.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] if directory reads or the final write fail,
    /// or frontmatter errors if any document file is malformed.
    pub fn generate_index(&mut self) -> Result<()> {
        crate::ops::index::generate_index(&mut self.store)
    }
}
