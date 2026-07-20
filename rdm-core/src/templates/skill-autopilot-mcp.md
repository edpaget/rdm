---
name: rdm-autopilot
description: Drive one named rdm roadmap from not-started to reviewed autonomously — pick the next actionable phase, estimate it, dispatch it on its model tier, interpret the outcome, and advance — batching decisions and blockers instead of interrupting
allowed-tools:
  - Read
  - Glob
  - Grep
  - Write
  - Edit
  - Agent
  - {t_next}
  - {t_phase_list}
  - {t_phase_show}
  - {t_phase_update}
---

Drive **one** rdm roadmap to `reviewed` with no per-phase human approval. This is the capstone of autonomous execution: a loop that repeatedly asks rdm for the next actionable phase, runs it end-to-end, interprets the structured outcome, and moves on — until nothing actionable is left. It is the **active driver**; the per-phase work itself is delegated to `rdm-dispatch-phase` (which owns the plan gate, implementation, and `rdm-review`), and difficulty/model-tier assessment to `rdm-estimate`. Autopilot never re-implements those — it composes them.

Decisions and blockers are **batched, not raised mid-run**: a phase that cannot be decided is parked `blocked` and the loop keeps making progress on the rest, so the user answers the whole queue at once at the end rather than being interrupted per phase.
{principles}
## Contract

**Input** (`$ARGUMENTS`): a **required roadmap slug**, optionally followed by flags. The slug names the single roadmap this run drives. The loop **never roams to another roadmap** — choosing which roadmap to advance stays a human decision. If no slug is given, stop and say so.

This skill is non-interactive. Launch unattended runs with `--permission-mode auto` (or `bypassPermissions` in a sandbox) so file edits, rdm tool calls, and dispatched subagents don't block on permission prompts.

`main` is **never touched** unless `--land` is passed (see below). The default run leaves `main` exactly as it found it.

## Flags / run modes

- `--max-phases N` — bounded run: dispatch at most `N` phases this pass, then stop and summarize. Use it to take a roadmap a few phases at a time.
- `--plan-only` — dry-run the planning half: for each phase run `rdm-dispatch-phase` only through its **plan gate**, then stop before implementation. Cheap plan vetting across the roadmap without writing code.
- `--land` — opt-in. After the roadmap reaches `reviewed`, invoke the `rdm-land` skill to land the work to `main` with linear history (rebase + `merge --ff-only`). **Default OFF**: without `--land`, autopilot never touches `main`; it leaves every reviewed phase on the `roadmap/<slug>` branch for a human to land.

## Loop

Each iteration runs on **bounded state**: the only things the loop retains across iterations are the latest `{t_next}` result and the small structured outcome JSON returned by each dispatched phase. It never accumulates the plan, plan-review, implementation, or code-review detail of the phases it has already run — that stays behind the subagent boundary (see step 2). This is what keeps the loop context flat across a long roadmap instead of growing with every phase.

### Mandatory dispatch — no inline work

Dispatch is a **MUST**, not a suggestion, and it does not degrade based on how capable the current session feels. You (the loop) MUST NOT plan, implement, or review any phase's work yourself in this context — every phase's work MUST happen inside the `Agent` subagent dispatched in step 2. The loop's only job is orchestration: call the `{t_next}` MCP tool, dispatch, interpret the returned outcome, repeat.

**Failure mode to avoid — inline-collapse:** reading the phase, then estimating, planning, implementing, or reviewing it yourself in this same context — treating "dispatch a subagent" as narration rather than an actual `Agent` tool call, or reporting a result as if a subagent produced it when none was spawned. This is **inline-collapse**: it silently discards the context isolation and independent review the whole design depends on. It MUST NOT happen, regardless of model or session.

Before step 2's `Agent` call, and again before treating its result as final, you MUST complete this checklist:

1. **Declare** which subagent you are about to dispatch and its role (e.g. "Dispatching phase subagent for `<slug>/<phase-stem>`").
2. **Call** the `Agent` tool synchronously for that dispatch — never narrate it, actually invoke it.
3. **Block** until it returns; do not advance on any other signal (no polling, no assuming completion).
4. **Verify** the returned result is the structured `{roadmap, phase, outcome, summary, findings}` JSON described in step 2 before proceeding to step 3. If no `Agent` call actually happened, stop and redo this step — do not substitute your own inline work for the missing subagent's output.

This checklist runs fresh for every phase the loop dispatches — it is per-iteration, not once per run.

1. **Ask for the next actionable phase:** use the `{t_next}` MCP tool with `project: {proj_param}, roadmap: "<slug>"`. Branch on `result`:
   - `phase` → continue with this iteration (the result carries `stem`, `number`, `status`, and `difficulty`/`model` if assessed).
   - `nothing` → **stop**: no actionable phase remains (all phases are `reviewed`/`done`/`blocked`/`wont-fix`). Go to the summary.
   - `blocked-on-dependencies` → **stop**: the roadmap depends on other roadmaps that are not yet complete (`unmet` lists them). Go to the summary.

   `{t_next}` is also the **termination oracle**: it skips `needs-review`, `reviewed`, `done`, `blocked`, and `wont-fix`, so a parked or reviewed phase is automatically stepped over on the next iteration. A phase you left `in-progress` (a rework) is returned **again** — which is why each phase carries a retry budget (below). (`rdm_phase_show` / `rdm_phase_list` with `project: {proj_param}, roadmap: "<slug>"` give the same per-phase detail when you need it.)

