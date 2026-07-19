//! The clap command-line type tree for `rdm`.
//!
//! Holds the top-level [`Cli`] parser, the [`Command`] enum, every subcommand
//! enum, and the small value-enum helpers ([`ItemKindArg`], [`OutputFormat`]).
//! `main.rs` re-exports these (`pub(crate) use cli::*`) so command modules keep
//! referring to `crate::RoadmapCommand`, `crate::OutputFormat`, etc.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use rdm_core::model::{
    Difficulty, ModelTier, PhaseStatus, Priority, RoadmapSort, TaskStatus, TaskStatusFilter,
};
#[cfg(feature = "git")]
use rdm_core::model::{ReviewCommentStatus, ReviewState, Verdict};
use rdm_core::search::ItemKind;

#[derive(Parser)]
#[command(name = "rdm", about = "Manage project roadmaps, phases, and tasks")]
pub(crate) struct Cli {
    /// Path to the plan repo root.
    #[arg(long, env = "RDM_ROOT")]
    pub root: Option<PathBuf>,

    /// Suppress automatic INDEX.md regeneration after mutations.
    #[arg(long, global = true)]
    pub no_index: bool,

    /// Output format (human, json, table, or markdown).
    #[arg(long, global = true)]
    pub format: Option<OutputFormat>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub(crate) enum Command {
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
        /// Print only the resolved plan-repo path to stdout; all other output
        /// (the human banner, fetch/clone narration) moves to stderr. Designed
        /// for `$(...)` capture in shell hooks. Takes precedence over `--format`
        /// when both are given — even for `--format table`/`--format markdown`.
        /// The printed path mirrors `--path` as given (a relative `--path`
        /// prints relative); the default location is always absolute.
        #[arg(long)]
        print_root: bool,
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
    /// Promote a task to a new roadmap, or consolidate it into an existing one.
    Promote {
        /// Task slug to promote.
        task_slug: String,
        /// Roadmap slug for a brand-new roadmap (1:1 promotion). Mutually
        /// exclusive with `--into`.
        #[arg(long, conflicts_with = "into")]
        roadmap_slug: Option<String>,
        /// Slug of an existing roadmap to fold the task into as a new
        /// trailing phase. Mutually exclusive with `--roadmap-slug`.
        #[arg(long, conflicts_with = "roadmap_slug")]
        into: Option<String>,
        /// Phase body content when consolidating via `--into`. Accepts any
        /// text verbatim and always takes precedence over stdin.
        #[arg(long)]
        body: Option<String>,
        /// Suppress interactive editor for phase body content (applies to `--into`).
        #[arg(long)]
        no_edit: bool,
        /// Project the task belongs to.
        #[arg(long)]
        project: Option<String>,
    },
    /// Generate INDEX.md from current repo state.
    Index {
        /// Internal: path git substitutes for `%A` when `rdm index` is
        /// invoked as the auto-installed `rdm-index` merge driver. Not
        /// intended for direct interactive use.
        #[arg(long, requires = "merge_path")]
        merge_output: Option<PathBuf>,
        /// Internal: path git substitutes for `%P` when `rdm index` is
        /// invoked as the auto-installed `rdm-index` merge driver.
        #[arg(long, requires = "merge_output")]
        merge_path: Option<String>,
    },
    /// Generate agent configuration for AI coding assistants.
    AgentConfig {
        /// Target platform (claude, agents-md, cursor, copilot, pi).
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
        /// Also write the auto-review hook (claude and pi; composable with `--skills`).
        /// Claude: a Stop hook script registered in `.claude/settings.json`. Pi: a
        /// `.pi/extensions/rdm-review.ts` extension (auto-discovered, fires on `agent_end`).
        #[arg(long)]
        hooks: bool,
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
    /// Manage git worktrees keyed to plan items in the project (code) repo.
    #[cfg(feature = "git")]
    Worktree {
        #[command(subcommand)]
        command: WorktreeCommand,
    },
    /// Author document reviews and inspect items awaiting implementation review.
    #[cfg(feature = "git")]
    #[command(
        after_help = "Command families:\n  Author document reviews:\n      start, comment, submit, list, show, update, delete, requests\n  Inspect items awaiting implementation review (the needs-review queue):\n      pending, restamp, blocked"
    )]
    Review {
        #[command(subcommand)]
        command: ReviewCommand,
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
        /// Quick-filter chip to render on list pages.
        ///
        /// Format: `Label:tag` (e.g. `Bugs:bug`). Repeat to add more.
        /// Overrides any `[server.quick_filters]` from `rdm.toml` and the
        /// `RDM_SERVER_QUICK_FILTERS` env var.
        #[arg(long = "quick-filter")]
        quick_filter: Vec<String>,
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
    /// Show the next actionable phase in a roadmap.
    Next {
        /// Roadmap to resolve the next actionable phase for.
        #[arg(long)]
        roadmap: String,
        /// Project the roadmap belongs to.
        #[arg(long)]
        project: Option<String>,
    },
    /// Resolve or inspect the model-tier sizing policy for dispatch steps.
    Model {
        #[command(subcommand)]
        command: ModelCommand,
    },
    /// Backlog grooming signals (stale tasks, duplicates, tag clusters, archivable roadmaps).
    Backlog {
        #[command(subcommand)]
        command: BacklogCommand,
    },
    /// Inspect tags in use across a project.
    Tag {
        #[command(subcommand)]
        command: TagCommand,
    },
}

#[derive(Subcommand)]
pub(crate) enum TagCommand {
    /// List every tag in use with a count of items carrying it.
    ///
    /// Counts roadmaps and tasks (all statuses, including terminal ones);
    /// phases and archived roadmaps are not scanned. Tags are compared
    /// verbatim — `CLI` and `cli` are distinct tags. Performs zero writes.
    List {
        /// Project to list tags for.
        #[arg(long)]
        project: Option<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum BacklogCommand {
    /// Print a read-only backlog grooming report.
    ///
    /// Surfaces stale tasks, likely-duplicate task clusters (via the existing
    /// fuzzy search matcher), thematic tag clusters, and archivable roadmaps.
    /// Performs zero writes.
    Report {
        /// Staleness threshold in days: a task is flagged once its `created`
        /// date is at least this many days in the past. Default: 60.
        #[arg(long, default_value_t = rdm_core::ops::backlog::DEFAULT_STALE_THRESHOLD_DAYS as u32)]
        older_than: u32,
        /// Restrict every section to items carrying this tag.
        #[arg(long)]
        tag: Option<String>,
        /// Project to report on.
        #[arg(long)]
        project: Option<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum ModelCommand {
    /// Resolve a dispatch step (plus optional tier hint) to a concrete model id.
    Resolve {
        /// Dispatch step: plan, implement, review-find, review-verify, or mechanical.
        step: String,
        /// Caller tier hint (small, medium, large) overriding the step's configured/default tier.
        #[arg(long)]
        tier: Option<String>,
    },
    /// Show the resolved model policy: tier bindings, review floor, and each step's no-hint model.
    Show,
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
        /// Body content for the roadmap. Accepts any text verbatim
        /// (backticks, em-dashes, other Unicode/punctuation included) and
        /// always takes precedence over stdin — stdin is never read once
        /// this is set.
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
        /// Read the body as it was at a specific git revision.
        #[arg(long)]
        at: Option<String>,
    },
    /// Update a roadmap's title, priority, tags, and/or body.
    Update {
        /// Roadmap slug.
        slug: String,
        /// Project the roadmap belongs to.
        #[arg(long)]
        project: Option<String>,
        /// New title (renames the roadmap in place; the slug is unchanged).
        #[arg(long)]
        title: Option<String>,
        /// New priority level.
        #[arg(long, conflicts_with = "clear_priority")]
        priority: Option<Priority>,
        /// Remove the priority from this roadmap.
        #[arg(long, conflicts_with = "priority")]
        clear_priority: bool,
        /// New comma-separated tags (replaces existing).
        #[arg(long, value_delimiter = ',')]
        tags: Option<Vec<String>>,
        /// Body content for the roadmap. Accepts any text verbatim
        /// (backticks, em-dashes, other Unicode/punctuation included) and
        /// always takes precedence over stdin — stdin is never read once
        /// this is set.
        #[arg(long, conflicts_with = "clear_body")]
        body: Option<String>,
        /// Clear an existing body (replace it with an empty string).
        #[arg(long, conflicts_with = "body")]
        clear_body: bool,
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
        /// Estimated difficulty (trivial, easy, moderate, hard).
        #[arg(long)]
        difficulty: Option<Difficulty>,
        /// Model tier that should run the phase (small, medium, large).
        #[arg(long)]
        model: Option<ModelTier>,
        /// Body content for the phase. Accepts any text verbatim
        /// (backticks, em-dashes, other Unicode/punctuation included) and
        /// always takes precedence over stdin — stdin is never read once
        /// this is set.
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
        /// Read the body as it was at a specific git revision.
        #[arg(long)]
        at: Option<String>,
    },
    /// Update a phase's status, title, and/or body.
    Update {
        /// Phase stem or number (e.g. phase-1-core or 1).
        stem: String,
        /// New status (omit to preserve existing).
        #[arg(long)]
        status: Option<PhaseStatus>,
        /// New title (renames the phase in place; the stem/number is unchanged).
        #[arg(long)]
        title: Option<String>,
        /// Roadmap the phase belongs to.
        #[arg(long)]
        roadmap: String,
        /// Project the roadmap belongs to.
        #[arg(long)]
        project: Option<String>,
        /// New comma-separated tags (replaces existing).
        #[arg(long, value_delimiter = ',')]
        tags: Option<Vec<String>>,
        /// New estimated difficulty (trivial, easy, moderate, hard).
        #[arg(long, conflicts_with = "clear_difficulty")]
        difficulty: Option<Difficulty>,
        /// Remove the difficulty from this phase.
        #[arg(long, conflicts_with = "difficulty")]
        clear_difficulty: bool,
        /// New model tier (small, medium, large).
        #[arg(long, conflicts_with = "clear_model")]
        model: Option<ModelTier>,
        /// Remove the model tier from this phase.
        #[arg(long, conflicts_with = "model")]
        clear_model: bool,
        /// Body content for the phase. Accepts any text verbatim
        /// (backticks, em-dashes, other Unicode/punctuation included) and
        /// always takes precedence over stdin — stdin is never read once
        /// this is set.
        #[arg(long, conflicts_with = "clear_body")]
        body: Option<String>,
        /// Clear an existing body (replace it with an empty string).
        #[arg(long, conflicts_with = "body")]
        clear_body: bool,
        /// Escalation reason to record when parking the phase as blocked.
        #[arg(long, conflicts_with = "clear_reason")]
        reason: Option<String>,
        /// Remove the recorded blocked reason from this phase.
        #[arg(long, conflicts_with = "reason")]
        clear_reason: bool,
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
        /// Body content for the task. Accepts any text verbatim (backticks,
        /// em-dashes, other Unicode/punctuation included) and always takes
        /// precedence over stdin — stdin is never read once this is set.
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
        /// Read the body as it was at a specific git revision.
        #[arg(long)]
        at: Option<String>,
    },
    /// Update a task.
    Update {
        /// Task slug.
        slug: String,
        /// Project the task belongs to.
        #[arg(long)]
        project: Option<String>,
        /// New title (renames the task in place; the slug is unchanged).
        #[arg(long)]
        title: Option<String>,
        /// New status.
        #[arg(long)]
        status: Option<TaskStatus>,
        /// New priority.
        #[arg(long)]
        priority: Option<Priority>,
        /// New comma-separated tags (replaces existing).
        #[arg(long, value_delimiter = ',')]
        tags: Option<Vec<String>>,
        /// Body content for the task. Accepts any text verbatim (backticks,
        /// em-dashes, other Unicode/punctuation included) and always takes
        /// precedence over stdin — stdin is never read once this is set.
        #[arg(long, conflicts_with = "clear_body")]
        body: Option<String>,
        /// Clear an existing body (replace it with an empty string).
        #[arg(long, conflicts_with = "body")]
        clear_body: bool,
        /// Git commit SHA to associate with this task.
        #[arg(long)]
        commit: Option<String>,
        /// Reason to record when parking this task blocked or closing/retiring it
        /// (e.g. why it is blocked, or why it was marked wont-fix). Preserved
        /// across later status changes.
        #[arg(long, conflicts_with = "clear_reason")]
        reason: Option<String>,
        /// Remove the recorded close reason from this task.
        #[arg(long, conflicts_with = "reason")]
        clear_reason: bool,
        /// Suppress interactive editor for body content.
        #[arg(long)]
        no_edit: bool,
    },
    /// Merge one or more duplicate tasks into a survivor.
    ///
    /// Unions the sources' tags into the survivor, appends each source body to
    /// the survivor under a `## Merged from task <slug>` heading, and closes
    /// every source `wont-fix` with a `superseded by task/<survivor>` pointer.
    /// Idempotent: re-running the same merge is a no-op.
    Merge {
        /// Survivor task slug (the task that absorbs the sources).
        survivor: String,
        /// Source task slug(s) to fold into the survivor (repeatable,
        /// comma-separated).
        #[arg(long = "from", value_delimiter = ',', required = true, num_args = 1..)]
        from: Vec<String>,
        /// Project the tasks belong to.
        #[arg(long)]
        project: Option<String>,
        /// Suppress interactive editor for body content.
        #[arg(long)]
        no_edit: bool,
    },
    /// List tasks.
    List {
        /// Project to list tasks for.
        #[arg(long)]
        project: Option<String>,
        /// Filter by status (open, in-progress, needs-review, reviewed, done, blocked, wont-fix, or all).
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

#[cfg(feature = "git")]
#[derive(Subcommand)]
pub(crate) enum ReviewCommand {
    /// List items in `needs-review` that are in scope for the current
    /// source-repo checkout.
    ///
    /// Scope is decided by *branch identity* first: an item is in scope when its
    /// stamped `review_branch` equals the current checkout's branch (keeping
    /// roadmaps exactly isolated). When the item carries no branch stamp
    /// (legacy / pre-stamp) or the current checkout has no resolvable branch
    /// (detached HEAD, a non-repo cwd, or git unavailable), it falls back to SHA
    /// reachability — keeping items whose stamped `review_sha` is reachable from
    /// HEAD, and failing open on any git error so work is never silently hidden.
    /// This is the shared source of truth for the review Stop hook and the
    /// `rdm-review` skill.
    Pending {
        /// Project to inspect.
        #[arg(long)]
        project: Option<String>,
    },
    /// Refresh `review_sha`/`review_branch` on every in-scope `needs-review`
    /// item to the current source-repo HEAD and branch.
    ///
    /// Run this after amending or rebasing a commit while an item is still
    /// `needs-review`: the original stamp would otherwise point at a now-dangling
    /// commit and the item could silently drop out of [`Pending`](Self::Pending)
    /// scope (via the SHA-reachability fallback). Scope matches `review pending`
    /// exactly, so it only ever touches items this checkout already owns. It is
    /// idempotent — items already stamped at the current HEAD/branch are left
    /// untouched (no plan-repo write). The review Stop hook calls this
    /// automatically before checking scope, so it normally runs transparently.
    Restamp {
        /// Project to inspect.
        #[arg(long)]
        project: Option<String>,
    },
    /// List phases parked as `blocked` — the escalation queue awaiting a
    /// human decision — with their recorded reasons.
    ///
    /// A blocked phase is one a dispatched run could not resolve on its own: an
    /// ambiguous acceptance criterion, an architectural decision with no clear
    /// default, an exhausted retry budget, or a hard external dependency. This
    /// surfaces them all in one place so they can be answered in a batch.
    Blocked {
        /// Project to inspect.
        #[arg(long)]
        project: Option<String>,
    },
    /// Start a new draft review of a roadmap, phase, or task.
    ///
    /// Stamps the plan-repo HEAD as the review's `created_commit`, so quote
    /// anchors added later are derived against the version of the target the
    /// reviewer is looking at.
    Start {
        /// The item under review: `roadmap/<slug>`,
        /// `phase/<roadmap-slug>/<stem-or-number>`, or `task/<slug>`.
        #[arg(long)]
        on: String,
        /// Review author (defaults to $RDM_REVIEW_AUTHOR, then $USER).
        #[arg(long)]
        author: Option<String>,
        /// Project the target belongs to.
        #[arg(long)]
        project: Option<String>,
        /// Initial summary body for the review.
        #[arg(long)]
        body: Option<String>,
        /// Suppress interactive editor for body content.
        #[arg(long)]
        no_edit: bool,
    },
    /// Append a comment to a draft review.
    ///
    /// With `--quote`, the quoted text is located in the comment's document
    /// as of the review's `created_commit` and a text-quote anchor
    /// (quote + ~32 chars of surrounding context) is derived automatically.
    /// Without `--quote`, the comment applies to the whole document.
    Comment {
        /// Id of the draft review to comment on.
        review_id: String,
        /// Exact text in the target's body the comment is anchored to.
        #[arg(long)]
        quote: Option<String>,
        /// Which occurrence of `--quote` to anchor to (1-based), when the
        /// quoted text appears more than once. Occurrences are counted
        /// without overlap (the quote "aa" occurs twice in "aaaa", not
        /// three times).
        #[arg(long, requires = "quote")]
        occurrence: Option<usize>,
        /// Scope the comment to one of the roadmap's phases
        /// (`phase/<stem-or-number>`). Only valid on roadmap reviews.
        #[arg(long)]
        doc: Option<String>,
        /// Comment text.
        #[arg(long)]
        body: Option<String>,
        /// Suppress interactive editor for body content.
        #[arg(long)]
        no_edit: bool,
        /// Project the review belongs to.
        #[arg(long)]
        project: Option<String>,
    },
    /// Submit a draft review with a verdict.
    ///
    /// After submission the comment structure is locked; only comment
    /// resolutions (status, applied commit, reply) may change.
    Submit {
        /// Id of the draft review to submit.
        review_id: String,
        /// Overall verdict (approve, request-changes, or comment).
        #[arg(long)]
        verdict: Verdict,
        /// Replace the review's summary body before submitting.
        #[arg(long)]
        body: Option<String>,
        /// Suppress interactive editor for body content.
        #[arg(long)]
        no_edit: bool,
        /// Project the review belongs to.
        #[arg(long)]
        project: Option<String>,
    },
    /// List document reviews, optionally filtered.
    List {
        /// Keep only reviews of this target: `roadmap/<slug>`,
        /// `phase/<roadmap-slug>/<stem-or-number>`, or `task/<slug>`.
        #[arg(long)]
        on: Option<String>,
        /// Keep only reviews in this state (draft, submitted, addressed,
        /// dismissed).
        #[arg(long)]
        state: Option<ReviewState>,
        /// Keep only reviews with this verdict (approve, request-changes,
        /// comment).
        #[arg(long)]
        verdict: Option<Verdict>,
        /// Keep only reviews by this author.
        #[arg(long)]
        author: Option<String>,
        /// Project to list reviews for.
        #[arg(long)]
        project: Option<String>,
    },
    /// Show a review: summary plus each comment with its anchor quote and
    /// resolution state (resolved / drifted / unresolved).
    Show {
        /// Id of the review to show.
        review_id: String,
        /// Suppress the summary and comment bodies (metadata only).
        #[arg(long)]
        no_body: bool,
        /// Project the review belongs to.
        #[arg(long)]
        project: Option<String>,
    },
    /// Update a review's state, or a single comment's resolution.
    ///
    /// Transitions are validated by the review lifecycle: `dismissed` closes
    /// a draft or submitted review without acting on it; `addressed`
    /// requires a submitted review whose comments are all resolved.
    Update {
        /// Id of the review to update.
        review_id: String,
        /// Transition the review to a terminal state.
        #[arg(long)]
        state: Option<ReviewTransitionArg>,
        /// Comment id to update (see `rdm review show` for ids).
        #[arg(long)]
        comment: Option<u32>,
        /// New resolution status for the comment (addressed or wont-fix).
        #[arg(long, requires = "comment")]
        status: Option<ReviewCommentStatus>,
        /// Commit SHA that addressed the comment.
        #[arg(long, requires = "comment")]
        applied_commit: Option<String>,
        /// Reply note recorded on the comment.
        #[arg(long, requires = "comment")]
        reply: Option<String>,
        /// Project the review belongs to.
        #[arg(long)]
        project: Option<String>,
    },
    /// Delete a review (drafts only, unless forced).
    Delete {
        /// Id of the review to delete.
        review_id: String,
        /// Delete even a submitted or terminal review.
        #[arg(long)]
        force: bool,
        /// Project the review belongs to.
        #[arg(long)]
        project: Option<String>,
    },
    /// The agent work queue: submitted reviews requesting changes
    /// (shorthand for `list --state submitted --verdict request-changes`).
    Requests {
        /// Project to list requests for.
        #[arg(long)]
        project: Option<String>,
    },
}

/// Terminal state transitions accepted by `rdm review update --state`.
///
/// `draft` and `submitted` are deliberately unrepresentable: submission goes
/// through `rdm review submit` (verdict-gated) and nothing returns to draft.
#[cfg(feature = "git")]
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum ReviewTransitionArg {
    /// Close the review as addressed (every comment must be resolved).
    Addressed,
    /// Dismiss the review without acting on it.
    Dismissed,
}

#[cfg(feature = "git")]
impl From<ReviewTransitionArg> for rdm_core::ops::reviews::ReviewTransition {
    fn from(arg: ReviewTransitionArg) -> Self {
        match arg {
            ReviewTransitionArg::Addressed => Self::Addressed,
            ReviewTransitionArg::Dismissed => Self::Dismissed,
        }
    }
}

#[cfg(feature = "git")]
#[derive(Subcommand)]
pub(crate) enum WorktreeCommand {
    /// Create (or reuse) a worktree for a plan item.
    ///
    /// The item is `<roadmap>/<phase-stem-or-number>`, `task/<slug>`, or a bare
    /// `<roadmap>` (one worktree on `roadmap/<slug>` shared by all its phases).
    Add {
        /// Plan item to key the worktree/branch to.
        item: String,
        /// Base ref to branch from (defaults to the invoking checkout's
        /// current branch, falling back to HEAD if detached).
        #[arg(long)]
        base: Option<String>,
        /// Project to resolve the item against.
        #[arg(long)]
        project: Option<String>,
    },
    /// List rdm-managed worktrees in the current project repo.
    List,
    /// Report the plan item the current checkout corresponds to.
    ///
    /// Detection prefers the rdm worktree marker; otherwise the item is inferred
    /// from the branch name (`phase/<roadmap>/<stem>`, `task/<slug>`, or
    /// `roadmap/<slug>`), so a
    /// hand-made worktree or the main checkout sitting on an item branch is also
    /// recognized. Prints "Not in an rdm worktree." (text) / `null` (JSON) and
    /// exits 0 when the checkout is on neither — e.g. the main checkout on
    /// `main`. The `rdm-do` skill uses this to decide whether to reuse the
    /// current worktree or create a new one.
    Current,
    /// Remove an rdm-managed worktree by item or path.
    Remove {
        /// Item (`<roadmap>/<phase-stem-or-number>`, `task/<slug>`, or
        /// `<roadmap>`) or worktree path.
        target: String,
        /// Also delete the worktree's branch.
        #[arg(long)]
        delete_branch: bool,
        /// Remove even if dirty (and force-delete the branch).
        #[arg(long)]
        force: bool,
        /// Project to resolve a phase-number item against.
        #[arg(long)]
        project: Option<String>,
    },
    /// Remove every worktree whose plan item is already `done`, in one pass.
    ///
    /// Enumerates rdm-managed worktrees, keeps only those whose item resolves to
    /// a `done` status, and removes them. Dirty worktrees are skipped unless
    /// `--force`; `--delete-branch` also deletes each pruned branch; `--dry-run`
    /// reports what would be removed without removing anything.
    Prune {
        /// Project to resolve worktree items against.
        #[arg(long)]
        project: Option<String>,
        /// Also delete each pruned worktree's branch.
        #[arg(long)]
        delete_branch: bool,
        /// Prune even dirty worktrees (and force-delete their branches).
        #[arg(long)]
        force: bool,
        /// Report what would be pruned without removing anything.
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum ConfigCommand {
    /// Get the resolved value of a config key.
    Get {
        /// Config key (e.g. default_project, default_format, remote.default,
        /// root, server.quick_filters).
        key: String,
    },
    /// Set a config key.
    Set {
        /// Config key to set.
        key: String,
        /// Value to set. For `server.quick_filters`, use the
        /// `Label:tag,Label2:tag2` form (e.g. `Bug:bug,Refactor:refactor`);
        /// an empty string clears all chips.
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
    Review,
}

impl From<ItemKindArg> for ItemKind {
    fn from(arg: ItemKindArg) -> Self {
        match arg {
            ItemKindArg::Roadmap => ItemKind::Roadmap,
            ItemKindArg::Phase => ItemKind::Phase,
            ItemKindArg::Task => ItemKind::Task,
            ItemKindArg::Review => ItemKind::Review,
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
