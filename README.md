<p align="center">
  <img src="logo-blue.png" alt="rdm logo" width="300">
</p>

[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-blue.svg)](https://www.gnu.org/licenses/gpl-3.0)

A zero-dependency CLI for managing project roadmaps, phases, and tasks across multiple projects from a central plan repository.

`rdm` separates the **tool** (this repo — a Rust binary) from the **plan repo** (a git-managed directory of markdown files). The plan repo is where your roadmaps and tasks live; `rdm` is how you read and write them.

## Quick Start

```bash
# Point rdm at your plan repo
export RDM_ROOT=~/Projects/my-plans

# Initialize the plan repo
rdm init

# Create a project
rdm project create fbm --title "Fantasy Baseball Manager"

# Create a roadmap with phases
rdm roadmap create two-way-players --project fbm --title "Two-Way Player Identity"
rdm phase create two-way-players/core-valuation --project fbm --title "Core valuation layer"
rdm phase create two-way-players/keeper-service --project fbm --title "Keeper service threading"

# Track progress
rdm phase update two-way-players/core-valuation --project fbm --status done
rdm roadmap show two-way-players --project fbm

# One-off work items
rdm task create fix-barrel-nulls --project fbm --title "Fix barrel column NULL for 2024" --priority high
rdm task update fix-barrel-nulls --project fbm --status done

# Regenerate the index (also runs automatically after mutations)
rdm index

# See everything at a glance
rdm list --project fbm
rdm list --all
```

## Plan Repo Structure

`rdm init` creates a git-managed directory of markdown files — roadmaps, phases, tasks, and auto-generated indexes. See [docs/file-formats.md](docs/file-formats.md) for the full directory layout, file format reference, and field descriptions.

## Agent Integration

`rdm` is designed to be used by AI coding agents. Instead of granting the agent filesystem access to your plan repo, you allowlist the `rdm` binary and the agent reads/writes roadmaps through the CLI.

```bash
# Generate CLAUDE.md instructions for a target project
rdm agent-config claude --project fbm > ~/Projects/fbm/.claude/rdm.md

# Generate skill definitions
rdm agent-config claude --skills --project fbm --out ~/Projects/fbm/.claude/skills/
```

For agents that support MCP, see the [MCP Server](#mcp-server) section for a more direct integration.

The generated agent config tells the agent:
- How to read roadmaps and tasks via `rdm show`, `rdm list`
- How to update phase status via `rdm phase update`
- How to create tasks for discovered bugs via `rdm task create`
- The workflow for implementing roadmap phases

## MCP Server

rdm includes a [Model Context Protocol](https://modelcontextprotocol.io/) server that exposes plan repo operations as MCP tools, enabling direct integration with AI agents that support MCP.

```bash
# Start the MCP server (stdio transport)
rdm mcp

# With an explicit plan repo root
rdm --root ~/Projects/my-plans mcp
```

### Configuration

Generate a `.mcp.json` configuration file for MCP-aware clients:

```bash
# Print config to stdout
rdm agent-config --mcp

# Write to a directory
rdm agent-config --mcp --out ~/Projects/my-app
# → writes ~/Projects/my-app/.mcp.json
```

### Available Tools

| Tool | Description |
|------|-------------|
| `rdm_project_list` | List all projects |
| `rdm_roadmap_list` | List roadmaps with progress |
| `rdm_roadmap_show` | Show roadmap details with phases |
| `rdm_roadmap_create` | Create a new roadmap |
| `rdm_phase_list` | List phases in a roadmap |
| `rdm_phase_show` | Show phase details |
| `rdm_phase_create` | Create a new phase |
| `rdm_phase_update` | Update phase status or content |
| `rdm_task_list` | List tasks with optional filters |
| `rdm_task_show` | Show task details |
| `rdm_task_create` | Create a new task |
| `rdm_task_update` | Update task status or fields |
| `rdm_task_promote` | Promote a task to a roadmap |
| `rdm_search` | Fuzzy search across all items |

## REST API

For integrations beyond the CLI:

```bash
# Start the API server
rdm serve --port 8400

# Endpoints mirror the CLI
GET  /projects
GET  /projects/:project/roadmaps
GET  /projects/:project/roadmaps/:roadmap
PATCH /projects/:project/roadmaps/:roadmap/phases/:phase
GET  /projects/:project/tasks
POST /projects/:project/tasks
PATCH /projects/:project/tasks/:task
GET  /index
```

## Architecture

```
rdm (this repo)
├── rdm-core/       # library: data model, parsing, file I/O, index generation
├── rdm-cli/        # binary: CLI porcelain over rdm-core
├── rdm-mcp/        # library: MCP server over rdm-core (stdio transport)
└── rdm-server/     # binary: REST API over rdm-core
```

The core library is the source of truth. CLI and server are thin layers that parse arguments/requests, call core, and format output. This makes it straightforward to add new interfaces (TUI, MCP server, etc.) without duplicating logic.

## Installation

```bash
# Homebrew (macOS)
brew install edpaget/rdm/rdm

# From source
cargo install --path rdm-cli
```

## License

This project is licensed under the [GNU General Public License v3.0](LICENSE).
