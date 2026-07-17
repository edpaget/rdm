use anyhow::{Context, Result, bail};
use rdm_core::config::Config;
use rdm_core::display;
use rdm_core::json;
use rdm_core::ops::{
    BodyUpdate, DifficultyUpdate, ModelTierUpdate, ReasonUpdate, TagsUpdate, TitleUpdate,
};

use super::{commit_mutation, map_body_clobber, maybe_print_uncommitted_hint, resolve_body};
use crate::paths;
use crate::table;
use crate::{AppStore, OutputFormat, PhaseCommand};

/// Builds the prev/next navigation footer for `phase show`.
///
/// A leading blank line is always emitted, followed by `Prev:`/`Next:` lines
/// for whichever neighbors exist. With `markdown` true, the lines use a `> …`
/// backtick blockquote. This CLI vocabulary deliberately lives here rather than
/// in `rdm-core`, so MCP/server output does not inherit CLI hints.
fn phase_nav_footer(
    prev: Option<&str>,
    next: Option<&str>,
    roadmap: &str,
    project: &str,
    markdown: bool,
) -> String {
    let mut out = String::from("\n");
    if markdown {
        if let Some(prev) = prev {
            out.push_str(&format!(
                "> Prev: `rdm phase show {prev} --roadmap {roadmap} --project {project}`\n"
            ));
        }
        if let Some(next) = next {
            out.push_str(&format!(
                "> Next: `rdm phase show {next} --roadmap {roadmap} --project {project}`\n"
            ));
        }
    } else {
        if let Some(prev) = prev {
            out.push_str(&format!(
                "Prev: rdm phase show {prev} --roadmap {roadmap} --project {project}\n"
            ));
        }
        if let Some(next) = next {
            out.push_str(&format!(
                "Next: rdm phase show {next} --roadmap {roadmap} --project {project}\n"
            ));
        }
    }
    out
}

