use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Mutex;

use rdm_core::display;
use rdm_core::model::{PhaseStatus, Priority, TaskStatus};
use rdm_core::repo::PlanRepo;
use rdm_core::search::{self, ItemKind, ItemStatus, SearchFilter};
use rdm_store_fs::FsStore;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::Content;
use rmcp::{
    ErrorData, ServerHandler, ServiceExt,
    model::{CallToolResult, Implementation, ProtocolVersion, ServerCapabilities, ServerInfo},
    schemars, serde,
    transport::io::stdio,
};
use serde::Deserialize;

/// Converts an `rdm_core::error::Error` into an MCP `CallToolResult` with `is_error` set.
fn core_err(e: rdm_core::error::Error) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::error(vec![Content::text(e.to_string())]))
}

/// Returns a successful `CallToolResult` containing text.
fn ok_text(text: String) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::success(vec![Content::text(text)]))
}

/// Returns an error `CallToolResult` containing a message.
fn err_text(msg: String) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::error(vec![Content::text(msg)]))
}

// ---------- Parameter structs (read-only) ----------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ProjectParams {
    /// The project name.
    project: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RoadmapParams {
    /// The project name.
    project: String,
    /// The roadmap slug.
    roadmap: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PhaseParams {
    /// The project name.
    project: String,
    /// The roadmap slug.
    roadmap: String,
    /// The phase stem or number.
    phase: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TaskShowParams {
    /// The project name.
    project: String,
    /// The task slug.
    task: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TaskListParams {
    /// The project name.
    project: String,
    /// Filter by status (e.g. "open", "in-progress", "done", "wont-fix", or "all"). Omit for default (open + in-progress).
    status: Option<String>,
    /// Filter by priority (e.g. "low", "medium", "high", "critical").
    priority: Option<String>,
    /// Filter by tag.
    tag: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SearchParams {
    /// The search query string.
    query: String,
    /// Restrict to a specific project.
    project: Option<String>,
    /// Restrict to a specific item kind: "roadmap", "phase", or "task".
    kind: Option<String>,
    /// Filter by status (e.g. "open", "in-progress", "done").
    status: Option<String>,
    /// Maximum number of results to return (default 20).
    limit: Option<usize>,
}

// ---------- Parameter structs (mutation) ----------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RoadmapCreateParams {
    /// The project name.
    project: String,
    /// The roadmap slug (URL-friendly identifier).
    slug: String,
    /// The roadmap title.
    title: String,
    /// Optional body content (Markdown).
    body: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PhaseCreateParams {
    /// The project name.
    project: String,
    /// The roadmap slug to add the phase to.
    roadmap: String,
    /// The phase slug (URL-friendly identifier).
    slug: String,
    /// The phase title.
    title: String,
    /// Optional phase number. If omitted, auto-assigns the next available number.
    number: Option<u32>,
    /// Optional body content (Markdown).
    body: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PhaseUpdateParams {
    /// The project name.
    project: String,
    /// The roadmap slug.
    roadmap: String,
    /// The phase stem or number.
    phase: String,
    /// New status: "not-started", "in-progress", "done", or "blocked".
    status: Option<String>,
    /// New body content (Markdown). Replaces the existing body.
    body: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TaskCreateParams {
    /// The project name.
    project: String,
    /// The task slug (URL-friendly identifier).
    slug: String,
    /// The task title.
    title: String,
    /// Priority: "low", "medium", "high", or "critical". Defaults to "medium".
    priority: Option<String>,
    /// Optional tags for categorization.
    tags: Option<Vec<String>>,
    /// Optional body content (Markdown).
    body: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TaskUpdateParams {
    /// The project name.
    project: String,
    /// The task slug.
    task: String,
    /// New status: "open", "in-progress", "done", or "wont-fix".
    status: Option<String>,
    /// New priority: "low", "medium", "high", or "critical".
    priority: Option<String>,
    /// New tags (replaces existing tags).
    tags: Option<Vec<String>>,
    /// New body content (Markdown). Replaces the existing body.
    body: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TaskPromoteParams {
    /// The project name.
    project: String,
    /// The task slug to promote.
    task: String,
    /// The slug for the new roadmap.
    roadmap_slug: String,
}

// ---------- Server ----------

/// MCP server backed by an rdm plan repo.
struct RdmMcpServer {
    repo: Mutex<PlanRepo<FsStore>>,
    tool_router: ToolRouter<Self>,
}

impl RdmMcpServer {
    fn new(plan_root: PathBuf) -> Self {
        Self {
            repo: Mutex::new(PlanRepo::new(FsStore::new(plan_root))),
            tool_router: Self::tool_router(),
        }
    }
}

#[rmcp::tool_router]
impl RdmMcpServer {
    // ==================== Read-only tools ====================

    /// List all projects in the plan repo.
    #[rmcp::tool(
        description = "List all projects in the plan repo",
        annotations(read_only_hint = true)
    )]
    async fn rdm_project_list(&self) -> Result<CallToolResult, ErrorData> {
        let repo = self.repo.lock().unwrap();
        match repo.list_projects() {
            Ok(projects) => ok_text(projects.join("\n")),
            Err(e) => core_err(e),
        }
    }

    /// List all roadmaps in a project with phase progress.
    #[rmcp::tool(
        description = "List all roadmaps in a project with phase progress",
        annotations(read_only_hint = true)
    )]
    async fn rdm_roadmap_list(
        &self,
        Parameters(params): Parameters<ProjectParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let repo = self.repo.lock().unwrap();
        let roadmaps = match repo.list_roadmaps(&params.project) {
            Ok(r) => r,
            Err(e) => return core_err(e),
        };

        let mut entries = Vec::new();
        for roadmap_doc in roadmaps {
            let slug = &roadmap_doc.frontmatter.roadmap;
            let phases = match repo.list_phases(&params.project, slug) {
                Ok(p) => p,
                Err(e) => return core_err(e),
            };
            entries.push((roadmap_doc, phases));
        }

        ok_text(display::format_roadmap_list(&entries))
    }

    /// Show details of a specific roadmap including its phases.
    #[rmcp::tool(
        description = "Show details of a specific roadmap including its phases",
        annotations(read_only_hint = true)
    )]
    async fn rdm_roadmap_show(
        &self,
        Parameters(params): Parameters<RoadmapParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let repo = self.repo.lock().unwrap();
        let doc = match repo.load_roadmap(&params.project, &params.roadmap) {
            Ok(d) => d,
            Err(e) => return core_err(e),
        };
        let phases = match repo.list_phases(&params.project, &params.roadmap) {
            Ok(p) => p,
            Err(e) => return core_err(e),
        };

        ok_text(display::format_roadmap_summary(&doc, &phases))
    }

    /// List all phases in a roadmap.
    #[rmcp::tool(
        description = "List all phases in a roadmap",
        annotations(read_only_hint = true)
    )]
    async fn rdm_phase_list(
        &self,
        Parameters(params): Parameters<RoadmapParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let repo = self.repo.lock().unwrap();
        match repo.list_phases(&params.project, &params.roadmap) {
            Ok(phases) => ok_text(display::format_phase_list(&phases)),
            Err(e) => core_err(e),
        }
    }

    /// Show details of a specific phase.
    #[rmcp::tool(
        description = "Show details of a specific phase in a roadmap",
        annotations(read_only_hint = true)
    )]
    async fn rdm_phase_show(
        &self,
        Parameters(params): Parameters<PhaseParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let repo = self.repo.lock().unwrap();
        let stem = match repo.resolve_phase_stem(&params.project, &params.roadmap, &params.phase) {
            Ok(s) => s,
            Err(e) => return core_err(e),
        };
        let doc = match repo.load_phase(&params.project, &params.roadmap, &stem) {
            Ok(d) => d,
            Err(e) => return core_err(e),
        };

        ok_text(display::format_phase_detail(&stem, &doc, None))
    }

    /// List tasks in a project with optional filters.
    #[rmcp::tool(
        description = "List tasks in a project, optionally filtered by status, priority, or tag",
        annotations(read_only_hint = true)
    )]
    async fn rdm_task_list(
        &self,
        Parameters(params): Parameters<TaskListParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let repo = self.repo.lock().unwrap();
        let all_tasks = match repo.list_tasks(&params.project) {
            Ok(t) => t,
            Err(e) => return core_err(e),
        };

        let filtered: Vec<_> = all_tasks
            .into_iter()
            .filter(|(_slug, doc)| {
                let status_ok = match &params.status {
                    Some(s) if s == "all" => true,
                    Some(s) => doc.frontmatter.status.to_string() == *s,
                    None => matches!(
                        doc.frontmatter.status,
                        TaskStatus::Open | TaskStatus::InProgress
                    ),
                };
                let priority_ok = match &params.priority {
                    Some(p) => doc.frontmatter.priority.to_string() == *p,
                    None => true,
                };
                let tag_ok = match &params.tag {
                    Some(tag) => doc
                        .frontmatter
                        .tags
                        .as_ref()
                        .is_some_and(|tags| tags.contains(tag)),
                    None => true,
                };
                status_ok && priority_ok && tag_ok
            })
            .collect();

        ok_text(display::format_task_list(&filtered))
    }

    /// Show details of a specific task.
    #[rmcp::tool(
        description = "Show details of a specific task",
        annotations(read_only_hint = true)
    )]
    async fn rdm_task_show(
        &self,
        Parameters(params): Parameters<TaskShowParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let repo = self.repo.lock().unwrap();
        match repo.load_task(&params.project, &params.task) {
            Ok(doc) => ok_text(display::format_task_detail(&params.task, &doc)),
            Err(e) => core_err(e),
        }
    }

    /// Search for items across the plan repo.
    #[rmcp::tool(
        description = "Search for items across the plan repo by fuzzy-matching titles and body content",
        annotations(read_only_hint = true)
    )]
    async fn rdm_search(
        &self,
        Parameters(params): Parameters<SearchParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let repo = self.repo.lock().unwrap();
        let kind = match &params.kind {
            Some(k) => match k.as_str() {
                "roadmap" => Some(ItemKind::Roadmap),
                "phase" => Some(ItemKind::Phase),
                "task" => Some(ItemKind::Task),
                other => {
                    return err_text(format!(
                        "Invalid kind: {other}. Expected: roadmap, phase, or task"
                    ));
                }
            },
            None => None,
        };

        let status = match &params.status {
            Some(s) => match parse_item_status(s) {
                Ok(st) => Some(st),
                Err(msg) => return err_text(msg),
            },
            None => None,
        };

        let filter = SearchFilter {
            kind,
            project: params.project,
            status,
        };

        match search::search(&repo, &params.query, &filter) {
            Ok(mut results) => {
                let limit = params.limit.unwrap_or(20);
                results.truncate(limit);
                ok_text(display::format_search_results(&results))
            }
            Err(e) => core_err(e),
        }
    }

    // ==================== Mutation tools ====================

    /// Create a new roadmap in a project.
    #[rmcp::tool(
        description = "Create a new roadmap in a project",
        annotations(read_only_hint = false)
    )]
    async fn rdm_roadmap_create(
        &self,
        Parameters(params): Parameters<RoadmapCreateParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let mut repo = self.repo.lock().unwrap();
        let doc = match repo.create_roadmap(
            &params.project,
            &params.slug,
            &params.title,
            params.body.as_deref(),
        ) {
            Ok(d) => d,
            Err(e) => return core_err(e),
        };
        if let Err(e) = repo.generate_index() {
            return core_err(e);
        }
        let phases = match repo.list_phases(&params.project, &params.slug) {
            Ok(p) => p,
            Err(e) => return core_err(e),
        };
        ok_text(display::format_roadmap_summary(&doc, &phases))
    }

    /// Create a new phase in a roadmap.
    #[rmcp::tool(
        description = "Create a new phase in a roadmap",
        annotations(read_only_hint = false)
    )]
    async fn rdm_phase_create(
        &self,
        Parameters(params): Parameters<PhaseCreateParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let mut repo = self.repo.lock().unwrap();
        let doc = match repo.create_phase(
            &params.project,
            &params.roadmap,
            &params.slug,
            &params.title,
            params.number,
            params.body.as_deref(),
        ) {
            Ok(d) => d,
            Err(e) => return core_err(e),
        };
        if let Err(e) = repo.generate_index() {
            return core_err(e);
        }
        let stem = doc.frontmatter.stem(&params.slug);
        ok_text(display::format_phase_detail(&stem, &doc, None))
    }

    /// Update a phase's status or body.
    #[rmcp::tool(
        description = "Update a phase's status or body content",
        annotations(read_only_hint = false)
    )]
    async fn rdm_phase_update(
        &self,
        Parameters(params): Parameters<PhaseUpdateParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let mut repo = self.repo.lock().unwrap();
        let stem = match repo.resolve_phase_stem(&params.project, &params.roadmap, &params.phase) {
            Ok(s) => s,
            Err(e) => return core_err(e),
        };

        let status = match &params.status {
            Some(s) => match PhaseStatus::from_str(s) {
                Ok(st) => Some(st),
                Err(msg) => return err_text(msg),
            },
            None => None,
        };

        let doc = match repo.update_phase(
            &params.project,
            &params.roadmap,
            &stem,
            status,
            params.body.as_deref(),
        ) {
            Ok(d) => d,
            Err(e) => return core_err(e),
        };
        if let Err(e) = repo.generate_index() {
            return core_err(e);
        }
        ok_text(display::format_phase_detail(&stem, &doc, None))
    }

    /// Create a new task in a project.
    #[rmcp::tool(
        description = "Create a new task in a project",
        annotations(read_only_hint = false)
    )]
    async fn rdm_task_create(
        &self,
        Parameters(params): Parameters<TaskCreateParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let priority = match &params.priority {
            Some(p) => match Priority::from_str(p) {
                Ok(pr) => pr,
                Err(msg) => return err_text(msg),
            },
            None => Priority::Medium,
        };

        let mut repo = self.repo.lock().unwrap();
        let doc = match repo.create_task(
            &params.project,
            &params.slug,
            &params.title,
            priority,
            params.tags,
            params.body.as_deref(),
        ) {
            Ok(d) => d,
            Err(e) => return core_err(e),
        };
        if let Err(e) = repo.generate_index() {
            return core_err(e);
        }
        ok_text(display::format_task_detail(&params.slug, &doc))
    }

    /// Update a task's status, priority, tags, or body.
    #[rmcp::tool(
        description = "Update a task's status, priority, tags, or body content",
        annotations(read_only_hint = false)
    )]
    async fn rdm_task_update(
        &self,
        Parameters(params): Parameters<TaskUpdateParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let status = match &params.status {
            Some(s) => match TaskStatus::from_str(s) {
                Ok(st) => Some(st),
                Err(msg) => return err_text(msg),
            },
            None => None,
        };

        let priority = match &params.priority {
            Some(p) => match Priority::from_str(p) {
                Ok(pr) => Some(pr),
                Err(msg) => return err_text(msg),
            },
            None => None,
        };

        let mut repo = self.repo.lock().unwrap();
        let doc = match repo.update_task(
            &params.project,
            &params.task,
            status,
            priority,
            params.tags,
            params.body.as_deref(),
        ) {
            Ok(d) => d,
            Err(e) => return core_err(e),
        };
        if let Err(e) = repo.generate_index() {
            return core_err(e);
        }
        ok_text(display::format_task_detail(&params.task, &doc))
    }

    /// Promote a task to a roadmap.
    #[rmcp::tool(
        description = "Promote a task to a roadmap with an initial phase",
        annotations(read_only_hint = false)
    )]
    async fn rdm_task_promote(
        &self,
        Parameters(params): Parameters<TaskPromoteParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let mut repo = self.repo.lock().unwrap();
        let doc = match repo.promote_task(&params.project, &params.task, &params.roadmap_slug) {
            Ok(d) => d,
            Err(e) => return core_err(e),
        };
        if let Err(e) = repo.generate_index() {
            return core_err(e);
        }
        let phases = match repo.list_phases(&params.project, &params.roadmap_slug) {
            Ok(p) => p,
            Err(e) => return core_err(e),
        };
        ok_text(display::format_roadmap_summary(&doc, &phases))
    }
}

/// Parse a status string into an `ItemStatus`.
fn parse_item_status(s: &str) -> Result<ItemStatus, String> {
    if let Ok(ps) = PhaseStatus::from_str(s) {
        return Ok(ItemStatus::Phase(ps));
    }
    if let Ok(ts) = TaskStatus::from_str(s) {
        return Ok(ItemStatus::Task(ts));
    }
    Err(format!(
        "Invalid status: {s}. Expected a phase status (not-started, in-progress, done, blocked) or task status (open, in-progress, done, wont-fix)"
    ))
}

#[rmcp::tool_handler]
impl ServerHandler for RdmMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::default(),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "rdm-mcp".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                ..Default::default()
            },
            instructions: Some("MCP server for managing rdm plan repos.".into()),
        }
    }
}

/// Start the MCP server on stdin/stdout.
///
/// # Errors
///
/// Returns an error if the transport fails to initialize or the server
/// encounters a fatal I/O error.
pub async fn run(plan_root: PathBuf) -> anyhow::Result<()> {
    let server = RdmMcpServer::new(plan_root);
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
