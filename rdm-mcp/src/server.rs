use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Mutex;

use rdm_core::display;
use rdm_core::model::{
    PhaseStatus, Priority, ReviewCommentStatus, ReviewTarget, ReviewTargetKind, RoadmapSort,
    TaskStatus,
};
use rdm_core::ops::{BodyUpdate, PriorityUpdate, ReasonUpdate, TagsUpdate};
use rdm_core::search::{self, ItemKind, ItemStatus, SearchFilter};
use rdm_core::store::{Store, VersionedStore};
#[cfg(not(feature = "git"))]
use rdm_store_fs::FsStore;
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
use rmcp::model::ContentBlock;
use rmcp::{
    ErrorData, ServerHandler, ServiceExt,
    model::{CallToolResult, Implementation, ServerCapabilities, ServerInfo},
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
    Ok(CallToolResult::error(vec![ContentBlock::text(msg)]))
}

/// Returns a successful `CallToolResult` containing text.
fn ok_text(text: String) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
}

/// Returns an error `CallToolResult` containing a message.
fn err_text(msg: String) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::error(vec![ContentBlock::text(msg)]))
}

/// Appends a machine-readable `Commit: <sha>` trailer line to a tool's
/// human-formatted output, so agents can thread the resulting plan-repo
/// commit into review provenance (`applied_commit`) without a follow-up
/// call. No-op when there is no commit to report.
///
/// Used exclusively by the `rdm_commit` tool (see the `git`-feature-gated
/// tool router below) — mutation tools no longer report a commit since MCP
/// mutations only stage.
#[cfg(feature = "git")]
fn with_commit_trailer(mut text: String, sha: Option<String>) -> String {
    if let Some(sha) = sha {
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&format!("Commit: {sha}\n"));
    }
    text
}

/// Renders every document a review references — the target itself plus each
/// distinct phase a comment scopes to (roadmap reviews only) — as JSON
/// entries carrying both the body at the review's `created_commit` (what
/// the reviewer saw; `Original`/drifted resolution ranges index it) and the
/// current body (`Current` resolution ranges index it; also what
/// whole-document comments should be read against). Either body is `null`
/// when unavailable (no recorded commit, document absent at that revision,
/// deleted since, or a historyless backend).
fn review_documents(
    store: &AppStore,
    project: &str,
    review: &rdm_core::model::Review,
) -> Vec<serde_json::Value> {
    let mut refs: Vec<(String, rdm_core::store::RelPath)> = Vec::new();
    match &review.target {
        ReviewTarget::Roadmap { roadmap } => refs.push((
            format!("roadmap/{roadmap}"),
            rdm_core::paths::roadmap_path(project, roadmap),
        )),
        ReviewTarget::Phase { roadmap, stem } => refs.push((
            format!("phase/{roadmap}/{stem}"),
            rdm_core::paths::phase_path(project, roadmap, stem),
        )),
        ReviewTarget::Task { slug } => refs.push((
            format!("task/{slug}"),
            rdm_core::paths::task_path(project, slug),
        )),
    }
    // Comment doc scopes are only valid on roadmap reviews (enforced at
    // write time) and only phase-kinded today; first-appearance order.
    if let ReviewTarget::Roadmap { roadmap } = &review.target {
        for comment in &review.comments {
            if let Some(doc) = &comment.doc {
                let label = format!("phase/{roadmap}/{}", doc.stem);
                if !refs.iter().any(|(l, _)| l == &label) {
                    refs.push((
                        label,
                        rdm_core::paths::phase_path(project, roadmap, &doc.stem),
                    ));
                }
            }
        }
    }

    let body_of = |content: String| {
        rdm_core::markdown::split_frontmatter(&content)
            .ok()
            .map(|(_, body)| body.to_string())
    };
    refs.into_iter()
        .map(|(label, path)| {
            let current_body = store.read(&path).ok().and_then(body_of);
            let body_at_created_commit = review
                .created_commit
                .as_ref()
                .and_then(|sha| store.fetch_body_at(&path, sha).ok())
                .and_then(body_of);
            serde_json::json!({
                "ref": label,
                "created_commit": review.created_commit,
                "body_at_created_commit": body_at_created_commit,
                "current_body": current_body,
            })
        })
        .collect()
}

