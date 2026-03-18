# CLAUDE.md

## Project Overview

rdm is a Rust CLI for managing project roadmaps, phases, and tasks. "Zero-dependency" means users only need the compiled binary — no runtime dependencies, interpreters, or external tools. Cargo crate dependencies are fine. It separates the **tool** (this repo) from the **plan repo** (a git-managed directory of markdown files).

### Architecture

```
rdm-core/       # library: data model, parsing, file I/O, index generation
rdm-cli/        # binary: CLI porcelain over rdm-core
rdm-server/     # binary: REST API over rdm-core
```

Core is the source of truth. CLI and server are thin layers. New interfaces (TUI, MCP server) should call core, not duplicate logic.

### Key Concepts

- **Plan repo**: a git-managed directory (`RDM_ROOT`) containing markdown files for roadmaps and tasks
- **INDEX.md**: auto-generated from individual files — never edited by hand
- **Roadmaps** contain ordered **phases** (not-started | in-progress | done | blocked)
- **Tasks** are standalone work items (open | in-progress | done | wont-fix)
- Agent integration: `rdm agent-config` generates config for AI agents to interact via CLI

## Development Practices

### Commits

Use [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/). Format:

```
<type>(<scope>): <description>

[optional body]

[optional footer(s)]
```

Types: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`.
Scopes: `core`, `cli`, `server`, or omit for cross-cutting changes.

### TDD

Write tests **before** implementation code:

1. Write a failing test that describes the desired behavior
2. Write the minimal code to make the test pass
3. Refactor while keeping tests green

Run tests with `cargo nextest run`. Run specific crate tests with `cargo nextest run -p rdm-core`, etc. Use `cargo watch -x 'nextest run'` for continuous testing during development.

### Changelog

Maintain a `CHANGELOG.md` following [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) format:

- Keep an `[Unreleased]` section at the top for pending changes
- Categories: Added, Changed, Deprecated, Removed, Fixed, Security
- Move entries from Unreleased to a versioned section on release
- Update the changelog with every user-facing change

### Public API Docs

`rdm-core` must have `#![warn(missing_docs)]`. All public types and functions in the core library require doc comments. Use `///` for items and `//!` for module-level docs. Content is Markdown.

Include these rustdoc sections where applicable:

- **`# Errors`** — required on any function returning `Result`. List each error variant and when it occurs.
- **`# Panics`** — required if the function can panic. Describe the conditions.
- **`# Examples`** — encouraged for public API entry points. Examples are compiled and run by `cargo test`.
- **`# Safety`** — required on any `unsafe fn`. Document the invariants the caller must uphold.

Optional sections (`# Arguments`, `# Returns`) are fine but not required — prefer making signatures self-documenting with descriptive parameter names and types.

### Unsafe Policy

No `unsafe` without a `// SAFETY:` comment explaining the invariant. Prefer safe alternatives.

### Error Handling

- **`rdm-core`**: hand-written error enums implementing `std::error::Error` + `Display`. Keep errors matchable — no `anyhow` or type erasure in the library.
- **`rdm-cli` / `rdm-server`**: use `anyhow` with `.context()` for readable error chains. Add `anyhow` only when context chaining becomes useful; `Box<dyn Error>` is fine to start.
- User-facing CLI errors must be actionable: state what went wrong and what the user can do about it. Do not surface raw debug output or backtraces by default.

### Feature Flags

If `rdm-server` becomes optional, gate it behind a cargo feature flag so users who only need the CLI can skip it.

### Edition & MSRV

