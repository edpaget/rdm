# Workflow-tool orchestration: conventions & schema contracts

The autonomous lane of rdm's tooling is expressed as **Claude Code Workflow-tool
scripts** under `.claude/workflows/`, a sibling of `.claude/skills/` and
`.claude/hooks/`. This document defines the conventions those scripts follow and
the canonical schema contracts they exchange.

> **Scope:** mostly dogfood-only, with one emitted exception. The three workflow
> scripts — `autopilot.js`, `dispatch-phase.js`, `review-refute-fix.js` — ARE now
> emitted by `rdm agent-config claude --skills --out <dir>`, byte-identical to
> this repo's own `.claude/workflows/` copies, under `<dir>/.claude/workflows/`
> (Claude-only, `--out`-only — see `CHANGELOG.md`). Everything else stays
> dogfood-only and unshipped: `lib/*.mjs` (no regeneration script travels
> downstream to consume it), the generator scripts (`scripts/gen-workflow-review.sh`
> and friends), and the hardcoded `./target/debug/rdm` / `--project rdm`
> invocations baked into the shipped scripts (not yet parameterized for an
> arbitrary target repo). rdm's shipped autonomous skills
> (`rdm-core/src/templates/skill-{autopilot,dispatch-phase}-{cli,mcp}.md`, and the
> `--auto` section of `skill-do-{cli,mcp}.md`) are the user-facing autonomous
> lane and are now thin shims that invoke the three workflow scripts above via
> the `Workflow` tool, instead of re-narrating the orchestration in prose.
> Distributing the still-unshipped pieces (parameterization, `lib/`, a
> downstream regeneration story) remains a follow-up roadmap.

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

**Result: it cannot — and there is no runtime workaround.** A zero-agent spike
workflow tried `import()` of an absolute `file://` URL, a bare absolute path, and a
relative path — all three failed with `import() is not available in workflow
scripts.`, and `typeof require` was `undefined`. A follow-up spike probed every
other way to load code into the runtime, and all are closed:

| Vector                                            | Result                                                       |
| ------------------------------------------------- | ------------------------------------------------------------ |
| `import()` — relative / absolute / `file:` / `data:` / `https:` | `import() is not available in workflow scripts.` |
| `eval('…')`                                       | `Code generation from strings disallowed for this context`   |
| `new Function('…')()`                             | `Code generation from strings disallowed for this context`   |
| source injected via `args`, then `eval`'d         | same — code generation is disabled                           |
| `require` / `module` / `exports`                  | `undefined`                                                  |
| `process` / `Deno` / `Bun` / `fetch`              | `undefined`                                                  |

The entire global scope is `log`, `phase`, `console`, `budget`, `setTimeout`,
`clearTimeout`, `Date`, `agent`, `parallel`, `pipeline`, `workflow`, `args`, plus
pure JS built-ins (`Object`, `Array`, `JSON`, `Math`, `Reflect`, typed arrays…) —
nothing that loads or generates code. This is a **hardened V8 isolate** with two
independent locks: `import()` is host-guarded *and* code-generation-from-strings is
disabled (`SetAllowCodeGenerationFromStrings(false)`). Any code sharing must
therefore happen **before** the script reaches the runtime — there is no in-runtime
hack.

### agent() options spike (does `agent()` honor `model`?)

`agent(prompt, opts)` accepts an **`opts.model`** key that selects a concrete model
for that one subagent. This was settled empirically the same way the import
question was — a 5-case spike workflow dispatched sequentially, reading the model
each agent ACTUALLY ran on out of its transcript (`message.model`), rather than
trusting that the option was merely accepted:

| `model:` passed | Actually ran on | Conclusion |
|---|---|---|
| *(key omitted)* | `claude-opus-4-8` (session model) | inherits the session model |
| `'haiku'` | `claude-haiku-4-5-20251001` | **honored** |
| `'sonnet'` | `claude-sonnet-5` | **honored** |
| `undefined` | `claude-opus-4-8` (session model) | **inert — identical to omitting the key** |
| `'not-a-real-model-xyz'` | *(never ran)* | rejected: "There's an issue with the selected model" |

