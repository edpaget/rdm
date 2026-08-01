---
name: rdm-autopilot
description: Drive one named rdm roadmap from not-started to reviewed autonomously — pick the next actionable phase, estimate it, dispatch it on its model tier, interpret the outcome, and advance — batching decisions and blockers instead of interrupting
allowed-tools:
  - Bash
  - Workflow
---

Drive **one** rdm roadmap from `not-started` to `reviewed` with no per-phase human approval. This skill drives the loop **itself**, in prose — it is no longer a thin shim over a `.claude/workflows/autopilot.js` Workflow. The two Workflow-tool calls it makes are `estimate` (the difficulty pre-pass, a real parallel fan-out) and `dispatch-phase` (one phase's plan → plan-review → implement → code-review pipeline). Every other step — fetching `rdm next`, persisting an advance or a park, reading a write back to confirm it landed, resolving the mechanical model — is a plain Bash command this skill runs directly in its own context, because it is already a live agent with Bash access and the repo in context. That subagent elimination is the concrete point of this migration: those same steps used to cost a dedicated mechanical `agent()` subagent inside the headless workflow, which cannot run Bash itself; a prose skill already can, so that cost disappears entirely. See `docs/workflow-vs-prose-boundary.md` for why the drive loop specifically (a low-iteration sequential policy driver) moved while `dispatch-phase` and `estimate` stayed workflows (real fan-out over a fixed mechanism).

Decisions and blockers are **batched, not raised mid-run**: a phase that cannot be advanced is parked `blocked` and the run keeps making progress on the rest, so the user answers the whole queue at once at the end rather than being interrupted per phase.

## Contract

**Input** (`$ARGUMENTS`): a **required roadmap slug**, optionally followed by `--max-phases N`, `--plan-only`, `--max-plan-revise N`, and/or `--max-code-rework N`. The slug names the single roadmap this run drives. If no slug is given, stop before invoking anything and say so — do not attempt a partial estimate or drive-loop start.

**The four guardrails, together, in one place:**

1. **Single roadmap.** The loop never roams to another roadmap — choosing which roadmap to advance stays a human decision. Every `rdm next` / `rdm phase update` / `rdm phase show` command below is scoped with the **same fixed** `--roadmap <slug>` throughout this run; nothing ever substitutes a different one.
2. **`main` is never touched.** Autopilot leaves every reviewed phase on the `roadmap/<slug>` branch; landing to `main` is the separate **`rdm-land`** skill. There is no `--land` flag here, and no Bash command in this loop ever runs `git checkout`/`merge`/`rebase` against `main`.
3. **No `Done:` trailer.** This skill never emits a `Done:` line: its advance step only persists the status the OUTCOME carries, directly via `rdm phase update --status <status> --no-edit` (a status-only write — it never stages or writes a commit message). **`rdm-land` is the land-time writer** — it reads the OUTCOME's `writesCompletion: true` and synthesizes the trailer from the item's identifiers via `rdm hook done-line`, amending it onto the branch tip before the rebase.
4. **`--permission-mode auto` for unattended runs.** Launch with `--permission-mode auto` (or `bypassPermissions` in a sandbox) so this skill's own Bash commands and the `estimate`/`dispatch-phase` Workflow calls don't block on a permission prompt. It is now this skill's own calls that must not stall, not a dispatched workflow's.

This skill is **non-interactive**.

## What to do

### 1. Parse `$ARGUMENTS`

- `roadmap` — the required slug (first positional argument). Missing → stop immediately, before step 2, and say so.
- `maxPhases` — the positive integer following `--max-phases`, when present (omit otherwise — unbounded by phase count).
- `planOnly` — `true` when `--plan-only` is present (omit otherwise).
- `maxPlanRevise` — the non-negative integer following `--max-plan-revise`, when present (omit otherwise — `dispatch-phase` applies its own default of 2). `0` is legal and distinct from unset: it means "terminate on the first blocking plan review, no revise round at all".
- `maxCodeRework` — the non-negative integer following `--max-code-rework`, when present (omit otherwise — same default of 2, same `0`-is-legal rule).
- `globalBudget` — **not** a user-facing flag. It never was: the previous shim's own "Parse `$ARGUMENTS`" step listed only the four flags above. It stays an internal constant, `DEFAULT_GLOBAL_BUDGET = 50`, hardcoded in this loop (see step 3).

### 2. Hoist the mechanical model and phase list

- Run `./target/debug/rdm model resolve mechanical` and take its printed output verbatim as `mechanicalModel`. This is no longer needed to feed this loop's own fetch/advance/park steps (those are direct Bash now — there is no subagent to hand a model id to); it is needed **only** to forward into the `estimate` Workflow call in step 3, mirroring `rdm-estimate`'s own contract. If the command fails or prints nothing, treat this as the fail-closed **`mechanical-model-unresolved`** stop: log it, print the summary (step 5) with `stopReason: mechanical-model-unresolved`, and stop before invoking either Workflow. This is the one stop reason still named after "mechanical model" even though the loop's own steps no longer depend on one — it now guards only the `estimate` pre-pass's bootstrap.
- Run `./target/debug/rdm phase list --roadmap <slug> --project rdm --format json` and take the parsed array verbatim as `phaseList`. It feeds the `estimate` Workflow's unestimated-phase filter directly (mirroring `rdm-estimate`'s own contract) — do not filter or summarize it yourself.

