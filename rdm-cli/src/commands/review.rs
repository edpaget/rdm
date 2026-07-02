use std::path::Path;

use anyhow::{Context, Result, bail};
use rdm_core::anchor::ResolvedComment;
use rdm_core::config::Config;
use rdm_core::document::Document;
use rdm_core::model::{PhaseStatus, Review, ReviewState, ReviewTarget, TaskStatus};
use rdm_core::ops::review::{PendingReviewItem, PendingReviewKind};
use rdm_core::ops::reviews::{AddComment, CreateReview, ReviewFilter, UpdateComment};
use rdm_core::ops::{BodyUpdate, TagsUpdate};
use rdm_core::{display, json};

use super::{commit_mutation, maybe_print_uncommitted_hint, resolve_body};
use crate::paths;
use crate::table;
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
                                        rdm_core::ops::TitleUpdate::Keep,
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
                                        rdm_core::ops::TitleUpdate::Keep,
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
        ReviewCommand::Start {
            on,
            author,
            project,
            body,
            no_edit,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            let author = paths::resolve_review_author(author)?;
            let body = resolve_body(body, no_edit)?;
            let target = rdm_core::ops::reviews::parse_review_target_ref(store, &project, &on)
                .context("failed to resolve review target")?;
            let doc = commit_mutation(
                store,
                &project,
                no_index,
                staging,
                "failed to start review",
                |s| {
                    rdm_core::ops::reviews::create_review(
                        s,
                        CreateReview {
                            project: &project,
                            author: &author,
                            target: target.clone(),
                            body: body.as_deref(),
                        },
                    )
                },
            )?;
            let id = doc.frontmatter.id.clone();
            match format {
                OutputFormat::Json => println!(
                    "{}",
                    serde_json::to_string_pretty(&json::review_to_json(&id, &doc, &[]))
                        .context("failed to serialize review")?
                ),
                _ => println!(
                    "Started review '{id}' on {} (draft)",
                    doc.frontmatter.target.label()
                ),
            }
        }
        ReviewCommand::Comment {
            review_id,
            quote,
            occurrence,
            doc,
            body,
            no_edit,
            project,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            let review_doc = rdm_core::ops::reviews::get_review(store, &project, &review_id)
                .context("failed to load review")?;
            // Fail early with core's lifecycle error so a submitted review
            // rejects the comment before any quote-derivation work.
            if review_doc.frontmatter.state != ReviewState::Draft {
                return Err(anyhow::Error::new(rdm_core::error::Error::ReviewNotDraft(
                    review_id.clone(),
                )));
            }
            let doc_scope = match &doc {
                None => None,
                Some(reference) => match &review_doc.frontmatter.target {
                    ReviewTarget::Roadmap { roadmap } => Some(
                        rdm_core::ops::reviews::parse_comment_doc_ref(
                            store, &project, roadmap, reference,
                        )
                        .context("failed to resolve --doc")?,
                    ),
                    _ => {
                        return Err(anyhow::Error::new(
                            rdm_core::error::Error::CommentDocNotApplicable,
                        ));
                    }
                },
            };
            let anchor = match &quote {
                Some(q) => {
                    let (target_body, at) = rdm_core::anchor::body_for_comment(
                        store,
                        &project,
                        &review_doc.frontmatter,
                        doc_scope.as_ref(),
                    )
                    .context("failed to load the document the quote anchors into")?;
                    Some(rdm_core::anchor::derive_text_quote(
                        &target_body,
                        q,
                        occurrence,
                        at.as_deref(),
                    )?)
                }
                None => None,
            };
            let Some(body) = resolve_body(body, no_edit)?.filter(|b| !b.trim().is_empty()) else {
                bail!(
                    "comment body must not be empty — pass --body <text> or pipe content via stdin"
                );
            };
            let updated = commit_mutation(
                store,
                &project,
                no_index,
                staging,
                "failed to add comment",
                |s| {
                    rdm_core::ops::reviews::add_comment(
                        s,
                        AddComment {
                            project: &project,
                            review_id: &review_id,
                            body: &body,
                            doc: doc_scope,
                            anchor,
                        },
                    )
                },
            )?;
            let comment_id = updated
                .frontmatter
                .comments
                .last()
                .map(|c| c.id)
                .unwrap_or_default();
            println!("Added comment {comment_id} to review '{review_id}'");
        }
        ReviewCommand::Submit {
            review_id,
            verdict,
            body,
            no_edit,
            project,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            // A blank --body (empty or whitespace-only) is never persisted
            // as the summary. Against an existing non-empty summary it is
            // refused (the anti-clobber convention); otherwise it is
            // treated as "no summary given". The trim stays confined to
            // this call site (and the comment arm) instead of living in
            // `BodyUpdate::apply`, whose exact-emptiness semantics other
            // entities (task/phase/roadmap update) already depend on.
            let raw_body = resolve_body(body, no_edit)?;
            let blank_body_given = raw_body.as_deref().is_some_and(|b| b.trim().is_empty());
            let body = raw_body.filter(|b| !b.trim().is_empty());
            if blank_body_given {
                let existing = rdm_core::ops::reviews::get_review(store, &project, &review_id)
                    .context("failed to load review")?;
                if !existing.body.trim().is_empty() {
                    bail!(
                        "refusing to replace the review's non-empty summary with a blank --body — pass non-empty text, or omit --body to keep the existing summary"
                    );
                }
            }
            let doc = commit_mutation(
                store,
                &project,
                no_index,
                staging,
                "failed to submit review",
                |s| {
                    if let Some(b) = &body {
                        rdm_core::ops::reviews::set_summary(
                            s,
                            &project,
                            &review_id,
                            BodyUpdate::Set(b.clone()),
                        )?;
                    }
                    rdm_core::ops::reviews::submit_review(s, &project, &review_id, Some(verdict))
                },
            )
            .map_err(|e| map_blank_summary(e, blank_body_given))?;
            println!(
                "Submitted review '{review_id}' with verdict {}",
                doc.frontmatter
                    .verdict
                    .map(|v| v.to_string())
                    .unwrap_or_default()
            );
        }
        ReviewCommand::List {
            on,
            state,
            verdict,
            author,
            project,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            let target = match &on {
                Some(reference) => Some(
                    rdm_core::ops::reviews::parse_review_target_ref(store, &project, reference)
                        .context("failed to resolve --on target")?,
                ),
                None => None,
            };
            let reviews = rdm_core::ops::reviews::list_reviews(store, &project)
                .context("failed to list reviews")?;
            let filtered = rdm_core::ops::reviews::filter_reviews(
                reviews,
                &ReviewFilter {
                    target,
                    state,
                    verdict,
                    author,
                },
            );
            render_review_list(store, &project, &filtered, format, staging)?;
        }
        ReviewCommand::Requests { project } => {
            let project = paths::resolve_project(project, repo_config)?;
            let reviews = rdm_core::ops::reviews::list_reviews(store, &project)
                .context("failed to list reviews")?;
            let filtered = rdm_core::ops::reviews::filter_reviews(
                reviews,
                &ReviewFilter {
                    state: Some(ReviewState::Submitted),
                    verdict: Some(rdm_core::model::Verdict::RequestChanges),
                    ..Default::default()
                },
            );
            render_review_list(store, &project, &filtered, format, staging)?;
        }
        ReviewCommand::Show {
            review_id,
            no_body,
            project,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            let mut doc = rdm_core::ops::reviews::get_review(store, &project, &review_id)
                .context("failed to load review")?;
            let resolutions = resolve_all(store, &project, &doc);
            if no_body {
                doc.body = String::new();
                for comment in &mut doc.frontmatter.comments {
                    comment.body = String::new();
                }
            }
            match format {
                OutputFormat::Human => print!(
                    "{}",
                    display::format_review_detail(&review_id, &doc, &resolutions)
                ),
                OutputFormat::Markdown => print!(
                    "{}",
                    display::format_review_detail_md(&review_id, &doc, &resolutions)
                ),
                OutputFormat::Json => println!(
                    "{}",
                    serde_json::to_string_pretty(&json::review_to_json(
                        &review_id,
                        &doc,
                        &resolutions
                    ))
                    .context("failed to serialize review")?
                ),
                OutputFormat::Table => bail!(
                    "--format table is not supported for 'review show'; use --format human, --format json, --format markdown, or omit --format"
                ),
            }
            maybe_print_uncommitted_hint(store, staging);
        }
        ReviewCommand::Update {
            review_id,
            state,
            comment,
            status,
            applied_commit,
            reply,
            project,
        } => {
            if state.is_none() && comment.is_none() {
                bail!(
                    "nothing to update — pass --state <addressed|dismissed> and/or --comment <n> with --status/--applied-commit/--reply"
                );
            }
            if let Some(comment_id) = comment
                && status.is_none()
                && applied_commit.is_none()
                && reply.is_none()
            {
                bail!(
                    "pass at least one of --status, --applied-commit, or --reply with --comment {comment_id}"
                );
            }
            let project = paths::resolve_project(project, repo_config)?;
            let doc = commit_mutation(
                store,
                &project,
                no_index,
                staging,
                "failed to update review",
                |s| {
                    if let Some(comment_id) = comment {
                        rdm_core::ops::reviews::update_comment(
                            s,
                            UpdateComment {
                                project: &project,
                                review_id: &review_id,
                                comment_id,
                                status,
                                applied_commit: applied_commit.as_deref(),
                                reply: reply.as_deref(),
                                ..Default::default()
                            },
                        )?;
                    }
                    match state {
                        Some(transition) => rdm_core::ops::reviews::update_review(
                            s,
                            &project,
                            &review_id,
                            transition.into(),
                        ),
                        None => rdm_core::ops::reviews::get_review(s, &project, &review_id),
                    }
                },
            )?;
            println!(
                "Updated review '{review_id}' → state: {}",
                doc.frontmatter.state
            );
        }
        ReviewCommand::Delete {
            review_id,
            force,
            project,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            commit_mutation(
                store,
                &project,
                no_index,
                staging,
                "failed to delete review",
                |s| rdm_core::ops::reviews::delete_review(s, &project, &review_id, force),
            )
            .map_err(map_delete_not_draft)?;
            println!("Deleted review '{review_id}' from project '{project}'");
        }
    }
    Ok(())
}