Three consequences the dispatch path depends on:

1. **`model:` is honored**, not merely accepted — the haiku/sonnet cases ran on
   models different from the session's.
2. **`model: undefined` is inert.** Always-assigning the key is safe; no
   conditional-assignment helper is needed for callers that have no model to pass
   (e.g. the standalone `review-refute-fix` consumer).
3. **An unknown model id does NOT throw — `agent()` RESOLVES to `null`.** This is
   the dangerous one: `[models]` tier bindings are user-configurable, so a binding
   this runtime does not recognise would make every dispatched agent yield `null`
   and the pipeline would proceed into a null plan / silently-clean review. Both
   `dispatch-phase.js` (plan/implement) and `lib/review.mjs` (finders)
   therefore guard explicitly against a `null` agent result whenever an explicit
   model was supplied, and fail loudly instead. Note a `null` finder result would
   otherwise be laundered into `[]` by the refute stage's `(found && …) || []`,
   so the guard converts it to a thrown stage — the only thing `pipeline()` turns
   into a `null` element.

Tier→model resolution itself belongs to `rdm-core` (`rdm model resolve <step>
[--tier <t>]`). The hint is forwarded **only** for `plan`/`implement`, and only
when a tier is actually persisted: `resolve_tier` gives the caller hint top
precedence, and `ReviewVerify.default_tier()` is `Large`, so passing a hint to
`review-find`/`review-verify` can only ever *downgrade* the reviewer
(`resolve review-verify` → opus, but `--tier medium` → sonnet). Review sizing is
core's to own. `scripts/verify-workflow-dispatch.sh` gates both rules (AC-MODEL,
AC-TIER) with planted-mutation self-tests.

**Prior art.** This is a well-known sandbox posture, not a rough edge. Temporal's
TypeScript SDK runs workflow code in a deterministic V8 isolate that throws the
identical `Code generation from strings disallowed` error and blocks `eval` /
`import()`; its official model is to author normal modules with real imports and
let a **build-time bundler (webpack/esbuild) inline everything into a single file**
before it enters the sandbox — for the same reasons (security, deterministic
replay, full code visibility). The same error and the same "bundle at build time,
never eval/import at runtime" resolution recur in isolated-vm, Deno, Cloudflare
Workers, and n8n. Our generated-copy stamper is the minimal form of exactly this
pattern: a bundler's output *is* an inlined copy.

**Chosen mechanism — single-source-of-truth generated copy.** The shared pipeline
is authored once in `lib/review.mjs` — the **canonical review source** — between
`review-refute-fix:begin` / `review-refute-fix:end` marker comments.
`scripts/gen-workflow-review.sh` extracts that block and stamps it **verbatim**
into each consumer between matching markers; `--check` mode asserts no consumer
drifted from the source, and `scripts/verify-workflow-review.sh` (and CI) run that
check. Editing happens in the lib; consumers are regenerated, never hand-edited.

This is distinct from a cross-`workflow()` call: sharing is a **compile-time copy**
of a helper block, not a runtime sub-workflow invocation, so it does not consume
the one allowed level of `workflow()` nesting. dispatch-phase (Phase 2) embeds the
same block in its plan-review and code-review stages the same way.

**Upgrade path (if sharing grows) — a real bundler.** The awk stamper is the
zero-dependency form of build-time inlining, chosen because we share a single
~130-line block and rdm values having no toolchain beyond the compiled binary. If
the shared surface grows to multiple modules, transitive helpers, or npm
dependencies, the drop-in scale-up is to author consumers with a real
`import './lib/review.mjs'` and replace the generator with
`esbuild --bundle --format=esm` — the same category (compile-time inlining), just
authored with real ESM imports instead of marker blocks. The cost is adding
`esbuild` + `node_modules` as a dev dependency, which is why we defer it until the
one-block stamper stops being enough. Either way the author's source has no
duplication; the copy exists only in the generated artifact, exactly like a
bundler's `dist/`.

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
shapes below are defined as JSON Schema in `lib/review.mjs`
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

