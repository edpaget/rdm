# Workflow-tool orchestration: conventions & schema contracts

The autonomous lane of rdm's tooling is expressed as **Claude Code Workflow-tool
scripts** under `.claude/workflows/`, a sibling of `.claude/skills/` and
`.claude/hooks/`. This document defines the conventions those scripts follow and
the canonical schema contracts they exchange.

> **Scope:** mostly dogfood-only, with one emitted exception. The two workflow
> scripts — `rdm-wf-dispatch-phase.js`, `rdm-wf-review-refute-fix.js` — ARE now
> emitted by `rdm agent-config claude --skills --out <dir>`, byte-identical to
> this repo's own `.claude/workflows/` copies, under `<dir>/.claude/workflows/`
> (Claude-only, `--out`-only — see `CHANGELOG.md`). Those two emitted engines are
> **project- and binary-agnostic**: they name no particular rdm executable and no
> particular rdm project, because both arrive as runtime arguments (see
> § "Environment args: `rdmBin` and `project`"). Byte-identity with this repo's
> copies is a CONSEQUENCE of that design, not a limitation of it, and
> `scripts/verify-agent-config-distribution.sh` § 7 gates the claim by emitting
> into a hermetic non-rdm, non-Rust fixture repo and then executing the emitted
> engines' pipeline logic — and one of the rdm commands they build — there. The
> unshipped set is now exactly: `lib/*.mjs` (no regeneration script travels
> downstream to consume it) and the generator scripts
> (`scripts/gen-workflow-review.sh` and friends). rdm's shipped autonomous skills
> (`rdm-core/src/templates/skill-{autopilot,dispatch-phase}-{cli,mcp}.md`, and the
> `--auto` section of `skill-do-{cli,mcp}.md`) are the user-facing autonomous
> lane: `skill-autopilot-{cli,mcp}.md` is now a **prose** skill that itself
> drives the roadmap loop, invoking `rdm-wf-dispatch-phase` (and, locally, `rdm-wf-estimate`)
> as ordinary `Workflow` calls rather than being a thin shim over a workflow
> script of its own — see `docs/workflow-vs-prose-boundary.md` for why autopilot
> was retired from `.claude/workflows/` in favor of prose. `skill-dispatch-phase-{cli,mcp}.md`
> remains a thin shim that invokes `rdm-wf-dispatch-phase.js` via the `Workflow` tool,
> instead of re-narrating the orchestration in prose. Distributing the
> still-unshipped pieces (`lib/`, a downstream regeneration story) remains a
> follow-up roadmap.

## The `.claude/workflows/` convention

```
.claude/workflows/
  rdm-wf-<name>.js       # an ENGINE script — invoked via the Workflow tool
  lib/<name>.mjs         # a canonical source module (Node ES module; see below)
```

**Engine filenames carry the `rdm-wf-` prefix; `lib/*.mjs` filenames do not.**
An engine's filename and its `meta.name` are the entry a user sees in the
skill/slash-command listing, right next to the `rdm-*` skill front door that
drives it — so `rdm-dispatch-phase` (the skill) and `rdm-wf-dispatch-phase`
(the engine it invokes) are now distinguishable at a glance. A `lib/*.mjs` is a
shared source module, never a listing entry, so its name is deliberately
unprefixed and frozen. `spike-agent-type.js` is an exempt spike artifact and
keeps its bare name.

### Determinism: no `Date.now()`/`Math.random()`

