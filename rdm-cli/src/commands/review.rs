use std::path::Path;

use anyhow::{Context, Result};
use rdm_core::config::Config;
use rdm_core::model::{PhaseStatus, TaskStatus};
use rdm_core::ops::review::{PendingReviewItem, PendingReviewKind};
use rdm_core::ops::{BodyUpdate, TagsUpdate};

use super::commit_mutation;
use crate::paths;
use crate::{AppStore, OutputFormat, ReviewCommand};

/// Filters `items` to those in scope for the checkout rooted at `cwd`.
///
/// Scope is decided by branch identity first (an item's stamped `review_branch`
/// must equal the checkout's current branch), falling back to SHA reachability
/// for items with no branch stamp or when the checkout's branch is unresolvable.
/// Any git error fails open so work is never silently hidden. This is the single
/// scoping rule shared by `review pending` and `review restamp`.
fn filter_in_scope(items: Vec<PendingReviewItem>, cwd: &Path) -> Vec<PendingReviewItem> {
    let current_branch = rdm_git::current_branch_at(cwd).ok().flatten();
    // SHA-reachability fallback: keep an item whose stamped sha is reachable
    // from HEAD, keep unstamped items, and fail open on any git error — a
    // transient git state must never hide work from review.
    let sha_reachable = |item: &PendingReviewItem| match &item.review_sha {
        None => true,
        Some(sha) => rdm_git::is_ancestor_of_head_at(cwd, sha).unwrap_or(true),
    };
    items
        .into_iter()
        .filter(
            |item| match (&item.review_branch, current_branch.as_deref()) {
                // A branch is checked out and the item is branch-stamped:
                // exact identity match keeps roadmaps perfectly isolated.
                (Some(branch), Some(cur)) => cur == branch.as_str(),
                // Branch-stamped, but the firing checkout has no resolvable
                // branch (detached HEAD, a non-repo cwd, or git unavailable):
                // identity can't be compared, so fall back to SHA reachability
                // and fail open rather than hiding stamped work.
                (Some(_), None) => sha_reachable(item),
                // Legacy item with no stamped branch: SHA-reachability fallback
                // so nothing pre-stamp is ever dropped.
                (None, _) => sha_reachable(item),
            },
        )
        .collect()
}

