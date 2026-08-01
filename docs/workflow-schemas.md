# Workflow-tool orchestration: conventions & schema contracts

The autonomous lane of rdm's tooling is expressed as **Claude Code Workflow-tool
scripts** under `.claude/workflows/`, a sibling of `.claude/skills/` and
`.claude/hooks/`. This document defines the conventions those scripts follow and
the canonical schema contracts they exchange.

> **Scope:** mostly dogfood-only, with one emitted exception. The two workflow
> scripts — `dispatch-phase.js`, `review-refute-fix.js` — ARE now
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
> lane: `skill-autopilot-{cli,mcp}.md` is now a **prose** skill that itself
> drives the roadmap loop, invoking `dispatch-phase` (and, locally, `estimate`)
> as ordinary `Workflow` calls rather than being a thin shim over a workflow
> script of its own — see `docs/workflow-vs-prose-boundary.md` for why autopilot
> was retired from `.claude/workflows/` in favor of prose. `skill-dispatch-phase-{cli,mcp}.md`
> remains a thin shim that invokes `dispatch-phase.js` via the `Workflow` tool,
> instead of re-narrating the orchestration in prose. Distributing the
> still-unshipped pieces (parameterization, `lib/`, a downstream regeneration
> story) remains a follow-up roadmap.

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

**`plan-review.js` omits `findModel`/`verifyModel` — that omission is an
OVERSIGHT, not policy.** `.claude/workflows/lib/plan-review.mjs` calls
`runPlanReview({ target })` at both call sites, so `buildReviewPipeline` sees no
`ctx.findModel`/`ctx.verifyModel` and its finders and refuters inherit the
ambient session model (see the "key omitted" row above), while the sibling
`dispatch-phase.js` threads
`{ findModel: models.review_find, verifyModel: models.review_verify }`. The
counter-argument that judgment sites are deliberately unpinned does not cover
this case: `f4e89d7` and `scripts/verify-workflow-review.sh` §5b-mechanical both
govern only the MECHANICAL pin, and no artifact says a judgment site should
carry no model at all. Evidence, citations and the follow-up task are in
[`docs/refuter-model-tiering.md`](refuter-model-tiering.md) — not restated here.

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

### agentType / effort options spike (can a mechanical subagent be trimmed?)

**Status: SPIKE RUN, with one half of it INVALID.** `spike-agent-type.js` was
dispatched via the `Workflow` tool on 2026-07-27 (run `wf_2bea58b9-38f`, 8 cases,
5 OK / 3 threw), plus a two-case follow-up probe (`wf_6cca94eb-de0`).

- **Q2 is POSITIVE**, reversing the definition-side negative.
  `agent(prompt, { effort: 'low' })` produced a transcript recording
  `effort: "low"` — the first such record in a 156 384-record corpus. This result
  is sound and unaffected by the problem below.
- **Q1a is UNRESOLVED — the `agentType` cases did not test what they appear to.**
  They raised `agent type not found`, but for a **documented setup reason on the
  dispatching side, not a runtime limitation**: the spike was dispatched from a
  session whose project root had no `.claude/agents/` directory at session start,
  and the definition was copied in mid-run. Claude Code's subagent documentation
  states the watcher "covers only directories that existed when the session
  started, so after creating a scope's first agent file in a new `agents`
  directory, restart to load it." The cases therefore measured the operator's
  mistake, and **must not be read as evidence about `agent({ agentType })`.**
  See § Q1a for the full retraction.

**`agentType` IS threaded at the mechanical call sites of the four local-only
workflows** (19 records across `document.js`, `backlog.js`, `estimate.js`,
`plan-review.js` and `lib/plan-review.mjs`), on the strength of the documented
registry behaviour plus the measured `--agent` resolution — with Q1a's confirming
dispatch still outstanding. `effort:` is threaded nowhere. See "Disposition" at
the end.

Per-agent context is the whole cost of a mechanical agent:
`docs/token-baseline.json` records `agentContextFloor.measuredFloor.reportedFloorTokens`
= **38837.5** (median first transcript request, before any tool use, n=1920) and
`agentContextFloor.legacyRegression.interceptTokens` = **37552**. Two `agent()`
options plausibly move it — `opts.agentType` (resolve the subagent against a
purpose-built definition with a trimmed system prompt and a restricted tool list)
and `opts.effort: 'low'` (mechanical agents transcribe command output; they make
no judgment call). Neither appears at any call site under `.claude/workflows/`.

The discipline is the one the `model` spike above established: an option that is
*accepted* is not evidence it is *honored*. That spike recorded `model: undefined`
as inert and an unknown model id resolving the agent to `null` rather than
throwing — both invisible to a caller who only checks that the key did not throw.

#### The apparatus

| Artifact | Purpose |
|---|---|
| `.claude/agents/rdm-mechanical.md` | The custom agent definition. Minimal system prompt, `tools: Bash, StructuredOutput`. Deliberately carries **no `model:` key** — every mechanical call site already passes `model: models.mechanical` / `_mechanicalModel`, and `scripts/verify-workflow-review.sh` §5b-mechanical asserts that pinning. |
| `.claude/workflows/spike-agent-type.js` | Sequential probe. Crosses `agentType` (absent / `'rdm-mechanical'` / unknown id / `undefined`) with `effort` (absent / `'low'` / `undefined` / invalid), one identical trivial prompt per case, each returning a small schema'd probe object. Excluded from the inventory gate by `scripts/verify-workflow-dispatch.sh` §7 and from `docs/mechanical-agent-inventory.md`'s mechanical-label derivation; **not** excluded from the dir-wide hygiene greps, so it complies with them. |

<a id="the-workflow-run"></a>

#### The Workflow-path run (live, `wf_2bea58b9-38f`)

The probe was dispatched. All 8 cases, verbatim:

| Case | `opts` | Shape | Detail |
|---|---|---|---|
| A-control | `{}` | OK | full default tool list |
| B-agentType-valid | `{agentType:'rdm-mechanical'}` | **THREW** | `agent({agentType}): agent type 'rdm-mechanical' not found. Available agents: claude, claude-code-guide, Explore, general-purpose, Plan, statusline-setup` |
| C-agentType-unknown | `{agentType:'no-such-agent-xyz'}` | **THREW** | same error string, same "Available agents" list |
| D-agentType-undefined | `{agentType:undefined}` | OK | inert |
| E-effort-low | `{effort:'low'}` | OK | **transcript records `effort:"low"`** |
| F-effort-low+agentType | `{effort:'low',agentType:'rdm-mechanical'}` | **THREW** | agentType error; effort never reached |
| G-effort-undefined | `{effort:undefined}` | OK | inert |
| H-effort-invalid | `{effort:'not-an-effort-xyz'}` | OK | ran at `"high"` — accepted, silently degraded, no throw |

Every OK case reported the identical full tool list
(`Artifact, Bash, Edit, Read, ReportFindings, Skill, ToolSearch, Write,
StructuredOutput`) and an identical `firstRequestTokens` of **37725** — expected,
since all five ran as the default agent. The run therefore yields **no
Workflow-path floor measurement**; the 2×2 below remains the only floor evidence.

> **⚠ Cases B and F are INVALID — see § Q1a.** They were dispatched from a session
> that could not see the definition, for a documented reason, so their throws say
> nothing about whether a *properly registered* `agentType` resolves. Case C
> (a genuinely unknown id) and case H remain valid: C exercises the not-found path
> deliberately, and neither depends on the registry containing our definition.

**What the run does establish about the two options' failure modes** is an
asymmetry, and this part stands: an `agentType` that the registry does not contain
*raises* (C, deliberately), while an invalid `effort` string is *accepted and
silently degraded* (H).

**The questions did not have to wait for it.** `agentType` resolves against the
same `.claude/agents/` registry the CLI's own `--agent <name>` flag uses, and a
`claude -p` session records exactly the same per-request usage the phase-4
instrument reads out of `agent-*.jsonl`. That makes a **controlled 2×2** possible
with no `Workflow` tool at all — and, unlike a single spike run, it isolates each
variable rather than confounding them.

<a id="the-2x2"></a>

#### The measurement (2×2, live, this repo)

One identical trivial prompt (`Run the command: echo RESOLUTION_PROBE_OK`), run
four times in this worktree on `claude-opus-5`. Two factors, crossed:

- **agent** — the session default agent vs. `--agent rdm-mechanical` (this repo's
  definition, resolved out of `.claude/agents/`).
- **project `CLAUDE.md`** — present, vs. temporarily moved aside for the duration
  of the run and restored immediately after. The user-global
  `~/.claude/CLAUDE.md` (13040 chars) is present in **all four** cells and
  therefore cancels out of every difference below.

Cell values are `firstRequestTokens` — `input + cache_creation + cache_read` on
the session's first assistant record, the same quantity
`scripts/lib/token-report.mjs` computes per agent, so it is directly comparable to
`floorByAgentClass`. It is cache-placement-independent by construction (the three
classes are summed), which is why the numbers are stable despite the four runs
warming each other's caches.