// ---------- Parameter structs (read-only) ----------

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
    /// Filter by status (e.g. "open", "in-progress", "needs-review", "reviewed", "done", "blocked", "wont-fix", or "all"). Omit for default (open + in-progress).
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
    /// Filter by status (e.g. "open", "in-progress", "needs-review", "reviewed", "done").
    status: Option<String>,
    /// Filter by tags (AND semantics — items must carry every listed tag).
    /// Items with no tags are excluded by any non-empty list.
    tags: Option<Vec<String>>,
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
    /// Priority level: "low", "medium", "high", or "critical".
    priority: Option<String>,
    /// Optional tags for categorization (e.g. ["bug", "ui"]).
    tags: Option<Vec<String>>,
    /// Optional body content (Markdown).
    body: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RoadmapListParams {
    /// The project name.
    project: String,
    /// Sort order: "alphabetical" (default) or "priority" (descending).
    sort: Option<String>,
    /// Filter by priority level: "low", "medium", "high", or "critical".
    priority: Option<String>,
    /// Filter to roadmaps carrying this tag.
    tag: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RoadmapUpdateParams {
    /// The project name.
    project: String,
    /// The roadmap slug.
    roadmap: String,
    /// New priority level: "low", "medium", "high", or "critical".
    priority: Option<String>,
    /// Set to true to remove the priority from this roadmap.
    clear_priority: Option<bool>,
    /// New tags (replaces existing). Pass an empty array or set
    /// `clear_tags: true` to remove all tags.
    tags: Option<Vec<String>>,
    /// Set to true to remove all tags from this roadmap.
    clear_tags: Option<bool>,
    /// New body content (Markdown). Replaces the existing body. Passing
    /// an empty string against a non-empty body is rejected; use
    /// `clear_body: true` to confirm.
    body: Option<String>,
    /// Set to true to clear the existing body (mutually exclusive with `body`).
    clear_body: Option<bool>,
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
    /// Optional tags for categorization (e.g. ["bug", "ui"]).
    tags: Option<Vec<String>>,
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
    /// New status: "not-started", "in-progress", "needs-review", "reviewed", "done", "blocked", or "wont-fix".
    status: Option<String>,
    /// New tags (replaces existing). Pass an empty array or set
    /// `clear_tags: true` to remove all tags.
    tags: Option<Vec<String>>,
    /// Set to true to remove all tags from this phase.
    clear_tags: Option<bool>,
    /// New body content (Markdown). Replaces the existing body. Passing
    /// an empty string against a non-empty body is rejected; use
    /// `clear_body: true` to confirm.
    body: Option<String>,
    /// Set to true to clear the existing body (mutually exclusive with `body`).
    clear_body: Option<bool>,
    /// Escalation reason to record when parking the phase as `blocked`. Prefix
    /// with the stage that raised it: `[plan]` or `[code]`. Preserved across a
    /// later resume; mutually exclusive with `clear_reason`.
    reason: Option<String>,
    /// Set to true to clear the recorded blocked reason (mutually exclusive
    /// with `reason`).
    clear_reason: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PhaseListParams {
    /// The project name.
    project: String,
    /// The roadmap slug.
    roadmap: String,
    /// Filter to phases carrying this tag.
    tag: Option<String>,
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
    /// Optional tags for categorization (e.g. ["bug", "ui"]).
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
    /// New status: "open", "in-progress", "needs-review", "reviewed", "done", "blocked", or "wont-fix".
    status: Option<String>,
    /// New priority: "low", "medium", "high", or "critical".
    priority: Option<String>,
    /// New tags (replaces existing). Pass an empty array to remove all tags.
    tags: Option<Vec<String>>,
    /// New body content (Markdown). Replaces the existing body. Passing
    /// an empty string against a non-empty body is rejected; use
    /// `clear_body: true` to confirm.
    body: Option<String>,
    /// Set to true to clear the existing body (mutually exclusive with `body`).
    clear_body: Option<bool>,
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

// ---------- Parameter structs (document reviews) ----------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ReviewRequestsParams {
    /// The project name.
    project: String,
    /// Optional target-kind filter: "roadmap", "phase", or "task".
    target_kind: Option<String>,
    /// Optional target-id filter (requires `target_kind`): a roadmap slug,
    /// "<roadmap>/<phase-stem-or-number>" for a phase, or a task slug.
    target_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ReviewShowParams {
    /// The project name.
    project: String,
    /// Id of the review to show.
    review_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ReviewAddressCommentParams {
    /// The project name.
    project: String,
    /// Id of the review containing the comment.
    review_id: String,
    /// Id of the comment to resolve (see `rdm_review_show`).
    comment_id: u32,
    /// New resolution status: "addressed" or "wont-fix". Omit to only
    /// record a reply and leave the comment open (e.g. asking the reviewer
    /// for clarification).
    status: Option<String>,
    /// Plan-repo commit SHA that made the change, recorded on the comment.
    /// Thread the `Commit:` value reported by the `rdm_commit` tool call
    /// that landed the fix. Left `null` if omitted — MCP mutations only
    /// stage, so there is no commit to default to.
    applied_commit: Option<String>,
    /// Reply explaining what changed (or why the comment won't be fixed),
    /// including whether the anchor resolved.
    reply: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ReviewCompleteParams {
    /// The project name.
    project: String,
    /// Id of the review to close as addressed.
    review_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct InitParams {
    /// Optional default project to create during initialization.
    default_project: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ProjectCreateParams {
    /// The project name (slug identifier).
    name: String,
    /// The project title. Defaults to the name if omitted.
    title: Option<String>,
}

// ---------- Parameter structs (worktree; git feature only) ----------

/// Parameters for `rdm_worktree_add`.
#[cfg(feature = "git")]
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct WorktreeAddParams {
    /// The project name (used to validate the item against the plan repo).
    project: String,
    /// The plan item: `<roadmap>/<phase-stem-or-number>`, `task/<slug>`, or a
    /// bare `<roadmap>` (one worktree on `roadmap/<slug>` shared by its phases).
    item: String,
    /// Optional base ref to branch from (defaults to the current HEAD).
    base: Option<String>,
}

/// Parameters for `rdm_worktree_remove`.
#[cfg(feature = "git")]
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct WorktreeRemoveParams {
    /// The project name (used to resolve a numeric phase identifier in `target`).
    project: Option<String>,
    /// The worktree to remove: a plan item reference or a filesystem path.
    target: String,
    /// Also delete the worktree's branch after removal.
    delete_branch: Option<bool>,
    /// Remove even if the worktree is dirty (and force-delete the branch).
    force: Option<bool>,
}

// ---------- Parameter structs (git status/commit/discard; git feature only) ----------

/// Parameters for `rdm_commit`.
#[cfg(feature = "git")]
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CommitParams {
    /// Commit message. Omit to auto-generate a summary from the staged
    /// changes (matching the CLI's `rdm commit` default).
    message: Option<String>,
}

/// Parameters for `rdm_discard`.
#[cfg(feature = "git")]
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DiscardParams {
    /// Must be `true` to confirm the (irreversible) discard of staged
    /// changes. Omitting it is treated as `false` (rejected) rather than a
    /// schema validation error, so a missing confirmation always surfaces as
    /// an ordinary tool error instead of a protocol-level one.
    confirm: Option<bool>,
}

// ---------- Store helpers ----------

/// Creates an [`AppStore`] for an existing plan repo.
///
/// MCP mutations always stage to disk without an implicit git commit — there
/// is no human present to run `rdm commit` after every tool call, so the
/// server relies on the explicit `rdm_commit` tool to land history.
fn make_store(root: &Path) -> anyhow::Result<AppStore> {
    #[cfg(feature = "git")]
    {
        rdm_store_git::GitStore::new(root)
            .map_err(|e| anyhow::anyhow!("failed to open git repository: {e}"))
    }
    #[cfg(not(feature = "git"))]
    {
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
}

impl RdmMcpServer {
    fn new(plan_root: PathBuf, auto_init: bool) -> anyhow::Result<Self> {
        // Try opening an existing repo. If it doesn't exist yet (common when the
        // MCP server starts before `rdm_init`), create the git repo so the server
        // can start — the plan-level initialisation happens later via the rdm_init
        // tool or maybe_auto_init.
        let store = match make_store(&plan_root) {
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

    /// Builds the full tool router, combining the core tools with the
    /// worktree tools when the `git` feature is enabled.
    ///
    /// The worktree tools live in a separate `#[tool_router]` impl block so the
    /// whole block (and its generated router) can be `#[cfg]`-gated — the
    /// `#[tool_router]` macro does not propagate `#[cfg]` attributes on
    /// individual tool methods.
    fn all_tools_router() -> rmcp::handler::server::router::tool::ToolRouter<Self> {
        #[allow(unused_mut)]
        let mut router = Self::tool_router();
        #[cfg(feature = "git")]
        router.merge(Self::worktree_tool_router());
        #[cfg(feature = "git")]
        router.merge(Self::git_ops_tool_router());
        router
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
            && let Err(e) = rdm_core::ops::mutate(&mut *store, proj, |s| {
                rdm_core::ops::project::create_project(s, proj, proj)
            })
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

    /// List all roadmaps in a project with phase progress. Supports sorting and priority/tag filters.
    #[rmcp::tool(
        description = "List all roadmaps in a project with phase progress. Supports optional sort (alphabetical or priority), priority filter, and tag filter.",
        annotations(read_only_hint = true)
    )]
    async fn rdm_roadmap_list(
        &self,
        Parameters(params): Parameters<RoadmapListParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.maybe_auto_init();

        let sort = match &params.sort {
            Some(s) => match RoadmapSort::from_str(s) {
                Ok(rs) => Some(rs),
                Err(msg) => return err_text(msg.to_string()),
            },
            None => None,
        };
        let priority_filter = match &params.priority {
            Some(p) => match Priority::from_str(p) {
                Ok(pr) => Some(pr),
                Err(msg) => return err_text(msg.to_string()),
            },
            None => None,
        };

        let store = self.store.lock().unwrap();
        let roadmaps = match rdm_core::ops::roadmap::list_roadmaps(
            &*store,
            &params.project,
            sort,
            priority_filter,
        ) {
            Ok(r) => r,
            Err(e) => return core_err(e),
        };

        let filtered_roadmaps: Vec<_> = match &params.tag {
            Some(tag) => roadmaps
                .into_iter()
                .filter(|doc| {
                    doc.frontmatter
                        .tags
                        .as_ref()
                        .is_some_and(|tags| tags.iter().any(|t| t == tag))
                })
                .collect(),
            None => roadmaps,
        };

        let mut entries = Vec::new();
        for roadmap_doc in filtered_roadmaps {
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

        ok_text(display::format_roadmap_summary(&doc, &phases, None))
    }

    /// List all phases in a roadmap, optionally filtered by tag.
    #[rmcp::tool(
        description = "List all phases in a roadmap, optionally filtered by tag",
        annotations(read_only_hint = true)
    )]
    async fn rdm_phase_list(
        &self,
        Parameters(params): Parameters<PhaseListParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.maybe_auto_init();
        let store = self.store.lock().unwrap();
        let phases =
            match rdm_core::ops::phase::list_phases(&*store, &params.project, &params.roadmap) {
                Ok(p) => p,
                Err(e) => return core_err(e),
            };
        let filtered: Vec<_> = match &params.tag {
            Some(tag) => phases
                .into_iter()
                .filter(|(_, doc)| {
                    doc.frontmatter
                        .tags
                        .as_ref()
                        .is_some_and(|tags| tags.iter().any(|t| t == tag))
                })
                .collect(),
            None => phases,
        };
        ok_text(display::format_phase_list(&filtered))
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
            Ok(doc) => ok_text(display::format_task_detail(&params.task, &doc, None)),
            Err(e) => core_err(e),
        }
    }

    /// Search for items across the plan repo.
    #[rmcp::tool(
        description = "Search for items across the plan repo by fuzzy-matching titles and body content. Optional tag filter ANDs across the listed tags.",
        annotations(read_only_hint = true)
    )]
    async fn rdm_search(
        &self,
        Parameters(params): Parameters<SearchParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.maybe_auto_init();
        let store = self.store.lock().unwrap();
        let kind = match &params.kind {
            Some(k) => match k.parse::<ItemKind>() {
                Ok(kind) => Some(kind),
                Err(msg) => return err_text(msg.to_string()),
            },
            None => None,
        };

        let status = match &params.status {
            Some(s) => match s.parse::<ItemStatus>() {
                Ok(st) => Some(st),
                Err(msg) => return err_text(msg.to_string()),
            },
            None => None,
        };

        let filter = SearchFilter {
            kind,
            project: params.project,
            status,
            tags: params.tags,
            min_score_ratio: None,
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

    /// Create a new project.
    #[rmcp::tool(
        description = "Create a new project in the plan repo",
        annotations(read_only_hint = false)
    )]
    async fn rdm_project_create(
        &self,
        Parameters(params): Parameters<ProjectCreateParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.maybe_auto_init();
        let mut store = self.store.lock().unwrap();
        let title = params.title.as_deref().unwrap_or(&params.name);
        let doc = match rdm_core::ops::mutate(&mut *store, &params.name, |s| {
            rdm_core::ops::project::create_project(s, &params.name, title)
        }) {
            Ok(d) => d,
            Err(e) => return core_err(e),
        };
        ok_text(format!("Created project '{}'", doc.frontmatter.name))
    }

    /// Create a new roadmap in a project.
    #[rmcp::tool(
        description = "Create a new roadmap in a project, optionally with priority and tags",
        annotations(read_only_hint = false)
    )]
    async fn rdm_roadmap_create(
        &self,
        Parameters(params): Parameters<RoadmapCreateParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.maybe_auto_init();
        let priority = match &params.priority {
            Some(p) => match Priority::from_str(p) {
                Ok(pr) => Some(pr),
                Err(msg) => return err_text(msg.to_string()),
            },
            None => None,
        };
        let mut store = self.store.lock().unwrap();
        let doc = match rdm_core::ops::mutate(&mut *store, &params.project, |s| {
            rdm_core::ops::roadmap::create_roadmap(
                s,
                rdm_core::ops::roadmap::CreateRoadmap {
                    project: &params.project,
                    slug: &params.slug,
                    title: &params.title,
                    body: params.body.as_deref(),
                    priority,
                    tags: params.tags.clone(),
                },
            )
        }) {
            Ok(d) => d,
            Err(e) => return core_err(e),
        };
        let phases = match rdm_core::ops::phase::list_phases(&*store, &params.project, &params.slug)
        {
            Ok(p) => p,
            Err(e) => return core_err(e),
        };
        ok_text(display::format_roadmap_summary(&doc, &phases, None))
    }

    /// Update a roadmap's priority, tags, and/or body.
    #[rmcp::tool(
        description = "Update a roadmap's priority, tags, and/or body content",
        annotations(read_only_hint = false)
    )]
    async fn rdm_roadmap_update(
        &self,
        Parameters(params): Parameters<RoadmapUpdateParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.maybe_auto_init();

        let body = match BodyUpdate::from_args(params.body, params.clear_body.unwrap_or(false)) {
            Ok(b) => b,
            Err(e) => return core_err(e),
        };
        let priority_val = match &params.priority {
            Some(p) => match Priority::from_str(p) {
                Ok(pr) => Some(pr),
                Err(msg) => return err_text(msg.to_string()),
            },
            None => None,
        };
        let priority =
            match PriorityUpdate::from_args(priority_val, params.clear_priority.unwrap_or(false)) {
                Ok(p) => p,
                Err(e) => return core_err(e),
            };
        let tags = match TagsUpdate::from_args(params.tags, params.clear_tags.unwrap_or(false)) {
            Ok(t) => t,
            Err(e) => return core_err(e),
        };

        let mut store = self.store.lock().unwrap();
        let doc = match rdm_core::ops::mutate(&mut *store, &params.project, |s| {
            rdm_core::ops::roadmap::update_roadmap(
                s,
                &params.project,
                &params.roadmap,
                body,
                priority,
                tags,
                rdm_core::ops::TitleUpdate::Keep,
            )
        }) {
            Ok(d) => d,
            Err(e) => return core_err(e),
        };
        let phases =
            match rdm_core::ops::phase::list_phases(&*store, &params.project, &params.roadmap) {
                Ok(p) => p,
                Err(e) => return core_err(e),
            };
        ok_text(display::format_roadmap_summary(&doc, &phases, None))
    }

    /// Create a new phase in a roadmap.
    #[rmcp::tool(
        description = "Create a new phase in a roadmap, optionally with tags",
        annotations(read_only_hint = false)
    )]
    async fn rdm_phase_create(
        &self,
        Parameters(params): Parameters<PhaseCreateParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.maybe_auto_init();
        let mut store = self.store.lock().unwrap();
        let doc = match rdm_core::ops::mutate(&mut *store, &params.project, |s| {
            rdm_core::ops::phase::create_phase(
                s,
                rdm_core::ops::phase::CreatePhase {
                    project: &params.project,
                    roadmap: &params.roadmap,
                    slug: &params.slug,
                    title: &params.title,
                    number: params.number,
                    body: params.body.as_deref(),
                    tags: params.tags.clone(),
                    ..Default::default()
                },
            )
        }) {
            Ok(d) => d,
            Err(e) => return core_err(e),
        };
        let stem = doc.frontmatter.stem(&params.slug);
        ok_text(display::format_phase_detail(&stem, &doc, None))
    }

    /// Update a phase's status, tags, or body.
    #[rmcp::tool(
        description = "Update a phase's status, tags, or body content",
        annotations(read_only_hint = false)
    )]
    async fn rdm_phase_update(
        &self,
        Parameters(params): Parameters<PhaseUpdateParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.maybe_auto_init();

        let body = match BodyUpdate::from_args(params.body, params.clear_body.unwrap_or(false)) {
            Ok(b) => b,
            Err(e) => return core_err(e),
        };
        let tags = match TagsUpdate::from_args(params.tags, params.clear_tags.unwrap_or(false)) {
            Ok(t) => t,
            Err(e) => return core_err(e),
        };
        let reason_update =
            match ReasonUpdate::from_args(params.reason, params.clear_reason.unwrap_or(false)) {
                Ok(r) => r,
                Err(e) => return core_err(e),
            };
        let has_reason = !matches!(reason_update, ReasonUpdate::Keep);

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
                Err(msg) => return err_text(msg.to_string()),
            },
            None => None,
        };

        let doc = match rdm_core::ops::mutate(&mut *store, &params.project, |s| {
            let doc = rdm_core::ops::phase::update_phase(
                s,
                &params.project,
                &params.roadmap,
                &stem,
                status,
                tags,
                body,
                None,
                None,
                None,
                rdm_core::ops::TitleUpdate::Keep,
            )?;
            if has_reason {
                rdm_core::ops::phase::set_phase_blocked_reason(
                    s,
                    &params.project,
                    &params.roadmap,
                    &stem,
                    reason_update,
                )
            } else {
                Ok(doc)
            }
        }) {
            Ok(d) => d,
            Err(e) => return core_err(e),
        };
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
                Err(msg) => return err_text(msg.to_string()),
            },
            None => Priority::Medium,
        };

        let mut store = self.store.lock().unwrap();
        let doc = match rdm_core::ops::mutate(&mut *store, &params.project, |s| {
            rdm_core::ops::task::create_task(
                s,
                rdm_core::ops::task::CreateTask {
                    project: &params.project,
                    slug: &params.slug,
                    title: &params.title,
                    priority,
                    tags: params.tags,
                    body: params.body.as_deref(),
                },
            )
        }) {
            Ok(d) => d,
            Err(e) => return core_err(e),
        };
        ok_text(display::format_task_detail(&params.slug, &doc, None))
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
        let body = match BodyUpdate::from_args(params.body, params.clear_body.unwrap_or(false)) {
            Ok(b) => b,
            Err(e) => return core_err(e),
        };
        let tags = match TagsUpdate::from_args(params.tags, false) {
            Ok(t) => t,
            Err(e) => return core_err(e),
        };
        let status = match &params.status {
            Some(s) => match TaskStatus::from_str(s) {
                Ok(st) => Some(st),
                Err(msg) => return err_text(msg.to_string()),
            },
            None => None,
        };

        let priority = match &params.priority {
            Some(p) => match Priority::from_str(p) {
                Ok(pr) => Some(pr),
                Err(msg) => return err_text(msg.to_string()),
            },
            None => None,
        };

        let mut store = self.store.lock().unwrap();
        let doc = match rdm_core::ops::mutate(&mut *store, &params.project, |s| {
            rdm_core::ops::task::update_task(
                s,
                &params.project,
                &params.task,
                status,
                priority,
                tags,
                body,
                None,
                None,
                None,
                rdm_core::ops::TitleUpdate::Keep,
            )
        }) {
            Ok(d) => d,
            Err(e) => return core_err(e),
        };
        ok_text(display::format_task_detail(&params.task, &doc, None))
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
        let doc = match rdm_core::ops::mutate(&mut *store, &params.project, |s| {
            rdm_core::ops::task::promote_task(
                s,
                &params.project,
                &params.task,
                &params.roadmap_slug,
            )
        }) {
            Ok(d) => d,
            Err(e) => return core_err(e),
        };
        let phases =
            match rdm_core::ops::phase::list_phases(&*store, &params.project, &params.roadmap_slug)
            {
                Ok(p) => p,
                Err(e) => return core_err(e),
            };
        ok_text(display::format_roadmap_summary(&doc, &phases, None))
    }

    // ==================== Document review tools ====================

    /// List the change-request review queue.
    #[rmcp::tool(
        description = "List the change-request review queue: submitted document reviews with verdict request-changes, each with id, target, author, summary, created_commit, and open_comment_count. Optional target_kind/target_id filters narrow to one plan item.",
        annotations(read_only_hint = true)
    )]
    async fn rdm_review_requests(
        &self,
        Parameters(params): Parameters<ReviewRequestsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.maybe_auto_init();
        let store = self.store.lock().unwrap();
        let reviews = match rdm_core::ops::reviews::change_requests(&*store, &params.project) {
            Ok(r) => r,
            Err(e) => return core_err(e),
        };

        let target_kind = match params.target_kind.as_deref() {
            None => None,
            Some(k) => match ReviewTargetKind::from_str(k) {
                Ok(kind) => Some(kind),
                Err(msg) => return err_text(msg.to_string()),
            },
        };
        // With both kind and id, resolve to an exact target (this also
        // resolves a phase *number* to its stem). An id without a kind is
        // ambiguous — roadmap and task slugs can collide.
        let target = match (target_kind, &params.target_id) {
            (Some(kind), Some(id)) => {
                match rdm_core::ops::reviews::parse_review_target_ref(
                    &*store,
                    &params.project,
                    &format!("{kind}/{id}"),
                ) {
                    Ok(t) => Some(t),
                    Err(e) => return core_err(e),
                }
            }
            (None, Some(_)) => {
                return err_text(
                    "target_id requires target_kind (\"roadmap\", \"phase\", or \"task\")"
                        .to_string(),
                );
            }
            _ => None,
        };

        let filtered = rdm_core::ops::reviews::filter_reviews(
            reviews,
            &rdm_core::ops::reviews::ReviewFilter {
                target,
                target_kind,
                ..Default::default()
            },
        );
        let entries: Vec<serde_json::Value> = filtered
            .iter()
            .map(|(id, doc)| {
                let open_comment_count = doc
                    .frontmatter
                    .comments
                    .iter()
                    .filter(|c| !c.status.is_terminal())
                    .count();
                serde_json::json!({
                    "id": id,
                    "target": doc.frontmatter.target,
                    "target_ref": doc.frontmatter.target.label(),
                    "author": doc.frontmatter.author,
                    "summary": doc.body,
                    "created_commit": doc.frontmatter.created_commit,
                    "open_comment_count": open_comment_count,
                })
            })
            .collect();
        let value = serde_json::Value::Array(entries);
        ok_text(serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()))
    }

    /// Show a review in full, with resolutions and referenced document bodies.
    #[rmcp::tool(
        description = "Show a document review in full: summary, every comment with its tagged-union anchor (dispatch on anchor_type) and resolution state (resolved/drifted/unresolved, with the byte range and which body it indexes), plus the rendered body of every referenced document at both the review's created_commit and the current HEAD — everything needed to act on the review in one call. Comments with no anchor or an unrecognized anchor_type resolve as unresolved: treat them as whole-document feedback against current_body.",
        annotations(read_only_hint = true)
    )]
    async fn rdm_review_show(
        &self,
        Parameters(params): Parameters<ReviewShowParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.maybe_auto_init();
        let store = self.store.lock().unwrap();
        let doc =
            match rdm_core::ops::reviews::get_review(&*store, &params.project, &params.review_id) {
                Ok(d) => d,
                Err(e) => return core_err(e),
            };
        let resolutions =
            rdm_core::anchor::resolve_comments(&*store, &params.project, &doc.frontmatter);
        let review = rdm_core::json::review_to_json(&params.review_id, &doc, &resolutions);
        let documents = review_documents(&store, &params.project, &doc.frontmatter);
        let value = serde_json::json!({
            "review": review,
            "documents": documents,
        });
        ok_text(serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()))
    }

    /// Resolve (or reply to) a single review comment.
    #[rmcp::tool(
        description = "Resolve one comment on a submitted change-request review: set status (\"addressed\" or \"wont-fix\"), record the plan-repo commit that applied the change, and store a reply. Omit status to only record a reply and leave the comment open (e.g. asking the reviewer for clarification). Pass applied_commit explicitly from the `Commit:` value the rdm_commit tool reported for the batch that applied this fix — MCP mutations only stage, so there is nothing to default it to. Returns the updated comment plus review state and remaining open comment count.",
        annotations(read_only_hint = false)
    )]
    async fn rdm_review_address_comment(
        &self,
        Parameters(params): Parameters<ReviewAddressCommentParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.maybe_auto_init();
        let status = match params.status.as_deref() {
            None => None,
            Some(s) => match ReviewCommentStatus::from_str(s) {
                Ok(ReviewCommentStatus::Open) => {
                    return err_text(
                        "status must be \"addressed\" or \"wont-fix\" — omit status entirely to leave the comment open with a clarification reply"
                            .to_string(),
                    );
                }
                Ok(st) => Some(st),
                Err(msg) => return err_text(msg.to_string()),
            },
        };

        let mut store = self.store.lock().unwrap();
        // An explicitly supplied applied_commit is always honored (even for
        // wont-fix). There is no auto-default: MCP mutations only stage, so
        // there is no plan-repo commit to fall back to until the agent calls
        // rdm_commit.
        let applied_commit = params.applied_commit.clone();
        let doc = match rdm_core::ops::mutate(&mut *store, &params.project, |s| {
            rdm_core::ops::reviews::update_comment(
                s,
                rdm_core::ops::reviews::UpdateComment {
                    project: &params.project,
                    review_id: &params.review_id,
                    comment_id: params.comment_id,
                    status,
                    applied_commit: applied_commit.as_deref(),
                    reply: Some(&params.reply),
                    ..Default::default()
                },
            )
        }) {
            Ok(d) => d,
            Err(e) => return core_err(e),
        };
        let comment = doc
            .frontmatter
            .comments
            .iter()
            .find(|c| c.id == params.comment_id)
            .expect("comment presence enforced by update_comment");
        let open_comment_count = doc
            .frontmatter
            .comments
            .iter()
            .filter(|c| !c.status.is_terminal())
            .count();
        let value = serde_json::json!({
            "review_id": params.review_id,
            "comment_id": params.comment_id,
            "status": comment.status,
            "applied_commit": comment.applied_commit,
            "reply": comment.reply,
            "review_state": doc.frontmatter.state,
            "open_comment_count": open_comment_count,
        });
        ok_text(serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()))
    }

    /// Close a review as addressed.
    #[rmcp::tool(
        description = "Close a submitted review as addressed. Refuses while any comment is still open, listing the offending comment ids — resolve each via rdm_review_address_comment first, or leave the review submitted while clarification is pending.",
        annotations(read_only_hint = false)
    )]
    async fn rdm_review_complete(
        &self,
        Parameters(params): Parameters<ReviewCompleteParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.maybe_auto_init();
        let mut store = self.store.lock().unwrap();
        // Pre-check open comments so the refusal can name the offenders —
        // core's ReviewOpenComments error carries only the count.
        let existing =
            match rdm_core::ops::reviews::get_review(&*store, &params.project, &params.review_id) {
                Ok(d) => d,
                Err(e) => return core_err(e),
            };
        let open_ids: Vec<String> = existing
            .frontmatter
            .comments
            .iter()
            .filter(|c| !c.status.is_terminal())
            .map(|c| c.id.to_string())
            .collect();
        if !open_ids.is_empty() {
            return err_text(format!(
                "cannot complete review '{}': {} comment(s) still open: {} — resolve each with rdm_review_address_comment (status \"addressed\" or \"wont-fix\"), or leave the review submitted while clarification is pending",
                params.review_id,
                open_ids.len(),
                open_ids.join(", "),
            ));
        }
        let doc = match rdm_core::ops::mutate(&mut *store, &params.project, |s| {
            rdm_core::ops::reviews::update_review(
                s,
                &params.project,
                &params.review_id,
                rdm_core::ops::reviews::ReviewTransition::Addressed,
            )
        }) {
            Ok(d) => d,
            Err(e) => return core_err(e),
        };
        ok_text(format!(
            "Review '{}' → state: {}",
            params.review_id, doc.frontmatter.state
        ))
    }
}