/// Builds the empty-finalize data-integrity warning for a phase entering
/// `needs-review`, or `None` when HEAD carries a reviewable diff.
///
/// `review_sha` is the source-repo HEAD SHA being stamped, `review_branch` the
/// firing checkout's branch (if resolvable), and `default_branch` the configured
/// trunk. The check runs against the source repo discovered from the current
/// working directory.
///
/// Baseline selection:
/// - If the firing checkout is on `default_branch`, the check is skipped (a diff
///   "vs trunk" is degenerate there).
/// - Otherwise, the **nearest prior finalized sibling phase** in the same
///   roadmap is the baseline. rdm's one-worktree-per-roadmap model shares a
///   long-lived `roadmap/<slug>` branch that is never an ancestor of the default
///   branch, so once an earlier phase lands a commit the branch is permanently a
///   non-ancestor of trunk — making the default-branch baseline useless for
///   phases 2..N. The sibling is selected by **recency**, not phase number: among
///   all other stamped siblings (any phase number, not just lower ones) whose
///   recorded SHA (`review_sha`, falling back to `commit`) is reachable from
///   HEAD, the baseline is the one every other reachable candidate is an
///   ancestor of — i.e. the most-recently-finalized sibling. This matters under
///   out-of-order finalize: e.g. finalizing phase-3 before phase-2 means
///   phase-3's commit, not phase-1's, is the true nearest-prior baseline for
///   phase-2. If HEAD has not advanced past that sibling's commit (`review_sha
///   == sibling_sha`), nothing new was committed for this phase and we warn.
///   (Known limitation: an unrelated intervening commit on the shared roadmap
///   branch — e.g. a concurrent phase's work — can still suppress the warning,
///   since "any new commit since the baseline" is only a proxy for "this phase
///   produced a diff.")
/// - When there is no prior stamped sibling (the first finalized phase of a
///   roadmap), we fall back to the default-branch baseline: warn when HEAD has
///   no commits beyond `default_branch`.
///
/// Fails open (returns `None`, no warning) on any git error or when the working
/// directory cannot be read, so a transient git state never produces a false
/// positive.
#[cfg(feature = "git")]
fn empty_finalize_warning(
    store: &AppStore,
    project: &str,
    roadmap: &str,
    current_stem: &str,
    review_sha: &str,
    review_branch: Option<&str>,
    default_branch: &str,
) -> Option<String> {
    // On the default branch itself, "diff vs trunk" is degenerate — skip.
    if review_branch == Some(default_branch) {
        return None;
    }
    let cwd = std::env::current_dir().ok()?;

    let phases = rdm_core::ops::phase::list_phases(store, project, roadmap).ok()?;

    // Gather every OTHER stamped sibling's SHA (any phase number — finalize
    // order is not phase-number order) that is reachable from HEAD. Any git
    // error comparing reachability fails the whole check open.
    let mut candidates: Vec<String> = Vec::new();
    for (stem, doc) in &phases {
        if stem == current_stem {
            continue;
        }
        let Some(sha) = doc
            .frontmatter
            .review_sha
            .clone()
            .or_else(|| doc.frontmatter.commit.clone())
        else {
            continue;
        };
        match rdm_git::is_ancestor_of_head_at(&cwd, &sha) {
            Ok(true) => {
                if !candidates.contains(&sha) {
                    candidates.push(sha);
                }
            }
            Ok(false) => {}
            Err(_) => return None,
        }
    }

    // Among the reachable candidates, the nearest-prior one (most recently
    // finalized) is the candidate every OTHER candidate is an ancestor of —
    // i.e. no other candidate is its descendant. Pairwise ancestor checks fail
    // the whole check open on any git error.
    let mut prior_sibling_sha: Option<String> = None;
    'outer: for candidate in &candidates {
        for other in &candidates {
            if other == candidate {
                continue;
            }
            match rdm_git::is_ancestor_at(&cwd, other, candidate) {
                Ok(true) => {}
                Ok(false) => continue 'outer,
                Err(_) => return None,
            }
        }
        prior_sibling_sha = Some(candidate.clone());
        break;
    }

    if let Some(sibling_sha) = prior_sibling_sha {
        // HEAD has not advanced past the previous phase's commit → no new work.
        // (Known limitation: an unrelated intervening commit on the shared
        // roadmap branch — e.g. a concurrent phase's work — can suppress this
        // warning, since "any new commit since the baseline" is only a proxy
        // for "this phase produced a diff.")
        if review_sha == sibling_sha {
            return Some(format!(
                "warning: phase '{current_stem}' is now needs-review, but HEAD has no new commit since the previous finalized phase — there may be nothing to review. Confirm this phase's work was committed before finalizing."
            ));
        }
        return None;
    }

    // First finalized phase of the roadmap: fall back to the default-branch
    // baseline.
    match rdm_git::is_ancestor_of_branch_at(&cwd, default_branch, review_sha) {
        Ok(true) => Some(format!(
            "warning: phase '{current_stem}' is now needs-review, but HEAD has no commits beyond '{default_branch}' — there may be nothing to review. Confirm this phase's work was committed before finalizing."
        )),
        _ => None,
    }
}

