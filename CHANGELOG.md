# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.6.2] - 2026-04-12
### Added

- `rdm bootstrap --plan-repo <url> [--path <dir>] [--branch <name>] [--init]` clones a plan repo into a target directory (defaulting to `$XDG_DATA_HOME/rdm/plan-repo`) and fast-forwards it on subsequent runs. Designed for Claude Code web session-start hooks and other sandbox bootstrap scripts that need an idempotent "get me a plan repo" command.

### Changed

- Upgraded rmcp dependency from 0.16 to 1.4
- `GitStore::clone_remote` now takes an optional `branch: Option<&str>` argument to clone a specific branch via `git clone --branch`

## [0.6.1] - 2026-03-31

## [0.6.0] - 2026-03-31

### Added

- Roadmap priority support in REST API: list/detail responses include priority, create accepts optional priority, new PATCH endpoint for updating priority, `?sort=priority` and `?priority=<level>` query params on list
- Roadmap priority support in MCP tools: `rdm_roadmap_create` accepts optional priority, `rdm_roadmap_list` supports sort and priority filter, new `rdm_roadmap_update` tool for setting/clearing priority and body
- Roadmap priority badges in HTML views: list page shows a Priority column and detail page displays priority next to status

## [0.5.0] - 2026-03-26

### Added

- `rdm_create_project` MCP tool to create new projects from within MCP clients
- Search results are now capped by relevance score, filtering out low-quality matches
- Optional `priority` field on roadmaps (`low`, `medium`, `high`, `critical`) — reuses the existing priority model from tasks
- `rdm roadmap create --priority <level>` and `rdm roadmap update` command for setting/clearing priority via CLI
- `rdm roadmap list --sort priority` sorts roadmaps by priority descending; `--priority <level>` filters by priority level
- `rdm roadmap show` displays priority when set

### Fixed

- `default_branch` is now recognized as a valid config key for `rdm config get` and `rdm config set`
- MCP server logs errors when store construction silently falls back instead of swallowing them

## [0.4.0] - 2026-03-24
### Added

- `rdm agent-config --user` writes agent config to the user-level config directory (e.g. `~/.claude/`) instead of a project directory, enabling global agent integration

## [0.3.1] - 2026-03-21

### Added

- `rdm agent-config --mcp` now generates MCP-oriented agent instructions referencing MCP tool names instead of CLI commands
- `rdm agent-config --mcp --skills` generates MCP-aware Claude Code skills that use `mcp__rdm__*` tools in `allowed-tools`
- When `--mcp --out` is used, `.mcp.json` is written alongside the instructions or skills
- MCP agent instructions include a Searching section with `rdm_search` tool

### Changed

- `--mcp` flag is no longer mutually exclusive with `--skills`; it is now a modifier that switches output to MCP tool references
- Restructured README to lead with installation and quick start, added "Core Workflow: Plan, Implement, Done" section showcasing the plan-implement-done cycle, and moved reference material (architecture, REST API endpoints) to dedicated docs

## [0.3.0] - 2026-03-21

### Added

- `rdm hook post-commit` subcommand: parses `Done:` directives from HEAD on the default branch, enabling automatic phase/task completion for fast-forward merges
- `rdm hook install` now installs both `post-merge` and `post-commit` hooks
- `rdm hook uninstall` now removes both hooks
- `default_branch` config key in both repo (`rdm.toml`) and global config — sets the branch name used by the post-commit hook (defaults to `main`)
- `current_branch_at()` public function in `rdm-store-git` for querying the current branch name
- `rdm_init` MCP tool to initialize a plan repo from within an MCP client (e.g. Cursor); accepts an optional `default_project` parameter to create a project during init
- `auto_init` global config option — when `true`, the MCP server automatically initializes the plan repo on first tool call if not already set up
- Improved MCP error messages for uninitialized repos: errors now mention the `rdm_init` tool instead of the CLI `rdm init` command

## [0.2.0] - 2026-03-20

### Added

