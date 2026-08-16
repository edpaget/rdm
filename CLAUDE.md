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

**FORBIDDEN, categorically: no test, harness, or CI step may assert on `CHANGELOG.md`.** Not that `[Unreleased]` contains a word, names a file, or describes a feature; not that it is co-staged with a code change; and not a planted-mutation self-test over the changelog body. Release automation moves the whole `[Unreleased]` body into a versioned section, so any such check goes red on `main` the moment a release lands — this blocked v0.18.1. The changelog rule above is enforced by review, not by a gate. Assert on the code or the emitted artifact, never on the prose describing it.

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

## Agent Distribution

rdm's autonomous lane (skills and workflows) is distributed via two channels:

### Recommended: Claude Code Plugin Marketplace

Downstream consumers should install rdm via the plugin marketplace:

```bash
claude plugin marketplace add edpaget/rdm
claude plugin install rdm@rdm
```

This installs 11 skills (`rdm:roadmap`, `rdm:dispatch-phase`, etc.) and 2 workflow engines (`rdm:rdm-wf-dispatch-phase`, `rdm:rdm-wf-review-refute-fix`). The skills invoke bare `rdm` at runtime; the plugin shims resolve the rdm binary via the `--rdm-bin` flag, the `RDM_BIN` environment variable, or PATH lookup.

**This repo does not install its own plugin.** It runs the local `.claude/` lane instead — a deliberately divergent surface that supports development and testing. See `docs/plugin-distribution.md` § "Which copy runs?" for the three surfaces and why this repo's configuration differs from downstream consumers.

### Fallback: Raw Skills Emission

For users unable to use the plugin marketplace:

```bash
rdm agent-config claude --skills --out <dir>
```

This emits 11 skills and 2 workflow engines to a target directory, with no collision protection or namespace prefixing. Raw emission is **not** the recommended path and carries higher friction — consumers must manage skill naming, binary path resolution, and workflow discovery manually.

The plugin marketplace is recommended for these reasons: automatic namespace prefixing avoids collisions, automatic workflow discovery, and a single installed entity vs. 13 separate file copies.

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
- Token-lane measurement harness: `bash scripts/verify-token-report.sh` — hermetic regression for `scripts/lib/token-report.mjs` + `scripts/measure-lane-tokens.mjs`, the stdlib-only Node tool that measures token usage across Claude Code Workflow session sidecars (`~/.claude/projects/**/workflows/wf_*.json` + their `subagents/workflows/<runId>/agent-*.jsonl` transcripts) so later phases of the `workflow-token-reduction` roadmap can substantiate a token-saving claim instead of asserting one. Covers: `requestId`-deduped usage (last-write-wins, verified against a planted duplicate), a token-class breakdown (output/uncached-input/cache-write/cache-read) rather than a single conflated total, grouping by agent class/full label/model/workflow, cross-project-slug session location including `--worktrees-`-named directories, the never-reconciled sidecar-`totalTokens`-vs-deduped-sum discrepancy line, a no-transcript/unreadable-transcript/empty-transcript agent degrading to a distinct warning instead of throwing, and two independent planted-mutation self-tests. Section 6 additionally gates the sibling instrument `scripts/measure-refuter-severity.mjs` (refuter spend by graded finding severity — the evidence behind the non-gating-refutation skip) against `tests/fixtures/token-refuter-severity/`, plus a corpus-free `--audit` of the committed `docs/token-baseline.json` § `nonGatingRefutationSkip` figures. Run it after touching `scripts/lib/token-report.mjs`, `scripts/measure-lane-tokens.mjs`, `scripts/measure-refuter-severity.mjs`, or either fixture tree.

- Refuter-agreement harness: `bash scripts/verify-refuter-agreement.sh` — hermetic gate for the on-demand refuter model-tiering instrument (`scripts/lib/refuter-agreement.mjs`, `scripts/mine-refuter-corpus.mjs`, `scripts/run-refuter-agreement.mjs`) and its 56-item adjudicated finding corpus, which decides whether refuters must stay on Opus. Run it after touching any of those three scripts or `tests/fixtures/refuter-agreement/`; the method, the numbers, the `rdm-wf-plan-review.js` model-omission answer and the decision live in `docs/refuter-model-tiering.md`. The same instrument also A/Bs refuter *shape* (one refuter per gating finding vs one per dimension) — outcome `no-measurement`, pipeline unchanged; figures and the unit-scoped power analysis (`--batch-power`) live in `docs/refuter-batching.md`.
- Finder-collapse harness: `bash scripts/verify-finder-collapse.sh` — hermetic gate for the collapsed-plan-finder A/B instrument (`scripts/lib/finder-collapse.mjs`, `scripts/mine-plan-finder-corpus.mjs`, `scripts/run-finder-collapse.mjs`, `tests/fixtures/finder-collapse/`), which measured whether plan mode's three always-on lenses can run in ONE agent — outcome `no-ship`, pipeline unchanged, figures in `docs/finder-collapse.md` and `docs/token-baseline.json` § `planFinderCollapse`. Run it after touching any of those, and note it enforces a decision/pipeline XOR: a merged plan dimension can never coexist with a no-ship figure.

