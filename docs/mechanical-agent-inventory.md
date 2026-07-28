# Mechanical agent inventory

The call-site census behind **phase 3 of the `workflow-token-reduction` roadmap** —
*eliminate the mechanical subagent rather than shrink it*.

A **mechanical** agent runs one `rdm` or `git` command and returns its output. It does no
judgment work, yet it pays the same context-loading cost as a reviewer: a `fetch:next` agent
averaged ~29k tokens for a single Bash call, and the cheapest agent observed anywhere in the
lane cost 27k. **The cheapest mechanical agent is the one that is never spawned.**

This document is the phase's primary deliverable and **phase 4's input**: phase 4 is the
context-trimming lever over the irreducible set named in § Irreducible below.

Related: [`docs/token-baseline.md`](token-baseline.md) (the before-figures),
[`docs/workflow-schemas.md`](workflow-schemas.md) (the optional args each workflow now
accepts), [`docs/autonomous-loop.md`](autonomous-loop.md).

## Raw inventory (derived live, not transcribed)

```
grep -n "label: *['\"]" .claude/workflows/*.js | grep -v spike-agent-type
```

**44 labelled `agent()` call sites** across the seven workflow scripts:

| file | call sites |
|---|---|
| `autopilot.js` | 7 |
| `backlog.js` | 3 |
| `dispatch-phase.js` | 11 |
| `document.js` | 5 |
| `estimate.js` | 5 |
| `plan-review.js` | 9 |
| `review-refute-fix.js` | 4 |
| **total** | **44** |

Adding `.claude/workflows/lib/*.mjs` to the glob raises the count to **53**. That is *not* nine
extra call sites: the libs hold the single-source originals of blocks that are stamped or
byte-copied into the `.js` consumers, so the same site is counted twice. **The seven `.js`
files are the authoritative surface** — they are what the Workflow runtime executes.

Of the 44, **28 are mechanical** and **16 are judgment** agents. The judgment set is out of
scope for this phase and is listed here only so the split is checkable: `find:*`, `refute:*`,
`plan:author`, `plan:revise`, `implement:worktree`, `implement:rework`, `act:code`,
`act:<kind>:<ident>`, `estimate:<stem>` / `estimate:rate:*`, `analyze:*`, `synthesize:draft`.

## The classification rule

Applied uniformly, so the table below is principled rather than ad hoc:

- **HOISTABLE** iff (a) the site is read-only, **and** (b) every input it needs is derivable
  from `args` before the run starts, **and** (c) it fires before any judgment agent (a
  bootstrap / Stage-0 read). The caller — the parent skill shim — is already a running agent
  with the repo in context; if it runs the command itself and passes the result through the
  `Workflow` tool's `args`, the workflow needs no agent for it.
- **ABSORBABLE** iff the site is read-only, its input is only known mid-run, **and** an
  already-running agent immediately precedes it with no intervening stage. Folding the command
  into that agent's prompt costs a tool call, not a 27k-token context load.
- **REDUNDANT** iff a caller in some lane has already performed the same work.
- **IRREDUCIBLE** otherwise. This is phase 4's scope.

Every hoist is **optional by construction**: the original `agent()` call is kept byte-unchanged
and reached through an `else` branch, so a direct `Workflow` invocation — which has no caller
to supply anything — behaves exactly as before.

## Maintenance routes

Where a call site lives determines how it is edited. Three routes exist:

| route | blocks | how to edit |
|---|---|---|
| **stamped** | `review-refute-fix` (in `dispatch-phase.js`, `plan-review.js`, `review-refute-fix.js`), `estimate-core` (in `estimate.js`, `autopilot.js`) | edit the lib (`lib/review.mjs` / `lib/estimate.mjs`), re-run `scripts/gen-workflow-review.sh` / `gen-workflow-estimate.sh`; `--check` gates drift |
| **byte-copied** | `dispatch-outcome` (`lib/dispatch-phase.mjs`), `autopilot-loop` (`lib/autopilot.mjs`), `plan-review-driver` (`lib/plan-review.mjs`) | edit the lib **first**, then copy the block verbatim into the consumer; `verify-workflow-dispatch.sh` §2 / `verify-workflow-autopilot.sh` / `verify-workflow-review.sh` §5b-drift gate byte-equality |
| **unprojected** | everything below a `:end` marker (the driver regions), plus `document-core`/`backlog-groom` consumers | edit in place |

**Every mechanical call site sits in an unprojected DRIVER region**, *except* plan-review's
`fetch:roadmap` / `fetch:<kind>` / `fetch:wontfix` / `act:round-note:*` / `gate:clear-tag:*`,
which sit inside the byte-copied `plan-review-driver` block. No mechanical site sits inside a
generator-**stamped** block, so no generator had to learn anything new for this phase.

