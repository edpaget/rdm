use anyhow::{Context, Result, bail};
use rdm_core::config::Config;
use rdm_core::display;
use rdm_core::json;
use rdm_core::ops::{BodyUpdate, ReasonUpdate, TagsUpdate, TitleUpdate};

use super::{commit_mutation, map_body_clobber, maybe_print_uncommitted_hint, resolve_body};
use crate::commands;
use crate::paths;
use crate::table;
use crate::{AppStore, OutputFormat, TaskCommand};

pub fn run(
    command: TaskCommand,
    store: &mut AppStore,
    root: &std::path::Path,
    repo_config: &Config,
    format: OutputFormat,
) -> Result<()> {
    match command {
        TaskCommand::Create {
            slug,
            title,
            project,
            priority,
            tags,
            body,
            no_edit,
            no_plan_review,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            let title = title.as_deref().unwrap_or(&slug);
            let body = resolve_body(body, no_edit)?;
            let plan_review = paths::resolve_plan_review(repo_config)?;
            let tags = rdm_core::tags::stamp_plan_review_tag(tags, plan_review && !no_plan_review);
            commit_mutation(store, "failed to create task", |s| {
                rdm_core::ops::task::create_task(
                    s,
                    rdm_core::ops::task::CreateTask {
                        project: &project,
                        slug: &slug,
                        title,
                        priority,
                        tags,
                        body: body.as_deref(),
                    },
                )
            })?;
            println!("Created task '{slug}' in project '{project}'");
        }
        TaskCommand::Show {
            slug,
            project,
            no_body,
            at,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            let mut doc = match at.as_deref() {
                Some(sha) => rdm_core::io::load_task_at(store, &project, &slug, sha)
                    .with_context(|| format!("failed to load task '{slug}' at revision '{sha}'"))?,
                None => rdm_core::io::load_task(store, &project, &slug)
                    .context("failed to load task")?,
            };
            if no_body {
                doc.body = String::new();
            }
            // Implementation plans implementing this task. Under `--at <sha>`
            // only the *body* is historical — the plan list always reflects
            // current state, matching `Revision:` semantics.
            let plans = rdm_core::ops::plan::plans_implementing(
                store,
                &project,
                &rdm_core::link::ItemRef::Task { slug: slug.clone() },
            )
            .context("failed to list plans implementing this task")?;

            let revision = at.as_deref();
            match format {
                OutputFormat::Human => print!(
                    "{}",
                    display::format_task_detail_with_plans(&slug, &doc, revision, &plans)
                ),
                OutputFormat::Markdown => print!(
                    "{}",
                    display::format_task_detail_md_with_plans(&slug, &doc, revision, &plans)
                ),
                OutputFormat::Json => {
                    let j = json::task_to_json(&slug, &doc, revision).with_plans(
                        plans
                            .iter()
                            .map(|(s, d)| json::plan_ref_to_json(s, d))
                            .collect(),
                    );
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&j).context("failed to serialize task")?
                    );
                }
                OutputFormat::Table => bail!(
                    "--format table is not supported for 'task show'; use --format human, --format json, --format markdown, or omit --format"
                ),
            }
            maybe_print_uncommitted_hint(store);
        }
        TaskCommand::Update {
            slug,
            project,
            title,
            status,
            priority,
            tags,
            body,
            clear_body,
            commit,
            reason,
            clear_reason,
            override_gate,
            source,
            no_edit: _,
        } => {
            // Reject an override on a transition the gate never guards, rather
            // than silently ignoring it.
            if override_gate.is_some() && status != Some(rdm_core::model::TaskStatus::Reviewed) {
                anyhow::bail!(
                    "--override-gate can only be used with --status reviewed — it bypasses the \
                     `reviewed` transition gate, and no other transition is gated"
                );
            }
            let project = paths::resolve_project(project, repo_config)?;
            let title = TitleUpdate::from_args(title);
            // `update` consults the body only when the user is explicit:
            // `--body` sets it, `--clear-body` clears it, otherwise it is left
            // untouched. Unlike `create`, this never reads stdin or opens the
            // editor, so a tags-only/status-only update can't hang on an open
            // pipe or clobber the body from stray stdin bytes. This is
            // load-bearing for the `Done:` hook: the installed post-merge/
            // post-commit hooks run `task update --status done --no-edit` as git
            // subprocesses with inherited pipes — exactly what would hang before.
            let body = BodyUpdate::from_args(body, clear_body)?;
            let tags = TagsUpdate::from_args(tags, false)?;
            let reason_update = ReasonUpdate::from_args(reason, clear_reason)?;
            let has_reason = !matches!(reason_update, ReasonUpdate::Keep);
            let explicit_source = source.source.is_some()
                || source.base.is_some()
                || source.expected_head.is_some()
                || source.expected_branch.is_some()
                || source.no_code;
            if explicit_source && status.is_none() {
                anyhow::bail!("source binding requires an explicit --status");
            }
            #[cfg(not(feature = "git"))]
            if explicit_source {
                anyhow::bail!("explicit source binding requires git support");
            }
            #[cfg(feature = "git")]
            let source_binding = if explicit_source {
                Some(commands::resolve_source_args(
                    store,
                    &project,
                    &source,
                    &rdm_core::link::ItemRef::Task { slug: slug.clone() },
                    repo_config.default_branch.as_deref().unwrap_or("main"),
                    Some(root),
                )?)
            } else {
                None
            };
            // Stamp the source-repo HEAD SHA when entering needs-review, so the
            // review can later be scoped to the branch/worktree that produced
            // it. No commit yet (unstamped) → fail open downstream.
            #[cfg(feature = "git")]
            let review_sha = if status == Some(rdm_core::model::TaskStatus::NeedsReview) {
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
            let review_branch = if status == Some(rdm_core::model::TaskStatus::NeedsReview) {
                rdm_git::current_branch_at(&std::env::current_dir()?)
                    .ok()
                    .flatten()
            } else {
                None
            };
            #[cfg(not(feature = "git"))]
            let review_branch = None;
            #[cfg(feature = "git")]
            let review_sha = source_binding
                .as_ref()
                .filter(|_| {
                    review_sha.is_some()
                        || status.map(|s| s.to_string()) == Some("needs-review".into())
                })
                .map(|(_, source)| source.head.clone())
                .or(review_sha);
            #[cfg(feature = "git")]
            let review_branch = source_binding
                .as_ref()
                .filter(|_| review_sha.is_some())
                .map(|(_, source)| source.branch.clone())
                .or(review_branch);

            // Write-once: resolve the task's starting HEAD the first time it
            // enters `in-progress`, so `rdm review source`'s default base (the
            // recorded `started_head`) reviews exactly this task's own
            // commits. An explicit `--source <path>` binds to that
            // checkout's HEAD directly; otherwise resolve the task's
            // registered worktree the same way `rdm verify run --item`
            // does. Best-effort: no worktree yet (or none resolvable)
            // records nothing rather than failing the status update — the
            // write-once apply leaves an already-recorded value untouched
            // regardless.
            #[cfg(feature = "git")]
            let started_head = if status == Some(rdm_core::model::TaskStatus::InProgress) {
                source_binding
                    .as_ref()
                    .map(|(_, source)| source.head.clone())
                    .or_else(|| {
                        let cwd = std::env::current_dir().ok()?;
                        let repo =
                            rdm_git::worktree::discover_distinct_project_repo(&cwd, root).ok()?;
                        let item = rdm_git::worktree::ItemRef::Task { slug: slug.clone() };
                        let worktree = rdm_git::worktree::registered_worktree_for(&repo, &item)
                            .ok()
                            .flatten()?;
                        rdm_git::head_commit_info_at(&worktree.path)
                            .ok()
                            .flatten()
                            .map(|c| c.sha)
                    })
            } else {
                None
            };
            #[cfg(not(feature = "git"))]
            let started_head = None;

            // Data-integrity guard: warn (non-blocking) when a task reaches
            // needs-review with no committed diff beyond the default branch.
            // Tasks have no sibling phases, so they always use this baseline.
            #[cfg(feature = "git")]
            let needs_review_warning: Option<String> = review_sha.as_deref().and_then(|sha| {
                let default_branch = repo_config.default_branch.as_deref().unwrap_or("main");
                if review_branch.as_deref() == Some(default_branch) {
                    return None;
                }
                let cwd = std::env::current_dir().ok()?;
                match rdm_git::is_ancestor_of_branch_at(&cwd, default_branch, sha) {
                    Ok(true) => Some(format!(
                        "warning: task '{slug}' is now needs-review, but HEAD has no commits beyond '{default_branch}' — there may be nothing to review. Confirm this task's work was committed before finalizing."
                    )),
                    _ => None,
                }
            });
            #[cfg(not(feature = "git"))]
            let needs_review_warning: Option<String> = None;
            // Build the `reviewed` transition gate ABOVE the mutation, and
            // after the needs-review warning is computed, so a refusal cannot
            // change whether that warning exists.
            let gate_enabled = paths::resolve_reviewed_gate(repo_config)?;
            let gate_actor = if override_gate.is_some() {
                Some(paths::resolve_review_author(None)?)
            } else {
                None
            };
            // `build_gate_probe` holds the one git/non-git split, so this arm
            // needs no second code path. It degrades to `None` — precondition
            // (c) skipped, (a) and (b) still enforced — whenever rdm cannot
            // resolve a worktree.
            let gate_probe: Option<commands::GateProbe> =
                commands::build_gate_probe(gate_enabled, root);
            #[cfg(feature = "git")]
            let gate_probe = source_binding.map(|(probe, _)| probe).or(gate_probe);

            let gate = commands::build_reviewed_gate(
                gate_enabled,
                gate_probe.as_ref(),
                override_gate.as_deref(),
                gate_actor.as_deref(),
            );
            let doc = commit_mutation(store, "failed to update task", |s| {
                let mut doc = rdm_core::ops::task::update_task_gated(
                    s,
                    &project,
                    &slug,
                    status,
                    priority,
                    tags,
                    body,
                    commit,
                    review_sha,
                    review_branch,
                    started_head,
                    title,
                    &gate,
                )?;
                if has_reason {
                    doc = rdm_core::ops::task::set_task_close_reason(
                        s,
                        &project,
                        &slug,
                        reason_update,
                    )?;
                }
                Ok(doc)
            })
            .map_err(map_body_clobber)?;
            println!(
                "Updated task '{slug}' → status: {}, priority: {}",
                doc.frontmatter.status, doc.frontmatter.priority
            );
            if let Some(warning) = needs_review_warning {
                eprintln!("{warning}");
            }
        }
        TaskCommand::Merge {
            survivor,
            from,
            project,
            no_edit: _,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            let (_survivor_doc, closed) = commit_mutation(store, "failed to merge tasks", |s| {
                rdm_core::ops::task::merge_tasks(s, &project, &survivor, &from)
            })?;
            // Report the count actually folded this call (deduped, minus any
            // already-superseded sources skipped as a no-op), not the raw input.
            let folded = closed.len();
            if folded == 0 {
                println!(
                    "No tasks merged into '{survivor}' — all specified sources were already folded."
                );
            } else {
                println!("Merged {folded} task(s) into '{survivor}'.");
            }
        }
        TaskCommand::List {
            project,
            status,
            priority,
            tags,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            let all_tasks =
                rdm_core::ops::task::list_tasks(store, &project).context("failed to list tasks")?;

            let filtered = rdm_core::ops::task::filter_tasks(
                all_tasks,
                &rdm_core::ops::task::TaskFilter {
                    status,
                    priority,
                    tags,
                },
            );

            match format {
                OutputFormat::Human => print!("{}", display::format_task_list(&filtered)),
                OutputFormat::Table => print!("{}", table::format_task_table(&filtered)),
                OutputFormat::Markdown => {
                    print!("{}", display::format_task_list_md(&filtered))
                }
                OutputFormat::Json => {
                    let summaries: Vec<_> = filtered
                        .iter()
                        .map(|(slug, doc)| json::task_summary_to_json(slug, doc))
                        .collect();
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&summaries)
                            .context("failed to serialize tasks")?
                    );
                }
            }
            maybe_print_uncommitted_hint(store);
        }
    }
    Ok(())
}
