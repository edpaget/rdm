use anyhow::{Context, Result};
use rdm_core::config::Config;
use rdm_core::ops::review::{PendingReviewItem, PendingReviewKind};

use crate::paths;
use crate::{AppStore, OutputFormat, ReviewCommand};

/// Runs `rdm review` subcommands.
///
/// # Errors
///
/// Returns an error if the project cannot be resolved, the pending-review
/// listing fails, the current directory cannot be read, or serialization
/// fails.
pub fn run(
    command: ReviewCommand,
    store: &mut AppStore,
    repo_config: &Config,
    format: OutputFormat,
) -> Result<()> {
    match command {
        ReviewCommand::Pending { project } => {
            let project = paths::resolve_project(project, repo_config)?;
            let items = rdm_core::ops::review::pending_review_items(store, &project)
                .context("failed to list pending-review items")?;

            // Scope to the current source-repo HEAD: keep items whose stamped
            // SHA is reachable from HEAD, and keep unstamped items (fail open).
            // Reachability errors (e.g. unknown SHA, unborn HEAD) also fail
            // open so a transient git state never hides work from review.
            let cwd = std::env::current_dir().context("failed to read current directory")?;
            let in_scope: Vec<PendingReviewItem> = items
                .into_iter()
                .filter(|item| match &item.review_sha {
                    None => true,
                    Some(sha) => rdm_store_git::is_ancestor_of_head_at(&cwd, sha).unwrap_or(true),
                })
                .collect();

            match format {
                OutputFormat::Json => {
                    let arr: Vec<_> = in_scope
                        .iter()
                        .map(|item| {
                            serde_json::json!({
                                "kind": kind_str(item.kind),
                                "identifier": item.identifier,
                                "project": item.project,
                                "title": item.title,
                            })
                        })
                        .collect();
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&arr)
                            .context("failed to serialize pending-review items")?
                    );
                }
                _ => {
                    if in_scope.is_empty() {
                        println!("No items pending review.");
                    } else {
                        for item in &in_scope {
                            println!(
                                "{} {}  {}",
                                kind_str(item.kind),
                                item.identifier,
                                item.title
                            );
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// Lowercase label for a pending-review item's kind.
fn kind_str(kind: PendingReviewKind) -> &'static str {
    match kind {
        PendingReviewKind::Phase => "phase",
        PendingReviewKind::Task => "task",
    }
}