2. **Dispatch the phase in an isolated subagent:** spawn **one `Agent` subagent** for this phase, **dispatched synchronously — the loop blocks until the subagent returns its structured outcome; never background-and-poll, never resume a returned subagent by message** — briefed with only the roadmap slug, the phase stem, and whether `--plan-only` is in effect — *not* this loop's accumulated context. That subagent does the whole per-phase job and returns only its structured outcome:
   - If the phase's `difficulty` is unset in the `{t_next}` result, the subagent runs the `rdm-estimate` skill on it first (`<slug> <phase>`) so it has a difficulty and a derived model tier before dispatch; if it already has a difficulty, it skips estimation.
   - The subagent then runs the `rdm-dispatch-phase` skill (`<slug> <phase>`), which runs the phase end-to-end in the roadmap's shared worktree on its model tier — plan, independent plan gate, implement, code-review — and produces a structured `reviewed | rework | escalated` outcome.
   - Under `--plan-only`, the subagent stops the dispatch after its plan gate and reports the gate verdict instead of implementing.
   - The subagent returns **only** the structured `{roadmap, phase, outcome, summary, findings}` JSON. That blob — not the transcript — is all that crosses back into the loop.
   - **Self-check before proceeding:** you MUST NOT continue to step 3 until you have confirmed the `Agent` call above actually happened and returned this JSON. If you find yourself drafting a phase's plan, code, or review directly in this loop's context, stop immediately — that is the inline-collapse failure above, and this step MUST be redone with a real dispatch.

   **Why the boundary.** The `Skill` tool runs **inline** in this conversation, so invoking `rdm-estimate`/`rdm-dispatch-phase`/`rdm-review` directly from the loop would pour every phase's estimate, plan, plan-review, implementation, and code-review detail into the loop context — growing it roughly quadratically over a multi-phase run and diluting attention on later phases. Dispatching each phase as its own `Agent` subagent keeps that detail inside the subagent; only the outcome JSON returns. This mirrors the isolation `rdm-dispatch-phase` already applies one level down (its own **Context isolation** section, where the planner, plan reviewer, and implementer are three separate subagents). **Never** invoke `rdm-estimate`, `rdm-dispatch-phase`, or `rdm-review` directly with the `Skill` tool from this loop — that reintroduces the inline-accumulation trap.

   **Subagents never `SendMessage` their orchestrator.** A dispatched subagent must not attempt to `SendMessage` back to its parent — there is no resolvable parent name (`"claude"` does not resolve). The orchestrator reads each subagent's returned result; that result is the sole channel for outcomes to flow back.

3. **Interpret the outcome** returned by the subagent:
   - **reviewed** → the phase is `reviewed` with a `Done:` line on the branch. Advance: the next `{t_next}` call steps past it.
   - **rework** → code review failed but it is a fixable defect. Re-dispatch the phase — spawn a fresh subagent per step 2 — counting against its **per-phase rework-retry budget**. When the budget is exhausted (the phase still isn't converging), **park it `blocked`** with a `code`-stage reason and continue: use `rdm_phase_update` with `project: {proj_param}, roadmap: "<slug>", phase: "<phase>", status: "blocked", reason: "[code] rework budget exhausted: <what kept failing>"`. Parking it `blocked` is what lets `{t_next}` move past it instead of handing you the same `in-progress` phase forever.
   - **escalated** → `rdm-dispatch-phase` already parked the phase `blocked` (stage `[plan]` or `[code]`). Do nothing more to it; continue the loop with the remaining actionable phases.

4. **Repeat** until a stop condition fires.

## Budgets and stop conditions

The per-phase rework-retry budget and the plan/escalation rules are defined **once** in `docs/escalation-protocol.md` — the single shared source the dispatch flow and this loop both apply. Do **not** redefine them here; follow that doc for what escalates, what retries, and the retry count. In addition, autopilot bounds the *whole run*:

- **Global step budget** — a cap on total phase dispatches per run, so a pathological roadmap can never loop forever even if every phase keeps reworking. When hit, stop and summarize.
- `--max-phases N` — the user-supplied bound, applied the same way.

Stop the loop when **any** of these holds:

- `{t_next}` returns `nothing` (no actionable phase left — the roadmap is fully `reviewed`/`done` or everything remaining is parked/terminal).
- `{t_next}` returns `blocked-on-dependencies` (the roadmap is waiting on another roadmap).
- All remaining work is `blocked`/escalated (every further `{t_next}` call yields `nothing`).
- `--max-phases` or the global step budget is reached.

## Summary (always emitted)

At the end of **every** run — whatever stopped it — print a summary:

- **Phases completed** this run (reached `reviewed`), in order.
- **Tasks filed** by the dispatched runs (side-work discovered during implementation/review).
- **Escalations awaiting the user**, each tagged `plan` vs `code`, with the parked reason. Point the user at the batch queue — the `rdm review blocked` command lists every parked escalation at once.
- Whether landing ran (`--land`) or the reviewed work is left on the `roadmap/<slug>` branch for a human to land.

## Relation to review

Autopilot is the **active driver**: it pushes a roadmap forward phase by phase, and every dispatched phase actively runs review (via the dispatched `rdm-review`) before advancing — nothing is left parked in `needs-review` for a passive net to catch. Both paths leave the `Done:` line to `rdm-review` — autopilot writes one only via the dispatched `rdm-review`, never by hand.
