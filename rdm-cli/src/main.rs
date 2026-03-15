use std::path::PathBuf;
use std::process;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use rdm_core::display;
use rdm_core::model::{PhaseStatus, Priority, TaskStatus};
use rdm_core::repo::PlanRepo;

#[derive(Parser)]
#[command(name = "rdm", about = "Manage project roadmaps, phases, and tasks")]
struct Cli {
    /// Path to the plan repo root.
    #[arg(long, env = "RDM_ROOT")]
    root: Option<PathBuf>,

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
    },
    /// Show a roadmap and its phases.
    Show {
        /// Roadmap slug.
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
    },
    /// Update a phase's status.
    Update {
        /// Phase stem or number (e.g. phase-1-core or 1).
        stem: String,
        /// New status.
        #[arg(long)]
        status: PhaseStatus,
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
    },
    /// Show a task.
    Show {
        /// Task slug.
        slug: String,
        /// Project the task belongs to.
        #[arg(long)]
        project: Option<String>,
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
    },
    /// List tasks.
    List {
        /// Project to list tasks for.
        #[arg(long)]
        project: Option<String>,
        /// Filter by status (use "all" to show all).
        #[arg(long)]
        status: Option<String>,
        /// Filter by priority.
        #[arg(long)]
        priority: Option<Priority>,
        /// Filter by tag.
        #[arg(long)]
        tag: Option<String>,
    },
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err:#}");
        process::exit(1);
    }
}

/// Resolves project: --project flag > config default_project > error.
fn resolve_project(flag: Option<String>, repo: &PlanRepo) -> Result<String> {
    if let Some(p) = flag {
        return Ok(p);
    }
    if let Ok(config) = repo.load_config() {
        if let Some(p) = config.default_project {
            return Ok(p);
        }
    }
    bail!("no project specified — use --project or set default_project in rdm.toml")
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

        Command::Project { command } => {
            let repo = PlanRepo::open(&root);
            match command {
                ProjectCommand::Create { name, title } => {
                    let title = title.as_deref().unwrap_or(&name);
                    let doc = repo
                        .create_project(&name, title)
                        .context("failed to create project")?;
                    println!("Created project '{}'", doc.frontmatter.name);
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
                } => {
                    let project = resolve_project(project, &repo)?;
                    let title = title.as_deref().unwrap_or(&slug);
                    repo.create_roadmap(&project, &slug, title)
                        .context("failed to create roadmap")?;
                    println!("Created roadmap '{slug}' in project '{project}'");
                }
                RoadmapCommand::Show { slug, project } => {
                    let project = resolve_project(project, &repo)?;
                    let roadmap_doc = repo
                        .load_roadmap(&project, &slug)
                        .context("failed to load roadmap")?;
                    let phases = repo
                        .list_phases(&project, &slug)
                        .context("failed to list phases")?;
                    print!(
                        "{}",
                        display::format_roadmap_summary(&roadmap_doc.frontmatter, &phases)
                    );
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
                } => {
                    let project = resolve_project(project, &repo)?;
                    let title = title.as_deref().unwrap_or(&slug);
                    let doc = repo
                        .create_phase(&project, &roadmap, &slug, title, number)
                        .context("failed to create phase")?;
                    let stem = format!("phase-{}-{slug}", doc.frontmatter.phase);
                    println!("Created phase '{stem}' in roadmap '{roadmap}'");
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
                } => {
                    let project = resolve_project(project, &repo)?;
                    let stem = repo
                        .resolve_phase_stem(&project, &roadmap, &stem)
                        .context("failed to resolve phase")?;
                    let doc = repo
                        .load_phase(&project, &roadmap, &stem)
                        .context("failed to load phase")?;
                    print!("{}", display::format_phase_detail(&stem, &doc));
                }
                PhaseCommand::Update {
                    stem,
                    status,
                    roadmap,
                    project,
                } => {
                    let project = resolve_project(project, &repo)?;
                    let stem = repo
                        .resolve_phase_stem(&project, &roadmap, &stem)
                        .context("failed to resolve phase")?;
                    repo.update_phase(&project, &roadmap, &stem, status)
                        .context("failed to update phase")?;
                    println!("Updated '{stem}' → {status}");
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
                } => {
                    let project = resolve_project(project, &repo)?;
                    let title = title.as_deref().unwrap_or(&slug);
                    repo.create_task(&project, &slug, title, priority, tags)
                        .context("failed to create task")?;
                    println!("Created task '{slug}' in project '{project}'");
                }
                TaskCommand::Show { slug, project } => {
                    let project = resolve_project(project, &repo)?;
                    let doc = repo
                        .load_task(&project, &slug)
                        .context("failed to load task")?;
                    print!("{}", display::format_task_detail(&slug, &doc));
                }
                TaskCommand::Update {
                    slug,
                    project,
                    status,
                    priority,
                    tags,
                } => {
                    let project = resolve_project(project, &repo)?;
                    let doc = repo
                        .update_task(&project, &slug, status, priority, tags)
                        .context("failed to update task")?;
                    println!(
                        "Updated task '{slug}' → status: {}, priority: {}",
                        doc.frontmatter.status, doc.frontmatter.priority
                    );
                }
                TaskCommand::List {
                    project,
                    status,
                    priority,
                    tag,
                } => {
                    let project = resolve_project(project, &repo)?;
                    let all_tasks = repo.list_tasks(&project).context("failed to list tasks")?;

                    let show_all = status.as_deref() == Some("all");
                    let status_filter: Option<TaskStatus> = if show_all {
                        None
                    } else {
                        status
                            .as_deref()
                            .map(|s| s.parse::<TaskStatus>())
                            .transpose()
                            .map_err(|e| anyhow::anyhow!(e))?
                    };

                    let filtered: Vec<(String, _)> = all_tasks
                        .into_iter()
                        .filter(|(_, doc)| {
                            if let Some(ref s) = status_filter {
                                doc.frontmatter.status == *s
                            } else if !show_all {
                                // Default: show open + in-progress
                                doc.frontmatter.status == TaskStatus::Open
                                    || doc.frontmatter.status == TaskStatus::InProgress
                            } else {
                                true
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
