//! `rdm info` — reports what rdm actually resolved for the current
//! environment: `{root, project, default_branch, default_format}`.
//!
//! This is the single-call discovery contract an editor or plugin
//! integration needs to map its working directory to a plan repo location,
//! without scraping `rdm config list` (which checks the wrong env var for
//! `default_project` — see below) or reading `rdm.toml`/`config.toml`
//! directly.

use std::path::Path;

use anyhow::{Context, Result, bail};
use clap::parser::ValueSource;
use rdm_core::config::{Config, ConfigSource, GlobalConfig};
use rdm_core::json::InfoJson;

use crate::OutputFormat;
use crate::paths;

/// One resolved field, paired with the source it came from (`None` only for
/// an unresolved `project`, which has no source to report).
struct InfoField {
    key: &'static str,
    value: Option<String>,
    source: Option<ConfigSource>,
}

/// Resolves and prints `{root, project, default_branch, default_format}`.
///
/// `root` and `root_source` are already resolved by the caller (`main.rs`)
/// — `root_source` distinguishes the `--root` flag from the `RDM_ROOT` env
/// var, which clap merges into a single value before this function ever
/// sees it, via `ArgMatches::value_source("root")`. `global_config` and
/// `repo_config` are also already loaded and merged (`repo_config` has
/// global defaults folded in via `with_global_defaults`) — this function
/// re-reads the *raw*, unmerged repo config internally to tell a
/// repo-config-sourced value apart from a global-config-sourced one, which
/// the merge collapses.
///
/// `format` is the fully-resolved output format governing *this command's*
/// own rendering (flag → `RDM_FORMAT` env → config → human default).
/// `cli_format_flag` is the raw, un-resolved `--format` flag alone (`None`
/// unless the flag was passed literally on the command line) — it is used
/// only to compute the *reported* `default_format` field's source, since
/// the same global `--format` flag does double duty as the value a plugin
/// would see for "what default_format would other commands use".
///
/// Project resolution deliberately goes through [`paths::resolve_project`]
/// (which reads `RDM_PROJECT`), never the `config list`/`config get` code
/// path (which checks `RDM_DEFAULT_PROJECT` for the `default_project` key)
/// — those are two different env vars, and `info` must report what the CLI
/// would really use.
///
/// # Errors
///
/// Returns an error if `--format table` is requested (not supported for
/// this command). Never errors on an unresolved project — it is reported
/// with the `project` field absent (JSON) or `(not set)` (human/markdown)
/// instead.
pub fn run(
    root: &Path,
    root_source: Option<ValueSource>,
    global_config: &GlobalConfig,
    repo_config: &Config,
    format: OutputFormat,
    cli_format_flag: Option<OutputFormat>,
    project_flag: Option<String>,
) -> Result<()> {
    // Re-load the raw (unmerged) repo config, purely to disambiguate a
    // repo-sourced value from a global-sourced one for display — resolution
    // itself always uses the already-merged `repo_config`.
    let raw_repo_config = paths::load_repo_config(root);

    let root_str = root.display().to_string();
    let root_source_label = match root_source {
        Some(ValueSource::CommandLine) => ConfigSource::Flag,
        Some(ValueSource::EnvVariable) => ConfigSource::Env,
        _ if global_config.root.is_some() => ConfigSource::Global,
        _ => ConfigSource::Default,
    };

    let project = paths::resolve_project(project_flag.clone(), repo_config).ok();
    let project_source = if project_flag.is_some() {
        Some(ConfigSource::Flag)
    } else if std::env::var("RDM_PROJECT").is_ok() {
        Some(ConfigSource::Env)
    } else if raw_repo_config.default_project.is_some() {
        Some(ConfigSource::Repo)
    } else if global_config.default_project.is_some() {
        Some(ConfigSource::Global)
    } else {
        None
    };

    let default_branch = repo_config.default_branch.as_deref().unwrap_or("main");
    let default_branch_source = if raw_repo_config.default_branch.is_some() {
        ConfigSource::Repo
    } else if global_config.default_branch.is_some() {
        ConfigSource::Global
    } else {
        ConfigSource::Default
    };

    // The global `--format` flag does double duty here: it selects info's
    // own rendering AND supplies the "flag" source/value for the reported
    // `default_format` field. There is no separate flag to represent "what
    // default_format would other commands use" — using the same one is
    // intentional, not an oversight.
    let cli_format = cli_format_flag.map(|f| f.to_string());
    let default_format = paths::resolve_format(cli_format.clone(), repo_config);
    let default_format_source = if cli_format.is_some() {
        ConfigSource::Flag
    } else if std::env::var("RDM_FORMAT").is_ok() {
        ConfigSource::Env
    } else if raw_repo_config.default_format.is_some() {
        ConfigSource::Repo
    } else if global_config.default_format.is_some() {
        ConfigSource::Global
    } else {
        ConfigSource::Default
    };

    let fields = vec![
        InfoField {
            key: "root",
            value: Some(root_str.clone()),
            source: Some(root_source_label),
        },
        InfoField {
            key: "project",
            value: project.clone(),
            source: project_source,
        },
        InfoField {
            key: "default_branch",
            value: Some(default_branch.to_string()),
            source: Some(default_branch_source),
        },
        InfoField {
            key: "default_format",
            value: Some(default_format.clone()),
            source: Some(default_format_source),
        },
    ];

    match format {
        OutputFormat::Json => {
            let info = InfoJson {
                root: root_str,
                project,
                default_branch: default_branch.to_string(),
                default_format,
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&info).context("failed to serialize info result")?
            );
        }
        OutputFormat::Human => {
            print!("{}", render_human(&fields));
        }
        OutputFormat::Markdown => {
            print!("{}", render_markdown(&fields));
        }
        OutputFormat::Table => bail!(
            "--format table is not supported for 'info'; use --format human, --format json, --format markdown, or omit --format"
        ),
    }
    Ok(())
}

fn render_human(fields: &[InfoField]) -> String {
    let mut out = String::new();
    for field in fields {
        match (&field.value, &field.source) {
            (Some(value), Some(source)) => {
                out.push_str(&format!("{}: {value}  (source: {source})\n", field.key));
            }
            _ => {
                out.push_str(&format!("{}: (not set)\n", field.key));
            }
        }
    }
    out
}

fn render_markdown(fields: &[InfoField]) -> String {
    let mut out = String::from("| Key | Value | Source |\n| --- | --- | --- |\n");
    for field in fields {
        match (&field.value, &field.source) {
            (Some(value), Some(source)) => {
                out.push_str(&format!("| {} | {value} | {source} |\n", field.key));
            }
            _ => {
                out.push_str(&format!("| {} | (not set) | |\n", field.key));
            }
        }
    }
    out
}
