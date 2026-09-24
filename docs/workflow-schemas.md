# Workflow-tool orchestration: conventions & schema contracts

The autonomous lane of rdm's tooling is expressed as **Claude Code Workflow-tool
scripts** under `.claude/workflows/`, a sibling of `.claude/skills/` and
`.claude/hooks/`. This document defines the conventions those scripts follow and
the canonical schema contracts they exchange.

> **Scope:** mostly dogfood-only, with one emitted exception. ONE workflow
> script — `rdm-wf-review-refute-fix.js` — IS
> emitted by `rdm agent-config claude --skills --out <dir>`, byte-identical to
> this repo's own `.claude/workflows/` copy, under `<dir>/.claude/workflows/`
> (Claude-only, `--out`-only — see `CHANGELOG.md`). That emitted engine is
> **project- and binary-agnostic**: it names no particular rdm executable and no
> particular rdm project, because both arrive as runtime arguments (see
> § "Environment args: `rdmBin` and `project`"). Byte-identity with this repo's
> copy is a CONSEQUENCE of that design, not a limitation of it, and
> `scripts/verify-agent-config-distribution.sh` § 7 gates the claim by emitting
> into a hermetic non-rdm, non-Rust fixture repo and then executing the emitted
> engine's pipeline logic — and the rdm command ladders it builds — there.
> `rdm-wf-dispatch-phase.js` was a second emitted engine until
> `agent-orchestrated-dispatch` phase 7 retired it (see
> `docs/workflow-vs-prose-boundary.md` § "Retirement record"); every reference to
> it below is historical. The
> unshipped set is now exactly: `lib/*.mjs` (no regeneration script travels
> downstream to consume it) and the generator scripts
> (`scripts/gen-workflow-review.sh` and friends). rdm's shipped autonomous skills
> (`rdm-core/src/templates/skill-{autopilot,dispatch-phase}-cli.md`, and the
> `--auto` section of `skill-do-cli.md`) are the user-facing autonomous
> lane: `skill-autopilot-cli.md` is a **prose** skill that itself
> drives the roadmap loop, entering the prose `rdm-dispatch-phase` orchestrator with
> `Skill` and (locally) invoking `rdm-wf-estimate`
> as an ordinary `Workflow` call rather than being a thin shim over a workflow
> script of its own — see `docs/workflow-vs-prose-boundary.md` for why autopilot
> was retired from `.claude/workflows/` in favor of prose. `skill-dispatch-phase-cli.md`
> is likewise prose: the per-phase procedure itself, whose plan-review and
> code-review stages are `Workflow` calls. Distributing the
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
drives it — so `rdm-review` (the skill) and `rdm-wf-review-refute-fix`
(the engine it invokes) are distinguishable at a glance. A `lib/*.mjs` is a
shared source module, never a listing entry, so its name is deliberately
unprefixed and frozen. (`spike-agent-type.js` used to be an exempt bare-named
spike artifact; it was deleted along with the mechanical lane it probed, so every
`.claude/workflows/*.js` now carries the prefix.)

### Determinism: no `Date.now()`/`Math.random()`