### Workflow-tool orchestration (autonomous lane)

The autonomous do/autopilot lane has migrated from prose-orchestrated skills to deterministic **Claude Code Workflow-tool** scripts under `.claude/workflows/` (a sibling of `.claude/skills/` and `.claude/hooks/`). `rdm agent-config claude --skills --out <dir>` now **emits both halves of this lane**: the thin `rdm-dispatch-phase` skill shim (`rdm-core/src/templates/skill-dispatch-phase-{cli,mcp}.md`) AND the two `.claude/workflows/*.js` scripts it and the prose `rdm-autopilot` skill invoke (`rdm-wf-dispatch-phase.js`, `rdm-wf-review-refute-fix.js`), embedded via `rdm-core/src/templates/workflows/` and `generate_workflows`/`write_workflows`, byte-identical to this repo's own `.claude/workflows/` copies. The autopilot drive loop itself is no longer a Workflow script — it is the prose `rdm-autopilot` skill (`.claude/skills/rdm-autopilot/SKILL.md`), which calls `rdm-wf-dispatch-phase` and `rdm-wf-estimate` directly; there is no shipped `autopilot.js` template. The two emitted engines are project- and binary-agnostic (`rdmBin`/`project` are runtime args); `lib/*.mjs` and the generator scripts (`scripts/gen-workflow-review.sh` and friends) remain unshipped — see `docs/workflow-schemas.md`'s Scope callout for the exact boundary. The interactive skills (`rdm-do`, `rdm-roadmap`, `rdm-revise`) stay on skills because workflows are headless and cannot pause for human approval gates.

