use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Mutex;

use rdm_core::display;
use rdm_core::model::{PhaseStatus, Priority, TaskStatus};
use rdm_core::search::{self, ItemKind, ItemStatus, SearchFilter};
#[cfg(not(feature = "git"))]
use rdm_store_fs::FsStore;
use rmcp::handler::server::router::tool::ToolRouter;

use rmcp::handler::server::wrapper::Parameters;

/// Store backend selected by feature flags.
///
/// With the `git` feature enabled, operations go through [`rdm_store_git::GitStore`]
/// which auto-commits changes. Without it, plain filesystem I/O via [`FsStore`].
#[cfg(feature = "git")]
type AppStore = rdm_store_git::GitStore;
/// See the `git`-feature variant for documentation.
#[cfg(not(feature = "git"))]
type AppStore = FsStore;
use rmcp::model::Content;
use rmcp::{
    ErrorData, ServerHandler, ServiceExt,
    model::{CallToolResult, Implementation, ProtocolVersion, ServerCapabilities, ServerInfo},
    schemars, serde,
    transport::io::stdio,
};
use serde::Deserialize;

/// Converts an `rdm_core::error::Error` into an MCP `CallToolResult` with `is_error` set.
///
/// Detects `ConfigNotFound` and returns a message mentioning the `rdm_init` tool.
fn core_err(e: rdm_core::error::Error) -> Result<CallToolResult, ErrorData> {
    let msg = if matches!(e, rdm_core::error::Error::ConfigNotFound) {
        "Plan repo is not initialized. Call the rdm_init tool to set up your plan repo.".to_string()
    } else {
        e.to_string()
    };
    Ok(CallToolResult::error(vec![Content::text(msg)]))
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

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct InitParams {
    /// Optional default project to create during initialization.
    default_project: Option<String>,
}

// ---------- Store helpers ----------

/// Creates an [`AppStore`] for an existing plan repo.
fn make_store(root: &Path, staging: bool) -> anyhow::Result<AppStore> {
    #[cfg(feature = "git")]
    {
        Ok(rdm_store_git::GitStore::new(root)
            .map_err(|e| anyhow::anyhow!("failed to open git repository: {e}"))?
            .with_staging_mode(staging))
    }
    #[cfg(not(feature = "git"))]
    {
        let _ = staging;
        Ok(FsStore::new(root))
    }
}

/// Creates an [`AppStore`] for initializing a new plan repo.
fn make_init_store(root: &Path) -> anyhow::Result<AppStore> {
    #[cfg(feature = "git")]
    {
        rdm_store_git::GitStore::init(root)
            .map_err(|e| anyhow::anyhow!("failed to initialize git repository: {e}"))
    }
    #[cfg(not(feature = "git"))]
    {
        Ok(FsStore::new(root))
    }
}

// ---------- Server ----------

/// MCP server backed by an rdm plan repo.
struct RdmMcpServer {
    store: Mutex<AppStore>,
    plan_root: PathBuf,
    auto_init: bool,
    tool_router: ToolRouter<Self>,
}

impl RdmMcpServer {
    fn new(plan_root: PathBuf, auto_init: bool, staging: bool) -> anyhow::Result<Self> {
        // Try opening an existing repo. If it doesn't exist yet (common when the
        // MCP server starts before `rdm_init`), create the git repo so the server
        // can start — the plan-level initialisation happens later via the rdm_init
        // tool or maybe_auto_init.
        let store = match make_store(&plan_root, staging) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("failed to open existing store, falling back to init: {e}");
                let _ = std::fs::create_dir_all(&plan_root);
                make_init_store(&plan_root)?
            }
        };
        Ok(Self {
            store: Mutex::new(store),
            plan_root,
            auto_init,
            tool_router: Self::tool_router(),
        })
    }

    /// If `auto_init` is enabled and the repo is not yet initialized, initialize it with defaults.
    fn maybe_auto_init(&self) {
        if !self.auto_init {
            return;
        }
        let store = self.store.lock().unwrap();
        if rdm_core::io::load_config(&*store).is_ok() {
            return;
        }
        drop(store);

        // Create the directory if needed
        let _ = std::fs::create_dir_all(&self.plan_root);

        let new_store = match make_init_store(&self.plan_root) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("auto-init: failed to create store: {e}");
                return;
            }
        };
        *self.store.lock().unwrap() = new_store;
        let mut store = self.store.lock().unwrap();
        match rdm_core::ops::init::init_with_config(
            &mut *store,
            rdm_core::config::Config::default(),
        ) {
            Ok(()) => {}
            Err(rdm_core::error::Error::AlreadyInitialized) => {
                // Race condition or stale check — fine, just reload
            }
            Err(e) => {
                tracing::warn!("auto-init: failed to initialize config: {e}");
            }
        }
    }
}

