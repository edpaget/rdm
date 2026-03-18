use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use is_terminal::IsTerminal;
use rdm_core::agent_config::{self, AgentConfigOptions, Platform, SkillOptions};
use rdm_core::display;
use rdm_core::model::{PhaseStatus, Priority, TaskStatus, TaskStatusFilter};
use rdm_core::repo::PlanRepo;
use rdm_core::search::{self, ItemKind, ItemStatus, SearchFilter};

#[derive(Parser)]
#[command(name = "rdm", about = "Manage project roadmaps, phases, and tasks")]
struct Cli {
    /// Path to the plan repo root.
    #[arg(long, env = "RDM_ROOT")]
    root: Option<PathBuf>,

    /// Suppress automatic INDEX.md regeneration after mutations.
    #[arg(long, global = true)]
    no_index: bool,

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
        #[arg(long)]
        skills: bool,
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
        /// Output format.
        #[arg(long, default_value = "text")]
        format: OutputFormat,
    },
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

/// Output format for search results.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum OutputFormat {
    Text,
    Json,
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

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err:#}");
        process::exit(1);
    }
}

/// Resolves project: --project flag > `RDM_PROJECT` env var > config default_project > error.
fn resolve_project(flag: Option<String>, repo: &PlanRepo) -> Result<String> {
    if let Some(p) = flag {
        return Ok(p);
    }
    if let Ok(p) = std::env::var("RDM_PROJECT") {
        return Ok(p);
    }
    if let Ok(config) = repo.load_config() {
        if let Some(p) = config.default_project {
            return Ok(p);
        }
    }
    bail!(
        "no project specified — use --project, set RDM_PROJECT, or set default_project in rdm.toml"
    )
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

fn maybe_regenerate_index(repo: &PlanRepo, no_index: bool) -> Result<()> {
    if !no_index {
        repo.generate_index()
            .context("failed to regenerate INDEX.md")?;
    }
    Ok(())
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let root = cli
        .root
        .unwrap_or_else(|| std::env::current_dir().expect("cannot determine current directory"));

    match cli.command {
        Command::Init => {
            PlanRepo::init(&root).context("failed to initialize plan repo")?;
            println!("Initialized plan repo at {}", root.display());
        }

        Command::Index => {
            let repo = PlanRepo::open(&root);
            repo.generate_index().context("failed to generate index")?;
            println!("Generated INDEX.md");
        }

        Command::Project { command } => {
            let repo = PlanRepo::open(&root);
            match command {
                ProjectCommand::Create { name, title } => {
                    let title = title.as_deref().unwrap_or(&name);
                    let doc = repo
                        .create_project(&name, title)
                        .context("failed to create project")?;
                    println!("Created project '{}'", doc.frontmatter.name);
                    maybe_regenerate_index(&repo, cli.no_index)?;
                }
                ProjectCommand::List => {
                    let projects = repo.list_projects().context("failed to list projects")?;
                    if projects.is_empty() {
                        println!("No projects yet.");
                    } else {
                        for p in &projects {
                            println!("{p}");
                        }
                    }
                }
            }
        }

        Command::Roadmap { command } => {
            let repo = PlanRepo::open(&root);
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
                    maybe_regenerate_index(&repo, cli.no_index)?;
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
                    print!("{}", display::format_roadmap_summary(&roadmap_doc, &phases));
                }
                RoadmapCommand::List { project } => {
                    let project = resolve_project(project, &repo)?;
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
                    print!("{}", display::format_roadmap_list(&entries));
                }
            }
        }

        Command::Phase { command } => {
            let repo = PlanRepo::open(&root);
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
                    let stem = format!("phase-{}-{slug}", doc.frontmatter.phase);
                    println!("Created phase '{stem}' in roadmap '{roadmap}'");
                    maybe_regenerate_index(&repo, cli.no_index)?;
                }
                PhaseCommand::List { roadmap, project } => {
                    let project = resolve_project(project, &repo)?;
                    let phases = repo
                        .list_phases(&project, &roadmap)
                        .context("failed to list phases")?;
                    print!("{}", display::format_phase_list(&phases));
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
                    print!("{}", display::format_phase_detail(&stem, &doc));
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
                    maybe_regenerate_index(&repo, cli.no_index)?;
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
                    maybe_regenerate_index(&repo, cli.no_index)?;
                }
            }
        }

        Command::Task { command } => {
            let repo = PlanRepo::open(&root);
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
                    maybe_regenerate_index(&repo, cli.no_index)?;
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
                    print!("{}", display::format_task_detail(&slug, &doc));
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
                    maybe_regenerate_index(&repo, cli.no_index)?;
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

                    print!("{}", display::format_task_list(&filtered));
                }
            }
        }

        Command::Promote {
            task_slug,
            roadmap_slug,
            project,
        } => {
            let repo = PlanRepo::open(&root);
            let project = resolve_project(project, &repo)?;
            let doc = repo
                .promote_task(&project, &task_slug, &roadmap_slug)
                .context("failed to promote task")?;
            println!(
                "Promoted task '{task_slug}' → roadmap '{}'",
                doc.frontmatter.roadmap
            );
            maybe_regenerate_index(&repo, cli.no_index)?;
        }

        Command::AgentConfig {
            platform,
            project,
            out,
            principles_file,
            skills,
        } => {
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
            format,
        } => {
            let repo = PlanRepo::open(&root);
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
                OutputFormat::Text => {
                    if results.is_empty() {
                        println!("No results found for '{query}'.");
                    } else {
                        print!("{}", display::format_search_results(&results));
                    }
                }
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&results)
                            .context("failed to serialize results")?
                    );
                }
            }
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

        Command::List { project, all } => {
            let repo = PlanRepo::open(&root);
            let projects = if all {
                repo.list_projects().context("failed to list projects")?
            } else {
                let p = resolve_project(project, &repo)?;
                vec![p]
            };

            for project in &projects {
                if all {
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
                print!("{}", display::format_roadmap_list(&entries));
            }
        }
    }

    Ok(())
}
