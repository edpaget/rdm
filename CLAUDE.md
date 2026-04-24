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
- **Claude Code skills** (`.claude/skills/`): `rdm-roadmap` (create roadmaps), `rdm-implement` (implement phases), `rdm-tasks` (work on tasks), `rdm-document` (generate docs from completed roadmaps)

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
- **Every commit with a user-facing change MUST include a corresponding `CHANGELOG.md` update in the same commit.** Do not defer changelog entries to a later commit or batch them up. If you are making a `feat`, `fix`, or any change that affects CLI commands, API endpoints, MCP tools, config options, or observable behavior, add the entry before committing.
- Entries should describe the change from a user's perspective (what they can now do, what was fixed) rather than internal implementation details

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

## Git Hooks

### Pre-commit

The pre-commit hook lives in `.githooks/` and is shared via the repo. New clones need to configure the hooks path:

```bash
git config core.hooksPath .githooks
```

Runs `cargo fmt --check`, `cargo clippy -- -D warnings`, and `cargo nextest run`.

### Post-merge & post-commit: `Done:` convention

Install the hooks in your plan repo with:

```bash
rdm hook install          # writes shims to .git/hooks/post-merge and .git/hooks/post-commit
rdm hook install --force  # overwrite existing hooks
rdm hook uninstall        # remove hooks (only if installed by rdm)
```

When a PR merges, `rdm hook post-merge` parses the commit message for lines matching:

```
Done: <roadmap>/<phase>
Done: task/<slug>
```

`rdm hook post-commit` does the same but only on the default branch (configured via `default_branch` in `rdm.toml`, defaults to `main`). This covers fast-forward merges (`git merge --ff-only`) which don't trigger `post-merge` hooks.

For phase directives, it calls `rdm phase update <phase> --status done --commit <sha> --no-edit --roadmap <roadmap>`. For task directives, it calls `rdm task update <slug> --status done --commit <sha> --no-edit`. Both are idempotent — running the hook multiple times or re-marking a done item with a new commit SHA is safe (the SHA updates, the completed date is preserved). Note: `task` is a reserved prefix and cannot be used as a roadmap slug.

Project resolution follows the standard chain: `--project` flag > `RDM_PROJECT` env var > `default_project` in `rdm.toml`.

**Example commit message:**

```
feat(core): implement search indexing

Done: search-feature/phase-2-indexing
Done: task/fix-search-edge-case
```

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

### Claude Code web

For sessions running in a sandboxed Claude Code web environment (no local plan repo mounted), use the template + harness shipped in this repo:

- Setup: `scripts/install-claude-code-web-template.sh <target-source-repo>` — drops in a `SessionStart` hook that clones the plan repo into the sandbox on every session start.
- Full setup, credentials, and troubleshooting: `docs/claude-code-web.md`.
- Regression harness: `bash scripts/verify-claude-code-web-loop.sh` — stands up a hermetic simulation of the bootstrap → Done: → plan-repo-update loop using temp dirs. Run it after touching the template, `rdm bootstrap`, or `rdm hook post-commit`.

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
4. Include a `Done:` line in the git commit message — the post-merge hook will mark the phase done and record the commit SHA.
   **Use the exact roadmap slug and phase stem from the rdm commands above — do NOT invent or paraphrase them:**
   ```
   Done: <roadmap-slug>/<phase-stem>
   ```
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

### Staging mode

By default, every mutation auto-commits to git. Use `--stage` (or `RDM_STAGE=true`, or `stage = true` in `rdm.toml`) to defer git commits — files are written to disk but the git commit is skipped until you explicitly run `rdm commit`.

```bash
./target/debug/rdm --stage task create fix-bug --title "Fix bug" --no-edit --project rdm  # writes file, no git commit
./target/debug/rdm status                          # show uncommitted changes
./target/debug/rdm commit -m "batch: fix bug and update phase"  # explicit git commit
./target/debug/rdm discard --force                 # reset working directory to HEAD (destructive)
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