| `firstRequestTokens` | project `CLAUDE.md` present | absent | **Δ `CLAUDE.md`** |
|---|---:|---:|---:|
| default agent | 47084 | 27775 | **19309** |
| `--agent rdm-mechanical` | 27190 | 7870 | **19320** |
| **Δ `agentType`** | **−19894** | **−19905** | |

The two factors are **independent and additive to within 11 tokens** on both
margins — a 0.06 % disagreement across a 39 k-token span. Neither effect is an
artifact of the other, and both replicate.

Every number in Q1 and Q3 below is read off this table.

#### Q1 — is the `agentType` registry reachable, and does a definition resolve?

**Mechanism: YES, and resolution is real.** Read out of the shipped Claude Code
`2.1.220` runtime binary rather than inferred:

| Evidence (runtime string table) | What it establishes |
|---|---|
| `agent()` opts destructure `schema`, `model`, `effort`, `isolation`, `agentType` | `agentType` and `effort` are first-class `agent()` options, not keys that fall through to an ignore-unknowns path |
| `` agent({agentType}): agent type '…' not found. Available agents: … `` | `agentType` is resolved against a registry, and the lookup can fail — i.e. it is a real lookup, not a decorative label |
| `` agent({agentType}): '…' is denied by permission rule '…' `` | the resolved definition's tool list is *enforced*, which is the mechanism the trim depends on |
| `/context` renders "Custom agents … `.claude/agents/`" | `.claude/agents/` is the registry directory |
| Task-tool docs: "Each agent type's model, reasoning effort, and tools come from its definition (`.claude/agents/*.md` frontmatter or SDK `agents`)" … "the `model` parameter here overrides the definition for this one call" | definitions carry model + reasoning effort + tools; a per-call `model` wins over the definition — so our call sites' existing `model:` pins survive an `agentType` |

**This repo, live: YES — `.claude/agents/rdm-mechanical.md` resolves, and the
trim is real and large.** `claude --agent rdm-mechanical -p "Run the command: echo
RESOLUTION_PROBE_OK"`, run in this worktree, resolved the definition, ran the
command through `Bash`, and answered in exactly the definition's voice (the
command, its output, its exit status, no commentary) — so the registry lookup, the
custom system prompt, and the restricted tool list are all in force, not merely
accepted.

