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
                            commit_mutation(store, "failed to restamp phase", |s| {
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
                            })?;
                        }
                        PendingReviewKind::Task => {
                            let slug = identifier.clone();
                            let sha = sha.clone();
                            let branch = target_branch.clone();
                            commit_mutation(store, "failed to restamp task", |s| {
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
                            })?;
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
            base,
            implements,
            author,
            project,
            body,
            no_edit,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            let author = paths::resolve_review_author(author)?;
            let body = resolve_body(body, no_edit)?;
            let parsed = rdm_core::ops::reviews::parse_review_target_ref(store, &project, &on)
                .context("failed to resolve review target")?;
            let is_change = matches!(parsed, ReviewTarget::Change { .. });
            if !is_change {
                if base.is_some() {
                    bail!(
                        "--base only applies to a change review — pass --on change/<sha> to use it"
                    );
                }
                if implements.is_some() {
                    return Err(anyhow::Error::new(
                        rdm_core::error::Error::ReviewImplementsNotApplicable(parsed.label()),
                    ));
                }
            }
            let (target, change_branch) = if is_change {
                resolve_change_target(store, &project, repo_config, &parsed, base.as_deref())?
            } else {
                (parsed, None)
            };
            let implements_ref = if is_change {
                resolve_implements(store, &project, implements.as_deref())?
            } else {
                None
            };
            let doc = commit_mutation(store, "failed to start review", |s| {
                rdm_core::ops::reviews::create_review(
                    s,
                    CreateReview {
                        project: &project,
                        author: &author,
                        target: target.clone(),
                        body: body.as_deref(),
                        implements: implements_ref.clone(),
                        change_branch: change_branch.clone(),
                    },
                )
            })?;
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
            path,
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
            let anchor = if let ReviewTarget::Change { head, base } = &review_doc.frontmatter.target
            {
                derive_change_anchor(
                    store,
                    &project,
                    head,
                    base.as_deref(),
                    quote.as_deref(),
                    path.as_deref(),
                    occurrence,
                )?
            } else {
                if path.is_some() {
                    bail!(
                        "--path only applies to a change review — review '{review_id}' targets {}, whose quotes are located in the plan-repo document itself",
                        review_doc.frontmatter.target.label()
                    );
                }
                match &quote {
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
                }
            };
            let Some(body) = resolve_body(body, no_edit)?.filter(|b| !b.trim().is_empty()) else {
                bail!(
                    "comment body must not be empty — pass --body <text> or pipe content via stdin"
                );
            };
            let updated = commit_mutation(store, "failed to add comment", |s| {
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
            })?;
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
            let doc = commit_mutation(store, "failed to submit review", |s| {
                if let Some(b) = &body {
                    rdm_core::ops::reviews::set_summary(
                        s,
                        &project,
                        &review_id,
                        BodyUpdate::Set(b.clone()),
                    )?;
                }
                rdm_core::ops::reviews::submit_review(s, &project, &review_id, Some(verdict))
            })
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
                    ..Default::default()
                },
            );
            render_review_list(store, &project, &filtered, format)?;
        }
        ReviewCommand::Requests { project } => {
            let project = paths::resolve_project(project, repo_config)?;
            let filtered = rdm_core::ops::reviews::change_requests(store, &project)
                .context("failed to list change requests")?;
            render_review_list(store, &project, &filtered, format)?;
        }
        ReviewCommand::Show {
            review_id,
            no_body,
            project,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            let mut doc = rdm_core::ops::reviews::get_review(store, &project, &review_id)
                .context("failed to load review")?;
            let (resolutions, source_note) = resolve_all(store, &project, &doc);
            if no_body {
                doc.body = String::new();
                for comment in &mut doc.frontmatter.comments {
                    comment.body = String::new();
                }
            }
            match format {
                OutputFormat::Human => print!(
                    "{}",
                    display::format_review_detail(
                        &review_id,
                        &doc,
                        &resolutions,
                        source_note.as_deref()
                    )
                ),
                OutputFormat::Markdown => print!(
                    "{}",
                    display::format_review_detail_md(
                        &review_id,
                        &doc,
                        &resolutions,
                        source_note.as_deref()
                    )
                ),
                OutputFormat::Json => println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &json::review_to_json(&review_id, &doc, &resolutions)
                            .with_source_note(source_note)
                    )
                    .context("failed to serialize review")?
                ),
                OutputFormat::Table => bail!(
                    "--format table is not supported for 'review show'; use --format human, --format json, --format markdown, or omit --format"
                ),
            }
            maybe_print_uncommitted_hint(store);
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
            let doc = commit_mutation(store, "failed to update review", |s| {
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
            })?;
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
            commit_mutation(store, "failed to delete review", |s| {
                rdm_core::ops::reviews::delete_review(s, &project, &review_id, force)
            })
            .map_err(map_delete_not_draft)?;
            println!("Deleted review '{review_id}' from project '{project}'");
        }
    }
    Ok(())
}