### `OUTCOME` (review pipeline)

The value `buildReviewPipeline(mode)(context)` resolves to: a **ranked** array of
the surviving `FINDING`s. The dispatch-phase keystone (below) consumes this
array at each of its two review gates and folds it into its own,
differently-shaped `OUTCOME`.

The standalone `review-refute-fix.js` consumer has three invocation shapes: (a)
`mode: 'plan'`, and (b) `mode: 'code'` with no `roadmap`+`phase` or `task`
identifier, both keep returning the legacy survivors-only `{ mode, survivors }`
shape unchanged, for backward compatibility with ad hoc/document-less reviews;
(c) `mode: 'code'` with `{ roadmap, phase }` or `{ task }` runs the SAME
`buildReviewPipeline('code')` pass, then additionally derives real diff signals
from the item's worktree (mirroring dispatch-phase's code gate — see below) and
composes the survivors through `classifyOutcome` plus `statusFor` /
`writesCompletion` / `summarizeFindings` / `gateFor` into the dispatch-shaped
`OUTCOME` contract: `{ roadmap, phase, outcome, status, writesCompletion,
summary, reason, findings }` (or the `{ task, ... }` shape). An optional
`gate: true` persists the mapped rdm status via a mechanical Bash agent, for
headless/ad hoc callers of the workflow only. The interactive `rdm-review`
skill invokes shape (c) with `gate: false` and performs its own gate step
(including the `Done:` completion trailer), so the two review surfaces never
double-write rdm state.

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
`pipeline(selectDimensions(mode, context.signals), find, refute)`:

0. **Select** — the deterministic pre-step `selectDimensions(mode, signals)`
   decides which dimensions actually run (see below).
1. **Find** — one finder `agent()` per selected dimension, in parallel
   (`pipeline` stage 1).
2. **Refute** — a **fresh** refuter `agent()` per finding, in parallel (stage 2).
3. **Filter** — drop findings that were refuted or fell below `CONFIDENCE_FLOOR`.
4. **Rank** — return the survivors via `rankFindings`.

`context.target` (and any other fields) is threaded into every finder and refuter
prompt, so the review material reaches the agents. `deps` (`{ agent, pipeline,
parallel, log }`) is omitted in the Workflow runtime (the ambient globals are
used) and injected by the verify harness to drive the pipeline with fakes.

### Dimensions and `when` triggers