/// Runs the shared resolution pass: one
/// [`resolve_comment`](rdm_core::anchor::resolve_comment) per comment, in
/// comment order. The same slice feeds the JSON, human, and markdown
/// renderers.
fn resolve_all(store: &AppStore, project: &str, doc: &Document<Review>) -> Vec<ResolvedComment> {
    doc.frontmatter
        .comments
        .iter()
        .map(|c| rdm_core::anchor::resolve_comment(store, project, &doc.frontmatter, c))
        .collect()
}

/// Renders a filtered review list in the requested format.
fn render_review_list(
    store: &AppStore,
    project: &str,
    reviews: &[(String, Document<Review>)],
    format: OutputFormat,
    staging: bool,
) -> Result<()> {
    match format {
        OutputFormat::Json => {
            let arr: Vec<_> = reviews
                .iter()
                .map(|(id, doc)| {
                    let resolutions = resolve_all(store, project, doc);
                    json::review_to_json(id, doc, &resolutions)
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&arr).context("failed to serialize reviews")?
            );
        }
        OutputFormat::Table => print!("{}", table::format_review_table(reviews)),
        OutputFormat::Markdown => print!("{}", display::format_review_list_md(reviews)),
        OutputFormat::Human => print!("{}", display::format_review_list(reviews)),
    }
    maybe_print_uncommitted_hint(store, staging);
    Ok(())
}

