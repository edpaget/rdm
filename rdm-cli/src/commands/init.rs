use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::commands;
use crate::paths;

/// Initializes a new plan repo (or clones a remote one with `--remote`).
///
/// # Errors
///
/// Returns an error if `--default-format` is invalid, the repo cannot be
/// created/cloned/initialized, or required config writes fail.
pub fn run(
    root: &Path,
    global_config: &rdm_core::config::GlobalConfig,
    stage: bool,
    default_project: Option<String>,
    default_format: Option<String>,
    #[cfg(feature = "git")] remote: Option<String>,
) -> Result<()> {
    // Validate --default-format before any side effects.
    if let Some(ref fmt) = default_format {
        use rdm_core::config::VALID_FORMATS;
        if !VALID_FORMATS.contains(&fmt.as_str()) {
            bail!(
                "invalid default_format '{}' — valid values: {}",
                fmt,
                VALID_FORMATS.join(", ")
            );
        }
    }

    #[cfg(feature = "git")]
    if let Some(ref url) = remote {
        // Clone path: fetch remote repo into root.
        let store = rdm_store_git::GitStore::clone_remote(url, root, None)
            .context("failed to clone remote plan repo")?;

        // Validate and load config: must be a valid rdm plan repo (has rdm.toml).
        let mut config = rdm_core::io::load_config(&store)
            .context("not a valid rdm plan repo — cloned repository has no rdm.toml")?;
        config.remote = Some(rdm_core::config::RemoteConfig {
            default: Some("origin".to_string()),
        });
        if stage {
            config.stage = Some(true);
        }
        paths::save_repo_config(root, &config).context("failed to update repo config")?;

        // Commit the config update.
        let store = rdm_store_git::GitStore::new(root).context("failed to open cloned repo")?;
        store
            .git()
            .git_commit("rdm: configure remote.default = origin")
            .context("failed to commit remote config")?;

        // Save global config (best-effort, required if --default-format).
        let mut global = global_config.clone();
        if let Some(ref fmt) = default_format {
            global.default_format = Some(fmt.clone());
        }
        let global_saved = match paths::save_global_config(&global) {
            Ok(()) => true,
            Err(e) if default_format.is_some() => {
                return Err(e).context("failed to save global config");
            }
            Err(_) => false,
        };

        // Print banner and summary.
        let b = "\x1b[38;2;74;144;217m";
        let r = "\x1b[0m";
        println!(
            "\n\
             {b}██████▄    ██████▄    ██▄    ▄██{r}\n\
             \n\
             {b}██   ██    ██   ██    ████  ████{r}\n\
             \n\
             {b}██████▀    ██    ██   ██ ████ ██{r}\n\
             \n\
             {b}██▀▀█      ██    ██   ██  ██  ██{r}\n\
             \n\
             {b}██  ▀█     ██   ██    ██      ██{r}\n\
             \n\
             {b}██   █▄    ██████▀    ██      ██{r}\n"
        );
        println!("Cloned plan repo from {url}");
        println!("  location: {}", root.display());
        println!("  repo config: {}/rdm.toml", root.display());
        if global_saved && let Some(gp) = paths::global_config_path() {
            println!("  global config: {}", gp.display());
        }
        println!("  default remote: origin");
        if let Some(ref fmt) = default_format {
            println!("  default format: {fmt}");
        }
        if stage {
            println!("  staging mode: enabled");
        }
        println!();
        println!("Next steps:");
        println!("  rdm roadmap list   # see available roadmaps");
        println!("  rdm task list      # see open tasks");
        println!("  rdm pull           # fetch latest changes");

        return Ok(());
    }

    // Create root directory recursively.
    std::fs::create_dir_all(root)
        .with_context(|| format!("failed to create {}", root.display()))?;

    // Build repo config from flags.
    let init_config = rdm_core::config::Config {
        default_project: default_project.clone(),
        stage: if stage { Some(true) } else { None },
        ..Default::default()
    };

    let mut store = commands::make_init_store(root)?;
    rdm_core::ops::init::init_with_config(&mut store, init_config)
        .context("failed to initialize plan repo")?;

    // Create project directory if --default-project was given.
    if let Some(ref proj) = default_project {
        rdm_core::ops::mutate(&mut store, proj, |s| {
            rdm_core::ops::project::create_project(s, proj, proj)
        })
        .with_context(|| format!("failed to create project '{proj}'"))?;
    }

    // Ensure global config exists; required if --default-format was given,
    // best-effort otherwise.
    let mut global = global_config.clone();
    if let Some(ref fmt) = default_format {
        global.default_format = Some(fmt.clone());
    }
    let global_saved = match paths::save_global_config(&global) {
        Ok(()) => true,
        Err(e) if default_format.is_some() => {
            return Err(e).context("failed to save global config");
        }
        Err(_) => false, // Best-effort: global config path may not be writable.
    };

    // Print banner and summary.
    let b = "\x1b[38;2;74;144;217m";
    let r = "\x1b[0m";
    println!(
        "\n\
         {b}██████▄    ██████▄    ██▄    ▄██{r}\n\
         \n\
         {b}██   ██    ██   ██    ████  ████{r}\n\
         \n\
         {b}██████▀    ██    ██   ██ ████ ██{r}\n\
         \n\
         {b}██▀▀█      ██    ██   ██  ██  ██{r}\n\
         \n\
         {b}██  ▀█     ██   ██    ██      ██{r}\n\
         \n\
         {b}██   █▄    ██████▀    ██      ██{r}\n"
    );
    println!("Initialized plan repo at {}", root.display());
    println!("  repo config: {}/rdm.toml", root.display());
    if global_saved && let Some(gp) = paths::global_config_path() {
        println!("  global config: {}", gp.display());
    }
    if let Some(ref proj) = default_project {
        println!("  default project: {proj}");
    }
    if let Some(ref fmt) = default_format {
        println!("  default format: {fmt}");
    }
    if stage {
        println!("  staging mode: enabled");
    }
    println!();
    println!("Next steps:");
    if default_project.is_none() {
        println!("  rdm project create <name>  # create a project");
    }
    println!("  rdm roadmap create <slug>  # create a roadmap");
    println!("  rdm task create <slug>     # create a task");
    Ok(())
}
