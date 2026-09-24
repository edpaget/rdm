# CLAUDE.md

## Project Overview

rdm is a Rust CLI for managing project roadmaps, phases, and tasks. "Zero-dependency" means users only need the compiled binary — no runtime dependencies, interpreters, or external tools. Cargo crate dependencies are fine. It separates the **tool** (this repo) from the **plan repo** (a git-managed directory of markdown files).

### Architecture

```
rdm-core/       # library: data model, parsing, file I/O
rdm-cli/        # binary: CLI porcelain over rdm-core
rdm-server/     # binary: REST API over rdm-core
```

See [`docs/principles.md`](docs/principles.md) §1–2 for architecture principles.

### Key Concepts

- **Plan repo**: a git-managed directory (`RDM_ROOT`) containing markdown files for roadmaps and tasks
- See [`docs/principles.md`](docs/principles.md) §6 for data storage and indexing.
- **Roadmaps** contain ordered **phases** (not-started | in-progress | needs-review | reviewed | done | blocked | wont-fix)
- **Tasks** are standalone work items (open | in-progress | needs-review | reviewed | done | wont-fix)
- Agent integration: `rdm agent-config` generates config for AI agents to interact via CLI
- **Claude Code skills** (`.claude/skills/`): `rdm-roadmap` (create roadmaps), `rdm-do` (implement phases / work on tasks; finalize runs the canonical code review), `rdm-document` (generate docs from completed roadmaps), `rdm-revise` (act on document reviews requesting changes), `rdm-plan-review` (review a plan before implementation begins), `rdm-backlog` (propose-only batched backlog grooming plan)

## Development Practices

### Commits

See [`docs/principles.md`](docs/principles.md) §13 for commit and changelog rules.

### TDD

See [`docs/principles.md`](docs/principles.md) §4 for testing principles. Run tests with `cargo nextest run`. Run specific crate tests with `cargo nextest run -p rdm-core`, etc. Use `cargo watch -x 'nextest run'` for continuous testing during development.

### Public API Docs

See [`docs/principles.md`](docs/principles.md) §8.

### Unsafe Policy

See [`docs/principles.md`](docs/principles.md) §10.

### Error Handling

See [`docs/principles.md`](docs/principles.md) §5.

### Feature Flags

See [`docs/principles.md`](docs/principles.md) §12.

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

## Agent Distribution

rdm's autonomous lane (skills and workflows) is distributed via two channels:

### Recommended: Claude Code Plugin Marketplace

Downstream consumers should install rdm via the plugin marketplace:

```bash
claude plugin marketplace add edpaget/rdm
claude plugin install rdm@rdm
```

**This repo does not install its own plugin.** It runs the local `.claude/` lane instead — a deliberately divergent surface that supports development and testing. See `docs/plugin-distribution.md` § "Which copy runs?" for the three surfaces and why this repo's configuration differs from downstream consumers.

### Fallback: Raw Skills Emission

For users unable to use the plugin marketplace:

```bash
rdm agent-config claude --skills --out <dir>
```

Not the recommended path — see [`docs/plugin-distribution.md`](docs/plugin-distribution.md) § "Which copy runs?" for why the plugin marketplace above is preferred.

## Dogfooding

rdm's own development is tracked in a plan repo at `$RDM_ROOT` (set in `.mise.toml` to `~/Projects/rdm-atlas-repo`).

### Claude Code web

For sessions running in a sandboxed Claude Code web environment (no local plan repo mounted), use the template + harness shipped in this repo:

- Setup: `scripts/install-claude-code-web-template.sh <target-source-repo>` — drops in a `SessionStart` hook that clones the plan repo into the sandbox on every session start.
- Full setup, credentials, and troubleshooting: `docs/claude-code-web.md`.

Regression harnesses for this loop and the rest of the dogfooding/autonomous lane: [`docs/dev-harnesses.md`](docs/dev-harnesses.md).

### Workflow-tool orchestration (autonomous lane)