### 3. Run the estimate pre-pass — one Workflow call, always

Invoke the **`estimate` Workflow** via the Workflow tool with `{ roadmap, mechanicalModel, phaseList }` (omit `phase` — autopilot always estimates the whole roadmap, never a single phase number). Run this call **unconditionally**, even if `phaseList` shows zero unestimated phases — it is a cheap no-op fan-out in that case, the same always-invoke-and-let-it-no-op design the `rdm-estimate` skill itself uses; do not skip it as an optimization.

This is a **genuinely new call path**, not a relocation: the retired `autopilot.js` reached the same rating fan-out only through a stamped `estimate-core` copy embedded in `lib/autopilot.mjs`, never a real `workflow('estimate', …)` call. This skill is the first caller to invoke `estimate` for real from the autopilot lane.

Do not reimplement any part of the estimate pass in prose here — the filtering, the per-phase rating fan-out, and the persistence step all stay entirely inside `estimate.js`'s own pipeline, untouched by this skill.

If the `estimate` invocation itself errors or throws (e.g. it can't resolve its own model), log a warning and continue straight into the drive loop **non-fatally** — an unrated phase simply falls back to whatever tier `rdm next` reports (empty/medium) when it's dispatched. Do not let a failed pre-pass abort the run.

### 4. Enter the drive loop

Maintain, in this skill's own working context (nothing here is persisted by rdm): `dispatchCount = 0`, `completed = []` (ordered), `escalations = []` (ordered), and — **only when `planOnly` is set** — a `planOnlySeen` set of stems already plan-vetted this run. There is no "seen" tracking in normal mode; normal-mode progress is driven purely by the persisted phase status that an advance/park write changes, which `rdm next` reads to step forward.

Loop:

1. **Budget check.** If `dispatchCount >= 50` (the internal `DEFAULT_GLOBAL_BUDGET`) or (`maxPhases` is set and `dispatchCount >= maxPhases`), stop with `stopReason: budget` and go to step 5. **This one counter is shared** between the global cap and `--max-phases`, and it increments on every dispatch — including a `rework` re-dispatch of the *same* phase, not just on distinct phases. `--max-phases 3` can therefore stop the run after as few as **one** distinct phase if it reworks twice. This is genuinely surprising but is the preserved, existing semantic — do not smooth it over when explaining a run to the user.
2. **Fetch the next phase.** Run `./target/debug/rdm next --roadmap <slug> --project rdm --format json` directly via Bash and read its JSON output yourself — there is no fetch subagent.
3. **Classify the result** (mirrors `interpretNext`):
   - `result: "phase"` with a non-empty `stem` → work it (go to 4).
   - `result: "phase"` **missing** `stem` → this is malformed. Stop with `stopReason: unparseable` — **never** treat a malformed or unrecognized payload as `"nothing"` (that reason is reserved for a genuine, well-formed "no actionable phase" answer). Record a summary-only escalation `{ stem: "(fetch:next)", reason: "[fetch] unparseable rdm-next payload: <bounded description of the raw output>" }` — since no phase stem is known at this point, this entry is never parked via `rdm phase update` and will **not** appear in `rdm review blocked`; it only ever appears in this run's printed summary. (Dropped defense, explicitly: the JS loop also defensively unwrapped up to 3 levels of a JSON-string-encoded `result` field, a hedge against an intermediate schema-constrained agent re-transcribing `rdm next`'s output as a string. That intermediate agent no longer exists — this skill reads Bash stdout directly — so that unwrap layer is deliberately not reproduced here; a malformed payload of that shape now falls straight through to `unparseable`, which is still the fail-closed, never-silently-"nothing" outcome the dropped defense existed to guarantee.)
   - `result: "blocked-on-dependencies"` → stop with `stopReason: blocked-on-dependencies` (well-formed, known-good).
   - `result: "nothing"` → stop with `stopReason: nothing` (well-formed, known-good).
   - Go to step 5 for any stop.
