use std::path::PathBuf;
use std::process;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use rdm_core::agent_config::{self, AgentConfigOptions, McpConfigOptions, Platform, SkillOptions};
use rdm_core::display;
use rdm_core::json;
use rdm_core::model::{PhaseStatus, Priority, RoadmapSort, TaskStatus, TaskStatusFilter};
use rdm_core::search::{self, ItemKind, SearchFilter};
use rdm_core::tree;
#[cfg(not(feature = "git"))]
use rdm_store_fs::FsStore;

mod commands;
mod paths;
mod table;

#[cfg(feature = "git")]
pub(crate) type AppStore = rdm_store_git::GitStore;
#[cfg(not(feature = "git"))]
pub(crate) type AppStore = FsStore;

#[derive(Parser)]
#[command(name = "rdm", about = "Manage project roadmaps, phases, and tasks")]
struct Cli {
    /// Path to the plan repo root.
    #[arg(long, env = "RDM_ROOT")]
    root: Option<PathBuf>,

    /// Suppress automatic INDEX.md regeneration after mutations.
    #[arg(long, global = true)]
    no_index: bool,

    /// Defer git commits until an explicit `rdm commit`.
    #[arg(long, global = true, env = "RDM_STAGE")]
    stage: bool,

    /// Output format (human, json, table, or markdown).
    #[arg(long, global = true)]
    format: Option<OutputFormat>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Initialize a new plan repo.
    Init {
        /// Set the default project in repo config and create its directory.
        #[arg(long)]
        default_project: Option<String>,
        /// Set the default output format in global config.
        #[arg(long)]
        default_format: Option<String>,
        /// Clone a remote plan repo instead of creating an empty one.
        #[cfg(feature = "git")]
        #[arg(long, conflicts_with = "default_project")]
        remote: Option<String>,
    },
    /// Clone or fast-forward a plan repo into a target directory.
    ///
    /// Designed for session-start hooks and sandbox bootstrap scripts: safe to
    /// re-run on every invocation. Clones on first run, fast-forwards on
    /// subsequent runs.
    #[cfg(feature = "git")]
    Bootstrap {
        /// Git URL of the plan repo to clone. Required unless a subcommand is
        /// given (e.g. `rdm bootstrap doctor`).
        #[arg(long)]
        plan_repo: Option<String>,
        /// Target directory for the clone.
        ///
        /// Defaults to `$XDG_DATA_HOME/rdm/plan-repo` (or `~/.local/share/rdm/plan-repo`).
        #[arg(long)]
        path: Option<PathBuf>,
        /// Branch to check out at clone time.
        #[arg(long)]
        branch: Option<String>,
        /// If the cloned repo has no `rdm.toml`, run `rdm init` on it.
        #[arg(long)]
        init: bool,
        /// Access token injected into an HTTPS clone URL. Read from
        /// `RDM_PLAN_REPO_TOKEN` if not passed explicitly.
        #[arg(long, env = "RDM_PLAN_REPO_TOKEN", hide_env_values = true)]
        token: Option<String>,
        #[command(subcommand)]
        command: Option<BootstrapSubcommand>,
    },
    /// View or modify configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Manage projects.
    Project {
        #[command(subcommand)]
        command: ProjectCommand,
    },
    /// Manage roadmaps.
    Roadmap {
        #[command(subcommand)]
        command: RoadmapCommand,
    },
    /// Manage phases.
    Phase {
        #[command(subcommand)]
        command: PhaseCommand,
    },
    /// Manage tasks.
    Task {
        #[command(subcommand)]
        command: TaskCommand,
    },
    /// Promote a task to a roadmap.
    Promote {
        /// Task slug to promote.
        task_slug: String,
        /// Roadmap slug for the new roadmap.
        #[arg(long)]
        roadmap_slug: String,
        /// Project the task belongs to.
        #[arg(long)]
        project: Option<String>,
    },
    /// Generate INDEX.md from current repo state.
    Index,
    /// Generate agent configuration for AI coding assistants.
    AgentConfig {
        /// Target platform (claude, agents-md, cursor, copilot).
        #[arg(default_value = "agents-md")]
        platform: String,
        /// Project name to embed in generated examples.
        #[arg(long)]
        project: Option<String>,
        /// Write to platform-conventional path within this directory.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Path to a principles/conventions file to reference in generated instructions.
        #[arg(long)]
        principles_file: Option<String>,
        /// Generate Claude Code skill files instead of an instruction file.
        #[arg(long)]
        skills: bool,
        /// Generate MCP-oriented instructions (referencing MCP tool names instead of CLI commands).
        /// When combined with --out, also writes .mcp.json alongside.
        #[arg(long)]
        mcp: bool,
        /// Write to the user-level config directory (e.g. ~/.claude/) instead of a project directory.
        /// Mutually exclusive with --out.
        #[arg(long, conflicts_with = "out")]
        user: bool,
    },
    /// Describe the rdm data model (entities and their fields).
    Describe {
        /// Entity name to describe (project, roadmap, phase, task). Omit to list all.
        entity: Option<String>,
    },
    /// Show a hierarchical tree of a project's roadmaps, phases, and tasks.
    Tree {
        /// Project to show the tree for.
        #[arg(long)]
        project: Option<String>,
    },
    /// Search across roadmaps, phases, and tasks.
    Search {
        /// The search query (fuzzy matched against titles and body content).
        query: String,
        /// Filter by item type.
        #[arg(long = "type")]
        kind: Option<ItemKindArg>,
        /// Filter by status (e.g., done, in-progress, open).
        #[arg(long)]
        status: Option<String>,
        /// Filter by project.
        #[arg(long)]
        project: Option<String>,
        /// Filter by tag. Repeat to require multiple tags (AND).
        #[arg(long = "tag")]
        tags: Vec<String>,
        /// Maximum number of results to return.
        #[arg(long, default_value = "20")]
        limit: usize,
        /// Minimum score ratio (0.0–1.0). Results below this fraction of the top
        /// score are dropped. Default: 0.25. Use 0 to disable.
        #[arg(long, default_value = "0.25")]
        min_score_ratio: f64,
    },
    /// Show uncommitted changes and sync status in the plan repo.
    #[cfg(feature = "git")]
    Status {
        /// Fetch from the default remote before checking sync status.
        #[arg(long)]
        fetch: bool,
    },
    /// Commit staged changes to git.
    #[cfg(feature = "git")]
    Commit {
        /// Commit message (auto-generated if omitted).
        #[arg(short, long)]
        message: Option<String>,
    },
    /// Discard uncommitted changes, restoring the working directory to HEAD.
    #[cfg(feature = "git")]
    Discard {
        /// Confirm the destructive operation.
        #[arg(long)]
        force: bool,
    },
    /// List unresolved merge conflicts with rdm item context.
    #[cfg(feature = "git")]
    Conflicts,
    /// Mark a conflicted file as resolved and auto-complete the merge when all are resolved.
    #[cfg(feature = "git")]
    Resolve {
        /// Path of the file to mark as resolved.
        file: String,
    },
    /// Manage git remotes.
    #[cfg(feature = "git")]
    Remote {
        #[command(subcommand)]
        command: RemoteCommand,
    },
    /// Manage the post-merge and post-commit git hooks.
    #[cfg(feature = "git")]
    Hook {
        #[command(subcommand)]
        command: HookCommand,
    },
    /// Start the MCP server on stdin/stdout.
    #[cfg(feature = "mcp")]
    Mcp,
    /// Start the rdm REST API server.
    #[cfg(feature = "server")]
    Serve {
        /// Port to listen on.
        #[arg(long, default_value = "3000")]
        port: u16,
        /// Address to bind to.
        #[arg(long, default_value = "127.0.0.1")]
        bind: String,
    },
    /// List roadmaps and their progress.
    List {
        /// Project to list roadmaps for.
        #[arg(long)]
        project: Option<String>,
        /// List all projects and roadmaps.
        #[arg(long)]
        all: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum ProjectCommand {
    /// Create a new project.
    Create {
        /// Project slug (used in directory names).
        name: String,
        /// Human-readable title.
        #[arg(long)]
        title: Option<String>,
    },
    /// Show project details.
    Show {
        /// Project slug.
        name: String,
    },
    /// List all projects.
    List,
}

#[derive(Subcommand)]
pub(crate) enum RoadmapCommand {
    /// Create a new roadmap.
    Create {
        /// Roadmap slug.
        slug: String,
        /// Human-readable title.
        #[arg(long)]
        title: Option<String>,
        /// Project to create the roadmap in.
        #[arg(long)]
        project: Option<String>,
        /// Priority level.
        #[arg(long)]
        priority: Option<Priority>,
        /// Comma-separated tags.
        #[arg(long, value_delimiter = ',')]
        tags: Option<Vec<String>>,
        /// Body content for the roadmap.
        #[arg(long)]
        body: Option<String>,
        /// Suppress interactive editor for body content.
        #[arg(long)]
        no_edit: bool,
    },
    /// Show a roadmap and its phases.
    Show {
        /// Roadmap slug.
        slug: String,
        /// Project the roadmap belongs to.
        #[arg(long)]
        project: Option<String>,
        /// Suppress body content in output.
        #[arg(long)]
        no_body: bool,
    },
    /// Update a roadmap's priority and/or body.
    Update {
        /// Roadmap slug.
        slug: String,
        /// Project the roadmap belongs to.
        #[arg(long)]
        project: Option<String>,
        /// New priority level.
        #[arg(long, conflicts_with = "clear_priority")]
        priority: Option<Priority>,
        /// Remove the priority from this roadmap.
        #[arg(long, conflicts_with = "priority")]
        clear_priority: bool,
        /// New comma-separated tags (replaces existing).
        #[arg(long, value_delimiter = ',')]
        tags: Option<Vec<String>>,
        /// Body content for the roadmap.
        #[arg(long)]
        body: Option<String>,
        /// Suppress interactive editor for body content.
        #[arg(long)]
        no_edit: bool,
    },
    /// List all roadmaps in a project.
    List {
        /// Project to list roadmaps for.
        #[arg(long)]
        project: Option<String>,
        /// Show archived roadmaps instead of active ones.
        #[arg(long)]
        archived: bool,
        /// Sort order (alphabetical or priority).
        #[arg(long)]
        sort: Option<RoadmapSort>,
        /// Filter by priority level.
        #[arg(long)]
        priority: Option<Priority>,
    },
    /// Add a dependency on another roadmap.
    Depend {
        /// Roadmap slug that will depend on another.
        slug: String,
        /// The roadmap to depend on.
        #[arg(long)]
        on: String,
        /// Project the roadmaps belong to.
        #[arg(long)]
        project: Option<String>,
    },
    /// Remove a dependency on another roadmap.
    Undepend {
        /// Roadmap slug to remove a dependency from.
        slug: String,
        /// The dependency to remove.
        #[arg(long)]
        on: String,
        /// Project the roadmaps belong to.
        #[arg(long)]
        project: Option<String>,
    },
    /// Show the dependency graph for all roadmaps.
    Deps {
        /// Project to show dependencies for.
        #[arg(long)]
        project: Option<String>,
    },
    /// Delete a roadmap and all its phases.
    Delete {
        /// Roadmap slug to delete.
        slug: String,
        /// Project the roadmap belongs to.
        #[arg(long)]
        project: Option<String>,
        /// Confirm deletion (required).
        #[arg(long)]
        force: bool,
    },
    /// Split a roadmap by extracting phases into a new roadmap.
    Split {
        /// Source roadmap slug.
        slug: String,
        /// Phase stems or numbers to extract.
        #[arg(long, required = true, num_args = 1..)]
        phases: Vec<String>,
        /// Slug for the new roadmap.
        #[arg(long)]
        into: String,
        /// Title for the new roadmap.
        #[arg(long)]
        title: String,
        /// Project the roadmap belongs to.
        #[arg(long)]
        project: Option<String>,
        /// Add a dependency from the new roadmap on the source.
        #[arg(long)]
        depends_on: bool,
    },
    /// Archive a completed roadmap.
    Archive {
        /// Roadmap slug to archive.
        slug: String,
        /// Project the roadmap belongs to.
        #[arg(long)]
        project: Option<String>,
        /// Archive even if some phases are not done.
        #[arg(long)]
        force: bool,
    },
    /// Restore an archived roadmap to active status.
    Unarchive {
        /// Roadmap slug to restore.
        slug: String,
        /// Project the roadmap belongs to.
        #[arg(long)]
        project: Option<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum PhaseCommand {
    /// Create a new phase in a roadmap.
    Create {
        /// Phase slug (appended to phase-N-).
        slug: String,
        /// Human-readable title.
        #[arg(long)]
        title: Option<String>,
        /// Roadmap to add the phase to.
        #[arg(long)]
        roadmap: String,
        /// Project the roadmap belongs to.
        #[arg(long)]
        project: Option<String>,
        /// Explicit phase number (auto-assigned if omitted).
        #[arg(long)]
        number: Option<u32>,
        /// Comma-separated tags.
        #[arg(long, value_delimiter = ',')]
        tags: Option<Vec<String>>,
        /// Body content for the phase.
        #[arg(long)]
        body: Option<String>,
        /// Suppress interactive editor for body content.
        #[arg(long)]
        no_edit: bool,
    },
    /// List phases in a roadmap.
    List {
        /// Roadmap to list phases for.
        #[arg(long)]
        roadmap: String,
        /// Project the roadmap belongs to.
        #[arg(long)]
        project: Option<String>,
    },
    /// Show a phase.
    Show {
        /// Phase stem or number (e.g. phase-1-core or 1).
        stem: String,
        /// Roadmap the phase belongs to.
        #[arg(long)]
        roadmap: String,
        /// Project the roadmap belongs to.
        #[arg(long)]
        project: Option<String>,
        /// Suppress body content in output.
        #[arg(long)]
        no_body: bool,
    },
    /// Update a phase's status and/or body.
    Update {
        /// Phase stem or number (e.g. phase-1-core or 1).
        stem: String,
        /// New status (omit to preserve existing).
        #[arg(long)]
        status: Option<PhaseStatus>,
        /// Roadmap the phase belongs to.
        #[arg(long)]
        roadmap: String,
        /// Project the roadmap belongs to.
        #[arg(long)]
        project: Option<String>,
        /// New comma-separated tags (replaces existing).
        #[arg(long, value_delimiter = ',')]
        tags: Option<Vec<String>>,
        /// Body content for the phase.
        #[arg(long)]
        body: Option<String>,
        /// Git commit SHA to associate with phase completion.
        #[arg(long)]
        commit: Option<String>,
        /// Suppress interactive editor for body content.
        #[arg(long)]
        no_edit: bool,
    },
    /// Remove a phase from a roadmap.
    Remove {
        /// Phase stem or number (e.g. phase-1-core or 1).
        stem: String,
        /// Roadmap the phase belongs to.
        #[arg(long)]
        roadmap: String,
        /// Project the roadmap belongs to.
        #[arg(long)]
        project: Option<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum TaskCommand {
    /// Create a new task.
    Create {
        /// Task slug.
        slug: String,
        /// Human-readable title.
        #[arg(long)]
        title: Option<String>,
        /// Project to create the task in.
        #[arg(long)]
        project: Option<String>,
        /// Priority level.
        #[arg(long, default_value = "medium")]
        priority: Priority,
        /// Comma-separated tags.
        #[arg(long, value_delimiter = ',')]
        tags: Option<Vec<String>>,
        /// Body content for the task.
        #[arg(long)]
        body: Option<String>,
        /// Suppress interactive editor for body content.
        #[arg(long)]
        no_edit: bool,
    },
    /// Show a task.
    Show {
        /// Task slug.
        slug: String,
        /// Project the task belongs to.
        #[arg(long)]
        project: Option<String>,
        /// Suppress body content in output.
        #[arg(long)]
        no_body: bool,
    },
    /// Update a task.
    Update {
        /// Task slug.
        slug: String,
        /// Project the task belongs to.
        #[arg(long)]
        project: Option<String>,
        /// New status.
        #[arg(long)]
        status: Option<TaskStatus>,
        /// New priority.
        #[arg(long)]
        priority: Option<Priority>,
        /// New comma-separated tags (replaces existing).
        #[arg(long, value_delimiter = ',')]
        tags: Option<Vec<String>>,
        /// Body content for the task.
        #[arg(long)]
        body: Option<String>,
        /// Git commit SHA to associate with this task.
        #[arg(long)]
        commit: Option<String>,
        /// Suppress interactive editor for body content.
        #[arg(long)]
        no_edit: bool,
    },
    /// List tasks.
    List {
        /// Project to list tasks for.
        #[arg(long)]
        project: Option<String>,
        /// Filter by status (open, in-progress, done, wont-fix, or all).
        #[arg(long)]
        status: Option<TaskStatusFilter>,
        /// Filter by priority.
        #[arg(long)]
        priority: Option<Priority>,
        /// Filter by tag.
        #[arg(long)]
        tag: Option<String>,
    },
}

#[cfg(feature = "git")]
#[derive(Subcommand)]
pub(crate) enum RemoteCommand {
    /// Add a new remote.
    Add {
        /// Remote name (e.g., "origin").
        name: String,
        /// Remote URL.
        url: String,
    },
    /// Remove a remote.
    Remove {
        /// Remote name to remove.
        name: String,
    },
    /// List all remotes.
    List,
    /// Fetch from a remote.
    Fetch {
        /// Remote name (defaults to the configured default remote).
        name: Option<String>,
    },
    /// Push local commits to a remote.
    Push {
        /// Remote name (defaults to the configured default remote).
        name: Option<String>,
        /// Force push (overwrite remote history).
        #[arg(long)]
        force: bool,
    },
    /// Pull (fetch + fast-forward merge) from a remote.
    Pull {
        /// Remote name (defaults to the configured default remote).
        name: Option<String>,
    },
}

#[cfg(feature = "git")]
#[derive(Subcommand)]
pub(crate) enum BootstrapSubcommand {
    /// Diagnose a sandbox's readiness to bootstrap a plan repo.
    ///
    /// Checks whether the `rdm` binary is on `PATH`, whether a plan-repo
    /// root is configured, whether a plan-repo URL and access token are
    /// available, and — for GitHub HTTPS URLs — whether the token has the
    /// required scopes. Does not clone.
    Doctor {
        /// Plan-repo URL (falls back to `RDM_PLAN_REPO`).
        #[arg(long, env = "RDM_PLAN_REPO", hide_env_values = true)]
        plan_repo: Option<String>,
        /// Access token (falls back to `RDM_PLAN_REPO_TOKEN`).
        #[arg(long, env = "RDM_PLAN_REPO_TOKEN", hide_env_values = true)]
        token: Option<String>,
    },
}

#[cfg(feature = "git")]
#[derive(Subcommand)]
pub(crate) enum HookCommand {
    /// Install the post-merge and post-commit git hooks in the current
    /// directory's git repo.
    Install {
        /// Overwrite existing hooks.
        #[arg(long)]
        force: bool,
    },
    /// Remove the rdm git hooks (post-merge and post-commit).
    Uninstall,
    /// Run post-merge logic: parse Done: directives and mark phases/tasks done.
    PostMerge {
        /// Scan commits since this ref (tag, SHA, branch) instead of the
        /// default reflog anchor `HEAD@{1}`.
        #[arg(long)]
        since: Option<String>,
    },
    /// Run post-commit logic: on the default branch, parse Done: directives
    /// from HEAD and mark phases/tasks done.
    PostCommit,
}

#[derive(Subcommand)]
pub(crate) enum ConfigCommand {
    /// Get the resolved value of a config key.
    Get {
        /// Config key (e.g. default_project, default_format, stage, remote.default, root).
        key: String,
    },
    /// Set a config key.
    Set {
        /// Config key to set.
        key: String,
        /// Value to set.
        value: String,
        /// Write to global config instead of repo config.
        #[arg(long)]
        global: bool,
    },
    /// List all config keys with their resolved values and sources.
    List,
}

/// Item type argument for `--type` flag.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum ItemKindArg {
    Roadmap,
    Phase,
    Task,
}

impl From<ItemKindArg> for ItemKind {
    fn from(arg: ItemKindArg) -> Self {
        match arg {
            ItemKindArg::Roadmap => ItemKind::Roadmap,
            ItemKindArg::Phase => ItemKind::Phase,
            ItemKindArg::Task => ItemKind::Task,
        }
    }
}

/// Output format for command results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum OutputFormat {
    #[value(alias = "text")]
    Human,
    Json,
    Table,
    Markdown,
}

impl std::str::FromStr for OutputFormat {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "human" | "text" => Ok(Self::Human),
            "json" => Ok(Self::Json),
            "table" => Ok(Self::Table),
            "markdown" => Ok(Self::Markdown),
            _ => Err(format!("unknown format: {s}")),
        }
    }
}

impl std::fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Human => write!(f, "human"),
            Self::Json => write!(f, "json"),
            Self::Table => write!(f, "table"),
            Self::Markdown => write!(f, "markdown"),
        }
    }
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err:#}");
        process::exit(1);
    }
}