Each dimension is either **always-on** (no `when` key) or **triggered** (a
`when(signals) => boolean` predicate evaluated over both the change's shape and
the target's type).

| mode | always-on | triggered |
| --- | --- | --- |
| `code` | `ac`, `correctness` | `tests`, `architecture`, `api-docs`, `changelog`, `security` |
| `plan` | `coherence`, `architectural-fit` | `unit-of-work` (phases only) |

`unit-of-work` triggers on `signals.targetType === 'phase'`, which is why target
type is a first-class signal rather than diff shape alone.

### `context.signals` and `selectDimensions(mode, signals)`

`selectDimensions` has a **three-way contract**, and the fail-open branch is
load-bearing:

- `signals == null` (omitted, or genuinely unknown) → return **ALL** dimensions
  for the mode, untouched. A caller that cannot compute a diff knows the least,
  so it must get the most coverage. `review-refute-fix.js`'s legacy
  survivors-only shapes ((a) `mode: 'plan'`, (b) `mode: 'code'` with no item
  identifier) and dispatch-phase's **plan** gate take this path today;
  dispatch-phase's **code** gate and `review-refute-fix.js`'s full
  `{ roadmap, phase }` / `{ task }` code-review path both now compute real
  signals (see below) and only fall back to this branch when the diff is
  unavailable.
- an **explicit** signals object — even `{}` — → the always-on dimensions plus
  exactly those whose `when` fires. `{}` means "computed, nothing triggered".
- an unknown `mode` → throw.

Do **not** write `d.when(signals || {})`. Substituting `{}` for omitted signals
makes every conditional predicate read falsy and silently drops the triggered
dimensions — a strict coverage subset returned precisely when the caller had no
information. Omitted signals and an empty signals object are deliberately
different paths, and `verify-workflow-review.sh` asserts both.

### `deriveSignals(input)`

Pure and deterministic (no `Date.now`/`Math.random`, no shell). Maps
`{ targetType, changedFiles, diffText? }` onto a **fully-populated** signals
object — every boolean key in `SIGNAL_KEYS` (`changesLogic`, `missingTests`,
`multiModule`, `publicApiChanged`, `userFacing`, `securitySurface`, `hasUnsafe`)
is set explicitly. A partially-populated object would make a conditional
dimension drop out on a *missing* key rather than a real negative, so callers
that cannot compute a diff must pass **no** signals rather than a partial object.

**Who feeds it.** `dispatch-phase`'s code gate runs a mechanical `diff:signals`
agent inside the item's worktree (`git diff --name-only main...HEAD` plus a
truncated `git diff main...HEAD`) and threads the result through `deriveSignals`
into `buildReviewPipeline('code')` — recomputed on **every** rework round, so a
round-2 fix that newly touches a public `rdm-core` item turns `api-docs` on for
that round. The three-dot base scopes to the branch's own changes; for a phase in
a shared per-roadmap worktree that is over-inclusive (earlier phases' files ride
along) but never under-inclusive, which is the safe direction for a coverage
gate. A very large diff is truncated in the prompt, which likewise only weakens
trigger detection toward fail-open. **Signals-absent fail-open contract:** if the
diff agent fails, returns null, or reports no changed files, the driver omits the
`signals` key **entirely** — never `{}` — so every dimension runs.

### Verdict and status mapping

`classifyOutcome(input)` — the total, deterministic decision tree — now lives in
`lib/review.mjs` alongside `hasBlocking`, `summarizeFindings`,
`codeReviewRounds`, and `DEFAULT_MAX_CODE_REWORK`, so every surface shares one
classifier. It returns exactly one of the canonical `OUTCOMES`:

| outcome | when | phase status | task status | writes the completion trailer |
| --- | --- | --- | --- | --- |
| `reviewed` | clean, or clean after small fixes | `reviewed` | `reviewed` | yes |
| `rework` | a fixable defect or an unmet AC | `in-progress` | `in-progress` | no |
| `escalated` | a blocker needing a human decision | `blocked` | `blocked` | no |

`statusFor(outcome, kind)` and `writesCompletion(outcome)` expose that table and
throw on an unknown outcome or item kind rather than returning `undefined`. The
land-time completion trailer is expressed here **only** as the boolean
`writesCompletion` — never as the literal string — because the stamped block is
copied into workflow scripts, where `verify-workflow-dispatch.sh` AC-1 forbids
that literal. The trailer's format string lives in `rdm-core`
(`rdm_core::hook::format_done_directive`, surfaced as `rdm hook done-line`), and
is written only by non-stamped code: the interactive skill's gate step and
`rdm-land`'s land-time synthesis.

### Two projections, two `--check`-gated generators

`lib/review.mjs` carries two marker systems:

- the **stamped block** (`review-refute-fix` markers) — copied verbatim into the
  workflow consumers by `scripts/gen-workflow-review.sh`;
- the **skill-renderable spec** — a `review-spec` region nested *inside* the
  stamped block plus a `review-gate-spec` region *after* it, whose `//| `
  literate comment lines `scripts/gen-skill-review.sh` renders into
  `rdm-core/src/templates/skill-review-{cli,mcp}.md` between
  `<!-- rdm:review-spec:begin/end -->` markers. It is mode-dispatched
  (`--mode code|plan`), and `--mode plan` renders the SAME regions into
  `skill-plan-review-{cli,mcp}.md` — one source, one emitter, two skills. The
  gate region sits outside the stamped block precisely because it is the one
  place the completion-trailer literal may appear.

