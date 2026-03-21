use std::path::PathBuf;

use anyhow::{Result, bail};

use crate::ConfigCommand;
use crate::paths;

pub fn run(
    command: ConfigCommand,
    cli_root: &Option<PathBuf>,
    global_config: &rdm_core::config::GlobalConfig,
) -> Result<()> {
    match command {
        ConfigCommand::Get { key } => {
            paths::validate_key(&key)?;

            // Check env var first
            let env_key = format!("RDM_{}", key.to_uppercase().replace('.', "_"));
            if let Ok(v) = std::env::var(&env_key) {
                println!("{v}  (source: environment variable)");
                return Ok(());
            }

            // Try repo + global config
            let root = paths::resolve_root(cli_root.clone(), global_config);
            let repo_config = if let Ok(ref root) = root {
                paths::load_repo_config(root)
            } else {
                rdm_core::config::Config::default()
            };

            if let Some(resolved) = paths::resolve_config_value(&key, &repo_config, global_config) {
                println!("{}  (source: {})", resolved.value, resolved.source);
            } else {
                println!("(not set)");
            }
        }
        ConfigCommand::Set { key, value, global } => {
            paths::validate_key(&key)?;

            if global {
                let mut config = global_config.clone();
                paths::set_global_config_field(&mut config, &key, &value)?;
                paths::save_global_config(&config)?;
                println!("Set {key} = {value} in global config");
            } else {
                if paths::is_global_only(&key) {
                    bail!("'{key}' can only be set in global config — use --global");
                }
                let root = paths::resolve_root(cli_root.clone(), global_config)?;
                let root = paths::expand_root(root)?;
                let mut config = paths::load_repo_config(&root);
                paths::set_config_field(&mut config, &key, &value)?;
                paths::save_repo_config(&root, &config)?;
                println!("Set {key} = {value} in repo config");
            }
        }
        ConfigCommand::List => {
            let root = paths::resolve_root(cli_root.clone(), global_config);
            let repo_config = if let Ok(ref root) = root {
                paths::load_repo_config(root)
            } else {
                rdm_core::config::Config::default()
            };

            let max_key_len = rdm_core::config::KNOWN_KEYS
                .iter()
                .map(|k| k.len())
                .max()
                .unwrap_or(0);

            for key in rdm_core::config::KNOWN_KEYS {
                // Check env var
                let env_key = format!("RDM_{}", key.to_uppercase().replace('.', "_"));
                if let Ok(v) = std::env::var(&env_key) {
                    println!("{key:<max_key_len$}  {v}  (source: environment variable)");
                    continue;
                }

                if let Some(resolved) =
                    paths::resolve_config_value(key, &repo_config, global_config)
                {
                    println!(
                        "{key:<max_key_len$}  {}  (source: {})",
                        resolved.value, resolved.source
                    );
                } else {
                    println!("{key:<max_key_len$}  (not set)");
                }
            }
        }
    }
    Ok(())
}
