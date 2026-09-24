---
name: rdm-autopilot
description: Drive one named rdm roadmap from not-started to reviewed autonomously — pick the next actionable phase, estimate it, dispatch it on its model tier, interpret the outcome, and advance, parking the phase and stopping the run on the first blocker instead of interrupting
allowed-tools:
  - Bash
  - Workflow
  - Skill
---

Drive **one** rdm roadmap from `not-started` to `reviewed` with no per-phase human approval. This skill drives the loop **itself**, in prose. The one Workflow-tool call it makes is `rdm-wf-estimate` (the difficulty pre-pass, a real parallel fan-out); each per-phase unit is the `rdm-dispatch-phase` orchestrator, entered with the `Skill` tool into this same session (see step 4). Every other step — fetching `rdm next`, persisting an advance or a park, reading a write back to confirm it landed — is a plain Bash command this skill runs directly in its own context, because it is already a live agent with Bash access and the repo in context.

**Why `Skill` and not `Agent` for the per-phase unit:** an `Agent`-spawned subagent has no `Workflow` tool at all, so the orchestrator's code-review call could not be made there. `Skill` loads the procedure into this turn and keeps the tool; `Agent` would silently strand the call. See [`docs/workflow-vs-prose-boundary.md`](docs/workflow-vs-prose-boundary.md) for why the drive loop and the per-phase driver are both prose while `rdm-wf-estimate` and the review engine stay Workflow scripts (a fixed mechanism over a real fan-out).

A phase that cannot be advanced is parked `blocked`, not raised as a mid-run question — parking is still non-interactive, so the run never stops to ask the user anything. But a park now **ends the run immediately**: the loop never dispatches a later phase on top of one already known to be defective on the shared `roadmap/<slug>` worktree. This was observed on 2026-09-22 — a phase escalated with real defects, and autopilot parked it and dispatched the next phase on top of it anyway. See [`docs/autonomous-loop.md`](docs/autonomous-loop.md) § "Budgets and stop conditions" for the full rationale.
{principles}
## Contract

**Input** (`$ARGUMENTS`): a **required roadmap slug**, optionally followed by `--rdm-bin <path>`, `--project <name>`, `--max-phases N`, `--plan-only`, `--max-plan-revise N`, and/or `--max-code-rework N`. The slug names the single roadmap this run drives. If no slug is given, stop before invoking anything and say so — do not attempt a partial estimate or drive-loop start. `--rdm-bin` is **optional** and has no pre-flight stop of its own; when it is not supplied, use `$RDM_BIN` if it is set, otherwise a plain `rdm` on `PATH`. `docs/workflow-schemas.md` § "Environment args: `rdmBin` and `project`" is the canonical resolution order — do not restate it here.

Every Bash command and Workflow payload below is written against two placeholders resolved once in step 1: `<rdmBin>` — the executable resolved by that order — and `<proj-flag>` — ` --project <project>` for the project resolved there (never an empty `--project` value).

**The four guardrails, together, in one place:**

1. **Single roadmap.** The loop never roams to another roadmap — choosing which roadmap to advance stays a human decision. Every `rdm next` / `rdm phase update` / `rdm phase show` command below is scoped with the **same fixed** `--roadmap <slug>` throughout this run; nothing ever substitutes a different one.
2. **`main` is never touched.** Autopilot leaves every reviewed phase on the `roadmap/<slug>` branch; landing to `main` is the separate **`rdm-land`** skill. There is no `--land` flag here, and no Bash command in this loop ever runs `git checkout`/`merge`/`rebase` against `main`.
3. **No `Done:` trailer.** This skill never emits a `Done:` line: its advance step only persists the status the OUTCOME carries, directly via `rdm phase update --status <status> --no-edit` (a status-only write — it never stages or writes a commit message). **`rdm-land` reads the same `writesCompletion: true` signal** and, after landing, marks the item `done` itself with the landed tip's commit — no trailer is written by anyone in this flow.
4. **`--permission-mode auto` for unattended runs.** Launch with `--permission-mode auto` (or `bypassPermissions` in a sandbox) so this skill's own Bash commands, the `rdm-wf-estimate` Workflow call, and the `rdm-dispatch-phase` orchestrator's own calls don't block on a permission prompt.