Which mode a prose line belongs to is declared by an optional **per-line mode
tag**, written immediately after the `//|` prefix: an untagged line is shared
and renders in every mode, a `code|`-tagged line renders only under
`--mode code`, and a `plan|`-tagged line only under `--mode plan`. The tag is
recognized only as that literal text immediately after `//|`, so shared prose
must never begin with it. There is no second region, no second generator, and no
second consumer list — the tag is the whole mechanism. Mode-isolation greps in
`scripts/verify-workflow-review.sh` (code dimension names and the trailer
literal must be absent from the plan render; `needs-plan-review` and
`unit-of-work` absent from the code render) are the detector for a mistagged
line leaking across.

The gate itself is likewise mode-dispatched data rather than a fork:
`GATE_POLICY[mode][outcome]` yields `{ status, writesCompletion,
clearsPlanReviewTag, reasonPrefix }`, and `STATUS_MAPPING` *is*
`GATE_POLICY.code`, so `statusFor`/`writesCompletion` are unchanged for
`dispatch-phase`/`autopilot`. The plan rows carry an explicit `status: null` — a
plan review never persists an rdm status; it clears `needs-plan-review` on
`reviewed` and leaves it on `rework`/`escalated`.

Everything else inside the stamped block is **machinery** (JSON schemas,
`survives`/`rankFindings`/`selectDimensions`/`deriveSignals`, the classifier and
the gate policy) and is never rendered into a skill. Both generators are
`--check`-gated by `scripts/verify-workflow-review.sh` — the skill generator in
BOTH modes — which CI runs.

## dispatch-phase contracts

`dispatch-phase` (`.claude/workflows/dispatch-phase.js`) is the keystone per-phase
unit of autonomous execution: a deterministic 4-stage pipeline
`Plan → PlanReview → Implement → CodeReview`. Its plan-review and code-review
stages call `buildReviewPipeline('plan')` / `buildReviewPipeline('code')` inline
(from the stamped review block — never via a nested `workflow()` call). Its pure
decision core lives once in `lib/dispatch-phase.mjs` and is copied byte-identical
into the workflow script (gated by `scripts/verify-workflow-dispatch.sh`).

### `PHASE_META`

What the Stage-0 mechanical fetch agent returns from `rdm phase show … --format
json` (the Workflow runtime has no `process`/`child_process`, so it cannot shell
out itself — a Bash-capable agent does).

| field    | type              | notes                                             |
| -------- | ----------------- | ------------------------------------------------- |
| `roadmap`| string (required) | roadmap slug                                      |
| `phase`  | string (required) | the stem-or-number that was dispatched            |
| `stem`   | string (required) | the phase's canonical stem                        |
| `model`  | string (required) | the tier (`small` \| `medium` \| `large`)         |
| `body`   | string (required) | the full phase markdown; empty ⇒ fetch failure    |

### `PLAN_DOC`

The plan document the planner agent produces from **only** the phase body (no
worktree, no code). Rendered to text and fed to the plan-review gate, then — once
approved — to the implementer. A vague or empty plan is flagged `blocking` by the
plan-review `coherence` dimension and escalates before any implementation.

| field              | type                                    | notes                                    |
| ------------------ | --------------------------------------- | ---------------------------------------- |
| `steps_per_ac`     | array of `{ ac, steps[] }` (required)   | ordered steps for each acceptance criterion |
| `file_map`         | array of `{ path, change }` (required)  | files to create/edit and how             |
| `tests_per_ac`     | array of `{ ac, test }` (required)      | the test that proves each criterion      |
| `edge_cases`       | array of string (required)              | edge cases the implementation must handle|
| `cross_phase_deps` | array of string (required)              | what this phase consumes from / provides to siblings |
| `summary`          | string (required)                       | one-paragraph plan summary               |

### `OUTCOME` (dispatch-phase)

