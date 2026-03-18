use std::path::PathBuf;

use rdm_core::repo::PlanRepo;
use rdm_core::store::FsStore;

/// Shared application state for the rdm server.
#[derive(Debug, Clone)]
pub struct AppState {
    /// Root path of the plan repository.
    pub plan_root: PathBuf,
}

impl AppState {
    /// Opens a [`PlanRepo`] backed by an [`FsStore`] at the configured root path.
    pub fn plan_repo(&self) -> PlanRepo<FsStore> {
        PlanRepo::new(FsStore::new(&self.plan_root))
    }
}