/// Runs the shared resolution pass and returns it alongside an optional
/// note explaining why source-repo verification was skipped.
///
/// Dispatches on the review's target kind: a plan-repo target resolves
/// through [`resolve_comments`](rdm_core::anchor::resolve_comments), while a
/// `change/<sha>` target resolves through
/// [`rdm_core::change::resolve_change_comments`] against the discovered
/// source repository.
///
/// The read path **degrades, never fails**: with no source repo reachable
/// every change comment comes back unresolved and the note says so — the
/// same policy `rdm link check`'s `path_verification_skipped` established.
/// (The write path — `review comment --path` — fails loudly instead; a new
/// anchor derived against nothing would be a lie.)
fn resolve_all(
    store: &AppStore,
    project: &str,
    doc: &Document<Review>,
) -> (Vec<ResolvedComment>, Option<String>) {
    if !matches!(doc.frontmatter.target, ReviewTarget::Change { .. }) {
        return (
            rdm_core::anchor::resolve_comments(store, project, &doc.frontmatter),
            None,
        );
    }
    let unresolved = || {
        doc.frontmatter
            .comments
            .iter()
            .map(|_| ResolvedComment {
                resolution: rdm_core::anchor::Resolution::Unresolved,
                quote: None,
            })
            .collect::<Vec<_>>()
    };
    #[cfg(feature = "git")]
    {
        use rdm_core::source::SourceRepo;
        let source = match crate::source_repo::discover_source_repo(store, project) {
            Ok(source) => source,
            Err(e) => return (unresolved(), Some(e.to_string())),
        };
        // Drift is measured against the branch the change was on when the
        // review started; a deleted/renamed branch (or a detached-HEAD
        // review) degrades to the repository's current HEAD.
        let tip = doc
            .frontmatter
            .change_branch
            .as_deref()
            .and_then(|b| source.rev_parse(b).ok().flatten())
            .or_else(|| source.head().ok().flatten());
        let Some(tip) = tip else {
            return (
                unresolved(),
                Some(
                    "the source repository has no resolvable HEAD — anchor resolution skipped"
                        .to_string(),
                ),
            );
        };
        (
            rdm_core::change::resolve_change_comments(&source, &doc.frontmatter, &tip),
            None,
        )
    }
    #[cfg(not(feature = "git"))]
    {
        (
            unresolved(),
            Some("this build has no git support — anchor resolution skipped".to_string()),
        )
    }
}

