use std::path::PathBuf;

use rdm_core::config::QuickFilter;
use rdm_store_fs::FsStore;

use crate::templates::{QuickFilterView, quick_filter_views};

/// Shared application state for the rdm server.
#[derive(Debug, Clone, Default)]
pub struct AppState {
    /// Root path of the plan repository.
    pub plan_root: PathBuf,
    /// Quick-filter chips configured for HTML list views.
    ///
    /// Resolved by the CLI from `[server.quick_filters]` in `rdm.toml`,
    /// `RDM_SERVER_QUICK_FILTERS` env var, and `--quick-filter` CLI flags.
    pub quick_filters: Vec<QuickFilter>,
}

impl AppState {
    /// Opens an [`FsStore`] at the configured root path.
    pub fn store(&self) -> FsStore {
        FsStore::new(&self.plan_root)
    }

    /// Build the [`QuickFilterView`] list for a given page path.
    ///
    /// `page_path` should be the page's path without query string (e.g.
    /// `/projects/demo/roadmaps`); each chip's href is built by appending
    /// `?tag=<encoded-tag>`. `active_tag` highlights the matching chip.
    pub fn quick_filter_views_for_path(
        &self,
        page_path: &str,
        active_tag: Option<&str>,
    ) -> Vec<QuickFilterView> {
        quick_filter_views(&self.quick_filters, page_path, active_tag)
    }
}
