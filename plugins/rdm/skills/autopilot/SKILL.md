---
name: autopilot
description: Drive one named rdm roadmap from not-started to reviewed autonomously — pick the next actionable phase, dispatch it at its current model tier, interpret the outcome, and advance — batching decisions and blockers instead of interrupting
allowed-tools:
  - Bash
  - Workflow
---

Drive **one** rdm roadmap from `not-started` to `reviewed` with no per-phase human approval. This skill drives the loop **itself**, in prose: the one Workflow-tool call it makes is `rdm:rdm-wf-dispatch-phase` (one phase's plan → plan-review → implement → code-review pipeline). Every other step — fetching `rdm next`, persisting an advance or a park, reading a write back to confirm it landed — is a plain Bash command this skill runs directly in its own context, because it is already a live agent with Bash access and the repo in context. See [`docs/workflow-vs-prose-boundary.md`](docs/workflow-vs-prose-boundary.md) for why the drive loop specifically (a low-iteration sequential policy driver) stays prose while `rdm:rdm-wf-dispatch-phase` stays a Workflow script (real fan-out over a fixed mechanism).

Decisions and blockers are **batched, not raised mid-run**: a phase that cannot be advanced is parked `blocked` and the run keeps making progress on the rest, so the user answers the whole queue at once at the end rather than being interrupted per phase.

## Contract

**Input** (`$ARGUMENTS`): a **required roadmap slug**, optionally followed by `--max-phases N`, `--plan-only`, `--max-plan-revise N`, and/or `--max-code-rework N`. The slug names the single roadmap this run drives. If no slug is given, stop before invoking anything and say so — do not attempt a partial drive-loop start.

**The four guardrails, together, in one place:**

1. **Single roadmap.** The loop never roams to another roadmap — choosing which roadmap to advance stays a human decision. Every `rdm next` / `rdm phase update` / `rdm phase show` command below is scoped with the **same fixed** `--roadmap <slug>` throughout this run; nothing ever substitutes a different one.
2. **`main` is never touched.** Autopilot leaves every reviewed phase on the `roadmap/<slug>` branch; landing to `main` is the separate **`land`** skill. There is no `--land` flag here, and no Bash command in this loop ever runs `git checkout`/`merge`/`rebase` against `main`.
3. **No `Done:` trailer.** This skill never emits a `Done:` line: its advance step only persists the status the OUTCOME carries, directly via `rdm phase update --status <status> --no-edit` (a status-only write — it never stages or writes a commit message). **`land` is the land-time writer** — it reads the OUTCOME's `writesCompletion: true` and synthesizes the trailer from the item's identifiers via `rdm hook done-line`, amending it onto the branch tip before the rebase.
4. **`--permission-mode auto` for unattended runs.** Launch with `--permission-mode auto` (or `bypassPermissions` in a sandbox) so this skill's own Bash commands and the `rdm:rdm-wf-dispatch-phase` Workflow call don't block on a permission prompt.

This skill is **non-interactive**.

## What to do

### 1. Parse `$ARGUMENTS`

- `roadmap` — the required slug (first positional argument). Missing → stop immediately, before step 2, and say so.
- `maxPhases` — the positive integer following `--max-phases`, when present (omit otherwise — unbounded by phase count).
- `planOnly` — `true` when `--plan-only` is present (omit otherwise).
- `maxPlanRevise` — the non-negative integer following `--max-plan-revise`, when present (omit otherwise — `rdm:rdm-wf-dispatch-phase` applies its own default of 2). `0` is legal and distinct from unset: it means "terminate on the first blocking plan review, no revise round at all".
- `maxCodeRework` — the non-negative integer following `--max-code-rework`, when present (omit otherwise — same default of 2, same `0`-is-legal rule).
- `globalBudget` — **not** a user-facing flag. It stays an internal constant, `DEFAULT_GLOBAL_BUDGET = 50`, hardcoded in this loop (see step 3).

### 2. Fetch the initial phase cursor

- Run `rdm next --roadmap <slug> --format json --project <PROJECT>` and take the parsed object verbatim as `next`. This loop consumes this **one-shot, on the first loop iteration only**; every later iteration re-reads live state via the same command directly, because `rdm next` is what steps the cursor forward once a phase's status is persisted. Fetch it fresh at invocation time and never cache it across runs.

**Why no estimate pre-pass here:** unlike the local dogfood `autopilot` skill, this shipped template never invokes an `rdm-wf-estimate` Workflow before dispatching phases. The generated workflow directory downstream never contains an `rdm-wf-estimate` script: it references `agentType: 'rdm-mechanical'`, and a downstream repo receives no `.claude/agents/` registry at all, so an unresolved `agentType` there would raise rather than degrade silently — the same hazard `autopilot.js` itself was retired for. Lifting that is out of scope here; it belongs to the separate `ship-mechanical-agent-type-downstream` task. Every phase this loop dispatches therefore runs at whatever tier `next.model` already reports (default `medium`), never freshly rated first — a deliberate, permanent divergence from the local dogfood skill. See [`docs/workflow-vs-prose-boundary.md`](docs/workflow-vs-prose-boundary.md) for the full decision record.

### 3. Enter the drive loop

Maintain, in this skill's own working context (nothing here is persisted by rdm): `dispatchCount = 0`, `completed = []` (ordered), `escalations = []` (ordered), and — **only when `planOnly` is set** — a `planOnlySeen` set of stems already plan-vetted this run. There is no "seen" tracking in normal mode; normal-mode progress is driven purely by the persisted phase status that an advance/park write changes, which `rdm next` reads to step forward.

Loop:

1. **Budget check.** If `dispatchCount >= 50` (the internal `DEFAULT_GLOBAL_BUDGET`) or (`maxPhases` is set and `dispatchCount >= maxPhases`), stop with `stopReason: budget` and go to step 4. **This one counter is shared** between the global cap and `--max-phases`, and it increments on every dispatch — including a `rework` re-dispatch of the *same* phase, not just on distinct phases. `--max-phases 3` can therefore stop the run after as few as **one** distinct phase if it reworks twice. This is genuinely surprising but is the preserved, existing semantic — do not smooth it over when explaining a run to the user.
2. **Fetch the next phase.** Use the hoisted `next` value from step 2 on the very first iteration only; every later iteration, run `rdm next --roadmap <slug> --format json --project <PROJECT>` directly via Bash and read its JSON output yourself.
3. **Classify the result** (mirrors `interpretNext`):
   - `result: "phase"` with a non-empty `stem` → work it (go to 4).
   - `result: "phase"` **missing** `stem` → this is malformed. Stop with `stopReason: unparseable` — **never** treat a malformed or unrecognized payload as `"nothing"` (that reason is reserved for a genuine, well-formed "no actionable phase" answer). Record a summary-only escalation `{ stem: "(fetch:next)", reason: "[fetch] unparseable rdm-next payload: <bounded description of the raw output>" }` — since no phase stem is known at this point, this entry is never parked via `rdm phase update` and will **not** appear in `rdm review blocked`; it only ever appears in this run's printed summary.
   - `result: "blocked-on-dependencies"` → stop with `stopReason: blocked-on-dependencies` (well-formed, known-good).
   - `result: "nothing"` → stop with `stopReason: nothing` (well-formed, known-good).
   - Go to step 4 for any stop.
4. **Work the phase**, stem `S`, tier `T` (`next.model`, defaulting to `medium` when unset):
   - Under `--plan-only`, if `S` is already in `planOnlySeen`, stop with `stopReason: plan-only-exhausted` and go to step 4 — a plan-only pass never advances or persists a terminal status, so `rdm next` will keep returning the same stem forever; this in-memory check is what detects the repeat (there is no rdm-side marker to lean on).
   - Otherwise, before invoking the Workflow, fetch phase-meta **yourself**, directly via Bash — this mirrors `buildFetchPrompt` and, when it succeeds, skips `rdm:rdm-wf-dispatch-phase`'s own Stage-0 fetch agent entirely for this dispatch (that agent is the one call in the whole lane with no `model:` key, since it runs at whatever tier resolves the other five). Run `rdm phase show --roadmap <slug> S --project <PROJECT> --format json` and read `stem`, `model` (the difficulty tier, call it `T`), and `body` from its JSON. If the command fails or `body` is empty or missing, abandon this whole procedure and invoke the Workflow below with **no** `phaseMeta` key at all — never forward a partial object; `hoistedMetaComplete` rejects anything short of complete, so a partial payload only wastes the read. Then run `rdm roadmap show <slug> --project <PROJECT> --format json` and keep that JSON's `body` field VERBATIM as `roadmapBody` — the parent roadmap's `## Intent` section lives there, and it is the plan gate's ONLY intent source on this path, so omitting it silently disables the `intent-alignment` dimension for every phase you dispatch. This one field is the single exception to the all-or-nothing rule above: `PHASE_META_SCHEMA` marks it optional, so if that command fails or has no body, omit `roadmapBody` and carry on rather than abandoning the hoist.
   - Otherwise, resolve the five per-step model ids per `buildFetchPrompt`'s exact rule: when `T` is a non-empty string, run `rdm model resolve plan --tier T` and `rdm model resolve implement --tier T`; when `T` is empty or missing, run those same two with no `--tier` at all. Always run `rdm model resolve review-find`, `rdm model resolve review-verify`, and `rdm model resolve mechanical`, all three with no `--tier`, regardless of `T` — none of these five model-resolve calls ever takes a project flag. If any of the five commands fails or prints nothing, abandon this whole procedure and invoke the Workflow below with no `phaseMeta` key. Then run `rdm config get dispatch.verify --raw` and keep a non-empty printed line verbatim as `verify` (`--raw` prints the bare value with no `(source: ...)` annotation, and nothing at all when the key is unset); if it prints nothing or fails, abandon this whole procedure and invoke the Workflow below with no `phaseMeta` key — only the workflow's own Stage-0 agent knows how to discover a verification command from CI config, `docs/principles.md`, or `CLAUDE.md`/`AGENTS.md`. Then run `rdm dispatch directives --format json` and forward its `directives` and `skipped` arrays as `directives` and `directivesSkipped`, copying every `text` value character for character; **unlike `verify`, an empty result or a failed command must NOT abandon the hoist** — absent directives are normal, so just omit both keys (see `docs/project-directives.md`). Otherwise assemble `phaseMeta = { roadmap: <slug>, phase: S, stem, model: T, body, roadmapBody, verify, directives, directivesSkipped, models: { plan, implement, review_find, review_verify, mechanical } }` from exactly the values just gathered — field-for-field what `PHASE_META_SCHEMA` requires, plus the optional `roadmapBody`, `directives` and `directivesSkipped` (drop any of those keys entirely if its read failed or came back empty; never drop the hoist over one).
   - Invoke the **`rdm:rdm-wf-dispatch-phase` Workflow** via the Workflow tool with exactly `{ roadmap: <slug>, phase: S, planOnly, rdmBin, project, phaseMeta }` — omit the `phaseMeta` key entirely when the sub-step above didn't complete — plus `maxPlanRevise` and/or `maxCodeRework` **only when this run's `$ARGUMENTS` set them**. `rdmBin` is the rdm executable you invoke (the same one the commands in this loop use) and is optional: omit it and a plain `rdm` on `PATH` is used. An explicitly passed value always wins verbatim. If a "Resolving `rdmBin`" section is appended to this skill, it is the single authoritative resolution order — follow it and do not re-derive one here. `project` is the project name used in `--project <PROJECT>`.
   - Read the returned OUTCOME object's `outcome`, `status`, and `reason` fields.
   - **Interpret the outcome** (mirrors `interpretOutcome`):
     - `outcome: "reviewed"`, **not** plan-only → **advance**: run `rdm phase update S --status <OUTCOME.status || reviewed> --no-edit --roadmap <slug> --project <PROJECT>`, then read it back with `rdm phase show S --roadmap <slug> --format json --project <PROJECT>` and confirm `status` matches. Retry the write+read-back up to **2** times total. On success: append `S` to `completed`, log `"phase S reviewed — advancing"`, continue the loop from step 1. On repeated failure: park `S` (below) with reason `"[code] advance to reviewed failed repeatedly"` — never report a false completion.
     - `outcome: "reviewed"`, plan-only → **noop-vetted**: add `S` to `planOnlySeen`, append `S` to `completed` (a plan-only pass records a vetted phase as completed, same bucket), log `"plan-only vetted S"`, continue the loop from step 1.
     - `outcome: "rework"` → if this phase's own rework count so far is **<** 1, increment it and **retry**: dispatch the **same** stem `S` again (go back to the dispatch call above, still counting against the shared budget in step 1) — do not call `rdm next` again first. Once the count reaches 1, **park** with reason `"[code] rework budget exhausted"`.
     - `outcome: "escalated"` → **park** with `OUTCOME.reason` if present, else `"[plan] dispatch escalated at the plan gate"`.
     - Anything else (a corrupted or unrecognized OUTCOME value) → **park** with reason `"[code] unrecognized dispatch outcome: <value>"` — never silently advance or silently stop.
   - **Park**: run `rdm phase update S --status blocked --reason "<reason>" --no-edit --roadmap <slug> --project <PROJECT>`, then read it back with `rdm phase show S --roadmap <slug> --format json --project <PROJECT>` and confirm `status: blocked`. Retry up to **2** times total. Whether or not the read-back ever confirms, append `{ stem: S, reason }` to `escalations` and continue the loop from step 1 — an unconfirmed park write must never abort the run before it can print its summary; log a loud warning in that case instead (the plan-repo status may not reflect the park, but the escalation is still recorded here).
5. **Stop.** Exit the loop and proceed to "Print the summary" below.

### 4. Print the summary

Compose this yourself, in this exact structure (mirrors `buildSummary`), and print it verbatim as your final message — do not paraphrase or truncate it:

```
autopilot summary for roadmap/<slug>
<one of the two lines below>
phases completed (<n>): <stem, stem, ... or "none">
escalations awaiting review (<n>): <"none", or the block below>
  - <stem> [<stage>]: <reason>
  ...
review the queue: rdm review blocked --project <PROJECT>
[note: ... only if a [fetch]-tagged escalation is present, see below]
reviewed work is left on the roadmap/<slug> branch; main is never touched.
```

- The stop-reason line is either `stop reason: <reason>` (a known-good reason) or, for anything else, the loud `*** ABNORMAL TERMINATION (stop reason: <reason>) — the roadmap was NOT driven to exhaustion; do not read this as a completed run.` The **known-good allowlist** is exactly: `nothing`, `blocked-on-dependencies`, `budget`, `plan-only-exhausted`. Anything else — including a hypothetical future reason, or literally `unparseable` — gets the abnormal banner, never the plain line. Treat this as an allowlist, not a denylist keyed on one literal, so an unrecognized future reason is fail-safe flagged rather than silently trusted.
- `phases completed` lists stems in the order they completed (advance or noop-vetted), comma-joined, or `none`.
- `escalations awaiting review` renders each entry as `<stem> [<stage>]: <reason>`, where `<stage>` is parsed from the reason's leading `[tag]` (defaulting to `code` if unparseable), followed by the `rdm review blocked` pointer line.
- If **any** escalation is tagged `[fetch]`, add the loud note: `note: [fetch]-tagged entries above are summary-only — no phase is known to park against, so they will NOT appear in the` `rdm review blocked` `queue; act on them directly from this summary.` (Only a fetch-stage escalation, from step 3.3 above, is ever fetch-tagged — `[plan]`/`[code]` escalations always have a real stem and are genuinely queued.)
- The closing line is always present, regardless of outcome.

## Run modes

- `--max-phases N` — bounded run: dispatch at most `N` phases this pass, then stop and summarize. Use it to take a roadmap a few phases at a time.
- `--plan-only` — dry-run the planning half: each dispatch stops after its plan gate, so you get cheap plan vetting without writing any code.
- `--max-plan-revise N` / `--max-code-rework N` — override `rdm:rdm-wf-dispatch-phase`'s two **in-run** retry budgets, which are counted **independently** of each other and default to **2** each (budget N = N reworks after the original attempt, i.e. N + 1 attempts). `0` is legal and means "terminate on the first blocking review" — no revise/rework agent runs at all. These are distinct from autopilot's own roadmap-level rework re-dispatch budget (step 3, capped at 1 retry per phase) and its global step budget (step 3.1, default 50); see [`docs/escalation-protocol.md`](docs/escalation-protocol.md) § Budgets for all four.

## Recovering a crashed dispatch

This skill's drive loop is itself prose, driven by plain Bash — it has no Workflow run of its own to resume. But the `rdm:rdm-wf-dispatch-phase` Workflow call in step 3.4 (`rdm:rdm-wf-dispatch-phase`) is a real `Workflow` run, and it can crash mid-flight. If it does, relaunch that same script — same file, with an added `resumeFromRunId: '<prior runId>'` argument — instead of re-invoking it fresh. Any `agent()` call inside that run whose `(prompt, opts)` are byte-unchanged from the crashed attempt replays its cached result instead of re-dispatching. Four caveats apply every time:

- **Stop the prior run first** — a still-running run cannot be resumed.
- **Same-session only** — this only resumes within the current Claude Code session; a later session cannot resume a `runId` from an earlier one.
- **A cached result can be empty** — if the crashed agent produced nothing before dying, the resume replays that emptiness; check the run's `journal.jsonl` before assuming there is something to recover.
- **Conservative prefix** — resume replays only the longest unchanged prefix of the call sequence; the first edited-or-new call, and every call after it, run live. Do not plan around a specific savings figure.

## Relation to the other lanes

- **`land`** owns landing reviewed work to `main` (rebase + `merge --ff-only`); this skill never does. Run it after a run reaches `reviewed` if you want the work on `main`.
- This skill is the **active driver**: every dispatched phase actively runs review (`rdm:rdm-wf-dispatch-phase`'s code review is the canonical review pipeline `rdm:rdm-wf-dispatch-phase` embeds) before advancing, so nothing is left parked in `needs-review`.
- No `Done:` line is ever written here — this skill's advance step only persists the status the OUTCOME carries, directly via Bash. **`land` is the land-time writer**: it reads the OUTCOME's `writesCompletion: true` and synthesizes the trailer from the item's identifiers via `rdm hook done-line`, amending it onto the branch tip before the rebase. No pre-step is required.

See [`docs/autonomous-loop.md`](docs/autonomous-loop.md), [`docs/workflow-schemas.md`](docs/workflow-schemas.md), and [`docs/workflow-vs-prose-boundary.md`](docs/workflow-vs-prose-boundary.md) for the full contract and the reasoning behind this migration.

## Resolving `rdmBin` (plugin install)

This skill was installed from the `rdm` plugin, so there is no repo-local build path to assume. Resolve the `rdmBin` argument in this order and use the first that exists:

1. an explicitly supplied `--rdm-bin <path>`;
2. the `RDM_BIN` environment variable;
3. a plain `rdm` on `PATH`.

If none resolves, stop and report: `rdm binary not found. Install rdm, then set RDM_BIN=/path/to/rdm, put rdm on your PATH, or pass --rdm-bin /path/to/rdm.` Never guess a path, and never invoke a workflow without one.
