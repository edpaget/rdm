use std::path::{Path, PathBuf};
use std::sync::Arc;

use rdm_core::config::QuickFilter;
use rdm_core::store::VersionedStore;
use rdm_store_fs::FsStore;

use crate::templates::{QuickFilterView, quick_filter_views};

/// Constructs a fresh [`VersionedStore`] instance rooted at the given path.
///
/// Called once per request/lookup by [`AppState::store`]; defaults to
/// [`FsStore`]. Tests may override it via [`AppState::with_store_factory`]
/// to inject a real [`VersionedStore`] backend (e.g. a git-backed store),
/// exercising the `?at=<sha>` historical-read path end-to-end.
pub type StoreFactory = Arc<dyn Fn(&Path) -> Box<dyn VersionedStore + Send + Sync> + Send + Sync>;

/// Shared application state for the rdm server.
#[derive(Clone)]
pub struct AppState {
    /// Root path of the plan repository.
    pub plan_root: PathBuf,
    /// Quick-filter chips configured for HTML list views.
    ///
    /// Resolved by the CLI from `[server.quick_filters]` in `rdm.toml`,
    /// `RDM_SERVER_QUICK_FILTERS` env var, and `--quick-filter` CLI flags.
    pub quick_filters: Vec<QuickFilter>,
    /// Constructs the [`VersionedStore`] backend used by [`AppState::store`].
    ///
    /// Defaults to [`FsStore`]. This field is `pub` (rather than
    /// crate-private) so that `AppState { .. , ..Default::default() }`
    /// struct-update syntax — used pervasively by production call sites and
    /// test fixtures across crate boundaries (`rdm-cli`, `rdm-server`'s own
    /// `tests/` integration binaries) — type-checks regardless of module or
    /// crate visibility. Prefer [`AppState::with_store_factory`] over setting
    /// this directly; it reads as intent ("inject a store backend") rather
    /// than a raw field assignment.
    pub store_factory: StoreFactory,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("plan_root", &self.plan_root)
            .field("quick_filters", &self.quick_filters)
            .finish_non_exhaustive()
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            plan_root: PathBuf::new(),
            quick_filters: Vec::new(),
            store_factory: Arc::new(|root| Box::new(FsStore::new(root))),
        }
    }
}

impl AppState {
    /// Opens the configured [`VersionedStore`] backend at the plan root.
    ///
    /// Defaults to [`FsStore`] (no history). Overridden by
    /// [`AppState::with_store_factory`] in tests that need real history.
    pub fn store(&self) -> Box<dyn VersionedStore + Send + Sync> {
        (self.store_factory)(&self.plan_root)
    }

    /// Overrides the store backend used by [`AppState::store`].
    ///
    /// Used by integration tests to back the server with a real
    /// git-backed [`rdm_store_git::GitStore`] so the `?at=<sha>`
    /// historical-read path can be exercised end-to-end.
    #[must_use]
    pub fn with_store_factory(mut self, factory: StoreFactory) -> Self {
        self.store_factory = factory;
        self
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