The top-level return of `dispatch-phase`. Distinct from the review pipeline's
`OUTCOME` array above — this is the phase-level verdict consumed by the Phase 3
autopilot and Phase 4 `rdm-do --auto`. The `rdm-do --auto` wiring into this
contract is regression-tested by `scripts/verify-workflow-do-auto.sh` (SKILL.md
static invariants, the OUTCOME→status contract against the real binary, and a
prose-only self-test of the distributed template).

| field     | type                                      | notes                                          |
| --------- | ----------------------------------------- | ---------------------------------------------- |
| `roadmap` | string                                    | echoed from the dispatch args                  |
| `phase`   | string                                    | echoed from the dispatch args                  |
| `outcome` | `reviewed` \| `rework` \| `escalated`     | the phase verdict (see the decision tree below)|
| `status`  | string                                    | `statusFor(outcome, kind)` — the rdm status to persist |
| `writesCompletion` | boolean                          | `writesCompletion(outcome)` — is this branch owed its land-time trailer? |
| `summary` | string                                    | deterministic one-liner from outcome + top finding |
| `reason`  | string                                    | gate-tagged park note (`[plan]`/`[code]`); empty on `reviewed` |
| `findings`| array of `FINDING`                        | the relevant ranked surviving findings         |

Task mode emits the same shape keyed by `task` instead of `roadmap`/`phase`.

**`status` / `writesCompletion` carry the gate policy as data.** They are derived
from the canonical `statusFor` / `writesCompletion` in `lib/review.mjs`, so
consumers (autopilot's advance/park, `rdm-do --auto`, `rdm-land`) read the policy
off the OUTCOME instead of restating the mapping. `writesCompletion` is a
**boolean, never the trailer literal** — the stamped block may not contain that
string (`verify-workflow-dispatch.sh` AC-1). `rdm-land` reads
`writesCompletion: true` and synthesizes the real trailer at land time via
`rdm hook done-line`, amending it **before** the rebase, so an autonomously
produced branch never needs a manual rebase to gain it.

**`reason` tags the gate, not the module.** dispatch's `escalated` is tagged
`[plan]` because `classifyOutcome` only escalates from the plan gate, while
`rework` is tagged `[code]`. This deliberately differs from
`STATUS_MAPPING.reasonPrefix` (`[code]`), which describes the *interactive*
review surface, where escalation comes out of the code gate.

**Decision tree (`classifyOutcome`, total and deterministic).** Tier-scaled via
`hasBlocking(findings, tier)`: only `blocking` counts as blocking, except at the
`large` tier where a surviving `concern` blocks too (a one-directional tightening
— the gate can only get stricter, never looser).

1. **Plan gate.** If the plan-review findings are blocking → `escalated` (findings
   = the plan findings). An empty/ambiguous plan lands here via a blocking
   `coherence` finding. `fetchError` short-circuits here too (`phase fetch
   failed`). The pipeline never implements on a failed plan.
2. **Code gate** (plan approved, implement ran, code-review ran):
   - clean first pass → `reviewed`;
   - else the **one** bounded rework ran: if its re-review is clean → `reviewed`,
     otherwise → `rework` (findings = the post-rework code findings).

Both loops are bounded to exactly one extra pass (≤1 plan-revise, ≤1 code-rework),
so the classifier — which consumes only the first-pass and one-rework arrays —
always reaches a terminal value. Because the deterministic pipeline cannot
classify a code finding's *nature* (the `FINDING` schema carries severity but no
fixable/decision flag), a code defect surviving the one rework resolves to
`rework`, and genuine decisions surface earlier at the plan gate as `escalated`;
that is why the code stage yields only `reviewed`/`rework`. `dispatch-phase` never
emits a `Done:` line — it emits `writesCompletion` and landing is a separate,
later step.