| Question | Answer | Evidence |
|---|---|---|
| Does the registry exist and is it reachable? | **YES** | `.claude/agents/` — the definition resolved by name |
| Does *this repo's* definition resolve? | **YES** | the run above succeeded; an unresolvable name raises instead |
| Is the restricted tool list enforced? | **YES** | the agent ran `Bash` and nothing else; the runtime carries a dedicated `agent({agentType}): '…' is denied by permission rule '…'` error |
| How much context does it save? | **19894–19905 tokens (−42.3 % / −41.7 %)** | [the 2×2](#the-2x2), both `CLAUDE.md` conditions |

That trim is *net of* `CLAUDE.md`, which loads either way (Q3) — it is the default
agent's system prompt plus the tool schemas of every tool `rdm-mechanical` does
not get, and it is the entire prize on offer.

##### Q1a — does it resolve from *inside a Workflow run*? **YES — CONFIRMED 2026-07-28.**

This is the sub-question AC4 gates on. The [Workflow-path run](#the-workflow-run)
appeared to close it negatively. **That reading is retracted.** The `agentType`
cases were confounded by how the run was set up, and they measured the setup, not
the runtime.

*What happened.* The spike was dispatched from a session rooted at the main
checkout (`/Users/edward/Projects/rdm`), not at this worktree. That root had **no
`.claude/agents/` directory at all** when the session started. Noticing this, the
operator created the directory and copied the definition in *mid-session*, then
dispatched. Cases B and F raised `agent type 'rdm-mechanical' not found`, and a
retry probe minutes later raised identically — which looked like proof that the
registry is a non-refreshing snapshot.

*Why that inference was wrong.* Claude Code's subagent documentation describes
this exact situation as a known restart case:

> "Claude Code watches `~/.claude/agents/` and `.claude/agents/`. When you add or
> edit a subagent file on disk … Claude Code detects the change within a few
> seconds and the next delegation uses the updated definition, with no restart
> needed. Two cases still need a restart: the watcher covers only directories that
> existed when the session started, so **after creating a scope's first agent file
> in a new `agents` directory, restart to load it.**"

Creating the scope's first agent file in a new `agents` directory is precisely
what was done, so the watcher never covered it and the definition was never
loaded. The retry probe did not confirm a snapshot; it confirmed this documented
restriction. The docs also note project subagents are "discovered by walking up
from the current working directory", so the worktree's own committed
`.claude/agents/` was never in scope for a session rooted elsewhere either.

*What this means for the two hypotheses.* The earlier H1 ("a Workflow run never
consults a project-local `.claude/agents/`") is **ruled out by the documentation**
— the directory is consulted, and watched live once it exists at session start.
H2 was never really in question either. Neither is a finding of this run.

| Case | Status |
|---|---|
| B, `agentType: 'rdm-mechanical'` | **INVALID** — definition not loadable in the dispatching session |
| F, same + `effort` | **INVALID**, same cause |
| C, a deliberately unknown id | **Valid** — an id absent from the registry raises. Independent of our definition |
| Retry probe `wf_6cca94eb-de0` | **INVALID** — re-tested the same unwatched directory |

**What is still known, and it is not nothing.** The definition resolves and its
restricted tool list is enforced through the CLI's session-agent path — that is
what [the 2×2](#the-2x2) measured, and it is where the 19894-token figure comes
from. The Agent-tool contract states an `agentType` is resolved "from the same
registry as the Agent tool", i.e. `.claude/agents/`. So the residual gap is
narrow: whether `agent({ agentType })` inside a Workflow run reads that registry
the same way, tested from a session that can actually see the definition.

**That dispatch has now been done** (run `wf_40f5594e-208`), from a session
restarted with its root inside this worktree so `.claude/agents/` was present at
session start. **The answer is YES:**

| Evidence | Result |
|---|---|
| Case B `toolNames` | **`["Bash", "StructuredOutput"]`** vs the control's nine — the definition loaded *and* its tool restriction is enforced |
| Case C's registry listing | now enumerates `rdm-mechanical` alongside the built-ins |
| Live `backlog` lane sidecars | `{"agentType":"rdm-mechanical","model":"haiku"}` — confirming a per-call `model` still overrides the definition, the assumption behind omitting `model:` from the agent file |

AC4's precondition is therefore met, and the threading described below rests on a
measured result rather than a documented expectation.

<a id="workflow-path-trim"></a>

##### The Workflow-path trim, measured — and it is HALF the 2×2's prediction

Cases A and B ran back-to-back in one session on an identical prompt:

| Case | `firstRequestTokens` |
|---|---:|
| A — control | 38689 |
| **B — `agentType`** | **29782** |
| **Δ** | **8907 (−23.0 %)** |

Cases E/F repeat the pair with `effort: 'low'` on both sides and reproduce it
*exactly*: 38689 → 29782. All five default-agent cases measured exactly 38689;
both `agentType` cases exactly 29782.

A first live lane dispatch (`backlog`, propose-only, verified zero-mutation)
agrees, against the pinned per-class medians:

| Site | post | pinned pre | Δ |
|---|---:|---:|---:|
| `model:mechanical` | 29524 | 36877 (n=16) | −7353 (−19.9 %) |
| `fetch:report` | 24418 | 30098 (n=112) | −5680 (−18.9 %) |

n=1 per class, so those are **directional, not a re-baseline**.

**The 19894-token figure from [the 2×2](#the-2x2) overstates these call sites by
2.23×.** Both ends compress on the Workflow path: a Workflow subagent's default
floor is far cheaper than a CLI session's (38689 vs 47084 — fewer tools, no skill
listing, leaner harness), while the trimmed agent is slightly *dearer* (29782 vs
27190). The 2×2 is not wrong; it measures a different call path. **Quote 8907
(−23 %) for these sites, never 19894.**

#### Q2 — is `effort: 'low'` honored, or merely accepted?

**Mechanism: forwarded, not swallowed — and the verification channel is now
identified and validated.** Three independent pieces:

1. `effort` is in the same `agent()` opts destructure as `model` and `agentType`.
2. The runtime carries an API-level `effort parameter … not support` error string,
   which only exists if `effort` is put on the wire — a dropped key cannot be
   rejected by the model endpoint.
3. **Transcripts record the effort a request actually ran at.** Each `assistant`
   record in `subagents/workflows/<runId>/agent-*.jsonl` carries a *top-level*
   `effort` field, a sibling of `message.model` — the exact field shape the model
   spike used for `message.model`. This is the primary evidence channel Q2 needs,
   and it did not previously have a name.

Surveying every agent transcript on this machine (6290 files, 156 384 assistant
records) fixes the pre-change control:

| `effort` recorded | Records |
|---|---|
| `"high"` | 55 945 |
| *(absent)* | 100 439 |
| `"low"` | **0** |

So `low` has never been observed. A single post-change run is therefore
conclusive in both directions: an `effort: 'low'` call site that produces
`effort: "low"` records is honored; one that keeps producing `"high"`/absent is
the `model: undefined` inert case and `effort` must be dropped.

**Live result: NEGATIVE on the one route that could be tested.** The
definition-side route the Task-tool docs surface — declaring reasoning effort in
the agent definition rather than threading a key at each call site — *is* testable
without the `Workflow` tool, via `--agents` (the same schema an `.claude/agents/`
frontmatter parses into; the runtime's zod object for it carries
`effort: E.union([E.enum(vO), E.number().int()]).optional()`, so the key is
genuinely part of the contract and not silently dropped at parse time):

| Declared | Recorded on the first assistant record | Conclusion |
|---|---|---|
| *(nothing)* | `effort: "high"` | control |
| `effort: "low"` in the agent definition | `effort: "high"` | **accepted, not honored** |

The agent resolved and ran normally — the key did not throw, was not rejected, and
did not prevent the definition from loading. It simply had no observable effect on
what the request ran at. This is the `model: undefined` inert case in another
costume, and it is precisely the outcome the "accepted ≠ honored" discipline
exists to catch.

That result carried one stated limit: it tests the **definition-side** route, not
`agent(prompt, { effort })` from a Workflow run. The Workflow-path run tested the
other route, and **it reverses the answer.**

##### Q2a — is `effort: 'low'` honored at the *call site*? **YES (measured).**

Read out of the run's `agent-*.jsonl` transcripts — the top-level `effort` field,
per the channel fixed above — and mapped to cases by the journal's dispatch order:

| Case | `opts` | Recorded `effort` | assistant records |
|---|---|---|---|
| A-control | `{}` | `high` | 4 |
| D-agentType-undefined | `{agentType:undefined}` | `high` | 4 |
| **E-effort-low** | **`{effort:'low'}`** | **`low`** | 3 |
| G-effort-undefined | `{effort:undefined}` | `high` | 6 |
| H-effort-invalid | `{effort:'not-an-effort-xyz'}` | `high` | 6 |

**Exactly the one case that asked for `low` recorded `low`.** By the control fixed
above — `"low"` appears **0** times in 156 384 assistant records across 6290
transcripts — a single such record is conclusive, and this is that record.

So the two routes genuinely differ, and both results stand:

| Route | Verdict |
|---|---|
| Agent-definition frontmatter (`effort:` in `.claude/agents/*.md`, via `--agents`) | **accepted, not honored** |
| `agent(prompt, { effort: 'low' })` from a Workflow run | **HONORED** |

Case H adds the failure mode: an invalid effort value does *not* throw, it
silently runs at `high`. So a typo'd `effort` degrades to the status quo — the
opposite of `agentType`, where a typo takes the lane down.

**The guard stays, but its rationale changes.** `scripts/verify-workflow-review.sh`
§2b still forbids `effort:` anywhere under `.claude/workflows/` except the spike —
now **not** because the option is inert (it demonstrably is not), but because
threading it is outside this phase's scope: the phase body's step 4 says "do not
thread `effort:` anywhere", and it says so on the strength of the definition-side
negative that Q2a has just overturned. Flipping a gated invariant on the back of a
result the plan did not anticipate is a scope decision, not an implementation
detail. It is carried by
`finish-agent-type-effort-spike-and-thread-mechanical-sites`, whose scope item 5
("only if Q2 is positive: thread `effort: 'low'` … and drop the `effort:` half of
§2b in the same commit") is now **live and unblocked** — and, unlike the
`agentType` half, is not blocked on anything else.

One honest limit on the positive: n=1 per cell, and this run measured only that
the request *ran* at `low`. It did not measure whether `low` effort degrades
mechanical transcription fidelity, which is the actual risk of threading it and
which the follow-up task must establish before it ships.

#### Q3 — does `CLAUDE.md` load into a custom `agentType` subagent?

**YES — it loads, in full, and there is no way to stop it per agent type.**

*Why self-report and transcript-reading both fail.* `agent-*.jsonl` records
`user`, `assistant` and `attachment` entries only; its two attachments are
`deferred_tools_delta` and `skill_listing`, never a memory payload. The system
prompt — where `CLAUDE.md` is injected — is not recorded anywhere. And asking the
agent to quote a sentinel is not evidence; it is the "the key was accepted"
fallacy wearing a different hat. The sound instrument is the one phase 4 built:
`firstRequestTokens`, which prices the system prompt through
`input + cache_creation + cache_read`. [The 2×2](#the-2x2) applies it with
`CLAUDE.md` as a directly manipulated variable.

*The measurement.* Moving this repo's `CLAUDE.md` aside and restoring it changes
the floor by:

| Agent | with `CLAUDE.md` | without | measured cost |
|---|---:|---:|---:|
| default | 47084 | 27775 | **19309** |
| `rdm-mechanical` (custom `agentType`) | 27190 | 7870 | **19320** |

**The custom-`agentType` agent pays the same 19.3 k tokens the default agent
does** — the two costs differ by 11 tokens (0.06 %). A trimmed system prompt and a
two-tool allowlist do not displace one byte of project memory.

*Against the recorded estimate.* `docs/token-baseline.json`
`agentContextFloor.attribution.claudeMdProject.estimatedTokens` records **12052**,
derived as `chars / 4` from a measured 48207 chars, with the baseline's own caveat
that no local tokenizer was available.

| | tokens | chars/token |
|---|---:|---:|
| recorded estimate (`chars / 4`) | 12052 | 4.00 |
| **measured (this 2×2)** | **19320** | **2.49** |
| error | **+7268 (+60.3 %)** | |

The `chars/4` heuristic **understates this file by 60 %**, which is the expected
direction for its content — dense Markdown tables, fenced shell blocks, slugs and
paths tokenize far below prose. The 15312-token `claudeMdSubtotal` (project +
user-global) is understated the same way: at the measured 2.49 ratio its 61247
chars price at roughly **24.6 k tokens**, and the residual
`remainderToolSchemasSystemPrompt` = 23526 is correspondingly overstated, since it
is defined as the floor minus that subtotal.

*Share of the floor.* `CLAUDE.md` is **41.0 %** of the default agent's 47084-token
floor and **71.1 %** of the trimmed agent's 27190 — i.e. trimming the agent makes
project memory the overwhelmingly dominant remaining term.

*Is it avoidable?* **No, not per agent type.** The runtime's agent-definition
frontmatter schema accepts `description`, `tools`, `disallowedTools`, `prompt`,
`model`, `effort`, `permissionMode`, `mcpServers`, `hooks`, `maxTurns`, `color`,
`background`, `memory`, `isolation`, `observer`, `observerMessage`,
`observeSubagents` — and **none of them suppresses `CLAUDE.md`**. The one that
looks like it might, `memory`, is
`E.enum(["user","project","local"]).optional().describe("Scope for auto-loading
agent memory files…")` — it governs `~/.claude/agent-memory/` and *adds* context.
The only suppression switches in the runtime are process-global and unusable here:
`CLAUDE_CODE_DISABLE_CLAUDE_MDS`, and `--bare` / `CLAUDE_CODE_SIMPLE=1` ("skip …
CLAUDE.md auto-discovery"), which also strips hooks, LSP and auth.

Per the phase body — "if it is unavoidable, record that as a finding and stop
pursuing it — do not restructure `CLAUDE.md` speculatively" — **that is where this
stops.** No `CLAUDE.md` restructuring was attempted or is proposed here. Note only
that the 19.3 k figure reprices the option: a hypothetical 30 % reduction in
`CLAUDE.md` would be worth ~5.8 k tokens per agent, not the ~3.6 k the recorded
estimate implied.

*Corroboration from the existing corpus.* Independently of the 2×2, grouping every
workflow-agent record by project slug and agent class reproduces the effect across
two repos whose `CLAUDE.md` files differ by 31144 chars (rdm 48207 vs.
bowling-app 17063; both share the same user-global copy, which cancels):

| agent class | rdm median | bowling-app median | Δ |
|---|---:|---:|---:|
| `estimate` | 30050 (n=79) | 21710 (n=15) | 8340 |
| `find` | 40032 (n=622) | 28764 (n=7) | 11268 |
| `model` | 37003 (n=13) | 24530 (n=3) | 12473 |
| `refute` | 39444 (n=954) | 23792 (n=209) | 15652 |

Median Δ **11870.5** against a `chars/4` prediction of 7786 — the same
"understated by roughly half again" signature, from a completely different
dataset. This is corroborating only: the two repos differ in skills, hooks and
prompt sizes as well, so it cannot isolate `CLAUDE.md` the way the 2×2 does.

#### Distribution: the phase's stated assumption is WRONG, and it inverts the risk

§ Distribution of the phase plan assumed that "per the runtime precedent, an
unresolvable reference likely degrades silently rather than failing loudly",
reasoning by analogy from the model spike's unknown-id → `null` behaviour.

**The runtime does the opposite for `agentType` — and this is now OBSERVED, not
read off a string table.** Case C of the [Workflow-path run](#the-workflow-run)
passed a deliberately unknown id and raised
`agent({agentType}): agent type '…' not found. Available agents: …`, and the
workflow script distinguishes a throw from a null return explicitly, so the shape
is unambiguous. (Cases B and F raised the same error, but for the setup reason in
§ Q1a; C alone carries this conclusion, and it is sufficient — it is the case
designed to test exactly this.) `agentType` and `model` are handled by different
code paths and only `model` has the silent-null hazard.

This remains a **distribution-scoped** hazard, as originally framed. A downstream
repo receives no `.claude/agents/` definitions at all, so an `agentType` in a
shipped template resolves against a registry that provably cannot contain it. That
is not the situation for the four local-only workflows, whose definition sits in
the same repo at the path the runtime searches.

The consequence is material. `rdm-core/src/agent_config.rs` exposes exactly
`generate_skills` and `generate_workflows`; there is **no** emission surface for
`.claude/agents/`, and adding one is out of scope for this phase by decision. Had
`agentType: 'mechanical'` been threaded into `dispatch-phase.js`
and `review-refute-fix.js` and re-synced into
`rdm-core/src/templates/workflows/`, every downstream repo running
`rdm agent-config claude --skills --out <dir>` would receive workflows that
**hard-fail on first dispatch** — not a "known-degraded surface", a broken lane.
`scripts/verify-agent-config-distribution.sh`'s semantic check greps only for
literal `.claude/workflows/<name>.js` references and would not catch it.

This is a blocking reason not to thread the three distributed files until the
follow-up task `ship-mechanical-agent-type-downstream` lands an emission
surface plus a reference-resolution gate. **This phase does not claim
distribution self-consistency, and it does not introduce a distributed dangling
reference either — it declines to create one.**

#### Disposition

The phase is feasibility-gated for both options, and a recorded result is a
legitimate completion. Landed: the agent definition, the spike, **the spike's live
run**, these findings and their measurements, the `effort:` guard, the
distributed-`agentType` guard, and a fixed pre-change comparison point in
`docs/token-baseline.json` (`mechanicalContextTrim`).

**Answers, in one place.** The Source column matters: two answers come from the
`claude -p` 2×2 and two from the Workflow dispatch, and where they disagree the
Workflow-path answer governs, because that is the call path the call sites use.

| Question | Answer | Source |
|---|---|---|
| Q1 — registry reachable, definition resolves? | **YES** — worth **19894–19905 tokens (≈42 %)** per agent | `claude -p` 2×2 |
| **Q1a — resolves from *inside a Workflow run*?** | **YES** — case B returned the trimmed `["Bash","StructuredOutput"]`; worth a measured **8907 tokens (−23 %)**, not the 19894 the 2×2 predicted | **Workflow run, observed** |
| Q1b — how does an `agentType` absent from the registry fail? | **RAISES** (was inferred from a string table; now observed via case C) | **Workflow run, observed** |
| Q2 — `effort: 'low'` declared in a *definition*? | **NOT honored** — ran at `high` | `claude -p` |
| **Q2a — `effort: 'low'` passed to `agent()`?** | **HONORED** — first `effort:"low"` record in a 156 384-record corpus | **Workflow run, observed** |
| Q2b — an *invalid* `effort` value? | **Accepted, silently degrades to `high`** — no throw | **Workflow run, observed** |
| Q3 — does `CLAUDE.md` load into a custom `agentType` agent? | **YES**, unavoidably, at **19320 measured tokens** (recorded estimate 12052 understates by 60 %) | `claude -p` 2×2 |

**What was threaded, and what was not.**

**`agentType: 'rdm-mechanical'` IS threaded** at every mechanical call site of the
four local-only workflows — 19 records in all (15 call sites in the `.js`
consumers, 4 of which are duplicated into `lib/plan-review.mjs` as the byte-copied
source of `plan-review.js`'s `plan-review-driver` block):

| File | Sites | Route |
|---|---|---|
| `document.js` | `model:mechanical`, `fetch:roadmap-meta`, `gather:<stem>`, `write:draft` | unprojected driver |
| `backlog.js` | `model:mechanical`, `fetch:report` | unprojected driver |
| `estimate.js` | `model:mechanical`, `estimate:list`, `estimate:write:<stem>`, `estimate:tier:<stem>` | unprojected driver, below `estimate-core:end` — not distributed to any downstream workflow (`autopilot.js` formerly carried its own duplicate `estimate-core` copy before its retirement to prose) |
| `plan-review.js` + `lib/plan-review.mjs` | `fetch:roadmap`, `fetch:<kind>`, `fetch:wontfix`, `gate:clear-tag:<kind>:<ident>` | byte-copied block — both halves edited, gated by §5b-drift |
| `plan-review.js` | `model:mechanical` | unprojected driver, below `plan-review-driver:end` |

It is written as a plain literal at each site, never a module-level constant,
because the source text is byte-copied across files with different scopes.
`scripts/verify-workflow-review.sh` §2c asserts this **bidirectionally** — every
mechanical site carries it, no judgment site does — with planted-mutation
self-tests in both directions and a completeness sweep that fails if a site is
added or removed without updating the asserted list, or if any `agentType` other
than `rdm-mechanical` appears.

**Q1a has since confirmed this threading** — see
[the Workflow-path trim](#workflow-path-trim). Case B resolves with the trimmed
tool list, and a live `backlog` dispatch shows both its threaded sites dropping
~19–20 %. The measured saving is **8907 tokens/agent (−23 %)**, not the 19894 the
`claude -p` 2×2 predicted; every figure quoted for these call sites is the
measured one.

**Not threaded, and each for its own reason:**

1. **The three distributed workflows** — blocked outright by the missing emission
   surface (§ Distribution). A downstream tree receives no `.claude/agents/` at
   all, so the reference could never resolve there. §2b(ii) gates this.
2. **Every judgment site** — finders, refuters, planners, implementers,
   `synthesize:draft`, `analyze:*`, `estimate:rate:*` and plan-review's `act:*`.
   `rdm-mechanical` is a transcribe-only agent with a two-tool allowlist; giving
   it work that requires reasoning would break it. §2c(ii) gates this.
3. **`effort:` anywhere** — even though Q2a is positive. The phase body forbids it,
   and lifting a gated invariant on the back of a result the plan did not
   anticipate is a scope decision, carried by
   `finish-agent-type-effort-spike-and-thread-mechanical-sites`. Its real
   remaining risk is fidelity: the run showed the request *ran* at low effort, not
   that low effort preserves mechanical transcription accuracy — and a degraded
   schema return is a silent corruption, unlike a raise.

**A methodological note, since this phase's whole discipline is about evidence.**
The Q1a error was not a subtle one: an experiment was run in an environment that
could not produce a positive result, and its negative was written up across five
surfaces as a measured finding before anyone checked the documented behaviour of
the thing being tested. The "accepted ≠ honored" rule this section exists to
enforce has a mirror image — *failed ≠ impossible* — and the same standard of
proof applies to a negative as to a positive. The guard
   therefore stays, with a corrected rationale, and the work is handed over.

**Nothing about the `agentType` prize is in doubt, only its delivery** — the saving
is measured (19894 tokens/agent), it replicates, it is additive with `CLAUDE.md`,
and it is 42 % of a mechanical agent's floor. What remains is carried by two tasks:

- `finish-agent-type-effort-spike-and-thread-mechanical-sites` — its `effort` half
  (scope item 5) is **unblocked and actionable**: Q2a is positive, so thread
  `effort: 'low'` at the mechanical sites, drop §2b's `effort:` half in the same
  commit, and first establish that low effort does not degrade transcription
  fidelity. Its `agentType` half (scope items 3 and 4) needs **one valid re-run of
  the spike first** — from a session whose project root holds
  `.claude/agents/rdm-mechanical.md` at session start, which any ordinary session
  in this repo satisfies once this branch lands. Scope item 3 (does the
  `{ schema }` structured-return path survive the restricted `tools:` list; is
  `agentType` honored through `parallel()`) is untested for the same reason: in
  the invalid run, nothing ever ran under the definition.
- `ship-mechanical-agent-type-downstream` — the `.claude/agents/` emission surface
  and its reference-resolution gate, which unblocks the three distributed
  workflows and lifts §2b(ii). Its "hard failure on first dispatch" premise is now
  observed rather than inferred.

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
| `unrefuted`     | `true` (post-pipeline only)              | set by `buildReviewPipeline`, never by a finder    |
| `unrefutedReason` | `'non-gating'` \| `'budget'` (post-pipeline only) | present iff `unrefuted` is; WHY it went ungraded |
| `refuterError`  | `true` (post-pipeline only)              | a refuter was dispatched and CRASHED; never combined with `unrefuted` |

**Four states, four markers.** Every finding a consumer receives is in exactly
one of these, and they are distinguishable by markers alone — this is the single
documented contract:

| state | markers |
| --- | --- |
| graded and survived | no `unrefuted`, no `refuterError` |
| skipped as non-gating | `unrefuted: true`, `unrefutedReason: 'non-gating'` |
| passed over for budget | `unrefuted: true`, `unrefutedReason: 'budget'` |
| grading crashed | `refuterError: true`, and never `unrefuted` |

`unrefuted` is added by the pipeline, not returned by a finder: a finding whose
severity is in `NON_GATING_SEVERITIES` (`['suggestion']`) gets **no refuter at
all** — its verdict could not change the outcome at any tier, since
`hasBlocking`'s blocker set is `['blocking']` (widened to
`['blocking','concern']` at the `large` tier) and the AC table never reads
finding severity — so it passes straight through carrying `unrefuted: true`.
The confidence floor still applies to it (`survives(finding, null)`), and the
rule is fail-safe: a finding whose severity is missing or unrecognized is
refuted like a gating one. A refuter that *crashes* also yields a null verdict,
but such a finding is **not** marked `unrefuted` — the marker means
"deliberately never graded", not "grading failed"; it carries `refuterError:
true` instead. The SECOND reason a finding can be deliberately ungraded is the
per-unit **refutation budget** (see § Refutation budget below): an over-budget
finding takes the same pass-through path with `unrefutedReason: 'budget'`, and
is likewise still subject to the confidence floor. Consumers must treat an
`unrefuted` finding as an observation, never a confirmed defect (see
`UNREFUTED_DISPOSITION`, single-sourced in the stamped block and appended to
both act prompts). Measured evidence for the set's membership —
per-severity refutation rates and the token cost of the skipped refuters — is in
`docs/token-baseline.json` § `nonGatingRefutationSkip`.

### `AC_ENTRY` / `AC_REVIEW_SCHEMA`

The `ac` dimension in `code` mode is the **one** dimension that does not return
the bare `FINDINGS_SCHEMA` shape. Its finder is forced to satisfy
`AC_REVIEW_SCHEMA` instead: a required per-criterion `ac` table (`AC_ENTRY`
rows) plus an OPTIONAL `findings` array (same shape as `FINDINGS_SCHEMA`'s) for
narrative notes that don't reduce to a single criterion's status.

`AC_ENTRY`:

| field       | type                              | notes                                   |
| ----------- | --------------------------------- | ---------------------------------------- |
| `criterion` | string (required)                 | the acceptance criterion being rated     |
| `status`    | `PASS` \| `FAIL` \| `PARTIAL` (required) | the finder's rating for this criterion |
| `evidence`  | string (required)                 | file:line, test name, or other citation  |

This table is a **structured side-channel**, not a finding: `classifyOutcome`
(see below) checks it directly, independent of finding severity and
refutation — a hallucinated `AC_ENTRY.status: 'FAIL'` therefore bypasses
refutation entirely and can force a spurious `rework` with no counter-check,
which is the deliberate trade-off for a guarantee that can no longer be
silently defeated by a refuter or the confidence floor. The AC table and any
`ac`-dimension `findings` entry about the same criterion are two independent
channels, never deduplicated against each other. `plan` mode has no `ac`
dimension, so it never populates this table.

### `VERDICT`

A refuter agent's grade of a single `FINDING`. A **fresh** refuter grades each
finding — the finder never grades its own work.

| field        | type                     | notes                                                  |
| ------------ | ------------------------ | ------------------------------------------------------ |
| `refuted`    | boolean (required)       | `true` ⇒ the finding does not hold up ⇒ dropped        |
| `confidence` | integer 0–100 (required) | the refuter's confidence in **its verdict** (advisory) |
| `rationale`  | string                   | why the finding was or was not refuted                 |

`VERDICT` is the **single-finding** contract, and it is the contract the shipped
pipeline uses. A batched sibling — one refuter per dimension over that review
unit's gating findings, with verdicts attributed by an explicit per-finding id —
was measured and **not shipped**: grouped by the key a real dispatch actually
forms (`runId | unitIdent | mode | dim.key`, because `buildReviewPipeline` runs
once per review unit), the adjudicated corpus yields only 1 qualifying batch of 3
findings against a pre-registered floor of 6 batches / 18 findings, so the A/B
returned `no-measurement` rather than a decision. Method, figures, decision rule
and limitations: [`docs/refuter-batching.md`](refuter-batching.md). The
experiment's batched prompt, verdict parsing and anchoring scorer live in
`scripts/lib/refuter-agreement.mjs`, deliberately **not** in
`.claude/workflows/lib/review.mjs` — `scripts/verify-refuter-agreement.sh`
asserts the pipeline carries no batched symbols while the recorded decision is
anything other than `ship-batched`.

### `OUTCOME` (review pipeline)

The value `buildReviewPipeline(mode)(context)` resolves to
`{ survivors, acTable, budget }`: `survivors` is a **ranked** array of the
surviving `FINDING`s, `budget` is the refutation-budget accounting (below), and
`acTable` is the captured `AC_ENTRY[]` from the `ac`
dimension's finder in `code` mode (`null` in `plan` mode, and `null` whenever
the `ac` dimension didn't run or its finder failed to resolve a table — see
`AC_ENTRY` / `AC_REVIEW_SCHEMA` above). The dispatch-phase keystone (below)
consumes both fields at each of its two review gates and folds them into its
own, differently-shaped `OUTCOME`; `classifyOutcome` (see "Verdict and status
mapping" below) checks `acTable` directly via `acTableHasGap`, independent of
`survivors`' severity/refutation, and can only ever push the outcome to
`rework` — never `escalated`.

The third field, **`budget`**, records what the per-unit refutation budget did:
`{ max, produced, gating, graded, passedThroughNonGating, passedThroughBudget,
refuterErrors, hit }`. It describes the **pipeline**, not any consumer-side
post-filtering — plan-review's `stripNonPhaseUnitOfWork` / `suppressWontFixed`
run afterwards and may drop a survivor that consumed budget. Consumers project
it onto their own shape with the two shared helpers in the same stamped block:
`buildReviewBudget(budgetRounds, planBudget)` yields the `reviewBudget` field
(last round's counts, `rounds`, `planRounds`, `everHit`, the last `hit` object,
and the plan gate's own budget), and `budgetSummaryClause(reviewBudget)` yields
the visible ` [review budget hit: N produced, M graded, K ungraded]` marker —
empty when the bound was never hit, so an unbounded run's summary is
byte-unchanged.

**Both** of `buildReviewBudget`'s parameters take the gate's FULL per-round
array. Passing only a last-round object silently drops an early round that hit
its bound and was then resolved by a later revision/rework — precisely what
`everHit` promises to keep visible — so `dispatch-phase` threads
`planGate.budgetRounds`, not `planGate.budget`. (`planBudget` still accepts a
single object, for a caller that predates the plan gate returning an array.) The
two arrays are merged in **temporal** order, plan rounds first, because the plan
gate runs to completion before the code gate starts; consequently, when both
gates hit, `hit` — and therefore the summary clause — reports the later code
round, not the earlier plan one.

The standalone `review-refute-fix.js` consumer has three invocation shapes: (a)
`mode: 'plan'`, and (b) `mode: 'code'` with no `roadmap`+`phase` or `task`
identifier, both keep returning the legacy survivors-only `{ mode, survivors }`
shape (plus an additive `budget` field) for backward compatibility with ad
hoc/document-less reviews;
(c) `mode: 'code'` with `{ roadmap, phase }` or `{ task }` runs the SAME
`buildReviewPipeline('code')` pass, then additionally derives real diff signals
from the item's worktree (mirroring dispatch-phase's code gate — see below) and
composes the survivors through `classifyOutcome` plus `statusFor` /
`writesCompletion` / `summarizeFindings` / `gateFor` into the dispatch-shaped
`OUTCOME` contract: `{ roadmap, phase, outcome, status, writesCompletion,
summary, reason, reviewBudget, findings }` (or the `{ task, ... }` shape). An optional
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
`agent()` errors, its finding is kept as **un-refuted** (`verdict = null`,
marked `refuterError: true`) and survives on the confidence floor alone, rather
than being silently dropped as if refuted — the pipeline logs how many findings
were kept this way. A finder crash instead drops only its own dimension to
`null` (the runtime's `parallel` sends a thrown thunk to null), so the other
dimensions still contribute and the review degrades rather than failing; a
crashed dimension contributes no candidates, so it never inflates
`budget.produced` with coverage it did not provide.

**Ranking (`rankFindings`).** A total order, so `OUTCOME` is deterministic across
runs (the runtime forbids `Date.now()`/`Math.random()`): by `severity`
(`blocking` < `concern` < `suggestion`), then `confidence` descending, then `id`
ascending as a stable tiebreaker.

## `buildReviewPipeline(mode, deps?)`

Returns an async `runReview(context)` that composes
`parallel(finders)` → **barrier** → budget cut → `parallel(refuters)`:

0. **Select** — the deterministic pre-step `selectDimensions(mode, signals)`
   decides which dimensions actually run (see below).
1. **Find** — one finder `agent()` per selected dimension, in parallel, as a
   `parallel()` fan-out of per-dimension thunks. In `code` mode, the `ac`
   dimension's finder is forced to satisfy `AC_REVIEW_SCHEMA` instead of
   `FINDINGS_SCHEMA`, and the first `ac` array it resolves is captured into the
   run's `acTable`.
2. **Barrier + budget cut** — every finder settles, then all dimensions' findings
   are flattened into ONE unit-wide candidate list, partitioned by
   `needsRefutation`, and the gating half is ranked by `rankBudgetCandidates` and
   cut at the refutation budget (see below). The cut is taken BEFORE any refuter
   is dispatched, so it can never depend on agent-completion order.
3. **Refute** — a **fresh** refuter `agent()` per finding in the top N, in
   parallel. Non-gating findings and the over-budget overflow take the
   un-refuted pass-through instead.
4. **Filter** — drop findings that were refuted or fell below `CONFIDENCE_FLOOR`.
5. **Rank** — resolve `{ survivors: rankFindings(survivors), acTable, budget }`.

The barrier is why stage 1 is `parallel()` rather than a single-stage
`pipeline()`: the budget must rank a unit's WHOLE candidate list across
dimensions, which the previous no-barrier `pipeline(dims, find, refute)`
composition — where each dimension's find→refute chain ran independently —
structurally cannot do. `parallel()`'s thrown-thunk → null degradation is
identical to `pipeline()`'s thrown-stage → null, so the per-dimension crash
behavior is unchanged, and it makes no assumption about a minimum `pipeline()`
stage count.

### Refutation budget

At most `DEFAULT_MAX_REFUTATIONS` (**5**) GATING findings per review unit are
handed to a refuter. Everything past the cut takes the EXISTING un-refuted
pass-through carrying `unrefuted: true` and `unrefutedReason: 'budget'` — no
second mechanism, and the confidence floor still applies (the budget skips
**grading**, never **filtering**). Non-gating `suggestion` findings never consume
budget, since they were already never refuted.

| | |
| --- | --- |
| arg name | `maxRefutations` (on `dispatch-phase`, `plan-review`, and `review-refute-fix` args; reaches `runReview` as `context.maxRefutations`) |
| default | `DEFAULT_MAX_REFUTATIONS` = 5 |
| `0` | LEGAL and meaningful — grade nothing, pass every gating finding through as `unrefutedReason: 'budget'`. Never conflated with "unset" by a falsy check. |
| uncapped | no sentinel exists; express an effectively-uncapped run as a large N |
| validation | `resolveRefutationBudget(value)`, mirroring `parseBudget`'s contract — a number or integer-ONLY string; `'5abc'` is rejected, not coerced. `dispatch-phase`/`plan-review` validate at PARSE time, before any `agent()` call. |
| ranking | `rankBudgetCandidates`: severity → confidence descending → id → source order. The source-order tiebreak is what makes the cut total when two dimensions emit the same finding id. |

**Why 5.** Measured, not guessed: `docs/token-baseline.json` §
`determiningFindingRank` replayed this pipeline's own ranking over the recorded
corpus and located the outcome-determining finding within the top 5 for 100 % of
determining units at the default tier and 98.2 % at the `large` tier. The full
derivation, the rejection of N = 3, and the monotonicity argument that makes the
single residual safe live in `docs/token-baseline.md` § "Phase 4: the chosen
refutation budget" and in the constant's own comment block in
`.claude/workflows/lib/review.mjs` (canonical) — they are not restated here.

**Why it is safe.** `survives(finding, verdict)` reads the FINDING's confidence,
never the verdict's, so the only effect a verdict can have is
`refuted === true ⇒ drop`. Skipping refutation is therefore monotone-increasing
in the survivor set: the budgeted survivors are always a SUPERSET of the
unbudgeted ones, and since `hasBlocking` is an existential over that set, the
budget can only ever move `reviewed → rework`, never `rework → reviewed`. The AC
table is never budgeted, so `classifyOutcome` step 2 is bit-identical under every
N including 0. `scripts/verify-workflow-review.sh` § 9 encodes this as an
exhaustive subset property test, not only as prose.

`context.target` (and any other fields) is threaded into every finder and refuter
prompt, so the review material reaches the agents. `deps` (`{ agent, pipeline,
parallel, log }`) is omitted in the Workflow runtime (the ambient globals are
used) and injected by the verify harness to drive the pipeline with fakes.

**Every consumer of `runReview`/`d.review(...)` must destructure
`{ survivors, acTable, budget }`** rather than treat the resolved value as a bare
array. In `lib/dispatch-phase.mjs` this means **both** `runCodeGate` (which
tracks a per-round `acRounds` array alongside `rounds` and checks
`acTableHasGap` in its rework-loop continuation) and `runPlanGate` (which
discards `acTable` — always `null` in `plan` mode — and uses `survivors` as
its `findings`) needed updating; `lib/plan-review.mjs`'s `reviewUnit` and its
`--implementation-plan` branch, and `review-refute-fix.js`'s legacy and
standalone driver paths, do the same.

### Dimensions and `when` triggers

Each dimension is either **always-on** (no `when` key) or **triggered** (a
`when(signals) => boolean` predicate evaluated over both the change's shape and
the target's type).

| mode | always-on | triggered |
| --- | --- | --- |
| `code` | `ac`, `correctness` | `tests`, `architecture`, `api-docs`, `changelog`, `security` |
| `plan` | `coherence`, `architectural-fit`, `restraint` | `unit-of-work` (phases only) |

`unit-of-work` triggers on `signals.targetType === 'phase'`, which is why target
type is a first-class signal rather than diff shape alone.

#### Why the always-on sets are not collapsed into one finder per mode

Each always-on dimension is its own agent, paying its own agent context floor.
Collapsing them is an obvious token target, and both halves of the answer are
recorded rather than assumed.

**`plan` — measured, and rejected.** The three always-on plan dimensions all
resolve the same `FINDINGS_SCHEMA`, so merging them into one agent holding three
lenses needs no schema change and was a live candidate. It was A/B'd against the
current three-finder shape over real mined plan documents; the collapsed finder
lost a material share of findings in **every** lens. The pre-registered decision
rule, the run, the per-lens figures and the `no-ship` DECISION are in
[`finder-collapse.md`](finder-collapse.md), with the machine-checkable figures in
[`token-baseline.json`](token-baseline.json) § `planFinderCollapse`. A
decision/pipeline XOR in `scripts/verify-finder-collapse.sh` keeps a half-landed
merge from ever coexisting with that figure.

**`code` — not a candidate at all, and this is the canonical statement of why.**
`ac` and `correctness` are not symmetric with plan mode's lenses:

- `ac` is the ONE dimension that resolves `AC_REVIEW_SCHEMA` rather than
  `FINDINGS_SCHEMA`. Merging it would force a union schema on the merged agent.
- Its per-criterion `ac` table is the structured side-channel `classifyOutcome`
  step 2 consumes **directly**, via `acTableHasGap`. That channel never reads a
  finding's severity, is never refuted, and never consumes refutation budget —
  three properties deliberately chosen so the acceptance-criteria guarantee
  cannot be silently defeated by a refuter or by the 70-point confidence floor.
  Folding `ac` into a shared findings stream would route the acceptance-criteria
  contract through exactly the path it was kept out of.

A short form of this rationale lives in the `//|code|` spec prose in
`.claude/workflows/lib/review.mjs`, so it renders into the shipped code-review
skill templates and travels with the lane rather than staying tribal knowledge.

`unit-of-work` likewise stays a separate triggered dimension in either scenario:
it is scoped to phase units CONSUMER-SIDE by `stripNonPhaseUnitOfWork` in
`plan-review.js`, which filters on `f.concern === 'unit-of-work'`, and folding a
conditionally-scoped lens into an unconditional agent would defeat that scoping.

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

`classifyOutcome` also accepts `input.acTable` — the `AC_ENTRY[]` belonging to
the LAST completed code round (see `AC_ENTRY` / `AC_REVIEW_SCHEMA` above). A
surviving `FAIL`/`PARTIAL` criterion (`acTableHasGap(acTable)`) mechanically
forces `rework`, checked as its own step BEFORE the code-findings check and
AFTER the plan-gate check — so it can only ever yield `rework`, never
`escalated`, and it is independent of finding severity or refutation
entirely: an AC-table `FAIL` forces `rework` even when zero findings survived.

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

`gen-skill-review.sh` also carries an orthogonal **`--target shipped|local`**
axis (default `shipped`), independent of `--mode`: `shipped` renders the
`rdm-core/src/templates/skill-{review,plan-review}-{cli,mcp}.md` files baked
into released binaries; `local` renders this repo's own dogfood skill copies,
`.claude/skills/{rdm-review,rdm-plan-review}/SKILL.md` — nothing else
re-stamps them, so without this target they drift silently behind the
canonical source (as the plan-mode `restraint`/severity-calibration gap this
axis was added to close in fact did). A third, innermost marker pair nested
inside `review-spec` — `find-refute-verdict` and its sibling
`find-refute-verdict:local-code-override` — lets `--target local --mode code`
swap in `rdm-review`'s workflow-delegation recap in place of the default
Find/Refute/Verdict-point-2 prose; every other `(target, mode)` pair renders
the default span unchanged and never sees the override block. A `{rdm_bin}`
placeholder on example commands resolves to `rdm` for `shipped` and
`./target/debug/rdm` for `local` (this repo's own hard dev-build rule) from
the one substitution point in the generator. Both local targets are
`--check`-gated in `scripts/verify-workflow-review.sh` § 1g alongside the
shipped ones in § 1c/1d.

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
| `models` | object (required) | resolved model ids: `plan`, `implement`, `review_find`, `review_verify`, `mechanical` — an incomplete map short-circuits to `fetchError: true` before any other agent runs |

### `TASK_META`

What the Stage-0 mechanical fetch agent returns from `rdm task show … --format
json` in task mode. A task does not have a containing roadmap or phase number.

| field    | type              | notes                                             |
| -------- | ----------------- | ------------------------------------------------- |
| `task`   | string (required) | task slug                                         |
| `body`   | string (required) | the full task markdown; empty ⇒ fetch failure     |
| `models` | object (required) | resolved model ids: `plan`, `implement`, `review_find`, `review_verify`, `mechanical` — an incomplete map short-circuits to `fetchError: true` before any other agent runs |

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
| `reviewBudget` | object \| `null`                     | `buildReviewBudget(...)` — the refutation bound: last round's `max`/`produced`/`graded`/`passedThroughBudget`, plus `rounds`, `planRounds`, `everHit`, the last `hit` object, and the plan gate's own `plan` budget. `null` when no review reported one. |
| `findings`| array of `FINDING`                        | the relevant ranked surviving findings         |

**Task mode** emits this structure keyed by `task` instead of `roadmap`/`phase`:

| field     | type                                      | notes                                          |
| --------- | ----------------------------------------- | ---------------------------------------------- |
| `task`    | string                                    | echoed from the dispatch args                  |
| `outcome` | `reviewed` \| `rework` \| `escalated`     | the task verdict (same decision tree)          |
| `status`  | string                                    | `statusFor(outcome, kind)` — the rdm status to persist |
| `writesCompletion` | boolean                          | `writesCompletion(outcome)` — is this branch owed its land-time trailer? |
| `summary` | string                                    | deterministic one-liner from outcome + top finding |
| `reason`  | string                                    | gate-tagged park note (`[plan]`/`[code]`); empty on `reviewed` |
| `reviewBudget` | object \| `null`                     | `buildReviewBudget(...)` — the refutation bound: last round's `max`/`produced`/`graded`/`passedThroughBudget`, plus `rounds`, `planRounds`, `everHit`, the last `hit` object, and the plan gate's own `plan` budget. `null` when no review reported one. |
| `findings`| array of `FINDING`                        | the relevant ranked surviving findings         |

The `rdm-do --auto --task` wiring into this task-mode contract is regression-tested by `scripts/verify-workflow-do-auto-task.sh` (SKILL.md static invariants, the OUTCOME→status contract against the real binary, and a prose-only self-test of the distributed template).

**`status` / `writesCompletion` carry the gate policy as data.** They are derived
from the canonical `statusFor` / `writesCompletion` in `lib/review.mjs`, so
consumers (autopilot's advance/park, `rdm-do --auto`, `rdm-land`) read the policy
off the OUTCOME instead of restating the mapping. `writesCompletion` is a
**boolean, never the trailer literal** — the stamped block may not contain that
string (`verify-workflow-dispatch.sh` AC-1). `rdm-land` reads
`writesCompletion: true` and synthesizes the real trailer at land time via
`rdm hook done-line`, amending it **before** the rebase, so an autonomously
produced branch never needs a manual rebase to gain it.

**`reviewBudget` makes a bounded review legible.** When ANY round hit the
refutation budget, `budgetSummaryClause` appends a short
` [review budget hit: N produced, M graded, K ungraded]` marker to `summary` in
all three outcome branches — and because `outcomePolicy` derives `reason` from
`summary`, a parked or escalated budget-hit unit surfaces it in the
`rdm review blocked` queue for free. `autopilot` additionally suffixes a
` [budget]` tag (`budgetHitTag`) onto that phase's stem in `buildSummary`'s
`phases completed (...)` line, so a *reviewed* budget-hit phase is visible too.
A run that stayed under budget keeps a byte-unchanged summary.

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

**Rework notes carry the AC table too.** `runCodeGate`'s rework call is
`d.implement({ findings, acTable })` — never a bare findings array. Because
the AC table is a structured side-channel decoupled from `findings` (a
`FAIL`/`PARTIAL` criterion need not also appear as a finding), an AC-only-gap
rework round has an *empty* `findings` array; without also passing `acTable`
the implementer would receive no signal at all about what to fix and the
rework budget would very likely burn out reproducing the same gap.
`dispatch-phase.js`'s `buildImplementPrompt` renders the two channels
separately — "ranked issues" from `findings` and "UNMET criteria" from the
`FAIL`/`PARTIAL` entries of `acTable` — and explicitly notes they are not a
duplicate report of the same thing.

**The AC-only-gap summary fix applies everywhere `acTable` feeds
`classifyOutcome`.** `buildOutcome`/`buildTaskOutcome` name the real cause
(`'code rework unresolved: unmet acceptance criteria in AC table'`) instead of
the misleading `summarizeFindings([])` → `'no surviving findings'` when an
AC-only gap forces `rework` with an empty findings array; `review-refute-fix.js`'s
standalone `{ roadmap, phase }`/`{ task }` code-review path applies the
identical branch to its own `rework` summary, since it independently threads
`acTable` into its own `classifyOutcome` call.

### `CODE_ACT_SCHEMA` and the code-lane Act step

Once `runCodeGate`'s rework loop settles on a **clean** final round (no
blocking finding, no AC-table gap) with **non-empty** surviving findings, the
gate invokes the optional `d.act(findings)` dep exactly once — mirroring the
plan-review skill's small/large Act split (`buildActPrompt`), but for code: a
finding is fixed inline in the worktree (small) or filed as a task via
`rdm task create --tags code-review` (large), never both, and never a
separate landing commit for a small fix (it folds into the eventual land-time
commit). `dispatch-phase.js` wires this dep to an `agent()` call using
`buildCodeActPrompt` and `CODE_ACT_SCHEMA`:

| field                | type                                       | notes                                    |
| -------------------- | ------------------------------------------ | ------------------------------------------ |
| `handled`            | array of `{ id, action, taskSlug?, reason? }` (required) | one entry per finding the Act step was asked to incorporate |
| `handled[].id`       | string (required)                         | matches the `FINDING.id` it disposed of  |
| `handled[].action`   | `fixed-inline` \| `filed-as-task` \| `skipped` (required) | how the finding was incorporated       |
| `handled[].taskSlug` | string                                     | present when `action` is `filed-as-task` |
| `handled[].reason`   | string                                     | why, when `action` is `skipped`          |

`skipped` exists for the `unrefuted` half of a mixed payload (see `FINDING`
above): the disposition rule tells the act step to incorporate the un-refuted
observations that are not major, to **file** the ones it does not incorporate
but that are worth keeping (so a real-but-too-big observation still becomes a
durable task rather than a transient reason string), and to skip only the rest
— with a stated reason. Without this action the act step would have to
misreport such a skip as one of the other two. The prompt only asks for it when a survivor actually carries
`unrefuted: true`; with an all-verified payload `buildCodeActPrompt` is
byte-identical to its pre-pass-through form.

The Act step is a no-op when `d.act` is omitted, and its result (or a thrown
call) never affects the outcome — concern/suggestion findings are non-gating
by the module's own severity contract, so a failed fix-attempt must never
downgrade a `reviewed` outcome. `runCodeGate` reports the raw agent result as
`actResult`; `buildOutcome`/`buildTaskOutcome` thread it through
`annotateHandled`, which stamps each REPORTED finding (only on a `reviewed`
outcome) with a `handled` field from the matching `actResult.handled` entry —
defaulting to `'unhandled'` when a specific finding wasn't addressed, and
leaving findings unannotated (no `handled` key) when `actResult` itself is
absent (Act was never invoked, or it threw). This never runs when the loop
exited still-blocking or AC-table-gapped — large/unresolved defects stay
owned by the rework/status machinery, per "never fix large changes inline".

**The code-review stage is the canonical review.** `dispatch-phase` builds it
from the stamped `buildReviewPipeline('code')` — there is no independent
code-review logic in the driver — and feeds it `deriveSignals` output from the
real branch diff (see `deriveSignals(input)` above for the signals-absent
fail-open contract). `verify-workflow-dispatch.sh` pins both halves: exactly one
`buildReviewPipeline('code')` binding site and one declaration each of
`findPrompt`/`refutePrompt`, plus the `deriveSignals(` / `signals:` /
`diff:signals` wiring.

## autopilot contract

Autopilot is now the prose `rdm-autopilot` skill
([`docs/autonomous-loop.md`](./autonomous-loop.md)); it consumes the same
dispatch-phase OUTCOME contract below (`statusFor`/`writesCompletion`) rather
than defining its own, and its own advance/park Bash steps are what persist the
terminal status per "The core fix" below — dispatch-phase itself never does.

### The core fix: the loop advances off PERSISTED status

`dispatch-phase` persists **no terminal** phase status — it does stamp the
phase (or task) `in-progress` itself, best-effort, right after Stage 0 (metadata
+ model resolution) and before it starts working the item; a `--plan-only` run
skips that stamp, since it never implements — and `rdm next` returns only
`not-started`/`in-progress` phases (it skips `reviewed`/`blocked`/…). So the
driving loop persists the terminal status **itself** (its own advance/park
steps), which is what makes `rdm next` step forward and eventually return
`nothing`:

- a `reviewed` OUTCOME (normal mode) → advance runs
  `rdm phase update <stem> --status reviewed`;
- a rework-exhausted or `escalated` OUTCOME → park runs
  `rdm phase update <stem> --status blocked --reason "[code|plan] …"`.

There is **no** normal-mode in-memory `seen` Set; progress is driven entirely by
the persisted status the selector reads back.

### `dispatch-phase` `planOnly`

`dispatch-phase` accepts an optional `planOnly` arg. When set, once the plan gate
passes it returns early — `{ outcome: 'reviewed', summary: 'plan-only: plan gate
passed', findings: <planFindings> }` — before implementing, so a caller such as
the prose `rdm-autopilot` loop can vet the plan half cheaply. This early return
lives in the driver, outside the copied `dispatch-outcome` block, and adds no
new nested `workflow()` call.

## Testing convention

`scripts/verify-workflow-review.sh` is the hermetic gate. It uses **Node standard
library only** — no `package.json`, no `node_modules`, no third-party packages;
the reference `pipeline`/`parallel` implementations and assertions are written
inline with `node:assert`. It resolves `node` via the `.mise.toml`-pinned
toolchain (bare `node`, else `mise exec node --`) and fails hard if node is truly
absent, matching the sibling harnesses' tool-guard convention.

## Optional caller-supplied args (mechanical-agent hoists)

A Workflow script cannot run a shell command itself, which is why the lane spawns
**mechanical** subagents — agents that run one `rdm`/`git` command and return its
output. That forces a *subagent*; it does not force a *dedicated* one. The parent
**skill shim** is already a running agent with the repo in context, so anything it
runs itself and passes through the `Workflow` tool's `args` costs a tool call
instead of a whole 27k-token context load.

Each workflow therefore accepts a set of **optional** args. The full census, the
classification rule behind them, and the measured delta live in
[`docs/mechanical-agent-inventory.md`](mechanical-agent-inventory.md).

| workflow | optional arg | replaces | shape guard |
|---|---|---|---|
| `dispatch-phase` | `phaseMeta` | `fetch:phase-meta` | **all-or-nothing** — see below |
| `dispatch-phase` | `taskMeta` | `fetch:task-meta` | **all-or-nothing** — see below |
| `dispatch-phase` | `alreadyInProgress` | `stamp:in-progress` | boolean; the caller must already have written the status |
| `estimate` | `mechanicalModel` | `model:mechanical` | non-empty string |
| `estimate` | `phaseList` | `estimate:list` | array |
| `plan-review` | `fetched` | `fetch:roadmap` / `fetch:<kind>` | object with a non-empty `body` **and** a `tags` array of strings (roadmap kind additionally: an array `phases` whose every entry carries a non-empty `stem`, a string `body`, and its own `tags` array) |
| `plan-review` | `wontFixedTexts` | `fetch:wontfix` | array |
| `plan-review` | `mechanicalModel` | `model:mechanical` | non-empty string |
| `backlog` | `mechanicalModel` | `model:mechanical` | non-empty string |
| `backlog` | `report` | `fetch:report` | object carrying all four signal arrays |
| `document` | `mechanicalModel` | `model:mechanical` | non-empty string |
| `document` | `roadmapMeta` | `fetch:roadmap-meta` | object with `found === true` and an array `phases` |
| `review-refute-fix` | `diff` | `diff:signals` | object with an array `changedFiles` |

### The invariant: every hoist is optional

At every one of these sites the original `agent()` call is kept **byte-unchanged**
and reached through an `else` branch. It is never deleted, and no hoisted field is
ever added to a schema's `required` list. A missing key, a wrong type, a `null`, an
empty string, an empty array, or a JSON-string `args` payload that fails to parse
all reject-and-fall-back rather than throwing. A **direct `Workflow` invocation**
therefore behaves exactly as it did before — and, until the five unconverted skill
templates become shims, that fallback is a live production path, not a degenerate
case.

Each site logs which path it took (`hoisted` vs `fetched`), so a direct invocation
is observable in the run transcript.

### `phaseMeta` / `taskMeta` are all-or-nothing

`hoistedMetaComplete(meta, isTask)` (in `.claude/workflows/lib/dispatch-phase.mjs`)
accepts a payload only when the `body` is a non-empty string, all five model
ids (`plan`, `implement`, `review_find`, `review_verify`, `mechanical`) are
non-empty strings, and — in **phase** mode — the `model` difficulty tier is a
non-empty string. A partial payload is rejected outright, because the fetch agent
it replaces did *two* things — read the body **and** resolve the five per-step model
ids — so a partial hoist would still need a model-resolving agent (saving nothing)
while tripping the driver's `unresolvedStep` check and short-circuiting the whole
dispatch as a `fetchError`.

The `model` tier is in that set for a sharper reason than cost. It is the driver's
**sole** source for the phase's difficulty: unlike `stem` and `roadmap`, which fall
back to values the top-level args already carry, an absent `model` falls back to a
hard-coded `'medium'` (`const tier = isTask ? 'medium' : phaseMeta.model || 'medium'`).
That default is not neutral — `hasBlocking(findings, tier)` treats a surviving
`concern`-severity finding as blocking at `large` and not at `medium`, so a `large`
phase whose hoisted payload silently lost its tier would pass straight to `reviewed`
on a finding that should have forced a rework round. Accepting it would loosen the
gate with no error, warning, or log, which is the opposite direction from the
one-directional tightening the gate exists to uphold. `PHASE_META_SCHEMA` lists
`model` in its own `required` array, so the fallback agent path always supplies it;
the hoist path is simply held to the same bar. `TASK_META` carries no tier at all
and the driver hard-codes a task to `medium`, so task mode imposes no such
requirement. `scripts/verify-workflow-dispatch.sh` §6a covers both directions: four
negative cases (absent / empty / blank / non-string tier) fall back to the agent,
and a positive pair proves a hoisted `large` and a hoisted `medium` produce
*different* outcomes from one identical concern seed.

### `plan-review`'s `fetched` is structured-keys-only

`parsePlanArgs` reads `fetched` / `wontFixedTexts` / `mechanicalModel` from
**structured object keys only** — never from the `$ARGUMENTS` flag string, which
would let a raw prose target string masquerade as a fetched payload. This hoist is
the one in the set that is not a pure cost question: the agents it replaces have
twice transcribed junk over real plan tags in production, and `agent(..., { schema })`
provably cannot catch it (both corrupt returns were schema-valid). See
[`docs/mechanical-agent-inventory.md`](mechanical-agent-inventory.md) §
"The hoist with a recorded correctness failure". Driver-side validation of a hoisted
payload's *content* is deliberately out of scope there and owned by task
`fix-plan-review-gate-tag-clobber`.

`hoistedFetchedOk` is nonetheless held to be **no weaker than the schema it stands
in for**: `PLAN_TARGET_SCHEMA` / `ROADMAP_TARGET_SCHEMA` both list `tags` as
`required`, so the guard requires it too, all-or-nothing, per phase entry as well.
That is not shape pedantry — the gate writes the list back with `rdm ... update
--tags "<list>"`, and `--tags` **replaces** the whole list. A payload accepted with
no `tags` would be defaulted to `[]` by `buildReviewUnits` and issued as
`--tags ""`, wiping every real tag the item carried. Rejecting it costs one fetch
agent; accepting it costs the item's tags.

### `dispatch-phase` absorbs its diff instead of hoisting it

`diff:signals` is not hoisted — it is **absorbed**. `runCodeGate` calls
`d.implement(...)` immediately before every `d.review()` with nothing in between, so
the implementer (already in the worktree it just wrote to) runs the same two `git
diff` commands and returns `{ changedFiles, diffText }` under
`IMPLEMENT_RESULT_SCHEMA`. The review closure consumes it **one-shot** — read and
cleared — so a round-2 review can never inherit round 1's diff, and a null/empty
return falls back to the untouched `diff:signals` agent. Adding the schema changes
only the implementer's output contract: its `model` and effort are untouched.

`stamp:in-progress` is deliberately **not** absorbed: it fires before `runPlanGate`,
and a blocking plan finding escalates before any implementer runs, which would take
the item from `not-started` straight to `blocked` with no in-progress signal.

### Which caller surfaces supply them today

- **`dispatch-phase`, `rdm-do --auto`** — supplied by the *distributed*
  skill shims (`rdm-core/src/templates/skill-{dispatch-phase,do}-{cli,mcp}.md`)
  and their local copies.
- **`plan-review`, `backlog`, `document`, `review-refute-fix`, `estimate`** — supplied
  only by this repo's **local** `.claude/skills/*/SKILL.md` dogfood copies. Their
  distributed templates are not yet Workflow shims; converting them is tracked by task
  `convert-remaining-skill-templates-to-workflow-shims`.
- **MCP shims** hoist only what their tool surface produces. There is no MCP
  model-resolve tool, so `mechanicalModel` — and, by the all-or-nothing rule,
  `phaseMeta`/`taskMeta` — are omitted there and the in-workflow agent runs.