/// Runs `rdm review` subcommands.
///
/// # Errors
///
/// Returns an error if the project cannot be resolved, the pending-review
/// listing fails, the current directory cannot be read, a restamp mutation
/// fails, or serialization fails.
pub fn run(
    command: ReviewCommand,
    store: &mut AppStore,
    repo_config: &Config,
    format: OutputFormat,
    no_index: bool,
    staging: bool,
) -> Result<()> {
    match command {
        ReviewCommand::Pending { project } => {
            let project = paths::resolve_project(project, repo_config)?;
            let items = rdm_core::ops::review::pending_review_items(store, &project)
                .context("failed to list pending-review items")?;

            let cwd = std::env::current_dir().context("failed to read current directory")?;
            let in_scope = filter_in_scope(items, &cwd);

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
                                "branch": item.review_branch,
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
        ReviewCommand::Restamp { project } => {
            // Wiring this to run fail-open immediately before every `review
            // pending` call (in the Stop hook / Pi extension) makes the *net*
            // effect equivalent to self-healing scope. It is nonetheless a
            // separate command rather than a side effect of `pending`: keeping
            // the plan-repo mutation isolated behind an explicit verb preserves
            // the architectural rule that `pending` is a pure read (no surprise
            // commits / staged diffs for anyone scripting it).
            let project = paths::resolve_project(project, repo_config)?;
            let items = rdm_core::ops::review::pending_review_items(store, &project)
                .context("failed to list pending-review items")?;

            let cwd = std::env::current_dir().context("failed to read current directory")?;
            let in_scope = filter_in_scope(items, &cwd);

            // Resolve the current source-repo HEAD/branch we will stamp toward.
            // If HEAD is unresolvable (no commit, non-repo cwd, git missing),
            // restamp is a no-op — fail open, never error the hook path.
            let sha = rdm_git::head_commit_info_at(&cwd)
                .ok()
                .flatten()
                .map(|c| c.sha);
            let branch = rdm_git::current_branch_at(&cwd).ok().flatten();

            // Each entry records the kind, identifier, and the *effective* branch
            // actually stamped (which may be the item's preserved branch, not the
            // unresolved current one) so the JSON output reports the truth.
            let mut restamped: Vec<(PendingReviewKind, String, Option<String>)> = Vec::new();
            if let Some(sha) = sha.clone() {
                for item in in_scope {
                    // Never downgrade an existing branch stamp: when the current
                    // checkout has no resolvable branch (detached HEAD, etc.) but
                    // the item passed scope via the SHA-reachability fallback,
                    // preserve its stamped branch rather than overwriting it with
                    // `None` — otherwise a sibling branch sharing history would
                    // pick the item up, the exact cross-branch leakage that
                    // branch-identity scoping prevents. We still refresh the SHA.
                    let target_branch = branch.clone().or_else(|| item.review_branch.clone());
                    // Idempotency guard: skip items already stamped at the
                    // current HEAD and (effective) branch so the hook's per-turn
                    // call generates no plan-repo write or commit churn.
                    if item.review_sha.as_deref() == Some(sha.as_str())
                        && item.review_branch == target_branch
                    {
                        continue;
                    }
                    let identifier = item.identifier.clone();
                    let kind = item.kind;
                    match kind {
                        PendingReviewKind::Phase => {
                            let Some((roadmap, stem)) = identifier.split_once('/') else {
                                continue;
                            };
                            let roadmap = roadmap.to_string();
                            let stem = stem.to_string();
                            let sha = sha.clone();
                            let branch = target_branch.clone();
                            commit_mutation(
                                store,
                                &project,
                                no_index,
                                staging,
                                "failed to restamp phase",
                                |s| {
                                    rdm_core::ops::phase::update_phase(
                                        s,
                                        &project,
                                        &roadmap,
                                        &stem,
                                        Some(PhaseStatus::NeedsReview),
                                        TagsUpdate::Keep,
                                        BodyUpdate::Keep,
                                        None,
                                        Some(sha),
                                        branch,
                                    )
                                },
                            )?;
                        }
                        PendingReviewKind::Task => {
                            let slug = identifier.clone();
                            let sha = sha.clone();
                            let branch = target_branch.clone();
                            commit_mutation(
                                store,
                                &project,
                                no_index,
                                staging,
                                "failed to restamp task",
                                |s| {
                                    rdm_core::ops::task::update_task(
                                        s,
                                        &project,
                                        &slug,
                                        Some(TaskStatus::NeedsReview),
                                        None,
                                        TagsUpdate::Keep,
                                        BodyUpdate::Keep,
                                        None,
                                        Some(sha),
                                        branch,
                                    )
                                },
                            )?;
                        }
                    }
                    restamped.push((kind, identifier, target_branch));
                }
            }

            match format {
                OutputFormat::Json => {
                    let arr: Vec<_> = restamped
                        .iter()
                        .map(|(kind, identifier, item_branch)| {
                            serde_json::json!({
                                "kind": kind_str(*kind),
                                "identifier": identifier,
                                "project": project,
                                "sha": sha,
                                "branch": item_branch,
                            })
                        })
                        .collect();
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&arr)
                            .context("failed to serialize restamped items")?
                    );
                }
                _ => {
                    if restamped.is_empty() {
                        println!("Nothing to restamp.");
                    } else {
                        let sha_display = sha.as_deref().unwrap_or("(unknown)");
                        for (kind, identifier, _branch) in &restamped {
                            println!(
                                "restamped {} {} -> {}",
                                kind_str(*kind),
                                identifier,
                                sha_display
                            );
                        }
                    }
                }
            }
        }
        ReviewCommand::Blocked { project } => {
            let project = paths::resolve_project(project, repo_config)?;
            let items = rdm_core::ops::review::blocked_phases(store, &project)
                .context("failed to list blocked phases")?;

            match format {
                OutputFormat::Json => {
                    let arr: Vec<_> = items
                        .iter()
                        .map(|item| {
                            serde_json::json!({
                                "identifier": item.identifier,
                                "project": item.project,
                                "title": item.title,
                                "reason": item.reason,
                            })
                        })
                        .collect();
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&arr)
                            .context("failed to serialize blocked phases")?
                    );
                }
                _ => {
                    if items.is_empty() {
                        println!("No blocked phases.");
                    } else {
                        for item in &items {
                            let reason = item.reason.as_deref().unwrap_or("(no reason recorded)");
                            println!("phase {}  {}  — {}", item.identifier, item.title, reason);
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