Independently: `autopilot.js`, `dispatch-phase.js` and `review-refute-fix.js` carry
hand-maintained **byte-identical copies** under `rdm-core/src/templates/workflows/`, embedded
by `rdm-core/src/agent_config.rs` via `include_str!`. **No generator writes those copies** —
they are re-synced by `cp`, and `scripts/verify-agent-config-distribution.sh` plus
`scripts/verify-workflow-review-outcome.sh` hard-fail on any divergence.

## Caller surfaces: which shims can actually hoist today

Only **three** skills are Workflow-invoking shims on the *distribution* surface. The other five
are shims **only** in this repo's local `.claude/skills/` dogfood copies; their distributed
templates (`rdm-core/src/templates/skill-{plan-review,backlog,document,review,estimate}-{cli,mcp}.md`)
contain zero `.claude/workflows` references and still dispatch their own Agent/Bash prose.

| skill | distributed template is a shim? | local `.claude/skills` copy is a shim? | hoists reach |
|---|---|---|---|
| `rdm-autopilot` | yes | yes | distributed **and** local |
| `rdm-dispatch-phase` | yes | yes | distributed **and** local |
| `rdm-do` (`--auto`) | yes | yes | distributed **and** local |
| `rdm-plan-review` | **no** | yes | local dogfood only |
| `rdm-backlog` | **no** | yes | local dogfood only |
| `rdm-document` | **no** | yes | local dogfood only |
| `rdm-review` | **no** | yes | local dogfood only |
| `rdm-estimate` | **no** | yes | local dogfood only |

Converting those ten templates into thin Workflow shims belongs to the `distribute-workflow-lane`
roadmap and is explicitly **out of scope** here. It is tracked by task
**`convert-remaining-skill-templates-to-workflow-shims`** (tagged `depends-unlanded`), whose body
names the ten templates and the exact hoist arg keys the converted shims must gather. Until it
lands, the **distributed-caller path of those five workflows takes the in-workflow fallback** —
which is why that path appears in the irreducible set below.

Separately, **MCP shims can never hoist a model id**: there is no MCP model-resolve tool, and
`phaseMeta`/`taskMeta` are all-or-nothing (see below), so the MCP `rdm-dispatch-phase` and
`rdm-do` shims hoist only `alreadyInProgress`, and the MCP `rdm-autopilot` shim omits
`mechanicalModel`. That degradation is silent and correct — never a partial or invented value.

## The classification table

Columns: **route** = stamped / byte-copied / unprojected. **dist copy** = has a hand-maintained
byte-identical copy under `rdm-core/src/templates/workflows/`. **caller** = which caller surface
can supply the hoist today.