- `rdm init --remote <url>` to clone an existing shared plan repo instead of creating an empty one; sets `remote.default = "origin"` and validates the cloned repo has `rdm.toml`
- `GitStore::clone_remote(url, root)` static constructor for cloning remote git repositories
- `rdm init --default-project <name>` flag to set `default_project` in repo config and create the project directory
- `rdm init --default-format <fmt>` flag to set `default_format` in global config
- `rdm init` with `--stage` persists `stage = true` to repo config
- `rdm init` now creates parent directories recursively, creates the global config file, and prints a summary with paths, settings, and next steps
- `PlanRepo::init_with_config()` in rdm-core for initializing with a custom `Config`

- `rdm config get <key>` command to view a config value with its source (CLI flag, env var, repo config, global config, or default)
- `rdm config set <key> <value> [--global]` command to set config values in repo or global config with validation
- `rdm config list` command to display all known config keys with resolved values and sources
- `default_format` config key in both repo (`rdm.toml`) and global config — sets the default output format (human, json, table, markdown)
- Format resolution chain: `--format` flag > `RDM_FORMAT` env var > `default_format` in config > `human`
- `InvalidConfigValue` error variant in rdm-core with actionable error messages
- `ConfigSource` and `ResolvedValue<T>` types in rdm-core for tracking where config values come from
- Config validation: invalid `default_format` values are rejected at parse time with clear error messages

### Changed

- `--format` flag no longer defaults to `human` at the clap level; the default is now resolved through the config hierarchy, allowing `default_format` in config files to take effect

### Fixed

- `--root` and `RDM_ROOT` now expand `~` to the home directory and resolve `.`/`..` segments, fixing silent failures when paths are set in config files like `.mise.toml` where the shell doesn't perform tilde expansion

### Added

- `Done: task/<slug>` directive support in post-merge hook — tasks can now be marked done via commit messages, just like phases
- `commit` and `completed` fields on the Task model — automatically set when a task transitions to done
- `--commit` flag on `rdm task update` for manually associating a commit SHA with a task

- XDG-compliant default paths: `rdm` now works out of the box without `RDM_ROOT` by resolving a plan repo root from `~/.config/rdm/config.toml` (global config) or `$XDG_DATA_HOME/rdm` (default data dir)
- `GlobalConfig` struct in rdm-core for parsing global config files with `root`, `default_project`, `stage`, and `remote` fields
- Config merging: CLI flags > env vars > repo config (`rdm.toml`) > global config (`~/.config/rdm/config.toml`) for project, staging, and remote resolution
- `rdm-review` skill for independent post-implementation review with parallel AC compliance and code quality agents
- `skill_review()` generator function in `rdm-core::agent_config` for generating the review skill via `rdm agent-config --skills`
- `rdm-document` Claude Code skill for generating user documentation from completed roadmaps using phase descriptions and commit SHAs
- `rdm agent-config --skills` now generates the `rdm-document` skill alongside the existing three
- `Done:` commit message convention documented in generated agent configs, `rdm-implement` and `rdm-tasks` skills
- `rdm hook install` / `rdm hook uninstall` to manage the post-merge git hook in the plan repo
- `rdm hook post-merge` subcommand: parses `Done: roadmap/phase` directives from the HEAD commit and marks matching phases done with the commit SHA
- `update_phase` is now idempotent for Done→Done transitions: re-marking a done phase with a new commit SHA updates the SHA while preserving the completed date; omitting `--commit` is a safe no-op
- `HeadCommitInfo`, `head_commit_info()`, `git_dir()`, and `default_branch_name()` on `GitStore`
- `rdm_core::hook` module with `DoneDirective` and `parse_done_directives()` for parsing `Done:` directives from commit messages

### Removed

- `.githooks/post-merge` bash script (replaced by `rdm hook` subcommands)
- `--commit <sha>` flag on `rdm phase update` to associate a git commit SHA with phase completion (requires `--status done`)
- `commit` field in phase frontmatter, phase detail display, and JSON output

### Added

