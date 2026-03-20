use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use is_terminal::IsTerminal;
use rdm_core::agent_config::{self, AgentConfigOptions, McpConfigOptions, Platform, SkillOptions};
use rdm_core::display;
use rdm_core::json;
use rdm_core::model::{PhaseStatus, Priority, TaskStatus, TaskStatusFilter};
use rdm_core::repo::PlanRepo;
use rdm_core::search::{self, ItemKind, ItemStatus, SearchFilter};
use rdm_core::tree;
#[cfg(not(feature = "git"))]
use rdm_store_fs::FsStore;

mod table;

#[cfg(feature = "git")]
type AppStore = rdm_store_git::GitStore;
#[cfg(not(feature = "git"))]
type AppStore = FsStore;

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
    #[arg(long, global = true, default_value = "human")]
    format: OutputFormat,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Initialize a new plan repo.
    Init,
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
        #[arg(long, conflicts_with = "mcp")]
        skills: bool,
        /// Generate .mcp.json configuration for MCP server.
        #[arg(long, conflicts_with = "skills")]
        mcp: bool,
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
        /// Maximum number of results to return.
        #[arg(long, default_value = "20")]
        limit: usize,
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
enum ProjectCommand {
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
enum RoadmapCommand {
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
    /// List all roadmaps in a project.
    List {
        /// Project to list roadmaps for.
        #[arg(long)]
        project: Option<String>,
        /// Show archived roadmaps instead of active ones.
        #[arg(long)]
        archived: bool,
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
enum PhaseCommand {
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
        /// Body content for the phase.
        #[arg(long)]
        body: Option<String>,
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
enum TaskCommand {
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
enum RemoteCommand {
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

/// Item type argument for `--type` flag.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum ItemKindArg {
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
enum OutputFormat {
    #[value(alias = "text")]
    Human,
    Json,
    Table,
    Markdown,
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

/// Parses a status string into an `ItemStatus`, using the `--type` hint if available.
fn parse_status(status: &str, kind: Option<ItemKindArg>) -> Result<ItemStatus> {
    match kind {
        Some(ItemKindArg::Phase) => {
            let s: PhaseStatus = status.parse().map_err(|e: String| anyhow::anyhow!("{e}"))?;
            Ok(ItemStatus::Phase(s))
        }
        Some(ItemKindArg::Task) => {
            let s: TaskStatus = status.parse().map_err(|e: String| anyhow::anyhow!("{e}"))?;
            Ok(ItemStatus::Task(s))
        }
        Some(ItemKindArg::Roadmap) => {
            bail!("roadmaps do not have a status — remove --status or change --type")
        }
        None => {
            // Try both; phase first then task
            if let Ok(s) = status.parse::<PhaseStatus>() {
                Ok(ItemStatus::Phase(s))
            } else if let Ok(s) = status.parse::<TaskStatus>() {
                Ok(ItemStatus::Task(s))
            } else {
                bail!(
                    "invalid status '{status}' — use a phase status (not-started, in-progress, done, blocked) or task status (open, in-progress, done, wont-fix)"
                )
            }
        }
    }
}

/// Resolves whether staging mode is active.
///
/// Priority: `--stage` flag / `RDM_STAGE` env > `stage` in `rdm.toml` > false.
fn resolve_staging(flag: bool, root: &Path) -> bool {
    if flag {
        return true;
    }
    // Check rdm.toml for stage setting
    let config_path = root.join("rdm.toml");
    if let Ok(contents) = std::fs::read_to_string(&config_path)
        && let Ok(config) = rdm_core::config::Config::from_toml(&contents)
        && config.stage == Some(true)
    {
        return true;
    }
    false
}

/// Opens a store for an existing plan repo.
fn make_store(root: &Path, staging: bool) -> Result<AppStore> {
    #[cfg(feature = "git")]
    {
        Ok(rdm_store_git::GitStore::new(root)
            .context("failed to open git repository")?
            .with_staging_mode(staging))
    }
    #[cfg(not(feature = "git"))]
    {
        let _ = staging;
        Ok(FsStore::new(root))
    }
}

/// Creates a store for initializing a new plan repo.
fn make_init_store(root: &Path) -> Result<AppStore> {
    #[cfg(feature = "git")]
    {
        rdm_store_git::GitStore::init(root).context("failed to initialize git repository")
    }
    #[cfg(not(feature = "git"))]
    {
        Ok(FsStore::new(root))
    }
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err:#}");
        process::exit(1);
    }
}

/// Resolves project: --project flag > `RDM_PROJECT` env var > config default_project > error.
fn resolve_project(flag: Option<String>, repo: &PlanRepo<AppStore>) -> Result<String> {
    if let Some(p) = flag {
        return Ok(p);
    }
    if let Ok(p) = std::env::var("RDM_PROJECT") {
        return Ok(p);
    }
    if let Ok(config) = repo.load_config()
        && let Some(p) = config.default_project
    {
        return Ok(p);
    }
    bail!(
        "no project specified — use --project, set RDM_PROJECT, or set default_project in rdm.toml"
    )
}

/// Resolve a remote name from an explicit argument or `remote.default` in `rdm.toml`.
#[cfg(feature = "git")]
fn resolve_remote_name(name: Option<String>, root: &Path) -> Result<String> {
    if let Some(n) = name {
        return Ok(n);
    }
    let config_path = root.join("rdm.toml");
    let default = std::fs::read_to_string(&config_path)
        .ok()
        .and_then(|s| rdm_core::config::Config::from_toml(&s).ok())
        .and_then(|c| c.remote)
        .and_then(|r| r.default);
    match default {
        Some(d) => Ok(d),
        None => bail!("no remote specified — pass a remote name or set remote.default in rdm.toml"),
    }
}

/// Resolve body content from `--body` flag, piped stdin, or interactive editor.
/// Returns an error if both `--body` and stdin are provided.
fn resolve_body(body_flag: Option<String>, no_edit: bool) -> Result<Option<String>> {
    let is_tty = io::stdin().is_terminal();

    let stdin_body = if !is_tty {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf)?;
        let trimmed = buf.trim_end_matches('\n');
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    } else {
        None
    };

    match (body_flag, stdin_body) {
        (Some(_), Some(_)) => bail!("cannot use --body and piped stdin together; pick one"),
        (Some(b), None) => Ok(Some(b)),
        (None, Some(s)) => Ok(Some(s)),
        (None, None) => {
            if no_edit || !is_tty {
                Ok(None)
            } else {
                open_editor()
            }
        }
    }
}

/// Launch `$VISUAL` / `$EDITOR` / `vi` to interactively edit body content.
/// Returns `None` if the user saves an empty file.
fn open_editor() -> Result<Option<String>> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());

    let mut tmp = tempfile::Builder::new()
        .suffix(".md")
        .tempfile()
        .context("failed to create temp file for editor")?;

    writeln!(
        tmp,
        "<!-- Enter body content below. This comment will be removed. -->"
    )?;
    tmp.flush()?;

    let path = tmp.path().to_owned();

    let status = std::process::Command::new(&editor)
        .arg(&path)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()
        .with_context(|| format!("failed to launch editor '{editor}'"))?;

    if !status.success() {
        bail!("editor exited with non-zero status");
    }

    let content = std::fs::read_to_string(&path).context("failed to read editor temp file")?;

    let body: String = content
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !(trimmed.starts_with("<!--") && trimmed.ends_with("-->"))
        })
        .collect::<Vec<_>>()
        .join("\n");
    let body = body.trim().to_string();