// ==================== Worktree tools (git feature only) ====================
//
// These tools live in their own `#[tool_router]` impl block so the entire block
// — and the `worktree_tool_router()` it generates — can be `#[cfg]`-gated. The
// router is combined with the core router in [`RdmMcpServer::all_tools_router`].

#[cfg(feature = "git")]
#[rmcp::tool_router(router = worktree_tool_router)]
impl RdmMcpServer {
    /// Create (or idempotently reuse) an isolated git worktree for a plan item.
    #[rmcp::tool(
        description = "Create (or idempotently reuse) an isolated git worktree and branch in the project (code) repo for a plan item (`<roadmap>/<phase-stem-or-number>`, `task/<slug>`, or a bare `<roadmap>` for one worktree on `roadmap/<slug>` shared by all its phases). Runs against the repo discovered from the server's working directory and refuses to run inside the plan repo. Returns the item, branch, path, and whether it was newly created.",
        annotations(read_only_hint = false)
    )]
    async fn rdm_worktree_add(
        &self,
        Parameters(params): Parameters<WorktreeAddParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.maybe_auto_init();
        let cwd = match std::env::current_dir() {
            Ok(c) => c,
            Err(e) => return err_text(format!("cannot determine current directory: {e}")),
        };
        let repo = match rdm_git::worktree::discover_distinct_project_repo(&cwd, &self.plan_root) {
            Ok(r) => r,
            Err(e) => return err_text(format!("{e}")),
        };
        let item = {
            let store = self.store.lock().unwrap();
            match rdm_git::worktree::resolve_item(&*store, &params.project, &params.item) {
                Ok(i) => i,
                Err(e) => return err_text(format!("{e}")),
            }
        };
        let info =
            match rdm_git::worktree::add(&repo, &item, &item.branch_name(), params.base.as_deref())
            {
                Ok(i) => i,
                Err(e) => return err_text(format!("{e}")),
            };
        let value = serde_json::json!({
            "item": info.item,
            "branch": info.branch,
            "path": info.path.display().to_string(),
            "created": info.created,
        });
        ok_text(serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()))
    }

    /// List the rdm-managed git worktrees in the project (code) repo.
    #[rmcp::tool(
        description = "List the rdm-managed git worktrees in the project (code) repo, with each worktree's item, branch, path, and dirty flag. Runs against the repo discovered from the server's working directory.",
        annotations(read_only_hint = true)
    )]
    async fn rdm_worktree_list(&self) -> Result<CallToolResult, ErrorData> {
        let cwd = match std::env::current_dir() {
            Ok(c) => c,
            Err(e) => return err_text(format!("cannot determine current directory: {e}")),
        };
        let repo = match rdm_git::worktree::discover_project_repo(&cwd) {
            Ok(r) => r,
            Err(e) => return err_text(format!("{e}")),
        };
        let worktrees = match rdm_git::worktree::list(&repo) {
            Ok(w) => w,
            Err(e) => return err_text(format!("{e}")),
        };
        let arr: Vec<_> = worktrees
            .iter()
            .map(|w| {
                serde_json::json!({
                    "item": w.item,
                    "branch": w.branch,
                    "path": w.path.display().to_string(),
                    "dirty": w.dirty,
                })
            })
            .collect();
        let value = serde_json::Value::Array(arr);
        ok_text(serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()))
    }

    /// Report the plan item the server's current checkout corresponds to.
    #[rmcp::tool(
        description = "Report the plan item the current checkout (the server's working directory) corresponds to: the rdm worktree marker if present, otherwise the item inferred from the branch name (`phase/<roadmap>/<stem>`, `task/<slug>`, or `roadmap/<slug>`). Returns null when the checkout is on neither (e.g. the main checkout on `main`). Read-only.",
        annotations(read_only_hint = true)
    )]
    async fn rdm_worktree_current(&self) -> Result<CallToolResult, ErrorData> {
        let cwd = match std::env::current_dir() {
            Ok(c) => c,
            Err(e) => return err_text(format!("cannot determine current directory: {e}")),
        };
        let value = match rdm_git::worktree::current(&cwd) {
            Ok(Some(c)) => serde_json::json!({
                "item": c.item,
                "branch": c.branch,
                "path": c.path.display().to_string(),
                "rdm_managed": c.rdm_managed,
            }),
            Ok(None) => serde_json::Value::Null,
            Err(e) => return err_text(format!("{e}")),
        };
        ok_text(serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()))
    }

    /// Remove an rdm-managed git worktree from the project (code) repo.
    #[rmcp::tool(
        description = "Remove an rdm-managed git worktree (by plan item reference or filesystem path) from the project (code) repo. Refuses a dirty worktree unless `force`; `delete_branch` also drops the branch (refusing unmerged commits without `force`).",
        annotations(read_only_hint = false)
    )]
    async fn rdm_worktree_remove(
        &self,
        Parameters(params): Parameters<WorktreeRemoveParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.maybe_auto_init();
        let cwd = match std::env::current_dir() {
            Ok(c) => c,
            Err(e) => return err_text(format!("cannot determine current directory: {e}")),
        };
        let repo = match rdm_git::worktree::discover_project_repo(&cwd) {
            Ok(r) => r,
            Err(e) => return err_text(format!("{e}")),
        };
        let resolved = {
            let store = self.store.lock().unwrap();
            let project = params.project.as_deref().unwrap_or("");
            rdm_git::worktree::resolve_target(&*store, project, &params.target)
        };
        match rdm_git::worktree::remove(
            &repo,
            &resolved,
            rdm_git::worktree::RemoveOptions {
                force: params.force.unwrap_or(false),
                delete_branch: params.delete_branch.unwrap_or(false),
            },
        ) {
            Ok(()) => ok_text(format!("Removed worktree for {}", params.target)),
            Err(e) => err_text(format!("{e}")),
        }
    }
}