#[cfg(feature = "server")]
async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();

    #[cfg(unix)]
    {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler");
        tokio::select! {
            _ = ctrl_c => {},
            _ = sigterm.recv() => {},
        }
    }

    #[cfg(not(unix))]
    {
        ctrl_c.await.ok();
    }

    eprintln!("\nShutting down gracefully...");
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let global_config = paths::load_global_config();

    // Handle config commands early — some don't need a repo.
    if let Command::Config { command } = cli.command {
        return commands::config::run(command, &cli.root, &global_config);
    }

    let root = paths::resolve_root(cli.root, &global_config)?;
    let root = paths::expand_root(root)?;
    let repo_config = paths::load_repo_config(&root).with_global_defaults(&global_config);
    let staging = paths::resolve_staging(cli.stage, &repo_config);
    let format_str = paths::resolve_format(cli.format.map(|f| f.to_string()), &repo_config);
    let format: OutputFormat = format_str
        .parse::<OutputFormat>()
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    match cli.command {
        Command::Config { .. } => unreachable!("handled above"),
        Command::Init {
            default_project,
            default_format,
            #[cfg(feature = "git")]
            remote,
        } => {
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
                let store = rdm_store_git::GitStore::clone_remote(url, &root, None)
                    .context("failed to clone remote plan repo")?;

                // Validate and load config: must be a valid rdm plan repo (has rdm.toml).
                let mut config = rdm_core::io::load_config(&store)
                    .context("not a valid rdm plan repo — cloned repository has no rdm.toml")?;
                config.remote = Some(rdm_core::config::RemoteConfig {
                    default: Some("origin".to_string()),
                });
                if cli.stage {
                    config.stage = Some(true);
                }
                paths::save_repo_config(&root, &config).context("failed to update repo config")?;

                // Commit the config update.
                let store =
                    rdm_store_git::GitStore::new(&root).context("failed to open cloned repo")?;
                store
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
                if cli.stage {
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
            std::fs::create_dir_all(&root)
                .with_context(|| format!("failed to create {}", root.display()))?;

            // Build repo config from flags.
            let init_config = rdm_core::config::Config {
                default_project: default_project.clone(),
                stage: if cli.stage { Some(true) } else { None },
                ..Default::default()
            };

            let mut store = commands::make_init_store(&root)?;
            rdm_core::ops::init::init_with_config(&mut store, init_config)
                .context("failed to initialize plan repo")?;

            // Create project directory if --default-project was given.
            if let Some(ref proj) = default_project {
                rdm_core::ops::project::create_project(&mut store, proj, proj)
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
            if cli.stage {
                println!("  staging mode: enabled");
            }
            println!();
            println!("Next steps:");
            if default_project.is_none() {
                println!("  rdm project create <name>  # create a project");
            }
            println!("  rdm roadmap create <slug>  # create a roadmap");
            println!("  rdm task create <slug>     # create a task");
        }

        #[cfg(feature = "git")]
        Command::Bootstrap {
            plan_repo,
            path,
            branch,
            init,
            token,
            command,
        } => match command {
            Some(BootstrapSubcommand::Doctor {
                plan_repo: doc_plan_repo,
                token: doc_token,
            }) => {
                let exit_code = commands::bootstrap::doctor(
                    doc_plan_repo.or(plan_repo).as_deref(),
                    doc_token.or(token).as_deref(),
                );
                process::exit(exit_code);
            }
            None => {
                let url = plan_repo.as_deref().ok_or_else(|| {
                    anyhow::anyhow!("--plan-repo is required (or pass a subcommand like `doctor`)")
                })?;
                commands::bootstrap::run(url, path, branch, init, token.as_deref())?;
            }
        },

        Command::Index => {
            let mut store = commands::make_store(&root, staging)?;
            rdm_core::ops::index::generate_index(&mut store).context("failed to generate index")?;
            println!("Generated INDEX.md");
        }

        Command::Project { command } => {
            let mut store = commands::make_store(&root, staging)?;
            commands::project::run(command, &mut store, format, cli.no_index, staging)?;
        }

        Command::Roadmap { command } => {
            let mut store = commands::make_store(&root, staging)?;
            commands::roadmap::run(
                command,
                &mut store,
                &repo_config,
                format,
                cli.no_index,
                staging,
            )?;
        }

        Command::Phase { command } => {
            let mut store = commands::make_store(&root, staging)?;
            commands::phase::run(
                command,
                &mut store,
                &repo_config,
                format,
                cli.no_index,
                staging,
            )?;
        }

        Command::Task { command } => {
            let mut store = commands::make_store(&root, staging)?;
            commands::task::run(
                command,
                &mut store,
                &repo_config,
                format,
                cli.no_index,
                staging,
            )?;
        }

        Command::Promote {
            task_slug,
            roadmap_slug,
            project,
        } => {
            let mut store = commands::make_store(&root, staging)?;
            let project = paths::resolve_project(project, &repo_config)?;
            let doc =
                rdm_core::ops::task::promote_task(&mut store, &project, &task_slug, &roadmap_slug)
                    .context("failed to promote task")?;
            println!(
                "Promoted task '{task_slug}' → roadmap '{}'",
                doc.frontmatter.roadmap
            );
            commands::maybe_regenerate_index(&mut store, cli.no_index, staging, Some(&project))?;
        }

        Command::Tree { project } => {
            let store = commands::make_store(&root, staging)?;
            let project = paths::resolve_project(project, &repo_config)?;
            let node = tree::build_tree(&store, &project).context("failed to build tree")?;
            match format {
                OutputFormat::Human => print!("{}", tree::format_tree(&node)),
                OutputFormat::Markdown => print!("{}", tree::format_tree_md(&node)),
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&node).context("failed to serialize tree")?
                    );
                }
                OutputFormat::Table => bail!(
                    "--format table is not supported for 'tree'; use --format human, --format json, --format markdown, or omit --format"
                ),
            }
            commands::maybe_print_uncommitted_hint(&store, staging);
        }

        Command::Describe { entity } => {
            let entities = rdm_core::describe::all_entities();
            match entity {
                None => {
                    let output = match format {
                        OutputFormat::Json => serde_json::to_string_pretty(&entities)?,
                        OutputFormat::Markdown => {
                            rdm_core::describe::format_entity_list_md(&entities)
                        }
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
                                OutputFormat::Markdown => {
                                    rdm_core::describe::format_entity_detail_md(e)
                                }
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
        }

        Command::AgentConfig {
            platform,
            project,
            out,
            principles_file,
            skills,
            mcp,
            user,
        } => {
            let platform: Platform = platform.parse().map_err(|e: String| anyhow::anyhow!(e))?;

            // Resolve output directory: --user resolves to platform's user-level dir,
            // --out uses the provided path, otherwise None (stdout).
            let resolved_out = if user {
                if skills {
                    Some(Platform::user_level_skills_dir().map_err(|e| anyhow::anyhow!(e))?)
                } else {
                    Some(platform.user_level_dir().map_err(|e| anyhow::anyhow!(e))?)
                }
            } else {
                out
            };

            if skills {
                if platform != Platform::Claude {
                    bail!("--skills is only supported for the claude platform");
                }
                let dir = resolved_out.as_ref().ok_or_else(|| {
                    anyhow::anyhow!(
                        "--skills requires --out or --user to specify the output directory"
                    )
                })?;
                let skill_files = agent_config::generate_skills(&SkillOptions {
                    project,
                    principles_file,
                    mcp,
                });
                for skill in &skill_files {
                    let path = dir.join(skill.relative_path);
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent).with_context(|| {
                            format!("failed to create directory {}", parent.display())
                        })?;
                    }
                    std::fs::write(&path, &skill.content)
                        .with_context(|| format!("failed to write {}", path.display()))?;
                    println!("Wrote {}", path.display());
                }
                // When --mcp, also write .mcp.json alongside skills
                if mcp {
                    let root_str = root.to_string_lossy().to_string();
                    let mcp_content = agent_config::generate_mcp_config(&McpConfigOptions {
                        root: Some(root_str),
                    });
                    let mcp_path = dir.join(".mcp.json");
                    std::fs::write(&mcp_path, &mcp_content)
                        .with_context(|| format!("failed to write {}", mcp_path.display()))?;
                    println!("Wrote {}", mcp_path.display());
                }
            } else {
                let content = agent_config::generate_agent_config(&AgentConfigOptions {
                    platform,
                    project,
                    principles_file,
                    mcp,
                });
                if let Some(dir) = resolved_out {
                    let path = dir.join(platform.conventional_path());
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent).with_context(|| {
                            format!("failed to create directory {}", parent.display())
                        })?;
                    }
                    std::fs::write(&path, &content)
                        .with_context(|| format!("failed to write {}", path.display()))?;
                    println!("Wrote {}", path.display());
                    // When --mcp, also write .mcp.json alongside instructions
                    if mcp {
                        let root_str = root.to_string_lossy().to_string();
                        let mcp_content = agent_config::generate_mcp_config(&McpConfigOptions {
                            root: Some(root_str),
                        });
                        let mcp_path = dir.join(".mcp.json");
                        std::fs::write(&mcp_path, &mcp_content)
                            .with_context(|| format!("failed to write {}", mcp_path.display()))?;
                        println!("Wrote {}", mcp_path.display());
                    }
                } else {
                    print!("{content}");
                }
            }
        }

        Command::Search {
            query,
            kind,
            status,
            project,
            tags,
            limit,
            min_score_ratio,
        } => {
            let store = commands::make_store(&root, staging)?;
            let item_status = status
                .as_deref()
                .map(|s| commands::parse_status(s, kind))
                .transpose()?;
            let filter = SearchFilter {
                kind: kind.map(ItemKind::from),
                project,
                status: item_status,
                tags: if tags.is_empty() { None } else { Some(tags) },
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
                    let json_results: Vec<_> =
                        results.iter().map(json::search_result_to_json).collect();
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&json_results)
                            .context("failed to serialize results")?
                    );
                }
            }
            commands::maybe_print_uncommitted_hint(&store, staging);
        }

        #[cfg(feature = "mcp")]
        Command::Mcp => {
            let auto_init = global_config.auto_init.unwrap_or(false);
            let rt = tokio::runtime::Runtime::new().context("failed to create tokio runtime")?;
            rt.block_on(rdm_mcp::run(root, auto_init, staging))?;
        }

        #[cfg(feature = "server")]
        Command::Serve { port, bind } => {
            let rt = tokio::runtime::Runtime::new().context("failed to create tokio runtime")?;
            rt.block_on(async {
                let state = rdm_server::state::AppState {
                    plan_root: root.clone(),
                };
                let app = rdm_server::router::build_router(state);
                let addr = format!("{bind}:{port}");
                let listener = tokio::net::TcpListener::bind(&addr)
                    .await
                    .with_context(|| format!("failed to bind to {addr}"))?;
                let local_addr = listener.local_addr()?;
                eprintln!("rdm serve listening on http://{local_addr}");
                axum::serve(listener, app)
                    .with_graceful_shutdown(shutdown_signal())
                    .await
                    .context("server error")?;
                Ok::<(), anyhow::Error>(())
            })?;
        }

        #[cfg(feature = "git")]
        Command::Status { fetch } => {
            let mut store = commands::make_store(&root, staging)?;

            // Check for merge in progress
            if store
                .git_is_merge_in_progress()
                .context("failed to check merge state")?
            {
                let unmerged = store
                    .git_list_unmerged()
                    .context("failed to list unmerged files")?;
                let count = unmerged.len();
                if count > 0 {
                    println!(
                        "Merge in progress — {count} conflict(s) remaining. Run `rdm conflicts` for details."
                    );
                } else {
                    println!(
                        "Merge in progress — all conflicts resolved. Run `rdm resolve <file>` or `git commit --no-edit` to complete."
                    );
                }
                println!();
            }

            let statuses = store.git_status().context("failed to get git status")?;
            if statuses.is_empty() {
                println!("No uncommitted changes.");
            } else {
                println!("Uncommitted changes:");
                for fs in &statuses {
                    let prefix = match fs.change {
                        rdm_store_git::FileChange::Added => "  added:    ",
                        rdm_store_git::FileChange::Modified => "  modified: ",
                        rdm_store_git::FileChange::Deleted => "  deleted:  ",
                    };
                    println!("{prefix}{}", fs.path);
                }
                println!(
                    "\n{} file(s) changed. Run `rdm commit` to persist or `rdm discard --force` to reset.",
                    statuses.len()
                );
            }

            // Show sync status if a default remote is configured
            let config_path = root.join("rdm.toml");
            let default_remote = std::fs::read_to_string(&config_path)
                .ok()
                .and_then(|s| rdm_core::config::Config::from_toml(&s).ok())
                .and_then(|c| c.remote)
                .and_then(|r| r.default);
            if let Some(remote_name) = default_remote {
                if fetch && let Err(e) = store.git_fetch(&remote_name) {
                    eprintln!("warning: fetch failed: {e}");
                }
                match store.git_sync_status(&remote_name) {
                    Ok(Some(sync)) => {
                        println!();
                        match (sync.ahead, sync.behind) {
                            (0, 0) => {
                                println!("Up to date with '{}/{}'.", sync.remote, sync.branch)
                            }
                            (a, 0) => println!(
                                "Your branch is ahead of '{}/{}' by {} commit(s).",
                                sync.remote, sync.branch, a
                            ),
                            (0, b) => println!(
                                "Your branch is behind '{}/{}' by {} commit(s).",
                                sync.remote, sync.branch, b
                            ),
                            (a, b) => println!(
                                "Your branch and '{}/{}' have diverged ({} ahead, {} behind).",
                                sync.remote, sync.branch, a, b
                            ),
                        }
                    }
                    Ok(None) => {
                        // No tracking ref — silently skip
                    }
                    Err(e) => {
                        eprintln!("warning: could not determine sync status: {e}");
                    }
                }
            }
        }

        #[cfg(feature = "git")]
        Command::Commit { message } => {
            let store = commands::make_store(&root, staging)?;
            let statuses = store.git_status().context("failed to get git status")?;
            if statuses.is_empty() {
                println!("Nothing to commit.");
            } else {
                let msg = message.unwrap_or_else(|| {
                    let summary: Vec<String> = statuses
                        .iter()
                        .map(|s| {
                            let kind = match s.change {
                                rdm_store_git::FileChange::Added => "add",
                                rdm_store_git::FileChange::Modified => "update",
                                rdm_store_git::FileChange::Deleted => "delete",
                            };
                            format!("{kind} {}", s.path)
                        })
                        .collect();
                    if summary.len() == 1 {
                        format!("rdm: {}", summary[0])
                    } else {
                        let mut msg = format!("rdm: update {} files", statuses.len());
                        for s in &summary {
                            msg.push_str(&format!("\n\n- {s}"));
                        }
                        msg
                    }
                });
                store
                    .git_commit(&msg)
                    .context("failed to create git commit")?;
                println!("Committed {} file(s).", statuses.len());
            }
        }

        #[cfg(feature = "git")]
        Command::Discard { force } => {
            if !force {
                bail!("discarding changes is irreversible — pass --force to confirm");
            }
            let mut store = commands::make_store(&root, staging)?;
            // Abort merge if one is in progress
            if store
                .git_is_merge_in_progress()
                .context("failed to check merge state")?
            {
                store.git_merge_abort().context("failed to abort merge")?;
                println!("Aborted in-progress merge.");
            }
            let statuses = store.git_status().context("failed to get git status")?;
            if statuses.is_empty() {
                println!("Nothing to discard.");
            } else {
                store.git_discard().context("failed to discard changes")?;
                println!("Discarded {} file(s).", statuses.len());
                for fs in &statuses {
                    let prefix = match fs.change {
                        rdm_store_git::FileChange::Added => "  removed:  ",
                        rdm_store_git::FileChange::Modified => "  restored: ",
                        rdm_store_git::FileChange::Deleted => "  restored: ",
                    };
                    println!("{prefix}{}", fs.path);
                }
            }
        }

        #[cfg(feature = "git")]
        Command::Conflicts => {
            let store = commands::make_store(&root, staging)?;
            if !store
                .git_is_merge_in_progress()
                .context("failed to check merge state")?
            {
                println!("No merge in progress.");
            } else {
                let unmerged = store
                    .git_list_unmerged()
                    .context("failed to list unmerged files")?;
                if unmerged.is_empty() {
                    println!("Merge in progress but all conflicts are resolved.");
                    println!(
                        "Run `rdm resolve <file>` on any remaining file to complete the merge,"
                    );
                    println!("or commit manually with `git commit --no-edit`.");
                } else {
                    println!(
                        "Merge in progress — {} conflict(s) remaining:\n",
                        unmerged.len()
                    );
                    for path in &unmerged {
                        let item = rdm_core::conflict::classify_path(path);
                        println!("  {path} — {item}");
                    }
                    println!();
                    println!("Edit each file to resolve conflicts, then run `rdm resolve <file>`.");
                    println!("Run `rdm discard --force` to abort the merge.");
                }
            }
        }

        #[cfg(feature = "git")]
        Command::Resolve { file } => {
            let mut store = commands::make_store(&root, staging)?;
            let result = store
                .git_resolve_conflict(&file)
                .context("failed to resolve conflict")?;
            println!("Resolved: {}", result.path);
            if result.merge_completed {
                println!("All conflicts resolved — merge complete.");
                // Regenerate INDEX.md after merge completion
                rdm_core::ops::index::generate_index(&mut store)
                    .context("failed to regenerate INDEX.md after merge")?;
            } else {
                println!(
                    "{} conflict(s) remaining. Run `rdm conflicts` to see them.",
                    result.remaining
                );
            }
        }

        #[cfg(feature = "git")]
        Command::Remote { command } => {
            let mut store = commands::make_store(&root, staging)?;
            commands::remote::run(command, &mut store, &root, &repo_config, staging)?;
        }

        #[cfg(feature = "git")]
        Command::Hook { command } => {
            commands::hook::run(command, &root, staging)?;
        }

        Command::List { project, all } => {
            let store = commands::make_store(&root, staging)?;
            let projects = if all {
                rdm_core::ops::project::list_projects(&store).context("failed to list projects")?
            } else {
                let p = paths::resolve_project(project, &repo_config)?;
                vec![p]
            };

            // For JSON, collect all projects' summaries into one array.
            let mut all_summaries: Vec<json::RoadmapSummaryJson> = Vec::new();

            for project in &projects {
                if all && format != OutputFormat::Json {
                    println!("Project: {project}");
                }
                let roadmaps = rdm_core::ops::roadmap::list_roadmaps(&store, project, None, None)
                    .context("failed to list roadmaps")?;
                let mut entries = Vec::new();
                for roadmap_doc in roadmaps {
                    let slug = &roadmap_doc.frontmatter.roadmap;
                    let phases = rdm_core::ops::phase::list_phases(&store, project, slug)
                        .with_context(|| format!("failed to list phases for roadmap '{slug}'"))?;
                    entries.push((roadmap_doc, phases));
                }
                match format {
                    OutputFormat::Human => print!("{}", display::format_roadmap_list(&entries)),
                    OutputFormat::Table => print!("{}", table::format_roadmap_table(&entries)),
                    OutputFormat::Markdown => {
                        print!("{}", display::format_roadmap_list_md(&entries))
                    }
                    OutputFormat::Json => {
                        for (doc, phases) in &entries {
                            all_summaries.push(json::roadmap_summary_to_json(doc, phases));
                        }
                    }
                }
            }
            if format == OutputFormat::Json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&all_summaries)
                        .context("failed to serialize roadmaps")?
                );
            }
            commands::maybe_print_uncommitted_hint(&store, staging);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn expand_root_tilde_expands_to_home() {
        let home = env::var("HOME").unwrap();
        let result = paths::expand_root(PathBuf::from("~")).unwrap();
        assert_eq!(result, PathBuf::from(&home));
    }

    #[test]
    fn expand_root_tilde_slash_expands_to_home_subpath() {
        let home = env::var("HOME").unwrap();
        let result = paths::expand_root(PathBuf::from("~/foo/bar")).unwrap();
        assert_eq!(result, PathBuf::from(format!("{home}/foo/bar")));
    }

    #[test]
    fn expand_root_dot_resolves_to_cwd() {
        let cwd = env::current_dir().unwrap();
        let result = paths::expand_root(PathBuf::from(".")).unwrap();
        assert_eq!(result, cwd);
    }

    #[test]
    fn expand_root_dotdot_resolves_relative_to_cwd() {
        let cwd = env::current_dir().unwrap();
        let result = paths::expand_root(PathBuf::from("../foo")).unwrap();
        let expected = cwd.parent().unwrap().join("foo");
        assert_eq!(result, expected);
    }

    #[test]
    fn expand_root_absolute_path_unchanged() {
        let result = paths::expand_root(PathBuf::from("/tmp/plans")).unwrap();
        assert_eq!(result, PathBuf::from("/tmp/plans"));
    }
}