- Merge conflict detection during `rdm remote pull` with rdm-aware item context (roadmap, phase, task classification)
- `rdm conflicts` command to list unresolved merge conflicts with item context
- `rdm resolve <file>` command to mark conflicts resolved and auto-complete merge with INDEX.md regeneration
- `rdm discard --force` now aborts an in-progress merge before discarding changes
- `rdm status` shows merge-in-progress state with conflict count
- `MergeConflictResult`, `PullOutcome`, `ResolveResult` structs and `git_list_unmerged`, `git_is_merge_in_progress`, `git_merge_abort`, `git_resolve_conflict` methods on `GitStore`
- `MergeConflict`, `NoMergeInProgress`, `NotConflicted` error variants in rdm-core
- `classify_path` function and `ConflictItem`/`ConflictItemKind` types in new `rdm-core::conflict` module

### Changed

- `rdm remote pull` now attempts a real merge when branches have diverged instead of rejecting with `BranchesDiverged`; non-conflicting concurrent edits merge cleanly

- Top-level `INDEX.md` now shows a lightweight summary table linking to each project's `INDEX.md` instead of inlining all project details

### Added

- `rdm remote push [name]` command to push local commits to a remote (supports `--force`)
- `rdm remote pull [name]` command to fetch and fast-forward merge from a remote, with automatic INDEX.md regeneration
- `PushResult`, `PullResult` structs and `git_push`/`git_pull` methods on `GitStore`
- `PushRejected` and `BranchesDiverged` error variants with actionable messages
- `rdm remote add <name> <url>` command to register a git remote on the plan repo
- `rdm remote remove <name>` command to remove a git remote
- `rdm remote list` command to display all configured remotes with their URLs
- `rdm remote fetch [name]` command to fetch from a git remote (defaults to `remote.default` in `rdm.toml`)
- `rdm status --fetch` flag to fetch from the default remote before showing sync status
- Sync status display on `rdm status` showing ahead/behind commit counts relative to the default remote's tracking branch
- `SyncStatus` struct and `git_fetch`/`git_sync_status` methods on `GitStore` for programmatic fetch and ahead/behind detection
- `RemoteInfo` struct and `git_remote_add/remove/list` methods on `GitStore` for programmatic remote management
- `RemoteConfig` struct in `rdm-core::config` with `[remote]` section support in `rdm.toml`
- `RemoteNotFound` and `DuplicateRemote` error variants in rdm-core
- `format_top_level_index` function in `rdm-core::display` for the new summary-style root index
- Per-project `INDEX.md` files at `projects/<name>/INDEX.md` with relative links, generated alongside the root index
- `format_project_index` function in `rdm-core::display` for standalone per-project index rendering
- `PlanRepo::generate_project_index` method and `project_index_path` path builder in `rdm-core`
- Web UI hides completed roadmaps by default; toggle link (`?show_completed=true`) reveals them
- `rdm tree` command — hierarchical overview of a project's roadmaps, phases, and tasks with statuses (human, JSON, and Markdown formats)
- `rdm-core::tree` module with `TreeNode` types, `build_tree()`, and formatting functions
- Navigation hints in `roadmap show` output — shows how to drill into individual phases
- Prev/next phase navigation in `phase show` output — human and Markdown formats show commands for adjacent phases; JSON includes `prev_phase`/`next_phase` fields
- `rdm describe` command for model introspection — lists entity types or shows fields for a specific entity (project, roadmap, phase, task)
- `rdm-core::describe` module with `Describe` trait, `EntityInfo`/`FieldInfo` types, and formatting functions
- End-to-end agent workflow integration tests validating the full project → roadmap → phase → body discovery path, JSON parity, schema coverage, and programmatic navigation
- Drift tests that compare serde keys against `Describe` field names to catch struct/describe mismatches at compile time
- `project show` command with `--format human/json/markdown` support
- `--format json` support on all read commands: `roadmap list/show`, `phase list/show`, `task list/show`, `project list/show`, `search`, and top-level `list`
- `rdm-core::json` module with serializable JSON output structs (`RoadmapJson`, `PhaseJson`, `TaskJson`, `ProjectJson`, `SearchResultJson`, and summary variants) for stable machine-readable output

### Changed

