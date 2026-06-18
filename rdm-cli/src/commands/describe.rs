use anyhow::{Result, bail};

use crate::OutputFormat;

/// Describes the rdm data model (entities and their fields).
///
/// # Errors
///
/// Returns an error if serialization fails or an unknown entity name is given.
pub fn run(format: OutputFormat, entity: Option<String>) -> Result<()> {
    let entities = rdm_core::describe::all_entities();
    match entity {
        None => {
            let output = match format {
                OutputFormat::Json => serde_json::to_string_pretty(&entities)?,
                OutputFormat::Markdown => rdm_core::describe::format_entity_list_md(&entities),
                _ => rdm_core::describe::format_entity_list(&entities),
            };
            print!("{output}");
        }
        Some(name) => {
            let entity = entities.iter().find(|e| e.name == name);
            match entity {
                Some(e) => {
                    let output = match format {
                        OutputFormat::Json => serde_json::to_string_pretty(e)?,
                        OutputFormat::Markdown => rdm_core::describe::format_entity_detail_md(e),
                        _ => rdm_core::describe::format_entity_detail(e),
                    };
                    print!("{output}");
                }
                None => {
                    let valid: Vec<&str> = entities.iter().map(|e| e.name).collect();
                    bail!(
                        "unknown entity '{}'. Valid entities: {}",
                        name,
                        valid.join(", ")
                    );
                }
            }
        }
    }
    Ok(())
}
