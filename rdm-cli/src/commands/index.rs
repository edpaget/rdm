use std::path::Path;

use anyhow::{Context, Result};
use rdm_core::store::{RelPath, Store};

use crate::commands;

/// Regenerates `INDEX.md` from the current repo state.
///
/// When `merge_output`/`merge_path` are both set, this doubles as the
/// auto-installed `rdm-index` git merge driver: after regenerating every
/// `INDEX.md` (root and per-project), the regenerated content at
/// `merge_path` is written to `merge_output` — the file git substitutes for
/// `%A` and copies back into the merge result. Without them, `rdm index`
/// behaves as plain porcelain: regenerate and print a confirmation.
///
/// # Errors
///
/// Returns an error if the store cannot be opened, index generation fails,
/// `merge_path` is not a valid relative path, the regenerated index at
/// `merge_path` cannot be read, or `merge_output` cannot be written.
pub fn run(root: &Path, merge_output: Option<&Path>, merge_path: Option<&str>) -> Result<()> {
    let mut store = commands::make_store(root)?;
    rdm_core::ops::index::generate_index(&mut store).context("failed to generate index")?;
    match (merge_output, merge_path) {
        (Some(out), Some(p)) => {
            let rel = RelPath::new(p).with_context(|| format!("invalid --merge-path {p:?}"))?;
            let content = store
                .read(&rel)
                .with_context(|| format!("regenerated index missing at {p:?}"))?;
            std::fs::write(out, content)
                .with_context(|| format!("failed to write merge driver output to {out:?}"))?;
        }
        _ => println!("Generated INDEX.md"),
    }
    Ok(())
}