// ==================== Git status/commit/discard tools (git feature only) ====================
//
// MCP mutations only stage changes to disk (see `make_store`) — there is no
// human present to run `rdm commit` after every tool call, so these tools
// mirror the CLI's `rdm status` / `rdm commit` / `rdm discard`, letting an
// agent inspect what's staged and land (or discard) a batch explicitly. They
// live in their own `#[tool_router]` impl block for the same `#[cfg]`-gating
// reason as the worktree tools above.

#[cfg(feature = "git")]
#[rmcp::tool_router(router = git_ops_tool_router)]
impl RdmMcpServer {
    /// Report staged-but-uncommitted changes in the plan repo.
    #[rmcp::tool(
        description = "List staged-but-uncommitted changes in the plan repo, each as {path, change} where change is \"added\", \"modified\", or \"deleted\". MCP mutation tools only stage to disk — call this to see what a batch of edits touched before landing it with rdm_commit.",
        annotations(read_only_hint = true)
    )]
    async fn rdm_status(&self) -> Result<CallToolResult, ErrorData> {
        self.maybe_auto_init();
        let store = self.store.lock().unwrap();
        let statuses = match store.git().git_status() {
            Ok(s) => s,
            Err(e) => return core_err(e),
        };
        let arr: Vec<serde_json::Value> = statuses
            .iter()
            .map(|s| {
                let change = match s.change {
                    rdm_store_git::FileChange::Added => "added",
                    rdm_store_git::FileChange::Modified => "modified",
                    rdm_store_git::FileChange::Deleted => "deleted",
                };
                serde_json::json!({ "path": s.path, "change": change })
            })
            .collect();
        let value = serde_json::Value::Array(arr);
        ok_text(serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()))
    }

    /// Commit all currently staged changes as a single git commit.
    #[rmcp::tool(
        description = "Land every currently staged change as one git commit. Mutate freely across as many tool calls as you need, then call rdm_commit once per logical batch of work — do not commit after every single edit. Omit `message` to auto-generate a summary from the changed files (matching the CLI's `rdm commit` default). No-op (`Nothing to commit.`) if the working tree is already clean. Returns a `Commit: <sha>` line — thread that value into `applied_commit` on rdm_review_address_comment.",
        annotations(read_only_hint = false)
    )]
    async fn rdm_commit(
        &self,
        Parameters(params): Parameters<CommitParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.maybe_auto_init();
        let store = self.store.lock().unwrap();
        let statuses = match store.git().git_status() {
            Ok(s) => s,
            Err(e) => return core_err(e),
        };
        if statuses.is_empty() {
            return ok_text("Nothing to commit.".to_string());
        }
        let message = params
            .message
            .unwrap_or_else(|| rdm_store_git::GitRepo::default_commit_message(&statuses));
        if let Err(e) = store.commit_now(&message) {
            return core_err(e);
        }
        let sha = store.head_sha().ok();
        ok_text(with_commit_trailer(
            format!("Committed {} file(s).", statuses.len()),
            sha,
        ))
    }

    /// Discard all staged changes, reverting the plan repo to HEAD.
    #[rmcp::tool(
        description = "Discard every staged-but-uncommitted change, reverting the plan repo's working tree to its last commit. Irreversible — requires confirm: true, and rejects the call before touching anything if it is missing or false. No-op (`Nothing to discard.`) if the working tree is already clean.",
        annotations(read_only_hint = false)
    )]
    async fn rdm_discard(
        &self,
        Parameters(params): Parameters<DiscardParams>,
    ) -> Result<CallToolResult, ErrorData> {
        if !params.confirm.unwrap_or(false) {
            return err_text(
                "discarding changes is irreversible — pass confirm: true to proceed".to_string(),
            );
        }
        self.maybe_auto_init();
        let store = self.store.lock().unwrap();
        let statuses = match store.git().git_status() {
            Ok(s) => s,
            Err(e) => return core_err(e),
        };
        if statuses.is_empty() {
            return ok_text("Nothing to discard.".to_string());
        }
        if let Err(e) = store.git().git_discard() {
            return core_err(e);
        }
        ok_text(format!("Discarded {} file(s).", statuses.len()))
    }
}

/// Parse a status string into an `ItemStatus`.
///
#[rmcp::tool_handler(router = Self::all_tools_router())]
impl ServerHandler for RdmMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("rdm-mcp", env!("CARGO_PKG_VERSION")))
            .with_instructions("MCP server for managing rdm plan repos.")
    }
}

/// Start the MCP server on stdin/stdout.
///
/// # Errors
///
/// Returns an error if the transport fails to initialize or the server
/// encounters a fatal I/O error.
pub async fn run(plan_root: PathBuf, auto_init: bool) -> anyhow::Result<()> {
    let server = RdmMcpServer::new(plan_root, auto_init)?;
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