- `roadmap show --format json` now nests phase summaries (without body) instead of full phase objects; use `phase show --format json` for full phase content
- `search --format json` now outputs via `SearchResultJson` types from the `json` module for a consistent contract
- `--mcp` flag on `rdm agent-config` to generate `.mcp.json` configuration for MCP-aware clients
- `generate_mcp_config` function in `rdm-core::agent_config` for programmatic MCP config generation
- End-to-end MCP workflow integration test covering the full agent lifecycle
- MCP Server section in README with tool table, config generation, and usage instructions
- `--format markdown` option for clean Markdown output on list, show, and search commands
- `--format table` option for pretty terminal tables on list and search commands (powered by `tabled` crate)
- Global `--format` flag on all read commands (defaults to `human`; `text` accepted as alias for backward compatibility)
- 6 mutation MCP tools: `rdm_roadmap_create`, `rdm_phase_create`, `rdm_phase_update`, `rdm_task_create`, `rdm_task_update`, `rdm_task_promote`
- 8 read-only MCP tools: `rdm_project_list`, `rdm_roadmap_list`, `rdm_roadmap_show`, `rdm_phase_list`, `rdm_phase_show`, `rdm_task_list`, `rdm_task_show`, `rdm_search`
- `rdm roadmap archive <slug>` command with `--force` flag to archive completed roadmaps
- `rdm roadmap list --archived` flag to show archived roadmaps
- `rdm roadmap unarchive <slug>` command to restore archived roadmaps to active status
- `RoadmapHasIncompletePhases` error variant in rdm-core for archive validation
- `rdm roadmap split <slug> --phases <stems-or-numbers>... --into <new-slug> --title "Title"` command to extract selected phases from an existing roadmap into a new one, with automatic renumbering and optional `--depends-on` flag
- `PlanRepo::split_roadmap` method in rdm-core for programmatic roadmap splitting
- `InvalidPhaseSelection` error variant in rdm-core for phase selection validation
- Dark mode support for the web UI with toggle button and system-preference detection
- Theme preference persists to `localStorage` across sessions
- Computed overall status badge (done / in-progress / not-started) on roadmap list and detail pages
- Last-changed timestamp on roadmap list and detail pages, derived from file modification times
- `--stage` global flag and `RDM_STAGE` env var for deferred git commits — files are written to disk but the git commit is skipped until explicitly requested
- `rdm status` command to show uncommitted changes in the plan repo
- `rdm commit -m "message"` command for explicit git commits (auto-generates message if `-m` is omitted)
- `rdm discard --force` command to reset working directory to HEAD state
- `stage` option in `rdm.toml` for persistent staging mode
- `staging_mode` on `GitStore` with `git_commit()`, `git_status()`, and `git_discard()` public methods
- `FileChange` enum and `FileStatus` struct in `rdm-store-git` for working directory status reporting
- Uncommitted changes hint on read-only commands (list, show, search) when staging mode is active
- `rdm-store-git` crate — git-backed Store with automatic commits via gitoxide; every `commit()` builds a tree from the working directory and creates a git commit with an auto-generated message
- `git` feature flag on `rdm-cli` (default-on) — enables `GitStore` for automatic git commits on all plan repo mutations
- `Error::Git(String)` variant in rdm-core for git-specific errors
- `rdm-store-fs` crate: filesystem-backed `Store` with in-memory staging — writes buffer in memory, `commit()` flushes to disk using write-to-temp + rename for best-effort atomicity, `discard()` drops the buffer
- `PlanRepo` mutation methods now auto-commit staged changes, so callers don't need explicit `commit()` calls
- `rdm mcp` subcommand: stdio MCP server (scaffold, no tools yet)
- `mcp` feature flag in rdm-cli (default-enabled)

### Changed

- Refactored all inline CSS colors in `base.html` to use CSS custom properties
- Bump `headers-accept` from 0.1 to 0.3
- Bump `mediatype` from 0.19 to 0.21

## [0.1.1] - 2026-03-18

### Added

- Homebrew tap (`edpaget/homebrew-rdm`) with auto-updated formula on release via cargo-dist
- `sign-release.yml` workflow: Sigstore cosign keyless signing of release artifacts with verification instructions appended to GitHub Release notes
- `prepare-release.yml` workflow: one-click version bump, changelog update, commit, tag, and push via `workflow_dispatch`
- cargo-dist configuration for automated binary releases (`rdm` binary for `aarch64-apple-darwin`)
- GitHub Actions release workflow (`.github/workflows/release.yml`) triggered by version tags
- `[profile.dist]` with thin LTO for optimized release builds