The review pipeline is a deterministic **Claude Code Workflow-tool** script under `.claude/workflows/` (a sibling of `.claude/skills/` and `.claude/hooks/`); every driver above it is prose. `rdm agent-config claude --skills --out <dir>` emits the one shipped engine (`rdm-wf-review-refute-fix.js`) alongside the `rdm-dispatch-phase` skill that invokes it. Neither the autopilot drive loop nor the per-phase unit is a Workflow script — both are prose skills. See [`docs/workflow-schemas.md`](docs/workflow-schemas.md)'s Scope callout for the full shipped-vs-dogfood-only breakdown.

- Conventions and the canonical `FINDING` / `VERDICT` / `OUTCOME` schema contracts live in `docs/workflow-schemas.md`. A workflow script (`.js`) starts with `export const meta` and uses the ambient `agent()`/`pipeline()`/`parallel()`/`log()` globals; a canonical source module (`lib/*.mjs`) holds shared logic once and is Node-importable for testing.
- **The Workflow runtime cannot `import`/`require`** — see [`docs/workflow-schemas.md`](docs/workflow-schemas.md) § "Import spike". Shared pipeline logic is therefore kept single-source in `.claude/workflows/lib/` and stamped verbatim into consumers by the owning `scripts/gen-workflow-*.sh` (run it after editing a `lib/*.mjs`; `--check` mode gates drift), which also syncs the consumer's embedded `rdm-core/src/templates/workflows/` copy AND its checked-in `plugins/rdm/workflows/` copy in the same run — no separate step for either. This is a compile-time copy, not a cross-`workflow()` call, so it does not consume the one-level `workflow()` nesting budget. **A `lib/*.mjs` edit that reaches the distributed engine (`rdm-wf-review-refute-fix.js`) — including a comment-only one — changes its emitted bytes, so in the SAME commit regenerate the raw-skills baseline (`cargo test -p rdm-core regenerate_raw_skills_baseline -- --ignored`, covered only by `cargo nextest run`); the plugin tree's workflow bytes need no separate step either — the same generator run wrote them directly. Its SKILL-markdown bytes are the unrelated propagation `lib/*.mjs` -> `gen-skill-review.sh` -> `rdm-core/src/templates/skill-*.md` -> `generate_plugin_*` -> `plugins/rdm/`, which still needs its own explicit command (`env -u RDM_ROOT -u RDM_PROJECT cargo run -q --bin rdm -- agent-config claude --plugin --out plugins/rdm`, covered by `scripts/verify-plugin-install.sh`) when a `lib/review.mjs` edit reaches the rendered `//|` spec prose. Miss either copy and it surfaces at land time.**
- `.claude/workflows/lib/review.mjs` is the **single canonical review implementation** every surface consumes — find → refute → filter → verdict → gate policy, across `code` and `plan` modes. It has two `--check`-gated projections (`gen-workflow-review.sh` into workflow consumers, `gen-skill-review.sh` into skill templates) and a mode-dispatched gate policy. See [`docs/workflow-schemas.md`](docs/workflow-schemas.md) §§ `buildReviewPipeline(mode, deps?)`, `OUTCOME (review pipeline)`, and "Two projections, two `--check`-gated generators" for the mechanics, and [`docs/mechanical-agent-inventory.md`](docs/mechanical-agent-inventory.md) for the canonical sweep record.

#### `.claude/agents/` — the custom-agent registry, now a shipped emission surface

`.claude/agents/` is the custom-agent registry `agent()`'s `opts.agentType` resolves against. It
holds one definition, `rdm-mechanical.md`; no workflow references it (a workflow contains only
judgment agents), but `rdm-core/src/agent_config.rs`'s `generate_agents()` still ships it into
every `rdm agent-config claude --skills`-produced downstream tree, so a downstream caller can.
Evidence, measurements and disposition live in `docs/workflow-schemas.md` § "agentType / effort
options spike" — that section is canonical; do not restate its tables here.

Note when adding an agent definition: Claude Code watches `.claude/agents/`, but the watcher
covers only directories that existed at session start, so creating a scope's **first** agent
file in a new `agents` directory needs a restart before it resolves. Project definitions are
also discovered by walking up from the cwd, so a session rooted outside this repo will not see
this one.

