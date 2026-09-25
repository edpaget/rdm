use std::path::PathBuf;

use anyhow::{Context, Result, bail};

use rdm_core::config::{
    Config, ConfigSource, GlobalConfig, KNOWN_KEYS, PROJECT_SCOPABLE_KEYS, ResolvedValue,
    resolve_scoped_value,
};

use crate::ConfigCommand;
use crate::commands::make_store;
use crate::paths;

pub fn run(
    command: ConfigCommand,
    cli_root: &Option<PathBuf>,
    global_config: &rdm_core::config::GlobalConfig,
) -> Result<()> {
    match command {
        ConfigCommand::Get { key, raw, project } => {
            paths::validate_key(&key)?;

            let resolved = if is_scopable(&key) {
                let repo_config = load_repo_config_or_default(cli_root, global_config);
                resolve_scoped(&key, project.as_deref(), &repo_config, global_config)?
            } else if project.is_some() {
                return Err(not_scopable(&key));
            } else if key == "max_refutations" {
                // Resolved through the core grammar the review engine shares:
                // a blank env value falls through to config and a malformed
                // one is an error naming the variable, never an echo.
                let repo_config = load_repo_config_or_default(cli_root, global_config);
                paths::resolve_max_refutations_value(&repo_config, global_config)?
                    .map(stringify_resolved)
            } else if let Ok(v) = std::env::var(env_key(&key)) {
                Some(ResolvedValue {
                    value: v,
                    source: ConfigSource::Env,
                })
            } else {
                let repo_config = load_repo_config_or_default(cli_root, global_config);
                paths::resolve_config_value(&key, &repo_config, global_config)
            };

            match resolved {
                // `--raw` prints the value ALONE: no source annotation, and
                // nothing at all when the key is unset. A consumer that pipes
                // the output into another command (the dispatch verify gate
                // resolves `dispatch.verify` this way) gets a value it can run
                // verbatim instead of one it has to parse.
                Some(resolved) if raw => println!("{}", resolved.value),
                Some(resolved) => println!("{}  (source: {})", resolved.value, resolved.source),
                None if raw => {}
                None => println!("(not set)"),
            }
        }
        ConfigCommand::Set {
            key,
            value,
            global,
            project,
        } => {
            paths::validate_key(&key)?;

            if let Some(project) = project {
                if !is_scopable(&key) {
                    return Err(not_scopable(&key));
                }
                let root = paths::resolve_root(cli_root.clone(), global_config)?;
                let root = paths::expand_root(root)?;
                let mut config = paths::load_repo_config(&root);
                paths::set_project_config_field(
                    config.projects.entry(project.clone()).or_default(),
                    &key,
                    &value,
                )?;
                let mut store = make_store(&root)?;
                rdm_core::io::save_config(&mut store, &config)
                    .context("failed to write repo config")?;
                println!("Set {key} = {value} in project config for '{project}'");
            } else if global {
                if paths::is_repo_only(&key) {
                    bail!("'{key}' can only be set in repo config — omit --global");
                }
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
                let mut store = make_store(&root)?;
                rdm_core::io::save_config(&mut store, &config)
                    .context("failed to write repo config")?;
                println!("Set {key} = {value} in repo config");
            }
        }
        ConfigCommand::List { project } => {
            let repo_config = load_repo_config_or_default(cli_root, global_config);

            // `--project` narrows the listing to the keys it can resolve.
            let keys = if project.is_some() {
                PROJECT_SCOPABLE_KEYS
            } else {
                KNOWN_KEYS
            };
            let max_key_len = keys.iter().map(|k| k.len()).max().unwrap_or(0);

            for key in keys {
                let resolved = if is_scopable(key) {
                    // One key's bad environment override (e.g. an invalid
                    // `RDM_REVIEWED_GATE`) is reported on that key's row
                    // rather than hiding every other key's value.
                    match resolve_scoped(key, project.as_deref(), &repo_config, global_config) {
                        Ok(resolved) => resolved,
                        Err(e) => {
                            println!("{key:<max_key_len$}  (error: {e})");
                            continue;
                        }
                    }
                } else if *key == "max_refutations" {
                    // One malformed override must not abort the whole list,
                    // so the error is reported in this key's row instead.
                    match paths::resolve_max_refutations_value(&repo_config, global_config) {
                        Ok(resolved) => resolved.map(stringify_resolved),
                        Err(e) => {
                            println!("{key:<max_key_len$}  (invalid: {e})");
                            continue;
                        }
                    }
                } else if let Ok(v) = std::env::var(env_key(key)) {
                    Some(ResolvedValue {
                        value: v,
                        source: ConfigSource::Env,
                    })
                } else {
                    paths::resolve_config_value(key, &repo_config, global_config)
                };

                match resolved {
                    Some(resolved) => println!(
                        "{key:<max_key_len$}  {}  (source: {})",
                        resolved.value, resolved.source
                    ),
                    None => println!("{key:<max_key_len$}  (not set)"),
                }
            }
        }
    }
    Ok(())
}

fn is_scopable(key: &str) -> bool {
    PROJECT_SCOPABLE_KEYS.contains(&key)
}

fn not_scopable(key: &str) -> anyhow::Error {
    rdm_core::error::Error::KeyNotProjectScopable {
        key: key.to_string(),
    }
    .into()
}

/// Renders a typed resolved value in the string form `config get`/`list` print.
fn stringify_resolved<T: ToString>(resolved: ResolvedValue<T>) -> ResolvedValue<String> {
    ResolvedValue {
        value: resolved.value.to_string(),
        source: resolved.source,
    }
}

/// The generic `RDM_<KEY>` environment override `config get`/`list` honor.
fn env_key(key: &str) -> String {
    format!("RDM_{}", key.to_uppercase().replace('.', "_"))
}

/// Resolves a project-scopable key through the shared core rule, reading the
/// process environment.
fn resolve_scoped(
    key: &str,
    project: Option<&str>,
    repo: &Config,
    global: &GlobalConfig,
) -> Result<Option<ResolvedValue<String>>> {
    Ok(resolve_scoped_value(key, project, repo, global, |k| {
        std::env::var(k).ok()
    })?)
}

/// Loads the repo config, falling back to the default when no root resolves.
fn load_repo_config_or_default(cli_root: &Option<PathBuf>, global: &GlobalConfig) -> Config {
    match paths::resolve_root(cli_root.clone(), global) {
        Ok(root) => paths::load_repo_config(&root),
        Err(_) => Config::default(),
    }
}