| label | file | route | dist copy | caller | class | reasoning | phase 4? |
|---|---|---|---|---|---|---|---|
| `fetch:phase-meta` | `dispatch-phase.js` | unprojected driver | yes | distributed shim (`rdm-dispatch-phase`, `rdm-do --auto`) — CLI only | **hoistable** | Read-only Stage-0 read, fires before any judgment agent, and the caller already ran `rdm phase show`. Accepted only when the payload carries a non-empty body, all five resolved model ids, **and** the `model` difficulty tier (`hoistedMetaComplete`) — the tier is the driver's sole source for gate strictness and its `'medium'` default would silently loosen a `large` phase's gate. | no (direct/shim path) |
| `fetch:task-meta` | `dispatch-phase.js` | unprojected driver | yes | distributed shim — CLI only | **hoistable** | Task-mode twin of the above, same body+models guard. No tier requirement: `TASK_META` carries none and the driver hard-codes a task to `medium`, so there is nothing to lose. | no (direct/shim path) |
| `stamp:in-progress` | `dispatch-phase.js` | unprojected driver | yes | distributed shim — CLI **and** MCP | **redundant** | Interactive `rdm-do`, `rdm-do --auto` and the `rdm-dispatch-phase` shim all write `--status in-progress` before invoking the workflow. Suppressed by an explicit `alreadyInProgress` flag set **only** when that write exited 0, and **never** for a `--plan-only` run. | **yes** (autopilot-nested + direct-`Workflow` paths) |
| `diff:signals` | `dispatch-phase.js` | unprojected driver | yes | n/a — absorbed, no caller needed | **absorbable** | `runCodeGate` calls `d.implement(...)` immediately before every `d.review()` with nothing in between, so the implementer — already in the worktree it just wrote to — reports the same two `git diff` commands. One-shot handoff (`pendingDiff` read-and-cleared) preserves per-round freshness. Works on **every** path, including autopilot-nested. | no |
| `diff:signals` | `review-refute-fix.js` | unprojected driver | yes | local shim only (`rdm-review`) | **hoistable** | No adjacent implementer in this workflow (it reviews an already-implemented item), but `worktreeRef` is fully determined by `args`, so the caller can run the diff itself. | partly (distributed-caller path) |
| `gate:persist` | `review-refute-fix.js` | unprojected driver | yes | — | **irreducible** | A write whose status/reason are computed mid-run from the classified outcome. | **yes** |
| `model:mechanical` | `autopilot.js` | unprojected driver | yes | distributed shim — **CLI only** | **hoistable** | Pure bootstrap read of `rdm model resolve mechanical`, before everything. MCP has no model-resolve tool, so the MCP shim omits it and the agent runs. | partly (MCP path) |
| `estimate:list` | `autopilot.js` | unprojected driver | yes | distributed shim (CLI + MCP) | **hoistable** | `rdm phase list --format json` — read-only, pre-run, no judgment agent before it. | no |
| `fetch:next` | `autopilot.js` | unprojected driver | yes | distributed shim (CLI + MCP) | **hoistable — first iteration only** | `rdm next` is what *advances the cursor* once advance/park has persisted a status, so a cached result is only valid for iteration 1. Consumed strictly one-shot (`pendingNext`); iterations 2..N always re-read live state. | **yes** (iterations 2..N) |
| `estimate:write:*` | `autopilot.js` | unprojected driver | yes | — | **irreducible** | A write whose difficulty/justification inputs are produced mid-run by the rater. | **yes** |
| `advance:*` | `autopilot.js` | unprojected driver | yes | — | **irreducible** | A write keyed on the OUTCOME the dispatch just produced. | **yes** |
| `park:*` | `autopilot.js` | unprojected driver | yes | — | **irreducible** | Same — a mid-run write with a computed reason. | **yes** |
| `model:mechanical` | `estimate.js` | unprojected driver | no | local shim only (`rdm-estimate`) | **hoistable** | Same bootstrap read as autopilot's. | partly (distributed-caller path) |
| `estimate:list` | `estimate.js` | unprojected driver | no | local shim only | **hoistable** | Same as autopilot's. | partly (distributed-caller path) |
| `estimate:write:*` | `estimate.js` | unprojected driver | no | — | **irreducible** | Mid-run write. | **yes** |
| `estimate:tier:*` | `estimate.js` | unprojected driver | no | — | **irreducible** | Reads back a tier that only exists *after* the writeback it follows. | **yes** |
| `model:mechanical` | `plan-review.js` | unprojected runtime entry | no | local shim only (`rdm-plan-review`) | **hoistable** | Same bootstrap read. | partly (distributed-caller path) |
| `fetch:roadmap` | `plan-review.js` | **byte-copied** `plan-review-driver` (gate: `verify-workflow-review.sh` §5b-drift) | no | local shim only | **hoistable — PRIORITY** | See § The hoist with a recorded correctness failure. Not ranked on cost. | partly (distributed-caller path) |
| `fetch:<kind>` (`fetch:task` / `fetch:phase`) | `plan-review.js` | **byte-copied** `plan-review-driver` | no | local shim only | **hoistable — PRIORITY** | Same — see below. | partly (distributed-caller path) |
| `fetch:wontfix` | `plan-review.js` | **byte-copied** `plan-review-driver` | no | local shim only | **hoistable** | One `rdm search` covering the whole run, read-only, before any unit is reviewed. | partly (distributed-caller path) |
| `act:round-note:*` | `plan-review.js` | **byte-copied** `plan-review-driver` | no | — | **irreducible** | A write whose round number and finding list are computed mid-run. | **yes** |
| `gate:clear-tag:*` | `plan-review.js` | **byte-copied** `plan-review-driver` | no | — | **irreducible** | A write keyed on the per-unit outcome the pipeline just produced. | **yes** |
| `model:mechanical` | `backlog.js` | unprojected driver | no | local shim only (`rdm-backlog`) | **hoistable** | Same bootstrap read. | partly (distributed-caller path) |
| `fetch:report` | `backlog.js` | unprojected driver | no | local shim only | **hoistable** | `rdm backlog report --format json` is read-only whoever runs it, so hoisting it does not weaken the propose-only contract. | partly (distributed-caller path) |
| `model:mechanical` | `document.js` | unprojected driver | no | local shim only (`rdm-document`) | **hoistable** | Same bootstrap read. | partly (distributed-caller path) |
| `fetch:roadmap-meta` | `document.js` | unprojected driver | no | local shim only | **hoistable** | `rdm roadmap show --format json`, read before the all-done validation and before any judgment agent. | partly (distributed-caller path) |
| `gather:*` | `document.js` | unprojected driver | no | — | **irreducible** | A per-phase mid-run read fan-out whose inputs come from the phase list. | **yes** |
| `write:draft` | `document.js` | unprojected driver | no | — | **irreducible** | Writes a document the synthesis agent produced mid-run. | **yes** |