#[rmcp::tool_router]
impl RdmMcpServer {
    // ==================== Init tool ====================

    /// Initialize the plan repo.
    #[rmcp::tool(
        description = "Initialize the plan repo. Call this before using any other tools if the repo is not yet set up.",
        annotations(read_only_hint = false)
    )]
    async fn rdm_init(
        &self,
        Parameters(params): Parameters<InitParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let _ = std::fs::create_dir_all(&self.plan_root);

        let new_store = match make_init_store(&self.plan_root) {
            Ok(s) => s,
            Err(e) => return err_text(format!("{e}")),
        };
        *self.store.lock().unwrap() = new_store;
        let config = if let Some(ref proj) = params.default_project {
            rdm_core::config::Config {
                default_project: Some(proj.clone()),
                ..Default::default()
            }
        } else {
            rdm_core::config::Config::default()
        };

        let mut store = self.store.lock().unwrap();
        if let Err(e) = rdm_core::ops::init::init_with_config(&mut *store, config) {
            return core_err(e);
        }

        if let Some(ref proj) = params.default_project
            && let Err(e) = rdm_core::ops::project::create_project(&mut *store, proj, proj)
        {
            return core_err(e);
        }

        let mut summary = format!("Plan repo initialized at {}", self.plan_root.display());
        if let Some(ref proj) = params.default_project {
            summary.push_str(&format!("\nDefault project: {proj}"));
        }
        ok_text(summary)
    }

    // ==================== Read-only tools ====================

    /// List all projects in the plan repo.
    #[rmcp::tool(
        description = "List all projects in the plan repo",
        annotations(read_only_hint = true)
    )]
    async fn rdm_project_list(&self) -> Result<CallToolResult, ErrorData> {
        self.maybe_auto_init();
        let store = self.store.lock().unwrap();
        match rdm_core::ops::project::list_projects(&*store) {
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
        self.maybe_auto_init();
        let store = self.store.lock().unwrap();
        let roadmaps = match rdm_core::ops::roadmap::list_roadmaps(&*store, &params.project) {
            Ok(r) => r,
            Err(e) => return core_err(e),
        };

        let mut entries = Vec::new();
        for roadmap_doc in roadmaps {
            let slug = &roadmap_doc.frontmatter.roadmap;
            let phases = match rdm_core::ops::phase::list_phases(&*store, &params.project, slug) {
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
        self.maybe_auto_init();
        let store = self.store.lock().unwrap();
        let doc = match rdm_core::io::load_roadmap(&*store, &params.project, &params.roadmap) {
            Ok(d) => d,
            Err(e) => return core_err(e),
        };
        let phases =
            match rdm_core::ops::phase::list_phases(&*store, &params.project, &params.roadmap) {
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
        self.maybe_auto_init();
        let store = self.store.lock().unwrap();
        match rdm_core::ops::phase::list_phases(&*store, &params.project, &params.roadmap) {
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
        self.maybe_auto_init();
        let store = self.store.lock().unwrap();
        let stem = match rdm_core::ops::phase::resolve_phase_stem(
            &*store,
            &params.project,
            &params.roadmap,
            &params.phase,
        ) {
            Ok(s) => s,
            Err(e) => return core_err(e),
        };
        let doc = match rdm_core::io::load_phase(&*store, &params.project, &params.roadmap, &stem) {
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
        self.maybe_auto_init();
        let store = self.store.lock().unwrap();
        let all_tasks = match rdm_core::ops::task::list_tasks(&*store, &params.project) {
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
        self.maybe_auto_init();
        let store = self.store.lock().unwrap();
        match rdm_core::io::load_task(&*store, &params.project, &params.task) {
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
        self.maybe_auto_init();
        let store = self.store.lock().unwrap();
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

        match search::search(&*store, &params.query, &filter) {
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
        self.maybe_auto_init();
        let mut store = self.store.lock().unwrap();
        let doc = match rdm_core::ops::roadmap::create_roadmap(
            &mut *store,
            &params.project,
            &params.slug,
            &params.title,
            params.body.as_deref(),
        ) {
            Ok(d) => d,
            Err(e) => return core_err(e),
        };
        if let Err(e) = rdm_core::ops::index::generate_index(&mut *store) {
            return core_err(e);
        }
        let phases = match rdm_core::ops::phase::list_phases(&*store, &params.project, &params.slug)
        {
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
        self.maybe_auto_init();
        let mut store = self.store.lock().unwrap();
        let doc = match rdm_core::ops::phase::create_phase(
            &mut *store,
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
        if let Err(e) = rdm_core::ops::index::generate_index(&mut *store) {
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
        self.maybe_auto_init();
        let mut store = self.store.lock().unwrap();
        let stem = match rdm_core::ops::phase::resolve_phase_stem(
            &*store,
            &params.project,
            &params.roadmap,
            &params.phase,
        ) {
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

        let doc = match rdm_core::ops::phase::update_phase(
            &mut *store,
            &params.project,
            &params.roadmap,
            &stem,
            status,
            params.body.as_deref(),
            None,
        ) {
            Ok(d) => d,
            Err(e) => return core_err(e),
        };
        if let Err(e) = rdm_core::ops::index::generate_index(&mut *store) {
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
        self.maybe_auto_init();
        let priority = match &params.priority {
            Some(p) => match Priority::from_str(p) {
                Ok(pr) => pr,
                Err(msg) => return err_text(msg),
            },
            None => Priority::Medium,
        };

        let mut store = self.store.lock().unwrap();
        let doc = match rdm_core::ops::task::create_task(
            &mut *store,
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
        if let Err(e) = rdm_core::ops::index::generate_index(&mut *store) {
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
        self.maybe_auto_init();
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

        let mut store = self.store.lock().unwrap();
        let doc = match rdm_core::ops::task::update_task(
            &mut *store,
            &params.project,
            &params.task,
            status,
            priority,
            params.tags,
            params.body.as_deref(),
            None,
        ) {
            Ok(d) => d,
            Err(e) => return core_err(e),
        };
        if let Err(e) = rdm_core::ops::index::generate_index(&mut *store) {
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
        self.maybe_auto_init();
        let mut store = self.store.lock().unwrap();
        let doc = match rdm_core::ops::task::promote_task(
            &mut *store,
            &params.project,
            &params.task,
            &params.roadmap_slug,
        ) {
            Ok(d) => d,
            Err(e) => return core_err(e),
        };
        if let Err(e) = rdm_core::ops::index::generate_index(&mut *store) {
            return core_err(e);
        }
        let phases =
            match rdm_core::ops::phase::list_phases(&*store, &params.project, &params.roadmap_slug)
            {
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
pub async fn run(plan_root: PathBuf, auto_init: bool, staging: bool) -> anyhow::Result<()> {
    let server = RdmMcpServer::new(plan_root, auto_init, staging)?;
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