**The code-review stage is the canonical review.** `dispatch-phase` builds it
from the stamped `buildReviewPipeline('code')` — there is no independent
code-review logic in the driver — and feeds it `deriveSignals` output from the
real branch diff (see `deriveSignals(input)` above for the signals-absent
fail-open contract). `verify-workflow-dispatch.sh` pins both halves: exactly one
`buildReviewPipeline('code')` binding site and one declaration each of
`findPrompt`/`refutePrompt`, plus the `deriveSignals(` / `signals:` /
`diff:signals` wiring.

## autopilot contract

`autopilot` (`.claude/workflows/autopilot.js`) is the **active driver**: given one
roadmap slug it drives every actionable phase to `reviewed` by calling
`dispatch-phase` via `workflow()` — the one allowed level of nesting (no deeper
`workflow()` call lives inside `dispatch-phase`). Its pure control core lives once
in `lib/autopilot.mjs` between `autopilot-loop:begin` / `autopilot-loop:end`
markers and is copied byte-identical into the workflow script (gated by
`scripts/verify-workflow-autopilot.sh`). The block names **no** ambient runtime
global — every side effect is reached through an injected `deps` object — so the
module imports cleanly in Node for unit testing and the harness drives the whole
loop with state-backed fakes.

### The core fix: the loop advances off PERSISTED status

`dispatch-phase` persists **no terminal** phase status — it does stamp the
phase (or task) `in-progress` itself, best-effort, right after Stage 0 (metadata
+ model resolution) and before it starts working the item; a `--plan-only` run
skips that stamp, since it never implements — and `rdm next` returns only
`not-started`/`in-progress` phases (it skips `reviewed`/`blocked`/…). So the loop
persists the terminal status **itself**, which is what makes `rdm next` step
forward and eventually return `nothing`:

- a `reviewed` OUTCOME (normal mode) → `advance` dep runs
  `rdm phase update <stem> --status reviewed`;
- a rework-exhausted or `escalated` OUTCOME → `park` dep runs
  `rdm phase update <stem> --status blocked --reason "[code|plan] …"`.

There is **no** normal-mode in-memory `seen` Set; progress is driven entirely by
the persisted status the selector reads back.

### Config (`parseAutopilotArgs`)

| field         | type               | notes                                                   |
| ------------- | ------------------ | ------------------------------------------------------- |
| `roadmap`     | string (required)  | the single roadmap slug; the loop never roams elsewhere |
| `maxPhases`   | positive int \| null | the `--max-phases` bound (null = unbounded by count)  |
| `planOnly`    | boolean            | `--plan-only`: each dispatch stops after its plan gate  |
| `globalBudget`| int                | total-dispatch cap per run (defaults to a sane constant)|

It never yields a `--land` flag — landing is the separate `rdm-land` skill.

### Dep interface (`buildAutopilot(deps)` → `runAutopilot(config)`)

The block reaches the runtime only through these injected deps; the real ones
(built outside the block) close over `agent()`/`parallel()`/`workflow()`/`log()`:

| dep                                   | effect                                                                                  |
| ------------------------------------- | --------------------------------------------------------------------------------------- |
| `estimateList(slug)`                  | Bash agent: `rdm phase list … --format json`                                            |
| `parallelEstimate(unestimated)`       | one `parallel()` fan-out of estimator agents → `{ stem, difficulty }[]`                  |
| `estimateWriteback(stem, diff, slug)` | Bash agent: `rdm phase update <stem> --difficulty <diff>` (tier auto-derives)           |
| `fetchNext(slug)`                     | Bash agent: `rdm next … --format json` → parsed JSON                                     |
| `dispatch(slug, stem, planOnly)`      | `workflow('dispatch-phase', { roadmap, phase, planOnly })` → the dispatch-phase OUTCOME  |
| `advance(stem, slug, status)`         | Bash agent: `rdm phase update <stem> --status <status>` (status from the OUTCOME)        |
| `park(stem, reason, slug)`            | Bash agent: `rdm phase update <stem> --status blocked --reason "<reason>"`               |
| `log(msg)`                            | progress line                                                                            |

### OUTCOME-driven transitions (`interpretOutcome`)

