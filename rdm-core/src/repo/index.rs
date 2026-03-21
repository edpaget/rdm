use crate::display::{self, ProjectIndex, RoadmapIndexEntry};
use crate::error::Result;
use crate::model::PhaseStatus;
use crate::store::Store;

use super::PlanRepo;

impl<S: Store> PlanRepo<S> {
    // -- Index generation --

    /// Builds a [`ProjectIndex`] for a single project.
    ///
    /// Scans roadmaps (with phase progress) and tasks, returning the
    /// aggregated index data without performing any I/O beyond reads.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] if directory reads fail, or frontmatter
    /// errors if any document file is malformed.
    fn build_project_index(&self, project: &str) -> Result<ProjectIndex> {
        let roadmap_docs = self.list_roadmaps(project)?;
        let mut roadmap_entries = Vec::new();
        for roadmap_doc in &roadmap_docs {
            let slug = &roadmap_doc.frontmatter.roadmap;
            let phases = self.list_phases(project, slug)?;
            let done_count = phases
                .iter()
                .filter(|(_, doc)| doc.frontmatter.status == PhaseStatus::Done)
                .count();
            roadmap_entries.push(RoadmapIndexEntry {
                slug: slug.clone(),
                project: project.to_string(),
                phase_count: phases.len(),
                done_count,
                dependencies: roadmap_doc.frontmatter.dependencies.clone(),
            });
        }

        let mut tasks = self.list_tasks(project)?;
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
    /// # Errors
    ///
    /// Returns [`Error::Io`] if directory reads or the write fail,
    /// or frontmatter errors if any document file is malformed.
    pub fn generate_project_index(&mut self, project: &str) -> Result<()> {
        let pi = self.build_project_index(project)?;
        let content = display::format_project_index(&pi);
        let path = self.project_index_path(project);
        self.store.write(&path, content)?;
        self.store.commit()?;
        Ok(())
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
        let project_names = self.list_projects()?;
        let mut project_indices = Vec::new();

        for project_name in &project_names {
            let pi = self.build_project_index(project_name)?;

            // Only write per-project INDEX.md for the targeted project
            if project_name == project {
                let project_content = display::format_project_index(&pi);
                let project_index_path = self.project_index_path(project_name);
                self.store.write(&project_index_path, project_content)?;
            }

            project_indices.push(pi);
        }

        let content = display::format_top_level_index(&project_indices);
        let index_path = self.index_path();
        self.store.write(&index_path, content)?;
        self.store.commit()?;
        Ok(())
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
        let project_names = self.list_projects()?;
        let mut project_indices = Vec::new();

        for project_name in &project_names {
            let pi = self.build_project_index(project_name)?;

            // Write per-project INDEX.md
            let project_content = display::format_project_index(&pi);
            let project_index_path = self.project_index_path(project_name);
            self.store.write(&project_index_path, project_content)?;

            project_indices.push(pi);
        }

        let content = display::format_top_level_index(&project_indices);
        let index_path = self.index_path();
        self.store.write(&index_path, content)?;
        self.store.commit()?;
        Ok(())
    }
}