### Why `stamp:in-progress` is NOT absorbed

The stamp is emitted right after Stage 0 and **before** `runPlanGate`. The implementer is
separated from it by `plan:author`, the whole `buildReviewPipeline('plan')` fan-out, and up to
`maxPlanRevise` `plan:revise` rounds — the most expensive stretch of the pipeline. Worse, a
**blocking plan finding escalates before any implementation**: `runPlanGate` returns, the driver
calls `itemOutcome({ planFindings, tier })`, and `implement:*` never runs at all. An absorbed
stamp would therefore take the item straight from `not-started` to `blocked` with **no
in-progress signal**, reopening exactly the gap CLAUDE.md's "Update (no-in-progress-stamp)"
records as closed.

So the stamp stays a dedicated pre-plan-gate call on every path where no caller stamped, and
`scripts/verify-workflow-dispatch.sh` §6e asserts the invariant directly: on a plan-escalation
seed with `alreadyInProgress` unset, `stamp:in-progress` appears **before** the first
`plan:author` in the recorded label order, and the run still returns the escalated OUTCOME.

## The hoist with a recorded correctness failure

Most sites here are a pure cost question. `plan-review.js`'s `fetch:roadmap` and `fetch:<kind>`
are not: they have **twice corrupted plan data in production**, and hoisting is the only
mechanism that removes the failure rather than checking for it after the fact. They are
classified **HOISTABLE, priority** — explicitly *not* ranked on cost.

Both returns are recorded in full (with sidecar paths) on task `fix-plan-review-gate-tag-clobber`:

- **run `wf_e3402021-0af`** — `{ body: "Fetched roadmap and phase data for workflow-token-reduction",
  tags: ["fetch","roadmap","workflow-token-reduction"], phases: [ONE entry, stem = the roadmap slug] }`.
  The agent transposed levels: a description of its own action into `body`, three words lifted
  from its own prompt into `tags`, and the roadmap's real body and tags packed into `phases[0]`.
  **Five of six phases vanished.** The gate then faithfully wrote the junk `tags` over the
  roadmap's real ones.
- **run `wf_f4be8027-dbb`** — `{ body: "fix-plan-review-gate-tag-clobber task fetched successfully",
  tags: ["plan-target"] }` on the `--task` path. `plan-target` is a word from the prompt's own
  "Return a PLAN_TARGET object". The gate's write was blocked by a safety classifier, not by
  anything in the lane.

Two properties make these hoist targets specifically:

1. **`agent(..., { schema })` cannot catch it.** Both returns satisfy their schema *exactly*;
   the schema constrains shape, never content. Schema validation is false assurance here.
2. **The caller can supply the value deterministically.** `args` is the only deterministic input
   channel the Workflow runtime has (`docs/workflow-schemas.md` § "Import spike": no `process`,
   no `fetch`, no `require`). A shim that runs `rdm task show --format json` itself and passes
   the parsed JSON through `args` eliminates the transcription entirely, for strictly less cost
   than the agent it replaces.

Because the hoist reaches these two sites only through the **local** `rdm-plan-review` shim
today, the distributed template still transcribes. Guarding *that* path — driver-side validation
of the fetched payload — is deliberately **not** this phase's job: it is scoped to task
`fix-plan-review-gate-tag-clobber`, kept independent so the two do not collide. That task's
originally-preferred mechanism (a `parallel()` per-phase fan-out) was rejected precisely because
it would multiply this phase's mechanical agents, and must not be reintroduced.

One consequence is worth stating on its own, because the hoist's shape guard replaces a
`required`-bearing schema: `hoistedFetchedOk` is held to be **no weaker** than
`PLAN_TARGET_SCHEMA` / `ROADMAP_TARGET_SCHEMA`. Both list `tags` as `required`, so the guard
requires a `tags` array of strings — on the payload *and* on every roadmap phase entry
(alongside a non-empty `stem` and a string `body`) — and rejects the payload outright otherwise.
The reason is the write-back: the gate issues `rdm ... update --tags "<list>"`, and `--tags`
**replaces** the whole list. A tags-less payload accepted here would be defaulted to `[]` by
`buildReviewUnits` and written as `--tags ""`, wiping every real tag the item carried — the same
outcome as the recorded incidents, reached by omission rather than by transcription. Rejecting
costs one fetch agent on the fallback path; accepting costs the item's tags.

