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

rdm's own development is tracked in a plan repo at `$RDM_ROOT` (set in `.mise.toml` to `~/Projects/rdm-atlas-repo`). Before starting implementation work, build and use the CLI to check the current plan:

```bash
eval "$(mise env -s bash)"        # load RDM_ROOT from .mise.toml
cargo build                        # build the rdm binary
./target/debug/rdm list --project rdm   # see roadmap progress
./target/debug/rdm roadmap show <slug> --project rdm   # see phases in a roadmap
./target/debug/rdm phase show <stem> --roadmap <slug> --project rdm  # read phase details
```

Use this to understand what phase you're working on, what the acceptance criteria are, and what comes next. When a phase is complete, update its status:

```bash
./target/debug/rdm phase update <stem> --status done --roadmap <slug> --project rdm
```

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
