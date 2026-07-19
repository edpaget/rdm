use anyhow::{Context, Result, bail};
use rdm_core::config::Config;
use rdm_core::ops::tag::{self, TagCount};

use crate::paths;
use crate::{AppStore, OutputFormat, TagCommand};

/// Runs a `rdm tag` subcommand.
///
/// # Errors
///
/// Returns an error if the project cannot be resolved, the tag inventory
/// fails to build, serialization fails, or `--format table` is requested (not
/// supported for this command).
pub fn run(
    command: TagCommand,
    store: &mut AppStore,
    repo_config: &Config,
    format: OutputFormat,
) -> Result<()> {
    match command {
        TagCommand::List { project } => {
            let project = paths::resolve_project(project, repo_config)?;
            let tags = tag::tag_list(store, &project).context("failed to list tags")?;

            match format {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&tags)
                            .context("failed to serialize tag list")?
                    );
                }
                OutputFormat::Human | OutputFormat::Markdown => {
                    print!("{}", render_human(&tags));
                }
                OutputFormat::Table => bail!(
                    "--format table is not supported for 'tag list'; use --format human, --format json, --format markdown, or omit --format"
                ),
            }
        }
    }
    Ok(())
}

/// Renders the human/markdown text for a tag inventory: one `tag (count)`
/// line per tag, most-used first.
fn render_human(tags: &[TagCount]) -> String {
    if tags.is_empty() {
        return "No tags in use.\n".to_string();
    }
    let mut out = String::new();
    for t in tags {
        out.push_str(&format!("{} ({})\n", t.tag, t.count));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tc(tag: &str, count: usize) -> TagCount {
        TagCount {
            tag: tag.to_string(),
            count,
            roadmaps: 0,
            tasks: count,
        }
    }

    #[test]
    fn renders_tag_with_count_per_line() {
        let out = render_human(&[tc("cli", 3), tc("web-ui", 5)]);
        assert_eq!(out, "cli (3)\nweb-ui (5)\n");
    }

    #[test]
    fn renders_empty_notice_when_no_tags() {
        assert_eq!(render_human(&[]), "No tags in use.\n");
    }
}
