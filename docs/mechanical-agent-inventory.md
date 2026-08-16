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

**41 labelled `agent()` call sites** across the six workflow scripts:

| file | call sites |
|---|---|
| `rdm-wf-backlog.js` | 3 |
| `rdm-wf-dispatch-phase.js` | 12 |
| `rdm-wf-document.js` | 5 |
| `rdm-wf-estimate.js` | 5 |
| `rdm-wf-plan-review.js` | 11 |
| `rdm-wf-review-refute-fix.js` | 5 |
| **total** | **41** |

(`autopilot.js` carried 7 of the original 44 call sites; it was retired in favor of the prose
`rdm-autopilot` skill by the `workflow-orchestration` roadmap's phase 3 — see
[`docs/workflow-vs-prose-boundary.md`](workflow-vs-prose-boundary.md). The historical rows below
that still cite `autopilot.js`/`advance:*`/`park:*`/`fetch:next` describe the measurements taken
while it was still a workflow script and are left as a dated record rather than rewritten; they
are not live-checked the way the totals above are.)

Adding `.claude/workflows/lib/*.mjs` to the glob raises the count to **52**. That is *not* eleven
extra call sites: the libs hold the single-source originals of blocks that are stamped or
byte-copied into the `.js` consumers, so the same site is counted twice. **The six `.js`
files are the authoritative surface** — they are what the Workflow runtime executes.

Of the 41, **23 are mechanical** and **18 are judgment** agents. (Three of the eighteen are the
canonical finder's `find:<mode>:<dim>:retry` site — the ONE bounded retry a finder that resolved
null gets before its dimension is recorded as non-participating — counted once per stamped
consumer.) The judgment set is out of
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
| **stamped** | `rdm-wf-review-refute-fix` (in `rdm-wf-dispatch-phase.js`, `rdm-wf-plan-review.js`, `rdm-wf-review-refute-fix.js`), `estimate-core` (in `rdm-wf-estimate.js`) | edit the lib (`lib/review.mjs` / `lib/estimate.mjs`), re-run `scripts/gen-workflow-review.sh` / `gen-workflow-estimate.sh`; `--check` gates drift |
| **byte-copied** | `dispatch-outcome` (`lib/dispatch-phase.mjs`), `plan-review-driver` (`lib/plan-review.mjs`) | edit the lib **first**, then copy the block verbatim into the consumer; `verify-workflow-dispatch.sh` §2 / `verify-workflow-review.sh` §5b-drift gate byte-equality |
| **unprojected** | everything below a `:end` marker (the driver regions), plus `document-core`/`backlog-groom` consumers | edit in place |

**Every mechanical call site sits in an unprojected DRIVER region**, *except* plan-review's
`fetch:roadmap` / `fetch:roadmap-body-check` / `fetch:<kind>` / `fetch:wontfix` /
`act:round-note:*` / `gate:clear-tag:*`, which sit inside the byte-copied `plan-review-driver`
block. No mechanical site sits inside a generator-**stamped** block, so no generator had to
learn anything new for this phase.