This skill is **non-interactive**.

## What to do

### 1. Parse `$ARGUMENTS`

- `roadmap` — the required slug (first positional argument). Missing → stop immediately, before step 2, and say so.
- `rdmBin` — the **optional** executable path following `--rdm-bin`. There is **no** pre-flight stop for it: when it is not supplied, use `$RDM_BIN` if set, else a plain `rdm` on `PATH`. The literal sentinel `rdm` requests `PATH` resolution deliberately. Never probe the filesystem to pick a binary. This resolves the `<rdmBin>` placeholder used everywhere below.
- `project` — the name following `--project` when given, otherwise the name in `{proj_flag}`. Only when no project name applies does the `<proj-flag>` placeholder render as nothing (never an empty `--project` value), leaving rdm's own `RDM_PROJECT`/`default_project` chain to resolve it. Forward this same `project` to `rdm-wf-estimate` and `rdm-dispatch-phase`, so the loop and every unit act on one project.
- `maxPhases` — the positive integer following `--max-phases`, when present (omit otherwise — unbounded by phase count).
- `planOnly` — `true` when `--plan-only` is present (omit otherwise).
- `maxPlanRevise` — the non-negative integer following `--max-plan-revise`, when present (omit otherwise — `rdm-dispatch-phase` applies its own default of 2). `0` is legal and distinct from unset: it means "terminate on the first blocking plan review, no revise round at all".
- `maxCodeRework` — the non-negative integer following `--max-code-rework`, when present (omit otherwise — same default of 2, same `0`-is-legal rule).
- `globalBudget` — **not** a user-facing flag. It stays an internal constant, `DEFAULT_GLOBAL_BUDGET = 50`, hardcoded in this loop (see step 4).

### 2. Hoist the phase list

- Run `<rdmBin> phase list --roadmap <slug><proj-flag> --format json` and take the parsed array verbatim as `phaseList`. It feeds the `rdm-wf-estimate` Workflow's unestimated-phase filter directly (mirroring `rdm-estimate`'s own contract) — do not filter or summarize it yourself.

### 3. Run the estimate pre-pass — one Workflow call, always

Invoke the **`rdm-wf-estimate` Workflow** (`.claude/workflows/rdm-wf-estimate.js`) via the Workflow tool with `{ roadmap, phaseList, rdmBin, project }` as a JSON object (omit `phase` — autopilot always estimates the whole roadmap, never a single phase number; omit `project` when no project name applies). `rdmBin` and `project` are the same resolved values from step 1, so the pre-pass runs against this loop's binary and project instead of an ambient default. Run this call **unconditionally**, even if `phaseList` shows zero unestimated phases — it is a cheap no-op fan-out in that case, the same always-invoke-and-let-it-no-op design the `rdm-estimate` skill itself uses; do not skip it as an optimization.

Do not reimplement any part of the estimate pass in prose here — the filtering and the per-phase rating fan-out stay entirely inside the workflow. The workflow persists nothing: run each returned `writebackCommands` entry in Bash, in order, **exactly as returned** (each is one `phase update --difficulty` command).

If the `rdm-wf-estimate` invocation or a writeback errors, log a warning and continue straight into the drive loop **non-fatally** — an unrated phase simply falls back to whatever tier `rdm next` reports (empty/medium) when it's dispatched. Do not let a failed pre-pass abort the run.

### 4. Enter the drive loop

Maintain, in this skill's own working context (nothing here is persisted by rdm): `dispatchCount = 0`, `completed = []` (ordered), `escalations = []` (ordered), and — **only when `planOnly` is set** — a `planOnlySeen` set of stems already plan-vetted this run. There is no "seen" tracking in normal mode; normal-mode progress is driven purely by the persisted phase status that an advance/park write changes, which `rdm next` reads to step forward.