pub fn run(
    command: PhaseCommand,
    store: &mut AppStore,
    repo_config: &Config,
    format: OutputFormat,
    no_index: bool,
) -> Result<()> {
    match command {
        PhaseCommand::Create {
            slug,
            title,
            roadmap,
            project,
            number,
            tags,
            difficulty,
            model,
            body,
            no_edit,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            let title = title.as_deref().unwrap_or(&slug);
            let body = resolve_body(body, no_edit)?;
            let difficulty_update = DifficultyUpdate::from_args(difficulty, false)?;
            let model_update = ModelTierUpdate::from_args(model, false)?;
            let plan_review = paths::resolve_plan_review(repo_config)?;
            let tags = rdm_core::tags::stamp_plan_review_tag(tags, plan_review);
            let doc = commit_mutation(store, &project, no_index, "failed to create phase", |s| {
                rdm_core::ops::phase::create_phase(
                    s,
                    rdm_core::ops::phase::CreatePhase {
                        project: &project,
                        roadmap: &roadmap,
                        slug: &slug,
                        title,
                        number,
                        body: body.as_deref(),
                        tags,
                        difficulty: difficulty_update,
                        model: model_update,
                    },
                )
            })?;
            let stem = doc.frontmatter.stem(&slug);
            println!("Created phase '{stem}' in roadmap '{roadmap}'");
        }
        PhaseCommand::List { roadmap, project } => {
            let project = paths::resolve_project(project, repo_config)?;
            let phases = rdm_core::ops::phase::list_phases(store, &project, &roadmap)
                .context("failed to list phases")?;
            match format {
                OutputFormat::Human => print!("{}", display::format_phase_list(&phases)),
                OutputFormat::Table => print!("{}", table::format_phase_table(&phases)),
                OutputFormat::Markdown => {
                    print!("{}", display::format_phase_list_md(&phases))
                }
                OutputFormat::Json => {
                    let summaries: Vec<_> = phases
                        .iter()
                        .map(|(stem, doc)| json::phase_summary_to_json(stem, doc))
                        .collect();
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&summaries)
                            .context("failed to serialize phases")?
                    );
                }
            }
            maybe_print_uncommitted_hint(store);
        }
        PhaseCommand::Show {
            stem,
            roadmap,
            project,
            no_body,
            at,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            let stem = rdm_core::ops::phase::resolve_phase_stem(store, &project, &roadmap, &stem)
                .context("failed to resolve phase")?;
            let mut doc = match at.as_deref() {
                Some(sha) => rdm_core::io::load_phase_at(store, &project, &roadmap, &stem, sha)
                    .with_context(|| {
                        format!("failed to load phase '{stem}' at revision '{sha}'")
                    })?,
                None => rdm_core::io::load_phase(store, &project, &roadmap, &stem)
                    .context("failed to load phase")?,
            };
            if no_body {
                doc.body = String::new();
            }

            // Compute prev/next phase stems for navigation
            let phases = rdm_core::ops::phase::list_phases(store, &project, &roadmap)
                .context("failed to list phases")?;
            let pos = phases.iter().position(|(s, _)| s == &stem);
            let prev_stem = pos.and_then(|i| {
                if i > 0 {
                    Some(phases[i - 1].0.as_str())
                } else {
                    None
                }
            });
            let next_stem = pos.and_then(|i| phases.get(i + 1).map(|(s, _)| s.as_str()));

            let revision = at.as_deref();
            match format {
                OutputFormat::Human => {
                    let mut out = display::format_phase_detail(&stem, &doc, revision);
                    out.push_str(&phase_nav_footer(
                        prev_stem, next_stem, &roadmap, &project, false,
                    ));
                    print!("{out}");
                }
                OutputFormat::Markdown => {
                    let mut out = display::format_phase_detail_md(&stem, &doc, revision);
                    out.push_str(&phase_nav_footer(
                        prev_stem, next_stem, &roadmap, &project, true,
                    ));
                    print!("{out}");
                }
                OutputFormat::Json => {
                    let j =
                        json::phase_to_json(&stem, &doc, &roadmap, prev_stem, next_stem, revision);
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&j).context("failed to serialize phase")?
                    );
                }
                OutputFormat::Table => bail!(
                    "--format table is not supported for 'phase show'; use --format human, --format json, --format markdown, or omit --format"
                ),
            }
            maybe_print_uncommitted_hint(store);
        }
        PhaseCommand::Update {
            stem,
            status,
            title,
            roadmap,
            project,
            tags,
            difficulty,
            clear_difficulty,
            model,
            clear_model,
            body,
            clear_body,
            reason,
            clear_reason,
            commit,
            no_edit: _,
        } => {
            if commit.is_some() && !matches!(status, Some(s) if s.is_terminal()) {
                anyhow::bail!("--commit can only be used with --status done or --status wont-fix");
            }
            let project = paths::resolve_project(project, repo_config)?;
            let stem = rdm_core::ops::phase::resolve_phase_stem(store, &project, &roadmap, &stem)
                .context("failed to resolve phase")?;
            // `update` consults the body only when the user is explicit:
            // `--body` sets it, `--clear-body` clears it, otherwise it is left
            // untouched. Unlike `create`, this never reads stdin or opens the
            // editor, so a tags-only/status-only update can't hang on an open
            // pipe or clobber the body from stray stdin bytes. (Retires the old
            // `update --tags x < body.md` form; compose `--body` with `--tags`.)
            let body = BodyUpdate::from_args(body, clear_body)?;
            let tags = TagsUpdate::from_args(tags, false)?;
            // Stamp the source-repo HEAD SHA when entering needs-review, so the
            // review can later be scoped to the branch/worktree that produced
            // it. No commit yet (unstamped) → fail open downstream.
            #[cfg(feature = "git")]
            let review_sha = if status == Some(rdm_core::model::PhaseStatus::NeedsReview) {
                rdm_git::head_commit_info_at(&std::env::current_dir()?)
                    .ok()
                    .flatten()
                    .map(|c| c.sha)
            } else {
                None
            };
            #[cfg(not(feature = "git"))]
            let review_sha = None;
            // Also stamp the firing checkout's branch so `review pending` can
            // scope by identity rather than by SHA reachability alone.
            #[cfg(feature = "git")]
            let review_branch = if status == Some(rdm_core::model::PhaseStatus::NeedsReview) {
                rdm_git::current_branch_at(&std::env::current_dir()?)
                    .ok()
                    .flatten()
            } else {
                None
            };
            #[cfg(not(feature = "git"))]
            let review_branch = None;
            // Data-integrity guard: if a phase reaches needs-review with no
            // committed diff worth reviewing, it would strand in review state
            // with nothing to review. Compute a non-blocking warning here (the
            // transition still proceeds).
            #[cfg(feature = "git")]
            let needs_review_warning: Option<String> = review_sha.as_deref().and_then(|sha| {
                let default_branch = repo_config.default_branch.as_deref().unwrap_or("main");
                empty_finalize_warning(
                    store,
                    &project,
                    &roadmap,
                    &stem,
                    sha,
                    review_branch.as_deref(),
                    default_branch,
                )
            });
            #[cfg(not(feature = "git"))]
            let needs_review_warning: Option<String> = None;
            let title_update = TitleUpdate::from_args(title);
            let difficulty_update = DifficultyUpdate::from_args(difficulty, clear_difficulty)?;
            let model_update = ModelTierUpdate::from_args(model, clear_model)?;
            let reason_update = ReasonUpdate::from_args(reason, clear_reason)?;
            let has_reason = !matches!(reason_update, ReasonUpdate::Keep);
            let doc = commit_mutation(store, &project, no_index, "failed to update phase", |s| {
                let mut doc = rdm_core::ops::phase::update_phase_with_estimate(
                    s,
                    &project,
                    &roadmap,
                    &stem,
                    status,
                    tags,
                    body,
                    commit,
                    review_sha,
                    review_branch,
                    difficulty_update,
                    model_update,
                    title_update,
                )?;
                if has_reason {
                    doc = rdm_core::ops::phase::set_phase_blocked_reason(
                        s,
                        &project,
                        &roadmap,
                        &stem,
                        reason_update,
                    )?;
                }
                Ok(doc)
            })
            .map_err(map_body_clobber)?;
            println!("Updated '{stem}' → {}", doc.frontmatter.status);
            if let Some(warning) = needs_review_warning {
                eprintln!("{warning}");
            }
        }
        PhaseCommand::Remove {
            stem,
            roadmap,
            project,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            let stem = rdm_core::ops::phase::resolve_phase_stem(store, &project, &roadmap, &stem)
                .context("failed to resolve phase")?;
            commit_mutation(store, &project, no_index, "failed to remove phase", |s| {
                rdm_core::ops::phase::remove_phase(s, &project, &roadmap, &stem)
            })?;
            println!("Removed phase '{stem}' from roadmap '{roadmap}'");
        }
    }
    Ok(())
}
