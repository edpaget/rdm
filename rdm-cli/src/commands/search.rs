use std::path::Path;

use anyhow::{Context, Result};
use rdm_core::display;
use rdm_core::json;
use rdm_core::search::{self, ItemKind, SearchFilter};

use crate::commands;
use crate::table;
use crate::{ItemKindArg, OutputFormat};

/// Searches across roadmaps, phases, and tasks.
///
/// # Errors
///
/// Returns an error if the status filter is invalid, the store cannot be
/// opened, the search fails, or serialization fails.
#[allow(clippy::too_many_arguments)]
pub fn run(
    root: &Path,
    staging: bool,
    format: OutputFormat,
    query: String,
    kind: Option<ItemKindArg>,
    status: Option<String>,
    project: Option<String>,
    tags: Vec<String>,
    limit: usize,
    min_score_ratio: f64,
) -> Result<()> {
    let store = commands::make_store(root, staging)?;
    let item_status = status
        .as_deref()
        .map(|s| commands::parse_status(s, kind))
        .transpose()?;
    let filter = SearchFilter {
        kind: kind.map(ItemKind::from),
        project,
        status: item_status,
        tags: Some(tags),
        min_score_ratio: Some(min_score_ratio),
    };
    let results = search::search(&store, &query, &filter).context("search failed")?;
    let results: Vec<_> = results.into_iter().take(limit).collect();

    match format {
        OutputFormat::Human => {
            if results.is_empty() {
                println!("No results found for '{query}'.");
            } else {
                print!("{}", display::format_search_results(&results));
            }
        }
        OutputFormat::Table => {
            if results.is_empty() {
                println!("No results found for '{query}'.");
            } else {
                print!("{}", table::format_search_table(&results));
            }
        }
        OutputFormat::Markdown => {
            if results.is_empty() {
                println!("No results found for '{query}'.");
            } else {
                print!("{}", display::format_search_results_md(&results));
            }
        }
        OutputFormat::Json => {
            let json_results: Vec<_> = results.iter().map(json::search_result_to_json).collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&json_results)
                    .context("failed to serialize results")?
            );
        }
    }
    commands::maybe_print_uncommitted_hint(&store, staging);
    Ok(())
}