Loop:

1. **Budget check.** If `dispatchCount >= 50` (the internal `DEFAULT_GLOBAL_BUDGET`) or (`maxPhases` is set and `dispatchCount >= maxPhases`), stop with `stopReason: budget` and go to step 5. **This one counter is shared** between the global cap and `--max-phases`, and it increments on every dispatch — including a `rework` re-dispatch of the *same* phase, not just on distinct phases. `--max-phases 3` can therefore stop the run after as few as **one** distinct phase if it reworks twice. This is genuinely surprising but is the preserved, existing semantic — do not smooth it over when explaining a run to the user.
2. **Fetch the next phase.** Run `<rdmBin> next --roadmap <slug><proj-flag> --format json` directly via Bash and read its JSON output yourself — there is no fetch subagent.
3. **Classify the result** (mirrors `interpretNext`):
   - `result: "phase"` with a non-empty `stem` → work it (go to 4).
   - `result: "phase"` **missing** `stem` → this is malformed. Stop with `stopReason: unparseable` — **never** treat a malformed or unrecognized payload as `"nothing"` (that reason is reserved for a genuine, well-formed "no actionable phase" answer). Record a summary-only escalation `{ stem: "(fetch:next)", reason: "[fetch] unparseable rdm-next payload: <bounded description of the raw output>" }` — since no phase stem is known at this point, this entry is never parked via `rdm phase update` and will **not** appear in `rdm review blocked`; it only ever appears in this run's printed summary.
   - `result: "blocked-on-dependencies"` → stop with `stopReason: blocked-on-dependencies` (well-formed, known-good).
   - `result: "nothing"` → stop with `stopReason: nothing` (well-formed, known-good).
   - Go to step 5 for any stop.