Every `.claude/workflows/*.js` script — all five of them, every one now
shipped downstream (`agent-orchestrated-dispatch` phase 26 registered
`rdm-wf-backlog.js`, `rdm-wf-document.js`, `rdm-wf-estimate.js` and
`rdm-wf-plan-review.js` alongside `rdm-wf-review-refute-fix.js`) — must not
call `Date.now()` / `Math.random()`. The reason is determinism of the pipeline
GENERALLY, not any one downstream consumer of it: the Rust workflow tests
(`workflow_review`'s `pipeline::output_deterministic_{code,plan}` and
`engine::driver_deterministic_and_emits_no_done_trailer`; `workflow_passes`'
`backlog::output_deterministic` and `estimate::engine_output_deterministic`)
assert byte-identical output on identical input as its own reproducibility
contract. Resume-cache validity (see
[`docs/autonomous-loop.md`](autonomous-loop.md) § "Recovering a crashed
run") is ONE consequence of that determinism, not the sole or primary
reason for the rule: a call whose `(prompt, opts)` pair is not reproducible
can never safely replay from a cached result, but the rule exists to keep
every workflow's output reproducible — and its harnesses' byte-identical
assertions meaningful — even in scripts no resume attempt ever touches.

This is a **repo convention, not a runtime restriction** — the global-scope
table above lists `Date` and `Math` as present in the isolate; nothing in the
runtime itself stops a script from calling `Date.now()`. The grep-based harness
checks that once enforced it were retired with those harnesses
(`rust-test-suite-consolidation` phase 3); the determinism tests above are the
behavioural form.

### Observing the rendered listing

The prefix only pays off if the **rendered** listing shows it, and that listing
is produced by the Claude Code client from `.claude/`, not by anything in this
repo — so no hermetic check here can confirm it. What this repo gates
hermetically (`rdm-core/src/agent_config.rs`,
`generate_workflows_are_byte_identical_to_source`) is that the shipped template
copies stay byte-identical to the local engines; the old filename-set listing
check went with `verify-workflow-review.sh`. Confirming the client agrees is a
separate, deliberate step:

```sh
scripts/observe-workflow-listing.sh                  # live capture + assert; not CI-run
```

The script derives the expected names from the tree (never a hardcoded list),
spawns a fresh `claude -p` rooted at this repo, and asserts four things against
what comes back: every declared engine renders under its `rdm-wf-` name, no
bare pre-rename name survives, every `rdm-*` front door still renders under its
original name, and nothing is double-prefixed.

**It discriminates.** Run against `main` and against the renamed tree at the
same moment on the same machine, the identical command returned different
listings — bare `backlog`, `dispatch-phase`, `document`, `estimate`,
`plan-review`, `review-refute-fix` in the first, and the `rdm-wf-`-prefixed
entries in the second, with all eleven `rdm-*` front doors unchanged in both
and no double-prefixed entry (a doubled `rdm-`, or the engine prefix stacked
in front of a front-door name) in either. The cwd is the only variable, so
the listing genuinely tracks the tree.

A prior hermetic `--self-test-only` half asserted the same discrimination
against a pinned pre-rename listing rather than a real capture — no `claude`
CLI, no network, nothing real involved. It was retired by the operator
amendment to the `retire-static-grep-harnesses` plan (2026-09-23), which
extended grep-only-harness retirement to workflow JS/skill-template source;
the since-deleted `verify-workflow-review.sh` § 2d stopped invoking it.

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
| `docs/token-baseline.json`'s bare per-engine record keys, and `docs/token-baseline.md`'s lane tables | **A frozen measurement corpus.** The figures are keyed to those names as recorded; rewriting them would invalidate `rdm-measure refuter-severity --audit` (gated by the `audit_committed_baseline_ok` test). |
| `autopilot.js`, `lib/autopilot.mjs`, and the `autopilot` Workflow name | **Retired, with no successor.** `rdm-autopilot` survives as a prose skill with no engine behind it, so `autopilot` must never be prefixed — doing so would corrupt the one front door the rename must leave untouched. `scripts/verify-agent-config-distribution.sh`'s self-test D depends on `autopilot` naming a Workflow that does not resolve. |
| Historical `CHANGELOG.md` entries | Descriptions of the pre-rename world; correct as written. |

`scripts/verify-workflow-review.sh` § 2e used to run the seven anchored
reference-form greps over the tree and fail on any hit outside that allowlist,
with a planted-mutation self-test; it was retired by the operator amendment to
`task/retire-static-grep-harnesses` (2026-09-23), so the survivors above are
now held by convention and code review.

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
   the retired `rdm-wf-dispatch-phase.js` (plan/implement) and `lib/review.mjs` (finders)
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
omitted" row above), while the then-sibling `rdm-wf-dispatch-phase.js` threaded
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
(`resolve review-verify` → opus @ high, but `--tier medium` → opus @ medium; see [`model-profiles.md`](model-profiles.md)). Review sizing is
core's to own. Both rules are gated by `rdm-cli`'s `model resolve` tests; the
retired `scripts/verify-workflow-dispatch.sh` also gated them (AC-MODEL, AC-TIER)
over the engine's JS, with planted-mutation self-tests.

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
drifted from the source, and the Rust `workflow_review` generator tests
(`cargo nextest run`, and so CI) run that check. Editing happens in the lib; consumers are regenerated, never hand-edited.

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
| `.claude/agents/rdm-mechanical.md` | The custom agent definition. Minimal system prompt, `tools: Bash, StructuredOutput`. **Nothing under `.claude/workflows/` references it any more** — the `no-mechanical-agents-in-workflows` phase removed every mechanical call site — but the definition is kept as a registry entry and is still emitted downstream by `rdm agent-config claude --skills`. |
| `.claude/workflows/spike-agent-type.js` | **DELETED** by the `no-mechanical-agents-in-workflows` phase. Its whole subject was `agentType: 'rdm-mechanical'` and `effort:` on a mechanical call site, and there are no mechanical call sites left; carving an exemption into the rule for it would have been the wrong trade. Its results are the tables in this section, which stand. |

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
the lane-token measurement (`rdm_devtools::measure::sidecar`, originally `scripts/lib/token-report.mjs`) computes per agent, so it is directly comparable to
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
§2b (since deleted) forbade `effort:` anywhere under `.claude/workflows/` except the spike —
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
`agentType: 'mechanical'` been threaded into the then-shipped engines
(`rdm-wf-dispatch-phase.js`, `rdm-wf-review-refute-fix.js`) and re-synced into
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
`scripts/verify-workflow-review.sh` §2c (since deleted) asserted this **bidirectionally** — every
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
`lib/plan-review.mjs:1856` fans out `reviewUnit`, which dispatches only judgment
agents; the act/gate half runs in a plain sequential `for` loop *after* that
barrier (`gate:clear-tag` at `:1960`; these two line numbers shift as the file
grows — re-locate by content, not by number, if they drift again).
`rdm-wf-estimate.js` has the same shape —
`parallel()` fans out the judgment `estimate:rate:*`, while the mechanical
`estimate:write:*`/`tier:*` follow sequentially. The corpus corroborates it
independently: across the eight multi-gate plan-review runs, **zero** of the 54
possible gate-agent pairs have overlapping execution windows.

So exactly one mechanical call site in the tree is dispatched through
`parallel()`: `gather:<stem>` in `rdm-wf-document.js`, via
`parallel(phases.map((p) => () => gatherPhase(p)))`. That set is now
machine-checked — `scripts/verify-workflow-review.sh` §2c(v) (since deleted) pinned it and failed if
it changed, so a future refactor cannot silently move a mechanical site into a
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

`scripts/verify-workflow-review.sh` §2b-fid (since deleted) gated that the instrument stayed
correctly *built* — coverage, pairing, discrimination (each write shape carries
an instance whose correct answer is `ok: false`, so a constant-answer guess
cannot score a false pass), throwaway roots required rather than defaulted since
two shapes write to a plan repo, and command validity. Each of the four has a
planted-mutation self-test.

### Planner/implementer effort route spike (phase 4, model-effort-profiles)

**2026-09-23.** The section above measured `agentType`/`effort` at the *mechanical*
`agent()` call sites the retired `spike-agent-type.js` dispatched from a
`Workflow` script. This spike asks a narrower, still-open question: can
`rdm-dispatch-phase`'s two **non-Workflow** roles — the planner (step 5) and the
implementer (step 10), each dispatched with the `Agent` tool from the main
session, not from a Workflow script — be given a reasoning-effort profile at
all? Every case below uses the trivial probe prompt `Reply with the word ok.`,
run from a fresh `mktemp -d` scratch cwd so its transcript is isolated under its
own `~/.claude/projects/<slug>/` directory, on `claude-opus-5-5` (this
environment's ambient session default effort recorded on every control case is
`"medium"`, not the `"high"` the § "agentType / effort options spike" 156k-record
corpus found — a different measurement window/config, noted here because it is
the baseline every row below is read against, not because it changes any
verdict: every comparison here is a same-run paired control/treatment, not a
cross-corpus one).

**Setup correction (Route 2):** the approved plan's `.claude/agents/effort-probe.md`
template carried no `name:` frontmatter key. Dispatched verbatim, the first
attempt (scratch cwd `tmp.kgyrVP7U8s`) raised `Agent type 'effort-probe' not
found. Available agents: claude, Explore, general-purpose, Plan,
statusline-setup` — the same failure mode § "agentType / effort options spike"
Q1a retracted, but for a different, verifiable cause this time: the directory
existed before the session started (so the documented restart caveat does not
apply), and this repo's own live `.claude/agents/rdm-mechanical.md` definition
does carry a `name:` key. Adding `name: effort-probe` to the frontmatter and
re-dispatching from a fresh scratch cwd resolved the agent type cleanly (see
Route 2 below) — recorded here as a setup-bug fix, not a deviation from what the
spike measures.

**Results:**

| Case | Declared | Recorded effort | Verdict | Transcript path |
|---|---|---|---|---|
| Route 1 — control (`claude -p` default agent) | *(none)* | `medium` | baseline | `~/.claude/projects/-private-var-folders-wh-d1mw3dm11z1-pglt1-w9t0mw0000gn-T-tmp-i8W8UPAiPT/b3784614-74a5-47e4-ab11-7463aa662c2a.jsonl` |
| Route 1 — treatment (`--agents '{"effortprobe":{...,"effort":"low"}}' --agent effortprobe`) | `effort: "low"` | `medium` | **NOT HONORED** — accepted (no throw), transcript unchanged from control | `~/.claude/projects/-private-var-folders-wh-d1mw3dm11z1-pglt1-w9t0mw0000gn-T-tmp-19E0rh50DC/4528a9e6-fca6-4123-a5f6-ffde4c71f08f.jsonl` |
| Route 2 — setup attempt, no `name:` key | `effort: "low"` in `.claude/agents/effort-probe.md`, no `name:` | n/a — dispatch never resolved | **INVALID SETUP** — `Agent type 'effort-probe' not found`; no child transcript exists | `~/.claude/projects/-private-var-folders-wh-d1mw3dm11z1-pglt1-w9t0mw0000gn-T-tmp-kgyrVP7U8s/` (top-level session only, no `subagents/` dir) |
| Route 2 — control (`subagent_type: 'general-purpose'`, no custom definition) | *(none)* | `medium` | baseline | `.../tmp-H3SADOGNZs/9bdff87f-2392-4284-9850-550075f573a3/subagents/agent-acdd6196ca3ab80e4.jsonl` |
| Route 2 — treatment (`subagent_type: 'effort-probe'`, `name:` key added) | `effort: "low"` in `.claude/agents/effort-probe.md` | `low` | **HONORED** | `.../tmp-njARIOwPYb/f67c1c8b-379d-40ff-a794-022ae9b1c0b0/subagents/agent-a997f4247016a72af.jsonl` |
| Route 3 | — | — | **SKIPPED** — stop condition reached at Route 2 | n/a |
| Route 4 | — | — | **SKIPPED** — stop condition reached at Route 2 | n/a |

Route 2's validity check passed: the treatment child's
`agent-a997f4247016a72af.meta.json` records `"agentType":"effort-probe"`
verbatim, and the top-level run's `subagent_stats.by_type` shows
`{"effort-probe":1}` — the dispatch really resolved the custom definition
rather than silently falling back to `general-purpose`. The paired control,
dispatched from a sibling scratch cwd with no `.claude/agents/` directory at
all, recorded the ambient `medium` on its own child transcript, so the `low` on
the treatment child is attributable to the agent definition's `effort: low`
frontmatter key, not to session-level drift between the two runs.

**Route 3 (Workflow `agent()`), cost write-up — read for context, not exercised
here.** Per the plan, Route 3 is tested only if no earlier route is honored; it
is not, since Route 2 is. It is already established (§ "agent() options spike")
that `agent(prompt, { effort })` called from a `.claude/workflows/*.js` script
*is* honored. Adopting that mechanism for the planner/implementer would mean
converting `rdm-dispatch-phase/SKILL.md` steps 5 and 10 from `Agent`-tool
dispatch to a `Workflow({ scriptPath: ... })` call, which the skill's own "How
this skill must be entered" section documents as reachable *only* from the main
session holding the `Workflow` tool — the same constraint step 6 (plan review)
and step 12 (code review) already live under. Costs of that move, judged from
reading those two steps and the delegation-boundary table above them: (1)
*tool access* — a Workflow-dispatched `agent()` call still spawns the same kind
of subagent the `Agent` tool does, but the *script* wrapping it runs in a
sandboxed engine that cannot `import`/`require` (documented above, § "Import
spike") and has no read/edit tools of its own — every read/write the
planner/implementer needs would have to move into the dispatched agent's own
prompt, as it already does for `Agent`-tool dispatch, so this cost is neutral;
(2) *fire-and-forget/control-flow semantics* — a Workflow call is a single
synchronous return once the sub-pipeline completes, unlike `Agent`'s
notification-driven background launch that step 5/10 already drive to
convergence "on each notification"; folding the planner and implementer into a
Workflow script would trade a documented, working async pattern for a new
one-shot script that would need its own retry/timeout handling authored from
scratch; (3) *context passed* — identical either way: item body, plan body,
`identity.path` as working directory; (4) *coupling* — this is the real cost.
The planner and implementer are currently prose steps in
`rdm-dispatch-phase/SKILL.md`, editable and reviewable as plan text; moving them
into `.claude/workflows/lib/*.mjs` would make them Workflow-authored JS,
subject to the stamped-copy generator/drift-gate discipline every other engine
module here carries (`scripts/gen-workflow-*.sh --check`), and would cross the
project's own workflow-vs-prose boundary rule (`docs/workflow-vs-prose-boundary.md`):
planning and implementing are judgment, which that rule reserves for prose, not
for a deterministic Workflow script. **Recommendation: not practically
adoptable for phase 6.** The coupling cost is real and the tool-access/context
costs are neutral at best, and Route 2 already resolves the actual gap through
the tool these two roles already use, with no boundary crossing at all.

**Phase 6's route: Route 2** — a per-role custom agent-definition (`.claude/agents/<name>.md`
with an `effort:` frontmatter key and a `name:` key matching the intended
`subagent_type`), selected per dispatch via the `Agent` tool's `subagent_type`.
This composes directly with `rdm-dispatch-phase` steps 5 and 10, which already
dispatch the planner and implementer as named `Agent` calls — phase 6 need only
add a resolved-tier `subagent_type` (backed by a generated or maintained
per-profile agent definition) to each of those two calls, with no change to the
tool used, the delegation boundary, or the async dispatch-and-converge pattern.
Routes 3 and 4 were not exercised (stop condition reached at Route 2, per the
approved protocol).

**Resolution (phase 6).** The earlier "no workflow script may pass `effort:`"
rule is retired. The two review engines (`rdm-wf-review-refute-fix`,
`rdm-wf-plan-review`) now take `findEffort`/`verifyEffort` next to
`findModel`/`verifyModel` and pass `effort:` on every finder (including its
retry) and refuter `agent()` call **only when one is supplied** — an absent
effort adds no key, since `effort: undefined` was never measured inert — and
refuse a value outside `low|medium|high|xhigh|max` before any agent runs
(`scripts/lib/review-effort.test.mjs`). The planner and implementer take Route
2 as five role-agnostic definitions, one per effort (`rdm-effort-low` …
`rdm-effort-max`, each with `name:` and `effort:`, no `model:` and no `tools:`),
dispatched as `subagent_type: rdm-effort-<effort>` plus the resolved `model:`.
Per-effort rather than per-role×effort because effort is the only thing the
definition must carry: the per-call `model` overrides a definition model. They
ship on `--skills` (`.claude/agents/`) and in the plugin (`agents/`, loaded as
`rdm:rdm-effort-<level>`). The other engines (`rdm-wf-estimate`, `-backlog`,
`-document`) still pass no effort.

### Orchestrator / Workflow-reachability spike (can an Agent subagent drive the review workflows?)

`agent-orchestrated-dispatch` phase 1. The whole roadmap assumed a prose per-phase
orchestrator, itself dispatched by `rdm-autopilot` as an **Agent-tool subagent**, could call
the `Workflow` tool for `rdm-wf-plan-review` / `rdm-wf-review-refute-fix` and receive their
completion. That assumption is FALSE. Three questions, four probes, all answered from run
evidence rather than self-report.

**Q1 — can an Agent-spawned subagent invoke `Workflow`? NO, structurally.**

The tool is absent from an Agent subagent's tool list entirely — neither loaded nor deferred.
A subagent's surface is `Agent, Artifact, Bash, Edit, Read, Skill, ToolSearch, Write` loaded,
plus `EnterWorktree, ExitWorktree, Monitor, NotebookEdit, SendMessage, TaskStop, WebFetch,
WebSearch` deferred. `ToolSearch("select:Workflow")` returns `No matching deferred tools
found`, so the call cannot even be **formed**. This is an absent capability, not a refusal:
there is no opt-in, permission mode, or prompt wording that reaches it.

Reproduced on two models (see Q2), so it is not a model-specific tool surface.

**Q1b — does the Skill route reach the engine instead? NO.** Each workflow script is also
surfaced as a same-named Skill (`rdm-wf-review-refute-fix`, and `rdm:`-namespaced under the
plugin), which makes it *look* reachable from a subagent. It is not. Invoking that skill
returns a harness-generated shim whose entire payload is the directive
`Invoke: Workflow({ name: "rdm-wf-review-refute-fix" })` — unsatisfiable for an agent with no
`Workflow` tool. No engine runs, no run id is issued. The shim is generated by Claude Code
from the workflow's `meta`, not authored in this repo: there is no `SKILL.md` for any
`rdm-wf-*` engine under `.claude/skills/` or `plugins/rdm/skills/`.

**Independent corroboration.** The workflow sidecar (`~/.claude/projects/**/workflows/wf_*.json`)
held 11469 runs before the probes and 11469 after. No probe created a run by either route, so
no probe's negative answer rests on its own say-so.

**Q2 — can the orchestrator's model be pinned independently of the phase tier? YES.**

Read from each subagent transcript's `message.model` field, not from what the agent says about
itself:

| Probe | `model` passed to `Agent` | Observed `message.model` |
|---|---|---|
| Q1 control | none (inherit) | `claude-opus-5` |
| Q2 treatment | `sonnet` | `claude-sonnet-5` |
| Q3 yield-early | none (inherit) | `claude-opus-5` |
| Q1b skill-route | none (inherit) | `claude-opus-5` |

An omitted `model` inherits the parent's; an explicit one is honored. Triage can therefore be
pinned to a strong tier independently of the phase's own `next.model`. Note the self-report
channel agreed here, but it is derived from the subagent's own system prompt and is not the
instrument.

**Q3 — does the yield-early failure reproduce? YES, and it is structural, not a bug.**

The `Agent` tool does not block. It returns in well under a second with a background-launch
acknowledgement — "The agent is working in the background. You will be notified automatically
when it completes… continue other work or respond to the user in the meantime." The parent has
a complete turn available at that instant and nothing forces it to stay, so a parent that
simply answers will return with no child result at all.

The probe obtained its child's result only by *voluntarily* spending tool rounds, and the real
result arrived out-of-band as a `task-notification` on a later round rather than as the `Agent`
call's return value. Note also that the sanctioned wait primitives are themselves async: the
Bash tool blocks foreground `sleep` and points at `Monitor`, which is itself notification-based.

**Mitigation phase 6 must use:** treat parent→child as fire-and-forget and drive the child
forward on each completion notification until it converges. This confirms the behavior recorded
in `docs/autonomous-loop.md`; the loop is correct, but it is a *convention the parent must
honor*, never an invariant the tool enforces.

**Decided orchestrator shape.** The per-phase orchestrator **cannot be an Agent-spawned
subagent**, because the unit that calls the review workflows must be one that holds the
`Workflow` tool, and only the main session loop does. Phase 6 is therefore written against the
fallback the phase-1 body already named: orchestration stays in the main-session prose loop
(the shape `rdm-autopilot` already uses — pick a phase, invoke the workflow, read the OUTCOME,
persist status, advance), and any work delegated to an Agent subagent must be work that needs
no `Workflow` call. Splitting the orchestrator across the boundary — subagent decides, main
session calls Workflow on its behalf — is possible in principle but costs a notification
round-trip per review and gives the subagent no way to act on the result it asked for; it is
recorded here as considered and not chosen.

Probe run date 2026-09-12. Evidence lives in this session's subagent transcripts
(`subagents/agent-*.jsonl`), read for `message.model` and tool-surface reports; the probes
created no workflow runs, which is itself the Q1 evidence.

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
| `location`      | string                                   | `file:line`, section heading, or phase stem — free-text, may carry extra human-readable detail |
| `path`          | string                                   | **optional, code mode**; a STRUCTURED repo-relative source path the `quote` was taken from, distinct from `location`. Required alongside `quote` by prompt convention (not schema-enforced), and the prompt asks for a BARE path with no `:line` suffix — but `persistAnchorFor` tolerates one anyway, stripping it via `stripPathLineSuffix` before validating (see below); a finding that carries `quote` with no usable `path` still falls back to the `location`-parsing heuristic (see `pathFromLocation`) |
| `quote`         | string                                   | **optional**; a VERBATIM excerpt of the reviewed text this finding is about — what makes a persisted comment anchorable |
| `severity`      | `blocking` \| `concern` \| `suggestion`  | required; drives ranking and the overall verdict  |
| `confidence`    | integer 0–100 (required)                 | the finder's confidence **in the finding**        |
| `what_fails`    | string (required)                        | the specific problem                              |
| `why`           | string                                   | root cause / which rule, AC, or principle         |
| `recommendation`| string                                   | concrete fix                                      |
| `unrefuted`     | `true` (post-pipeline only)              | set by `buildReviewPipeline`, never by a finder    |
| `unrefutedReason` | `'non-gating'` \| `'budget'` (post-pipeline only) | present iff `unrefuted` is; WHY it went ungraded |
| `refuterError`  | `true` (post-pipeline only)              | a refuter was dispatched and CRASHED; never combined with `unrefuted` |
| `inScope`       | boolean (post-pipeline only)             | folded from the refuter's `VERDICT.inScope` when it graded one; absent on every finding never graded for scope (plan mode, no associated plan, non-gating, over-budget, or refuter-crashed) |

**`category` is additive, optional, and read by nobody.** It exists because
`FINDINGS_SCHEMA` is `additionalProperties: false`: the `security` dimension's
prose asks a finder for a threat-category slug (`command-injection`,
`path-traversal`, `unsafe-ffi`, `hardcoded-secret`, `info-disclosure`, …), and
without a declared field the runtime would **reject** that output and silently
discard every security finding. It is deliberately NOT folded into `concern`,
which is the DIMENSION identity three consumers match on
(`classifyPlanOutcome` and `buildReviewPipeline`'s
`concern: f.concern || dim.key` backfill). The reference agent's
`(file, line, category)` **dedupe key is NOT implemented** in this pipeline — the
field is a carrier, not a half-built dedupe, and no consumer reads it today.

**`quote` is what makes a finding anchorable.** `location` is free-form prose —
`rdm review comment --quote` needs text that matches the reviewed document
EXACTLY, so the finder is asked for a verbatim excerpt instead, short enough to
be unique and never paraphrased. A finding about the document as a whole simply
omits it. The refuter VERIFIES it: `refutePrompt` appends a quote-verification
clause **only when the finding carries a quote** (a conditional that keeps the
56-item refuter-agreement corpus's recorded `promptSha256` values regenerating —
see `docs/refuter-model-tiering.md` § Maintenance gap), and an explicit
`quote_ok: false` verdict makes the pipeline `stripQuote` the finding: the
finding survives on its own merits and simply loses an excerpt that would not
have anchored. `quote_ok` is independent of `refuted`. A finding that was never
graded (`unrefuted` / `refuterError`) keeps an UNVERIFIED quote — the writer's
runtime whole-document fallback is what protects those.

**`path` is what makes a `code`-mode finding's `quote` land at BUILD time,
rather than depend on parsing `location`.** A code finding's `location`
conventionally reads `<path>:<line>`, but it is still free-text and may carry
extra human-readable detail past that (e.g. `"path/to/file.rs:12-18 (mirrored
at ...)"`) that defeats `pathFromLocation`'s end-anchored suffix-stripping
heuristic. `path` is the structured escape hatch: the code-mode finder prompt
asks for it explicitly whenever `quote` is given, and `persistAnchorFor` tries
it FIRST, ahead of the `location`-parsing fallback (see § "Persisting a
review" below). `plan` mode has no `path` field in its prompt — a plan review
targets the document itself, where a bare `--quote` is always the anchor.

**`path` itself must be BARE, with no `:line` suffix — but a suffixed one is
tolerated, not refused.** The prompt for `path` is explicit that it takes no
line suffix (unlike the adjacent `location: <path>:<line>` prompt line), but a
finder sometimes pattern-matches the two lines and tacks one on anyway (e.g.
`path: "src/foo.rs:12-18"`). `isRepoRelativePath` alone would wrongly ACCEPT
that value — it still contains a `/` — and the real binary then refuses the
resulting `--path` outright. `persistAnchorFor` therefore strips a trailing
`:<line>` or `:<start>-<end>` suffix from a declared `path` via
`stripPathLineSuffix` (the SAME regex `pathFromLocation` already applies to
`location`) before validating it. Stripping was chosen over rejecting the
whole declared value and falling back to `pathFromLocation(location)`: the
file half of a suffixed `path` is exactly the anchor the finder meant to give,
and a finder-supplied `path` is normally the MORE reliable signal — discarding
it in favor of `location` would throw away that reliability over a
self-inflicted formatting slip.

The code-mode `ac` dimension's prompt deliberately says nothing about `quote`.
It returns early from its own `AC_REVIEW_SCHEMA` branch and never reaches the
shared FINDINGS-schema prompt line, its prompt text was byte-pinned by the since-deleted
`scripts/verify-workflow-review.sh`'s `CODE_PROMPT_BASELINE.ac`, and its
`findings` array is narrative-only. Structurally `quote` is still accepted there,
because `AC_REVIEW_SCHEMA.properties.findings` aliases the same sub-schema. See
§ "Persisted review comment body" and § "Persisting a review" below.

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

### Persisted review comment body

Every comment `persistReviewCommands` writes starts with a fixed eight-line
header carrying the finding metadata rdm's comment frontmatter has no field for,
followed by a blank line, then the finding's own prose. The key ORDER is fixed
and the header is TOTAL — every key is always emitted, never sparse:

```
severity: blocking
confidence: 90
refuted: false
unrefutedReason: none
dimension: coherence
finding-id: f1
inScope: n/a
anchor: path

coherence
What fails: the retry backoff strategy is unspecified
Why: no stated rule covers it
Recommendation: state the backoff policy
```

Rules:

- The eight keys, in this order: `severity`, `confidence`, `refuted`,
  `unrefutedReason`, `dimension`, `finding-id`, `inScope`, `anchor`.
- `unrefutedReason: none` is the SENTINEL for a finding that carries none — the
  key is never omitted.
- `inScope: n/a` is the SENTINEL for a finding never graded for scope (plan
  mode, or a code-mode review with no associated plan) — the key is never
  omitted. `inScope: true` / `inScope: false` record an explicit refuter scope
  verdict (see § VERDICT and § `hasBlocking`); `parseCommentHeader` recovers
  `true` / `false` / `null` (never a bare string) from these three values.
- `refuted` is always `false`: a refuted finding never reaches the writer,
  because `survives()` already dropped it.
- `anchor` is the CLOSED `PERSIST_ANCHOR_STATES` vocabulary — the fourth
  disposition state a persisted comment can be in, distinguishing "this
  finding never asked for a file anchor" from "this finding asked and the
  request was dropped at build time" (both of which land as a bare
  whole-document comment otherwise indistinguishable from one another):
  - `path` — change target, `--path` + `--quote` both emitted (the normal
    anchored case)
  - `quote` — non-change (plan-repo document) target, bare `--quote` emitted
    (the normal case there)
  - `wholeDocumentIntended` — the finding never carried a `quote` at all
  - `degraded` — the finding carried a `quote` but the requested anchor was
    dropped, either at BUILD time (`persistAnchorFor`'s `reason` —
    `path-missing` or `outside-hunk`) or at RUN time (a build-time-valid
    `--path`/`--quote` pair the real binary still refused once the ladder
    ran — see "The all-anchors-degraded signal (AC4)" below). Both cases
    persist the SAME `anchor: degraded` header; the header does not
    distinguish which stage caused it, only the ladder's own runtime tally
    does.

  Computed by `persistAnchorState(finding, target, opts)`, which re-derives the
  SAME decision `persistAnchorFor` makes for the writer, so the header can
  never disagree with what the emitted `rdm review comment` line actually did.
- Every header VALUE is single-line (embedded newlines are collapsed to spaces),
  so the inverse parser can be line-based.
- `formatCommentBody` and its inverse `parseCommentHeader` live side by side in
  `.claude/workflows/lib/review.mjs` so the two cannot drift.
  `parseCommentHeader` returns `null` — never throws — on a body without the
  header, which is how a human-written comment is skipped rather than
  misread as a finding.

**Backward compatibility: a legacy SEVEN-key header still parses.** `anchor`
was added as a TRAILING eighth key; every comment persisted before that change
carries only the first seven (`LEGACY_PERSIST_HEADER_KEYS`). `parseCommentHeader`
tries the full eight-key match first and falls back to the seven-key match
only when that fails, so a pre-existing comment is still recognized as
machine-written — `anchor` comes back `undefined` (unknown, never guessed) —
rather than silently misclassified as an unheadered human comment, which would
defeat `priorFindingsFromReviews`'s repeat-finding detection (`lib/plan-review.mjs`)
on every review persisted before this change.

**Backward compatibility: a legacy SIX-key header still parses.** `inScope` was
added as a TRAILING seventh key (and `anchor` as an eighth); every comment
persisted before `inScope` was added carries only the first six
(`LEGACY_6KEY_PERSIST_HEADER_KEYS`). `parseCommentHeader` progressively tries
the full eight-key match, then the seven-key match, and finally the six-key
match only when both earlier matches fail, so a pre-existing comment is still
recognized as machine-written — `inScope` comes back `null` (unknown, never guessed)
and `anchor` comes back `undefined` (unknown, never guessed) — rather than
silently misclassified as an unheadered human comment, which would defeat
`priorFindingsFromReviews`'s repeat-finding detection (`lib/plan-review.mjs`)
on every review persisted before this change.

Carrying this metadata in comment FRONTMATTER instead is recorded as a
follow-up (`extend-review-comment-frontmatter-with-finding-metadata`), not done
here.

### Persisting a review (`persist: { on }`)

Both review workflows can record their surviving findings as a REAL rdm review
instead of leaving them in an ephemeral OUTCOME. It is OPT-IN and defaults OFF,
so a caller that omits it gets a byte-identical OUTCOME to before.

`persist` is read from STRUCTURED ARG KEYS ONLY — never tokenized out of an
`$ARGUMENTS` flag string. Legal shapes: absent / `false` (off), `true` or `{}`
(on; derive the ref per unit), `{ on: '<ref>' }` (on; explicit target). Anything
else throws.

**THREE ref grammars are in play and must never be conflated.** This is the one
sharp edge:

| ref | grammar | used by |
| --- | --- | --- |
| worktree ref | `<roadmap>/<phase>` or `task/<slug>` | `rdm worktree add` (`ItemRef::parse`) |
| prompt context target | free-form human-readable label | `context.target`, threaded into every find/refute prompt (byte-pinned) |
| review ref | `roadmap/<slug>` \| `phase/<roadmap-slug>/<stem-or-number>` \| `task/<slug>` \| `plan/<slug>` \| `change/<sha-or-rev>` | `rdm review --on` (`ReviewTarget::from_str`) |

`rdm-wf-review-refute-fix.js`'s existing `worktreeRef` and `reviewTarget` are
BOTH the first shape; `rdm review --on` rejects it. The persist `--on` ref is
therefore a THIRD variable, `persistReviewTarget`, and `lib/plan-review.mjs`
derives its own per unit through `persistTargetFor` (`phase/<roadmap>/<ident>` |
`task/<slug>` | `roadmap/<slug>`). A numeric phase identifier is legal —
`parse_review_target_ref` resolves `phase/<roadmap>/1` through
`resolve_phase_stem` — so it is never pre-resolved in the workflow.

The writer itself (`persistReviewCommands` in `lib/review.mjs`) treats `target`
as an OPAQUE, already-well-formed ref: no
per-kind branching, no prefixing, and a throw on a ref with no `/`. That is what
lets a future target kind reuse it unchanged.

**The writer's optional fourth argument, `opts`.** Every field is DEFAULT-OFF,
so a caller that passes no `opts` gets the plain document-target ladder that
`rdm-cli/tests/workflow_review/persist.rs` runs against the real binary. The consumer decides; the writer
still branches on nothing:

| `opts` field | effect |
| --- | --- |
| `pathAnchors` | for each survivor, resolve a repo-relative `--path`: try the finder's own structured `finding.path` first (validated through `isRepoRelativePath`), and fall back to the pure `pathFromLocation(finding.location)` heuristic only when `finding.path` is absent or invalid — then emit `--path "$RDM_PERSIST_PATH"` alongside `--quote`. Suppressed outright when `source.noCode` is set (no hunks exist, so every such comment would fail), and REFUSED with a throw on a non-change target. Against a change target `--quote` is emitted ONLY when a `--path` accompanies it — see the unanchorable-quote rule below. |

**`rdm-wf-review-refute-fix.js` defaults its code-review persist target to
`change/HEAD`** — a code review is about the code, so the recorded artifact
targets the change itself rather than the phase document. That decision lives in
the DRIVER region, not the stamped block, so a future consumer can choose a
different target without editing `lib/review.mjs`. An explicit `persist.on` still overrides it,
but only to the same change: the driver refuses a `persist.on` that names a
different artifact (`source-bound persistence cannot target a different
artifact`). See [`change-reviews.md`](change-reviews.md) for the target itself.

**The `fallbackTarget` option is gone**, along with the second command list it
built. It lived on `buildPersistReviewPrompts`, which no longer exists, and no
shipped consumer ever passed it: the source binding above makes switching a
persisted change review onto a document target forbidden POLICY rather than
merely unimplemented.

**Emitted commands**, in order: `rdm review start --on <target> --body <summary>
--no-edit --format json` → one `rdm review comment` per survivor → `rdm review
submit --verdict <v>` → a session-scoped `rdm commit`. Quotes and bodies are
captured through `persistCapture`, which assigns them to a shell variable via
a plain single-quoted `shellQuote` string (never interpolated into a command
line directly), so backticks, `$`, double quotes, em-dashes and newlines ride
through literally — a single-quoted string may itself span multiple lines,
since an embedded literal newline inside single quotes is valid POSIX shell.
`target` itself is shell-quoted (via that same `shellQuote` helper) at both
its occurrences — the `--on` argument and the `commit -m` message — so a
target containing `$(...)` or a backtick cannot execute a command when the
emitted lines run. A survivor carrying a `quote` gets `--quote`; one without
becomes a whole-document comment. `review start` always carries a NON-EMPTY
`--body`, or `submit_review` would raise `ReviewEmpty` on a clean review with
no comments.

`persistCapture` used to emit a QUOTED HEREDOC nested inside a `$(...)`
command substitution (`VAR=$(cat <<'TAG' ... TAG)`). macOS's system
`/bin/bash` (frozen at 3.2.57) cannot even parse that construct when the
heredoc body contains a literal apostrophe — the parser mis-tracks quote
balance across the nested heredoc, so the script fails before it ever runs.
The plain `shellQuote`-based assignment above replaced it, eliminating the
defect entirely rather than special-casing apostrophes (task
`persist-capture-bash32-heredoc-apostrophe`).

Every `rdm` line this ladder emits also redirects stdin from `/dev/null`.
`rdm` itself no longer blocks reading stdin for `review start`/`comment`/
`submit` (the CLI reads only `--body`, never stdin, for those three
commands), so this is defense-in-depth: the ladder stays safe against the
whole class of bug regardless of which `rdm` surface it invokes ever grows a
stdin read in the future. `planGateCommands` (`lib/plan-review.mjs`'s tag-clear
gate) carries the same redirect on its `task update`/`phase update`/`roadmap
update`/`commit` lines, for the same reason.

**The anchoring ladder is split between two mechanisms now: MECHANICAL, inside
the emitted ladder itself, for the one line that carries both `--path` and
`--quote` (a change-target, path-anchored comment); CALLER PROSE, bounded and
per-error, for everything else.**

For a change-target, path-anchored comment (`anchor.path !== null` —
`persistAnchorFor` already validated the path at build time), the emitted
`review comment --path … --quote …` line is wrapped in a shell `if`: on
refusal — for ANY reason, `QuoteOutsideChangedHunks`, `ChangePathNotInRevision`,
`ChangePathNotAFile`, or anything else the real binary reports for that
specific line — the SAME comment is re-emitted, in the SAME script, whole-document
(`--path`/`--quote` both dropped), header-rewritten to `anchor: degraded`, and
tallied into a run-time counter (`RDM_PERSIST_RUNTIME_DEGRADED`). This is a
mechanical property of the emitted bytes, not something a caller has to
remember to do — see "The all-anchors-degraded signal (AC4)" below for how the
tally reaches the ladder's own `anchorsDegraded=` line.

Every other refusal is still CALLER PROSE, stated in the skill that runs the
commands (the engine builds the ladder but never runs it), each rung naming
the real rdm-core refusal it recovers from, bounded to AT MOST TWO ATTEMPTS PER
FINDING, then a whole-document write, never a third:

| refusal | rung |
| --- | --- |
| `quote ... occurs N times` (`QuoteAmbiguous`) | retry with `--occurrence 1` |
| `quote ... not found` (`QuoteNotFound`) | drop `--quote`/`--occurrence` |
| `--occurrence N is out of range` (`QuoteOccurrenceOutOfRange`) | drop `--quote`/`--occurrence` |
| `--path only applies to a change review` | drop ONLY `--path` |
| `review start` refused the target | **park** — never choose a different target |
| anything else | **NEVER blanket-fallback** — stop and report the failure |

The three change-target `--path`+`--quote` refusals
(`QuoteOutsideChangedHunks`/`ChangePathNotInRevision`/`ChangePathNotAFile`)
used to be caller-prose rungs here too; they are now the mechanical case above
and no longer need a caller to remember them.

The last remaining "anything else" row is load-bearing: a Git or
source-identity failure (a `review source:` error, a source-repo discovery
failure, an invalid stored change revision, a moved HEAD) must SURFACE rather
than be laundered into a whole-document comment by a blanket flag-strip.

**An unanchorable quote is DOWNGRADED at build time, never emitted.**
`rdm review comment` on a `change/<sha>` review refuses `--quote` without
`--path` outright (`rdm_core::change::derive_change_anchor` ->
`Error::ChangeQuoteNeedsPath`, "--quote on a change review needs --path
<repo-relative path> naming the file the quote lives in"). That text matches NO
rung of the ladder above, so under the ladder's own never-blanket-fallback rule
an agent meeting it would report `ok: false` and abort the ENTIRE persist — no
review started, no comments recorded at all. So the writer never emits that
pair. `persistAnchorFor(finding, target, opts)` is the single decision: against
a change target a `--quote` rides only alongside a `--path`, and a quote with no
usable path is written whole-document instead. Two cases reach it:

- **An empty committed range** (`opts.source.noCode === true`): no hunks exist,
  so no `--path` can land. Both flags are suppressed and `emptyRange: true` is
  reported in the return. Reason `outside-hunk`.
- **No derivable path**: neither the finder's own `finding.path` (validated) nor
  `pathFromLocation(finding.location)` yields a usable repo-relative path — e.g.
  `path` is absent and `location` is free-form prose like `throughout the gate
  step`, or `location` carries extra prose past a parseable `path:line` prefix
  that defeats the end-anchored heuristic (`"src/x.rs:12 (mirrored at
  ...)"`) while `path` was also never supplied. Reason `path-missing`.

`persistPreDegradedAnchors(result, target, opts)` reports these as an array of
`{ findingId, reason }`, computed from the SAME decision the writer emitted. A
plan-repo document target is unaffected: there a bare `--quote` is the normal,
correct anchor and the array is empty.

This is the BUILD-TIME half of degradation only — a finding whose declared
`--path` passes every build-time check can still be refused once the ladder
actually runs (see the mechanical retry described above and "The
all-anchors-degraded signal (AC4)" below), which `persistPreDegradedAnchors`
cannot see, because nothing has run yet when it is computed.

**A degraded anchor is also reported in the review's OWN summary.**
`persistDegradationClause(result, target, opts)` composes
`persistPreDegradedAnchors`'s data into a one-line clause — "N of M requested
anchor(s) could not be placed at persist time (path-missing xN[, outside-hunk
xN]); see the `anchor` header on each comment." — appended to the `--body` text
`persistReviewCommands` gives `rdm review start`. Empty (and therefore
invisible) when nothing degraded. This is a pure BUILD-TIME computation over
data already in hand, not a report about what the emitted ladder did after
running, so it revives none of the removed persist-ack round trip. It also
does NOT touch `classifyOutcome`/`outcome` — outcome classification staying
independent of anchor plumbing remains a deliberate, recorded design decision
(see immediately below); the signal lives in the persisted document's own
body, not in the OUTCOME a caller gates on. Each comment's own `anchor` header
(§ "Persisted review comment body" above) still carries the per-finding
detail; this clause is what makes the aggregate visible without opening every
comment.

**The all-anchors-degraded signal (AC4).** A review whose anchors ALL degraded
is a stronger case than an ordinary partially-degraded one — the persisted
review carries no useful per-file anchoring at all — and a caller must be able
to tell the two apart WITHOUT parsing `persistDegradationClause`'s prose.

`persistDegradedSummary(result, target, opts)` is the BUILD-TIME half:
`{ requested, degraded, all }`, where `requested` is how many survivors asked
for a `--quote` anchor and `degraded` is how many were dropped before a single
command ran (the same count the prose clause composes). It cannot see a
run-time refusal, because nothing has run yet when it is computed — see the
mechanical retry above.

The REAL, post-run result — build-time drops PLUS run-time ones — has exactly
one authoritative source: **the persist ladder's own trailing line**,
`printf 'anchorsDegraded=%s\n' "$RDM_PERSIST_ANCHORS_DEGRADED"`. Unlike the
rest of the ladder, this line's VALUE is a genuine shell computation, not baked
in at build time: the ladder combines the build-time `degraded` count (a
literal integer, known when the command list is built) with
`RDM_PERSIST_RUNTIME_DEGRADED` (the counter the mechanical retry above
increments), and buckets the total against the build-time `requested` count
into `all` / `partial` / `none` with a shell `if`/`elif`/`else`. `all` fires
ONLY when at least one anchor was requested and every single one degraded,
whichever stage caused it; a run with zero requested anchors, or with some but
not all degraded, prints `partial` or `none`.

**The REAL total is also recorded IN THE REVIEW ITSELF, not only on stdout.**
The build-time-only `persistDegradationClause` above is baked into the
`--body` text before `review start` even runs, so it cannot see a run-time
refusal — a review whose every anchor degraded at run time used to persist
with a clean-looking summary and no trace beyond each affected comment's
`anchor: degraded` header. Once the block above has computed
`RDM_PERSIST_TOTAL_DEGRADED` (build-time plus run-time), and while the review
is still a DRAFT — before `review submit` — the ladder appends one
whole-document comment whenever that total is greater than zero:
`persistDegradationNoteBody(totalDegraded, requested)` (`persist-note:
anchors-degraded`, then "`N` of `M` requested anchor(s) could not be placed;
see the anchor header on each comment above."), with the run-time-only-known
total filled in through `printf` rather than `persistCapture` — `persistCapture`
emits a static `shellQuote`d string and cannot expand a shell variable at all,
so this call site needed `printf` regardless of which capture strategy
`persistCapture` itself uses (see task
`persist-capture-bash32-heredoc-apostrophe`, which replaced `persistCapture`'s
own heredoc-in-`$(...)` form with the same `shellQuote`-based approach for an
unrelated reason — that form could not even PARSE under macOS's bash 3.2 when
the captured text contained an apostrophe).
This note carries none of `PERSIST_HEADER_KEYS` (it describes the review, not
a survivor), so `parseCommentHeader` correctly reports it as unheadered. The
note is the SAME text a caller reading the persisted review sees regardless of
whether the degradation happened at build time or run time — the two cases
are no longer distinguishable only by whether the caller happened to capture
the ladder's stdout.

**The anchor-degraded park signal (AC1), keyed on CAUSE, not on the raw
`anchorsDegraded` tally (phase-46: anchor-degraded-park-by-cause).** A
degraded anchor is not, by itself, a park signal: a "you missed an edit
here" finding necessarily quotes a line the diff did not change, and a "you
missed editing this file entirely" finding names a real, in-range file the
diff never modifies at all — both are `Error::QuoteOutsideChangedHunks`
(one Display arm per case; see rule 1 below), `rdm review comment --path`
correctly refuses either anchor, and the ladder's mechanical retry lands it
whole-document with nothing lost — `anchorsDegraded=all` or `partial` in
that case describes a LOSSLESS fallback, not a problem. `anchorsDegraded`
could not previously distinguish those benign cases from a systemic one (a
path that does not exist at the reviewed head at all, a quote absent from
the document entirely, or an ambiguous quote), so the dispatch skill's park
rule used to fire on both alike.

A second, narrower line — `anchorsParkRequired=<yes|no>` — is what a caller
should actually key a park decision on. It is computed by the ladder from
two rules:

1. **A systemic cause always requires a park.** Any BUILD-TIME degradation
   (`persistPreDegradedAnchors` non-empty — no derivable path, or an empty
   reviewed range) is systemic by construction. Any RUN-TIME refusal whose
   captured stderr does NOT match either pattern in
   `ANCHOR_REFUSAL_BENIGN_TAIL_PATTERNS` — i.e. anything other than
   `QuoteOutsideChangedHunks` (a path absent at the reviewed head —
   `ChangePathNotInRevision`; a path naming a directory or submodule —
   `ChangePathNotAFile`; a quote absent from the document entirely —
   `QuoteNotFound`; or an ambiguous quote occurring more than once —
   `QuoteAmbiguous`/`QuoteOccurrenceOutOfRange`) — is also systemic.
   `ANCHOR_REFUSAL_BENIGN_TAIL_PATTERNS` and the pure helper
   `isAnchorRefusalBenign(stderrText)` are the single definition of this
   check.

   **The classification is NOT a plain substring search** (plan-review
   2026-09-23-1613-a415). An earlier version matched any stderr containing
   the substring `"not touch"`, on the theory that it is the one thing both
   `QuoteOutsideChangedHunks` Display arms share
   (`rdm-core/src/error.rs:877-886`). That is unsafe: several SYSTEMIC
   refusals this call site can produce echo the finder's own
   caller-controlled `--quote`/`--path` text back into their own message —
   `Error::QuoteNotFound` prints `quote {quote:?} not found ...`,
   `Error::QuoteAmbiguous` echoes the quote in its occurrence list, and
   `Error::ChangePathNotInRevision` echoes the path — so a finder whose
   quote or path happened to contain the words "not touch" (an ordinary
   thing to write about code, e.g. "this code does not touch the validation
   path") would misclassify a genuinely systemic refusal as benign, purely
   because of caller-supplied text nowhere near the real cause.

   Each entry in `ANCHOR_REFUSAL_BENIGN_TAIL_PATTERNS` instead requires a
   FIXED, rdm-authored substring unique to one `QuoteOutsideChangedHunks`
   Display arm, anchored at END OF LINE, so it can only match text rdm
   itself appends after every interpolated field:
   `quote text inside a changed hunk \(nearest: lines [0-9]+-[0-9]+\), or
   omit --path/--quote for a whole-change comment$` for the `Some(nearest)`
   arm (`"quote text inside a changed hunk (nearest: lines "` appears
   nowhere else in `error.rs`; the hunk line numbers rdm interpolates there
   are digits it computes itself from real git hunk data, never finder
   text), and `comment on a file the change modifies, or omit
   --path/--quote for a whole-change comment$` for the `None` arm
   (`"comment on a file the change modifies"` likewise appears nowhere
   else). Neither pattern is the shared trailing clause `"or omit
   --path/--quote for a whole-change comment"` BY ITSELF — that clause is
   also how `ChangePathNotInRevision`, `ChangePathNotAFile` and
   `ChangePathNotLinkable` end, so matching it alone would misclassify a
   path absent at the reviewed head as benign regardless of any caller data
   at all. `{quote:?}` is Rust's `Debug` format, which escapes embedded
   quotes and newlines, so caller text can never terminate a line early and
   forge either pattern's required end-of-line tail.

   `ANCHOR_REFUSAL_BENIGN_TAIL_PATTERNS` is a POSIX-ERE-compatible array
   (each entry escapes its one literal `(`/`)` pair and uses a `[0-9]+`
   class — no JS-only regex syntax), so the SAME array drives both
   `isAnchorRefusalBenign` (via `new RegExp(patterns.join('|'), 'm')`) and
   the `grep -qE` alternation the emitted ladder runs — the two cannot
   silently diverge.
2. **A `blocking` finding losing its anchor always requires a park, even for
   the benign cause.** Severity is known statically when the shell is
   generated, so this branches at JS code-gen time — a `severity: 'blocking'`
   survivor's run-time refusal unconditionally bumps the park counter,
   without needing to inspect stderr at all.

Everything else — a non-`blocking` survivor refused only because its quote
sits on an untouched line in an otherwise-touched file, OR because it names
a real, in-range file the diff never modifies at all — still contributes to
`anchorsDegraded` (the volume stays visible) but NOT to
`anchorsParkRequired`. Both are the SAME error variant
(`Error::QuoteOutsideChangedHunks`, just its two different `nearest` arms —
raised by `derive_file_quote` in `rdm-core/src/change.rs`, called from
`derive_change_anchor`), and both are equally legitimate findings: "you
missed an edit here" and "you missed editing this file entirely" are the
same class of comment, and neither loses anything by landing
whole-document.

Mechanically: a per-finding `mktemp` scratch file
(`RDM_PERSIST_ANCHOR_STDERR`, same hygiene as `RDM_PERSIST_START_JSON` —
created with `mktemp`, `|| exit 1`, removed after use) captures the
path-anchored `review comment` line's stderr; on refusal it is `cat`ted back
to stderr (so nothing already visible to the operator is lost) and then
either unconditionally bumps the running `RDM_PERSIST_PARK_REQUIRED` counter
(a `blocking` finding) or does so only `if grep -qE '<pattern1>$|<pattern2>$'
"$RDM_PERSIST_ANCHOR_STDERR"; then :; else RDM_PERSIST_PARK_REQUIRED=...; fi`
(everything else), where `<pattern1>` and `<pattern2>` are
`ANCHOR_REFUSAL_BENIGN_TAIL_PATTERNS` joined with `|` and shell-quoted as
ONE argument — the exact same array `isAnchorRefusalBenign` tests against,
so the JS helper and the emitted shell cannot silently diverge.
`RDM_PERSIST_PARK_REQUIRED` is seeded from the build-time degradation count
before the survivor loop runs. Once the loop finishes, the ladder buckets
the counter into `RDM_PERSIST_ANCHORS_PARK_REQUIRED=yes` (`-gt 0`) or `=no`,
and prints it as the trailing `anchorsParkRequired=<yes|no>` line —
immediately after the existing `anchorsDegraded=<all|partial|none>` line,
which keeps printing exactly as before.

There is no structured error surface for `review comment` — every refusal
exits 1 through `rdm-cli/src/main.rs`'s single `process::exit(1)`, and the
command supports no `--format json` error output — so this stderr-pattern
match is the only available machine-distinguishing signal between the benign
and systemic cases. This is a real, stated limitation of the design, not an
oversight: if a future `rdm-core` change adds a structured error surface for
`review comment`, this classification should move onto it.

`rdm-wf-review-refute-fix.js`'s standalone code-review path (the driver
region, not the stamped block) ALSO attaches `persistDegradedSummary`'s
`{ requested, degraded, all }` object as `result.persistDegraded`, alongside
`persistCommands` / `persistScript` — but this is a BUILD-TIME PREVIEW ONLY,
computed before the ladder has run, and it can under-report a run whose
anchors degraded (or whose park requirement) is decided at run time. **A
caller MUST key its park decision off the ladder's own printed
`anchorsParkRequired=` line — never off `anchorsDegraded` alone, and never
off `result.persistDegraded`.**

Neither the build-time preview nor either of the ladder's own printed lines
touches `classifyOutcome`/`outcome` — the same recorded design decision
applies. **The caller is responsible for acting on the printed line.**
`.claude/skills/rdm-dispatch-phase/SKILL.md` (and the shipped
`rdm-core/src/templates/skill-dispatch-phase-cli.md`) run the persist ladder,
capture its output, and PARK `blocked` when its last line reads
`anchorsParkRequired=yes`, even though the ladder itself exited 0 — a review
whose anchors were lost for a systemic cause, or whose `blocking` finding
lost its anchor at all, should not be reported as ordinary successful
persistence. `anchorsParkRequired=no` is not a park, regardless of what
`anchorsDegraded` reads; it proceeds normally, with the degradation already
visible in the review's own note comment (see above) and per-comment
`anchor` headers.

**The engine's own headless `gate: true` path reads the same signal, through
an environment variable.** `persistCommands`/`persistScript` and
`gateCommands`/`gateScript` are two SEPARATE shell sessions the orchestrator
pastes in turn, so the gate script — built before the persist ladder has ever
run — cannot see a JS-side value for the run-time result. Instead, when both
`persist` and `gate` are requested together and the outcome maps to
`reviewed`, the emitted `gateScript` opens with a guard reading the
`RDM_PERSIST_ANCHORS_PARK_REQUIRED` environment variable (defaulting to `no`
when unset): when it reads `yes`, the gate script refuses to write `reviewed`
and exits nonzero with an actionable message instead, rather than writing the
status. A caller running both ladders is responsible for threading the value
through — setting `RDM_PERSIST_ANCHORS_PARK_REQUIRED` from the persist
ladder's own printed `anchorsParkRequired=<value>` line before running the
gate script. No shipped consumer currently passes `gate: true` (both
`rdm-dispatch-phase` and `rdm-review` always pass `gate: false` and own their
own status write), so this is a completeness fix for the documented
capability rather than a change in any current caller's behavior. (The
variable was previously named `RDM_PERSIST_ANCHORS_DEGRADED` and keyed on
`= "all"`; phase-46 renamed and rekeyed it to match the new cause-based
signal.)

**The gate lines themselves are single-sourced** (arch-1): the guard shell
lines above are `persistDegradationGateLines()`, a function in
`lib/review.mjs`'s stamped block — not hand-written in any driver region. It returns the exact
`if [ "${RDM_PERSIST_ANCHORS_PARK_REQUIRED:-no}" = "yes" ]; then … fi` text as
a single string; a caller building a status-write ladder (today, only
`rdm-wf-review-refute-fix.js`'s `gateCommands` builder) pushes it verbatim.
Because it lives in the stamped block, `scripts/gen-workflow-review.sh`
copies it into every consumer along with the rest of the review pipeline, so
the RULE — not just its effect — has exactly one definition, rather than a
hand-copied `if`/`echo`/`exit` block per consumer. The dispatch-phase skill
(`.claude/skills/rdm-dispatch-phase/SKILL.md` and the shipped
`rdm-core/src/templates/skill-dispatch-phase-cli.md`) is prose, not code, so
it cannot call the function directly; its two copies instead carry the
IDENTICAL paragraph describing the rule, kept in sync by hand rather than by
a generator (dispatch-phase is not a `scripts/gen-skill-review.sh` consumer —
that script only renders `skill-review-cli.md` and `skill-plan-review-cli.md`
from `lib/review.mjs`'s `//|` spec prose).

**Verdict mapping** (`PERSIST_VERDICT` / `persistVerdictFor`, which THROWS on an
unrecognized outcome rather than defaulting to `comment`):

| outcome | verdict |
| --- | --- |
| `reviewed` | `approve` |
| `rework` | `request-changes` |
| `escalated` | `request-changes`, with the mode's `[code]`/`[plan]` escalation prefix on the review body |

**WHO RUNS THE LADDER.** Nobody inside a workflow. `persistReviewCommands`
returns the ordered command list as DATA; the engines hand it back on their
result (`persistCommands`, plus a newline-joined `persistScript`) and the
ORCHESTRATOR pastes it into one Bash session and reports the exit status. The
ladder prints `reviewId=<id>`, then `anchorsDegraded=<all|partial|none>`, then
`anchorsParkRequired=<yes|no>` on success — see "The all-anchors-degraded
signal" and "The anchor-degraded park signal" above.

There is consequently **no persist acknowledgement**, and the machinery that
existed to read one is gone: `PERSIST_ACK_SCHEMA`,
`buildPersistReviewPrompts`, `persistAccounting`, `classifyPersistOutcome` and
`degradationSummaryClause`. Every one of them reconciled an agent's self-report
about commands it claimed to have run against the survivor list the writer was
handed — a check a shell exit status does not need. Two consequences follow, and
both are deliberate:

- **Outcome classification no longer composes anchor degradation.** The outcome
  is decided once, from the survivors and the AC table, before any write is even
  described. A `reviewed` review whose anchor would not land is still `reviewed`;
  what to do about the anchor is the caller's, not the verdict's.
- **The anchoring fallback is split.** For the one line that carries both
  `--path` and `--quote` (a change-target, path-anchored comment), the retry
  is now MECHANICAL, inside the emitted ladder itself — see "The anchoring
  ladder" above. For everything else, it is still prose a caller skill states:
  if a bare `review comment --quote` line is refused, retry with
  `--occurrence 1` or drop `--quote`/`--occurrence` per the refusal table
  above; if `review start` itself is refused, park rather than choosing a
  different target.

The OUTCOME gains a `reviewId` key **only when the persist step actually ran** —
never `reviewId: null`. A failed persist logs loudly and changes nothing else.

**The agent type is a per-consumer parameter.** The writer contains no
`agentType` literal, and neither does any consumer: `no-mechanical-agents-in-workflows`
removed every `agentType` call site from `.claude/workflows/`, so all five
engines — every one of them distributed since `agent-orchestrated-dispatch`
phase 26 — thread none (see CLAUDE.md § `.claude/agents/`).

**The round channel.** Round state comes from exactly one channel, read
unconditionally by `reviewUnit` regardless of whether the CURRENT pass sets
`persist`: `unit.priorReviews`, the reviews already persisted on the target,
which the caller supplies (sourced from `rdm review list --on <target>
--format json`, normally by the orchestrator). It shapes into
`{ round, findings }` — the round number is the count of non-draft prior
reviews (`priorRoundFromReviews`), and the prior round's findings are the
latest prior review's comment bodies read back through `parseCommentHeader`
(`priorFindingsFromReviews`). A human's review on the same target legitimately
advances the round — a round is a pass over the plan, whoever made it.
Everything downstream is unchanged, so the round-3 escalation cap and the
REPORTING-ONLY repeat rule are preserved by construction rather than by a
second implementation. No prior persisted review for the target fails toward
round 1, the same "fails toward round 0" stance `priorRoundFromReviews` takes
on an empty list — but the reason it is empty matters, and the engine tells
the two apart:

- `priorReviews: []` — the caller ran `rdm review list` and there genuinely
  are none yet. Round 1 is the true state, reported silently, exactly as
  before.
- `priorReviews` absent (`null` — the key was never supplied, e.g. a caller
  that skipped `rdm review list` entirely) — the engine cannot know the
  round, and must not report round 1 as if it did. `reviewUnit` sets
  `roundUnknown: true` on the unit (mirroring `gateAction.tagsUnknown`'s
  absent-vs-empty treatment of the tag list) and the unit's `summary` carries
  a visible `[round unknown: …]` clause (`roundUnknownClause`) naming the
  fix (`rdm review list --on <target> --format json` as `priorReviews`).
  `round` still reports 1 — no round arithmetic changes — but the absence is
  now visible rather than silently indistinguishable from a genuine round 1.

The engine also RENDERS a `## Plan Review Round <N>` audit note
(`formatRoundNote`) and returns it to the caller as `roundNote`, a
human-readable log only; appending it to the document is the caller's write,
and its inverse, `parseRoundNotes`, is never called from `reviewUnit` — the
note is not a second input to the round channel. An `--implementation-plan`
target does not use the round channel at all: `classifyPlanOutcome`, not
`classifyRoundOutcome`, decides its outcome, and that target persists **only
when the caller names the plan document it handed over**, with the `planSlug`
argument below — independent of whether any prior review exists. A free-form
plan pasted into the args with no slug has nothing to hang a review off, so
persist is forced off there (`persistIgnored`) and it keeps the in-context
note.

**`planSlug` arg.** The slug of the persisted `plan/<slug>` document under
review, for an `--implementation-plan` target. Read from the STRUCTURED `args`
object only — never parsed out of the `$ARGUMENTS` flag string, the same rule as
`phases`/`tags`/`priorReviews`/`wontFixedTexts`/`reviewers`.

**It is the ONLY way to name the plan.** `planText` is gone, and with it the
dual-supply throw and the precedence rule that decided which of the two won —
there is nothing to transport, so there is nothing to disagree about. The
reviewers read the document themselves from the `rdm plan show <slug>
--format json` command their prompt names, which is also the ref the persist
ladder writes to, so the graded document and the recorded verdict cannot name
different documents.

`planSlug` is what makes an implementation-plan verdict persistable: with it and
`persist` on, the run returns `persistCommands` / `persistScript` for
`plan/<planSlug>` (never for `persist.on`, which is why a disagreeing one throws
at parse time). It adds no gate and no act step — a plan document carries no
tags, so there is no `needs-plan-review` to clear — and the branch returns without
`gateAction` or any gate key. A no-slug run reports the outcome and findings only.

### `VERDICT`

A refuter agent's grade of a single `FINDING`. A **fresh** refuter grades each
finding — the finder never grades its own work.

| field        | type                     | notes                                                  |
| ------------ | ------------------------ | ------------------------------------------------------ |
| `refuted`    | boolean (required)       | `true` ⇒ the finding does not hold up ⇒ dropped        |
| `confidence` | integer 0–100 (required) | the refuter's confidence in **its verdict** (advisory) |
| `rationale`  | string                   | why the finding was or was not refuted                 |
| `quote_ok`   | boolean                  | **optional**; did the finding's `quote` appear VERBATIM in the reviewed text? Requested only when the finding carries one, and INDEPENDENT of `refuted` — an explicit `false` strips the quote, never the finding |
| `inScope`    | boolean                  | **optional**; is the finding within the scope of the approved plan the change implements? Requested only for a code-mode review with an associated plan (`context.planCommand` set), and INDEPENDENT of `refuted` — a finding can be real (not refuted) and still out of scope. Absent/`true` means in scope; only an explicit `false` excludes the finding from `hasBlocking` (see `rdm:plan/refuters-grade-finding-scope`) |

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
`rdm_devtools::measure::refuter_agreement` (the `rdm-measure refuter-agreement`
instrument), deliberately **not** in `.claude/workflows/lib/review.mjs`, which
carries no batched symbols while the recorded decision is anything other than
`ship-batched`.

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
post-filtering — plan-review's `suppressWontFixed`
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
`everHit` promises to keep visible — so a dispatch threads
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
`OUTCOME`. `total` is the number of reviewers `resolveReviewers` returned for
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

0. **Select** — the deterministic pre-step `resolveReviewers(mode, reviewers)`
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
| arg name | `maxRefutations` (on `rdm-wf-plan-review` and `rdm-wf-review-refute-fix` args, and historically on `rdm-wf-dispatch-phase`'s; reaches `runReview` as `context.maxRefutations`) |
| default | `DEFAULT_MAX_REFUTATIONS` = 5 |
| `0` | LEGAL and meaningful — grade nothing, pass every gating finding through as `unrefutedReason: 'budget'`. Never conflated with "unset" by a falsy check. |
| uncapped | no sentinel exists; express an effectively-uncapped run as a large N |
| validation | `resolveRefutationBudget(value)`, mirroring `parseBudget`'s contract — a number or integer-ONLY string; `'5abc'` is rejected, not coerced. `rdm-wf-plan-review` validates at PARSE time, before any `agent()` call. |
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
N including 0. `rdm-cli/tests/workflow_review/budget.rs`
(`budget_ranking_deterministic_and_monotone`) encodes this as an exhaustive
subset property test, not only as prose.

`context.target` (and any other fields) is threaded into every finder and refuter
prompt, so the review material reaches the agents. `deps` (`{ agent, pipeline,
parallel, log }`) is omitted in the Workflow runtime (the ambient globals are
used) and injected by the verify harness to drive the pipeline with fakes.

### `context.sourceCommand`: pinning plan review to a worktree checkout

Plan review can read the same pinned worktree checkout the code-review engine already reads,
instead of whatever checkout the invoking session happens to be sitting in. This closed an observed
failure: a plan review of a phase in `agent-orchestrated-dispatch` was graded against `main`, while
the plan itself targeted the roadmap's own unlanded worktree — the reviewers reported real-looking
findings ("this call site does not exist anywhere in the file") that were true of `main` and false of
the branch the plan was written against.

`reviewTargetBlock(mode, context)` — the shared prompt-fragment builder both `findPrompt` and
`refutePrompt` call — takes a `mode` (`'code'` | `'plan'`) alongside `context`. When
`context.sourceCommand` is set, it renders mode-specific instructions: `code` mode's text is
unchanged from before this pin existed (resolve the change and review the committed `base..head`
range); `plan` mode's new text tells the reviewer to run the pinned `rdm review source` command
itself. For the exact wording — including how to interpret a non-zero exit (which may indicate drift or a bad pin/environment), when to quote stderr in a blocking finding, and how to read files from the pinned checkout — see the `plan` branch of `reviewTargetBlock` in `.claude/workflows/lib/review.mjs`.

`parsePlanArgs` (`.claude/workflows/lib/plan-review.mjs`) accepts four new, optional, structured-key-
only args reusing the code-review engine's own flat names rather than a nested shape: `source` (the
pinned checkout path), `base`, `expectedHead` (full hex SHA — same shape code review's `requireSha`
enforces), `expectedBranch`. **None-or-all**: supplying only some of the four throws at parse time,
before any agent runs, naming the missing ones. From these, plus the already-parsed `task` /
`roadmap`+`phase` identifiers, the parser derives `sourceItem` — `task/<slug>` or
`phase/<roadmap>/<phase>` — the `--on` value a pinned `rdm review source` call binds to. A pin with no
resolvable `sourceItem` on an `--implementation-plan` target (neither a `task` nor a `roadmap`+`phase`
given alongside it) throws for the same reason. `buildReviewUnits` derives each review unit's
`sourceCommand` from **that unit's own `target`** — a roadmap sweep's phase units each verify their
own checkout independently, and the bare roadmap-body unit gets no `sourceCommand` at all, since `rdm
review source` requires a phase or task item and rejects a roadmap (`resolve_review_source` in
`rdm-core/src/worktree.rs`). `--no-code` is always passed on the built command, unconditionally: plan
review runs before implementation, so an empty committed diff between `base` and `head` is the
expected, legitimate case, never a caller mistake the way it is in code mode.

**The no-source fallback is the explicit, permanent default**, not a stopgap: omitting all four pin
args makes every prompt render byte-identical to before this capability existed, reading from the
invoking session's own working directory — the correct behavior for the standalone
`rdm-plan-review`/`rdm-wf-plan-review` surface run outside a dispatch worktree. This is load-bearing
for the refuter-agreement instrument's 56-item adjudicated finding corpus, whose `corpus_prompts_regenerate_as_recorded` test (`rdm-devtools`) regenerates
every recorded prompt through the real `findPrompt`/`refutePrompt` with `context = { target:
item.target }` only (no `sourceCommand`, for both `code` and `plan` mode items) and asserts the
recorded `promptSha256` is unchanged — there is no supported way to re-baseline it wholesale (see
`docs/refuter-model-tiering.md` § Maintenance gap). The `context.sourceCommand` branch inside
`reviewTargetBlock` is therefore strictly conditional, exactly like the existing QUOTE VERIFICATION
and SCOPE GRADING clauses in `refutePrompt` it follows the same pattern as.

The `rdm-dispatch-phase` skill's step 6 passes its step-4-pinned `identity` (`source`, `base`,
`expectedHead`, `expectedBranch`) plus `phase`/`task` to the plan-review call, mirroring step 12's
code-review call. The distributed skill template omits this call entirely — that surface's plan gate
is a human-submitted approve review rather than a workflow verdict (see the template's own "Why there
is no plan-review Workflow call here" section) — so it has no step 6 pin to add; the engine change
still ships to it because `rdm-wf-plan-review.js` itself is emitted as-is to every downstream consumer.

**Every consumer of `runReview`/`d.review(...)` must destructure
`{ survivors, acTable, budget }`** rather than treat the resolved value as a bare
array. In the retired `lib/dispatch-phase.mjs` this meant **both** `runCodeGate`
(which tracked a per-round `acRounds` array alongside `rounds` and checked
`acTableHasGap` in its rework-loop continuation) and `runPlanGate` (which
discarded `acTable` — always `null` in `plan` mode — and used `survivors` as
its `findings`); `lib/plan-review.mjs`'s `reviewUnit` and its
`--implementation-plan` branch, and `rdm-wf-review-refute-fix.js`'s legacy and
standalone driver paths, do the same.

### Dimensions and `when` triggers

Each dimension is either **always-on** (no `when` key) or **triggered** (a
`when(signals) => boolean` predicate evaluated over both the change's shape and
the target's type).

| mode | always-on | triggered |
| --- | --- | --- |
| `code` | `ac`, `correctness` | `tests`, `architecture`, `api-docs`, `changelog`, `security` |
| `plan` | `coherence`, `architectural-fit`, `restraint` | `unit-of-work` (phases only), `intent-alignment` (recorded intent only) |

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
`scripts/verify-workflow-review.sh` § AC2b (retired with that script as a prose-string check under the operator's no-grep rule) asserted that the set of code
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
[`token-baseline.json`](token-baseline.json) § `planFinderCollapse`. The
instrument and its harness (`scripts/verify-finder-collapse.sh`) were retired
on 2026-09-23 — the decision is closed, and the harness's corpus assumed three
plan reviewers, which is stale now that there are five.

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

`unit-of-work` likewise stays a separate reviewer in either scenario: a caller
reviewing something that is not a phase simply omits it, and folding a
selectively-included lens into an unconditional agent would take that choice
away.

### `context.reviewers` and `resolveReviewers(mode, reviewers)`

**The caller selects the reviewers.** `resolveReviewers` only resolves the names
it was handed against the mode's catalogue:

- `reviewers` **omitted** (`null`/`undefined`) → every reviewer for the mode, in
  declaration order. A maximal default encodes no policy, where a selective one
  would.
- a **list of keys** → exactly those, filtered out of `DIMENSIONS[mode]` in
  declaration order (never the caller's order, so the fan-out and the candidate
  `order` tiebreak stay stable).
- an **unrecognised name** selects nothing and is **not** rejected. There is no
  unknown-name guard: the mistake shows in `coverage.selected` / `coverage.ran`,
  which is where under-coverage is meant to be visible.
- an unknown `mode` → throw.

**Coverage is visible, not enforced.** Nothing checks the set's size,
composition, or fitness for the target — a caller may deliberately under-review,
and `coverage.ran` records what actually ran. The one refusal kept is a set that
resolves to **zero** reviewers, which throws rather than reporting a clean review
over an empty fleet; that is the pre-existing "refusing to report a clean review"
invariant, and it reads only the list it was handed.

The `context` contract carries **no** diff-shape signals, no target type, and no
document body. It holds `target` (an identifier — an item ref, a plan slug, or
the `rdm … show` command a finder runs itself), the optional `reviewers`,
`maxRefutations`, and the optional `findModel`/`verifyModel` ids. A reviewer that
needs a document **fetches it itself**; nothing transcribes one into this object.

**What was deleted.** `selectDimensions`, every per-dimension `when` predicate,
`SIGNAL_KEYS`, `deriveSignals`, `contentSignal`, `addedLines`, `matchesAny`, the
`EXPORT_CONTENT_PATTERNS` / `USER_FACING_CONTENT_PATTERNS` /
`SECURITY_CONTENT_PATTERNS` vocabularies, `TEST_PATH_PATTERNS`,
`CODE_EXTENSIONS`, `stripNonPhaseUnitOfWork`, and the whole recorded-intent
transport (`extractIntent`, `intentPresent`, `INTENT_PREAMBLE`,
`INTENT_MISSING_NOTICE`). Diff-shape inference existed to guess what a caller
already knows; `unit-of-work` scoping and the intent channel existed because the
engine was choosing rather than being told. All three answers are now the
caller's, and each reviewer's catalogue entry carries a one-line cue for when to
include it — single-sourced in `review.mjs`'s `//|` prose and rendered into every
review skill.


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
copied into workflow scripts, where the since-deleted `verify-workflow-review.sh`
hygiene grep forbade that literal (the Rust
`engine::driver_deterministic_and_emits_no_done_trailer` now checks the driver's
returned commands carry no such directive). The trailer's format string lives in `rdm-core`
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
  `rdm-core/src/templates/skill-review-cli.md` between
  `<!-- rdm:review-spec:begin/end -->` markers. It is mode-dispatched
  (`--mode code|plan`), and `--mode plan` renders the SAME regions into
  `skill-plan-review-cli.md` — one source, one emitter, two skills. The
  gate region sits outside the stamped block precisely because it is the one
  place the completion-trailer literal may appear.

Which mode a prose line belongs to is declared by an optional **per-line mode
tag**, written immediately after the `//|` prefix: an untagged line is shared
and renders in every mode, a `code|`-tagged line renders only under
`--mode code`, and a `plan|`-tagged line only under `--mode plan`. The tag is
recognized only as that literal text immediately after `//|`, so shared prose
must never begin with it. There is no second region, no second generator, and no
second consumer list — the tag is the whole mechanism. Mode-isolation greps in
the since-deleted `scripts/verify-workflow-review.sh` were the detector for a
mistagged line leaking across; they were already gone before that script was.

`gen-skill-review.sh` also carries an orthogonal **`--target shipped|local`**
axis (default `shipped`), independent of `--mode`: `shipped` renders the
`rdm-core/src/templates/skill-{review,plan-review}-cli.md` files baked
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
`--check`-gated by `rdm-cli/tests/workflow_review/generators.rs`
(`local_skill_projection_in_sync_{code,plan}`) alongside the shipped ones.

The gate itself is likewise mode-dispatched data rather than a fork:
`GATE_POLICY[mode][outcome]` yields `{ status, writesCompletion,
clearsPlanReviewTag, reasonPrefix }`, and `STATUS_MAPPING` *is*
`GATE_POLICY.code`, so `statusFor`/`writesCompletion` are unchanged for the
code lane (`rdm-dispatch-phase`/`rdm-autopilot`). The plan rows carry an explicit `status: null` — a
plan review never persists an rdm status; it clears `needs-plan-review` on
`reviewed` and leaves it on `rework`/`escalated`.

### `rdm-wf-plan-review`'s gate disposition: `gateAction`

`GATE_POLICY.plan` above says what the gate *should* do. `gateAction` is what the
CALLER must run to do it: **the engine never writes the tag**, because it has no
agent that can run a shell command.

| field | where | meaning |
|---|---|---|
| `gateAction` | every unit, plus the single-target flatten | `{ kind, ident, roadmap, clearsPlanReviewTag, tagsUnknown, commands, remainingTags, removedTags }`. `commands` is `[updateCmd, commitCmd]` from `planGateCommands`; `[]` on a `rework`/`escalated` unit, which still gets an action so callers can iterate `units[].gateAction` without special-casing. |
| `tagsUnknown` | inside `gateAction` | `true` when the outcome clears the tag but the caller supplied no `tags` list. `--tags` replaces the whole list, so writing one the engine was never shown would silently drop a sibling such as `depends-unlanded`; `commands` is `[]` and the refusal is visible rather than silent. |
| `gatePendingCount` | run-level result | How many units are waiting on the caller to run their commands. Appended to the final `N unit(s) reviewed` log line when non-zero. |

There is no `gateBlocked` and no `gateDeferred`. The first meant "the write was
attempted and did not succeed" — nothing attempts it. The second meant
"`gateMode: 'return'` made the driver compute and hand back" — that is now the
only behaviour, so the distinction has no content. A unit still awaiting its
write carries a ` [gate pending: … — <update> && <commit>]` clause on its
`summary`, so a surface that reports only `summary` is already reporting exactly
what remains to be done.

The `--implementation-plan` branch has no persisted item, so it gains **none** of
these keys.

**Scope: `gateAction` is `rdm-wf-plan-review.js` driver surface.** It is
deliberately absent from the shared `//|plan|` review spec,
and therefore from the shipped `skill-plan-review-cli.md` templates and
`plugins/rdm/skills/plan-review/` — those skills perform the gate write
themselves, in hand-authored prose that shells out to `rdm … update --tags …`,
and have no driver to read a returned action off. The shared spec states the same
*policy* in terms of the write instead; the field name lives only in the
hand-authored half of `.claude/skills/rdm-plan-review/SKILL.md`.
`verify-workflow-review.sh` § 1d-gate-policy used to gate both directions,
with a § 1g self-test proving the detector fired; that section was retired by
the operator amendment to `task/retire-static-grep-harnesses` (2026-09-23), so
the scoping is now held by convention and code review. See
[`plan-review-gate-policy.md`](plan-review-gate-policy.md) § "What changed" ¶ 4.

The pending clause embeds an exact rdm command containing double quotes
(`--tags "a,b"`), so — unlike `coverageSummaryClause`, which is quote-free
precisely *because* it is interpolated into Bash prompts — `summary`/`reason` in
plan mode are returned **data** and must never reach a prompt builder.
`lib/plan-review.mjs` builds no prompt at all any more, so there is nothing for
the clause to leak into.

**`gateMode` is gone.** It chose between "the `gate:clear-tag` agent writes the
tag in-run" and "compute the action and hand it back", and only the second
exists. The gate prompt it governed — a four-clause authorization preamble plus
rendered review evidence — is gone too: it existed to persuade a safety
classifier that a MECHANICAL AGENT's write was authorized, and there is no such
agent to authorize. The decision it recorded, its boundary, and the classifier
blocks that forced it remain in
[`plan-review-gate-policy.md`](plan-review-gate-policy.md).

Everything else inside the stamped block is **machinery** (JSON schemas,
`survives`/`rankFindings`/`resolveReviewers`, the classifier and
the gate policy) and is never rendered into a skill. Both generators are
`--check`-gated by `rdm-cli/tests/workflow_review/generators.rs` — the skill
generator in BOTH modes — under `cargo nextest run`, which CI runs.

## dispatch-phase contracts

The keystone per-phase unit of autonomous execution is the **prose**
`rdm-dispatch-phase` orchestrator (`.claude/skills/rdm-dispatch-phase/SKILL.md`),
which runs the same procedure — `Plan → PlanReview → Implement → CodeReview` — in
the driving session and reaches the review pipeline through two `Workflow` calls
(`rdm-wf-plan-review` on the `plan/<slug>` document, `rdm-wf-review-refute-fix` on
`change/<sha>`).

> **These contracts predate that.** They were authored for the
> `rdm-wf-dispatch-phase` Workflow engine, which `agent-orchestrated-dispatch`
> phase 6 replaced and phase 7 **deleted** along with `lib/dispatch-phase.mjs`, the
> shipped template, the plugin-tree copy and the emission registration (see
> `docs/workflow-vs-prose-boundary.md` § "Retirement record"). The DATA SHAPES below
> are still live — the prose orchestrator produces the same `OUTCOME`, reads the
> same `PHASE_META`/`TASK_META` from the same `rdm … show --format json` commands,
> and `rdm-wf-review-refute-fix.js`'s `mode: 'code'` path composes the same
> `classifyOutcome` verdict. What is **historical** is every mention of the engine,
> its JS decision core, its args payload and its own harness
> (`scripts/verify-workflow-dispatch.sh`): read those as a record of how the shape
> came to be, not as a description of a file you can run.

### `PHASE_META`

What the Stage-0 mechanical fetch returns from `rdm phase show … --format
json`. *(Under the retired engine this had to be a Bash-capable sub-agent, because
the Workflow runtime has no `process`/`child_process`; the prose orchestrator runs
the command itself.)*

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

### `OUTCOME` (dispatch)

The per-phase verdict. Distinct from the review pipeline's
`OUTCOME` array above — this is the phase-level verdict consumed by `rdm-autopilot`
and by `rdm-do --auto`. Since `agent-orchestrated-dispatch` phase 6 the
prose `rdm-dispatch-phase` orchestrator produces this shape as its in-session
result (plus `planId` and `reviewIds`), and `rdm-wf-review-refute-fix.js`'s
`mode: 'code'` path with `{ roadmap, phase }` or `{ task }` composes the same
`classifyOutcome` verdict into the same shape for headless callers. It was
originally the top-level return of the retired `rdm-wf-dispatch-phase` engine; the
two harnesses that used to gate the `rdm-do --auto` → engine wiring
(`verify-workflow-do-auto.sh` and `-task.sh`) were deleted with that wiring.

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

The task-mode shape is unchanged by the phase-6 migration: the prose orchestrator returns `task` in place of `roadmap`/`phase` exactly as the engine did.

**`status` / `writesCompletion` carry the gate policy as data.** They are derived
from the canonical `statusFor` / `writesCompletion` in `lib/review.mjs`, so
consumers (autopilot's advance/park, `rdm-do --auto`, `rdm-land`) read the policy
off the OUTCOME instead of restating the mapping. `writesCompletion` is a
**boolean, never the trailer literal** — the stamped block may not contain that
string. `rdm-land` reads
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

**The code-review stage is the canonical review.** It is
`rdm-wf-review-refute-fix.js`'s stamped `buildReviewPipeline('code')` — there is no
independent code-review logic anywhere — fed the caller's reviewer set (see
`resolveReviewers(mode, reviewers)` above; omitting it runs them all).
The since-deleted `verify-workflow-review-outcome.sh` counted those two call
sites in the driver's source; the Rust `workflow_review` engine tests now
execute the driver instead.

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
`scripts/verify-agent-config-distribution.sh` § 7c drives the real emitted engine under a
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
beside `RDM_ROOT`/`RDM_PROJECT`. *(The retired `verify-workflow-dispatch.sh`
§ 9c-dogfood gated that entry with planted-mutation self-tests; the entry itself is
unchanged, and the project's development-build rule in `CLAUDE.md` is what states
it now.)*

Two rules survive the reversal unchanged:

- **An existence preflight is still forbidden.** The default must be a plain
  fallback, never a probe. `which -a rdm` resolves to the stale global, so a
  probe passes while running exactly the wrong binary — it would hide the
  dogfood hazard rather than close it. *(The retired
  `verify-workflow-dispatch.sh` § 9c greped every `resolveRdmBin`-bearing copy to
  prove no copy was implemented that way, with a planted-probe self-test.)*
- **An explicitly passed value still wins verbatim**, and the sentinel
  `rdmBin: 'rdm'` remains valid — it requests `PATH` resolution deliberately
  rather than falling into the default branch.

The wrong-type throw is deliberate: degrading a `rdmBin: 42` typo to `PATH` would
reintroduce exactly the silent-wrong-binary failure the absent-value default does
not need. Validation still runs in the surviving engines' own arg parsers, which
each execute as their driver's very first statement, so a mis-typed payload costs
zero tokens (the same discipline as `parseBudget`).

**Where `$RDM_BIN` is actually read.** Not here. The Workflow runtime has no
filesystem and no environment access, so no workflow JS reads `process.env` —
*(The retired `verify-workflow-dispatch.sh` § 9c-inventory asserted this across every
`resolveRdmBin`-bearing file, with a planted `process.env.RDM_BIN` self-test.)*
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
than a loud one. The verified `rdmBin`-threading shims are
`.claude/skills/rdm-dispatch-phase`, `.claude/skills/rdm-do` (both `--auto`
flows), `.claude/skills/rdm-autopilot`, and the shipped
`skill-dispatch-phase-cli.md` / `skill-do-cli.md` /
`skill-autopilot-cli.md` templates. All of them name `rdmBin`. The per-shim
grep that used to assert this (`verify-agent-config-distribution.sh` § 6d,
with a planted-removal self-test) was retired by the operator amendment to
`task/retire-static-grep-harnesses` (2026-09-23); it is now held by
convention and code review. *(Until
`agent-orchestrated-dispatch` phase 7 they threaded it into the
`rdm-wf-dispatch-phase` Workflow payload; the prose orchestrator uses the same
resolved value for its own Bash commands and for its two review-Workflow
payloads.)*

The `rdm-autopilot` shims were originally in that list only because of the
fail-closed rule on the one `rdm-wf-dispatch-phase` call payload. Their own
drive-loop prose has since been de-literalized too (phase 10 of
`project-agnostic-lane`): the skill parses an **optional** `--rdm-bin <path>`
(there is no pre-flight stop for it — the missing-roadmap-slug stop is unrelated
and stays) and an optional `--project <name>`, and threads them through every
Bash step it runs itself, as well as into both the `rdm-wf-estimate` and
`rdm-wf-dispatch-phase` payloads. `verify-skill-autopilot.sh` used to bound both
directions with static greps over the skill (each payload carries `rdmBin`; the skill
carries zero binary/project literals of its own); those greps were retired as not
behavioral coverage, and the script itself was deleted by
`rust-test-suite-consolidation` phase 3.

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
The since-deleted `verify-workflow-review-outcome.sh` § 6c pinned both directions: the standalone
path throws without `rdmBin` before any `agent()` call, while both legacy shapes
still succeed without it and still return `{ mode, survivors, budget }`.

Rewired callers: `.claude/skills/rdm-review` (the only caller of
`rdm-wf-review-refute-fix`; its invocation prose sits ABOVE the
`gen-skill-review.sh`-stamped region, and the shipped
`skill-review-cli.md` templates invoke no workflow at all, so
`lib/review.mjs` is never opened) and `.claude/skills/rdm-estimate` (the only
caller of `rdm-wf-estimate.js`; `skill-estimate-cli.md` remains the `{proj_flag}`
prose rating loop and needs no change). Asserted per-shim by
the since-deleted `verify-workflow-review-outcome.sh` § 4 and the since-deleted
`verify-workflow-estimate.sh`'s HOIST-SHIM section, each with a planted-typo self-test;
the allow-list was asserted AS DATA by the same two harnesses' driven prompt captures
(§ 6b / § 9b). Their behavioural successors execute the returned commands against the
real binary instead (for example `workflow_passes::estimate::engine_commands_use_injected_axes`
and `workflow_review::plan_driver::injected_axes_reach_executed_commands`).

**Bounded consequence, recorded rather than absorbed.** The prose
`rdm-autopilot` skill's estimate pre-pass passes no `rdmBin` yet — that payload,
and the loop's own literals, belong to the phase that parameterizes the prose
loop — so that one call throws until it is threaded. It does **not** break the
lane: the skill's own prose already says to log a warning and continue into the
drive loop non-fatally on an estimate error, and unrated phases simply dispatch
at whatever tier `rdm next` reports. Its `rdm-wf-dispatch-phase` payload was threaded
above and is unaffected.

## Verify gate

> **Historical, like "dispatch-phase contracts" above:** every `rdm-wf-dispatch-phase`
> mention in this section refers to the engine `agent-orchestrated-dispatch` phase 7
> deleted. The gate itself is live — the prose `rdm-dispatch-phase` orchestrator runs
> the same command and produces the same `VERIFY_RESULT`.

`rdm-wf-dispatch-phase` runs the project's single declared verification command once per
implementation attempt. Canonical write-up — what the command is, how it is resolved, the
not-a-task-runner non-goal, and the commit-time vs phase-time split — lives in
[`verify-gate.md`](verify-gate.md); this section records only the schema and the contract
the workflow layer owns.

**`VERIFY_RESULT`** — what the mechanical verification agent returns:

```
{ exitCode: <integer>, output: <string> }   // both required; output is the LAST 4000 chars
```

**Label:** `verify:run`, phase `Implement` (it reuses the existing phase title, so
`meta.phases` is unchanged), model `models.mechanical`.

**Contract:**

- **Exactly once per implementation attempt.** `runCodeGate` owns a single call site, fused
  with `d.implement(...)`, so the first pass and every rework round route through the same
  line. The gate returns `verifyCalls` and `verifyRounds` so the count is observable from
  outside without instrumenting a fake.
- **Fail-closed.** A thrown dep, a `null` resolution, or a payload with no integer exit code
  are all treated as a non-zero exit.
- **No new vocabulary.** A failure is folded in as a synthesized blocking finding, so the
  untouched `classifyOutcome`/`statusFor`/`writesCompletion`/`GATE_POLICY` resolve it to
  `rework`. The bound is the existing `maxCodeRework`; there is no second counter.
- **Escalate on unresolvable.** The command is resolved at Stage 0 as an optional `verify`
  string on `PHASE_META`/`TASK_META` (declared key first, then discovery). When it resolves
  empty and the run is not `--plan-only`, the driver short-circuits to the existing
  `escalated` outcome before planning — a dispatch that cannot determine how to verify
  itself must not report success.
- **Hoistable string, irreducible run.** `hoistedMetaComplete` requires the `verify` field,
  so a caller hoist forwards it; the RUN itself is state-dependent and cannot be hoisted.

## autopilot contract

Autopilot is now the prose `rdm-autopilot` skill
([`docs/autonomous-loop.md`](./autonomous-loop.md)); it consumes the same
dispatch-phase OUTCOME contract below (`statusFor`/`writesCompletion`) rather
than defining its own, and its own advance/park Bash steps are what persist the
terminal status per "The core fix" below — dispatch-phase itself never does.

> **Historical, like "dispatch-phase contracts" above:** every `rdm-wf-dispatch-phase`
> mention in this section and its subsections ("The core fix", "`planOnly`") refers to
> the engine `agent-orchestrated-dispatch` phase 7 deleted. The advance-off-persisted-status
> rule and the plan-only early return are live — the prose `rdm-dispatch-phase`
> orchestrator carries both.

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

The review core's hermetic gate is the Rust `workflow_review` test binary
(`rdm-cli/tests/workflow_review/`), run by `cargo nextest run`. It executes the
real JavaScript under Node through `rdm_devtools::workflow`, whose only
test-side JavaScript is generic execution glue: every scenario, scripted
fake-agent reply and assertion is Rust, and the fake `parallel` primitive is
documented in that module. Node is resolved from `RDM_TEST_NODE`, `PATH`, or
`mise which node`, and a missing runtime fails the run rather than skipping it.
See `docs/test-migration-inventory.md` § "Phase 2".

## Optional caller-supplied args (mechanical-agent hoists)

> **Historical, like "dispatch-phase contracts" above:** every `rdm-wf-dispatch-phase`
> row and mention in this section and its subsections — the hoist table, "absorbs its
> diff", "Which caller surfaces supply them today" — refers to the engine
> `agent-orchestrated-dispatch` phase 7 deleted; the prose `rdm-dispatch-phase`
> orchestrator runs in the driving session with Bash and hoists nothing. The rows for
> the surviving workflows (`rdm-wf-estimate`, `rdm-wf-plan-review`, `rdm-wf-backlog`,
> `rdm-wf-document`, `rdm-wf-review-refute-fix`) are live.

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
| `rdm-wf-review-refute-fix` | `diff` (untrusted compatibility input) | `source:resolve` always runs | committed content reacquired from validated source |

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

`hoistedMetaComplete(meta, isTask)` (in the retired `.claude/workflows/lib/dispatch-phase.mjs`)
accepted a payload only when the `body` is a non-empty string, all five model
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
requirement. *(The retired `scripts/verify-workflow-dispatch.sh` §6a covered both
directions: four negative cases — absent / empty / blank / non-string tier — falling
back to the agent, and a positive pair proving a hoisted `large` and a hoisted
`medium` produce different outcomes from one identical concern seed.)*

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
`buildReviewUnits` requires**: a `body`, and a `tags` value that is either a
real string array or (task `fix-plan-review-gate-tag-clobber`, "the one
blocking defect that remains") entirely absent — per phase entry as well.
The absent case is not laxity: `rdm ... show --format json` never prints
`tags: []` for an untagged item, it omits the key outright
(`Option<Vec<String>>` / `skip_serializing_if(Option::is_none)` in
`rdm-core/src/json.rs`), so treating omission as corruption meant an
untagged item could never be plan-reviewed. `tagsOk`/`normalizeTags` accept
the omission and normalize it to a real `[]`; a `tags` key that IS present
but malformed (not a string array) is still rejected outright — that is not
shape pedantry, since the gate writes the list back with `rdm ... update
--tags "<list>"`, and `--tags` **replaces** the whole list. A payload
accepted with a malformed `tags` value could be written verbatim and
corrupt the item's real tags. Rejecting a malformed value costs one fetch
agent; accepting it costs the item's tags. Accepting a merely-absent value
costs nothing.

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
the since-deleted `scripts/verify-workflow-review.sh` §7g/§7h for the regression coverage
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
heading) for the full account and the since-deleted `scripts/verify-workflow-review.sh`
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
  skill shims (`rdm-core/src/templates/skill-{dispatch-phase,do}-cli.md`)
  and their local copies.
- **`rdm-wf-plan-review`, `rdm-wf-backlog`, `rdm-wf-document`, `rdm-wf-review-refute-fix`, `rdm-wf-estimate`** — supplied
  only by this repo's **local** `.claude/skills/*/SKILL.md` dogfood copies. Their
  distributed templates are not yet Workflow shims; converting them is tracked by task
  `convert-remaining-skill-templates-to-workflow-shims`.


## Source-bound standalone code review

`rdm-wf-review-refute-fix` resolves the intended phase/task through `rdm review source` before every review, even when a caller supplies `diff`. Optional input keys are `source` (registered checkout), `base`, `expectedHead`, `expectedBranch`, `noCode`, and `implements` (`plan/<approved-plan>`). Phases use the shared roadmap checkout; tasks may bind an explicit registered shared checkout with an explicit base. Default bases are pinned merge bases with the configured default branch. Missing checkout, wrong repository/branch, moved expected head, and unexpected empty ranges refuse review. `noCode: true` is an explicit empty-diff declaration, not permission to omit acceptance review.

The returned `source` contains canonical item/repository/path/branch, full base/head, committed changed files and diff, and the no-code declaration. Every finder/refuter receives this identity and acceptance text. Supplied diffs are currently reacquired rather than trusted as an optimization. Persistence revalidates and runs in that checkout, targeting `change/<full-head> --base <full-base> --implements plan/<slug>`; omitting `implements` retains core's approved-plan inference. Source-bound approval never falls back to an item-document target. Explicit `persist.on` must equal the pinned change target. Failed persistence or failed status write/readback escalates. `review pending --format json` exposes `review_sha` alongside `branch` for stamp verification.

Automatic approval consumes the latest attempt's coverage, structured nonempty valid AC table, refutation overflow and grader errors. Missing selected dimensions, absent/invalid AC results, over-budget grading candidates (including concerns at small tiers), and failed grading produce `escalated` with `writesCompletion: false`; suggestions intentionally passed through remain non-gating. Existing severity thresholds, confidence floor 70 and refutation cap are unchanged. Historical incomplete attempts remain visible but a complete later attempt can approve. Legacy survivors-only calls remain reports and do not issue automatic approvals. The retiring dispatch driver's bespoke orchestration is unchanged.

The workflow's source validation and a plan Store write are not a transaction across repositories. Revalidation at acquisition, persistence and status boundaries detects drift; an external concurrent git mutation between checks remains possible. No live Claude Workflow execution was performed by the Codex implementation host; real Git/CLI fixtures and injected workflow agents supply regression evidence.


### Source-bound acceptance and implementation-plan identity

The standalone code review reads the intended item's complete body and builds
an authoritative criterion inventory before invoking finders. A single Markdown
heading named **Acceptance Criteria** (or **Acceptance**) defines the section,
ending at the next heading of equal or higher level. Supported criteria are
top-level bullets (including checkboxes), numbered list items, or prose
paragraphs separated by blank lines. Wrapped and indented nested lines remain
part of their parent criterion. Whitespace is normalized; each entry receives
the identity `AC<n>: <complete criterion text>`, in document order. Finder
results must repeat these identities verbatim, exactly once each, with valid
status and evidence. Missing, duplicate or unknown identities cannot approve.
Missing/empty sections, duplicate criteria, tables, fenced blocks and ambiguous
subheadings fail closed; they need explicit criteria before automatic review.
The fetched body is never replaced with a finder-created criterion subset.

Before reviewing code, the standalone driver loads the explicit implementation
plan (or resolves exactly one approved plan implementing the canonical source
item), verifies its approved status and exact `implements` relationship, and
pins its slug, relationship and body. It revalidates that plan before persistence
and gating, even when no gate was requested. Persisted reviews use only that
validated plan reference. Legacy report-only and plan-mode calls retain their
existing behavior.