### Changed

- Workspace version centralized in root `Cargo.toml`; all crates now use `version.workspace = true`
- Rust version bumped from 1.87 to 1.94
- `repository` field added to workspace package metadata

### Changed

- `FsStore` moved from `rdm-core::store::FsStore` to `rdm_store_fs::FsStore`; import path updated in `rdm-cli` and `rdm-server`

- `rdm phase update` no longer requires `--status`; omitting it preserves the existing status, enabling content-only updates
- `PlanRepo::update_phase` now accepts `Option<PhaseStatus>` instead of `PhaseStatus`
- Server `PATCH /phases/:phase` endpoint accepts optional `status` field in request body

### Added

- `rdm roadmap delete <slug> --force` command to delete a roadmap and all its phases, with automatic cleanup of dependency references from other roadmaps
- `PlanRepo::delete_roadmap` method in rdm-core for programmatic roadmap deletion
- `rdm-implement` and `rdm-tasks` skills now use plan mode (`EnterPlanMode`/`ExitPlanMode`) for a deliberate plan-then-execute workflow with explicit user approval before finalizing
- Generated skills from `rdm agent-config --skills` include the same plan mode workflow
- `rdm roadmap depend <slug> --on <other>` to add a dependency between roadmaps
- `rdm roadmap undepend <slug> --on <other>` to remove a dependency
- `rdm roadmap deps` to display the dependency graph for all roadmaps in a project
- Circular dependency detection rejects cycles with a clear error message
- `CyclicDependency` error variant in rdm-core for dependency cycle detection
- `add_dependency`, `remove_dependency`, and `dependency_graph` methods on `PlanRepo`
- `format_dependency_graph` display function in rdm-core
- `--skills` flag on `rdm agent-config claude` to generate Claude Code skill files (`rdm-roadmap`, `rdm-implement`, `rdm-tasks`) as reusable slash commands
- `rdm-core::agent_config::SkillFile`, `SkillOptions`, and `generate_skills` public API for skill generation
- `rdm agent-config` command to generate AI agent instruction files for Claude Code, Cursor, GitHub Copilot, and AGENTS.md
- Supports `--project` to embed project name in examples and `--out` to write to platform-conventional file paths
- `rdm-core::agent_config` module with `Platform` enum, `AgentConfigOptions`, and `generate_agent_config` function
- "Planning workflow" section in agent config output teaching agents when and how to use rdm commands
- "Status transitions" section documenting valid phase and task status transitions
- `--principles-file` flag on `rdm agent-config` to reference a project principles file in generated instructions
- CLAUDE.md "Searching the plan" subsection documenting `rdm search` usage for AI agents
- `rdm search <query>` CLI command with fuzzy matching across roadmaps, phases, and tasks
- Search flags: `--type` (roadmap|phase|task), `--status`, `--project`, `--limit`, `--format` (text|json)
- Text output displays ranked table with type, title, identifier, and snippet columns
- JSON output (`--format json`) for agent/programmatic consumption
- `format_search_results()` display function in rdm-core for text table formatting
- `Serialize` derives on `SearchResult` and `ItemKind` for JSON serialization
- `search` module in rdm-core: fuzzy search across roadmaps, phases, and tasks by title and body content using `nucleo-matcher`
- `SearchFilter` for narrowing results by item kind, project, or status
- `SearchResult` with kind, identifier, project, title, snippet, and score
- `rdm serve` command with `--port`, `--bind`, and `--root` options
- Graceful shutdown on SIGINT/SIGTERM for `rdm serve` and `rdm-server` binary
- `server` feature flag on `rdm-cli` (enabled by default; disable with `--no-default-features`)
- Integration tests for all server endpoints using reqwest against real TCP server
- Accessibility smoke tests verifying WCAG landmark structure, heading hierarchy, and ARIA attributes
- POST endpoints for creating projects, roadmaps, phases, and tasks (201 Created + Location header)
- PATCH endpoints for updating phase status and task fields (status, priority, tags, body)
- POST endpoint for promoting tasks to roadmaps (`/projects/{project}/tasks/{task}/promote`)
- Automatic index regeneration after all write operations
- Content negotiation for write responses: HAL+JSON returns resource, HTML returns 303 See Other redirect
- 422 Unprocessable Content for invalid request bodies (RFC 9457 Problem Details format)
- `hal_created_response` and `see_other_response` helpers in `rdm-server::extract`
- `validation_error` and `json_rejection_response` helpers in `rdm-server::error`
- HTML rendering for all endpoints with content negotiation: browsers get accessible HTML pages, API clients get HAL+JSON
- WCAG 2.1 AA accessibility: skip-to-content link, breadcrumb navigation with `aria-label` and `aria-current`, proper `<th scope>`, status conveyed by text (not color alone), focus outlines, sufficient color contrast
- Markdown-to-HTML rendering for phase and task body content using pulldown-cmark (raw HTML stripped)
- Format-aware error pages: HTML requests get styled error pages, HAL+JSON requests get RFC 9457 Problem Details
- Askama compile-time templates for all pages: index, roadmaps, roadmap detail, phase detail, task list, task detail, error
- Read-only HAL+JSON endpoints: `GET /` (root with project links), `GET /projects`, `GET /projects/:project/roadmaps`, `GET /projects/:project/roadmaps/:roadmap` (with embedded phases), `GET /projects/:project/roadmaps/:roadmap/phases/:phase` (with prev/next sibling links), `GET /projects/:project/tasks` (with `?status=`, `?priority=`, `?tag=` filters), `GET /projects/:project/tasks/:task`
- `load_project()` method on `PlanRepo` for loading project documents
- HAL+JSON response helpers (`require_hal_json`, `hal_response`) in `rdm-server::extract`
- Server foundation: `rdm-server` binary with axum, health check endpoint (`GET /healthz`), and shared `AppState`
- HAL (Hypertext Application Language) response types in `rdm-core`: `HalLink` and `HalResource<T>` with builder API
- RFC 9457 Problem Details type in `rdm-core` with mappings from all `rdm-core::Error` variants
- Content negotiation extractor parsing `Accept` header for `application/hal+json` and `text/html` (defaults to HTML)
- `AppError` wrapper in `rdm-server` converting core errors to Problem Details HTTP responses
- `phase remove` command to delete a phase from a roadmap (accepts stem or number)
- Interactive `$EDITOR` fallback when no `--body` or stdin is provided (checks `$VISUAL`, then `$EDITOR`, then `vi`)
- `--no-edit` flag on all `create` and `update` commands to suppress interactive editor
- `--body` flag on all `create` and `update` commands for roadmaps, phases, and tasks
- Piped stdin support: body content can be provided via stdin (e.g., `cat notes.md | rdm task create ...`)
- `rdm roadmap show` now displays document body content after the phase table
- `--no-body` flag on `roadmap show`, `phase show`, and `task show` to suppress body output
- `RDM_PROJECT` environment variable for session-level default project (resolution order: `--project` flag > `RDM_PROJECT` env var > `default_project` in `rdm.toml`)
- Body parameter on core create and update functions for roadmaps, phases, and tasks
- `rdm roadmap list --project P` command to list all roadmaps with phase progress
- `rdm index` command to generate `INDEX.md` from current repo state
- `PlanRepo::generate_index` in rdm-core for full index generation (projects, roadmaps with progress, tasks sorted by priority)
- `format_index` display function with `ProjectIndex` and `RoadmapIndexEntry` structs
- `--no-index` global flag to suppress automatic INDEX.md regeneration after mutations
- Auto-regenerate INDEX.md after all mutation commands (project/roadmap/phase/task create, phase/task update, promote)
- `Ord`/`PartialOrd` derive on `Priority` enum (Low < Medium < High < Critical)
- Integration tests for index generation, idempotency, sorting, dependency graphs, auto-index, and `--no-index`

