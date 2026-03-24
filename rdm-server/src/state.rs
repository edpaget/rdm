use std::path::PathBuf;

use rdm_store_fs::FsStore;

/// Shared application state for the rdm server.
#[derive(Debug, Clone)]
pub struct AppState {
    /// Root path of the plan repository.
    pub plan_root: PathBuf,
}

impl AppState {
    /// Opens an [`FsStore`] at the configured root path.
    pub fn store(&self) -> FsStore {
        FsStore::new(&self.plan_root)
    }
}
