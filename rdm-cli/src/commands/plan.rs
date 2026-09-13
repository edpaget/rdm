//! `rdm plan` — implementation-plan porcelain.
//!
//! A plan's status is derived from reviews (`rdm review submit --verdict` on
//! `plan/<slug>`) and from a later plan's `--supersedes`, so this module
//! deliberately exposes no `--status` flag on `update`.

use anyhow::{Context, Result, bail};
use rdm_core::config::Config;
use rdm_core::display;
use rdm_core::json;
use rdm_core::link::ItemRef;
use rdm_core::model::ReviewTarget;
use rdm_core::ops::reviews::ReviewFilter;
use rdm_core::ops::{BodyUpdate, TitleUpdate};

use super::{commit_mutation, map_body_clobber, maybe_print_uncommitted_hint, resolve_body};
use crate::paths;
use crate::table;
use crate::{AppStore, OutputFormat, PlanCommand};

/// Parses an item reference (`phase/<roadmap>/<stem-or-number>`,
/// `task/<slug>`, `plan/<slug>`) given on the command line, with an
/// actionable error naming the accepted forms.
fn parse_item_ref(raw: &str, flag: &str) -> Result<ItemRef> {
    raw.parse::<ItemRef>().map_err(|e| {
        anyhow::anyhow!(
            "{flag} '{raw}' is not a valid item reference ({e}) — pass phase/<roadmap>/<stem-or-number>, task/<slug>, or plan/<slug>"
        )
    })
}

/// Loads the reviews targeting `slug`, in id order.
fn reviews_on_plan(
    store: &AppStore,
    project: &str,
    slug: &str,
) -> Result<
    Vec<(
        String,
        rdm_core::document::Document<rdm_core::model::Review>,
    )>,
> {
    let all = rdm_core::ops::reviews::list_reviews(store, project)
        .context("failed to list reviews for the plan")?;
    Ok(rdm_core::ops::reviews::filter_reviews(
        all,
        &ReviewFilter {
            target: Some(ReviewTarget::Plan {
                slug: slug.to_string(),
            }),
            ..Default::default()
        },
    ))
}

pub fn run(
    command: PlanCommand,
    store: &mut AppStore,
    repo_config: &Config,
    format: OutputFormat,
    no_index: bool,
) -> Result<()> {
    match command {
        PlanCommand::Create {
            slug,
            title,
            implements,
            supersedes,
            project,
            body,
            no_edit,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            let title = title.clone().unwrap_or_else(|| slug.clone());
            let implements = parse_item_ref(&implements, "--implements")?;
            let supersedes = match supersedes.as_deref() {
                Some(raw) => Some(parse_item_ref(raw, "--supersedes")?),
                None => None,
            };
            // `--body` is authoritative; otherwise stdin is read to EOF, the
            // same contract `task create` carries.
            let body = resolve_body(body, no_edit)?;
            let doc = commit_mutation(store, &project, no_index, "failed to create plan", |s| {
                rdm_core::ops::plan::create_plan(
                    s,
                    rdm_core::ops::plan::CreatePlan {
                        project: &project,
                        slug: &slug,
                        title: &title,
                        implements: implements.clone(),
                        supersedes: supersedes.clone(),
                        body: body.as_deref(),
                    },
                )
            })?;
            match format {
                OutputFormat::Json => {
                    let j = json::plan_to_json(&slug, &doc, &[]);
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&j).context("failed to serialize plan")?
                    );
                }
                _ => println!("Created plan '{slug}' in project '{project}'"),
            }
        }
        PlanCommand::Show {
            slug,
            project,
            no_body,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            let mut doc =
                rdm_core::io::load_plan(store, &project, &slug).context("failed to load plan")?;
            if no_body {
                doc.body = String::new();
            }
            let reviews = reviews_on_plan(store, &project, &slug)?;
            match format {
                OutputFormat::Human => {
                    print!("{}", display::format_plan_detail(&slug, &doc, &reviews))
                }
                OutputFormat::Markdown => {
                    print!("{}", display::format_plan_detail_md(&slug, &doc, &reviews))
                }
                OutputFormat::Json => {
                    let j = json::plan_to_json(&slug, &doc, &reviews);
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&j).context("failed to serialize plan")?
                    );
                }
                OutputFormat::Table => bail!(
                    "--format table is not supported for 'plan show'; use --format human, --format json, --format markdown, or omit --format"
                ),
            }
            maybe_print_uncommitted_hint(store);
        }
        PlanCommand::List {
            project,
            implements,
            status,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            let implements = match implements.as_deref() {
                Some(raw) => Some(parse_item_ref(raw, "--implements")?),
                None => None,
            };
            let all =
                rdm_core::ops::plan::list_plans(store, &project).context("failed to list plans")?;
            let filtered = rdm_core::ops::plan::filter_plans(
                store,
                &project,
                all,
                &rdm_core::ops::plan::PlanFilter { implements, status },
            )
            .context("failed to filter plans")?;

            match format {
                OutputFormat::Human => print!("{}", display::format_plan_list(&filtered)),
                OutputFormat::Table => print!("{}", table::format_plan_table(&filtered)),
                OutputFormat::Markdown => print!("{}", display::format_plan_list_md(&filtered)),
                OutputFormat::Json => {
                    let summaries: Vec<_> = filtered
                        .iter()
                        .map(|(slug, doc)| json::plan_summary_to_json(slug, doc))
                        .collect();
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&summaries)
                            .context("failed to serialize plans")?
                    );
                }
            }
            maybe_print_uncommitted_hint(store);
        }
        PlanCommand::Update {
            slug,
            project,
            title,
            body,
            clear_body,
            no_edit: _,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            let title = TitleUpdate::from_args(title);
            // Like `task update`, this never reads stdin and never opens the
            // editor: `--body` sets, `--clear-body` clears, otherwise the body
            // is left untouched. A title-only update can't hang on an open pipe.
            let body = BodyUpdate::from_args(body, clear_body)?;
            let doc = commit_mutation(store, &project, no_index, "failed to update plan", |s| {
                rdm_core::ops::plan::update_plan(s, &project, &slug, title, body)
            })
            .map_err(map_body_clobber)?;
            println!(
                "Updated plan '{slug}' → status: {} (derived from reviews)",
                doc.frontmatter.status
            );
        }
        PlanCommand::Delete {
            slug,
            project,
            force,
        } => {
            if !force {
                bail!(
                    "deleting a plan is irreversible — pass --force to confirm deletion of '{slug}'"
                );
            }
            let project = paths::resolve_project(project, repo_config)?;
            commit_mutation(store, &project, no_index, "failed to delete plan", |s| {
                rdm_core::ops::plan::delete_plan(s, &project, &slug)
            })?;
            println!("Deleted plan '{slug}' from project '{project}'");
        }
    }
    Ok(())
}