/// Resolves a parsed `change/<rev>` target into a stored one: a full
/// 40-character head SHA plus the base the change is diffed against.
///
/// `base` precedence: an explicit `--base` (rev-parsed), else the merge-base
/// of the head with the project's default branch, itself resolved
/// `project.source.default_branch` → the repo's `rdm.toml`
/// `default_branch` → `"main"`.
///
/// Also returns the source checkout's current branch, stamped on the review
/// so later drift is measured against the branch the change was on rather
/// than whatever HEAD happens to be. `None` on a detached HEAD.
fn resolve_change_target(
    store: &AppStore,
    project: &str,
    repo_config: &Config,
    parsed: &ReviewTarget,
    base: Option<&str>,
) -> Result<(ReviewTarget, Option<String>)> {
    #[cfg(not(feature = "git"))]
    {
        let _ = (store, project, repo_config, parsed, base);
        bail!("this build has no git support — `change/` reviews require the `git` feature");
    }
    #[cfg(feature = "git")]
    {
        use rdm_core::source::SourceRepo;
        let ReviewTarget::Change { head: rev, .. } = parsed else {
            bail!("internal: resolve_change_target called on a non-change target");
        };
        let source = crate::source_repo::discover_source_repo(store, project)?;
        let head = source
            .rev_parse(rev)
            .context("failed to resolve the reviewed revision")?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "'{rev}' does not name a commit in the source repository — pass --on change/<sha>, a branch name, or change/HEAD from inside the checkout"
                )
            })?;

        let base = match base {
            Some(explicit) => source
                .rev_parse(explicit)
                .context("failed to resolve --base")?
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "--base '{explicit}' does not name a commit in the source repository"
                    )
                })?,
            None => {
                let default_branch = default_source_branch(store, project, repo_config);
                source
                    .merge_base(&head, &default_branch)
                    .context("failed to compute the change's merge base")?
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "no merge base between {head} and '{default_branch}' — unrelated history, an orphan branch, or a shallow clone; pass --base <rev> to name the revision the change is diffed against"
                        )
                    })?
            }
        };
        let branch = source.current_branch().ok().flatten();
        Ok((
            ReviewTarget::Change {
                head,
                base: Some(base),
            },
            branch,
        ))
    }
}

/// The branch a change's merge-base is computed against:
/// `project.source.default_branch` → the plan repo's `rdm.toml`
/// `default_branch` → `"main"`.
fn default_source_branch(store: &AppStore, project: &str, repo_config: &Config) -> String {
    rdm_core::io::load_project(store, project)
        .ok()
        .and_then(|doc| doc.frontmatter.source)
        .and_then(|source| source.default_branch)
        .or_else(|| repo_config.default_branch.clone())
        .unwrap_or_else(|| "main".to_string())
}

/// Resolves `--implements` for a `change/` review: the explicit reference
/// when given, otherwise inferred from the worktree the command is running
/// in.
///
/// Inference requires the checkout to map to a plan item (via
/// [`rdm_git::worktree::current`]) that has **exactly one** `approved`
/// plan. Zero and many are both actionable errors naming `--implements`,
/// with the candidates listed in the many case. A roadmap-level worktree
/// has no single item to look plans up for, so it errors too rather than
/// guessing across the roadmap's phases.
fn resolve_implements(
    store: &AppStore,
    project: &str,
    explicit: Option<&str>,
) -> Result<Option<ReviewTarget>> {
    if let Some(raw) = explicit {
        let stripped = raw.strip_prefix("rdm:").unwrap_or(raw);
        let parsed: ReviewTarget = stripped.parse().map_err(|_| {
            anyhow::anyhow!("invalid --implements '{raw}' — expected rdm:plan/<slug>")
        })?;
        if !matches!(parsed, ReviewTarget::Plan { .. }) {
            return Err(anyhow::Error::new(
                rdm_core::error::Error::ReviewImplementsInvalidKind(parsed.label()),
            ));
        }
        return Ok(Some(parsed));
    }
    #[cfg(not(feature = "git"))]
    {
        let _ = (store, project);
        bail!(
            "--implements is required in a build without git support — pass --implements rdm:plan/<slug>"
        );
    }
    #[cfg(feature = "git")]
    {
        let cwd = std::env::current_dir().context("failed to read the current directory")?;
        let current = rdm_git::worktree::current(&cwd).ok().flatten();
        let Some(current) = current else {
            bail!(
                "not inside a recognized phase or task worktree, so the implemented plan cannot be inferred — pass --implements rdm:plan/<slug>"
            );
        };
        let item = match rdm_git::worktree::ItemRef::parse(&current.item) {
            Ok(rdm_git::worktree::ItemRef::Phase { roadmap, stem }) => {
                ReviewTarget::Phase { roadmap, stem }
            }
            Ok(rdm_git::worktree::ItemRef::Task { slug }) => ReviewTarget::Task { slug },
            Ok(rdm_git::worktree::ItemRef::Roadmap { roadmap }) => bail!(
                "this worktree covers the whole roadmap '{roadmap}', which has no single plan to infer — pass --implements rdm:plan/<slug>"
            ),
            Err(e) => bail!(
                "could not read this worktree's plan item ({e}) — pass --implements rdm:plan/<slug>"
            ),
        };
        let approved = rdm_core::ops::plan::approved_plans_for(store, project, &item)
            .context("failed to list approved plans for this worktree's item")?;
        match approved.as_slice() {
            [(slug, _)] => Ok(Some(ReviewTarget::Plan { slug: slug.clone() })),
            [] => bail!(
                "no approved plan for {} — pass --implements rdm:plan/<slug>, or approve one (`rdm plan list --implements {}`)",
                item.label(),
                item.label()
            ),
            many => {
                let candidates: Vec<String> = many
                    .iter()
                    .map(|(slug, _)| format!("rdm:plan/{slug}"))
                    .collect();
                bail!(
                    "{} has {} approved plans, so the implemented plan is ambiguous — pass --implements with one of: {}",
                    item.label(),
                    many.len(),
                    candidates.join(", ")
                )
            }
        }
    }
}