4. **Work the phase**, stem `S`, tier `T` (`next.model`, defaulting to `medium` when unset):
   - Under `--plan-only`, if `S` is already in `planOnlySeen`, stop with `stopReason: plan-only-exhausted` and go to step 5 — a plan-only pass never advances or persists a terminal status, so `rdm next` will keep returning the same stem forever; this in-memory check is what detects the repeat (there is no rdm-side marker to lean on).
   - Otherwise, **enter the `rdm-dispatch-phase` orchestrator in this same session** with the `Skill` tool:

     ```
     Skill({ skill: 'rdm-dispatch-phase',
             args: '<slug> S' + flags })
     ```

     where `flags` forwards, as ARGUMENTS text and only when this run's `$ARGUMENTS` set them:
     `--plan-only`, `--max-plan-revise N`, `--max-code-rework N`, `--rdm-bin <rdmBin>`,
     `--project <project>`. **Never** enter it with `Agent` — see the reachability note above. There
     is no `phaseMeta`/`alreadyInProgress`/`dispatch.verify` hoist to assemble: the orchestrator
     runs in this same session with Bash, so it reads what it needs itself. The orchestrator stamps
     the phase `in-progress` (skipped under `--plan-only`), pins the checkout identity, plans, waits
     for the plan approval, implements, verifies, code-reviews, triages every comment on the
     persisted review, and performs the gated terminal status write, then returns the OUTCOME below.
   - Read the returned OUTCOME object's `outcome`, `status`, and `reason` fields.
   - **Interpret the outcome** (mirrors `interpretOutcome`):
     - `outcome: "reviewed"`, **not** plan-only → **advance**: run `<rdmBin> phase update S --status <OUTCOME.status || reviewed> --no-edit --roadmap <slug><proj-flag>`, then read it back with `<rdmBin> phase show S --roadmap <slug><proj-flag> --format json` and confirm `status` matches. Retry the write+read-back up to **2** times total (`DEFAULT_MAX_ADVANCE_ATTEMPTS`). On success: append `S` to `completed`, log `"phase S reviewed — advancing"`, continue the loop from step 1. On repeated failure: park `S` (below) with reason `"[code] advance to reviewed failed repeatedly"` — never report a false completion.
     - `outcome: "reviewed"`, plan-only → **noop-vetted**: add `S` to `planOnlySeen`, append `S` to `completed` (a plan-only pass records a vetted phase as completed, same bucket), log `"plan-only vetted S"`, continue the loop from step 1.
     - `outcome: "rework"` → if this phase's own rework count so far is **<** `DEFAULT_MAX_REWORK = 1`, increment it and **retry**: dispatch the **same** stem `S` again (go back to the dispatch call above, still counting against the shared budget in step 1) — do not call `rdm next` again first. Once the count reaches 1, **park** with reason `"[code] rework budget exhausted"`.
     - `outcome: "escalated"` → **park** with `OUTCOME.reason` if present, else `"[plan] dispatch escalated at the plan gate"`.
     - Anything else (a corrupted or unrecognized OUTCOME value) → **park** with reason `"[code] unrecognized dispatch outcome: <value>"` — never silently advance or silently stop.
   - **Park**: run `<rdmBin> phase update S --status blocked --reason "<reason>" --no-edit --roadmap <slug><proj-flag>`, then read it back with `<rdmBin> phase show S --roadmap <slug><proj-flag> --format json` and confirm `status: blocked`. Retry up to **2** times total (`DEFAULT_MAX_PARK_ATTEMPTS`). Whether or not the read-back ever confirms, append `{ stem: S, reason }` to `escalations`, set `stopReason: escalated`, and go to step 5 (stop) — do **not** continue the loop, regardless of whether the read-back confirmed. An unconfirmed park write must never abort the run before it can print its summary; log a loud warning in that case instead (the plan-repo status may not reflect the park, but the escalation is still recorded here). The parked phase's own commits stay right where they are, on the shared `roadmap/<slug>` branch; because this run stops here, nothing is dispatched on top of them in *this* run. A human who resolves the park and re-invokes autopilot (or `rdm-dispatch-phase` directly) resumes from `rdm next` exactly as before.
5. **Stop.** Exit the loop and proceed to "Print the summary" below.

### 5. Print the summary

Compose this yourself, in this exact structure (mirrors `buildSummary`), and print it verbatim as your final message — do not paraphrase or truncate it:

```
autopilot summary for roadmap/<slug>
<one of the two lines below>
phases completed (<n>): <stem, stem, ... or "none">
escalations awaiting review (<n>): <"none", or the block below>
  - <stem> [<stage>]: <reason>
  ...
review the queue: <rdmBin> review blocked<proj-flag>
[note: ... only if a [fetch]-tagged escalation is present, see below]
reviewed work is left on the roadmap/<slug> branch; main is never touched.
```

- The stop-reason line is either `stop reason: <reason>` (a known-good reason) or, for anything else, the loud `*** ABNORMAL TERMINATION (stop reason: <reason>) — the roadmap was NOT driven to exhaustion; do not read this as a completed run.` The **known-good allowlist** is exactly: `nothing`, `blocked-on-dependencies`, `budget`, `plan-only-exhausted`, `escalated`. Anything else — including a hypothetical future reason, or literally `unparseable` — gets the abnormal banner, never the plain line. Treat this as an allowlist, not a denylist keyed on one literal, so an unrecognized future reason is fail-safe flagged rather than silently trusted. `escalated` is the one known-good reason whose line embeds an identifier: when `stopReason` is `escalated`, render it as `stop reason: escalated (<stem>)`, naming the phase that caused the stop.
- `phases completed` lists stems in the order they completed (advance or noop-vetted), comma-joined, or `none`. Log each unit's `planId` and `reviewIds` alongside its stem as you go — they are pass-through fields the orchestrator returns, and they are the entry points into the persisted trail for whoever reads this summary. They add no branch: no decision in this loop reads them.
- `escalations awaiting review` renders each entry as `<stem> [<stage>]: <reason>`, where `<stage>` is parsed from the reason's leading `[tag]` (defaulting to `code` if unparseable), followed by the `rdm review blocked` pointer line.
- If **any** escalation is tagged `[fetch]`, add the loud note: `note: [fetch]-tagged entries above are summary-only — no phase is known to park against, so they will NOT appear in the` `rdm review blocked` `queue; act on them directly from this summary.` (Only a fetch-stage escalation, from step 4.3 above, is ever fetch-tagged — `[plan]`/`[code]` escalations always have a real stem and are genuinely queued.)
- The closing line is always present, regardless of outcome.