4. **Work the phase**, stem `S`, tier `T` (`next.model`, defaulting to `medium` when unset):
   - Under `--plan-only`, if `S` is already in `planOnlySeen`, stop with `stopReason: plan-only-exhausted` and go to step 5 — a plan-only pass never advances or persists a terminal status, so `rdm next` will keep returning the same stem forever; this in-memory check is what detects the repeat (there is no rdm-side marker to lean on).
   - Otherwise, invoke the **`dispatch-phase` Workflow** via the Workflow tool with exactly `{ roadmap: <slug>, phase: S, planOnly }`, plus `maxPlanRevise` and/or `maxCodeRework` **only when this run's `$ARGUMENTS` set them** — this is the exact payload shape `dispatch-phase` is invoked with today, preserved byte-for-shape.
   - Read the returned OUTCOME object's `outcome`, `status`, and `reason` fields.
   - **Interpret the outcome** (mirrors `interpretOutcome`):
     - `outcome: "reviewed"`, **not** plan-only → **advance**: run `./target/debug/rdm phase update S --status <OUTCOME.status || reviewed> --no-edit --roadmap <slug> --project rdm`, then read it back with `./target/debug/rdm phase show S --roadmap <slug> --project rdm --format json` and confirm `status` matches. Retry the write+read-back up to **2** times total (`DEFAULT_MAX_ADVANCE_ATTEMPTS`). On success: append `S` to `completed`, log `"phase S reviewed — advancing"`, continue the loop from step 1. On repeated failure: park `S` (below) with reason `"[code] advance to reviewed failed repeatedly"` — never report a false completion.
     - `outcome: "reviewed"`, plan-only → **noop-vetted**: add `S` to `planOnlySeen`, append `S` to `completed` (a plan-only pass records a vetted phase as completed, same bucket), log `"plan-only vetted S"`, continue the loop from step 1.
     - `outcome: "rework"` → if this phase's own rework count so far is **<** `DEFAULT_MAX_REWORK = 1`, increment it and **retry**: dispatch the **same** stem `S` again (go back to the dispatch call above, still counting against the shared budget in step 1) — do not call `rdm next` again first. Once the count reaches 1, **park** with reason `"[code] rework budget exhausted"`.
     - `outcome: "escalated"` → **park** with `OUTCOME.reason` if present, else `"[plan] dispatch escalated at the plan gate"`.
     - Anything else (a corrupted or unrecognized OUTCOME value) → **park** with reason `"[code] unrecognized dispatch outcome: <value>"` — never silently advance or silently stop.
   - **Park**: run `./target/debug/rdm phase update S --status blocked --reason "<reason>" --no-edit --roadmap <slug> --project rdm`, then read it back and confirm `status: blocked`. Retry up to **2** times total (`DEFAULT_MAX_PARK_ATTEMPTS`). Whether or not the read-back ever confirms, append `{ stem: S, reason }` to `escalations` and continue the loop from step 1 — an unconfirmed park write must never abort the run before it can print its summary; log a loud warning in that case instead (the plan-repo status may not reflect the park, but the escalation is still recorded here).
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
review the queue: ./target/debug/rdm review blocked --project rdm
[note: ... only if a [fetch]-tagged escalation is present, see below]
reviewed work is left on the roadmap/<slug> branch; main is never touched.
```

- The stop-reason line is either `stop reason: <reason>` (a known-good reason) or, for anything else, the loud `*** ABNORMAL TERMINATION (stop reason: <reason>) — the roadmap was NOT driven to exhaustion; do not read this as a completed run.` The **known-good allowlist** is exactly: `nothing`, `blocked-on-dependencies`, `budget`, `plan-only-exhausted`, `mechanical-model-unresolved`. Anything else — including a hypothetical future reason, or literally `unparseable` — gets the abnormal banner, never the plain line. Treat this as an allowlist, not a denylist keyed on one literal, so an unrecognized future reason is fail-safe flagged rather than silently trusted.
- `phases completed` lists stems in the order they completed (advance or noop-vetted), comma-joined, or `none`.
- `escalations awaiting review` renders each entry as `<stem> [<stage>]: <reason>`, where `<stage>` is parsed from the reason's leading `[tag]` (defaulting to `code` if unparseable), followed by the `rdm review blocked` pointer line.
- If **any** escalation is tagged `[fetch]`, add the loud note: `note: [fetch]-tagged entries above are summary-only — no phase is known to park against, so they will NOT appear in the` `rdm review blocked` `queue; act on them directly from this summary.` (Only a fetch-stage escalation, from step 4.3 above, is ever fetch-tagged — `[plan]`/`[code]` escalations always have a real stem and are genuinely queued.)
- The closing line is always present, regardless of outcome.

## Removed / changed from the workflow version

Every input `parseAutopilotArgs` accepted, with its disposition here:

- **`roadmap` / `maxPhases` / `planOnly` / `maxPlanRevise` / `maxCodeRework`** — **kept**, identical parsing rules and defaults (2 each inside `dispatch-phase` when omitted; `0` is legal and distinct from unset).
- **`globalBudget`** — **kept**, internal-only, `DEFAULT_GLOBAL_BUDGET = 50`, never a flag — the previous shim never exposed one either.
- **`mechanicalModel` hoist** — **kept, repurposed**. It no longer feeds this loop's own fetch/advance/park (those are direct Bash now, no subagent to hand a model id to); it is needed only to forward into the `estimate` Workflow call.
- **`phaseList` hoist** — **kept, repurposed**, for the same reason: it now feeds the `estimate` Workflow call directly instead of an inline JS pre-pass.
- **`next` hoist** — **removed**. In the JS loop this existed purely to save the *first* iteration's dedicated `fetchNext` agent dispatch. Since this skill reads `rdm next` via a direct Bash call on every iteration — including the first — at zero subagent cost, there is nothing left to economize by hoisting it, so the caller-supplied shortcut is dropped entirely.
- **`mechanical-model-unresolved` as a loop-level stop reason** — **narrowed, not removed**. The loop no longer resolves or depends on a mechanical model for its own bookkeeping steps, but `resolveMechanicalModel` is still run once (step 2) purely to supply the value forwarded to `estimate`, and a failure there still produces the same loud, named stop (reused literally, still in the known-good allowlist) so the abnormal-termination banner still fires correctly if the estimate pre-pass can't get a model id.
- **The triple-unwrap defense inside `interpretNext`** (an internal helper, not a top-level input, but recorded here in the same spirit) — **deliberately dropped**. It hedged against an intermediate agent re-encoding `rdm next`'s JSON as a string; that intermediate agent no longer exists once this skill reads Bash stdout directly, so a payload of that shape now falls straight through to the fail-closed `unparseable` stop instead of being unwrapped and recovered.

## Relation to the other lanes

- **`rdm-land`** owns landing reviewed work to `main` (rebase + `merge --ff-only`); this skill never does. Run it after a run reaches `reviewed` if you want the work on `main`.
- This skill is the **active driver**: every dispatched phase actively runs review (`dispatch-phase`'s code review is the canonical review pipeline stamped from `.claude/workflows/lib/review.mjs`) before advancing, so nothing is left parked in `needs-review`.
- No `Done:` line is ever written here — this skill's advance step only persists the status the OUTCOME carries, directly via Bash. **`rdm-land` is the land-time writer**: it reads the OUTCOME's `writesCompletion: true` and synthesizes the trailer from the item's identifiers via `rdm hook done-line`, amending it onto the branch tip before the rebase. No pre-step is required.

See [`docs/autonomous-loop.md`](docs/autonomous-loop.md), [`docs/workflow-schemas.md`](docs/workflow-schemas.md), and [`docs/workflow-vs-prose-boundary.md`](docs/workflow-vs-prose-boundary.md) for the full contract and the reasoning behind this migration.