**When editing this file:** the project `CLAUDE.md` is loaded into every subagent, including a
custom-`agentType` one, and cannot be suppressed per agent type — it was measured at 19320
tokens (2.49 chars/token, against a 48207-char file). Every paragraph added here is paid for once
per dispatched agent, so keep additions here short and put the detail in `docs/`.

#### Two surfaces: workflow vs skill

Decision rule for which surface to reach for:

The autonomous lane's completion trailer is written at **land time**: a dispatch emits `writesCompletion: true` on a `reviewed` OUTCOME and never writes the directive itself (the prose `rdm-autopilot` loop that dispatches a phase carries the same OUTCOME through, unread for this field); `rdm-land` reads `writesCompletion: true` and, after landing, marks the item `done` directly — via `phase update`/`task update ... --status done --commit <sha>` using the OUTCOME's identifiers — no rebase-time write and no trailer are involved any more.

The per-phase driver is **prose** — `.claude/skills/rdm-dispatch-phase/SKILL.md` is the orchestrator, `rdm-do` is a shim onto it, and `rdm-autopilot` step 4 enters it with `Skill`. See [`docs/autonomous-loop.md`](docs/autonomous-loop.md) § "The per-phase unit: the prose orchestrator" for the full sequence, delegation boundary, and gate policy.

- Reach for a **workflow** when the unit is fan-out-shaped, mechanism rather than policy, headless (no *mid-run* gate), deterministic, and hermetically gatable: the review pipeline (`rdm-wf-review-refute-fix`, `rdm-wf-plan-review`), the `rdm-wf-estimate` fan-out, and the batched passes (`rdm-wf-backlog` takes `{ project?, olderThan?, tag? }`; `rdm-wf-document`). Entry points: the `Workflow` tool, `rdm-do --auto <roadmap> <phase>`, or the thin skill shims.
- Reach for a **skill** when a human is in the loop for plan approval or discussion, or when the unit is a low-iteration sequential driver that is mostly policy: interactive `rdm-do` (no `--auto`), `rdm-roadmap`, `rdm-revise`, `rdm-plan-review`, `rdm-review`, `rdm-estimate`, `rdm-land`, `rdm-document`.
- The autopilot drive loop has migrated to prose (an orchestrating `rdm-autopilot` skill, `.claude/skills/rdm-autopilot/SKILL.md`) under the `prose-autopilot-orchestration` roadmap; phase 3 retired `autopilot.js`/`lib/autopilot.mjs` and its harness `scripts/verify-workflow-autopilot.sh` in favor of `scripts/verify-skill-autopilot.sh`.
- Canonical: [`docs/workflow-vs-prose-boundary.md`](docs/workflow-vs-prose-boundary.md) — criteria, all eight scripts' dispositions, non-goals.

Every review-producing lane runs the canonical review before its finalize step returns and persists the resulting status itself, so nothing is left parked in `needs-review` — see [`docs/autonomous-loop.md`](docs/autonomous-loop.md) § "Relation to the retired needs-review safety net".

The dispatch runs a phase-time verification gate through the repo-only `dispatch.verify` config key (with a documented discovery fallback and a bounded rework budget on failure) — see [`docs/verify-gate.md`](docs/verify-gate.md).

The dispatch also stamps the target phase/task `in-progress` itself, best-effort, before planning begins (skipped under `--plan-only`) — see [`docs/autonomous-loop.md`](docs/autonomous-loop.md).

### *** DEVELOPMENT BUILD REQUIREMENT ***

**This is the rdm source repo. You MUST build from source and use the local binary — NEVER use a globally installed `rdm`.**

```bash
cargo build   # ALWAYS run this before any rdm command
```

`.mise.toml` sets `$RDM_BIN` to the checkout the session started in, which stays wrong inside a roadmap or task worktree — `$RDM_BIN` must always name the build of the checkout you're editing. After `cd`-ing into a worktree, rebind it (`export RDM_BIN="$PWD/target/debug/rdm"`, mirroring `AGENTS.md`'s "rebind `RDM_BIN` to the returned checkout"), or just run that checkout's `./target/debug/rdm` directly. Run every rdm command as `"$RDM_BIN" <command>`, never a bare `rdm`, which risks a stale global install. If you modify any rdm source code, `cargo build` again before running further rdm commands.

