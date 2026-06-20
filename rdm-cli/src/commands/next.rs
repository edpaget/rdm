use anyhow::{Context, Result, bail};
use rdm_core::config::Config;
use rdm_core::ops::next::{NextActionable, next_actionable};

use crate::paths;
use crate::{AppStore, OutputFormat};

/// Resolves and prints the next actionable phase in a roadmap.
///
/// This is read-only: it never mutates or commits. "Nothing actionable" and
/// "blocked on dependencies" are valid answers, distinct from an error, so all
/// three results exit 0.
///
/// # Errors
///
/// Returns an error if the project cannot be resolved, the roadmap does not
/// exist, the selector fails to read the plan repo, or `--format table` is
/// requested (not supported for this command).
pub fn run(
    store: &mut AppStore,
    repo_config: &Config,
    format: OutputFormat,
    roadmap: String,
    project: Option<String>,
) -> Result<()> {
    let project = paths::resolve_project(project, repo_config)?;
    let result = next_actionable(store, &project, &roadmap)
        .context("failed to resolve next actionable phase")?;

    match format {
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(&result)
                    .context("failed to serialize next actionable result")?
            );
        }
        OutputFormat::Human | OutputFormat::Markdown => {
            print!("{}", render_text(&roadmap, &result));
        }
        OutputFormat::Table => bail!(
            "--format table is not supported for 'next'; use --format human, --format json, --format markdown, or omit --format"
        ),
    }
    Ok(())
}

/// Renders the human/markdown text for a [`NextActionable`] result.
fn render_text(roadmap: &str, result: &NextActionable) -> String {
    match result {
        NextActionable::Phase(p) => {
            let mut out = format!("{} {} ({})\n", p.number, p.stem, p.status);
            if let Some(difficulty) = p.difficulty {
                out.push_str(&format!("  difficulty: {difficulty}\n"));
            }
            if let Some(model) = p.model {
                out.push_str(&format!("  model: {model}\n"));
            }
            out
        }
        NextActionable::BlockedOnDependencies { unmet } => {
            format!("Blocked on dependencies: {}\n", unmet.join(", "))
        }
        NextActionable::Nothing => format!("Nothing actionable in {roadmap}.\n"),
    }
}