- Conventions and the canonical `FINDING` / `VERDICT` / `OUTCOME` schema contracts live in `docs/workflow-schemas.md`. A workflow script (`.js`) starts with `export const meta` and uses the ambient `agent()`/`pipeline()`/`parallel()`/`log()` globals; a canonical source module (`lib/*.mjs`) holds shared logic once and is Node-importable for testing.
- **The Workflow runtime cannot `import`/`require`** (proven by the P1 import spike, recorded in `docs/workflow-schemas.md`). Shared pipeline logic is therefore kept single-source in `.claude/workflows/lib/` and stamped verbatim into consumers by `scripts/gen-workflow-review.sh` (run it after editing a `lib/*.mjs`; `--check` mode gates drift). This is a compile-time copy, not a cross-`workflow()` call, so it does not consume the one-level `workflow()` nesting budget.
- `.claude/workflows/lib/review.mjs` is the **single canonical review implementation** every surface consumes — find → refute → filter → verdict → gate policy. `buildReviewPipeline(mode)` runs parallel dimension finders → BARRIER → a fresh refuter per GATING finding, bounded by a per-unit **refutation budget** (`maxRefutations`, default 5 from `docs/token-baseline.json` § `determiningFindingRank`; overflow is passed through marked `unrefutedReason: 'budget'`, still floor-filtered, so the bound can only ever add rework, never remove it) — a `suggestion` gates at no tier so it is passed through marked `unrefutedReason: 'non-gating'` and consumes no budget; unknown severities are refuted, fail-safe; a crashed refuter marks its finding `refuterError: true` → drops refuted-or-low-confidence → returns ranked survivors plus the budget accounting, behind `code` (ac/correctness always-on; tests/architecture/api-docs/changelog/security triggered) and `plan` (coherence/architectural-fit/restraint always-on; unit-of-work phases-only) modes; `classifyOutcome` yields the canonical `reviewed | rework | escalated` vocabulary, and `statusFor`/`writesCompletion` map it onto rdm statuses (escalated → `blocked` for phases *and* tasks). It has TWO `--check`-gated projections: `scripts/gen-workflow-review.sh` stamps its JS block into the workflow consumers, and `scripts/gen-skill-review.sh` renders its `//|` spec prose into the shipped skill templates. That skill projection is **mode-dispatched**: `--mode code` renders `skill-review-{cli,mcp}.md` and `--mode plan` renders `skill-plan-review-{cli,mcp}.md` from the SAME regions, selected by an optional per-line mode tag (`//|` shared, `//|code|`, `//|plan|`); BOTH modes are `--check`-gated by `scripts/verify-workflow-review.sh`. The gate is likewise mode-dispatched policy, not a fork — one `GATE_POLICY[mode][outcome]` table (code: persist an rdm status + permit the completion trailer; plan: clear `needs-plan-review` on `reviewed`, leave it on `rework`/`escalated`, never persist a status), with `STATUS_MAPPING` being exactly `GATE_POLICY.code`. `.claude/workflows/rdm-wf-review-refute-fix.js` has three invocation shapes over the same stamped pipeline: `mode: 'plan'`, and `mode: 'code'` with no item identifier, keep the legacy survivors-only `{ mode, survivors }` shape; `mode: 'code'` with `{ roadmap, phase }` or `{ task }` derives real diff signals from the item's worktree (fail-open, mirroring dispatch-phase's code gate) and composes ONE `classifyOutcome` call over the survivors into the same dispatch-shaped OUTCOME, with an optional mechanical `gate: true` status-persist step for headless/ad hoc callers only (never wired into the interactive `rdm-review` skill, which invokes it with `gate: false` and keeps its own gate). The land-time `Done:` trailer is never written by stamped workflow JS or by `rdm-wf-review-refute-fix.js`'s own optional gate: its format lives in `rdm_core::hook::format_done_directive` (surfaced as `rdm hook done-line`) and is written by the review skill's gate step or by `rdm-land` at land time. `.claude/workflows/rdm-wf-plan-review.js` is the plan-mode standalone consumer: it reuses `buildReviewPipeline('plan')` + `GATE_POLICY.plan` over all four target types (`--task`, `--roadmap`, positional `<slug> [phase]`, `--implementation-plan`), fanning out per-phase with `parallel()` under `--roadmap` and gating each unit independently. It threads a minimal `signals: { targetType }` object per review unit into `buildReviewPipeline('plan')`, so `unit-of-work`'s `when: targetType === 'phase'` predicate is evaluated at selection time instead of fail-opening; the stamped helper `stripNonPhaseUnitOfWork(survivors, targetType)` remains as a defense-in-depth backstop rather than the primary scoping mechanism. The gate clears `needs-plan-review` via `filterPlanReviewTag(tags)` (sibling-preserving read-filter-write) and classifies each unit via `classifyPlanOutcome(survivors)` — all three added inside review.mjs's stamped block and exported for the harness. `--implementation-plan` has no persisted item, so it skips both the act half and the gate. Its DRIVER (argument parsing `parsePlanArgs`, the fetch/act/gate prompt builders, and the dependency-injected orchestration `runPlanReviewDriver`) is single-sourced in `.claude/workflows/lib/plan-review.mjs` and copied BYTE-IDENTICAL into `rdm-wf-plan-review.js`'s `plan-review-driver` block — like dispatch-phase's `dispatch-outcome` block, this copy is NOT stamped by the generator; `verify-workflow-review.sh` gates the two for byte-equality and imports the lib to EXECUTE the driver against a fake agent/parallel (the workflow file holds only the verbatim copy plus a thin `return await runPlanReviewDriver(args, …)` runtime entry that stays outside the block).
- Review pipeline harness: `bash scripts/verify-workflow-review.sh` — hermetic DRIFT / HYGIENE / BEHAVIOR regression. It runs `gen-workflow-review.sh --check` and `gen-skill-review.sh --check` in **both** `--mode code` and `--mode plan` (with planted-drift/heal self-tests and mode-isolation greps in both directions), greps the workflow scripts for forbidden `Date.now(`/`Math.random(`, and drives the shared module in Node (stdlib-only, no packages; `node` pinned in `.mise.toml`) with an injected fake agent + reference `pipeline`/`parallel`, asserting a planted refutable finding is dropped, a real one survives, the confidence floor drops low-confidence findings, a fresh refuter grades each finding, context is threaded into every prompt, and output is deterministic across both modes. It also drives the plan-standalone path (section 5): the three consolidation helpers (`stripNonPhaseUnitOfWork` phase-only scoping, `filterPlanReviewTag` sibling-preservation, `classifyPlanOutcome` reviewed/rework/escalated), a seeded per-phase independent-gate scenario, the `--implementation-plan` no-gate carve-out, static greps over `rdm-wf-plan-review.js` (four target types, `parallel()` fan-out, core reuse, the implementation-plan act+gate guards), a byte-drift gate on the `plan-review-driver` block (`lib/plan-review.mjs` vs `rdm-wf-plan-review.js`, section 5b-drift), an EXECUTABLE driver section that imports `lib/plan-review.mjs` and drives `parsePlanArgs` + `runPlanReviewDriver` against a fake agent/parallel (four-target precedence across three arg shapes, malformed-JSON fallback, no-target throw, fail-closed on an unread plan, per-unit independent act+gate, single-target flattening, and the impl-plan no-act/no-gate carve-out — section 5b-exec), and planted-mutation self-tests on the two AC helpers plus `parsePlanArgs` precedence (section 6). `rdm-wf-plan-review.js` is a `gen-workflow-review.sh` consumer, so the section 1b scratch tree copies it too. Run it after touching `.claude/workflows/lib/review.mjs`, either generator, or the workflow/skill consumers (`rdm-wf-review-refute-fix.js`, `rdm-wf-dispatch-phase.js`, `rdm-wf-plan-review.js`).
- Rendered-listing observer: `bash scripts/observe-workflow-listing.sh` — NON-hermetic (needs the `claude` CLI): captures the real skill/slash-command listing from a fresh `claude -p` rooted here and asserts the `rdm-wf-` engine-prefix contract against it. Run it deliberately when engine names change. `--self-test-only` is the hermetic half `verify-workflow-review.sh` §2d runs. See `docs/workflow-schemas.md` § "Observing the rendered listing".
- Dispatch-phase harness: `bash scripts/verify-workflow-dispatch.sh` — hermetic regression for dispatch-phase's 4-stage pipeline and its reviewed/rework/escalated outcome branches, plus the byte-identical-copy drift gate against `lib/dispatch-phase.mjs`.
- Autopilot harness: `bash scripts/verify-skill-autopilot.sh` — hermetic regression for the prose `rdm-autopilot` skill (`.claude/skills/rdm-autopilot/SKILL.md`), which replaced the JS `autopilot.js`/`lib/autopilot.mjs` Workflow loop (`prose-autopilot-orchestration` roadmap, phase 3): static text invariants for the surviving loop policies (drive-to-reviewed, rework-retry-once-then-park, escalated→park, budget stops, estimate-pre-pass-always-runs, `--plan-only` dedup), a dynamic advance/park write+read-back contract against the real binary, the `rdm-wf-dispatch-phase`/`rdm-wf-estimate` sibling-harness gate, and the land-time completion-trailer contract. There is no `lib/autopilot.mjs` anymore, so there is no byte-identical-copy drift gate to run.
- Estimate-workflow harness: `bash scripts/verify-workflow-estimate.sh` — hermetic regression for the headless `rdm-wf-estimate` Workflow and the shared `estimate-core` block (list → filter-to-unestimated → parallel-rate → writeback the difficulty + a `## Estimate <difficulty> — <justification>` audit note → read the tier back from rdm-core): pure-helper behavior (arg parsing, `selectUnestimated`, the estimator/writeback/list/tier prompt contents asserting the note, `--difficulty` + `--body`, and NO `--model` on the update command), a driven-pipeline section fed state-backed fakes (rates ONLY unestimated stems, threads the justification, reports the read-back tier not a JS map, narrows to one phase number, is idempotent on re-run, is deterministic, and degrades gracefully on a failed-writeback `ok:false` or a thrown rater/writeback/tier read-back), the `gen-workflow-estimate.sh --check` drift gate stamping `estimate-core` into its sole consumer (`rdm-wf-estimate.js`) with a planted-drift/heal self-test, static invariants (module parse, no import/require, no `Date.now(`/`Math.random(`, no `difficultyToTier` anywhere under `.claude/workflows/`, no top-level `type:'array'` schema, `meta.phases` parity, and the `rdm-estimate` thin-shim SKILL.md referencing `rdm-wf-estimate.js` with no retired rating-loop prose), and a hermetic-seed section seeding a temp git-backed plan repo via the REAL `target/debug/rdm` binary and feeding its actual `phase list`/`phase show --format json` output through `selectUnestimated`/`buildEstimatePipeline` (backing AC1's real-binary claim and catching drift between the JS's assumed field names and rdm-core's emitted JSON). Run it after touching `.claude/workflows/lib/estimate.mjs`, `scripts/gen-workflow-estimate.sh`, `rdm-wf-estimate.js`, the autopilot estimate pre-pass (now in the prose `rdm-autopilot` skill), or the `rdm-estimate` skill shim.
- `rdm-do --auto` wiring harness: `bash scripts/verify-workflow-do-auto.sh` — hermetic regression for the `rdm-do --auto` phase-flow wiring into dispatch-phase (SKILL.md static invariants, the OUTCOME→status contract against the real binary, prose-only distributed-template self-test).
- `rdm-do --auto --task` wiring harness: `bash scripts/verify-workflow-do-auto-task.sh` — hermetic regression for the `rdm-do --auto` task-flow wiring into dispatch-phase (SKILL.md static invariants, the task-shaped OUTCOME→status contract against the real binary, prose-only distributed-template self-test).
- Distribution-boundary harness: `bash scripts/verify-agent-config-distribution.sh` — hermetic regression proving `rdm agent-config claude --skills` (both the plain CLI and `--mcp` variants) emits a self-consistent, working autonomous lane into a downstream repo: workflow-script byte-identity against `.claude/workflows/*.js`, skill path/frontmatter validity for all 11 skills, cross-file shim-reference resolution (every literal `.claude/workflows/<name>.js` mention inside an emitted skill resolves to a real file in the same emitted tree, with an occurrence floor so the check can't pass vacuously), Pi/`--user` scope negatives, plan-repo independence, and two planted-corruption self-tests (a workflow byte, a shim reference typo) proving neither gate is vacuous. Its § 7 additionally proves the emitted lane WORKS downstream, not merely that it matches: a hermetic non-rdm, non-Rust fixture repo (own binary path, own plan repo, own project), importable modules extracted from the EMITTED scripts (top-level `return` neutralized, explicit export block appended, inverse transform proving byte-identity with the emitted file), their pure logic executed against the fixture's real diffs, one built rdm command actually run against the fixture plan repo, and four planted corruptions in the emitted bytes. Run it after touching `agent_config.rs`'s `generate_skills`/`generate_workflows`, `rdm-cli`'s `--out` wiring, any `skill-*.md` template, or any `.claude/workflows/*.js` source.
- Plugin-distribution harness: `bash scripts/verify-plugin-distribution.sh` — the plugin-tree sibling of the distribution-boundary harness above, gating `rdm agent-config claude --plugin --out <dir>`'s emitted Claude Code plugin tree: manifest validity (`name`/`version`/`description`/`author`, no `workflows` key) and layout (skills at `skills/<name>/SKILL.md`, workflows at `workflows/<name>.js`, both plugin-root siblings of a `.claude-plugin/` holding nothing but the manifest), the Phase-1-recorded naming transform (skill dirs drop the `rdm-` prefix, engine files keep `rdm-wf-`), namespaced `rdm:<engine>` reference resolution with an occurrence floor, scope negatives for every rejected flag combination (`--plugin --skills`, `--plugin` with no destination, `--plugin --user`, `--plugin` on each non-Claude platform, and the Pi/`--mcp` precedence case) each asserted against its own distinct message rather than a generic reuse, and a planted-corruption self-test behind every assertion. Never invokes the `claude` CLI or asserts installability — that is the installability pair below. Run it after touching `agent_config.rs`'s `generate_plugin_*`/`PLUGIN_SKILL_NAMES`, `rdm-cli`'s `--plugin` wiring, or `docs/plugin-distribution.md`'s naming/layout decisions.
- Plugin-installability pair (the checked-in tree at `plugins/rdm/` + the repo-root `.claude-plugin/marketplace.json`): (a) `bash scripts/verify-plugin-install.sh` — hermetic, pure POSIX shell, no `python3`/`node`/`jq`/`claude`, **RUN BY CI** via the `scripts/verify-*.sh` glob. Gates version-normalized drift of `plugins/rdm/` against `rdm agent-config claude --plugin` output (the manifest `version` is normalized on BOTH sides so a release-time crate bump cannot red-light `main`), the manifest-version-equals-crate-version assertion against FRESH output only, marketplace shape + `source` resolution with a non-empty-entry floor, workflow byte-identity, and the 11-skill inventory with frontmatter validity — each behind a planted-corruption self-test. (b) `bash scripts/observe-plugin-install.sh` — NON-hermetic (needs the `claude` CLI), deliberately OUTSIDE the glob and **NOT run by CI**; does a real offline `validate --strict` → `marketplace add` → `install rdm@rdm` into an isolated `CLAUDE_CONFIG_DIR`, asserting installed skills/workflows on the filesystem and that the real `~/.claude` is unchanged; exits `2` with a NOTICE when `claude` is absent. Run both after touching `plugins/rdm/`, `.claude-plugin/marketplace.json`, `agent_config.rs`'s `generate_plugin_*`, or the `--plugin` CLI wiring — and **regenerate `plugins/rdm/` rather than hand-editing it** (`env -u RDM_ROOT -u RDM_PROJECT cargo run -q -- agent-config claude --plugin --out plugins/rdm`; the omitted `--project` is load-bearing). Detail lives in `docs/plugin-distribution.md`.
- Standalone review-outcome harness: `bash scripts/verify-workflow-review-outcome.sh` — hermetic regression for `rdm-wf-review-refute-fix.js`'s full `{ roadmap, phase }`/`{ task }` code-review path: byte-identical-copy check against the shipped template, driver-region-scoped structural checks (exactly one `buildReviewPipeline('code')` binding and one `classifyOutcome(` call below the `review-refute-fix:end` marker, with planted-mutation self-tests and a passing-on-real-file regression guard — scoped to the driver region because the stamped block above the marker already declares `classifyOutcome`'s definition), a whole-file hygiene grep (no `Done:`/`Date.now(`/`Math.random(`), a Node simulation driving the real driver under injected fakes (the full OUTCOME shape for `reviewed`/`rework` seeds, diff-signals fail-open vs. a real diff's `deriveSignals` threading, the `task`-vs-`roadmap`/`phase` mutual-exclusion guard, both legacy backward-compatible shapes, and the optional headless `gate` step), and a check that `.claude/skills/rdm-review/SKILL.md` still references the workflow while retaining its interactive Report/Act/Gate prose and completion-trailer mechanism. Run it after touching `rdm-wf-review-refute-fix.js`'s driver region or the `rdm-review` skill.
- Backlog-workflow harness: `bash scripts/verify-workflow-backlog.sh` — hermetic regression for the headless `rdm-wf-backlog` Workflow (the propose-only grooming pass over `rdm backlog report`'s four signal arrays): pure-helper and driven-pipeline behavior fed fakes (empty-report short-circuit with zero analyzer calls, a fully-populated report producing all four batch subsections, a fetch error propagating rather than being laundered into "Nothing to groom", a single analyzer crash degrading gracefully), a ZERO-MUTATION section against a real seeded plan repo (git HEAD/status/file-checksum identity before and after a run), the byte-identical-copy drift gate against `lib/backlog.mjs` (with a planted-mutation self-test), and static invariants (exactly one Bash-executing agent directive whose command template is provably read-only, no `Date.now(`/`Math.random(`, `meta.phases` parity). Run it after touching `.claude/workflows/lib/backlog.mjs`, `rdm-wf-backlog.js`, or the `rdm-backlog` skill shim.
- Document-workflow harness: `bash scripts/verify-workflow-document.sh` — hermetic regression for the headless `rdm-wf-document` Workflow (the all-done validation, per-phase `parallel()` git-gather with has-SHA/body-only fallback, synthesis, and disk-write pipeline behind `rdm-document`): static invariants (the `parallel()` fan-out, `rdm roadmap show`/`rdm phase show --format json` wiring, the abort-on-incomplete and has-SHA/body-only branches, no `--status` mutation or plan-mode call, no `Date.now(`/`Math.random(`, and that the skill shim retains its terminal "not done until reviewed and approved" human-approval language while dropping the old step-by-step git-gather prose), the byte-identical-copy drift gate against `lib/document.mjs` (with a planted-mutation self-test), Node-driven pure-logic behavior tests (`parseDocumentArgs`, `defaultOutPath`/`resolveOutPath`, `computeIncompletePhases`, `buildGitRangeCommands`), and a hermetic seed against a real temp plan repo plus a real temp source repo (a fully-done roadmap with real commit SHAs, a roadmap with an incomplete phase, and a done phase with no recorded commit) feeding real `rdm roadmap show`/`phase show --format json` output through the same pure functions. Run it after touching `.claude/workflows/lib/document.mjs`, `rdm-wf-document.js`, or the `rdm-document` skill shim.

#### `.claude/agents/` — the custom-agent registry, now a shipped emission surface

`.claude/agents/` is the custom-agent registry `agent()`'s `opts.agentType` resolves against. It
holds one definition, `rdm-mechanical.md`. The **four local-only workflows** (`rdm-wf-document.js`,
`rdm-wf-backlog.js`, `rdm-wf-estimate.js`, `rdm-wf-plan-review.js`) thread it at their mechanical call sites; the
two distributed workflows and every judgment site must not. `scripts/verify-workflow-review.sh`
§2c asserts both directions with planted-mutation self-tests. Resolution is confirmed on the
Workflow path and the trim measured at **8907 tokens/agent (−23 %)** — roughly half the 19894
the `claude -p` 2×2 predicts, so quote 8907 for these sites.
`rdm-core/src/agent_config.rs`'s `generate_agents()` now ships `rdm-mechanical.md` into every
`rdm agent-config claude --skills`-produced downstream tree (`ship-mechanical-agent-type-downstream`),
so a distributed workflow template CAN reference it — none does yet, since threading a distributed
site is a separate, not-yet-landed follow-up. `scripts/verify-agent-config-distribution.sh` § 3c
resolves any such reference against the emitted set. Evidence, measurements and disposition live
in `docs/workflow-schemas.md` § "agentType / effort options spike" — that section is canonical; do
not restate its tables here.

Note when adding an agent definition: Claude Code watches `.claude/agents/`, but the watcher
covers only directories that existed at session start, so creating a scope's **first** agent
file in a new `agents` directory needs a restart before it resolves. Project definitions are
also discovered by walking up from the cwd, so a session rooted outside this repo will not see
this one.

One rule remains, gated by `scripts/verify-workflow-review.sh` §2b with a planted-mutation
self-test:

- **No workflow script may pass `effort:`.** The reason is *scope*, not mechanism —
  `effort: 'low'` at a call site **is** honored. Lifting this is owned by
  `finish-agent-type-effort-spike-and-thread-mechanical-sites`.

**When editing this file:** the project `CLAUDE.md` is loaded into every subagent, including a
custom-`agentType` one, and cannot be suppressed per agent type — it was measured at 19320
tokens (2.49 chars/token, against a 48207-char file). Every paragraph added here is paid for once
per dispatched agent, so keep additions here short and put the detail in `docs/`.

#### Two surfaces: workflow vs skill

Decision rule for which surface to reach for:

The autonomous lane's completion trailer is written at **land time**: `rdm-wf-dispatch-phase` emits `writesCompletion: true` on a `reviewed` OUTCOME and never writes the directive itself (the prose `rdm-autopilot` loop that dispatches a phase carries the same OUTCOME through, unread for this field); `rdm-land` is the only reader — it synthesizes the trailer from the OUTCOME identifiers via `rdm hook done-line`, amending it before the rebase — so a landed autopilot branch never needs a manual rebase to gain it.

- Reach for a **workflow** when the unit is fan-out-shaped, mechanism rather than policy, headless (no *mid-run* gate), deterministic, and hermetically gatable: dispatching one phase (`rdm-wf-dispatch-phase`, takes `{ roadmap, phase }` or `{ task }`, plus optional `maxRefutations?`), the review pipeline (`rdm-wf-review-refute-fix`, `rdm-wf-plan-review`), the `rdm-wf-estimate` fan-out, and the batched passes (`rdm-wf-backlog` takes `{ project?, olderThan?, tag? }`; `rdm-wf-document`). Entry points: the `Workflow` tool, `rdm-do --auto <roadmap> <phase>`, or the thin skill shims.
- Reach for a **skill** when a human is in the loop for plan approval or discussion, or when the unit is a low-iteration sequential driver that is mostly policy: interactive `rdm-do` (no `--auto`), `rdm-roadmap`, `rdm-revise`, `rdm-plan-review`, `rdm-review`, `rdm-estimate`, `rdm-land`, `rdm-document`.
- The autopilot drive loop has migrated to prose (an orchestrating `rdm-autopilot` skill, `.claude/skills/rdm-autopilot/SKILL.md`) under the `prose-autopilot-orchestration` roadmap; phase 3 retired `autopilot.js`/`lib/autopilot.mjs` and its harness `scripts/verify-workflow-autopilot.sh` in favor of `scripts/verify-skill-autopilot.sh`. The `rdm-wf-dispatch-phase` and `rdm-wf-estimate` Workflows it drives are unaffected.
- Canonical: [`docs/workflow-vs-prose-boundary.md`](docs/workflow-vs-prose-boundary.md) — criteria, all eight scripts' dispositions, non-goals.

The `distribute-workflow-lane` roadmap is the distribution follow-up this section used to describe as not-yet-created: its phase 2 re-authored the distributed skill templates (`rdm-core/src/templates/skill-{autopilot,dispatch-phase}-{cli,mcp}.md`, plus the `--auto` section of `skill-do-{cli,mcp}.md`) as thin Workflow-invoking shims, deleting the now-superseded "Mandatory dispatch"/inline-collapse checklists (written before the workflow scripts existed) while preserving every real behavioral guardrail (e.g. the `--permission-mode auto` safety rules). The local `.claude/skills/rdm-autopilot` copy was already a shim (rewritten during phases 2–3 of the earlier `workflow-orchestration` roadmap); `.claude/skills/rdm-dispatch-phase` was NOT yet a shim before this phase — that earlier claim was itself stale — and is now regenerated to match the corrected template, alongside `.claude/skills/rdm-do`.

**Hook reconciliation (verified read-only, no code changes):** `scripts/verify-worktree-review-loop.sh` drives the real hook scripts / `rdm review pending` off directly-set rdm state (status/tags), never off a specific driver — a green run shows the `rdm review pending`/`restamp` scoping the retired needs-review hook used to depend on is agnostic to whatever set that state, workflow or skill.

**Update (unify-code-review phase 6 → phase 7):** phase 6 made review active in every lane that can produce a `needs-review` item — `rdm-wf-dispatch-phase`'s code-review stage is the canonical review (stamped from `.claude/workflows/lib/review.mjs` and fed diff-derived `signals`) and returns a `reviewed`/`blocked` status as OUTCOME data rather than persisting it; the prose `rdm-autopilot` skill's own advance/park Bash steps persist that status directly on dispatch-phase's behalf, so nothing is left parked in `needs-review`, and interactive `rdm-do`'s finalize actively invokes `rdm-review` after the human confirm gate before the session stops. With nothing left unreviewed, phase 7 retired the now-redundant `needs-review` auto-review Stop hook / Pi `agent_end` extension outright (`.claude/hooks/rdm-review-on-finalize.sh`, `rdm-core/src/templates/hook-review-on-finalize.sh`, `rdm-core/src/templates/extension-review-on-finalize.ts`, and their `agent-config`/`--hooks` wiring) — see [`docs/autonomous-loop.md`](docs/autonomous-loop.md). The sibling `needs-plan-review` Stop hook / Pi extension has since also been retired (`unify-plan-review` roadmap, phase 4) — see "Plan review" below; `scripts/verify-plan-review-hook-loop.sh`, which tested that hook's own re-prompt loop, was deleted with it.

**Update (no-in-progress-stamp):** `rdm-wf-dispatch-phase` also stamps the target phase (or task) `in-progress` itself now — a best-effort, mechanical write right after Stage 0 (metadata + model resolution) and before planning begins. A `--plan-only` run skips it via the args-level `if (!planOnly)` guard, since a plan-only pass does no implementation and stamping would misreport it. The prose `rdm-autopilot` skill's own advance/park Bash calls are unaffected by this change — they still persist the terminal `reviewed`/`blocked` status themselves once dispatch-phase returns an OUTCOME, since dispatch-phase writes no terminal status of its own; the in-progress stamp above lives solely in the unit that actually works the item (`rdm-wf-dispatch-phase`), distinct from the terminal-status writes the invoking loop performs. This closes the one gap left after phase 6: a direct `Workflow` invocation of `rdm-wf-dispatch-phase` (and therefore every autopilot-driven phase) used to jump straight from `not-started` to a terminal status with no observable in-progress signal during the run; interactive `rdm-do`, `rdm-do --auto`, and the `rdm-dispatch-phase` skill already stamped in-progress before invoking the workflow. This is purely an observability fix, not a mutual-exclusion mechanism — see `docs/autonomous-loop.md`. It does not revisit the `needs-review`-is-dormant-by-design finding above.

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

Run the `rdm-plan-review` skill against a pending item to review it: it dispatches parallel read-only sub-agents for coherence, architectural fit, restraint, and (for phases) unit-of-work sizing, then consolidates a **PASS** / **PASS WITH CONCERNS** / **REWORK** verdict. On PASS or PASS WITH CONCERNS it clears the `needs-plan-review` tag; on REWORK it leaves the tag in place and reports what must change. Both the plan-review Stop hook (`.claude/hooks/rdm-plan-review-on-create.sh`) and the earlier needs-review Stop hook are now retired — nothing reprompts automatically. Clearing `needs-plan-review` on items created via `rdm-roadmap`, ad hoc create commands, or `rdm-do` side-task filing is manual-only: run `rdm-plan-review` against the item, or periodically sweep with `rdm search "" --tag needs-plan-review`. Active enforcement is tracked as a follow-up task (`wire-active-plan-review-tag-gate`) filed in the plan repo.

The gate's self-review decision (may a session clear the tag on a plan it authored? — yes, because the verdict comes from independent finders/refuters, with a stated boundary), and the three recorded classifier blocks behind it, live in [`docs/plan-review-gate-policy.md`](docs/plan-review-gate-policy.md). A caller too close to the plan passes `gateMode: 'return'` to `rdm-wf-plan-review`: the gate is computed and returned as `gateAction.commands` and nothing is written; a gate that should have cleared but did not comes back `gateBlocked: true` with a `[GATE BLOCKED: …]` clause on the summary.

This gate composes with, and is independent from, the existing `needs-review` gate above: `plan_review`/`needs-plan-review` gates **before** implementation begins (on the plan document), while `rdm-review`/`needs-review` gates **after** implementation (on the diff). Neither the plan-review Stop hook (`rdm-plan-review-on-create.sh`) nor the needs-review Stop hook (`rdm-review-on-finalize.sh`) is active in this repo any longer — see "Hook reconciliation" above.

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
