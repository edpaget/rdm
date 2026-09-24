# The mechanical-agent sweep

**There are no mechanical agents in `.claude/workflows/`.** A workflow contains
judgment agents and nothing else.

This document is the record of the sweep that established that — the live grep,
every call site it found, and what happened to each one. It replaces the census
that used to live here, which inventoried the mechanical agents in order to
*shrink* them; the `no-mechanical-agents-in-workflows` phase removed them
instead.

Related: [`docs/workflow-schemas.md`](workflow-schemas.md) (the argument and
result contracts the removal produced), [`docs/autonomous-loop.md`](autonomous-loop.md),
[`docs/workflow-vs-prose-boundary.md`](workflow-vs-prose-boundary.md).

## The rule

> Every mechanical act — reading an rdm document, resolving a model, persisting a
> review, clearing a tag, filing a task, writing a round note — belongs to the
> ORCHESTRATOR, which has Bash. A judgment agent that needs a document fetches it
> itself and reads it in its own context, where it is never re-emitted and so
> cannot drift.

Two corollaries do the work:

1. **The orchestrator passes identifiers, never documents.** A slug, a stem, a
   list of stems, a SHA, a path, a tag list. Short values cross an agent boundary
   intact; bodies do not.
2. **A workflow with no shell-running agent cannot write.** So the engines
   compute and hand back; the orchestrator executes.

## Why — one failure class, five instances

Short values cross an agent boundary intact; documents do not.

| instance | what was transported | how it failed |
|---|---|---|
| `task/code-review-plan-stability-guard-trips-on-transcription-drift` | a 32 kB plan body, read twice | the two reads differed by one trailing newline, in opposite directions on two runs. Two consecutive real reviews aborted; ~855k subagent tokens; no review either time |
| `phase-28-code-finders-omit-structured-path` | a structured `path` field | silently dropped, so zero anchors landed across two reviews |
| `phase-32-engine-reads-plan-by-slug` | a 19,717-character plan body | made mandatory in the orchestrator's own output; superseded |
| `task/estimate-workflow-reports-tier-as-json-blob` | a tier string | came back a JSON object, for one phase of five |
| `fetch:roadmap-intent` | a 19,559-character roadmap body | the `## Intent` section did not survive the trip, `extractIntent` found nothing, and the path **fail-softed** to "no recorded intent" — so three consecutive plan reviews silently ran without the reviewer that checks work against recorded operator intent |

The last one is the important one precisely because it was quiet. The others
announced themselves. A mechanical agent that fails loudly costs tokens; one that
fail-softs costs coverage, invisibly, for as long as nobody checks.

## The sweep

The live grep, re-runnable:

```
grep -n "label: *['\"]" .claude/workflows/*.js .claude/workflows/lib/*.mjs
```

Every site it found, and its disposition. **Moved to the orchestrator** means the
engine returns identifiers or command text and the caller acts. **Moved into a
judgment agent** means the reviewer now runs the command itself, from its own
prompt. **Deleted** means the thing it did stopped being done at all.

### Reads

| site | engine | disposition |
|---|---|---|
| `source:resolve` / `source:revalidate` | review-refute-fix | **moved into a judgment agent** — each reviewer runs `rdm review source --on <item> --format json` itself and reviews the `base..head` it reports. The caller passes the pinned identity (path, base, head, branch) as arguments |
| `source:acceptance` | review-refute-fix | **moved into a judgment agent** — the `ac` reviewer runs `rdm phase show` / `rdm task show --format json` and takes the criteria from the body it read |
| `plan:resolve` / `plan:revalidate` | review-refute-fix | **moved into a judgment agent** (`rdm plan show`); the plan ref is a caller argument, and the auto-discovery `plan list` read is **deleted** — the real binary already infers a single approved plan at persist time |
| `fetch:roadmap` (+ its per-phase fan-out) | plan-review | **moved to the orchestrator** — it passes the phase stems to sweep as `phases`. With none, the roadmap document is reviewed alone |
| `fetch:roadmap-body-check` | plan-review | **deleted** — a second read whose only job was to confirm the first |
| `fetch:phase` / `fetch:task` | plan-review | **moved into a judgment agent** (`rdm phase show` / `rdm task show`) |
| `fetch:plan` | plan-review | **moved into a judgment agent** (`rdm plan show`), named by `planSlug` |
| `fetch:plan-body-check` | plan-review | **deleted** — the same second-read pattern, added by the superseded phase 32 |
| `fetch:roadmap-intent` | plan-review | **moved into a judgment agent** — `intent-alignment` reads the parent roadmap's `## Intent` itself, from the roadmap-show command its prompt names |
| `fetch:wontfix` | plan-review | **moved to the orchestrator** — it runs the one `rdm search` and passes the titles as `wontFixedTexts` |
| the tags read behind `snapshotOriginalTags` | plan-review | **moved to the orchestrator** — it passes the item's current tag list as `tags`; a unit whose tags were not supplied gets no gate commands rather than a `--tags ""` that would drop a sibling |
| the prior-review read riding in the fetch transcript | plan-review | **moved to the orchestrator** — it runs `rdm review list --on <ref>` and passes `priorReviews` |

### Model resolution

| site | engine | disposition |
|---|---|---|
| `model:mechanical` | plan-review | **deleted** — there is no mechanical model left to resolve. `findModel` / `verifyModel` are caller arguments, each independently optional; an omitted one lets that agent inherit the session model |

### Writes

