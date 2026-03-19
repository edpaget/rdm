# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- Web UI hides completed roadmaps by default; toggle link (`?show_completed=true`) reveals them
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
