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
- **Roadmaps** contain ordered **phases** (not-started | in-progress | needs-review | reviewed | done | blocked | wont-fix)
- **Tasks** are standalone work items (open | in-progress | needs-review | reviewed | done | wont-fix)
- Agent integration: `rdm agent-config` generates config for AI agents to interact via CLI
- **Claude Code skills** (`.claude/skills/`): `rdm-roadmap` (create roadmaps), `rdm-do` (implement phases / work on tasks; finalize runs the canonical code review), `rdm-document` (generate docs from completed roadmaps), `rdm-revise` (act on document reviews requesting changes), `rdm-plan-review` (review a plan before implementation begins), `rdm-backlog` (propose-only batched backlog grooming plan)

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

The pre-commit hook lives in `.githooks/` and is shared via the repo. New clones need to configure the hooks path and provision the toolchain:

```bash
git config core.hooksPath .githooks   # point git at the shared hooks
mise install                          # provision hk + shellcheck + shfmt + cargo tools
```

`.githooks/pre-commit` is a thin shim that delegates to [`hk`](https://hk.jdx.dev/) (`exec hk run pre-commit`); the gate itself is declared in `hk.pkl`. It runs `cargo fmt --check`, `cargo clippy -- -D warnings`, and `cargo nextest run`, plus `shellcheck` and `shfmt` over staged shell scripts (`.shellcheckrc` and `.editorconfig` are the shared config). It is check-only — it never rewrites files. `post-commit` / `post-merge` (the rdm `Done:`-convention hooks below) are left untouched.

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

**Reliability guarantees:** `rdm hook post-merge`/`post-commit` can never block the invoking `git commit`/`git merge` indefinitely. Execution is bounded by a `hook_timeout_secs` deadline (repo `rdm.toml` or global config, same precedence as `default_branch`; defaults to 30s, and `0` is treated as unset rather than "unbounded") — a hook that hits the deadline logs a `timeout` event and still exits 0. Independently, every git subprocess rdm spawns is hardened to be non-interactive (forced non-interactive editor and disabled credential/host-key prompts, regardless of the invoking user's git config), and a hook invocation that detects it was itself spawned as a git subprocess by rdm (e.g. a real `git commit` made by `rdm resolve` re-triggering this same repo's installed hooks) short-circuits immediately instead of re-running the `Done:`-directive pipeline.

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
cargo deny check                          # license & advisory audit
shellcheck $(git ls-files '*.sh')         # shell lint
shfmt -d $(git ls-files '*.sh')           # shell format check
for f in scripts/verify-*.sh; do bash "$f"; done   # shell integration harnesses
```

## Dogfooding

rdm's own development is tracked in a plan repo at `$RDM_ROOT` (set in `.mise.toml` to `~/Projects/rdm-atlas-repo`).

### Claude Code web

For sessions running in a sandboxed Claude Code web environment (no local plan repo mounted), use the template + harness shipped in this repo:

- Setup: `scripts/install-claude-code-web-template.sh <target-source-repo>` — drops in a `SessionStart` hook that clones the plan repo into the sandbox on every session start.
- Full setup, credentials, and troubleshooting: `docs/claude-code-web.md`.
- Regression harness: `bash scripts/verify-claude-code-web-loop.sh` — stands up a hermetic simulation of the bootstrap → Done: → plan-repo-update loop using temp dirs. Run it after touching the template, `rdm bootstrap`, or `rdm hook post-commit`.
- Cross-host worktree-review harness: `bash scripts/verify-worktree-review-loop.sh` — hermetic do → finalize → trigger → review regression for the one-worktree-per-roadmap model, asserting roadmap isolation across the `rdm review pending` scoping the Pi `agent_end` host path (and any future host) depends on. Run it after touching the worktree/review-trigger model (`rdm worktree`, the branch-scoped `rdm review pending` filter, or needs-review stamping).
- Review revision-loop harness: `bash scripts/verify-review-revision-loop.sh` — hermetic end-to-end regression for the LLM revision workflow (the `rdm-revise` loop): a submitted request-changes review is worked comment by comment against a temp git-backed plan repo, covering the resolved-anchor, whole-document, drifted-anchor-clarification (blocks close), wont-fix, and completion paths with explicit `--applied-commit` provenance. Run it after touching the review resolution model (`ops/reviews.rs` update paths, anchor drift resolution, `rdm review requests`/`update`, the MCP review tools, or the `rdm-revise` skill templates).
- Backlog grooming harness: `bash scripts/verify-backlog-groom-loop.sh` — hermetic end-to-end regression for the grooming loop (`rdm-backlog` skill's command surface): seeds stale tasks, a duplicate pair, a tag cluster, and a fully-terminal roadmap, then drives `rdm backlog report`, `rdm promote --into`, `rdm task merge`, `rdm task update --reason`, and `rdm roadmap archive` against the real binary end to end. Run it after touching `rdm-core/src/ops/backlog.rs`, `consolidate_task_into_roadmap`/`merge_tasks` in `rdm-core/src/ops/task.rs`, `rdm roadmap archive`, or the `rdm-backlog` skill template.
- Plan-review Stop hook harness: `bash scripts/verify-plan-review-hook-loop.sh` — hermetic FIRE / LOOP-GUARD / CLEARED / FAIL-OPEN regression driving the shipped `rdm-core/src/templates/hook-plan-review-on-create.sh` template against a hermetic temp plan + source repo (the dogfooded copy's own loop is covered by the manual reproduction recipe in `.claude/hooks/rdm-plan-review-on-create.sh`'s header). Run it after touching the hook template, `plan_review`/`needs-plan-review` stamping, or `rdm search --tag`.

### Workflow-tool orchestration (autonomous lane)

The autonomous do/autopilot lane is migrating from prose-orchestrated skills to deterministic **Claude Code Workflow-tool** scripts under `.claude/workflows/` (a sibling of `.claude/skills/` and `.claude/hooks/`). This lane is **dogfood-only** — the scripts are not emitted by `agent-config`; rdm's shipped autonomous skill templates remain the user-facing lane. The interactive skills (`rdm-do`, `rdm-roadmap`, `rdm-revise`) stay on skills because workflows are headless and cannot pause for human approval gates.

- Conventions and the canonical `FINDING` / `VERDICT` / `OUTCOME` schema contracts live in `docs/workflow-schemas.md`. A workflow script (`.js`) starts with `export const meta` and uses the ambient `agent()`/`pipeline()`/`parallel()`/`log()` globals; a canonical source module (`lib/*.mjs`) holds shared logic once and is Node-importable for testing.
- **The Workflow runtime cannot `import`/`require`** (proven by the P1 import spike, recorded in `docs/workflow-schemas.md`). Shared pipeline logic is therefore kept single-source in `.claude/workflows/lib/` and stamped verbatim into consumers by `scripts/gen-workflow-review.sh` (run it after editing a `lib/*.mjs`; `--check` mode gates drift). This is a compile-time copy, not a cross-`workflow()` call, so it does not consume the one-level `workflow()` nesting budget.
- `.claude/workflows/lib/review.mjs` is the **single canonical review implementation** every surface consumes — find → refute → filter → verdict → gate policy. `buildReviewPipeline(mode)` runs parallel dimension finders → a fresh refuter per finding → drops refuted-or-low-confidence → returns ranked survivors, behind `code` (ac/correctness always-on; tests/architecture/api-docs/changelog/security triggered) and `plan` (coherence/architectural-fit always-on; unit-of-work phases-only) modes; `classifyOutcome` yields the canonical `reviewed | rework | escalated` vocabulary, and `statusFor`/`writesCompletion` map it onto rdm statuses (escalated → `blocked` for phases *and* tasks). It has TWO `--check`-gated projections: `scripts/gen-workflow-review.sh` stamps its JS block into the workflow consumers, and `scripts/gen-skill-review.sh` renders its `//|` spec prose into the shipped `skill-review-{cli,mcp}.md` templates (mode-dispatched, so the plan-review surface can reuse it). `.claude/workflows/review-refute-fix.js` is a thin standalone wrapper. The land-time `Done:` trailer is never written by stamped workflow JS: its format lives in `rdm_core::hook::format_done_directive` (surfaced as `rdm hook done-line`) and is written by the review skill's gate step or by `rdm-land` at land time.
- Review pipeline harness: `bash scripts/verify-workflow-review.sh` — hermetic DRIFT / HYGIENE / BEHAVIOR regression. It runs both `gen-workflow-review.sh --check` and `gen-skill-review.sh --check --mode code`, greps the workflow scripts for forbidden `Date.now(`/`Math.random(`, and drives the shared module in Node (stdlib-only, no packages; `node` pinned in `.mise.toml`) with an injected fake agent + reference `pipeline`/`parallel`, asserting a planted refutable finding is dropped, a real one survives, the confidence floor drops low-confidence findings, a fresh refuter grades each finding, context is threaded into every prompt, and output is deterministic across both modes. Run it after touching `.claude/workflows/lib/review.mjs`, either generator, or the workflow/skill consumers.
- Dispatch-phase harness: `bash scripts/verify-workflow-dispatch.sh` — hermetic regression for dispatch-phase's 4-stage pipeline and its reviewed/rework/escalated outcome branches, plus the byte-identical-copy drift gate against `lib/dispatch-phase.mjs`.
- Autopilot harness: `bash scripts/verify-workflow-autopilot.sh` — hermetic regression for the autopilot roadmap-driving loop (drive-to-reviewed, rework→park, escalated, budget stops, estimate pre-pass, `--plan-only`) plus its byte-identical-copy drift gate against `lib/autopilot.mjs`.
- `rdm-do --auto` wiring harness: `bash scripts/verify-workflow-do-auto.sh` — hermetic regression for the `rdm-do --auto` phase-flow wiring into dispatch-phase (SKILL.md static invariants, the OUTCOME→status contract against the real binary, prose-only distributed-template self-test).

#### Two surfaces: workflow vs skill

Decision rule for which surface to reach for:

The autonomous lane's completion trailer is written at **land time**: `dispatch-phase`/`autopilot` emit `writesCompletion: true` on a `reviewed` OUTCOME and never write the directive themselves, and `rdm-land` synthesizes it from the OUTCOME identifiers via `rdm hook done-line`, amending it before the rebase — so a landed autopilot branch never needs a manual rebase to gain it.

- Reach for a **workflow** when the work is autonomous, opt-in, billed-per-run, and headless (no mid-run human gate): driving a whole roadmap (`autopilot`, Phase 3 of this roadmap) or dispatching one phase end-to-end (`dispatch-phase`, Phase 2). Entry points: invoke directly via the `Workflow` tool (`autopilot` takes `{ roadmap, maxPhases?, planOnly? }`; `dispatch-phase` takes `{ roadmap, phase }`), via `rdm-do --auto <roadmap> <phase>` (Phase 4, routes into `dispatch-phase`), or via the thin `rdm-autopilot`/`rdm-dispatch-phase` skill shims.
- Reach for a **skill** when a human is in the loop for plan approval or discussion: interactive `rdm-do` (no `--auto`), `rdm-roadmap`, `rdm-revise`, `rdm-plan-review`, `rdm-review`, `rdm-backlog`, `rdm-estimate`, `rdm-land`, `rdm-document`.
- The autonomous lane points at the workflow scripts under `.claude/workflows/` (`dispatch-phase.js`, `autopilot.js`, `review-refute-fix.js`).

This roadmap leaves the distributed skill templates (`rdm-core/src/templates/skill-{autopilot,dispatch-phase}-{cli,mcp}.md`) untouched — they remain the source of truth `agent-config` emits, still carrying the original defensive prose. (The local `.claude/skills/rdm-autopilot`/`rdm-dispatch-phase` copies were already rewritten as the thin shims noted above during Phases 2–3 of this roadmap; Phase 5 touches no skill files.) Trimming the templates' now-superseded defensive prose (the "Mandatory dispatch"/inline-collapse checklists, written before the workflow scripts existed) is deferred to the distribution follow-up roadmap — not yet created; see the `workflow-orchestration` roadmap body for the cross-reference.

**Hook reconciliation (verified read-only, no code changes):** `scripts/verify-plan-review-hook-loop.sh` and `scripts/verify-worktree-review-loop.sh` drive the real hook scripts / `rdm review pending` off directly-set rdm state (status/tags), never off a specific driver — a green run across both shows the surviving `needs-plan-review` Stop hook, and the `rdm review pending`/`restamp` scoping the retired hook used to depend on, are agnostic to whatever set that state, workflow or skill.

**Update (unify-code-review phase 6 → phase 7):** phase 6 made review active in every lane that can produce a `needs-review` item — `dispatch-phase`/`autopilot` persist `reviewed`/`blocked` status directly and never leave an item parked in `needs-review` (dispatch-phase's code-review stage is the canonical review, stamped from `.claude/workflows/lib/review.mjs` and fed diff-derived `signals`), and interactive `rdm-do`'s finalize actively invokes `rdm-review` after the human confirm gate before the session stops. With nothing left unreviewed, phase 7 retired the now-redundant `needs-review` auto-review Stop hook / Pi `agent_end` extension outright (`.claude/hooks/rdm-review-on-finalize.sh`, `rdm-core/src/templates/hook-review-on-finalize.sh`, `rdm-core/src/templates/extension-review-on-finalize.ts`, and their `agent-config`/`--hooks` wiring) — see [`docs/autonomous-loop.md`](docs/autonomous-loop.md). The sibling `needs-plan-review` Stop hook / Pi extension is a separate, still-live gate and is untouched.

**Update (no-in-progress-stamp):** `dispatch-phase` also stamps the target phase (or task) `in-progress` itself now — a best-effort, mechanical write right after Stage 0 (metadata + model resolution) and before planning begins. A `--plan-only` run skips it via the args-level `if (!planOnly)` guard, since a plan-only pass does no implementation and stamping would misreport it. `autopilot` writes no status beyond its existing terminal advance/park calls — the stamp lives solely in the unit that actually works the item (`dispatch-phase`), the same reasoning that put the terminal `reviewed`/`blocked` writes there in phase 6 above. This closes the one gap left after phase 6: a direct `Workflow` invocation of `dispatch-phase` (and therefore every autopilot-driven phase) used to jump straight from `not-started` to a terminal status with no observable in-progress signal during the run; interactive `rdm-do`, `rdm-do --auto`, and the `rdm-dispatch-phase` skill already stamped in-progress before invoking the workflow. This is purely an observability fix, not a mutual-exclusion mechanism — see `docs/autonomous-loop.md`. It does not revisit the `needs-review`-is-dormant-by-design finding above.

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
./target/debug/rdm roadmap list --project rdm              # list all roadmaps with progress
./target/debug/rdm roadmap list --project rdm --tag auth    # list roadmaps carrying tag "auth"
./target/debug/rdm task list --project rdm                  # list active tasks (open, in-progress, needs-review, reviewed)
./target/debug/rdm task list --project rdm --status all     # list all tasks including done
./target/debug/rdm task list --project rdm --tag bug        # list active tasks carrying tag "bug"
./target/debug/rdm task list --project rdm --tag bug --tag ui  # ANDs across tags
```

`--tag` is repeatable on `roadmap list`, `task list`, the top-level `rdm list`,
and `search`; repeats AND together, matching is exact and case-sensitive, and
passing none imposes no constraint. When any listed item is tagged, list output
gains a trailing `Tags` column (or a ` [tags: a, b]` line suffix in the default
`roadmap list` view).

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
./target/debug/rdm search "" --tag bug --project rdm                      # list every item carrying tag "bug"
./target/debug/rdm search auth --tag bug --tag ui --project rdm           # ANDs across tags
```

Available filters: `--type` (roadmap|phase|task), `--status` (e.g., done, in-progress, open), `--tag <name>` (repeatable, AND), `--limit` (default 20), `--format` (text|json).

### Updating status

Always pass `--no-edit` to prevent the CLI from opening an interactive editor (which will hang in non-interactive agent contexts).

```bash
./target/debug/rdm phase update <stem-or-number> --status done --no-edit --roadmap <slug> --project rdm
./target/debug/rdm task update <slug> --status done --no-edit --project rdm
./target/debug/rdm commit -m "chore(plan): update status"  # land the batch
```

### Document reviews

Structured feedback on a roadmap, phase, or task document, with comments anchored to quoted text. Lifecycle: `draft` → `submitted` (verdict: `approve` | `request-changes` | `comment`) → `addressed` | `dismissed`. Targets are `roadmap/<slug>`, `phase/<roadmap-slug>/<stem-or-number>`, or `task/<slug>`.

```bash
./target/debug/rdm review start --on task/<slug> --no-edit --project rdm            # start a draft; prints the review id
./target/debug/rdm review comment <review-id> --quote "exact text" --body "Feedback." --no-edit --project rdm
./target/debug/rdm review submit <review-id> --verdict request-changes --no-edit --project rdm
./target/debug/rdm review requests --project rdm                                    # agent queue: submitted + request-changes
./target/debug/rdm review show <review-id> --format json --project rdm              # anchors + resolution states in one call
./target/debug/rdm review update <review-id> --comment 1 --status addressed --applied-commit <sha> --reply "Fixed." --project rdm
./target/debug/rdm review update <review-id> --state addressed --project rdm        # close once every comment is resolved
./target/debug/rdm review list --state submitted --project rdm                      # filter by --on/--state/--verdict/--author
```

`--quote` must match the document text exactly; it is located in the document **as of the review's `created_commit`**, so it stays valid after later edits. An ambiguous quote fails with a 1-based occurrence list — re-run with `--occurrence <n>`. On a roadmap review, `--doc phase/<stem-or-number>` scopes a comment to one of the roadmap's phases. `rdm search <query> --type review --project rdm` matches review summaries and comment bodies.

### Plan review

A second, earlier gate than the document-review flow above: it reviews a roadmap/phase/task's **plan** before implementation begins, rather than the diff after implementation. Controlled by the `plan_review` config flag (`./target/debug/rdm config set plan_review true`, `RDM_PLAN_REVIEW` env override, default `false`) — enabled for this repo's own plan data. While the flag is on, `roadmap create` / `phase create` / `task create` automatically stamp a reserved `needs-plan-review` tag onto every new item, alongside any user-supplied `--tags`.

List pending items with:

```bash
./target/debug/rdm search "" --tag needs-plan-review --project rdm
```

Run the `rdm-plan-review` skill against a pending item to review it: it dispatches parallel read-only sub-agents for coherence, architectural fit, and (for phases) unit-of-work sizing, then consolidates a **PASS** / **PASS WITH CONCERNS** / **REWORK** verdict. On PASS or PASS WITH CONCERNS it clears the `needs-plan-review` tag; on REWORK it leaves the tag in place and reports what must change. A Stop hook (`.claude/hooks/rdm-plan-review-on-create.sh`) reprompts the agent to run the skill while any item still carries the tag.

This gate composes with, and is independent from, the existing `needs-review` gate above: `plan_review`/`needs-plan-review` gates **before** implementation begins (on the plan document), while `rdm-review`/`needs-review` gates **after** implementation (on the diff). Only the plan-review Stop hook (`rdm-plan-review-on-create.sh`) is active in this repo — the analogous needs-review Stop hook (`rdm-review-on-finalize.sh`) was retired once review became active on every finalize path; see "Hook reconciliation" above.

### Creating items

Always pass `--no-edit` to suppress the interactive editor.

```bash
./target/debug/rdm roadmap create <slug> --title "Title" --body "Summary." --tags bug,ui --no-edit --project rdm
./target/debug/rdm phase create <slug> --title "Title" --number <n> --body "Details." --tags audit --no-edit --roadmap <slug> --project rdm
./target/debug/rdm task create <slug> --title "Title" --body "Description." --tags bug --no-edit --project rdm
./target/debug/rdm commit -m "chore(plan): create <slug> roadmap/phase/task"  # land the batch
```

`--tags` is comma-separated. On `update`, `--tags` replaces the existing list; pass `--tags ""` to clear. Tagging convention: lowercase kebab-case (`bug`, `auth`, `tech-debt`); prefer existing tags — check with `./target/debug/rdm search "" --tag <candidate> --project rdm` before inventing a new one. `depends-unlanded` is a reserved tag (like `needs-plan-review`) for a side-task filed from inside a worktree whose body cites a file or behavior that only exists on an unlanded branch — see "Discovering bugs or side-work" above.

`--body` is **authoritative**: when you pass `--body`, rdm uses that value verbatim and ignores stdin. This includes backticks, em-dashes, curly quotes, and other Unicode/punctuation — none of it triggers stdin reads or hangs. To intentionally empty an existing body on `phase update`, `task update`, or `roadmap update`, pass `--clear-body` (mutually exclusive with `--body`); passing `--body ""` against a non-empty body is rejected with an actionable error to prevent silent clobber from a truncated heredoc or empty command substitution.

For multiline content on **`create`**, pipe via stdin (do not also pass `--body` — stdin is ignored when `--body` is present):

```bash
./target/debug/rdm task create <slug> --title "Title" --no-edit --project rdm <<'EOF'
Multi-line body content goes here.

It supports full Markdown.
EOF
```

`update` (`roadmap`/`phase`/`task`) does **not** read stdin — this is deliberate, so a tags-only or status-only update can't hang on an open pipe. A heredoc piped into `update` is silently ignored. To set a multiline body on an update, capture it into a shell variable with a quoted heredoc (which keeps backticks, `$`, and punctuation literal), then pass `--body`:

```bash
body=$(cat <<'EOF'
Multi-line body content goes here.

It supports full Markdown.
EOF
)
./target/debug/rdm task update <slug> --body "$body" --no-edit --project rdm
```

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

**Worktree-vs-main hazard:** when you are working inside a roadmap's or task's shared worktree (not `main`), any file or symbol you cite in a new task's body may exist only on that unlanded branch. Describing it as "existing" without qualification is a false premise — a plan-review gate that checks the current `main` will correctly REWORK it. Before filing a side-task from a worktree, check whether the paths/behavior you're citing are on `main` (e.g. if you introduced or modified them in this same roadmap's or task's phases, they are not yet on `main`). If they aren't, tag the new task with the reserved tag `depends-unlanded` and phrase the body as "`<file/behavior>`, introduced by `<roadmap-or-task-worktree-ref>`, not yet on main":

```bash
./target/debug/rdm task create sweep-x --title "..." --body "rdm-core/src/ops/tag.rs, introduced by roadmap tagging-support, not yet on main. ..." --tags depends-unlanded --no-edit --project rdm
```

Remember `--tags` replaces the whole list on `update` — if you later update a `depends-unlanded` task, read-modify-write the tag list so you don't silently drop the annotation.

#### When a task grows too complex

If a task becomes large enough to warrant multiple phases, promote it to a roadmap:

```bash
./target/debug/rdm promote <task-slug> --roadmap-slug <new-roadmap-slug> --project rdm
```

If the task instead belongs inside an already-existing thematic roadmap, fold it in as a new trailing phase instead of creating a new roadmap:

```bash
./target/debug/rdm promote <task-slug> --into <existing-roadmap-slug> --project rdm
```

### Status transitions

**Phase statuses:** `not-started` → `in-progress` → `needs-review` → `reviewed` → `done` (or `blocked`, or `wont-fix`). `done` and `wont-fix` are terminal.

**Task statuses:** `open` → `in-progress` → `needs-review` → `reviewed` → `done` (or `blocked`, or `wont-fix`). `done` and `wont-fix` are terminal.

### Workflow

Every mutation stages changes; run `rdm commit` to land them. There is no auto-commit and no opt-in flag — `roadmap`/`phase`/`task`/`review` create/update/delete commands all write to disk immediately, but the git commit is always deferred until you explicitly run `rdm commit`. Batch related mutations together, then land them in one commit.

```bash
./target/debug/rdm task create fix-bug --title "Fix bug" --no-edit --project rdm  # writes file, no git commit yet
./target/debug/rdm status                          # show staged changes
./target/debug/rdm commit -m "batch: fix bug and update phase"  # explicit git commit — lands the batch
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