## Run modes

- `--max-phases N` — bounded run: dispatch at most `N` phases this pass, then stop and summarize. Use it to take a roadmap a few phases at a time.
- `--plan-only` — dry-run the planning half: each dispatch stops after its plan gate, so you get cheap plan vetting without writing any code.
- `--max-plan-revise N` / `--max-code-rework N` — override the orchestrator's two **in-run** retry budgets, which are counted **independently** of each other and default to **2** each (budget N = N reworks after the original attempt, i.e. N + 1 attempts). `0` is legal and means "terminate on the first blocking review" — no revise/rework agent runs at all. These are distinct from autopilot's own roadmap-level rework re-dispatch budget (step 4, capped at 1 retry per phase) and its global step budget (step 4.1, default 50); see [`docs/escalation-protocol.md`](docs/escalation-protocol.md) § Budgets for all four.

## Recovering a crashed dispatch

This skill's drive loop is itself prose, driven by plain Bash, and so is the per-phase unit it enters with `Skill` — neither has a `Workflow` run of its own to resume. A unit that stalls is recovered by re-entering it: the orchestrator's own resume step reads `rdm plan list --implements <item>` and `rdm review requests` first and picks up at the pending work, so an approved plan and an open review are never discarded or duplicated.

Three `Workflow` runs remain in this lane: step 3's `rdm-wf-estimate` pre-pass, and the plan review and code review the orchestrator invokes. If any crashes mid-flight, relaunch that same script with an added `resumeFromRunId: '<prior runId>'` argument instead of invoking it fresh; any `agent()` call whose `(prompt, opts)` are byte-unchanged replays its cached result. Four caveats apply every time:

- **Stop the prior run first** — a still-running run cannot be resumed.
- **Same-session only** — this only resumes within the current Claude Code session; a later session cannot resume a `runId` from an earlier one.
- **A cached result can be empty** — if the crashed agent produced nothing before dying, the resume replays that emptiness; check the run's `journal.jsonl` before assuming there is something to recover.
- **Conservative prefix** — resume replays only the longest unchanged prefix of the call sequence; the first edited-or-new call, and every call after it, run live. Do not plan around a specific savings figure.

See [`docs/autonomous-loop.md`](docs/autonomous-loop.md) § "Recovering a crashed run" for the full contract.

## Relation to the other lanes

- **`rdm-land`** owns landing reviewed work to `main`; this skill never touches `main` (guardrail 2
  above). Run it after a run reaches `reviewed` if you want the work on `main`.
- This skill is the **active driver**: every dispatched phase actively runs review (the orchestrator's
  code review is the canonical pipeline, invoked through the `rdm-wf-review-refute-fix` Workflow) and
  triages every comment on the persisted review before advancing, so nothing is left parked in
  `needs-review` and nothing is left unresolved.
- No `Done:` line is ever written here (guardrail 3 above); `rdm-land` marks the item `done` itself, at
  land time, with the landed tip's commit.

See [`docs/autonomous-loop.md`](docs/autonomous-loop.md), [`docs/workflow-schemas.md`](docs/workflow-schemas.md), and [`docs/workflow-vs-prose-boundary.md`](docs/workflow-vs-prose-boundary.md) for the full contract and the reasoning behind this migration.
