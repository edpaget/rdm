# Workflow-tool orchestration: conventions & schema contracts

The autonomous lane of rdm's tooling is expressed as **Claude Code Workflow-tool
scripts** under `.claude/workflows/`, a sibling of `.claude/skills/` and
`.claude/hooks/`. This document defines the conventions those scripts follow and
the canonical schema contracts they exchange.

> **Scope:** dogfood-only. These scripts live in this repo's `.claude/workflows/`
> and are **not** emitted by `agent-config`. rdm's shipped autonomous skills
> (`rdm-core/src/templates/skill-*.md`) remain the user-facing autonomous lane and
> are untouched. Distributing the workflow lane downstream is a follow-up roadmap.

## The `.claude/workflows/` convention

```
.claude/workflows/
  <name>.js              # a workflow script — invoked via the Workflow tool
  lib/<name>.mjs         # a canonical source module (Node ES module; see below)
```

- A **workflow script** (`.js`) begins with `export const meta = { … }` (a pure
  literal) and uses the ambient Workflow globals `agent()`, `pipeline()`,
  `parallel()`, `log()`, `phase()`, and `args`. It runs in the Workflow runtime,
  which supports top-level `await` and a top-level `return` (the workflow's
  result). It is **not** a standard Node module and is not `import`-able.
- A **canonical source module** (`lib/*.mjs`) is a real Node ES module. It holds
  shared pipeline logic **once**, between marker comments, and is `import`-able by
  the verify harness so its pure logic is unit-testable without the Workflow
  runtime. Its marked block is copied verbatim into workflow-script consumers by
  a generator (see next section).

### Import spike (why the generated-copy mechanism exists)

Phase 1 spiked whether the Workflow runtime can `import`/`require` a local helper
module, to decide how `review-refute-fix` is shared between the standalone
wrapper and dispatch-phase without a cross-`workflow()` call (which would exceed
the one-level `workflow()` nesting limit).

**Result: it cannot.** A zero-agent spike workflow tried `import()` of an absolute
`file://` URL, a bare absolute path, and a relative path — all three failed with
`import() is not available in workflow scripts.`, and `typeof require` was
`undefined`. Workflow scripts run in a sandbox with **no module resolution and no
filesystem access**.

**Chosen mechanism — single-source-of-truth generated copy.** The shared pipeline
is authored once in `lib/review-refute-fix.mjs` between
`review-refute-fix:begin` / `review-refute-fix:end` marker comments.
`scripts/gen-workflow-review.sh` extracts that block and stamps it **verbatim**
into each consumer between matching markers; `--check` mode asserts no consumer
drifted from the source, and `scripts/verify-workflow-review.sh` (and CI) run that
check. Editing happens in the lib; consumers are regenerated, never hand-edited.

This is distinct from a cross-`workflow()` call: sharing is a **compile-time copy**
of a helper block, not a runtime sub-workflow invocation, so it does not consume
the one allowed level of `workflow()` nesting. dispatch-phase (Phase 2) embeds the
same block in its plan-review and code-review stages the same way.

The lib exposes its bindings to Node via an `export { … }` statement placed
**outside** the markers, so the generator never copies it (a bare `export`
mid-body would break a workflow script, whose only permitted export is `meta`).
Dependency resolution (`agent`/`pipeline`/`parallel`/`log`) is deferred to call
time inside `buildReviewPipeline` via a `ReferenceError`-safe `typeof` probe, so
importing the module in Node — where those globals do not exist — never throws;
the harness injects fakes through the `deps` argument instead.

## Schema contracts

Workflow stages exchange schema-typed values. When an `agent()` call passes a
`schema`, the subagent is forced to return a matching object. The canonical
shapes below are defined as JSON Schema in `lib/review-refute-fix.mjs`
(`FINDINGS_SCHEMA`, `VERDICT_SCHEMA`); `OUTCOME` is the pipeline's return value.

### `FINDING`

One issue raised by a finder agent. Finders return `{ findings: FINDING[] }`.

