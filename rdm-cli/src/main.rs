use std::path::PathBuf;
use std::process;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use rdm_core::display;
use rdm_core::model::PhaseStatus;
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
    /// Show a phase.
    Show {
        /// Phase stem (e.g. phase-1-core).
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
        /// Phase stem (e.g. phase-1-core).
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
                    repo.create_project(&name, title)
                        .context("failed to create project")?;
                    println!("Created project '{name}'");
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
                PhaseCommand::Show {
                    stem,
                    roadmap,
                    project,
                } => {
                    let project = resolve_project(project, &repo)?;
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
                    repo.update_phase(&project, &roadmap, &stem, status)
                        .context("failed to update phase")?;
                    println!("Updated '{stem}' → {status}");
                }
            }
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
                    let phases = repo
                        .list_phases(project, &roadmap_doc.frontmatter.roadmap)
                        .unwrap_or_default();
                    entries.push((roadmap_doc, phases));
                }
                print!("{}", display::format_roadmap_list(&entries));
            }
        }
    }

    Ok(())
}
