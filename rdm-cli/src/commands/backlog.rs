use anyhow::{Context, Result, bail};
use rdm_core::config::Config;
use rdm_core::ops::backlog::{self, BacklogReport, ReportOptions};

use crate::paths;
use crate::{AppStore, BacklogCommand, OutputFormat};

/// Runs a `rdm backlog` subcommand.
///
/// # Errors
///
/// Returns an error if the project cannot be resolved, the report fails to
/// build, serialization fails, or `--format table` is requested (not
/// supported for this command).
pub fn run(
    command: BacklogCommand,
    store: &mut AppStore,
    repo_config: &Config,
    format: OutputFormat,
) -> Result<()> {
    match command {
        BacklogCommand::Report {
            older_than,
            tag,
            project,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            let opts = ReportOptions {
                older_than_days: i64::from(older_than),
                tag,
            };
            let report = backlog::report(store, &project, &opts)
                .context("failed to build backlog report")?;

            match format {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&report)
                            .context("failed to serialize backlog report")?
                    );
                }
                OutputFormat::Human | OutputFormat::Markdown => {
                    print!("{}", render_human(&report));
                }
                OutputFormat::Table => bail!(
                    "--format table is not supported for 'backlog report'; use --format human, --format json, --format markdown, or omit --format"
                ),
            }
        }
    }
    Ok(())
}

/// Renders the human/markdown text for a [`BacklogReport`].
fn render_human(report: &BacklogReport) -> String {
    let mut out = String::new();

    out.push_str("## Stale tasks\n\n");
    if report.stale_tasks.is_empty() {
        out.push_str("No stale tasks.\n\n");
    } else {
        for t in &report.stale_tasks {
            out.push_str(&format!(
                "- {} — {} ({}, {} days old, created {})\n",
                t.slug, t.title, t.status, t.age_days, t.created
            ));
        }
        out.push('\n');
    }

    out.push_str("## Duplicate clusters\n\n");
    if report.duplicate_clusters.is_empty() {
        out.push_str("No duplicate clusters.\n\n");
    } else {
        for cluster in &report.duplicate_clusters {
            let members: Vec<String> = cluster
                .members
                .iter()
                .map(|m| format!("{} ({})", m.slug, m.title))
                .collect();
            out.push_str(&format!("- {}\n", members.join(", ")));
        }
        out.push('\n');
    }

    out.push_str("## Tag clusters\n\n");
    if report.tag_clusters.is_empty() {
        out.push_str("No tag clusters.\n\n");
    } else {
        for cluster in &report.tag_clusters {
            let members: Vec<String> = cluster
                .tasks
                .iter()
                .map(|m| format!("{} ({})", m.slug, m.title))
                .collect();
            out.push_str(&format!("- {}: {}\n", cluster.tag, members.join(", ")));
        }
        out.push('\n');
    }

    out.push_str("## Archivable roadmaps\n\n");
    if report.archivable_roadmaps.is_empty() {
        out.push_str("No archivable roadmaps.\n");
    } else {
        for r in &report.archivable_roadmaps {
            out.push_str(&format!(
                "- {} — {} ({} phases)\n",
                r.roadmap, r.title, r.phase_count
            ));
        }
    }

    out
}