`scripts/verify-workflow-review.sh` § 6 asserts the hoisted path carries the target's **real**
field values — `tags` deep-equal to what `./target/debug/rdm ... show --format json` reports,
real phase stems, and a `gate:clear-tag` prompt carrying exactly the sibling-preserved tag list
— and replays the `wf_e3402021-0af` payload as a negative, proving a shape-only/schema check
would have passed it.

## Irreducible (phase 4 scope)

Named explicitly, because this list *is* phase 4's input:

1. **`stamp:in-progress`** on the autopilot-nested and direct-`Workflow` paths. Autopilot is
   itself a workflow and cannot shell out, so it has no way to pre-stamp; absorbing into the
   implementer is forbidden by the escalation-path analysis above.
2. **Mid-run writes**, whose inputs are computed during the run: `estimate:write:*` (autopilot
   and estimate), `estimate:tier:*`, `advance:*`, `park:*`, `gate:clear-tag:*`, `gate:persist`,
   `act:round-note:*`, `write:draft`.
3. **`gather:*`** — a per-phase mid-run read fan-out in `document.js`.
4. **`fetch:next` iterations 2..N** — only the first is hoistable, because `rdm next` is the
   cursor.
5. **The retained in-workflow fallback path of every hoisted site.** A direct `Workflow`
   invocation has no caller and always takes it.
6. **The distributed-caller path of the five local-only-shim workflows** (`plan-review`,
   `backlog`, `document`, `review`, `estimate`) — until task
   `convert-remaining-skill-templates-to-workflow-shims` lands.
7. **The MCP path of `model:mechanical`/`phaseMeta`/`taskMeta`** — no MCP model-resolve tool
   exists, and the all-or-nothing guard correctly rejects a partial payload.

Model tiering is already done: every one of these resolves to the `mechanical` tier (haiku)
after commits `f4e89d7` and `caebcc0`. What remains for phase 4 is **context size and reasoning
effort**, not tier and not count.

## Measured delta

### Method, and its honest limits

`docs/token-baseline.json` carries **no `byLabel` section**, so the JSON comparison is done at
**class** granularity and the per-label figures are read from the CLI's own `byLabel` output:

```
node scripts/measure-lane-tokens.mjs --format json \
  --workflow autopilot --workflow dispatch-phase --workflow plan-review \
  --workflow backlog --workflow estimate --workflow document
```