### Removed

- `require_hal_json()` guard — all endpoints now support both HTML and HAL+JSON via content negotiation

### Changed

- `load_roadmap` and `load_task` now return `RoadmapNotFound`/`TaskNotFound` (404) instead of `Io` error (500) when the resource file does not exist
- `task list --status` now uses `TaskStatusFilter` enum for proper clap validation instead of raw string
- `promote` preserves task metadata (priority, created date, tags) in the roadmap body
- `list_tasks` returns `ProjectNotFound` for nonexistent projects instead of an empty list

### Added

- `TaskStatusFilter` type with `Display`/`FromStr` for type-safe status filtering (accepts `all` or any `TaskStatus`)
- `rdm task create`, `rdm task show`, `rdm task update`, and `rdm task list` CLI commands
- `rdm promote` command to convert a task into a roadmap with an initial phase
- `task list` defaults to showing `open` + `in-progress` tasks; `--status all` shows everything
- `task list` supports `--status`, `--priority`, and `--tag` filters
- `PlanRepo::create_task`, `list_tasks`, `update_task`, `promote_task` in rdm-core
- `Display` and `FromStr` impls for `TaskStatus` and `Priority` (enables CLI arg parsing via clap)
- `format_task_detail` and `format_task_list` display functions in rdm-core
- `TaskNotFound` error variant in rdm-core
- Integration tests for all task CLI commands and promote
- `rdm phase list` command to show phases in a roadmap with number, title, status, and stem
- Phase commands (`phase show`, `phase update`) accept phase number as alternative to stem
- `rdm project create` and `rdm project list` CLI commands
- `rdm roadmap create` and `rdm roadmap show` CLI commands
- `rdm phase create`, `rdm phase show`, and `rdm phase update` CLI commands
- `rdm list` command with `--project` and `--all` flags for roadmap progress summaries
- Project resolution: `--project` flag > `default_project` in `rdm.toml` > actionable error
- `PlanRepo::create_project`, `list_projects` for project management
- `PlanRepo::create_roadmap`, `list_roadmaps` for roadmap management
- `PlanRepo::create_phase`, `list_phases`, `update_phase` for phase management
- Auto-numbering for phases (next available number) with explicit `--number` override
- Auto-set `completed` date when phase status transitions to `Done`; auto-clear on non-`Done`
- `Display` and `FromStr` impls for `PhaseStatus` (enables `--status` CLI arg via clap)
- `rdm-core::display` module with `format_roadmap_summary`, `format_phase_detail`, `format_roadmap_list`
- Error variants: `RoadmapNotFound`, `PhaseNotFound`, `DuplicateSlug`, `ProjectNotSpecified`
- Integration tests for all new CLI commands (`cli_project`, `cli_roadmap`, `cli_phase`, `cli_list`, `cli_project_resolution`)
- Cargo workspace with `rdm-core`, `rdm-cli`, and `rdm-server` crates
- Data model types: `PhaseStatus`, `TaskStatus`, `Priority`, `Phase`, `Task`, `Roadmap`, `Project`
- Markdown frontmatter parsing and rendering (`split_frontmatter`, `join_frontmatter`)
- Generic `Document<T>` wrapper with `parse()` and `render()` methods
- Plan repo configuration (`Config` struct, `rdm.toml` parsing)
- `PlanRepo` with path builders, load/write operations for roadmaps, phases, and tasks
- `PlanRepo::load_config` to read and parse `rdm.toml` from an opened repo
- `PlanRepo::init` to initialize a new plan repo with `rdm.toml`, `projects/`, and `INDEX.md`
- `rdm init` CLI command with `--root` flag and `RDM_ROOT` env var support
- Hand-written error types in `rdm-core` with `Display`/`Error` impls
- `rdm-server` stub binary

### Changed

- `create_project` now returns `Document<Project>` for consistency with other create methods
- `Config::to_toml` now returns `crate::error::Result` instead of leaking `toml::ser::Error`

### Fixed

- `Config::from_toml` now returns `crate::error::Error` instead of leaking `toml::de::Error`
- `rdm list` now propagates phase-loading errors instead of silently swallowing them
- CLI integration tests use `tempfile::TempDir` instead of `.tmp/` in project root
