---
name: rdm-autopilot
description: Drive one named rdm roadmap from not-started to reviewed autonomously — pick the next actionable phase, estimate it, dispatch it on its model tier, interpret the outcome, and advance — batching decisions and blockers instead of interrupting
allowed-tools:
  - Workflow
  - {t_next}
  - {t_phase_update}
---

Drive **one** rdm roadmap from `not-started` to `reviewed` with no per-phase human approval, by invoking the **`autopilot` Workflow** (`.claude/workflows/autopilot.js`, provisioned automatically by `rdm agent-config claude --skills`). This skill is a **thin shim**: it parses the invocation, hands off to the workflow, and prints the batched summary the workflow returns. All the loop logic — the estimate pre-pass, the `{t_next}` drive loop, per-phase dispatch via the `dispatch-phase` workflow, outcome interpretation, status persistence, budgets, and the summary — lives in the workflow, not in this prose.

Decisions and blockers are **batched, not raised mid-run**: a phase that cannot be advanced is parked `blocked` and the run keeps making progress on the rest, so the user answers the whole queue at once at the end rather than being interrupted per phase.
{principles}
## Contract

**Input** (`$ARGUMENTS`): a **required roadmap slug**, optionally followed by `--max-phases N`, `--plan-only`, `--max-plan-revise N`, and/or `--max-code-rework N`. The slug names the single roadmap this run drives; the loop **never roams to another roadmap** — choosing which roadmap to advance stays a human decision. If no slug is given, stop and say so.

This skill is **non-interactive**. Launch unattended runs with `--permission-mode auto` (or `bypassPermissions` in a sandbox) so the workflow's dispatched agents and tool calls don't block on permission prompts.

`main` is **never touched**. Autopilot leaves every reviewed phase on the `roadmap/<slug>` branch; landing to `main` is the separate **`rdm-land`** skill. There is no `--land` flag here.

## What to do

1. **Parse `$ARGUMENTS`** into a config object:
   - `roadmap` — the required slug (the first positional argument).
   - `maxPhases` — the positive integer following `--max-phases`, when present (omit otherwise).
   - `planOnly` — `true` when `--plan-only` is present (omit otherwise).
   - `maxPlanRevise` — the non-negative integer following `--max-plan-revise`, when present (omit otherwise).
   - `maxCodeRework` — the non-negative integer following `--max-code-rework`, when present (omit otherwise).
2. **Invoke the `autopilot` workflow** via the Workflow tool with `{ roadmap, maxPhases, planOnly, maxPlanRevise, maxCodeRework }` (omit any of `maxPhases`/`planOnly`/`maxPlanRevise`/`maxCodeRework` when not supplied). Pass `args` as a JSON object, never a stringified value. The workflow:
   - runs the **estimate pre-pass** over the roadmap's unestimated phases in one parallel fan-out, persisting each difficulty (the model tier derives automatically);
   - loops the `{t_next}` MCP tool (`project: {proj_param}, roadmap: "<slug>"`) → the `dispatch-phase` workflow (via the one allowed level of `workflow()` nesting) → interpret the OUTCOME;
   - **advances** a `reviewed` phase (`{t_phase_update}` with `status: "reviewed"`, so `{t_next}` steps past it), **re-dispatches** a `rework` phase against a per-phase budget and **parks** it `blocked [code]` when the budget is spent, and **parks** an `escalated` phase `blocked [plan]`;
   - bounds the run with a **global step budget** and **`--max-phases`**, and under **`--plan-only`** stops each dispatch after its plan gate (no implementation), guarding against re-vetting the same phase.
3. **Print the returned summary verbatim.** It lists the phases completed this run (in order), the escalations awaiting review (each tagged `plan` vs `code`) pointing at the `rdm review blocked` command, the stop reason, and the note that reviewed work is left on the `roadmap/<slug>` branch with `main` untouched.

## Run modes

- `--max-phases N` — bounded run: dispatch at most `N` phases this pass, then stop and summarize. Use it to take a roadmap a few phases at a time.
- `--plan-only` — dry-run the planning half: each dispatch stops after its plan gate, so you get cheap plan vetting without writing any code.
- `--max-plan-revise N` / `--max-code-rework N` — override `dispatch-phase`'s two **in-run** retry budgets, which are counted **independently** of each other and default to **2** each (budget N = N reworks after the original attempt, i.e. N + 1 attempts). `0` is legal and means "terminate on the first blocking review" — no revise/rework agent runs at all. These are distinct from autopilot's own roadmap-level rework re-dispatch budget and its global step budget; see [`docs/escalation-protocol.md`](docs/escalation-protocol.md) § Budgets for all four.

## Relation to the other lanes

- **`rdm-land`** owns landing reviewed work to `main` (rebase + `merge --ff-only`); autopilot never does. Run it after a run reaches `reviewed` if you want the work on `main`.
- Autopilot is the **active driver**: every dispatched phase actively runs review (`dispatch-phase`'s code review is the canonical review pipeline stamped from `.claude/workflows/lib/review.mjs`) before advancing, so nothing is left parked in `needs-review`. The once-passive needs-review Stop hook (Claude Code) / Pi `agent_end` extension that used to catch a dropped finalize has been retired as redundant. The workflow lane never emits a `Done:` line: autopilot's advance step only persists the status the OUTCOME carries. **`rdm-land` is the land-time writer** — it reads the OUTCOME's `writesCompletion: true` and synthesizes the trailer from the item's identifiers via `rdm hook done-line`, amending it onto the branch tip before the rebase. No pre-step is required: run `rdm-land` directly, and it never needs a manual rebase to add the line.

See [`docs/autonomous-loop.md`](docs/autonomous-loop.md) and [`docs/workflow-schemas.md`](docs/workflow-schemas.md) for the full workflow contract.