The whole `dispatch-phase` OUTCOME **object** drives the next loop action.
`interpretOutcome` reads `status` and `reason` off it rather than restating the
mapping (a bare outcome string is still accepted, and falls back to the legacy
literals):

| OUTCOME      | mode        | action                                                          |
| ------------ | ----------- | -------------------------------------------------------------- |
| `reviewed`   | normal      | `advance` → `--status <outcome.status>`; record completed      |
| `reviewed`   | `--plan-only` | `noop-vetted` → record vetted, do NOT advance                |
| `rework`     | under budget| `retry` → re-dispatch the same phase                           |
| `rework`     | budget spent| `park` → `--status blocked --reason "[code] …"`                |
| `escalated`  | —           | `park` → `--status blocked --reason "[plan] …"`                |

Retained loop state is bounded: the latest `fetchNext` result, the current
OUTCOME, per-phase rework/advance counters, the running dispatch count, the
ordered `completed[]` and `escalations[]` arrays, and (only under `--plan-only`) a
`planOnlySeen` Set. The rework-budget park stays autopilot's **own** decision:
dispatch's `rework` status (`in-progress`) describes a single dispatch, whereas a
phase whose roadmap-level retry budget is spent belongs in the `blocked`
escalation queue. A mid-tier default (`resolveTier(model || 'medium')`) covers
any unset tier at dispatch. The run stops on `nothing` /
`blocked-on-dependencies`, on the global step budget or `--max-phases`, or (under
`--plan-only`) when a vetted phase is re-returned.

### Summary (`buildSummary`, always emitted)

Every run — whatever stopped it — returns a deterministic summary string: the
phases completed in order, the escalations each tagged `plan`/`code` with their
reason and a pointer at `./target/debug/rdm review blocked --project rdm`, the
stop reason, and a note that reviewed work is left on the `roadmap/<slug>` branch
and `main` is **never** touched.

### Harness invariant: the completion trailer (INVERTED)

`scripts/verify-workflow-autopilot.sh` used to assert the land-time completion
trailer was absent from `autopilot.js` **anywhere** — an absolute whole-file rule,
written when nothing wrote the trailer at all. Now that the write happens at land
time, the rule is deliberately **scoped**, and paired with a positive assertion:

- still absolutely forbidden in every **built prompt** (the Node `FORBIDDEN`
  sweep) — autopilot must never ask an agent to write the trailer itself;
- still forbidden in autopilot's own **code** — the whole-file grep is now scoped
  to non-comment lines;
- now **allowed in explanatory comments**, so the file may name `rdm-land` as the
  land-time writer;
- **new positive regression** (`verify-workflow-autopilot.sh` § 6, hermetic,
  against the real binary): a trailer-less commit on `roadmap/rm` — exactly the
  state an autopilot run leaves — gains `Done: rm/phase-1-x` from
  `rdm hook done-line` + `git commit --amend` with **no rebase**, and
  `rdm hook post-commit` then flips the phase to `done` with the landed SHA.

`verify-workflow-dispatch.sh` keeps the complementary absence assertion (AC-1:
no trailer literal inside a stamped region; no OUTCOME JSON contains one) so both
directions stay pinned.

### `dispatch-phase` `planOnly`

`dispatch-phase` accepts an optional `planOnly` arg. When set, once the plan gate
passes it returns early — `{ outcome: 'reviewed', summary: 'plan-only: plan gate
passed', findings: <planFindings> }` — before implementing, so autopilot can vet
the plan half cheaply. This early return lives in the driver, outside the copied
`dispatch-outcome` block, and adds no new nested `workflow()` call.

## Testing convention

`scripts/verify-workflow-review.sh` is the hermetic gate. It uses **Node standard
library only** — no `package.json`, no `node_modules`, no third-party packages;
the reference `pipeline`/`parallel` implementations and assertions are written
inline with `node:assert`. It resolves `node` via the `.mise.toml`-pinned
toolchain (bare `node`, else `mise exec node --`) and fails hard if node is truly
absent, matching the sibling harnesses' tool-guard convention.
