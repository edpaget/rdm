# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- Cargo workspace with `rdm-core`, `rdm-cli`, and `rdm-server` crates
- Data model types: `PhaseStatus`, `TaskStatus`, `Priority`, `Phase`, `Task`, `Roadmap`
- Markdown frontmatter parsing and rendering (`split_frontmatter`, `join_frontmatter`)
- Generic `Document<T>` wrapper with `parse()` and `render()` methods
- Plan repo configuration (`Config` struct, `rdm.toml` parsing)
- `PlanRepo` with path builders, load/write operations for roadmaps, phases, and tasks
- `PlanRepo::init` to initialize a new plan repo with `rdm.toml`, `projects/`, and `INDEX.md`
- `rdm init` CLI command with `--root` flag and `RDM_ROOT` env var support
- Hand-written error types in `rdm-core` with `Display`/`Error` impls
- `rdm-server` stub binary