The corpus has **grown since the baseline was taken** (42 runs / 2029 agent records at
measurement time, versus the baseline's 40 / 1943), and every one of those runs executed
**pre-change** code — including the dispatch that implemented this phase. A raw
`byAgentClass` subtraction between the two JSON files therefore measures corpus growth, not
this change, and is *not* reported as the delta.

What is reported instead is a **replay delta**: the per-label counts observed over the current
corpus, with this phase's elimination rules applied to them. That is exact and independently
checkable (every rule is a `--workflow`-scoped `byLabel` count), and it states which caller
surface each elimination depends on. A fresh post-change dispatch is the natural confirmation
and should be taken on the next real lane run; it could not be taken from inside the
implementing session, whose own workflow was already running the pre-change code.

Two further caveats carried over from phase 2: the baseline run set contains **zero standalone
`estimate` and `document` runs** (those lanes are only observable nested inside autopilot), and
the `estimate` class **mixes mechanical (`estimate:list`/`write`/`tier`) with judgment
(`estimate:rate`) agents**, so it must never be reported as a single mechanical figure.

### Per-label replay, by workflow

| workflow | label | observed | eliminated | remaining | depends on |
|---|---|---|---|---|---|
| autopilot | `fetch:phase-meta` (nested dispatch) | 39 | 0 | 39 | irreducible — autopilot cannot shell out |
| autopilot | `stamp:in-progress` (nested) | 20 | 0 | 20 | irreducible — same |
| autopilot | `diff:signals` (nested) | 27 | **27** | 0 | absorbed into `implement:*` — works on every path |
| autopilot | `model:mechanical` | 9 | **9** | 0 | `rdm-autopilot` CLI shim |
| autopilot | `estimate:list` | 14 | **14** | 0 | `rdm-autopilot` shim (CLI + MCP) |
| autopilot | `fetch:next` | 47 | **20** (one per run) | 27 | `rdm-autopilot` shim, first iteration only |
| autopilot | `estimate:write:*` | 41 | 0 | 41 | irreducible |
| autopilot | `advance:*` | 29 | 0 | 29 | irreducible |
| autopilot | `park:*` | 8 | 0 | 8 | irreducible |
| dispatch-phase | `fetch:phase-meta` (direct) | 4 | **4** | 0 | `rdm-dispatch-phase` / `rdm-do --auto` CLI shim |
| dispatch-phase | `fetch:task-meta` (direct) | 7 | **7** | 0 | same |
| dispatch-phase | `stamp:in-progress` (direct) | 5 | **5** | 0 | same shim, `alreadyInProgress` |
| dispatch-phase | `diff:signals` (direct) | 10 | **10** | 0 | absorbed |
| plan-review | `model:mechanical` | 6 | **6** | 0 | local `rdm-plan-review` shim |
| plan-review | `fetch:roadmap` | 7 | **7** | 0 | local shim (priority hoist) |
| plan-review | `fetch:wontfix` | 5 | **5** | 0 | local shim |
| plan-review | `gate:clear-tag:*` | 17 | 0 | 17 | irreducible |
| plan-review | `act:round-note:*` | 8 | 0 | 8 | irreducible |
| backlog | `fetch:report` | 1 | **1** | 0 | local `rdm-backlog` shim |
| **total mechanical** | | **304** | **115** | **189** | |

**115 of 304 mechanical agents (37.8%) would not have been spawned** had this phase's code been
live across the same corpus.

### Rolled up to the baseline's `byAgentClass` keys

`docs/token-baseline.json`'s reference figures are `fetch` 105, `stamp` 20, `model` 14,
`diff` 33, `estimate` 95 agents. Rolled up over the current corpus:

| class | observed | eliminated | remaining | vs. `token-baseline.json` |
|---|---|---|---|---|
| `fetch` | 110 | 44 | 66 | baseline 105 |
| `stamp` | 25 | 5 | 20 | baseline 20 |
| `model` | 15 | 15 | **0** | baseline 14 |
| `diff` | 37 | 37 | **0** | baseline 33 |
| `estimate` (mechanical part only) | 55 | 14 | 41 | baseline class 95 **mixes judgment** — not comparable as a whole |
| `advance` / `park` / `gate` / `act` | unchanged | 0 | — | irreducible |

The `model` and `diff` classes go to **zero on the shim-driven paths**. `diff` goes to zero
everywhere for dispatch-phase, because absorption needs no caller.

### Direct measurement of the shipped code

The replay delta above applies this phase's elimination rules to observed counts *by hand*,
which makes it only as trustworthy as the rules. `scripts/measure-hoist-delta.mjs` closes that
gap by **executing the real, post-change driver** under a recording fake `agent` — twice per
mode, once with the args a pre-change caller passed and once with the args the post-change shim
passes — and counting the mechanical subagents each run actually spawns. Agent counts are
therefore observed from the shipped code rather than asserted, and are then priced using
`docs/token-baseline.json`'s own measured per-class figures.

```
node scripts/measure-hoist-delta.mjs
node scripts/measure-hoist-delta.mjs --check docs/mechanical-agent-inventory.md
```

Over one dispatch pair it reports **6 of 6 mechanical subagents (100%) no longer spawned** —
`fetch:phase-meta`/`fetch:task-meta`, `stamp:in-progress` and `diff:signals` in both phase and
task mode. Priced against the baseline that is **1,536,932 tokens eliminated**, or **297,882 on
the fresh (ex-cache-read) column**, which is the decision-relevant one: cache reads dominate the
raw totals and are the cheapest token there is, so the script reports both and neither alone.

Those two figures, and the six labels above, are written here **verbatim** rather than rounded
because `--check` greps this document for them: it recomputes the delta from the shipped code and
fails if the numbers here no longer match, so this section cannot rot into a stale
hand-transcription. `scripts/verify-workflow-dispatch.sh` section 8 runs that `--check` (with a
planted-mutation self-test), so CI enforces it.

Its limits are stated in the script's own header and are worth repeating: a fake agent returns
canned values instantly, so this measures **agent count exactly** and token cost only as
(measured post-change counts) × (measured pre-change per-agent cost). It is not a substitute
for a fresh post-change lane run, which remains the natural confirmation.

### Not measured here

- No judgment agent's model or reasoning effort changed. The *only* change to a judgment call
  site is the `schema` key added to `implement:worktree` / `implement:rework` plus the prompt
  appendix asking for the diff — its `model: models.implement` and effort are untouched, and
  `scripts/verify-workflow-review.sh` §5b-mechanical's assertion that `act:*` is **not** pinned
  still passes unchanged.
- Token *sizes* per surviving mechanical agent are phase 4's subject, not this phase's.

## Context trim: the phase-5 result (measured — but no call site edited)

Phase 5 is the context-size lever over the § Irreducible set above. It is **feasibility-gated
for both of its options**, and it completed on a recorded result rather than a code change. The
full evidence tables live in [`docs/workflow-schemas.md`](workflow-schemas.md) § "agentType /
effort options spike"; the operative outcomes are:

- **`agentType` — resolves, and is worth 19894 tokens per agent (−42 %).** Measured live in
  this worktree by a controlled 2×2 over `claude -p` sessions, which resolve the same
  `.claude/agents/` registry and record the same per-request usage the phase-4 instrument
  reads:

  | `firstRequestTokens` | with `CLAUDE.md` | without | Δ `CLAUDE.md` |
  |---|---:|---:|---:|
  | default agent | 47084 | 27775 | 19309 |
  | `rdm-mechanical` | 27190 | 7870 | 19320 |
  | **Δ `agentType`** | **−19894** | **−19905** | |

  The two factors are independent and additive to within 11 tokens. A per-call `model`
  overrides the definition, so the mechanical-tier pins § Maintenance routes depends on survive
  an `agentType`.
- **…but `agentType` is NOT reachable from the Workflow runtime (measured).** The spike has now
  been dispatched (`wf_2bea58b9-38f`, plus probe `wf_6cca94eb-de0`), and it closed the phase's
  gating sub-question **negatively**. `agent({ agentType: 'rdm-mechanical' })` **raised**
  `agent type 'rdm-mechanical' not found. Available agents: claude, claude-code-guide, Explore,
  general-purpose, Plan, statusline-setup` — a list holding only built-in agent types, with no
  project-local definition of any kind. The definition was copied into the dispatching
  session's project root beforehand and still did not resolve; a retry minutes later threw
  identically, so the registry is a **session-start snapshot**.

  **This is the finding that bounds the whole phase, and it is wider than the distribution
  blocker below.** An `agentType` literal raises on first dispatch from any session whose
  start-of-session registry lacks the definition — which includes every session rooted outside
  this worktree. It therefore applies to the four **local-only** workflows exactly as it does
  to the three distributed ones. Since §2b greps only
  `rdm-core/src/templates/workflows/*.js`, threading `document.js` / `backlog.js` /
  `plan-review.js` / `estimate.js` would have passed every gate in this repo and still broken
  them. **No call site in either group was threaded**, and the ~19 k trim is measured but
  currently undeliverable.
- **`effort` — still not threaded, but the reason changed from "inert" to "out of scope".** The
  verification channel was identified (each `assistant` transcript record carries a top-level
  `effort` field) and its pre-change control fixed (156 384 records across the corpus: `"high"`
  or absent, `"low"` **never**). The two routes were then measured separately and they
  **disagree**:

  | Route | Result |
  |---|---|
  | declared in the agent *definition* frontmatter | ran at `"high"` — accepted, not honored |
  | **`agent(prompt, { effort: 'low' })` from a Workflow run** | **recorded `effort: "low"` — HONORED** |

  Spike case E is the first `"low"` record in the entire corpus, and the four sibling cases in
  the same run all recorded `"high"`, so it is conclusive by the control's own terms. Case H
  adds that an *invalid* effort value is accepted and silently degrades to `"high"` rather than
  throwing — the opposite of `agentType`'s failure mode. `scripts/verify-workflow-review.sh`
  §2b still fails if any workflow script passes `effort:`, but now because the phase body
  forbids threading it (a rule written on the strength of the definition-side negative this run
  overturned), not because the option does nothing. That reversal is handed to
  `finish-agent-type-effort-spike-and-thread-mechanical-sites` scope item 5, which is now live
  and unblocked — its remaining risk is fidelity, not mechanism.
- **`CLAUDE.md` loading — answered: it loads, in full, and cannot be stopped per agent type.**
  The same 2×2 measures it at **19320 tokens** inside the custom `agentType` agent, versus
  19309 inside the default one — an 11-token difference, i.e. a trimmed system prompt and a
  two-tool allowlist displace none of it. That is **60 % more** than the 12052 `chars/4`
  estimate `docs/token-baseline.json` records (measured ratio 2.49 chars/token), and it is
  **71 %** of a trimmed agent's remaining floor. No agent-definition frontmatter key suppresses
  it (`memory:` scopes `~/.claude/agent-memory/` auto-loading and *adds* context); the only
  switches are process-global. Per the phase body this is recorded and dropped — **no
  `CLAUDE.md` restructuring was attempted or is implied.**
- **The distribution assumption was wrong, and it inverts the risk — now OBSERVED.** An
  unresolvable `agentType` is a *raised* error in the runtime, not the silent `null` an unknown
  `model` id produces. This was previously inferred from a runtime string table; the spike
  observed it four times (cases B, C, F and the retry probe), and the script distinguishes a
  throw from a null return explicitly, so the shape is unambiguous. Since `rdm agent-config`
  emits no `.claude/agents/` definitions, threading `agentType` into the three distributed
  workflow templates would **hard-break** every downstream lane on first dispatch rather than
  degrade it. §2b of the review harness gates that. This phase therefore does not introduce a
  distributed dangling reference — it declines to.

### The threadable surface, enumerated (so the next attempt need not re-derive it)

The phase expected to thread ~16 mechanical sites across the four local-only workflows, and
that enumeration was completed before the spike returned. It is recorded here because it stays
valid: it is the exact worklist for whichever option becomes threadable first (`effort:` now,
`agentType:` if it ever resolves). Judgment sites are excluded by § The classification rule and
must stay excluded.

| File | Threadable mechanical sites | Maintenance route |
|---|---|---|
| `document.js` | `model:mechanical`, `fetch:roadmap-meta`, `gather:<stem>`, `write:draft` | unprojected driver (all below `document-core:end`) — edit in place |
| `backlog.js` | `model:mechanical`, `fetch:report` | unprojected driver — edit in place |
| `estimate.js` | `model:mechanical`, `estimate:list`, `estimate:write:<stem>`, `estimate:tier:<stem>` | unprojected driver (below `estimate-core:end`) — edit in place. **Verified NOT stamped**: the generator projects only the `estimate-core` block, so these do not propagate into the distributed `autopilot.js`, which carries its own duplicate driver copies of the same labels |
| `plan-review.js` | `fetch:roadmap`, `fetch:<kind>`, `fetch:wontfix`, `gate:clear-tag:<kind>:<ident>` | **byte-copied** — inside the `plan-review-driver` block; edit `lib/plan-review.mjs` first, then copy verbatim. `verify-workflow-review.sh` §5b-drift gates the pair |
| `plan-review.js` | `model:mechanical` | unprojected driver (below `plan-review-driver:end`) — edit in place |

15 sites, three maintenance routes. **Off-limits regardless:** `autopilot.js`,
`dispatch-phase.js`, `review-refute-fix.js` (byte-identical to the distributed templates, gated
by `verify-agent-config-distribution.sh`), anything stamped from `lib/review.mjs` or
`lib/estimate.mjs`, and every judgment site — `find:*`, `refute:*`, `plan:*`, `implement:*`,
`synthesize:draft`, `analyze:*`, `estimate:rate:*`, and plan-review's `act:*` (including
`act:round-note:*`, which the inventory classes as a mechanical mid-run write but the phase body
excludes).

