# CLAUDE.md

## Project Overview

rdm is a Rust CLI for managing project roadmaps, phases, and tasks. "Zero-dependency" means users only need the compiled binary — no runtime dependencies, interpreters, or external tools. Cargo crate dependencies are fine. It separates the **tool** (this repo) from the **plan repo** (a git-managed directory of markdown files).

### Architecture

```
rdm-core/       # library: data model, parsing, file I/O
rdm-cli/        # binary: CLI porcelain over rdm-core
rdm-server/     # binary: REST API over rdm-core
```

Core is the source of truth. CLI and server are thin layers. New interfaces (TUI, WASM module) should call core, not duplicate logic.

### Key Concepts

- **Plan repo**: a git-managed directory (`RDM_ROOT`) containing markdown files for roadmaps and tasks
- rdm generates no derived index file; use `rdm list --format markdown` for a browsable snapshot (see [`docs/index-removal.md`](docs/index-removal.md))
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

See [`docs/principles.md`](docs/principles.md) §5 for core error design. CLI and server layers use `anyhow` with `.context()` for readable error chains; add it only when context chaining becomes useful (start with `Box<dyn Error>` if not).

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

`./target/debug/rdm info --format json` reports what rdm actually resolved for the current environment in one call — `{root, project, default_branch, default_format}`, each following the CLI's real precedence chain — useful when a tool other than this project's own CLAUDE.md needs to discover its plan repo location without scraping `rdm config list` or reading `rdm.toml` directly.

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
./target/debug/rdm verify resolve --project rdm            # the configured dispatch.verify command, or `unresolved`
./target/debug/rdm verify run --item <roadmap> --format json --project rdm  # run it in the item's worktree
```

`--status reviewed` can be gated on real plan + change-review records and a clean worktree (repo-only `gates.reviewed`, default OFF, with an audited `--override-gate "<reason>"` restricted to a human operator or, for a single narrow stale-review case, the `rdm-dispatch-phase` orchestrator) — canonical: [`docs/core-enforced-gates.md`](docs/core-enforced-gates.md).

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

A fifth target reviews **code** rather than a document: `--on change/<sha>` (or `--on change/HEAD` from inside a source checkout) pins the reviewed commit, and comments anchor into the source repo with `--path <repo-relative path> --quote "exact text"`, restricted to hunks the change touches. Canonical: [`docs/change-reviews.md`](docs/change-reviews.md).

### Linking

Bodies can carry `rdm:` links — write them as ordinary Markdown links, e.g. `[the auth roadmap](rdm:roadmap/auth)`. Three item-link forms, using the same identifiers as `Done:` lines and `review --on`:

- `rdm:roadmap/<slug>`
- `rdm:phase/<roadmap-slug>/<stem-or-number>`
- `rdm:task/<slug>`

And one pinned code-link form, for a specific file (optionally a revision and line range) in the project's configured `source` repo:

- `rdm:src/<path>[@<rev>][#Lstart[-Lend]]` — e.g. `rdm:src/rdm-core/src/link.rs@a1b2c3d#L42-L58`

```bash
./target/debug/rdm link check --project rdm                       # validate every rdm: link in the project (CI-friendly, nonzero on anything broken)
./target/debug/rdm link check --on task/<slug> --project rdm      # scope the check to one document
./target/debug/rdm link list --on phase/<slug>/<stem> --project rdm  # a document's outgoing links, resolved
./target/debug/rdm link resolve rdm:task/fix-login --project rdm  # resolve a single URI given on the command line
./target/debug/rdm backlinks task/fix-login --project rdm         # documents that reference this item
```

Rules: use exact slugs/stems as printed by rdm, never invented or paraphrased — run `./target/debug/rdm search <topic> --project rdm` first if unsure an item exists. Pin a code link with the revision from `git rev-parse HEAD` in this source repo, or omit `@rev` inside a phase/task body to fall back to that item's own stamped `commit` field once one is recorded. Run `./target/debug/rdm link check --on <ref> --project rdm` before finalizing a body edit and treat a nonzero exit — a dangling item link, or a code link whose path doesn't exist at its pinned revision — as blocking.

### Plan review

A second, earlier gate than the document-review flow above: it reviews a roadmap/phase/task's **plan** before implementation begins, rather than the diff after implementation. Controlled by the `plan_review` config flag (`./target/debug/rdm config set plan_review true`, `RDM_PLAN_REVIEW` env override, default `false`) — enabled for this repo's own plan data. While the flag is on, `roadmap create` / `phase create` / `task create` automatically stamp a reserved `needs-plan-review` tag onto every new item, alongside any user-supplied `--tags`.

List pending items with:

```bash
./target/debug/rdm search "" --tag needs-plan-review --project rdm
```

Run the `rdm-plan-review` skill against a pending item to review it: it dispatches parallel read-only sub-agents for coherence, architectural fit, restraint, and (for phases) unit-of-work sizing, then consolidates a **PASS** / **PASS WITH CONCERNS** / **REWORK** verdict. On PASS or PASS WITH CONCERNS it clears the `needs-plan-review` tag; on REWORK it leaves the tag in place and reports what must change. Both the plan-review Stop hook (`.claude/hooks/rdm-plan-review-on-create.sh`) and the earlier needs-review Stop hook are now retired — nothing reprompts automatically. Clearing `needs-plan-review` on items created via `rdm-roadmap`, ad hoc create commands, or `rdm-do` side-task filing is manual-only: run `rdm-plan-review` against the item, or periodically sweep with `rdm search "" --tag needs-plan-review`. Active enforcement is tracked as a follow-up task (`wire-active-plan-review-tag-gate`) filed in the plan repo.

The gate's self-review decision (may a session clear the tag on a plan it authored? — yes, because the verdict comes from independent finders/refuters, with a stated boundary), and the three recorded classifier blocks behind it, live in [`docs/plan-review-gate-policy.md`](docs/plan-review-gate-policy.md). `rdm-wf-plan-review` never writes the tag — it always returns `gateAction.commands` and the caller runs them, which is what `gateMode: 'return'` used to opt into. A unit still awaiting that write carries a `[gate pending: …]` clause on its summary and is counted by `gatePendingCount`; one whose current tag list the caller did not supply carries `tagsUnknown: true` and no commands, because `--tags` replaces the whole list.

This gate composes with, and is independent from, the existing `needs-review` gate above: `plan_review`/`needs-plan-review` gates **before** implementation begins (on the plan document), while `rdm-review`/`needs-review` gates **after** implementation (on the diff). Neither the plan-review Stop hook (`rdm-plan-review-on-create.sh`) nor the needs-review Stop hook (`rdm-review-on-finalize.sh`) is active in this repo any longer.

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

`status`/`commit`/`discard` are scoped to **your own session's changeset**, not the whole plan repo: a concurrent session's uncommitted work is never swept into your commit nor destroyed by your discard, `rdm status` names it on a separate trailing line, and `--all` / `--changeset <id>` are the escape hatches. Session identity resolves automatically and outlives a single process (override with `RDM_SESSION`); inspect it with `rdm session id|list|journal`. Canonical: [`docs/session-identity.md`](docs/session-identity.md) (implementation + CLI surface) and [`docs/scoping-model-decision.md`](docs/scoping-model-decision.md) (the binding decision record).

```bash
./target/debug/rdm task create fix-bug --title "Fix bug" --no-edit --project rdm  # writes file, no git commit yet
./target/debug/rdm status                          # show this session's staged changes
./target/debug/rdm commit -m "batch: fix bug and update phase"  # lands this session's changeset
./target/debug/rdm discard --force                 # restore this session's paths to HEAD (destructive)
```
