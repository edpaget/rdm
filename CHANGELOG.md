# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

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