| site | engine | disposition |
|---|---|---|
| `persist:review` | review-refute-fix | **moved to the orchestrator** — the engine returns `persistCommands` / `persistScript`, built by the same pure `persistReviewCommands` the agent used to wrap |
| `persist:review:*` | plan-review | same, per unit |
| `gate:persist` | review-refute-fix | **moved to the orchestrator** — returned as `gateCommands` / `gateScript` |
| `gate:clear-tag:*` | plan-review | **moved to the orchestrator** — returned as `gateAction.commands` |
| `act:*` | plan-review | **moved to the orchestrator** — applying a small plan fix and filing a large finding as a task is judgment plus a write, which is what the orchestrator is |
| `act:round-note:*` | plan-review | **moved to the orchestrator** — the engine renders the note and returns it as `roundNote`, which the orchestrator appends with a read-modify-write in Bash. That read-modify-write costs nothing there: the body never leaves the shell, so it is neither transcribed nor at risk of a dropped line. Turning `roundNote` into a returned command ladder like the estimate writeback's is an open follow-up, not a claim made here |

### The three batch engines

| site | engine | disposition |
|---|---|---|
| `model:mechanical` | estimate, document, backlog | **deleted** — with no mechanical agent left, there is no mechanical model to resolve |
| `estimate:list` | estimate | **moved to the orchestrator** — it runs `rdm phase list --format json` and passes `phaseList`; the engine refuses to run without it and returns the command to use |
| `estimate:write:<stem>` | estimate | **moved to the orchestrator** — the engine returns each phase's `writebackCommands`: ONE `phase update --difficulty` command. Runnable exactly as emitted, and it holds no document at all — estimation writes nothing to the item body, so nothing crosses a model boundary and the ladder cannot destroy what it never held |
| `estimate:tier:<stem>` | estimate | **deleted** — rdm-core derives the tier from the difficulty the writeback sets (`Difficulty::model_tier`); nothing reads it back |
| `fetch:roadmap-meta` | document | **moved to the orchestrator** — it runs `rdm roadmap show --format json` and passes `roadmapMeta`; the engine refuses to run without it and returns the command |
| `gather:<stem>` | document | **kept, reclassified** — it reads a phase document and its commit's history and JUDGES what the change did. That is not transcription, so it lost its `agentType` and its mechanical-model pin rather than being moved. It also no longer RETURNS the phase body: reclassifying the agent answered what class it is, not whether a document crossed its boundary, and one did — `PHASE_RECORD` required the body verbatim and the engine re-emitted every one into the synthesis prompt. `PHASE_RECORD` now carries the gatherer's own `shipped` account and no body at all, and the synthesizer is handed each phase's `showCommand` and reads the document itself |
| `write:draft` | document | **moved to the orchestrator** — the engine returns `writeCommands` / `writeScript` (mkdir, then a quoted heredoc) |
| `fetch:report` | backlog | **moved to the orchestrator** — it runs `rdm backlog report --format json` and passes `report`; the engine refuses to run without it and returns the command. The propose-only contract is unaffected: that command is read-only whoever runs it |

`spike-agent-type.js` was **deleted**. Its whole subject was
`agentType: 'rdm-mechanical'` and `effort:` on a mechanical call site, and its
result is recorded in `docs/workflow-schemas.md` § "agentType / effort options
spike". Carving an exemption into the rule for it would have been the wrong
trade.

The custom agent definition `.claude/agents/rdm-mechanical.md` itself is kept —
it is a registry entry, still emitted downstream by
`rdm agent-config claude --skills`, and a future non-workflow caller may want it.
Nothing under `.claude/workflows/` references it any more.

### What went with them

The apparatus that existed only to police a transported value, all deleted:
`RAW_STDOUT_SCHEMA`, `ROADMAP_BODY_CHECK_SCHEMA`, `WONTFIX_LIST_SCHEMA`,
`STAMP_ACK_SCHEMA`, `PERSIST_ACK_SCHEMA`; the transcript parser and every
`extract*FromJson` identity validator; `assembleRoadmapFetchFromTranscript`;
`hoistedFetchedOk` / `fetchTranscriptionOk` / `RESERVED_FETCH_TOKENS`;
`roadmapBodyVerified`; `buildPersistReviewPrompts`, `persistAccounting`,
`classifyPersistOutcome`, `degradationSummaryClause`; and the gate's
classifier-persuasion prose (`buildGateEvidence`, `renderGateEvidence`,
`gateTwoPartyClause`, `buildTagWritePrompt`) — there is no agent left to
authorize.

The caller-hoist arguments that existed to route *around* those agents went too:
`fetched`, `roadmapBody`, `planText`, `mechanicalModel`, `gateMode`.
`wontFixedTexts` stayed — it is a list of short titles, the orchestrator's to
supply, and it is now the only path.

## The cost trade, stated

N reviewers each make one cheap Bash call inside a context they were loading
anyway, replacing a dedicated mechanical agent that cost ~27–29k tokens to make
the same call once. On a review that runs several reviewers this is not obviously
cheaper in tokens. It is bought for correctness: the document is read once, in the
context that uses it, and is never re-emitted — which is the only property that
makes the five failures above impossible rather than unlikely.

## How this is enforced

Not by a grep. `rdm-cli/tests/workflow_review/plan_driver.rs` (under
`cargo nextest run`) drives the real plan-review driver, and the shipped
`rdm-wf-plan-review.js` engine, over every target kind against a recording fake agent
and asserts the **dispatched label set contains only finder and refuter labels**
(`plan_driver::every_target_dispatches_only_finders_and_refuters`,
`plan_driver::shipped_engine_dispatches_only_finders_and_refuters`).
The equivalent claim for the code engine is decidable from its diff — it is a
top-level script with no importable surface, and adding a harness for it was
deliberately declined.
