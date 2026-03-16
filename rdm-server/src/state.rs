use std::path::PathBuf;

use rdm_core::repo::PlanRepo;

/// Shared application state for the rdm server.
#[derive(Debug, Clone)]
pub struct AppState {
    /// Root path of the plan repository.
    pub plan_root: PathBuf,
}

impl AppState {
    /// Opens a [`PlanRepo`] pointing at the configured root path.
    pub fn plan_repo(&self) -> PlanRepo {
        PlanRepo::open(&self.plan_root)
    }
}