Every `.claude/workflows/*.js` script — all nine of them, including the four
local-only ones that never leave this repo (`rdm-wf-backlog.js`,
`rdm-wf-document.js`, `rdm-wf-estimate.js`, `rdm-wf-plan-review.js`) — is
grepped for `Date.now(` / `Math.random(` by its own harness and must come
back clean. The reason is determinism of the pipeline GENERALLY, not any one
downstream consumer of it: `verify-workflow-backlog.sh` states the rule
plainly ("the pipeline must be deterministic"), and
`verify-workflow-dispatch.sh` asserts byte-identical output on identical
input as its own reproducibility contract. Resume-cache validity (see
[`docs/autonomous-loop.md`](autonomous-loop.md) § "Recovering a crashed
run") is ONE consequence of that determinism, not the sole or primary
reason for the rule: a call whose `(prompt, opts)` pair is not reproducible
can never safely replay from a cached result, but the rule exists to keep
every workflow's output reproducible — and its harnesses' byte-identical
assertions meaningful — even in scripts no resume attempt ever touches.

This is a **repo convention enforced by grep-based harness checks, not a
runtime restriction** — the global-scope table above lists `Date` and
`Math` as present in the isolate; nothing in the runtime itself stops a
script from calling `Date.now()`. `scripts/verify-workflow-estimate.sh`'s
own inline comment ("the runtime forbids them") is therefore inaccurate;
this section records the correct framing rather than editing that harness,
which is untouched in this pass.

### Observing the rendered listing

The prefix only pays off if the **rendered** listing shows it, and that listing
is produced by the Claude Code client from `.claude/`, not by anything in this
repo — so no hermetic check here can confirm it. What this repo gates
hermetically (`verify-workflow-review.sh` § 2d) is that the tree *declares* the
right names: engine filenames, `meta.name`-equals-stem parity, and engine/skill
name disjointness. Confirming the client agrees is a separate, deliberate step:

```sh
scripts/observe-workflow-listing.sh                  # live capture + assert
scripts/observe-workflow-listing.sh --self-test-only # hermetic; what CI runs
```

The script derives the expected names from the tree (never a hardcoded list),
spawns a fresh `claude -p` rooted at this repo, and asserts four things against
what comes back: every declared engine renders under its `rdm-wf-` name, no
bare pre-rename name survives, every `rdm-*` front door still renders under its
original name, and nothing is double-prefixed.

Two properties make the result trustworthy rather than decorative:

- **It discriminates.** Run against `main` and against the renamed tree at the
  same moment on the same machine, the identical command returned different
  listings — bare `backlog`, `dispatch-phase`, `document`, `estimate`,
  `plan-review`, `review-refute-fix` in the first, and the `rdm-wf-`-prefixed
  entries in the second, with all eleven `rdm-*` front doors unchanged in both
  and no double-prefixed entry (a doubled `rdm-`, or the engine prefix stacked
  in front of a front-door name) in either. The cwd is the only variable, so
  the listing genuinely tracks the tree.
- **Its assertions are not vacuous.** `--self-test-only` requires them to
  *reject* a pinned pre-rename listing and to *accept* one built from the
  tree's own declarations, and the script refuses to run at all if a derivation
  yields an empty name set — the failure mode where "no bare name was found" is
  true only because nothing was looked for. § 2d runs that half, so CI catches
  an observer that has stopped discriminating.

A stale listing is not evidence of a failed rename: a client only watches
directories that existed at *its* session start, so a long-running session can
render pre-rename names indefinitely. The script sidesteps this by spawning a
new client per run — a failure from it is real.

### Known-intentional survivors of an engine-name grep

Several tokens read like engine names but are **not** listing entries or
filenames, and are deliberately NOT renamed. Renaming them would ripple into
both generators and ~96 harness assertions for zero listing benefit:

| Token | What it actually is |
|---|---|
| `>>> review-refute-fix:begin` / `:end`, `find-refute-verdict`, `review-spec`, `estimate-core`, `dispatch-outcome`, `plan-review-driver`, `backlog-groom`, `document-core` | **Region marker names.** Internal identifiers naming a stamped or byte-copied block, consumed by the generators and their drift gates. |
| `'review-refute-fix: …'` runtime error prefixes in `lib/review.mjs` | **Module-scoped error prefixes**, identifying which module raised — not a file path. |
| `docs/token-baseline.json`'s bare per-engine record keys, and `docs/token-baseline.md`'s lane tables | **A frozen measurement corpus.** The figures are keyed to those names as recorded; rewriting them would invalidate `scripts/verify-token-report.sh --audit`. |
| `autopilot.js`, `lib/autopilot.mjs`, and the `autopilot` Workflow name | **Retired, with no successor.** `rdm-autopilot` survives as a prose skill with no engine behind it, so `autopilot` must never be prefixed — doing so would corrupt the one front door the rename must leave untouched. `scripts/verify-agent-config-distribution.sh`'s self-test D depends on `autopilot` naming a Workflow that does not resolve. |
| Historical `CHANGELOG.md` entries | Descriptions of the pre-rename world; correct as written. |

`scripts/verify-workflow-review.sh` § 2e runs the seven anchored reference-form
greps over the tree and fails on any hit outside that allowlist, with a
planted-mutation self-test proving the sweep is not vacuous.

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
module, to decide how `rdm-wf-review-refute-fix` is shared between the standalone
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
   (e.g. the standalone `rdm-wf-review-refute-fix` consumer).
3. **An unknown model id does NOT throw — `agent()` RESOLVES to `null`.** This is
   the dangerous one: `[models]` tier bindings are user-configurable, so a binding
   this runtime does not recognise would make every dispatched agent yield `null`
   and the pipeline would proceed into a null plan / silently-clean review. Both
   `rdm-wf-dispatch-phase.js` (plan/implement) and `lib/review.mjs` (finders)
   therefore guard explicitly against a `null` agent result whenever an explicit
   model was supplied, and fail loudly instead. Note a `null` finder result would
   otherwise be laundered into `[]` by the refute stage's `(found && …) || []`,
   so the guard converts it to a thrown stage — the only thing `pipeline()` turns
   into a `null` element.

**`rdm-wf-plan-review.js` used to omit `findModel`/`verifyModel` — that omission
was an OVERSIGHT, not policy, and has been fixed by
`thread-plan-review-judgment-models`.** `.claude/workflows/lib/plan-review.mjs`
used to call `runPlanReview({ target })` at both call sites with no model keys,
so `buildReviewPipeline` saw no `ctx.findModel`/`ctx.verifyModel` and its
finders and refuters inherited the ambient session model (see the "key
omitted" row above), while the sibling `rdm-wf-dispatch-phase.js` threaded
`{ findModel: models.review_find, verifyModel: models.review_verify }`. The
counter-argument that judgment sites are deliberately unpinned did not cover
this case: `f4e89d7` and `scripts/verify-workflow-review.sh` §5b-mechanical both
govern only the MECHANICAL pin, and no artifact said a judgment site should
carry no model at all. Both call sites in `lib/plan-review.mjs` (and the
byte-identical `rdm-wf-plan-review.js` copy) now pass `findModel`/`verifyModel`
inline on the same physical line as the `runPlanReview({...})` call, resolved
by extending the file's existing single `model:mechanical` bootstrap agent to
also resolve `review-find`/`review-verify` in one call (see the
`rdm-wf-plan-review` hoist-census rows below) rather than adding a second
bootstrap. Evidence, citations, and the still-unchanged refuter-*tier* decision
(`keep-opus`) are in [`docs/refuter-model-tiering.md`](refuter-model-tiering.md)
— not restated here.

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
workflows** (19 records across `rdm-wf-document.js`, `rdm-wf-backlog.js`, `rdm-wf-estimate.js`,
`rdm-wf-plan-review.js` and `lib/plan-review.mjs`), on the strength of the documented
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
| Live `rdm-wf-backlog` lane sidecars | `{"agentType":"rdm-mechanical","model":"haiku"}` — confirming a per-call `model` still overrides the definition, the assumption behind omitting `model:` from the agent file |

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

A first live lane dispatch (`rdm-wf-backlog`, propose-only, verified zero-mutation)
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
| `rdm-wf-estimate` | 30050 (n=79) | 21710 (n=15) | 8340 |
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

This remained a **distribution-scoped** hazard, as originally framed, for as
long as a downstream repo received no `.claude/agents/` definitions at all —
that gap has since closed (see below): a downstream repo now receives
`.claude/agents/rdm-mechanical.md`, so an `agentType: 'rdm-mechanical'`
reference in a shipped template WOULD resolve there if one were added. Neither
distributed template carries one yet.

The consequence was material at the time this section was written.
`rdm-core/src/agent_config.rs` exposed exactly `generate_skills` and
`generate_workflows`; there was **no** emission surface for `.claude/agents/`,
and adding one was out of scope for this phase by decision. Had
`agentType: 'mechanical'` been threaded into `rdm-wf-dispatch-phase.js`
and `rdm-wf-review-refute-fix.js` and re-synced into
`rdm-core/src/templates/workflows/`, every downstream repo running
`rdm agent-config claude --skills --out <dir>` would have received workflows
that **hard-fail on first dispatch** — not a "known-degraded surface", a broken
lane. `scripts/verify-agent-config-distribution.sh`'s semantic check greps only
for literal `.claude/workflows/<name>.js` references and would not have caught
it.

This was a blocking reason not to thread the two distributed files until the
follow-up task `ship-mechanical-agent-type-downstream` landed an emission
surface plus a reference-resolution gate. **This phase did not claim
distribution self-consistency, and did not introduce a distributed dangling
reference either — it declined to create one.** `ship-mechanical-agent-type-downstream`
has since landed that surface (`generate_agents()`, shipping
`.claude/agents/rdm-mechanical.md` into every downstream tree) and its
reference-resolution gate (`scripts/verify-agent-config-distribution.sh` § 3c) —
see the follow-up bullet below. Neither distributed template threads
`agentType` yet; that remains separate, not-yet-landed work.

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
source of `rdm-wf-plan-review.js`'s `plan-review-driver` block):

| File | Sites | Route |
|---|---|---|
| `rdm-wf-document.js` | `model:mechanical`, `fetch:roadmap-meta`, `gather:<stem>`, `write:draft` | unprojected driver |
| `rdm-wf-backlog.js` | `model:mechanical`, `fetch:report` | unprojected driver |
| `rdm-wf-estimate.js` | `model:mechanical`, `estimate:list`, `estimate:write:<stem>`, `estimate:tier:<stem>` | unprojected driver, below `estimate-core:end` — not distributed to any downstream workflow (`autopilot.js` formerly carried its own duplicate `estimate-core` copy before its retirement to prose) |
| `rdm-wf-plan-review.js` + `lib/plan-review.mjs` | `fetch:roadmap`, `fetch:<kind>`, `fetch:wontfix`, `gate:clear-tag:<kind>:<ident>` | byte-copied block — both halves edited, gated by §5b-drift |
| `rdm-wf-plan-review.js` | `model:mechanical` | unprojected driver, below `plan-review-driver:end` |

It is written as a plain literal at each site, never a module-level constant,
because the source text is byte-copied across files with different scopes.
`scripts/verify-workflow-review.sh` §2c asserts this **bidirectionally** — every
mechanical site carries it, no judgment site does — with planted-mutation
self-tests in both directions and a completeness sweep that fails if a site is
added or removed without updating the asserted list, or if any `agentType` other
than `rdm-mechanical` appears.

**Q1a has since confirmed this threading** — see
[the Workflow-path trim](#workflow-path-trim). Case B resolves with the trimmed
tool list, and a live `rdm-wf-backlog` dispatch shows both its threaded sites dropping
~19–20 %. The measured saving is **8907 tokens/agent (−23 %)**, not the 19894 the
`claude -p` 2×2 predicted; every figure quoted for these call sites is the
measured one.

**Not threaded, and each for its own reason:**

1. **The two distributed workflows** — `ship-mechanical-agent-type-downstream` has
   since landed the missing emission surface (§ Distribution):
   `generate_agents()` now ships `.claude/agents/rdm-mechanical.md` into every
   downstream tree, and `scripts/verify-agent-config-distribution.sh` § 3c
   resolves any emitted `agentType` reference against it — the successor to the
   removed `scripts/verify-workflow-review.sh` §2b(ii). Neither distributed
   template threads `agentType` yet; that remains a separate follow-up. The
   distributed reference count is zero at landing time, so § 3c's non-vacuity
   comes from an emitted-definition floor plus planted-corruption self-tests,
   not a real-reference occurrence floor.
2. **Every judgment site** — finders, refuters, planners, implementers,
   `synthesize:draft`, `analyze:*`, `estimate:rate:*` and plan-review's `act:*`.
   `rdm-mechanical` is a transcribe-only agent with a two-tool allowlist; giving
   it work that requires reasoning would break it. §2c(ii) gates this.
3. **`effort:` anywhere** — even though Q2a is positive. The fidelity question
   this bullet named as the remaining risk has since been **run**, and the answer
   is a recorded negative: fidelity passes, but there is no output-token drop and
   the option is unobservable on the mechanical tier's model. See
   [the 2026-08-15 follow-up](#regularize-followup) § 3. §2b(i) stays.

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
  **Both halves have since been settled — see [the follow-up](#regularize-followup).**
  Scope item 3 is answered in full: the structured-return question by 138 schema'd
  `rdm-mechanical` returns across 28 runs, and the `parallel()` question by two
  `rdm-wf-document` dispatches whose twelve fan-out agents all resolved. The
  `effort` half ran its fidelity study and came back a **negative** — threaded
  nowhere, §2b(i) unchanged.
- `ship-mechanical-agent-type-downstream` — **DONE.** Landed the `.claude/agents/`
  emission surface (`generate_agents()`) and its reference-resolution gate
  (`scripts/verify-agent-config-distribution.sh` § 3c), which lifted the
  now-removed §2b(ii). Its "hard failure on first dispatch" premise was
  observed rather than inferred (§ Distribution above). The emission surface
  ships the definition into every downstream tree; threading an `agentType`
  into either distributed workflow template remains separate, not-yet-landed
  follow-up work — the distributed reference count is zero.

<a id="regularize-followup"></a>

#### Follow-up: `regularize-mechanical-agents` (2026-08-14 / 2026-08-15)

Three things moved. Machine-readable twins live in `docs/token-baseline.json`
§ `mechanicalContextTrim` (`laneDeltaBroadened`, `parallelDispatchConfirmed`,
`effortFidelity`) — those are canonical for the figures; this section is the
narrative.

**1. The lane measurement is broadened, and it needed no fresh dispatch.** The
n=1 `laneDelta` above is now joined by a read of the corpus that accumulated on
its own between 2026-07-28 and 2026-08-11 — 28 runs, 138 `rdm-mechanical` agent
records:

| Class | Post-change median | n | Pinned pre-change | Delta |
|---|---|---|---|---|
| `fetch` | 25690.5 | 16 | 30098 (n=112) | −4407.5 (−14.6 %) |
| `gate` | 25664 | 25 | 29901 (n=9) | −4237 (−14.2 %) |
| `model` | 31782.5 | 22 | 36877 (n=16) | −5094.5 (−13.8 %) |

`--since` is `2026-07-28T17:37:00Z`, not a bare date: six runs earlier that same
day carry zero mechanical agents, so a day-granular boundary would have averaged
pre-change records in. `preChangeMedianTokens` is untouched — it is the pinned
comparator. These three are the *only* classes with both a clean pre-change row
and a threaded site; `stamp`/`advance`/`park`/`diff` have rows but occur only in
the unthreaded distributed workflows.

The trim is real and reproduces at n=16–25, but at **13.8–14.6 %**, below the
controlled pair's 23 %. That is expected rather than contradictory: the
controlled pair holds the prompt fixed, so it is an upper bound; these span real
prompts of differing length. Quote 8907 / 23 % for a *per-agent* claim and the
lane figures for a *lane* claim.

`estimate` stays out of any class-level claim, and the corpus now shows why
quantitatively instead of by assertion: in the same runs its mechanical
sub-labels sit at a 25592.5 median (n=64) while the judgment `estimate:rate:*`
sits at 41096 (n=32). Two incidental findings from those runs: `estimate:list`
and estimate's own `model:mechanical` bootstrap have **never** dispatched — every
estimate run took the caller-hoist path — so two threaded sites remain entirely
unmeasured. `gather`/`write` are reportable in ONE direction only. There is no
pre-change row for either — the corpus contains no pre-change standalone
document-workflow run — so no delta or percent may ever be quoted for them. The
two `rdm-wf-document` dispatches made for point 2 below do supply post-change
**absolutes**: `gather` 26923 (n=12), `write` 31595.5 (n=2), both on
`claude-haiku-4-5`. Absolutes, not deltas, and not comparable to another class's
pre-change median.

**2. `agentType` resolves through `parallel()` — confirmed, after correcting the
instrument everyone named for it.** The `regularize-mechanical-agents` phase body
and its approved plan both described plan-review's per-phase fan-out as carrying
its `gate:clear-tag:*` agent "inside the parallel thunk". It does not.
`lib/plan-review.mjs:849` fans out `reviewUnit`, which dispatches only judgment
agents; the act/gate half runs in a plain sequential `for` loop *after* that
barrier (`gate:clear-tag` at `:902`). `rdm-wf-estimate.js` has the same shape —
`parallel()` fans out the judgment `estimate:rate:*`, while the mechanical
`estimate:write:*`/`tier:*` follow sequentially. The corpus corroborates it
independently: across the eight multi-gate plan-review runs, **zero** of the 54
possible gate-agent pairs have overlapping execution windows.

So exactly one mechanical call site in the tree is dispatched through
`parallel()`: `gather:<stem>` in `rdm-wf-document.js`, via
`parallel(phases.map((p) => () => gatherPhase(p)))`. That set is now
machine-checked — `scripts/verify-workflow-review.sh` §2c(v) pins it and fails if
it changes, so a future refactor cannot silently move a mechanical site into a
fan-out, and the next reader cannot repeat the mis-selection.

Dispatching *that* lane answers the question. Two `rdm-wf-document` runs against
the fully-done `plugin-distribution` roadmap (6 done phases, `--out` pointed at a
scratch path, both returning `aborted:false`):

| Run | `gather:*` agents | all `agentType:'rdm-mechanical'` | `not found` raises | one `parallel()` batch | `startedAt` spread |
|---|---|---|---|---|---|
| `wf_762e3030-762` | 6 | yes | 0 | yes (single `queuedAt`) | 359 ms |
| `wf_e6452cce-cf7` | 6 | yes | 0 | yes (single `queuedAt`) | 390 ms |

All twelve fan-out agents resolved. Their `firstRequestTokens` median is 26923
(min 26920, max 26928) — on the trimmed side of the controlled pair's 29782 vs
38689, not the untrimmed one. The same two runs carry their own negative control:
`synthesize:draft`, the lane's one judgment agent, has no `agentType` and sits at
a 63188 median. **The revert branch was armed and not taken.** With this, every
threaded site's dispatch path — sequential and fanned-out — has been observed to
resolve.

**3. The `effort` fidelity study has been RUN, and the answer is a negative:
`effort: 'low'` is threaded nowhere.**

`spike-agent-type.js` gained a `mode: 'fidelity'` branch — 15 paired dispatches
(3 per schema shape across `STAMP_ACK`, `ACK`, `TIER`, `ESTIMATE`,
`DIFF_SIGNALS`), control = `effort` absent, treatment = `effort: 'low'`,
identical in everything else including `agentType` and the model pin, against a
throwaway plan repo and a throwaway 4-commit source repo seeded so every correct
answer is known independently of what an agent says. It was added to the existing
spike rather than as a new `spike-*.js`, which would have cost six separate
harness-exemption edits.

**The fidelity half passes.** Run `wf_0e8e31e2-415`, 30 dispatches: 15/15 pairs,
every `low` arm non-throwing, schema-valid, and semantically identical to its
`high` pair on the consumed fields — and in every case equal to the seeded
known-correct answer (`STAMP_ACK`/`ACK` true, true, false; `TIER` small, large,
medium; `ESTIMATE` easy, moderate, hard; `DIFF_SIGNALS` the three distinct
base-dependent file sets). Low effort did not degrade transcription here.

**Threading is still refused, on two independent grounds.**

*No output-token drop.* `effort` moves output/reasoning tokens, so that is the
axis to read (`byLabel`'s output columns — never `floorByAgentClass`, which
`effort` does not move). Over the same 15 pairs the treatment arm spent **more**:
11831 output tokens against 9819 (+20.5 %), median 798 vs 600, paired signs 8 up
/ 7 down. That is noise with, if anything, an adverse net. The plan's own rule is
to revert rather than keep the threading on faith when the drop fails to appear;
here it never appeared, so it is never applied.

*The treatment is unobservable at the tier these sites run on.* `effort` is
verified through the top-level `effort` field on each assistant transcript
record. Every mechanical site pins the mechanical tier, which resolves to haiku —
and across 9948 agent transcripts in the whole local corpus that field is absent
on **9914 of 9914** haiku assistant records. It has never once been emitted
there. The seven `"low"` records that exist corpus-wide are all opus, which is
where spike case E observed it. Combined with Q2b (an invalid `effort` value
degrades silently rather than throwing), threading `effort: 'low'` at a
mechanical site would be unfalsifiable at that site: no success channel, no error
channel. A fidelity pass on an unobservable treatment does not license shipping
it. Anyone revisiting this must first find a channel that exists on the
mechanical tier, or pin a model where the field is emitted.

**The negative-branch discipline therefore holds exactly.** No call site was
edited, and none of the four coupled artifacts was touched — §2b's `effort:` ban,
the `CLAUDE.md` rule, `effortDecision` and `CHANGELOG.md` all stand unchanged, so
the repo does not assert a prohibition its code violates in either direction.

**One discarded run, recorded so it is not mistaken for evidence.** The first
fidelity dispatch (`wf_8da984c5-f57`) is void. Its prompts appended `--root`
*after* the subcommand's arguments; `--root` is a global rdm flag, so every rdm
command in the study was rejected outright, both write shapes collapsed to a
constant `ok: false`, and some agents silently repaired the command while others
did not — leaving the arms uncomparable. Its one apparent divergence is an
artifact of that and is counted nowhere. Two fixes preceded the re-run: the flag
moved between binary and subcommand, and every shape gained the same explicit
"do not repair, reorder, or re-run a failing command with different arguments"
instruction. §2b-fid check (7) now gates the flag placement, with its own
planted-mutation self-test, so this class of instrument bug cannot recur silently.

`scripts/verify-workflow-review.sh` §2b-fid gates that the instrument stays
correctly *built* — coverage, pairing, discrimination (each write shape carries
an instance whose correct answer is `ok: false`, so a constant-answer guess
cannot score a false pass), throwaway roots required rather than defaulted since
two shapes write to a plan repo, and command validity. Each of the four has a
planted-mutation self-test.

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
| `category`      | string                                   | **optional**; security-style slug (injection / authorization / memory / crypto / exposure) |
| `location`      | string                                   | `file:line`, section heading, or phase stem       |
| `severity`      | `blocking` \| `concern` \| `suggestion`  | required; drives ranking and the overall verdict  |
| `confidence`    | integer 0–100 (required)                 | the finder's confidence **in the finding**        |
| `what_fails`    | string (required)                        | the specific problem                              |
| `why`           | string                                   | root cause / which rule, AC, or principle         |
| `recommendation`| string                                   | concrete fix                                      |
| `unrefuted`     | `true` (post-pipeline only)              | set by `buildReviewPipeline`, never by a finder    |
| `unrefutedReason` | `'non-gating'` \| `'budget'` (post-pipeline only) | present iff `unrefuted` is; WHY it went ungraded |
| `refuterError`  | `true` (post-pipeline only)              | a refuter was dispatched and CRASHED; never combined with `unrefuted` |

**`category` is additive, optional, and read by nobody.** It exists because
`FINDINGS_SCHEMA` is `additionalProperties: false`: the `security` dimension's
prose asks a finder for a threat-category slug (`command-injection`,
`path-traversal`, `unsafe-ffi`, `hardcoded-secret`, `info-disclosure`, …), and
without a declared field the runtime would **reject** that output and silently
discard every security finding. It is deliberately NOT folded into `concern`,
which is the DIMENSION identity three consumers match on
(`stripNonPhaseUnitOfWork`, `classifyPlanOutcome`, and `buildReviewPipeline`'s
`concern: f.concern || dim.key` backfill). The reference agent's
`(file, line, category)` **dedupe key is NOT implemented** in this pipeline — the
field is a carrier, not a half-built dedupe, and no consumer reads it today.

**Security severity maps onto the existing three-value ladder.** The `security`
dimension's impact scale is expressed directly in `blocking` / `concern` /
`suggestion` rather than as a parallel HIGH/MEDIUM/LOW enum: HIGH → `blocking`
(control of the system, or access to many users' data), MEDIUM → `concern` (real
but bounded — needs an authenticated account, a non-default configuration, or
victim interaction), LOW → `suggestion` (defense in depth and hygiene).
Uncertainty stays in `confidence`, never in `severity`.

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

A `null` `acTable` is **ambiguous on its own** — it means both "the table is
clean/empty" and "the `ac` dimension never ran". Do not read it as either. The
channel that distinguishes them is `coverage.acTableAbsent` (see `OUTCOME`
(review pipeline) below): true iff `ac` was selected in `code` mode and its
finder still resolved nothing after its one retry. It is recorded and named in
the summary, and it does NOT count as an AC gap.

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
`{ survivors, acTable, budget, coverage }`: `survivors` is a **ranked** array of
the surviving `FINDING`s, `budget` is the refutation-budget accounting (below),
`coverage` is the dimension-participation accounting (below), and
`acTable` is the captured `AC_ENTRY[]` from the `ac`
dimension's finder in `code` mode (`null` in `plan` mode, and `null` whenever
the `ac` dimension didn't run or its finder failed to resolve a table — the two
readings of that `null` are told apart by `coverage.acTableAbsent`, never by
`acTable` itself; see `AC_ENTRY` / `AC_REVIEW_SCHEMA` above). The dispatch-phase keystone (below)
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
`everHit` promises to keep visible — so `rdm-wf-dispatch-phase` threads
`planGate.budgetRounds`, not `planGate.budget`. (`planBudget` still accepts a
single object, for a caller that predates the plan gate returning an array.) The
two arrays are merged in **temporal** order, plan rounds first, because the plan
gate runs to completion before the code gate starts; consequently, when both
gates hit, `hit` — and therefore the summary clause — reports the later code
round, not the earlier plan one.

The fourth field, **`coverage`**, records which dimensions actually PARTICIPATED:
`{ mode, total, selected, ran, failed, retried, complete, acDimensionRan,
acTableAbsent }`. Every array is in `dims` **selection** order — written into an
index-keyed `attempts` record inside each finder thunk, never accumulated in
agent-completion order — so the field is as deterministic as the rest of the
`OUTCOME`. `total` is the number of dimensions `selectDimensions` returned for
this run (which, with no `signals`, is the fail-open full set), so it must never
be compared against a hard-coded dimension count. A dimension lands in `failed`
only when its finder resolved `null`/`undefined` on BOTH attempts (see **Failure
handling** below); a valid-but-empty payload (`{ findings: [] }`) PARTICIPATED
and is never a failure. `acDimensionRan` is `null` in `plan` mode and whenever
`ac` was not selected, which forces `acTableAbsent` false there.

It is projected exactly like `budget`, by the sibling pair in the same stamped
block — a second accounting FIELD, not a second mechanism:
`buildReviewCoverage(coverageRounds, planCoverage)` yields the `reviewCoverage`
field (the reported round's counts, `complete`, `everIncomplete`, `rounds`,
`planRounds`, `incomplete`, `last`), and `coverageSummaryClause(reviewCoverage)`
yields the visible ` [review coverage: N/M dimensions ran; failed: a,b]` marker
— with `; NO AC TABLE` appended when the `ac` dimension did not run — **empty**
when every round ran every dimension, so a complete run's summary is
byte-unchanged. Like `buildReviewBudget`, both parameters take the FULL
per-round array and are merged plan-first in temporal order; unlike it, the
REPORTED counts come from the chronologically last INCOMPLETE round, so the
clause names the real gap rather than a later healthy round's full numbers.

The clause is deliberately free of quotes, `$` and backticks: it is interpolated
into mechanical Bash prompts (plan-review's round-note write, the optional gate's
`--reason` flag). Its position is fixed — budget clause first, coverage clause
second — in every branch of `buildOutcome` / `buildTaskOutcome`, in
`plan-review`'s `reviewUnit`, and in `rdm-wf-review-refute-fix.js`'s driver, so a
run that hits both produces a deterministic string.

Non-participation is **recorded, never gated on**. A transient API blip must not
stall the autonomous lane, while the record keeps the reduced coverage auditable
after the fact. `coverage` therefore reaches exactly three places — the returned
`coverage`/`reviewCoverage` field, the `summary` string (and via it the derived
`reason`), and log lines — and nothing else: it is never an input to
`classifyOutcome`, `acTableHasGap`, `hasBlocking`, `survives`, `GATE_POLICY`, or
any rework/revise loop predicate. The trade this accepts is that an incomplete
review can still yield `reviewed`; that is tolerable only because the clause is
in the human-visible text, which is why it is a `summary` append rather than a
machine-readable key alone.

The standalone `rdm-wf-review-refute-fix.js` consumer has three invocation shapes: (a)
`mode: 'plan'`, and (b) `mode: 'code'` with no `roadmap`+`phase` or `task`
identifier, both keep returning the legacy survivors-only `{ mode, survivors }`
shape (plus the additive `budget` and `coverage` fields) for backward
compatibility with ad hoc/document-less reviews;
(c) `mode: 'code'` with `{ roadmap, phase }` or `{ task }` runs the SAME
`buildReviewPipeline('code')` pass, then additionally derives real diff signals
from the item's worktree (mirroring dispatch-phase's code gate — see below) and
composes the survivors through `classifyOutcome` plus `statusFor` /
`writesCompletion` / `summarizeFindings` / `gateFor` into the dispatch-shaped
`OUTCOME` contract: `{ roadmap, phase, outcome, status, writesCompletion,
summary, reason, reviewBudget, reviewCoverage, findings }` (or the
`{ task, ... }` shape). An optional
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
were kept this way.

A **finder** that resolves `null`/`undefined` is **retried exactly once** (same
prompt, same options, label suffixed `:retry`). `agent()` resolves null only
AFTER the runtime has exhausted its own internal retries, so this second-order
retry is the first one lane code controls; finders are read-only and idempotent,
so re-dispatching one is safe. A `null` **cannot be attributed to a cause** — it
means either a transient API death or an unknown/unavailable model id (the
`model` spike's silent-null consequence) — and nothing distinguishes them at the
call site, so no attempt is made to classify it: one retry handles a transient
failure, and a misconfigured model fails twice and is recorded loudly. There is
no loop and no backoff. A finder that THROWS is recorded and rethrown WITHOUT a
retry.

Only after the retry does the dimension drop to `null` (the runtime's `parallel`
sends a thrown thunk to null): the other dimensions still contribute, the review
degrades rather than failing, the dead dimension contributes no candidates (so it
never inflates `budget.produced` with coverage it did not provide), and it is
recorded in `coverage.failed`. Because the record is surfaced in the summary, a
3-of-7 review can never be mistaken for a clean 7-of-7 — which it silently was
before this contract existed.

The wholesale-failure guard — EVERY selected dimension resolving null — throws
rather than reporting a clean review, and is **model-INDEPENDENT**: only its
message branches, keeping the recognisable `check the [models] tier bindings`
text on the model path. This mattered because `lib/plan-review.mjs` used to
call `runPlanReview({ target })` with NO `findModel`/`verifyModel` at either
call site, so a model-conditional guard would have been inert in plan mode and
`GATE_POLICY.plan` would have cleared `needs-plan-review` off a review that
never ran. `thread-plan-review-judgment-models` has since threaded both keys
into plan-review's context (see the model-omission paragraph above), but the
guard stays model-independent regardless — it must hold for any caller,
threaded or not.

An **absent** AC table is distinct from a **clean** one. `acTableHasGap`'s
contract is unchanged and deliberately not widened (`acTableHasGap(null) ===
false` for both readings); `coverage.acTableAbsent` is the channel that tells
them apart, and it is recorded and named in the summary but never gates.

**Ranking (`rankFindings`).** A total order, so `OUTCOME` is deterministic across
runs (the convention bans `Date.now()`/`Math.random()`, see § "The
`.claude/workflows/` convention" above): by `severity`
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
| arg name | `maxRefutations` (on `rdm-wf-dispatch-phase`, `rdm-wf-plan-review`, and `rdm-wf-review-refute-fix` args; reaches `runReview` as `context.maxRefutations`) |
| default | `DEFAULT_MAX_REFUTATIONS` = 5 |
| `0` | LEGAL and meaningful — grade nothing, pass every gating finding through as `unrefutedReason: 'budget'`. Never conflated with "unset" by a falsy check. |
| uncapped | no sentinel exists; express an effectively-uncapped run as a large N |
| validation | `resolveRefutationBudget(value)`, mirroring `parseBudget`'s contract — a number or integer-ONLY string; `'5abc'` is rejected, not coerced. `rdm-wf-dispatch-phase`/`rdm-wf-plan-review` validate at PARSE time, before any `agent()` call. |
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
`--implementation-plan` branch, and `rdm-wf-review-refute-fix.js`'s legacy and
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

#### Dimension prose states intent; the project's principles document states the conventions

A dimension's `focus` string (and the `//|` spec prose rendered beside it) states
**generic intent** — what the lens is looking for — and then directs the finder
agent to read the consuming project's principles document for the concrete
conventions: `docs/principles.md` if present, otherwise `CLAUDE.md` / `AGENTS.md`
in the project root. `code`'s `correctness`, `architecture`, `api-docs`,
`changelog` and `security` follow the pattern `plan`'s `architectural-fit`
established. So
`api-docs` triggers on "the diff changes a public API item" and asks whether the
documentation sections the project requires are present, without naming any one
language's doc-section headings; `changelog` requires a same-commit entry without
fixing the changelog file or its format; `architecture` asks whether logic lives
where the project's stated layering contract puts it, rather than naming this
repo's own modules; and `security` asks whether each use of the language's
escape hatch out of its own safety guarantees is justified in the form the
project requires, without naming one language's keyword or comment convention.
The pipeline is the same reviewer in every repo; the rules it enforces come from
the repo it is pointed at.

**The channel is prose only.** The finder agent reads the file itself — agents
can read files, so the JS does not have to. There is deliberately **no
`principles` pipeline input, no substitution pass or template placeholder, and no
per-dimension convention override**. A placeholder is not even available here:
`.claude/workflows/*.js` is both the template and the executed file, so an
unsubstituted token would sit in the file the runtime actually runs, and any
emit-time substitution would break the byte-identity gates. Retargeting the
reviewer at a different project therefore requires no code change at all.

**No carve-out remains, and the empty set is enforced.**
`scripts/verify-workflow-review.sh` § AC2b asserts that the set of code
dimensions whose title or focus still carries a language-specific idiom is
**exactly `[]`** — the assertion is kept rather than deleted precisely so the
carve-out cannot silently re-open. The same section asserts both halves of the
property for `correctness`, `architecture`, `api-docs`, `changelog` and
`security`: no crate/language/doc-section token in their prose, **and** a
surviving pointer to the principles document, so genericity cannot be "achieved"
by deleting the convention channel outright. Three planted-mutation self-tests
(§ 4a) prove all three assertions fire, and § 10h applies the same zero-grep to
the rendered skill surfaces — whole-file for the crate/doc-section tokens, and
scoped to the `rdm:review-spec` region for the retired idioms, since each code
skill's own hand-written diff-signal prose (outside the markers, owned
elsewhere) still names them. A `//|` spec line and a `focus` string are
independent projections, so a regression could land in either alone.

What `security` deliberately does **not** do is restructure its threat taxonomy:
it still enumerates injection, path traversal, secret leakage, authorization and
deserialization by name. Rebuilding that half on a language-neutral threat-model
vocabulary is a separate unit.

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
`rdm-wf-plan-review.js`, which filters on `f.concern === 'unit-of-work'`, and folding a
conditionally-scoped lens into an unconditional agent would defeat that scoping.

### `context.signals` and `selectDimensions(mode, signals)`

`selectDimensions` has a **three-way contract**, and the fail-open branch is
load-bearing:

- `signals == null` (omitted, or genuinely unknown) → return **ALL** dimensions
  for the mode, untouched. A caller that cannot compute a diff knows the least,
  so it must get the most coverage. `rdm-wf-review-refute-fix.js`'s legacy
  survivors-only shapes ((a) `mode: 'plan'`, (b) `mode: 'code'` with no item
  identifier) and dispatch-phase's **plan** gate take this path today;
  dispatch-phase's **code** gate and `rdm-wf-review-refute-fix.js`'s full
  `{ roadmap, phase }` / `{ task }` code-review path both now compute real
  signals (see below) and only fall back to this branch when the diff is
  unavailable.
- an **explicit** signals object — even `{}` — → the always-on dimensions plus
  exactly those whose `when` fires. `{}` means "computed, nothing triggered".
- an unknown `mode` → throw.

The `context` contract carries **no** project-conventions key. Making the
dimension prose project-agnostic (above) added no pipeline input: the finder
agent is told in prose to read the project's principles document, so `context`
still holds only `target`, the optional `signals`, `maxRefutations`, and whatever
else a caller threads into the prompts.

Do **not** write `d.when(signals || {})`. Substituting `{}` for omitted signals
makes every conditional predicate read falsy and silently drops the triggered
dimensions — a strict coverage subset returned precisely when the caller had no
information. Omitted signals and an empty signals object are deliberately
different paths, and `verify-workflow-review.sh` asserts both.

**Two fail-open layers, and they are different things.** The rule above is the
**object-level** fail-open: a caller with no diff at all omits `signals`
entirely, and every dimension runs. `deriveSignals` adds a **value-level**
fail-open one layer down (below): a caller that HAS changed files but could not
read their content still returns a fully-populated object, with the
undeterminable signals set to `true`. Both remain live. "Callers that cannot
compute a diff must pass no signals rather than a partial object" still governs
the first case; the second case never produces a partial object at all.

### `deriveSignals(input)`

Pure and deterministic (no `Date.now`/`Math.random`, no shell). Maps
`{ targetType, changedFiles, diffText? }` onto a **fully-populated** signals
object — every boolean key in `SIGNAL_KEYS` (`changesLogic`, `missingTests`,
`multiModule`, `publicApiChanged`, `userFacing`, `securitySurface`) is set
explicitly. A partially-populated object would make a conditional
dimension drop out on a *missing* key rather than a real negative.

**Every conditional signal derives from diff CONTENT — the ADDED lines only —
not from declared or conventional paths.** There is no generic way to specify
paths that works across repos: a path list is either repo-specific (a hard crate
prefix, permanently false everywhere else, so its dimension silently never
fires) or fires on a spelling coincidence (a bundler config file matching a
`config` segment, so its dimension fires on an unrelated change). Both failure
modes are *confident* and both degrade silently, which is why neither is
tolerated. Scanning only added lines means a *removed* `export`/`exec(` line
never trips a signal, and a `+++ b/path` header is never read as content.

Three named vocabularies, all literal regexes, all module-level constants with
no `g`/`y` flag (a global regex carries `lastIndex` across `.test()` calls and
would break determinism):

| vocabulary | signal | covers |
| --- | --- | --- |
| `EXPORT_CONTENT_PATTERNS` | `publicApiChanged` | `export` / `export default`, `module.exports`, Rust `pub`/`pub(crate)` + item kind, Java/C#/TS `public`, a capitalized Go identifier, Python `__all__` |
| `USER_FACING_CONTENT_PATTERNS` | `userFacing` | CLI subcommand/argument/flag registration, the help/usage strings attached to them, HTTP/RPC route/handler/tool registration, printed or logged output |
| `SECURITY_CONTENT_PATTERNS` | `securitySurface` | process/command execution, filesystem access, environment and secret reads, deserialization/eval, raw memory (both Rust `unsafe` shapes: the inline `unsafe { … }` expression **and** the `unsafe fn`/`impl`/`trait`/`extern` declarations) |

A `CHANGELOG.md` path in `changedFiles` is a positive-**confirming** term for
`userFacing`, never a sole trigger — a CHANGELOG-only diff has no code files and
stays a genuine `false`.

Two exclusions are deliberate and must not be "fixed": a bare `function`/`def`
is not in the export vocabulary (a module-private definition is not a public-API
change, and including it would make `api-docs` always-on in every JS/Python
repo), and `JSON.parse(` is not in the security vocabulary (it is the most
common line in any JS/TS diff and would collapse `security` into an always-on
dimension). `Command::new(` is ambiguous — clap in one crate, `std::process` in
another — and is assigned to the security vocabulary only; user-facing CLI
detection uses `Arg::new(` / `.arg(` / `.about(` / `.help(` instead.

The two retired path lists — the security one and the user-facing one — **no
longer exist** and are deliberately not replaced, for that same reason. Their
identifiers are gone from the source, and `verify-workflow-review.sh` § 2a keeps
them gone. Paths survive only in `TEST_PATH_PATTERNS` and
`CODE_EXTENSIONS`, which answer *what kind of file is this* (test vs. code)
rather than *what surface does this change touch*; both are convention-based and
multi-language, and both are unchanged. Content is scanned in its ORIGINAL case
(Go's exported-identifier rule and the Rust/Java keywords are case-sensitive);
only paths are lowercased.

**No declared-path or project-config channel was introduced.** `deriveSignals`'
input shape is still exactly `{ targetType, changedFiles, diffText }` — content
derivation reads inputs every caller already supplies — and both call sites are
unchanged.

**The four-branch rule** (one shared helper, `contentSignal`, that all three
conditional signals route through; branch order is load-bearing):

1. A **positive content match** → `true`.
2. **`codeFiles.length === 0`** → a confident `false`, regardless of `diffText`.
   A docs-only diff is a genuine negative. This branch is tested FIRST — reversed
   with branch 3, a docs-only diff with an unreadable body would fail open and
   re-run every conditional dimension on prose.
3. Code files changed but the content could **not be read at all**
   (`diffText === null`) → undeterminable, so the signal **fails open by VALUE:
   it is set to `true`** so its dimension still runs. The key is **never
   omitted** — `selectDimensions`' `signals == null` test is a *whole-object*
   check, so an omitted key reads `undefined`, coerces false, and silently DROPS
   the dimension.
4. Content **was** read and nothing matched → a confident `false`. Absence of a
   match in readable content is a real negative, not an unknown; this is what
   keeps the fail-open from widening into "run every dimension on every code
   diff". Note `diffText: ''` is a *string*, not `null`: an empty-but-present
   diff takes this branch.

**Who feeds it.** `rdm-wf-dispatch-phase`'s code gate runs a mechanical `diff:signals`
agent inside the item's worktree (`git diff --name-only main...HEAD` plus a
truncated `git diff main...HEAD`) and threads the result through `deriveSignals`
into `buildReviewPipeline('code')` — recomputed on **every** rework round, so a
round-2 fix that newly adds an exported symbol turns `api-docs` on for that
round. The three-dot base scopes to the branch's own changes; for a phase in
a shared per-roadmap worktree that is over-inclusive (earlier phases' files ride
along) but never under-inclusive, which is the safe direction for a coverage
gate. **Truncation now cuts the other way.** A very large diff is truncated at
40000 chars in the prompt; a truncated diff is still a non-null *string*, so it
takes branch 4 (confident `false`) on content that was cut off. Under the old
path rules truncation only weakened detection toward fail-open; under content
derivation it weakens detection toward a false NEGATIVE. That is the one place
where content derivation is strictly less safe than the path rules it replaced,
and it is bounded by the same 40000-char window on both the `diff:signals` agent
and the implementer prompt that absorbs it. **Signals-absent fail-open
contract:** if the diff agent fails, returns null, or reports no changed files,
the driver omits the `signals` key **entirely** — never `{}` — so every
dimension runs.

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

- the **stamped block** (`rdm-wf-review-refute-fix` markers) — copied verbatim into the
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
`rdm-wf-dispatch-phase`/`autopilot`. The plan rows carry an explicit `status: null` — a
plan review never persists an rdm status; it clears `needs-plan-review` on
`reviewed` and leaves it on `rework`/`escalated`.

### `rdm-wf-plan-review`'s gate disposition: `gateAction` / `gateBlocked` / `gateDeferred`

`GATE_POLICY.plan` above says what the gate *should* do. These five result fields,
added by `phase-4-plan-review-gate-blocked-by-safety-classifier`, say what it
actually did — the two used to be silently conflated, so a refused tag write
returned `clearsPlanReviewTag: true, tagCleared: false` and nothing else.

| field | where | meaning |
|---|---|---|
| `gateAction` | every gated unit, plus the single-target flatten | The declarative action: `{ kind, ident, roadmap, clearsPlanReviewTag, commands, remainingTags, removedTags, applied, deferred, blocked, blockedReason }`. `commands` is `[updateCmd, commitCmd]` built by the same `planGateCommands` helper the gate PROMPT prints, so a caller applying it by hand issues byte-identical writes; it is `[]` on a `rework`/`escalated` unit, which still gets an action so callers can iterate `units[].gateAction` without special-casing. `blockedReason` is `'ack-not-ok'` (a refusal) or `'agent-error: <message>'` (a crash). |
| `gateBlocked` | every gated unit, plus the flatten | `true` when a `reviewed` unit's tag write was attempted and did not succeed. Also drives a ` [GATE BLOCKED: …]` clause on the unit's `summary` and a dedicated log line on BOTH failure paths. |
| `gateDeferred` | every gated unit, plus the flatten | `true` when `gateMode: 'return'` made the driver compute the action and write nothing. A hand-off, DISTINCT from `gateBlocked` — the loud clause must not fire on it. Drives a lowercase ` [gate deferred: … — apply: <update> && <commit>]` clause carrying the commands verbatim, so a surface that reports only `summary` is already reporting the escalation. |
| `gateBlockedCount` | run-level result (and the fetch-failure / model-abort early returns, as an explicit `0`) | How many units are blocked. Appended to the final `N unit(s) gated` log line when non-zero. |
| `gateDeferredCount` | same places, same explicit `0` | How many units were deferred. A SEPARATE count — a deferral is never folded into `gateBlockedCount`, so a caller alerting on "the gate did not land" and a caller that must go apply commands read different fields. Appended to the same log line, with the lowercase `gate deferred` marker rather than the uppercase `GATE BLOCKED` one, so the two stay greppable apart. |

The `--implementation-plan` branch has no persisted item, so it gains **none** of
these keys.

Both gate clauses embed an exact rdm command containing double quotes
(`--tags "a,b"`), so — unlike `coverageSummaryClause`, which is quote-free
precisely *because* it is interpolated into Bash prompts — `summary`/`reason` in
plan mode are returned **data** and must never reach a prompt builder.
`verify-workflow-review.sh` § 5b-gate-quoting pins that with a grep over every
`build*Prompt` body plus a planted-leak self-test.

**`gateMode` arg.** `'apply'` (default) | `'return'`, read from the STRUCTURED
`args` object only — never parsed out of the `$ARGUMENTS` flag string, the same
rule as `fetched`/`wontFixedTexts` — and validated at PARSE time, before any
`agent()` call, the `resolveRefutationBudget` precedent. Under `'return'` the
`gate:clear-tag` agent is not dispatched at all. The gate prompt itself is now
evidence-carrying (a four-clause authorization preamble plus the rendered review
evidence); the decision behind both, its boundary, and the recorded classifier
blocks that forced it live in
[`plan-review-gate-policy.md`](plan-review-gate-policy.md).

Everything else inside the stamped block is **machinery** (JSON schemas,
`survives`/`rankFindings`/`selectDimensions`/`deriveSignals`, the classifier and
the gate policy) and is never rendered into a skill. Both generators are
`--check`-gated by `scripts/verify-workflow-review.sh` — the skill generator in
BOTH modes — which CI runs.

## dispatch-phase contracts

`rdm-wf-dispatch-phase` (`.claude/workflows/rdm-wf-dispatch-phase.js`) is the keystone per-phase
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

The top-level return of `rdm-wf-dispatch-phase`. Distinct from the review pipeline's
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
| `reviewCoverage` | object \| `null`                   | `buildReviewCoverage(...)` — which review dimensions PARTICIPATED: the reported round's `total`/`selected`/`ran`/`failed`/`retried`/`acDimensionRan`/`acTableAbsent`, plus `complete`, `everIncomplete`, `rounds`, `planRounds`, `incomplete`, `last`. `null` when no review reported one — including the `fetchError` short-circuit, which never ran a review and must not read as full coverage. |
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
| `reviewCoverage` | object \| `null`                   | `buildReviewCoverage(...)` — which review dimensions PARTICIPATED: the reported round's `total`/`selected`/`ran`/`failed`/`retried`/`acDimensionRan`/`acTableAbsent`, plus `complete`, `everIncomplete`, `rounds`, `planRounds`, `incomplete`, `last`. `null` when no review reported one — including the `fetchError` short-circuit, which never ran a review and must not read as full coverage. |
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

**`reviewCoverage` makes an INCOMPLETE review legible.** When any round lost a
dimension (its finder resolved null on both its attempt and its one retry),
`coverageSummaryClause` appends
` [review coverage: N/M dimensions ran; failed: a,b]` — plus `; NO AC TABLE`
when the `ac` dimension is the one that died — to `summary` in all three outcome
branches, immediately AFTER the budget clause so a run that hit both produces a
deterministic string. Because `outcomePolicy` derives `reason` from `summary`, a
parked or escalated unit whose review was incomplete surfaces that in the
`rdm review blocked` queue for free. A run in which every dimension ran keeps a
byte-unchanged summary — the clause is empty.

Non-participation is **recorded, never gated on**: an incomplete review can still
yield `reviewed`, and `classifyOutcome` receives nothing new (a `coverage` key
never enters any `classifierInput`, and the rework/revise loop predicates never
consult it), so a transient API blip cannot stall the lane. What it can no longer
do is pass unnoticed.

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
that is why the code stage yields only `reviewed`/`rework`. `rdm-wf-dispatch-phase` never
emits a `Done:` line — it emits `writesCompletion` and landing is a separate,
later step.

**Rework notes carry the AC table too.** `runCodeGate`'s rework call is
`d.implement({ findings, acTable })` — never a bare findings array. Because
the AC table is a structured side-channel decoupled from `findings` (a
`FAIL`/`PARTIAL` criterion need not also appear as a finding), an AC-only-gap
rework round has an *empty* `findings` array; without also passing `acTable`
the implementer would receive no signal at all about what to fix and the
rework budget would very likely burn out reproducing the same gap.
`rdm-wf-dispatch-phase.js`'s `buildImplementPrompt` renders the two channels
separately — "ranked issues" from `findings` and "UNMET criteria" from the
`FAIL`/`PARTIAL` entries of `acTable` — and explicitly notes they are not a
duplicate report of the same thing.

**The AC-only-gap summary fix applies everywhere `acTable` feeds
`classifyOutcome`.** `buildOutcome`/`buildTaskOutcome` name the real cause
(`'code rework unresolved: unmet acceptance criteria in AC table'`) instead of
the misleading `summarizeFindings([])` → `'no surviving findings'` when an
AC-only gap forces `rework` with an empty findings array; `rdm-wf-review-refute-fix.js`'s
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
commit). `rdm-wf-dispatch-phase.js` wires this dep to an `agent()` call using
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

**The code-review stage is the canonical review.** `rdm-wf-dispatch-phase` builds it
from the stamped `buildReviewPipeline('code')` — there is no independent
code-review logic in the driver — and feeds it `deriveSignals` output from the
real branch diff (see `deriveSignals(input)` above for the signals-absent
fail-open contract). `verify-workflow-dispatch.sh` pins both halves: exactly one
`buildReviewPipeline('code')` binding site and one declaration each of
`findPrompt`/`refutePrompt`, plus the `deriveSignals(` / `signals:` /
`diff:signals` wiring.

### Environment args: `rdmBin` and `project`

`rdm-wf-dispatch-phase` names NO particular rdm executable and NO particular rdm
project. Both are **runtime args**, threaded through a trailing `cfg` parameter
on every prompt builder that shells out. This is the contract the rest of the
project-agnostic lane consumes — the same helper shape and the same allow-list
already apply to `rdm-wf-review-refute-fix` / `rdm-wf-estimate` and to the prose
`rdm-autopilot` loop; they must not re-derive it.

| arg       | required | shape                                            | applies to |
| --------- | -------- | ------------------------------------------------ | ---------- |
| `rdmBin`  | no (defaults to `rdm`) | non-empty, non-whitespace string; absent → a plain `rdm` on `PATH`; a present-but-non-string value throws | every emitted `rdm` invocation |
| `project` | no       | plain name matching `/^[A-Za-z0-9._-]+$/`, or absent | PROJECT-SCOPED subcommands only |

An emit-time `{rdm_bin}` placeholder is not workable here:
`.claude/workflows/*.js` is simultaneously the template `generate_workflows()`
`include_str!`s **and** the file the Workflow tool executes, so a placeholder
would sit unsubstituted in the file rdm itself runs. Runtime args change no
bytes, which is also why every byte-identity gate stays green across this
change.

#### The project-agnostic allow-list

`projectFlag(cfg)` (`cfg && cfg.project ? ' --project ' + cfg.project : ''`,
the same shape `rdm-wf-backlog.js` uses) is appended at **project-scoped** call sites
only. These subcommands reject `--project` outright and must carry NO flag:

    rdm model resolve, rdm commit    (and rdm status / rdm discard, if added)

Every other subcommand this lane emits is project-scoped and takes the flag:
`phase list/show/update`, `task list/show/create/update`, `worktree add`,
`next`, `search`. A blanket append would produce commands that fail at runtime
while still satisfying a naive whole-file grep, which is why
`scripts/verify-workflow-dispatch.sh` § 9b drives the real workflow under a
capturing fake agent, tokenizes every emitted `rdm <subcommand>` occurrence, and
checks each against the allow-list expressed **as data** — flag present iff the
subcommand is not on the list, and zero `--project` occurrences at all when no
project was configured.

#### Why `rdmBin` defaults to `rdm`, and how this repo overrides it

`resolveRdmBin(value)` has three states: it returns a non-empty string
**verbatim**, it returns a plain `'rdm'` for an **absent** value (`undefined`,
`null`, `''`, whitespace-only), or it **throws** on a present-but-wrong-**type**
value (`42`, `{}`, `[]`, `true`).

The shipped default is a plain `rdm` on `PATH`. A consumer who installs the
`rdm` plugin has no repo-local build path to pass, and `PATH` is the right answer
for essentially all of them, so a required arg made every downstream consumer pay
for a hazard that belongs to exactly one repo.

That hazard is real, and it is **dogfood-scoped**: inside the rdm repo a bare
`rdm` is a stale installed build, which the project's development-build rule
forbids. The compensating control therefore lives where the hazard does —
`RDM_BIN = "{{config_root}}/target/debug/rdm"` in this repo's `.mise.toml`,
beside `RDM_ROOT`/`RDM_PROJECT`. `verify-workflow-dispatch.sh` § 9c-dogfood gates
that entry (with planted-mutation self-tests for a deleted line and for a
downgrade to the bare `rdm` default), so the control cannot silently rot.

Two rules survive the reversal unchanged:

- **An existence preflight is still forbidden.** The default must be a plain
  fallback, never a probe. `which -a rdm` resolves to the stale global, so a
  probe passes while running exactly the wrong binary — it would hide the
  dogfood hazard rather than close it. `verify-workflow-dispatch.sh` § 9c greps
  (over non-comment lines) across every `resolveRdmBin`-bearing copy to prove no
  copy was implemented that way, with a planted-probe self-test.
- **An explicitly passed value still wins verbatim**, and the sentinel
  `rdmBin: 'rdm'` remains valid — it requests `PATH` resolution deliberately
  rather than falling into the default branch. A trailing-space assertion in
  § 9c keeps the two paths discriminable.

The wrong-type throw is deliberate: degrading a `rdmBin: 42` typo to `PATH` would
reintroduce exactly the silent-wrong-binary failure the absent-value default does
not need. Validation still runs inside `parseDispatchArgs`, which the driver
executes as its very first statement, so a mis-typed payload costs zero tokens
(the same discipline as `parseBudget`).

**Where `$RDM_BIN` is actually read.** Not here. The Workflow runtime has no
filesystem and no environment access, so no workflow JS reads `process.env` —
`verify-workflow-dispatch.sh` § 9c-inventory asserts this across every
`resolveRdmBin`-bearing file, with a planted `process.env.RDM_BIN` self-test.
Resolution happens in the **calling skill**, a live agent with Bash, which passes
the result down as an argument; the JS-side change is only the absent-value
default. The three-step order is `--rdm-bin` → `RDM_BIN` → a plain `rdm` on
`PATH`, and it is stated in exactly **two** places, never in a skill body:

- `PLUGIN_RDM_BIN_NOTE` in `rdm-core/src/agent_config.rs`, appended verbatim by
  `append_plugin_rdm_bin_note()` to every emitted **plugin** skill whose body
  mentions `rdmBin`. That appended section is authoritative for a
  plugin-installed consumer.
- this section, which the repo-local `.claude/skills/` copies cite by name.

Every skill body — the `rdm-core/src/templates/skill-*.md` base templates, their
emitted `--skills`/`--plugin` renderings, and this repo's dogfood copies — states
only the **default** (`rdmBin` is optional; omitted, a plain `rdm` on `PATH` is
used), that an explicit value wins verbatim, and a pointer to one of the two
sources above. None of them re-lists the ordered steps, so a change to the order
has two call sites rather than thirteen.

**Every caller still threads `rdmBin`, and every per-shim assertion stays.** What
changed is the consequence of omitting it: an un-threaded caller now **degrades**
to a `PATH`-resolved `rdm` rather than hard-breaking on first dispatch. In this
repo that degradation is the wrong-binary hazard above, which is why the per-shim
greps are worth keeping — they now guard a silent-wrong-binary failure rather
than a loud one. The verified callers of the `rdm-wf-dispatch-phase` Workflow are
`.claude/skills/rdm-dispatch-phase`, `.claude/skills/rdm-do` (both `--auto`
flows), `.claude/skills/rdm-autopilot`, and the shipped
`skill-dispatch-phase-{cli,mcp}.md` / `skill-do-{cli,mcp}.md` /
`skill-autopilot-{cli,mcp}.md` templates. All of them pass `rdmBin`, asserted
per-shim by `verify-workflow-do-auto.sh`, `verify-workflow-do-auto-task.sh`,
`verify-skill-autopilot.sh`, and `verify-agent-config-distribution.sh` § 6d, each
with a planted-removal self-test. The MCP shims are included on purpose: an MCP
shim runs no CLI commands of its own, but the workflow it invokes still shells
out through Bash agents, so `rdmBin` remains meaningful there.

The `rdm-autopilot` shims were originally in that list only because of the
fail-closed rule on the one `rdm-wf-dispatch-phase` call payload. Their own
drive-loop prose has since been de-literalized too (phase 10 of
`project-agnostic-lane`): the skill parses an **optional** `--rdm-bin <path>`
(there is no pre-flight stop for it — the missing-roadmap-slug stop is unrelated
and stays) and an optional `--project <name>`, and threads them through every
Bash step it runs itself, as well as into both the `rdm-wf-estimate` and
`rdm-wf-dispatch-phase` payloads. `verify-skill-autopilot.sh` bounds both
directions — it asserts each payload carries `rdmBin`, and asserts the skill
carries zero binary/project literals of its own.

#### The other two engines: `rdm-wf-review-refute-fix` and `rdm-wf-estimate`

Both now honor this contract, reusing the same three helpers (copied in shape,
since the runtime cannot import) rather than re-deriving it. `rdm-wf-estimate` takes
`rdmBin`/`project` through `parseEstimateArgs` — resolved **after** its
pre-existing required-roadmap throw, so the actionable "a roadmap slug is
required" message survives for the far more common mis-invocation — and threads
a `cfg` into `buildEstimateListPrompt` / `buildEstimateWritebackPrompt` /
`buildEstimateTierPrompt` plus its driver's own `model resolve` call and
per-phase rate directive. `rdm-wf-review-refute-fix` builds its `cfg` in the standalone
code-review path and threads it into `buildDiffSignalsPrompt` and the optional
gate's status command.

**One scope difference, and it is the same contract applied where it has a
referent — not a second contract.** `rdm-wf-review-refute-fix` calls `resolveRdmBin`
inside the standalone code-review path only. Its two legacy survivors-only
shapes (`mode: 'plan'`, and `mode: 'code'` with no item identifiers) emit
**zero** rdm invocations, so there is no binary for the fail-closed rule to
guard, and requiring the arg there would break a documented
backward-compatible shape for no safety gain.
`verify-workflow-review-outcome.sh` § 6c pins both directions: the standalone
path throws without `rdmBin` before any `agent()` call, while both legacy shapes
still succeed without it and still return `{ mode, survivors, budget }`.

Rewired callers: `.claude/skills/rdm-review` (the only caller of
`rdm-wf-review-refute-fix`; its invocation prose sits ABOVE the
`gen-skill-review.sh`-stamped region, and the shipped
`skill-review-{cli,mcp}.md` templates invoke no workflow at all, so
`lib/review.mjs` is never opened) and `.claude/skills/rdm-estimate` (the only
caller of `rdm-wf-estimate.js`; `skill-estimate-{cli,mcp}.md` remains the `{proj_flag}`
prose rating loop and needs no change). Asserted per-shim by
`verify-workflow-review-outcome.sh` § 4 and `verify-workflow-estimate.sh`'s
HOIST-SHIM section, each with a planted-typo self-test; the allow-list is
asserted AS DATA by the same two harnesses' driven prompt captures (§ 6b / § 9b).

**Bounded consequence, recorded rather than absorbed.** The prose
`rdm-autopilot` skill's estimate pre-pass passes no `rdmBin` yet — that payload,
and the loop's own literals, belong to the phase that parameterizes the prose
loop — so that one call throws until it is threaded. It does **not** break the
lane: the skill's own prose already says to log a warning and continue into the
drive loop non-fatally on an estimate error, and unrated phases simply dispatch
at whatever tier `rdm next` reports. Its `rdm-wf-dispatch-phase` payload was threaded
above and is unaffected.

## autopilot contract

Autopilot is now the prose `rdm-autopilot` skill
([`docs/autonomous-loop.md`](./autonomous-loop.md)); it consumes the same
dispatch-phase OUTCOME contract below (`statusFor`/`writesCompletion`) rather
than defining its own, and its own advance/park Bash steps are what persist the
terminal status per "The core fix" below — dispatch-phase itself never does.

### The core fix: the loop advances off PERSISTED status

`rdm-wf-dispatch-phase` persists **no terminal** phase status — it does stamp the
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

### `rdm-wf-dispatch-phase` `planOnly`

`rdm-wf-dispatch-phase` accepts an optional `planOnly` arg. When set, once the plan gate
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
| `rdm-wf-dispatch-phase` | `phaseMeta` | `fetch:phase-meta` | **all-or-nothing** — see below |
| `rdm-wf-dispatch-phase` | `taskMeta` | `fetch:task-meta` | **all-or-nothing** — see below |
| `rdm-wf-dispatch-phase` | `alreadyInProgress` | `stamp:in-progress` | boolean; the caller must already have written the status |
| `rdm-wf-estimate` | `mechanicalModel` | `model:mechanical` | non-empty string |
| `rdm-wf-estimate` | `phaseList` | `estimate:list` | array |
| `rdm-wf-plan-review` | `fetched` | `fetch:roadmap` / `fetch:<kind>` | object with a non-empty `body` **and** a `tags` array of strings (roadmap kind additionally: an array `phases` whose every entry carries a non-empty `stem`, a string `body`, and its own `tags` array) |
| `rdm-wf-plan-review` | `wontFixedTexts` | `fetch:wontfix` | array |
| `rdm-wf-plan-review` | `mechanicalModel` | `model:mechanical` | non-empty string, **all-or-nothing with `findModel`/`verifyModel`** — see below |
| `rdm-wf-plan-review` | `findModel` | `model:mechanical` | non-empty string, **all-or-nothing with `mechanicalModel`/`verifyModel`** — see below |
| `rdm-wf-plan-review` | `verifyModel` | `model:mechanical` | non-empty string, **all-or-nothing with `mechanicalModel`/`findModel`** — see below |
| `rdm-wf-backlog` | `mechanicalModel` | `model:mechanical` | non-empty string |
| `rdm-wf-backlog` | `report` | `fetch:report` | object carrying all four signal arrays |
| `rdm-wf-document` | `mechanicalModel` | `model:mechanical` | non-empty string |
| `rdm-wf-document` | `roadmapMeta` | `fetch:roadmap-meta` | object with `found === true` and an array `phases` |
| `rdm-wf-review-refute-fix` | `diff` | `diff:signals` | object with an array `changedFiles` |

`rdmBin` is **not** a hoist — it is an environment arg, and it is optional: an
absent value falls back to a plain `rdm` on `PATH` (see "Environment args"
above). `project` is optional too and is likewise an environment arg rather than
a mechanical-agent hoist.

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

### `rdm-wf-plan-review`'s `mechanicalModel` / `findModel` / `verifyModel` are all-or-nothing

The runtime-entry bootstrap in `rdm-wf-plan-review.js` accepts a hoisted model
trio only when `mechanicalModel`, `findModel`, **and** `verifyModel` are all
non-empty strings; a partial hoist (e.g. `mechanicalModel` alone) is discarded
wholesale and the bootstrap `model:mechanical` agent resolves all three from
scratch — the same rationale as `phaseMeta`/`taskMeta` above: the fetch agent
it replaces resolves all three ids in one call, so a partial hoist would still
need a model-resolving agent and saves nothing. The local
`.claude/skills/rdm-plan-review/SKILL.md` shim hoists all three together (see
its `mechanicalModel`/`findModel`/`verifyModel` bullet).

### `rdm-wf-plan-review`'s `fetched` is structured-keys-only

`parsePlanArgs` reads `fetched` / `wontFixedTexts` / `mechanicalModel` from
**structured object keys only** — never from the `$ARGUMENTS` flag string, which
would let a raw prose target string masquerade as a fetched payload. This hoist is
the one in the set that is not a pure cost question: the agents it replaces have
twice transcribed junk over real plan tags in production, and `agent(..., { schema })`
provably cannot catch it (both corrupt returns were schema-valid). See
[`docs/mechanical-agent-inventory.md`](mechanical-agent-inventory.md) §
"The hoist with a recorded correctness failure". Driver-side validation of a
*hoisted* payload's content is still out of scope for `fetched` specifically —
that path bypasses the agent entirely, so there is nothing for the fetch-side
guards below to run against. What task `fix-plan-review-gate-tag-clobber`
landed instead is identity/collision validation of the **fetch agent path** —
see "The fetch stage is now raw-transcript-capture + driver-side parse" below,
now further hardened by `fetchTranscriptionOk`'s body-content-blind check (same
section, bottom) — content validation of the caller-hoisted path above remains
the one deliberately-untouched exception.

`hoistedFetchedOk` is nonetheless held to be **no weaker than the shape
`buildReviewUnits` requires**: a `body`, and a `tags` array of strings —
`required`, all-or-nothing, per phase entry as well. That is not shape
pedantry — the gate writes the list back with `rdm ... update --tags
"<list>"`, and `--tags` **replaces** the whole list. A payload accepted with
no `tags` would be defaulted to `[]` by `buildReviewUnits` and issued as
`--tags ""`, wiping every real tag the item carried. Rejecting it costs one fetch
agent; accepting it costs the item's tags.

### The fetch stage is now raw-transcript-capture + driver-side parse

The `fetch:roadmap` / `fetch:<kind>` mechanical agents used to be handed a
composed-JSON schema (`PLAN_TARGET_SCHEMA` / `ROADMAP_TARGET_SCHEMA`, since
removed) and asked to fill it in from `rdm ... show --format json` — which is
exactly the shape that let an agent fabricate a schema-valid but unrelated
response (the two corruptions above). `PLAN_TARGET_SCHEMA` /
`ROADMAP_TARGET_SCHEMA` are gone; every fetch agent now satisfies a single
`RAW_STDOUT_SCHEMA` (one `transcript` string field) and is instructed to
transcribe the command's raw stdout verbatim — copy, not compose. This is the
closest achievable equivalent to a literal headless Bash call while staying at
**one agent per target**: the Workflow runtime has no `process`/`child_process`
(see "Import spike" above), so the agent stays in the loop as the thing that
actually runs the command, but it is reduced to a mechanical transcriber with
no JSON-composition step left to fail at. All parsing, field extraction, and —
new — identity/collision validation (a phase stem colliding with the roadmap's
own slug, a duplicate stem, or a phase block's own `roadmap` field disagreeing
with the one under review) happen driver-side, in
`.claude/workflows/lib/plan-review.mjs`'s `parseTranscriptBlocks` /
`parseJsonStdout` / `extractRoadmapFromJson` / `extractPhaseFromJson` /
`extractTaskFromJson`. The roadmap fetch keeps its existing single-turn,
multi-command shape (one `roadmap show` call, then one `phase show` call per
phase found) — it does **not** become a `parallel()` fan-out of one agent per
phase; see `docs/mechanical-agent-inventory.md`'s "must not be reintroduced"
note, which this design is held to.

**Update (`fix-plan-review-gate-tag-clobber` continued).** The identity/collision
checks above still leave one gap: they trust the agent's transcription the
moment it satisfies them, with no further content check. `fetchTranscriptionOk`
(`.claude/workflows/lib/plan-review.mjs`, next to `hoistedFetchedOk`) closes it
with a further, body-content-blind guard — rdm's own `phase-<N>-` phase-stem
convention plus a small closed `RESERVED_FETCH_TOKENS` list drawn verbatim from
both recorded incidents' own fabricated tags — applied ONLY to this
agent-transcribed fetch path, never to the caller-hoisted `fetched` payload
discussed above. A failing check triggers ONE bounded retry (a fresh,
independent `agent()` call) before falling into the existing fail-closed
`fetchFailed` path; see `docs/mechanical-agent-inventory.md` § "The hoist with
a recorded correctness failure" for the full account and
`scripts/verify-workflow-review.sh` §7g/§7h for the regression coverage
(both recorded corruption payloads replayed as negatives, a retry-recovery
positive, the empty-phases/body-text-mimicry non-tripping cases, and a
four-target-type sweep).

**Why this closes the "Workflow-driven via Bash" directive, not merely
approximates it.** `fix-plan-review-gate-tag-clobber`'s own follow-up
directives asked for the fetch stage to become "Workflow-driven via Bash, not
agent-driven, to the extent the Workflow runtime allows," with an explicit
fallback: "if a literal Bash call from the Workflow script is infeasible —
[the Workflow] should validate the agent's output as strictly as a genuine
call would allow." A literal call is not a design choice to weigh — it is
categorically unavailable. The "Import spike" table above tested every code
and process-execution vector the runtime exposes (`import()`, `eval`, `new
Function`, `require`, `process`, `Deno`, `Bun`, `fetch`) and the enumerated
global scope (`log, phase, console, budget, setTimeout, clearTimeout, Date,
agent, parallel, pipeline, workflow, args`, plus pure JS built-ins) contains no
`Bash`, no `exec`, and no I/O primitive of any kind — `agent()` is the *only*
channel from a Workflow script to any tool, including Bash. "To the extent the
runtime allows" therefore bottoms out at: the Workflow script cannot invoke
Bash itself, so the only remaining lever is *what the agent it dispatches is
permitted to do*, and that lever is pulled all the way. `fetch:roadmap` /
`fetch:<kind>` run under `agentType: 'rdm-mechanical'`
(`.claude/agents/rdm-mechanical.md`), whose `tools:` frontmatter is hard-set to
exactly `Bash, StructuredOutput` and confirmed enforced from inside a Workflow
run (`docs/workflow-schemas.md` § "agentType / effort options spike": case B's
recorded `toolNames` was exactly `["Bash", "StructuredOutput"]`, not the
default agent's nine). The dispatched agent is thus not "an LLM composing plan
data" but a sandboxed process with no capability except running the one
command named verbatim in its prompt and copying stdout into a single
`transcript` string (`RAW_STDOUT_SCHEMA` — no other field exists to compose
into). Combined with the fallback the directive itself authorizes — driver-side
validation "as strictly as a genuine call would allow," landed as
`fetchTranscriptionOk` plus the identity/collision checks and the bounded
retry-then-fail-closed loop above — every clause of the directive is satisfied
by the mechanism actually available, not worked around. There is no further
"more Bash-driven" state to move to inside this runtime; the remaining gap
between this design and a literal shell call is the V8 isolate boundary
itself, which is enforced host-side and cannot be crossed by any workflow
script, present or future, short of a runtime change tracked outside this
repo's control.

**Update (`fix-plan-review-gate-tag-clobber` continued — the gate writes from
a pre-fetch cache).** A separate literal gap survived the closures above: the
phase body's own "Implementation constraints" asked for the gate to "cache the
item's real tags before the fetch runs, then filter and write back the
filtered ORIGINAL tags — never the fetched tags," and the write instead read
`u.tags`, `buildReviewUnits`' own copy of the fetched tags threaded through the
review-unit object for the review pipeline's benefit. `snapshotOriginalTags`
closes this: called once, immediately after `fetched` is accepted and before
`buildReviewUnits` runs, it caches every unit's real tags into a dedicated
map that the gate write reads exclusively. This is a structural fix, not a
new trust source — it does not (and, while holding to the "one fetch call per
target" cost commitment above, cannot) make the write independent of
`fetchTranscriptionOk`'s own correctness, since the snapshot and the review
units are still both built from the same validated `fetched`. What it removes
is any dependency on `buildReviewUnits`/`reviewUnit`/the review pipeline
themselves for what gets written. The still-accurate claim above stands
unchanged: content validation of the **caller-hoisted** `fetched` payload
remains explicitly out of scope — only the agent-fetch path's write mechanics
changed. See `docs/mechanical-agent-inventory.md`'s matching update (same
heading) for the full account and `scripts/verify-workflow-review.sh`
§5b-cache / §5b-exec / §5b-mut(ix) for the regression coverage.

### `rdm-wf-dispatch-phase` absorbs its diff instead of hoisting it

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

- **`rdm-wf-dispatch-phase`, `rdm-do --auto`** — supplied by the *distributed*
  skill shims (`rdm-core/src/templates/skill-{dispatch-phase,do}-{cli,mcp}.md`)
  and their local copies.
- **`rdm-wf-plan-review`, `rdm-wf-backlog`, `rdm-wf-document`, `rdm-wf-review-refute-fix`, `rdm-wf-estimate`** — supplied
  only by this repo's **local** `.claude/skills/*/SKILL.md` dogfood copies. Their
  distributed templates are not yet Workflow shims; converting them is tracked by task
  `convert-remaining-skill-templates-to-workflow-shims`.
- **MCP shims** hoist only what their tool surface produces. There is no MCP
  model-resolve tool, so `mechanicalModel` — and, by the all-or-nothing rule,
  `phaseMeta`/`taskMeta` — are omitted there and the in-workflow agent runs.
