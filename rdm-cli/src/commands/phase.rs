use anyhow::{Context, Result, bail};
use rdm_core::config::Config;
use rdm_core::display;
use rdm_core::json;
use rdm_core::ops::{BodyUpdate, TagsUpdate};

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

pub fn run(
    command: PhaseCommand,
    store: &mut AppStore,
    repo_config: &Config,
    format: OutputFormat,
    no_index: bool,
    staging: bool,
) -> Result<()> {
    match command {
        PhaseCommand::Create {
            slug,
            title,
            roadmap,
            project,
            number,
            tags,
            body,
            no_edit,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            let title = title.as_deref().unwrap_or(&slug);
            let body = resolve_body(body, no_edit)?;
            let doc = commit_mutation(
                store,
                &project,
                no_index,
                staging,
                "failed to create phase",
                |s| {
                    rdm_core::ops::phase::create_phase(
                        s,
                        &project,
                        &roadmap,
                        &slug,
                        title,
                        number,
                        body.as_deref(),
                        tags,
                    )
                },
            )?;
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
            maybe_print_uncommitted_hint(store, staging);
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
            maybe_print_uncommitted_hint(store, staging);
        }
        PhaseCommand::Update {
            stem,
            status,
            roadmap,
            project,
            tags,
            body,
            clear_body,
            commit,
            no_edit,
        } => {
            if commit.is_some() && !matches!(status, Some(s) if s.is_terminal()) {
                anyhow::bail!("--commit can only be used with --status done or --status wont-fix");
            }
            let project = paths::resolve_project(project, repo_config)?;
            let stem = rdm_core::ops::phase::resolve_phase_stem(store, &project, &roadmap, &stem)
                .context("failed to resolve phase")?;
            let body = if clear_body {
                BodyUpdate::Clear
            } else {
                BodyUpdate::from_args(resolve_body(body, no_edit)?, false)?
            };
            let tags = TagsUpdate::from_args(tags, false)?;
            // Stamp the source-repo HEAD SHA when entering needs-review, so the
            // review can later be scoped to the branch/worktree that produced
            // it. No commit yet (unstamped) → fail open downstream.
            #[cfg(feature = "git")]
            let review_sha = if status == Some(rdm_core::model::PhaseStatus::NeedsReview) {
                rdm_store_git::head_commit_info_at(&std::env::current_dir()?)
                    .ok()
                    .flatten()
                    .map(|c| c.sha)
            } else {
                None
            };
            #[cfg(not(feature = "git"))]
            let review_sha = None;
            let doc = commit_mutation(
                store,
                &project,
                no_index,
                staging,
                "failed to update phase",
                |s| {
                    rdm_core::ops::phase::update_phase(
                        s, &project, &roadmap, &stem, status, tags, body, commit, review_sha,
                    )
                },
            )
            .map_err(map_body_clobber)?;
            println!("Updated '{stem}' → {}", doc.frontmatter.status);
        }
        PhaseCommand::Remove {
            stem,
            roadmap,
            project,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            let stem = rdm_core::ops::phase::resolve_phase_stem(store, &project, &roadmap, &stem)
                .context("failed to resolve phase")?;
            commit_mutation(
                store,
                &project,
                no_index,
                staging,
                "failed to remove phase",
                |s| rdm_core::ops::phase::remove_phase(s, &project, &roadmap, &stem),
            )?;
            println!("Removed phase '{stem}' from roadmap '{roadmap}'");
        }
    }
    Ok(())
}