Independently: `rdm-wf-dispatch-phase.js` and `rdm-wf-review-refute-fix.js` carry
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
| `fetch:phase-meta` | `rdm-wf-dispatch-phase.js` | unprojected driver | yes | distributed shim (`rdm-dispatch-phase`, `rdm-do --auto`, `rdm-autopilot` CLI) — CLI only | **hoistable** | Read-only Stage-0 read, fires before any judgment agent, and the caller already ran `rdm phase show`. Accepted only when the payload carries a non-empty body, all five resolved model ids, **and** the `model` difficulty tier (`hoistedMetaComplete`) — the tier is the driver's sole source for gate strictness and its `'medium'` default would silently loosen a `large` phase's gate. | no (direct/shim path) |
| `fetch:task-meta` | `rdm-wf-dispatch-phase.js` | unprojected driver | yes | distributed shim — CLI only | **hoistable** | Task-mode twin of the above, same body+models guard. No tier requirement: `TASK_META` carries none and the driver hard-codes a task to `medium`, so there is nothing to lose. | no (direct/shim path) |
| `stamp:in-progress` | `rdm-wf-dispatch-phase.js` | unprojected driver | yes | distributed shim — CLI **and** MCP | **redundant** | Interactive `rdm-do`, `rdm-do --auto` and the `rdm-dispatch-phase` shim all write `--status in-progress` before invoking the workflow. Suppressed by an explicit `alreadyInProgress` flag set **only** when that write exited 0, and **never** for a `--plan-only` run. | **yes** (autopilot-nested + direct-`Workflow` paths) |
| `diff:signals` | `rdm-wf-dispatch-phase.js` | unprojected driver | yes | n/a — absorbed, no caller needed | **absorbable** | `runCodeGate` calls `d.implement(...)` immediately before every `d.review()` with nothing in between, so the implementer — already in the worktree it just wrote to — reports the same two `git diff` commands. One-shot handoff (`pendingDiff` read-and-cleared) preserves per-round freshness. Works on **every** path, including autopilot-nested. | no |
| `diff:signals` | `rdm-wf-review-refute-fix.js` | unprojected driver | yes | local shim only (`rdm-review`) | **hoistable** | No adjacent implementer in this workflow (it reviews an already-implemented item), but `worktreeRef` is fully determined by `args`, so the caller can run the diff itself. | partly (distributed-caller path) |
| `gate:persist` | `rdm-wf-review-refute-fix.js` | unprojected driver | yes | — | **irreducible** | A write whose status/reason are computed mid-run from the classified outcome. | **yes** |
| `model:mechanical` | `autopilot.js` | unprojected driver | yes | distributed shim — **CLI only** | **hoistable** | Pure bootstrap read of `rdm model resolve mechanical`, before everything. MCP has no model-resolve tool, so the MCP shim omits it and the agent runs. | partly (MCP path) |
| `estimate:list` | `autopilot.js` | unprojected driver | yes | distributed shim (CLI + MCP) | **hoistable** | `rdm phase list --format json` — read-only, pre-run, no judgment agent before it. | no |
| `fetch:next` | `autopilot.js` | unprojected driver | yes | distributed shim (CLI + MCP) | **hoistable — first iteration only** | `rdm next` is what *advances the cursor* once advance/park has persisted a status, so a cached result is only valid for iteration 1. Consumed strictly one-shot (`pendingNext`); iterations 2..N always re-read live state. | **yes** (iterations 2..N) |
| `estimate:write:*` | `autopilot.js` | unprojected driver | yes | — | **irreducible** | A write whose difficulty/justification inputs are produced mid-run by the rater. | **yes** |
| `advance:*` | `autopilot.js` | unprojected driver | yes | — | **irreducible** | A write keyed on the OUTCOME the dispatch just produced. | **yes** |
| `park:*` | `autopilot.js` | unprojected driver | yes | — | **irreducible** | Same — a mid-run write with a computed reason. | **yes** |
| `model:mechanical` | `rdm-wf-estimate.js` | unprojected driver | no | local shim only (`rdm-estimate`) | **hoistable** | Same bootstrap read as autopilot's. | partly (distributed-caller path) |
| `estimate:list` | `rdm-wf-estimate.js` | unprojected driver | no | local shim only | **hoistable** | Same as autopilot's. | partly (distributed-caller path) |
| `estimate:write:*` | `rdm-wf-estimate.js` | unprojected driver | no | — | **irreducible** | Mid-run write. | **yes** |
| `estimate:tier:*` | `rdm-wf-estimate.js` | unprojected driver | no | — | **irreducible** | Reads back a tier that only exists *after* the writeback it follows. | **yes** |
| `model:mechanical` | `rdm-wf-plan-review.js` | unprojected runtime entry | no | local shim only (`rdm-plan-review`) | **hoistable** | Same bootstrap read — extended by `thread-plan-review-judgment-models` to also resolve `review-find`/`review-verify` in this one call, threaded through as `findModel`/`verifyModel`. | partly (distributed-caller path) |
| `fetch:roadmap` | `rdm-wf-plan-review.js` | **byte-copied** `plan-review-driver` (gate: `verify-workflow-review.sh` §5b-drift) | no | local shim only | **hoistable — PRIORITY** | See § The hoist with a recorded correctness failure. Not ranked on cost. | partly (distributed-caller path) |
| `fetch:roadmap-body-check` | `rdm-wf-plan-review.js` | **byte-copied** `plan-review-driver` (gate: `verify-workflow-review.sh` §5b-drift) | no | local shim only | **not a hoist candidate** | A SECOND, independent verification call for the roadmap-body unit only (task `plan-review-roadmap-body-fetch-status-line`): five recorded runs reviewed that unit against a one-line fetch-status sentence instead of the real body. Re-reads `roadmap show` itself and reports a checkable length/first-line property, compared against `fetch:roadmap`'s transcribed body; a confirmed mismatch discards the fetch and fails closed through the existing empty-body path. Skipped entirely on the hoisted-payload path (no LLM transcription step to distrust there) and for phase/task kinds. Costs one extra mechanical call per agent-fetch roadmap run — a deliberate, documented tension with this phase's own elimination goal, accepted for the correctness gap it closes. **Overlap decision (roadmap's "Known overlap to resolve at phase 3" directive):** does NOT collapse into phase 2's `fetchTranscriptionOk` — that check is deliberately body-content-blind (stem convention + reserved tag tokens only, never body text; see its doc comment in `lib/plan-review.mjs`), so it cannot catch a fetch whose body is a fabricated status sentence with otherwise-clean stems/tags, which is exactly this unit's evidence. | partly (distributed-caller path) |
| `fetch:<kind>` (`fetch:task` / `fetch:phase`) | `rdm-wf-plan-review.js` | **byte-copied** `plan-review-driver` | no | local shim only | **hoistable — PRIORITY** | Same — see below. | partly (distributed-caller path) |
| `fetch:wontfix` | `rdm-wf-plan-review.js` | **byte-copied** `plan-review-driver` | no | local shim only | **hoistable** | One `rdm search` covering the whole run, read-only, before any unit is reviewed. | partly (distributed-caller path) |
| `act:round-note:*` | `rdm-wf-plan-review.js` | **byte-copied** `plan-review-driver` | no | — | **irreducible** | A write whose round number and finding list are computed mid-run. | **yes** |
| `gate:clear-tag:*` | `rdm-wf-plan-review.js` | **byte-copied** `plan-review-driver` | no | — | **irreducible as a write, but SKIPPABLE** | A write keyed on the per-unit outcome the pipeline just produced, so it cannot be hoisted ahead of the run. Two changes from `phase-4-plan-review-gate-blocked-by-safety-classifier`: (a) the prompt is now **evidence-carrying** — a four-clause authorization preamble plus the rendered review evidence (dimensions that ran, findings produced, findings graded by an independent refuter, blocking survivors, the exact tag list to write) — because a bare two-command instruction was blocked as self-approval three times across two recorded runs; (b) a caller may pass **`gateMode: 'return'`**, under which the driver computes the gate as a returned `gateAction` and dispatches **no agent at all**, so a `'return'`-mode run's per-unit count for this site is 0 rather than 1. The agent-count totals above are unchanged: they count call SITES, and this remains one site. See [`plan-review-gate-policy.md`](plan-review-gate-policy.md). | **yes** |
| `model:mechanical` | `rdm-wf-backlog.js` | unprojected driver | no | local shim only (`rdm-backlog`) | **hoistable** | Same bootstrap read. | partly (distributed-caller path) |
| `fetch:report` | `rdm-wf-backlog.js` | unprojected driver | no | local shim only | **hoistable** | `rdm backlog report --format json` is read-only whoever runs it, so hoisting it does not weaken the propose-only contract. | partly (distributed-caller path) |
| `model:mechanical` | `rdm-wf-document.js` | unprojected driver | no | local shim only (`rdm-document`) | **hoistable** | Same bootstrap read. | partly (distributed-caller path) |
| `fetch:roadmap-meta` | `rdm-wf-document.js` | unprojected driver | no | local shim only | **hoistable** | `rdm roadmap show --format json`, read before the all-done validation and before any judgment agent. | partly (distributed-caller path) |
| `gather:*` | `rdm-wf-document.js` | unprojected driver | no | — | **irreducible** | A per-phase mid-run read fan-out whose inputs come from the phase list. | **yes** |
| `write:draft` | `rdm-wf-document.js` | unprojected driver | no | — | **irreducible** | Writes a document the synthesis agent produced mid-run. | **yes** |

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

Most sites here are a pure cost question. `rdm-wf-plan-review.js`'s `fetch:roadmap` and `fetch:<kind>`
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

**Since then (`fix-plan-review-gate-tag-clobber` landed).** The guard named above as
that task's job has now landed — as raw-stdout-capture + driver-side parse/identity-validate
(`.claude/workflows/lib/plan-review.mjs`'s `parseTranscriptBlocks` / `extractRoadmapFromJson` /
`extractPhaseFromJson` / `extractTaskFromJson`), still at the fixed 1-agent-per-target cost this
document holds `fetch:roadmap` to, never a per-phase fan-out. This paragraph is not left
still-open by that landing; see `docs/workflow-schemas.md` § "The fetch stage is now
raw-transcript-capture + driver-side parse" for the design. Content validation of a
caller-supplied `fetched` *hoist* (as opposed to the fetch agent path) remains a separate,
still out-of-scope concern — that path bypasses the agent entirely, so there is nothing for
these driver-side guards to run against.

**Update (`fix-plan-review-gate-tag-clobber` continued — the agent-transcription validation gap is
closed).** Even after the identity-based extraction above landed, the agent-transcription
fallback trusted its own `agent()` return the moment it satisfied the identity checks, with no
further content check. `fetchTranscriptionOk` (`.claude/workflows/lib/plan-review.mjs`, placed
immediately after `hoistedFetchedOk`) adds a further, body-content-blind guard applied ONLY to
that agent-transcribed fetch — never to a caller-hoisted payload, which stays governed by
`hoistedFetchedOk` alone, per the paragraph above. It checks (a) rdm's own `phase-<N>-`
stem-naming convention for every roadmap phase, and (b) that no tag — roadmap-level or
phase-level — equals a literal entry in a small closed `RESERVED_FETCH_TOKENS` list
(`['fetch', 'plan-target']`, lifted verbatim from both recorded incidents' own fabricated tags).
It never reads `fetched.body`'s text beyond `hoistedFetchedOk`'s existing non-empty check. On
failure, ONE bounded retry (a fresh, independent `agent()` call — never a reuse of the rejected
attempt) runs before falling into the existing fail-closed `fetchFailed` path. Both recorded
corruption payloads are replayed as negative regression tests in
`scripts/verify-workflow-review.sh` §7g (direct `fetchTranscriptionOk` assertions) and §7h
(driven through `runPlanReviewDriver`: corruption-replay fail-closed for both incidents, a
retry-recovery positive test proving no field from a rejected first attempt leaks into the
gate write, the empty-phases and body-text-mimicry non-tripping edge cases, and a
task/phase/roadmap/implementation-plan sweep), plus two mutation self-tests in §7e proving the
checks are not vacuous. The caller-hoisted path's content validation remains explicitly out of
scope, unchanged.

This also closes the task's own "Workflow-driven via Bash, to the extent the runtime allows"
directive rather than merely approximating it: `docs/workflow-schemas.md` § "The fetch stage is
now raw-transcript-capture + driver-side parse" (its "Why this closes the 'Workflow-driven via
Bash' directive" paragraph) walks the Import spike's proof that no script-level Bash/process
primitive exists in this runtime at all, so the only lever "to the extent allowed" can mean is
narrowing what the dispatched agent may do — done by `agentType: 'rdm-mechanical'`'s hard
`tools: Bash, StructuredOutput` restriction (confirmed enforced from inside a Workflow run) plus
the single-field `RAW_STDOUT_SCHEMA` that leaves no room to compose anything but the one command's
verbatim stdout — combined with the directive's own explicit fallback for that case, driver-side
validation "as strictly as a genuine call would allow," which is exactly `fetchTranscriptionOk`
plus the retry-then-fail-closed loop described above.

**Update (`fix-plan-review-gate-tag-clobber` continued — the gate now writes from a pre-fetch
cache, not from the review unit's own copy).** An AC-review pass on this same task found one more
literal gap: the phase body's "Implementation constraints" explicitly asked for the gate to
"cache the item's real tags before the fetch runs, then filter and write back the filtered
ORIGINAL tags — never the fetched tags," and no caching mechanism existed anywhere — the write
read `u.tags`, a copy `buildReviewUnits` had already threaded through the review-unit object
alongside `body`/`target` for the review pipeline's own purposes. `snapshotOriginalTags` (placed
immediately after `buildReviewUnits` in `.claude/workflows/lib/plan-review.mjs`) closes this: it
is called exactly once, right after `fetched` is accepted (via either the caller hoist or the
validated agent-transcription retry loop) and *before* `buildReviewUnits` or anything downstream
runs, and caches every unit's real tags — the roadmap's own plus each phase's own, keyed by
stem — into a dedicated map. The gate's `--tags` write reads only from that map; `u.tags` is no
longer read at the write site at all. A genuinely second, independent verification fetch (running
`roadmap show`/`task show`/`phase show` a second time solely to cross-check tags) was considered
and declined: it would double the mechanical-agent cost of every plan-review target and violate
this same task's own prior commitment that "the common case still issues exactly one
fetch:roadmap/fetch:task/fetch:phase call" (the `buildRoadmapFetchPrompt` "must not be
reintroduced" note above). So this closes the *structural* half of the ask — the write can no
longer be corrupted by a bug anywhere in `buildReviewUnits`, `reviewUnit`, or the review pipeline
— without re-opening the per-target agent-count question; it does not, and by construction cannot,
make the write independent of `fetchTranscriptionOk`'s own correctness, since both the snapshot
and `buildReviewUnits`' units are still built from the same validated `fetched`.
`scripts/verify-workflow-review.sh` §5b-cache asserts the write site by name (with two
planted-mutation self-tests: reverting to `u.tags`, and dropping the snapshot call entirely) and
§5b-exec drives `snapshotOriginalTags` directly (roadmap multi-phase keying, missing-tags
defaulting to `[]`, task/phase single-unit keying, a null fetch caching nothing), backed by a
5b-mut mutation self-test that guts the function to always cache nothing.

One consequence is worth stating on its own, because the hoist's shape guard replaces a
`required`-bearing schema: `hoistedFetchedOk` is held to be **no weaker** than the
`{ body, tags, phases }` shape `buildReviewUnits` requires (formerly enforced by the fetch
agent's own `PLAN_TARGET_SCHEMA` / `ROADMAP_TARGET_SCHEMA`, since replaced — see above — by a
single `RAW_STDOUT_SCHEMA` the agent satisfies and the driver parses). `tags` is required, so
the guard requires a `tags` array of strings — on the payload *and* on every roadmap phase entry
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
3. **`gather:*`** — a per-phase mid-run read fan-out in `rdm-wf-document.js`.
4. **`fetch:next` iterations 2..N** — only the first is hoistable, because `rdm next` is the
   cursor.
5. **The retained in-workflow fallback path of every hoisted site.** A direct `Workflow`
   invocation has no caller and always takes it.
6. **The distributed-caller path of the five local-only-shim workflows** (`rdm-wf-plan-review`,
   `rdm-wf-backlog`, `rdm-wf-document`, `review`, `rdm-wf-estimate`) — until task
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
`rdm-wf-estimate` and `rdm-wf-document` runs** (those lanes are only observable nested inside autopilot), and
the `rdm-wf-estimate` class **mixes mechanical (`estimate:list`/`write`/`tier`) with judgment
(`estimate:rate`) agents**, so it must never be reported as a single mechanical figure.

### Per-label replay, by workflow

| workflow | label | observed | eliminated | remaining | depends on |
|---|---|---|---|---|---|
| autopilot | `fetch:phase-meta` (nested dispatch, CLI lane) | 39 | 0 (historical) / **39 projected** | 39 (historical) / 0 projected | eliminated via direct Bash — `rdm-autopilot` skill (CLI), see note below |
| autopilot | `stamp:in-progress` (nested) | 20 | 0 | 20 | irreducible for elimination — this is a *write*, not a read, so autopilot still cannot produce it without a live call; already on the mechanical tier, so nothing to size |
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

**Correction (`regularize-mechanical-agents`):** the `fetch:phase-meta` (nested dispatch) row
above previously read "irreducible — autopilot cannot shell out". That framing was already
false by the time it was written and is corrected here: "cannot be hoisted" (true — a headless
Workflow script cannot shell out to build the payload itself) does not imply "cannot be sized"
(false — the caller that *invokes* the nested dispatch can build and forward it). Architecture
context: autopilot is no longer a headless `.claude/workflows/autopilot.js` Workflow that reaches
`rdm-wf-dispatch-phase` through a nested, JS-mediated `agent()` call (that file was retired by the
`prose-autopilot-orchestration` roadmap); it is the prose `.claude/skills/rdm-autopilot/SKILL.md`
skill, which is already a live agent with Bash access and invokes `rdm-wf-dispatch-phase` directly
via the `Workflow` tool. On the CLI surface it now runs the same `phase show` + five
`model resolve` calls `buildFetchPrompt` would have delegated to a Stage-0 agent, and forwards the
result as `phaseMeta` — eliminating that Opus-tier call per dispatched phase **on the success
path**, the same way `fetch:next`/`estimate:list`/`model:mechanical` above already do. This is
deliberately not unconditional: `hoistedMetaComplete` is all-or-nothing, and a cold direct
`Workflow` invocation must keep resolving models via the unsized bootstrap fetch (nothing about
that path changed), so if the skill's own `phase show` or any of the five `model resolve` calls
fails, it forwards no `phaseMeta` at all and the same unsized Stage-0 agent runs for that one
dispatch — an accepted, documented residual, not a closed gap. The MCP surface is unaffected by
design (`skill-autopilot-mcp.md` has no Bash or MCP model-resolve tool to run the procedure with),
so this elimination is CLI-lane only, and even there is bounded to the pre-fetch's own success;
see `skill-autopilot-mcp.md`'s explanatory note.

**Measured delta.** The row's `observed`/`eliminated`/`remaining` **historical** figures (39 / 0 /
39) are unchanged and stay historical: they are artifacts of one specific pre-migration
instrumented run (`wf_133bc5a5-ce3`, predating both the prose migration and this fix), and this
correction does not fabricate a fresh count against that corpus absent a new instrumented run. What
*can* be stated without a new run is a **projected** delta, and it is grounded in an
already-measured number from this same table, not invented: the two direct-dispatch rows below
(`fetch:phase-meta (direct)`, 4/4, and `fetch:task-meta (direct)`, 7/7) use the *identical*
`hoistedMetaComplete` guard and the *identical* five-model-resolve-plus-body assembly, just invoked
by a different caller (the `rdm-dispatch-phase`/`rdm-do --auto` shim instead of `rdm-autopilot`) —
and both measured **100%** elimination across their observed corpus. Since the nested autopilot
case runs the same assembly procedure against the same guard, on the CLI lane the expected
steady-state rate is the same **100% of the 39 observed calls (39 eliminated, 0 remaining)**,
*conditional on* the phase body being non-empty and all five `model resolve` calls succeeding —
which is the common case, not the exception, matching what the 4/4 and 7/7 rows already
demonstrate for the same mechanism. The table above states both figures explicitly: 0 historical
(this run predates the fix) and 39 projected (grounded in the sibling rows' 100% rate), so a reader
is not left inferring the fix accomplished nothing. This is a projection from already-measured
sibling data, not a new measurement of this row's own corpus — a fresh instrumented autopilot run
would be needed to confirm the projected figure directly, and is not run here.

### Rolled up to the baseline's `byAgentClass` keys

`docs/token-baseline.json`'s reference figures are `fetch` 105, `stamp` 20, `model` 14,
`diff` 33, `rdm-wf-estimate` 95 agents. Rolled up over the current corpus:

| class | observed | eliminated | remaining | vs. `token-baseline.json` |
|---|---|---|---|---|
| `fetch` | 110 | 44 | 66 | baseline 105 |
| `stamp` | 25 | 5 | 20 | baseline 20 |
| `model` | 15 | 15 | **0** | baseline 14 |
| `diff` | 37 | 37 | **0** | baseline 33 |
| `rdm-wf-estimate` (mechanical part only) | 55 | 14 | 41 | baseline class 95 **mixes judgment** — not comparable as a whole |
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
- **Workflow-path resolution: CONFIRMED, and the trim is HALF what the 2×2 predicted.** The
  spike's first `agentType` cases were invalid — dispatched from a session whose project root
  had no `.claude/agents/` directory at session start, which Claude Code's docs name as a
  restart case. Re-run on 2026-07-28 from a session rooted in this worktree (`wf_40f5594e-208`),
  case B resolved and reported `toolNames: ["Bash", "StructuredOutput"]` against the control's
  nine, and case C's registry listing now enumerates `rdm-mechanical`.

  The controlled A/B pair — same session, identical prompt, back to back — measures
  **38689 → 29782, a saving of 8907 tokens (−23.0 %)**, reproduced exactly by the E/F pair. A
  live `rdm-wf-backlog` dispatch (propose-only, verified zero-mutation) agrees against the pinned
  medians: `model:mechanical` 36877 → 29524 (−19.9 %) and `fetch:report` 30098 → 24418
  (−18.9 %), both n=1 and therefore directional rather than a re-baseline.

  **The 19894-token (−42 %) figure from the `claude -p` 2×2 overstates these call sites by
  2.23×** and must not be quoted for them. Both ends compress on the Workflow path: its default
  floor is far cheaper than a CLI session's (38689 vs 47084) while its trimmed agent is slightly
  dearer (29782 vs 27190). The 2×2 measures a different call path, not a wrong one.

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
  throw from a null return explicitly, so the shape is unambiguous. At the time this phase
  landed, `rdm agent-config` emitted no `.claude/agents/` definitions, so threading `agentType`
  into the two distributed workflow templates would have **hard-broken** every downstream lane
  on first dispatch rather than degrading it; `scripts/verify-workflow-review.sh` §2b gated
  that, and this phase declined to introduce a distributed dangling reference. **That gap is
  now closed:** `ship-mechanical-agent-type-downstream` landed `generate_agents()`, which ships
  `.claude/agents/rdm-mechanical.md` into every downstream tree, and
  `scripts/verify-agent-config-distribution.sh` § 3c is the reference-resolution gate that
  replaced §2b's blanket prohibition. Neither distributed template threads `agentType` yet —
  that remains separate follow-up work — but a reference could now resolve if one were added.

### The threaded surface, enumerated

These are the mechanical sites that now carry `agentType: 'rdm-mechanical'` — 15 call sites in
the four local-only workflows, 4 of them duplicated into `lib/plan-review.mjs` as the
byte-copied source of `rdm-wf-plan-review.js`'s driver block, for 19 records in total. The same table was also
the worklist for `effort:`, and that worklist is now **closed by a recorded negative, not
consumed** (2026-08-15). The gating fidelity study ran — `spike-agent-type.js`'s
`mode: 'fidelity'` branch, run `wf_0e8e31e2-415`, 15 pairs / 30 dispatches — and its transcription
half passed 15/15. Threading was still refused, on two grounds: the same 15 pairs show no
output-token drop (11831 low vs 9819 control, 8 pairs up / 7 down), and the mechanical tier
resolves to haiku, which emits no `effort` field at all in 9914/9914 corpus records, so the
option is unfalsifiable exactly where these sites run. No site carries `effort:`, §2b's ban
stands, and `docs/workflow-schemas.md` § "agentType / effort options spike" → the follow-up
carries the full record. Judgment sites are
excluded by § The classification rule and must stay excluded;
`scripts/verify-workflow-review.sh` §2c asserts both directions with planted-mutation
self-tests.

**One structural fact about this table that is easy to get wrong**, and which both this
roadmap's phase body and its approved plan did get wrong: of these 19 records, exactly ONE is
dispatched through a `parallel()` fan-out — `gather:<stem>` in `rdm-wf-document.js`. Plan-review's
`gate:clear-tag:*` and estimate's `estimate:write:*`/`tier:*` run in sequential loops *after*
their workflow's parallel barrier, not inside its thunk. It mattered because whether `agentType`
resolves through `parallel()` was the last open question, and only a `rdm-wf-document` dispatch
could answer it. Two such dispatches have since run (`wf_762e3030-762`, `wf_e6452cce-cf7`): all
twelve `gather:*` fan-out agents resolved, none raised, each run's six share a single `queuedAt`
and start within ~0.4 s, and their `firstRequestTokens` median is 26923 — the trimmed side.
**Resolution through `parallel()` is confirmed.** `verify-workflow-review.sh` §2c(v) still pins
the set so a future refactor cannot move a mechanical site into a fan-out unnoticed; see
`docs/workflow-schemas.md` § "agentType / effort options spike" → the follow-up.

| File | Mechanical sites now carrying `agentType` | Maintenance route |
|---|---|---|
| `rdm-wf-document.js` | `model:mechanical`, `fetch:roadmap-meta`, `gather:<stem>`, `write:draft` | unprojected driver (all below `document-core:end`) — edit in place |
| `rdm-wf-backlog.js` | `model:mechanical`, `fetch:report` | unprojected driver — edit in place |
| `rdm-wf-estimate.js` | `model:mechanical`, `estimate:list`, `estimate:write:<stem>`, `estimate:tier:<stem>` | unprojected driver (below `estimate-core:end`) — edit in place. **Verified NOT stamped**: the generator projects only the `estimate-core` block, so these do not propagate into any distributed workflow (`autopilot.js` formerly carried its own duplicate driver copies of the same labels, before its retirement to prose) |
| `rdm-wf-plan-review.js` | `fetch:roadmap`, `fetch:roadmap-body-check`, `fetch:<kind>`, `fetch:wontfix`, `gate:clear-tag:<kind>:<ident>` | **byte-copied** — inside the `plan-review-driver` block; edit `lib/plan-review.mjs` first, then copy verbatim. `verify-workflow-review.sh` §5b-drift gates the pair |
| `rdm-wf-plan-review.js` | `model:mechanical` | unprojected driver (below `plan-review-driver:end`) — edit in place |

16 sites, three maintenance routes, all threaded. **Off-limits regardless:**
`rdm-wf-dispatch-phase.js`, `rdm-wf-review-refute-fix.js` (byte-identical to the distributed templates, gated
by `verify-agent-config-distribution.sh`), anything stamped from `lib/review.mjs` or
`lib/estimate.mjs`, and every judgment site — `find:*`, `refute:*`, `plan:*`, `implement:*`,
`synthesize:draft`, `analyze:*`, `estimate:rate:*`, and plan-review's `act:*` (including
`act:round-note:*`, which the inventory classes as a mechanical mid-run write but the phase body
excludes).

One per-site check that mattered: **`write:draft` is threading-compatible.** Its prompt issues
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

**What the spike changed about that remaining work.** `effort` moved from "inert, do not ship"
to "honored, and blocked only on a scope decision plus a fidelity check" — that half is sound
and actionable. `agentType` did **not** move: its cases were invalid, so it remains where it
was, one valid dispatch away from an answer. The follow-up task carries both halves, and it
must re-run the spike for the `agentType` half specifically — from a session that can see the
definition at session start, a condition the invalid run did not meet.

**Since then (2026-08-14 / 2026-08-15).** The `agentType` half is **closed**: it was re-run
validly, threaded, its lane saving is measured across three agent classes at n=16–25 rather
than n=1 (`fetch` −14.6 %, `gate` −14.2 %, `model` −13.8 % against the pinned pre-change
medians), and its last open question — resolution through a `parallel()` fan-out — is now
confirmed by two `rdm-wf-document` dispatches. Two threaded sites in this table,
`estimate:list` and `rdm-wf-estimate.js`'s own `model:mechanical`, have **never dispatched** in
the recorded corpus (every estimate run took the caller-hoist path), so they remain threaded but
unmeasured. The `effort` half is **also closed, as a negative**: the fidelity study ran, its
transcription half passed 15/15, and threading was still refused for want of an output-token
drop and for want of any verification channel on the mechanical tier's model. Figures and method
live in `docs/token-baseline.json` § `mechanicalContextTrim` (`laneDeltaBroadened`,
`parallelDispatchConfirmed`, `effortFidelity`).