/// Derives a `change/` review comment's anchor from `--path`/`--quote`.
///
/// Requires `--path` alongside `--quote` (a change review has no single
/// document to search), reads the file's content at the reviewed `head`,
/// computes the hunks the change touches in it, and hands both to
/// [`rdm_core::change::derive_file_quote`], which enforces the
/// inside-a-touched-hunk rule.
///
/// Unlike the read path, this **fails loudly** when the source repository
/// is unreachable: a stored anchor that was never checked against real
/// content would silently mislead every later reader.
fn derive_change_anchor(
    store: &AppStore,
    project: &str,
    head: &str,
    base: Option<&str>,
    quote: Option<&str>,
    path: Option<&str>,
    occurrence: Option<usize>,
) -> Result<Option<rdm_core::model::Anchor>> {
    let Some(quote) = quote else {
        if path.is_some() {
            bail!("--path needs --quote — pass both, or neither for a whole-change comment");
        }
        return Ok(None);
    };
    let Some(path) = path else {
        bail!(
            "--quote on a change review needs --path <repo-relative path> naming the file the quote lives in"
        );
    };
    let path = rdm_core::change::normalize_source_path(path)?;
    #[cfg(not(feature = "git"))]
    {
        let _ = (store, project, head, base, quote, occurrence);
        bail!("this build has no git support — `change/` reviews require the `git` feature");
    }
    #[cfg(feature = "git")]
    {
        use rdm_core::source::SourceRepo;
        let source = crate::source_repo::discover_source_repo(store, project)?;
        let content = source
            .file_at(head, &path)
            .context("failed to read the quoted file at the reviewed head")?
            .ok_or_else(|| {
                anyhow::Error::new(rdm_core::error::Error::ChangePathNotInRevision {
                    path: path.clone(),
                    rev: head.to_string(),
                })
            })?;
        let (hunks, range_label) = match base {
            Some(base) => {
                let diff = source
                    .unified_diff(base, head, &path)
                    .context("failed to diff the quoted file")?
                    .unwrap_or_default();
                (
                    rdm_core::change::parse_hunks(&diff),
                    format!(
                        "{}..{}",
                        &base[..base.len().min(12)],
                        &head[..head.len().min(12)]
                    ),
                )
            }
            // A review with no recorded base (hand-edited frontmatter)
            // cannot compute hunks; treat the whole file as untouched so the
            // error names the real problem rather than anchoring blindly.
            None => (Vec::new(), "this change".to_string()),
        };
        Ok(Some(rdm_core::change::derive_file_quote(
            &content,
            &path,
            quote,
            occurrence,
            &hunks,
            &range_label,
        )?))
    }
}

/// Renders a filtered review list in the requested format.
fn render_review_list(
    store: &AppStore,
    project: &str,
    reviews: &[(String, Document<Review>)],
    format: OutputFormat,
) -> Result<()> {
    match format {
        OutputFormat::Json => {
            let arr: Vec<_> = reviews
                .iter()
                .map(|(id, doc)| {
                    let (resolutions, source_note) = resolve_all(store, project, doc);
                    json::review_to_json(id, doc, &resolutions).with_source_note(source_note)
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
    maybe_print_uncommitted_hint(store);
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