One per-site check worth keeping: **`write:draft` is threading-compatible.** Its prompt issues
only `mkdir -p` and a `cat` heredoc via Bash and returns a `WRITE_ACK` schema object, so it
needs nothing beyond `rdm-mechanical`'s `tools: Bash, StructuredOutput`. It does not use the
`Write` tool despite its name.

**Nothing in this document's counts changed.** No `agent()` call site was added, removed,
relabelled, or re-tiered, so the § Raw inventory total, the § Classification table, and the
§ Measured delta figures are all untouched and still gate as before.

The comparison point for the eventual change is pinned in `docs/token-baseline.json` under
`mechanicalContextTrim`, quoting phase 4's per-class pre-change medians verbatim so a later run
cannot silently re-baseline; the measured per-agent figures above are recorded alongside them
under `mechanicalContextTrim.measuredTrim2x2` and `.claudeMdFinding`, and the Workflow-path
dispatch under `.workflowPathSpike`. The remaining work is carried by task
`finish-agent-type-effort-spike-and-thread-mechanical-sites`, and the distribution close-out by
`ship-mechanical-agent-type-downstream`.

**What the spike changed about that remaining work.** It was expected to unblock threading; it
did the opposite for one option and the opposite-of-the-opposite for the other. `agentType` went
from "one dispatch away" to blocked on a newly-discovered prerequisite — making a project-local
definition resolvable from a Workflow run at all — which no task named before, and which is
strictly upstream of `ship-mechanical-agent-type-downstream`'s emission surface (emitting a
definition downstream is worthless if the runtime will not read it). `effort` went the other
way, from "inert, do not ship" to "honored, and blocked only on a scope decision plus a
fidelity check". The follow-up task carries both halves and no longer instructs anyone to redo
the spike.