### Hard rule — no direct access to the plan repo

Do NOT use the Read, Glob, Grep, or Bash tools to read, search, list, or modify any files under `~/Projects/rdm-atlas-repo` (or whatever `$RDM_ROOT` resolves to). Every interaction with plan data — reading, creating, updating, deleting — MUST go through `"$RDM_BIN"`. If the CLI cannot do something you need, that is a bug to fix in rdm, not a reason to bypass it.

### CLI usage

`"$RDM_BIN" agent-config claude --project rdm` prints rdm's full CLI usage guide (discovering work, reading, searching, updating status, document reviews, linking, creating items, status transitions, session-scoped commits) with `--project rdm` already filled in, instead of the guide's own `--project <PROJECT>` placeholder; the same content lives in `rdm-core/src/templates/instructions-cli.md`. Substitute `"$RDM_BIN"` for the bare `rdm` it shows.

This repo also enables `plan_review` (see "Plan review" below) and `gates.reviewed` on its own plan data — see [`docs/core-enforced-gates.md`](docs/core-enforced-gates.md) for the latter's config and override policy.

### Plan review

A second, earlier gate than the document-review flow described in the generated CLI guide's "Document reviews" section: it reviews a roadmap/phase/task's **plan** before implementation begins, rather than the diff after implementation. Controlled by the `plan_review` config flag (`"$RDM_BIN" config set plan_review true`, `RDM_PLAN_REVIEW` env override, default `false`) — enabled for this repo's own plan data. While the flag is on, `roadmap create` / `phase create` / `task create` automatically stamp a reserved `needs-plan-review` tag onto every new item, alongside any user-supplied `--tags`.

List pending items with:

```bash
"$RDM_BIN" search "" --tag needs-plan-review --project rdm
```

Run the `rdm-plan-review` skill against a pending item to review it: it dispatches parallel read-only sub-agents for coherence, architectural fit, restraint, and (for phases) unit-of-work sizing, then consolidates a **PASS** / **PASS WITH CONCERNS** / **REWORK** verdict. On PASS or PASS WITH CONCERNS it clears the `needs-plan-review` tag; on REWORK it leaves the tag in place and reports what must change. Both the plan-review Stop hook (`.claude/hooks/rdm-plan-review-on-create.sh`) and the earlier needs-review Stop hook are now retired — nothing reprompts automatically. Clearing `needs-plan-review` on items created via `rdm-roadmap`, ad hoc create commands, or `rdm-do` side-task filing is manual-only: run `rdm-plan-review` against the item, or periodically sweep with `"$RDM_BIN" search "" --tag needs-plan-review`. Active enforcement is tracked as a follow-up task (`wire-active-plan-review-tag-gate`) filed in the plan repo.

The gate's self-review decision (may a session clear the tag on a plan it authored? — yes, because the verdict comes from independent finders/refuters, with a stated boundary), and the three recorded classifier blocks behind it, live in [`docs/plan-review-gate-policy.md`](docs/plan-review-gate-policy.md). `rdm-wf-plan-review` never writes the tag — it always returns `gateAction.commands` and the caller runs them, which is what `gateMode: 'return'` used to opt into. A unit still awaiting that write carries a `[gate pending: …]` clause on its summary and is counted by `gatePendingCount`; one whose current tag list the caller did not supply carries `tagsUnknown: true` and no commands, because `--tags` replaces the whole list.

This gate composes with, and is independent from, the `needs-review` gate described in the generated CLI guide: `plan_review`/`needs-plan-review` gates **before** implementation begins (on the plan document), while `rdm-review`/`needs-review` gates **after** implementation (on the diff). Neither the plan-review Stop hook (`rdm-plan-review-on-create.sh`) nor the needs-review Stop hook (`rdm-review-on-finalize.sh`) is active in this repo any longer.

