---
name: rdm-autopilot
description: Drive one named rdm roadmap from not-started to reviewed autonomously — pick the next actionable phase, estimate it, dispatch it on its model tier, interpret the outcome, and advance — batching decisions and blockers instead of interrupting
allowed-tools:
  - Bash
  - Workflow
---

Drive **one** rdm roadmap from `not-started` to `reviewed` with no per-phase human approval, by invoking the **`autopilot` Workflow** (`.claude/workflows/autopilot.js`). This skill is a **thin shim**: it parses the invocation, hands off to the workflow, and prints the batched summary the workflow returns. All the loop logic — the estimate pre-pass, the `rdm next` drive loop, per-phase dispatch via the `dispatch-phase` workflow, outcome interpretation, status persistence, budgets, and the summary — lives in the workflow, not in this prose.

Decisions and blockers are **batched, not raised mid-run**: a phase that cannot be advanced is parked `blocked` and the run keeps making progress on the rest, so the user answers the whole queue at once at the end rather than being interrupted per phase.

**IMPORTANT: This is the rdm source repo. Always run `cargo build` first so the workflow's `./target/debug/rdm` calls reflect your working changes — never bare `rdm`. If you modify any rdm source, `cargo build` again before invoking the workflow.**

## Contract

**Input** (`$ARGUMENTS`): a **required roadmap slug**, optionally followed by `--max-phases N` and/or `--plan-only`. The slug names the single roadmap this run drives; the loop **never roams to another roadmap** — choosing which roadmap to advance stays a human decision. If no slug is given, stop and say so.

This skill is **non-interactive**. Launch unattended runs with `--permission-mode auto` (or `bypassPermissions` in a sandbox) so the workflow's dispatched agents and bash commands don't block on permission prompts.

`main` is **never touched**. Autopilot leaves every reviewed phase on the `roadmap/<slug>` branch; landing to `main` is the separate **`rdm-land`** skill. There is no `--land` flag here.

## What to do

1. **Build:** run `cargo build` so the workflow's `./target/debug/rdm` invocations are current.
2. **Parse `$ARGUMENTS`** into a config object:
   - `roadmap` — the required slug (the first positional argument).
   - `maxPhases` — the positive integer following `--max-phases`, when present (omit otherwise).
   - `planOnly` — `true` when `--plan-only` is present (omit otherwise).
3. **Invoke the `autopilot` workflow** via the Workflow tool with `{ roadmap, maxPhases, planOnly }` (omit `maxPhases`/`planOnly` when not supplied). The workflow:
   - runs the **estimate pre-pass** over the roadmap's unestimated phases in one parallel fan-out, persisting each difficulty (the model tier derives automatically);
   - loops `./target/debug/rdm next` → the `dispatch-phase` workflow (via the one allowed level of `workflow()` nesting) → interpret the OUTCOME;
   - **advances** a `reviewed` phase (`rdm phase update --status reviewed`, so `rdm next` steps past it), **re-dispatches** a `rework` phase against a per-phase budget and **parks** it `blocked [code]` when the budget is spent, and **parks** an `escalated` phase `blocked [plan]`;
   - bounds the run with a **global step budget** and **`--max-phases`**, and under **`--plan-only`** stops each dispatch after its plan gate (no implementation), guarding against re-vetting the same phase.
4. **Print the returned summary verbatim.** It lists the phases completed this run (in order), the escalations awaiting review (each tagged `plan` vs `code`) pointing at `./target/debug/rdm review blocked --project rdm`, the stop reason, and the note that reviewed work is left on the `roadmap/<slug>` branch with `main` untouched.

## Run modes

- `--max-phases N` — bounded run: dispatch at most `N` phases this pass, then stop and summarize. Use it to take a roadmap a few phases at a time.
- `--plan-only` — dry-run the planning half: each dispatch stops after its plan gate, so you get cheap plan vetting without writing any code.

## Relation to the other lanes

- **`rdm-land`** owns landing reviewed work to `main` (rebase + `merge --ff-only`); autopilot never does. Run it after a run reaches `reviewed` if you want the work on `main`.
- The needs-review **Stop hook** (Claude Code) and the Pi **`agent_end` extension** are the **passive safety net**: they only re-prompt when an item is *left* in `needs-review`. Autopilot is the **active driver**. The workflow lane never emits a `Done:` line: `dispatch-phase`'s review is an inline pipeline (not the `rdm-review` skill), and autopilot's advance step writes only `--status reviewed`. The `Done:` line is supplied later — by `rdm-review` or at landing — so run one of those before `rdm-land`.

See [`docs/autonomous-loop.md`](../../../docs/autonomous-loop.md) and [`docs/workflow-schemas.md`](../../../docs/workflow-schemas.md) for the full workflow contract.