    if body.is_empty() {
        Ok(None)
    } else {
        Ok(Some(body))
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

fn reject_non_human(format: OutputFormat, command_name: &str) -> Result<()> {
    if format != OutputFormat::Human {
        bail!(
            "--format {format} is not supported for '{command_name}'; use --format human or omit --format"
        );
    }
    Ok(())
}

fn maybe_regenerate_index(
    repo: &mut PlanRepo<AppStore>,
    no_index: bool,
    staging: bool,
) -> Result<()> {
    if !no_index {
        repo.generate_index()
            .context("failed to regenerate INDEX.md")?;
    }
    if staging {
        println!("  (staged — run `rdm commit` to persist)");
    }
    Ok(())
}

/// Prints a hint about uncommitted changes when staging mode is active.
///
/// Called after read-only commands (list, show, search) so the user is aware
/// that the data they see includes uncommitted staged mutations.
#[cfg(feature = "git")]
fn maybe_print_uncommitted_hint(store: &AppStore, staging: bool) {
    if !staging {
        return;
    }
    if let Ok(statuses) = store.git_status()
        && !statuses.is_empty()
    {
        println!(
            "\n  ({} uncommitted change(s) — run `rdm status` for details)",
            statuses.len()
        );
    }
}

#[cfg(not(feature = "git"))]
fn maybe_print_uncommitted_hint(_store: &AppStore, _staging: bool) {}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let format = cli.format;
    let root = cli
        .root
        .unwrap_or_else(|| std::env::current_dir().expect("cannot determine current directory"));
    let staging = resolve_staging(cli.stage, &root);

    match cli.command {
        Command::Init => {
            PlanRepo::init(make_init_store(&root)?).context("failed to initialize plan repo")?;
            println!("Initialized plan repo at {}", root.display());
        }

        Command::Index => {
            let mut repo = PlanRepo::new(make_store(&root, staging)?);
            repo.generate_index().context("failed to generate index")?;
            println!("Generated INDEX.md");
        }

        Command::Project { command } => {
            let mut repo = PlanRepo::new(make_store(&root, staging)?);
            match command {
                ProjectCommand::Create { name, title } => {
                    let title = title.as_deref().unwrap_or(&name);
                    let doc = repo
                        .create_project(&name, title)
                        .context("failed to create project")?;
                    println!("Created project '{}'", doc.frontmatter.name);
                    maybe_regenerate_index(&mut repo, cli.no_index, staging)?;
                }
                ProjectCommand::Show { name } => {
                    let doc = repo.load_project(&name).context("failed to load project")?;
                    match format {
                        OutputFormat::Json => {
                            let j = json::project_to_json(&doc);
                            println!(
                                "{}",
                                serde_json::to_string_pretty(&j)
                                    .context("failed to serialize project")?
                            );
                        }
                        OutputFormat::Markdown => {
                            println!("# {}", doc.frontmatter.title);
                            println!();
                            println!("- **Name:** {}", doc.frontmatter.name);
                            if !doc.body.is_empty() {
                                println!();
                                println!("{}", doc.body);
                            }
                        }
                        OutputFormat::Table => bail!(
                            "--format table is not supported for 'project show'; use --format human, --format json, --format markdown, or omit --format"
                        ),
                        OutputFormat::Human => {
                            println!("{} ({})", doc.frontmatter.title, doc.frontmatter.name);
                            if !doc.body.is_empty() {
                                println!();
                                println!("{}", doc.body);
                            }
                        }
                    }
                    maybe_print_uncommitted_hint(repo.store(), staging);
                }
                ProjectCommand::List => {
                    let projects = repo.list_projects().context("failed to list projects")?;
                    match format {
                        OutputFormat::Json => {
                            println!(
                                "{}",
                                serde_json::to_string_pretty(&projects)
                                    .context("failed to serialize projects")?
                            );
                        }
                        _ => {
                            reject_non_human(format, "project list")?;
                            if projects.is_empty() {
                                println!("No projects yet.");
                            } else {
                                for p in &projects {
                                    println!("{p}");
                                }
                            }
                        }
                    }
                    maybe_print_uncommitted_hint(repo.store(), staging);
                }
            }
        }

        Command::Roadmap { command } => {
            let mut repo = PlanRepo::new(make_store(&root, staging)?);
            match command {
                RoadmapCommand::Create {
                    slug,
                    title,
                    project,
                    body,
                    no_edit,
                } => {
                    let project = resolve_project(project, &repo)?;
                    let title = title.as_deref().unwrap_or(&slug);
                    let body = resolve_body(body, no_edit)?;
                    repo.create_roadmap(&project, &slug, title, body.as_deref())
                        .context("failed to create roadmap")?;
                    println!("Created roadmap '{slug}' in project '{project}'");
                    maybe_regenerate_index(&mut repo, cli.no_index, staging)?;
                }
                RoadmapCommand::Show {
                    slug,
                    project,
                    no_body,
                } => {
                    let project = resolve_project(project, &repo)?;
                    let mut roadmap_doc = repo
                        .load_roadmap(&project, &slug)
                        .context("failed to load roadmap")?;
                    let phases = repo
                        .list_phases(&project, &slug)
                        .context("failed to list phases")?;
                    if no_body {
                        roadmap_doc.body = String::new();
                    }
                    match format {
                        OutputFormat::Human => {
                            print!("{}", display::format_roadmap_summary(&roadmap_doc, &phases))
                        }
                        OutputFormat::Markdown => print!(
                            "{}",
                            display::format_roadmap_summary_md(&roadmap_doc, &phases)
                        ),
                        OutputFormat::Json => {
                            let j = json::roadmap_to_json(&roadmap_doc, &phases);
                            println!(
                                "{}",
                                serde_json::to_string_pretty(&j)
                                    .context("failed to serialize roadmap")?
                            );
                        }
                        OutputFormat::Table => bail!(
                            "--format table is not supported for 'roadmap show'; use --format human, --format json, --format markdown, or omit --format"
                        ),
                    }
                    maybe_print_uncommitted_hint(repo.store(), staging);
                }
                RoadmapCommand::List { project, archived } => {
                    let project = resolve_project(project, &repo)?;
                    let entries = if archived {
                        let roadmaps = repo
                            .list_archived_roadmaps(&project)
                            .context("failed to list archived roadmaps")?;
                        let mut entries = Vec::new();
                        for roadmap_doc in roadmaps {
                            let slug = &roadmap_doc.frontmatter.roadmap;
                            let phases =
                                repo.list_archived_phases(&project, slug).with_context(|| {
                                    format!("failed to list phases for archived roadmap '{slug}'")
                                })?;
                            entries.push((roadmap_doc, phases));
                        }
                        entries
                    } else {
                        let roadmaps = repo
                            .list_roadmaps(&project)
                            .context("failed to list roadmaps")?;
                        let mut entries = Vec::new();
                        for roadmap_doc in roadmaps {
                            let slug = &roadmap_doc.frontmatter.roadmap;
                            let phases = repo.list_phases(&project, slug).with_context(|| {
                                format!("failed to list phases for roadmap '{slug}'")
                            })?;
                            entries.push((roadmap_doc, phases));
                        }
                        entries
                    };
                    match format {
                        OutputFormat::Human => print!("{}", display::format_roadmap_list(&entries)),
                        OutputFormat::Table => print!("{}", table::format_roadmap_table(&entries)),
                        OutputFormat::Markdown => {
                            print!("{}", display::format_roadmap_list_md(&entries))
                        }
                        OutputFormat::Json => {
                            let summaries: Vec<_> = entries
                                .iter()
                                .map(|(doc, phases)| json::roadmap_summary_to_json(doc, phases))
                                .collect();
                            println!(
                                "{}",
                                serde_json::to_string_pretty(&summaries)
                                    .context("failed to serialize roadmaps")?
                            );
                        }
                    }
                    maybe_print_uncommitted_hint(repo.store(), staging);
                }
                RoadmapCommand::Depend { slug, on, project } => {
                    let project = resolve_project(project, &repo)?;
                    repo.add_dependency(&project, &slug, &on)
                        .context("failed to add dependency")?;
                    println!("Added dependency: {slug} → {on}");
                    maybe_regenerate_index(&mut repo, cli.no_index, staging)?;
                }
                RoadmapCommand::Undepend { slug, on, project } => {
                    let project = resolve_project(project, &repo)?;
                    repo.remove_dependency(&project, &slug, &on)
                        .context("failed to remove dependency")?;
                    println!("Removed dependency: {slug} → {on}");
                    maybe_regenerate_index(&mut repo, cli.no_index, staging)?;
                }
                RoadmapCommand::Deps { project } => {
                    reject_non_human(format, "roadmap deps")?;
                    let project = resolve_project(project, &repo)?;
                    let graph = repo
                        .dependency_graph(&project)
                        .context("failed to get dependency graph")?;
                    print!("{}", display::format_dependency_graph(&graph));
                    maybe_print_uncommitted_hint(repo.store(), staging);
                }
                RoadmapCommand::Delete {
                    slug,
                    project,
                    force,
                } => {
                    if !force {
                        bail!(
                            "deleting a roadmap is irreversible — pass --force to confirm deletion of '{slug}'"
                        );
                    }
                    let project = resolve_project(project, &repo)?;
                    repo.delete_roadmap(&project, &slug)
                        .context("failed to delete roadmap")?;
                    println!("Deleted roadmap '{slug}' from project '{project}'");
                    maybe_regenerate_index(&mut repo, cli.no_index, staging)?;
                }
                RoadmapCommand::Split {
                    slug,
                    phases,
                    into,
                    title,
                    project,
                    depends_on,
                } => {
                    let project = resolve_project(project, &repo)?;
                    // Resolve each phase identifier (number or stem)
                    let resolved_stems: Vec<String> = phases
                        .iter()
                        .map(|p| repo.resolve_phase_stem(&project, &slug, p))
                        .collect::<std::result::Result<Vec<_>, _>>()
                        .context("failed to resolve phase identifiers")?;
                    let dep = if depends_on {
                        Some(slug.as_str())
                    } else {
                        None
                    };
                    repo.split_roadmap(&project, &slug, &into, &title, &resolved_stems, dep)
                        .context("failed to split roadmap")?;
                    println!(
                        "Split {} phase(s) from '{slug}' into new roadmap '{into}'",
                        resolved_stems.len()
                    );
                    maybe_regenerate_index(&mut repo, cli.no_index, staging)?;
                }
                RoadmapCommand::Archive {
                    slug,
                    project,
                    force,
                } => {
                    let project = resolve_project(project, &repo)?;
                    repo.archive_roadmap(&project, &slug, force)
                        .context("failed to archive roadmap")?;
                    println!("Archived roadmap '{slug}' from project '{project}'");
                    maybe_regenerate_index(&mut repo, cli.no_index, staging)?;
                }
                RoadmapCommand::Unarchive { slug, project } => {
                    let project = resolve_project(project, &repo)?;
                    repo.unarchive_roadmap(&project, &slug)
                        .context("failed to unarchive roadmap")?;
                    println!("Restored roadmap '{slug}' to project '{project}'");
                    maybe_regenerate_index(&mut repo, cli.no_index, staging)?;
                }
            }
        }

        Command::Phase { command } => {
            let mut repo = PlanRepo::new(make_store(&root, staging)?);
            match command {
                PhaseCommand::Create {
                    slug,
                    title,
                    roadmap,
                    project,
                    number,
                    body,
                    no_edit,
                } => {
                    let project = resolve_project(project, &repo)?;
                    let title = title.as_deref().unwrap_or(&slug);
                    let body = resolve_body(body, no_edit)?;
                    let doc = repo
                        .create_phase(&project, &roadmap, &slug, title, number, body.as_deref())
                        .context("failed to create phase")?;
                    let stem = doc.frontmatter.stem(&slug);
                    println!("Created phase '{stem}' in roadmap '{roadmap}'");
                    maybe_regenerate_index(&mut repo, cli.no_index, staging)?;
                }
                PhaseCommand::List { roadmap, project } => {
                    let project = resolve_project(project, &repo)?;
                    let phases = repo
                        .list_phases(&project, &roadmap)
                        .context("failed to list phases")?;
                    match format {
                        OutputFormat::Human => print!("{}", display::format_phase_list(&phases)),
                        OutputFormat::Table => print!("{}", table::format_phase_table(&phases)),
                        OutputFormat::Markdown => {
                            print!("{}", display::format_phase_list_md(&phases))
                        }
                        OutputFormat::Json => {
                            let summaries: Vec<_> = phases
                                .iter()
                                .map(|(stem, doc)| json::phase_summary_to_json(stem, doc))
                                .collect();
                            println!(
                                "{}",
                                serde_json::to_string_pretty(&summaries)
                                    .context("failed to serialize phases")?
                            );
                        }
                    }
                    maybe_print_uncommitted_hint(repo.store(), staging);
                }
                PhaseCommand::Show {
                    stem,
                    roadmap,
                    project,
                    no_body,
                } => {
                    let project = resolve_project(project, &repo)?;
                    let stem = repo
                        .resolve_phase_stem(&project, &roadmap, &stem)
                        .context("failed to resolve phase")?;
                    let mut doc = repo
                        .load_phase(&project, &roadmap, &stem)
                        .context("failed to load phase")?;
                    if no_body {
                        doc.body = String::new();
                    }

                    // Compute prev/next phase stems for navigation
                    let phases = repo
                        .list_phases(&project, &roadmap)
                        .context("failed to list phases")?;
                    let pos = phases.iter().position(|(s, _)| s == &stem);
                    let prev_stem = pos.and_then(|i| {
                        if i > 0 {
                            Some(phases[i - 1].0.as_str())
                        } else {
                            None
                        }
                    });
                    let next_stem = pos.and_then(|i| phases.get(i + 1).map(|(s, _)| s.as_str()));

                    let nav = display::PhaseNav {
                        prev: prev_stem,
                        next: next_stem,
                        roadmap: &roadmap,
                        project: &project,
                    };

                    match format {
                        OutputFormat::Human => {
                            print!("{}", display::format_phase_detail(&stem, &doc, Some(&nav)))
                        }
                        OutputFormat::Markdown => {
                            print!(
                                "{}",
                                display::format_phase_detail_md(&stem, &doc, Some(&nav))
                            )
                        }
                        OutputFormat::Json => {
                            let j =
                                json::phase_to_json(&stem, &doc, &roadmap, prev_stem, next_stem);
                            println!(
                                "{}",
                                serde_json::to_string_pretty(&j)
                                    .context("failed to serialize phase")?
                            );
                        }
                        OutputFormat::Table => bail!(
                            "--format table is not supported for 'phase show'; use --format human, --format json, --format markdown, or omit --format"
                        ),
                    }
                    maybe_print_uncommitted_hint(repo.store(), staging);
                }
                PhaseCommand::Update {
                    stem,
                    status,
                    roadmap,
                    project,
                    body,
                    no_edit,
                } => {
                    let project = resolve_project(project, &repo)?;
                    let stem = repo
                        .resolve_phase_stem(&project, &roadmap, &stem)
                        .context("failed to resolve phase")?;
                    let body = resolve_body(body, no_edit)?;
                    let doc = repo
                        .update_phase(&project, &roadmap, &stem, status, body.as_deref())
                        .context("failed to update phase")?;
                    println!("Updated '{stem}' → {}", doc.frontmatter.status);
                    maybe_regenerate_index(&mut repo, cli.no_index, staging)?;
                }
                PhaseCommand::Remove {
                    stem,
                    roadmap,
                    project,
                } => {
                    let project = resolve_project(project, &repo)?;
                    let stem = repo
                        .resolve_phase_stem(&project, &roadmap, &stem)
                        .context("failed to resolve phase")?;
                    repo.remove_phase(&project, &roadmap, &stem)
                        .context("failed to remove phase")?;
                    println!("Removed phase '{stem}' from roadmap '{roadmap}'");
                    maybe_regenerate_index(&mut repo, cli.no_index, staging)?;
                }
            }
        }

        Command::Task { command } => {
            let mut repo = PlanRepo::new(make_store(&root, staging)?);
            match command {
                TaskCommand::Create {
                    slug,
                    title,
                    project,
                    priority,
                    tags,
                    body,
                    no_edit,
                } => {
                    let project = resolve_project(project, &repo)?;
                    let title = title.as_deref().unwrap_or(&slug);
                    let body = resolve_body(body, no_edit)?;
                    repo.create_task(&project, &slug, title, priority, tags, body.as_deref())
                        .context("failed to create task")?;
                    println!("Created task '{slug}' in project '{project}'");
                    maybe_regenerate_index(&mut repo, cli.no_index, staging)?;
                }
                TaskCommand::Show {
                    slug,
                    project,
                    no_body,
                } => {
                    let project = resolve_project(project, &repo)?;
                    let mut doc = repo
                        .load_task(&project, &slug)
                        .context("failed to load task")?;
                    if no_body {
                        doc.body = String::new();
                    }
                    match format {
                        OutputFormat::Human => {
                            print!("{}", display::format_task_detail(&slug, &doc))
                        }
                        OutputFormat::Markdown => {
                            print!("{}", display::format_task_detail_md(&slug, &doc))
                        }
                        OutputFormat::Json => {
                            let j = json::task_to_json(&slug, &doc);
                            println!(
                                "{}",
                                serde_json::to_string_pretty(&j)
                                    .context("failed to serialize task")?
                            );
                        }
                        OutputFormat::Table => bail!(
                            "--format table is not supported for 'task show'; use --format human, --format json, --format markdown, or omit --format"
                        ),
                    }
                    maybe_print_uncommitted_hint(repo.store(), staging);
                }
                TaskCommand::Update {
                    slug,
                    project,
                    status,
                    priority,
                    tags,
                    body,
                    no_edit,
                } => {
                    let project = resolve_project(project, &repo)?;
                    let body = resolve_body(body, no_edit)?;
                    let doc = repo
                        .update_task(&project, &slug, status, priority, tags, body.as_deref())
                        .context("failed to update task")?;
                    println!(
                        "Updated task '{slug}' → status: {}, priority: {}",
                        doc.frontmatter.status, doc.frontmatter.priority
                    );
                    maybe_regenerate_index(&mut repo, cli.no_index, staging)?;
                }
                TaskCommand::List {
                    project,
                    status,
                    priority,
                    tag,
                } => {
                    let project = resolve_project(project, &repo)?;
                    let all_tasks = repo.list_tasks(&project).context("failed to list tasks")?;

                    let filtered: Vec<(String, _)> = all_tasks
                        .into_iter()
                        .filter(|(_, doc)| match status {
                            Some(TaskStatusFilter::All) => true,
                            Some(TaskStatusFilter::Status(s)) => doc.frontmatter.status == s,
                            None => {
                                doc.frontmatter.status == TaskStatus::Open
                                    || doc.frontmatter.status == TaskStatus::InProgress
                            }
                        })
                        .filter(|(_, doc)| priority.is_none_or(|p| doc.frontmatter.priority == p))
                        .filter(|(_, doc)| {
                            tag.as_ref().is_none_or(|t| {
                                doc.frontmatter
                                    .tags
                                    .as_ref()
                                    .is_some_and(|tags| tags.contains(t))
                            })
                        })
                        .collect();

                    match format {
                        OutputFormat::Human => print!("{}", display::format_task_list(&filtered)),
                        OutputFormat::Table => print!("{}", table::format_task_table(&filtered)),
                        OutputFormat::Markdown => {
                            print!("{}", display::format_task_list_md(&filtered))
                        }
                        OutputFormat::Json => {
                            let summaries: Vec<_> = filtered
                                .iter()
                                .map(|(slug, doc)| json::task_summary_to_json(slug, doc))
                                .collect();
                            println!(
                                "{}",
                                serde_json::to_string_pretty(&summaries)
                                    .context("failed to serialize tasks")?
                            );
                        }
                    }
                    maybe_print_uncommitted_hint(repo.store(), staging);
                }
            }
        }

        Command::Promote {
            task_slug,
            roadmap_slug,
            project,
        } => {
            let mut repo = PlanRepo::new(make_store(&root, staging)?);
            let project = resolve_project(project, &repo)?;
            let doc = repo
                .promote_task(&project, &task_slug, &roadmap_slug)
                .context("failed to promote task")?;
            println!(
                "Promoted task '{task_slug}' → roadmap '{}'",
                doc.frontmatter.roadmap
            );
            maybe_regenerate_index(&mut repo, cli.no_index, staging)?;
        }

        Command::Tree { project } => {
            let repo = PlanRepo::new(make_store(&root, staging)?);
            let project = resolve_project(project, &repo)?;
            let node = tree::build_tree(&repo, &project).context("failed to build tree")?;
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
            maybe_print_uncommitted_hint(repo.store(), staging);
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
        } => {
            if mcp {
                let root_str = root.to_string_lossy().to_string();
                let content = agent_config::generate_mcp_config(&McpConfigOptions {
                    root: Some(root_str),
                });
                if let Some(dir) = out {
                    let path = dir.join(".mcp.json");
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent).with_context(|| {
                            format!("failed to create directory {}", parent.display())
                        })?;
                    }
                    std::fs::write(&path, &content)
                        .with_context(|| format!("failed to write {}", path.display()))?;
                    println!("Wrote {}", path.display());
                } else {
                    print!("{content}");
                }
                return Ok(());
            }

            let platform: Platform = platform.parse().map_err(|e: String| anyhow::anyhow!(e))?;

            if skills {
                if platform != Platform::Claude {
                    bail!("--skills is only supported for the claude platform");
                }
                let dir = out.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("--skills requires --out to specify the output directory")
                })?;
                let skill_files = agent_config::generate_skills(&SkillOptions {
                    project,
                    principles_file,
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
            } else {
                let content = agent_config::generate_agent_config(&AgentConfigOptions {
                    platform,
                    project,
                    principles_file,
                });
                if let Some(dir) = out {
                    let path = dir.join(platform.conventional_path());
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent).with_context(|| {
                            format!("failed to create directory {}", parent.display())
                        })?;
                    }
                    std::fs::write(&path, &content)
                        .with_context(|| format!("failed to write {}", path.display()))?;
                    println!("Wrote {}", path.display());
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
            limit,
        } => {
            let repo = PlanRepo::new(make_store(&root, staging)?);
            let item_status = status
                .as_deref()
                .map(|s| parse_status(s, kind))
                .transpose()?;
            let filter = SearchFilter {
                kind: kind.map(ItemKind::from),
                project,
                status: item_status,
            };
            let results = search::search(&repo, &query, &filter).context("search failed")?;
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
            maybe_print_uncommitted_hint(repo.store(), staging);
        }