Rust version and dev tools are managed via [mise](https://mise.jdx.dev/) (see `.mise.toml`). Run `mise install` to set up the environment. Pin the same version as `rust-version` in `Cargo.toml`.

### Dependency Auditing

Use `cargo deny` for license and advisory checks. Run it in CI.

## Pre-commit Hooks

Hooks live in `.githooks/` and are shared via the repo. New clones need to configure the hooks path:

```bash
git config core.hooksPath .githooks
```

The pre-commit hook runs `cargo fmt --check`, `cargo clippy -- -D warnings`, and `cargo nextest run`.

## CI Expectations

All of the following must pass before merging:

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo deny check        # license & advisory audit
```

## Dogfooding

rdm's own development is tracked in a plan repo at `$RDM_ROOT` (set in `.mise.toml` to `~/Projects/rdm-atlas-repo`).

### *** DEVELOPMENT BUILD REQUIREMENT ***

**This is the rdm source repo. You MUST build from source and use the local binary — NEVER use a globally installed `rdm`.**

```bash
cargo build                    # ALWAYS run this before any rdm command
./target/debug/rdm <command>   # ALWAYS use this path, not bare `rdm`
```

Every `rdm` command shown below MUST be run as `./target/debug/rdm`. If you type bare `rdm` you are using a stale installed version that does not reflect your working changes. **There are zero exceptions.**

If you modify any rdm source code, you MUST `cargo build` again before running any rdm commands.

### Hard rule — no direct access to the plan repo

Do NOT use the Read, Glob, Grep, or Bash tools to read, search, list, or modify any files under `~/Projects/rdm-atlas-repo` (or whatever `$RDM_ROOT` resolves to). Every interaction with plan data — reading, creating, updating, deleting — MUST go through `./target/debug/rdm`. If the CLI cannot do something you need, that is a bug to fix in rdm, not a reason to bypass it.

### Discovering work

```bash
./target/debug/rdm roadmap list --project rdm       # list all roadmaps with progress
./target/debug/rdm task list --project rdm           # list open/in-progress tasks
./target/debug/rdm task list --project rdm --status all  # list all tasks including done
```

### Reading details

```bash
./target/debug/rdm roadmap show <slug> --project rdm          # show roadmap with phases and body
./target/debug/rdm phase list --roadmap <slug> --project rdm  # list phases with numbers and statuses
./target/debug/rdm phase show <stem-or-number> --roadmap <slug> --project rdm  # show phase details
./target/debug/rdm task show <slug> --project rdm             # show task details
```

Add `--no-body` to any `show` command to suppress body content when you only need metadata.

### Searching

When looking for specific items by keyword, **prefer `rdm search` over listing and manually scanning results**. Search is fuzzy (typo-tolerant) and matches against both titles and body content.

```bash
./target/debug/rdm search auth --project rdm                              # find items mentioning "auth"
./target/debug/rdm search index --type task --project rdm                 # find only tasks matching "index"
./target/debug/rdm search search --status in-progress --project rdm       # find in-progress items
./target/debug/rdm search auth --format json --project rdm                # structured output for chaining
```

Available filters: `--type` (roadmap|phase|task), `--status` (e.g., done, in-progress, open), `--limit` (default 20), `--format` (text|json).

### Updating status

Always pass `--no-edit` to prevent the CLI from opening an interactive editor (which will hang in non-interactive agent contexts).

```bash
./target/debug/rdm phase update <stem-or-number> --status done --no-edit --roadmap <slug> --project rdm
./target/debug/rdm task update <slug> --status done --no-edit --project rdm
```

### Creating items

Always pass `--no-edit` to suppress the interactive editor.

```bash
./target/debug/rdm roadmap create <slug> --title "Title" --body "Summary." --no-edit --project rdm
./target/debug/rdm phase create <slug> --title "Title" --number <n> --body "Details." --no-edit --roadmap <slug> --project rdm
./target/debug/rdm task create <slug> --title "Title" --body "Description." --no-edit --project rdm
```

For multiline content, pipe via stdin:

```bash
./target/debug/rdm task create <slug> --title "Title" --no-edit --project rdm <<'EOF'
Multi-line body content goes here.

It supports full Markdown.
EOF
```

Do **not** use `--body` and stdin together — the CLI will error.

### Planning workflow

#### Before starting work

Run `./target/debug/rdm roadmap list --project rdm` to see all roadmaps and their progress. Check `./target/debug/rdm task list --project rdm` for open tasks. Identify what is in-progress and what comes next before writing any code.

#### Implementing a roadmap phase

1. Read the phase: `./target/debug/rdm phase show <stem-or-number> --roadmap <slug> --project rdm`
2. Plan your approach and get approval before starting
3. Implement the work described in the phase
4. Mark it done: `./target/debug/rdm phase update <stem-or-number> --status done --no-edit --roadmap <slug> --project rdm`
5. Check the next phase: `./target/debug/rdm phase list --roadmap <slug> --project rdm`

#### Discovering bugs or side-work

If you encounter a bug or unrelated improvement while working on a phase, do not fix it inline. Create a task instead:

```bash
./target/debug/rdm task create <slug> --title "Description of the issue" --body "Details." --no-edit --project rdm
```

#### When a task grows too complex

If a task becomes large enough to warrant multiple phases, promote it to a roadmap:

```bash
./target/debug/rdm promote <task-slug> --roadmap-slug <new-roadmap-slug> --project rdm
```

### Status transitions

**Phase statuses:** `not-started` → `in-progress` → `done` (or `blocked`). `done` is terminal.

**Task statuses:** `open` → `in-progress` → `done` (or `wont-fix`). `done` and `wont-fix` are terminal.

## Setup

```bash
mise install                  # install Rust + dev tools from .mise.toml
git config core.hooksPath .githooks   # enable pre-commit hooks
```

## Build & Test

```bash
cargo build                    # build all crates
cargo nextest run              # run all tests
cargo watch -x 'nextest run'   # re-run tests on file change
cargo clippy                   # lint
cargo fmt --check              # check formatting
```