/// Maps [`rdm_core::error::Error::ReviewEmpty`] into a message that names
/// `--body` when the user *did* pass one but it was whitespace-only (and
/// therefore treated as no summary) — otherwise the core message would
/// gaslight them with "no summary".
fn map_blank_summary(err: anyhow::Error, blank_body_given: bool) -> anyhow::Error {
    if blank_body_given
        && let Some(rdm_core::error::Error::ReviewEmpty(id)) =
            err.downcast_ref::<rdm_core::error::Error>()
    {
        return anyhow::anyhow!(
            "review '{id}' has no comments, and the --body you passed is blank — pass non-empty summary text to --body (or add a comment first)"
        );
    }
    err
}

/// Maps [`rdm_core::error::Error::ReviewNotDraft`] from a delete into a
/// message that names `--force` (the generic draft-state message doesn't).
fn map_delete_not_draft(err: anyhow::Error) -> anyhow::Error {
    if let Some(rdm_core::error::Error::ReviewNotDraft(id)) =
        err.downcast_ref::<rdm_core::error::Error>()
    {
        return anyhow::anyhow!(
            "review '{id}' has been submitted and is part of the record — pass --force to delete it anyway"
        );
    }
    err
}

/// Lowercase label for a pending-review item's kind.
fn kind_str(kind: PendingReviewKind) -> &'static str {
    match kind {
        PendingReviewKind::Phase => "phase",
        PendingReviewKind::Task => "task",
    }
}