        #[cfg(feature = "mcp")]
        Command::Mcp => {
            let rt = tokio::runtime::Runtime::new().context("failed to create tokio runtime")?;
            rt.block_on(rdm_mcp::run(root))?;
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
            let mut store = make_store(&root, staging)?;

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
            let store = make_store(&root, staging)?;
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
            let mut store = make_store(&root, staging)?;
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
            let store = make_store(&root, staging)?;
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
            let mut store = make_store(&root, staging)?;
            let result = store
                .git_resolve_conflict(&file)
                .context("failed to resolve conflict")?;
            println!("Resolved: {}", result.path);
            if result.merge_completed {
                println!("All conflicts resolved — merge complete.");
                // Regenerate INDEX.md after merge completion
                let mut repo = PlanRepo::new(store);
                repo.generate_index()
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
            let mut store = make_store(&root, staging)?;
            match command {
                RemoteCommand::Add { name, url } => {
                    store
                        .git_remote_add(&name, &url)
                        .context("failed to add remote")?;
                    println!("Added remote '{name}' ({url})");
                }
                RemoteCommand::Remove { name } => {
                    store
                        .git_remote_remove(&name)
                        .context("failed to remove remote")?;
                    println!("Removed remote '{name}'");
                }
                RemoteCommand::List => {
                    let remotes = store.git_remote_list().context("failed to list remotes")?;
                    if remotes.is_empty() {
                        println!("No remotes configured.");
                    } else {
                        for r in &remotes {
                            println!("{}\t{}", r.name, r.url);
                        }
                    }
                }
                RemoteCommand::Fetch { name } => {
                    let remote_name = resolve_remote_name(name, &root)?;
                    store
                        .git_fetch(&remote_name)
                        .context("failed to fetch from remote")?;
                    println!("Fetched from '{remote_name}'.");
                }
                RemoteCommand::Push { name, force } => {
                    let remote_name = resolve_remote_name(name, &root)?;
                    let result = store.git_push(&remote_name, force)?;
                    if result.commits_pushed == 0 {
                        println!("Already up to date.");
                    } else {
                        println!(
                            "Pushed {} commit(s) to {}/{}.",
                            result.commits_pushed, result.remote, result.branch
                        );
                    }
                }
                RemoteCommand::Pull { name } => {
                    let remote_name = resolve_remote_name(name, &root)?;
                    let outcome = store.git_pull(&remote_name)?;
                    match outcome {
                        rdm_store_git::PullOutcome::Success(result) => {
                            if !result.changed {
                                println!("Already up to date.");
                            } else {
                                println!(
                                    "Pulled {} commit(s) from {}/{}.",
                                    result.commits_merged, result.remote, result.branch
                                );
                                // Regenerate INDEX.md after pulling new content
                                let mut repo = PlanRepo::new(store);
                                repo.generate_index()
                                    .context("failed to regenerate INDEX.md after pull")?;
                            }
                        }
                        rdm_store_git::PullOutcome::Conflict(conflict) => {
                            eprintln!(
                                "Merge conflict: {} file(s) need resolution.",
                                conflict.conflicted_files.len()
                            );
                            eprintln!();
                            for item in &conflict.conflicted_files {
                                eprintln!("  conflict: {} — {item}", item.path);
                            }
                            eprintln!();
                            eprintln!(
                                "Edit the conflicted files, then run `rdm resolve <file>` for each."
                            );
                            eprintln!("Run `rdm conflicts` to see the full list.");
                            eprintln!(
                                "Run `rdm discard --force` to abort the merge and discard changes."
                            );
                            process::exit(1);
                        }
                    }
                }
            }
        }

        Command::List { project, all } => {
            let repo = PlanRepo::new(make_store(&root, staging)?);
            let projects = if all {
                repo.list_projects().context("failed to list projects")?
            } else {
                let p = resolve_project(project, &repo)?;
                vec![p]
            };

            // For JSON, collect all projects' summaries into one array.
            let mut all_summaries: Vec<json::RoadmapSummaryJson> = Vec::new();

            for project in &projects {
                if all && format != OutputFormat::Json {
                    println!("Project: {project}");
                }
                let roadmaps = repo
                    .list_roadmaps(project)
                    .context("failed to list roadmaps")?;
                let mut entries = Vec::new();
                for roadmap_doc in roadmaps {
                    let slug = &roadmap_doc.frontmatter.roadmap;
                    let phases = repo
                        .list_phases(project, slug)
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
            maybe_print_uncommitted_hint(repo.store(), staging);
        }
    }

    Ok(())
}