| field           | type                                     | notes                                             |
| --------------- | ---------------------------------------- | ------------------------------------------------- |
| `id`            | string (required)                        | short stable slug, unique within the finder       |
| `concern`       | string (required)                        | the dimension key (`ac`, `correctness`, …)        |
| `location`      | string                                   | `file:line`, section heading, or phase stem       |
| `severity`      | `blocking` \| `concern` \| `suggestion`  | required; drives ranking and the overall verdict  |
| `confidence`    | integer 0–100 (required)                 | the finder's confidence **in the finding**        |
| `what_fails`    | string (required)                        | the specific problem                              |
| `why`           | string                                   | root cause / which rule, AC, or principle         |
| `recommendation`| string                                   | concrete fix                                      |

### `VERDICT`

A refuter agent's grade of a single `FINDING`. A **fresh** refuter grades each
finding — the finder never grades its own work.

| field        | type                     | notes                                                  |
| ------------ | ------------------------ | ------------------------------------------------------ |
| `refuted`    | boolean (required)       | `true` ⇒ the finding does not hold up ⇒ dropped        |
| `confidence` | integer 0–100 (required) | the refuter's confidence in **its verdict** (advisory) |
| `rationale`  | string                   | why the finding was or was not refuted                 |

### `OUTCOME`

The value `buildReviewPipeline(mode)(context)` resolves to: a **ranked** array of
the surviving `FINDING`s (the standalone wrapper returns `{ mode, survivors }`).

**Survival rule (`survives`).** A finding survives iff it was **not** refuted
(`verdict.refuted !== true`) **and** its own `confidence >= CONFIDENCE_FLOOR`
(70). The two gates are independent: the refuter's boolean handles "is this
real?", while the confidence floor drops weak findings the finder itself was
unsure of. The floor reads the **finding's** confidence, not the verdict's —
matching the `rdm-review` skill, where the confidence filter applies to the
finding. `verdict.confidence` is recorded but does not gate.

**Failure handling.** A refuter crash is not proof of refutation: if a refuter
`agent()` errors, its finding is kept as **un-refuted** (`verdict = null`) and
survives on the confidence floor alone, rather than being silently dropped as if
refuted — the pipeline logs how many findings were kept this way. A finder crash
instead drops only its own dimension to `null` (the runtime's `pipeline` sends a
thrown stage to null), so the other dimensions still contribute and the review
degrades rather than failing.

**Ranking (`rankFindings`).** A total order, so `OUTCOME` is deterministic across
runs (the runtime forbids `Date.now()`/`Math.random()`): by `severity`
(`blocking` < `concern` < `suggestion`), then `confidence` descending, then `id`
ascending as a stable tiebreaker.

## `buildReviewPipeline(mode, deps?)`

Returns an async `runReview(context)` that composes
`pipeline(DIMENSIONS[mode], find, refute)`:

1. **Find** — one finder `agent()` per dimension, in parallel (`pipeline` stage 1).
   `code` mode dimensions: `ac`, `correctness`, `tests`, `architecture`.
   `plan` mode dimensions: `coherence`, `architectural-fit`, `unit-of-work`.
2. **Refute** — a **fresh** refuter `agent()` per finding, in parallel (stage 2).
3. **Filter** — drop findings that were refuted or fell below `CONFIDENCE_FLOOR`.
4. **Rank** — return the survivors via `rankFindings`.

`context.target` (and any other fields) is threaded into every finder and refuter
prompt, so the review material reaches the agents. `deps` (`{ agent, pipeline,
parallel, log }`) is omitted in the Workflow runtime (the ambient globals are
used) and injected by the verify harness to drive the pipeline with fakes.

## Testing convention

`scripts/verify-workflow-review.sh` is the hermetic gate. It uses **Node standard
library only** — no `package.json`, no `node_modules`, no third-party packages;
the reference `pipeline`/`parallel` implementations and assertions are written
inline with `node:assert`. It resolves `node` via the `.mise.toml`-pinned
toolchain (bare `node`, else `mise exec node --`) and fails hard if node is truly
absent, matching the sibling harnesses' tool-guard convention.
