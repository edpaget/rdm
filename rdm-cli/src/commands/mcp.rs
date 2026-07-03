use std::path::PathBuf;

use anyhow::{Context, Result};

/// Starts the MCP server on stdin/stdout.
///
/// # Errors
///
/// Returns an error if the tokio runtime cannot be created or the MCP server
/// errors.
pub fn run(root: PathBuf, global_config: &rdm_core::config::GlobalConfig) -> Result<()> {
    let auto_init = global_config.auto_init.unwrap_or(false);
    let rt = tokio::runtime::Runtime::new().context("failed to create tokio runtime")?;
    rt.block_on(rdm_mcp::run(root, auto_init))?;
    Ok(())
}
