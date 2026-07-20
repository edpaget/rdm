---
name: rdm-dispatch-phase
description: Dispatch a single rdm phase end-to-end in the roadmap's shared worktree on its assigned model tier — plan, independently review the plan, implement, then code-review — and return a structured outcome
allowed-tools:
  - Read
  - Glob
  - Grep
  - Write
  - Edit
  - Agent
  - {t_phase_list}
  - {t_phase_show}
  - {t_phase_update}
  - {t_worktree_add}
  - {t_task_create}
---

Run **one** rdm phase to completion, then report a structured outcome. This is the per-phase unit of autonomous execution: a fresh subagent implements the phase seeded with **only that phase's context**, on the model tier chosen during estimation. The orchestrator (the autonomous roadmap loop) calls this skill repeatedly, once per phase — and every phase of the roadmap is implemented **in place in the same worktree**, the roadmap's shared `roadmap/<slug>` worktree, reused across phases rather than created fresh per phase.

Unlike interactive `rdm-do`, there is no human to approve the tactical plan before code is written. This skill replaces that gate with a *lightweight, automated second opinion*: a **separate** agent reviews the plan against the phase's acceptance criteria before any code is written. Validating the plan (the intermediate artifact) is far cheaper than reworking code, and the reviewer must be independent — the planner never grades its own plan.
{principles}
## Dispatch contract

**Inputs** (from `$ARGUMENTS` and the phase record):

- `roadmap` — roadmap slug.
- `phase` — phase stem or number.
- `model tier` — read from the phase record (see step 2); the planning and implementer subagents both run on this tier.
- `worktree path` — the roadmap worktree, created or reused in step 3.

**Output** — a structured outcome you print as the final result, so the orchestrator can act on it:

```json
{
  "roadmap": "<slug>",
  "phase": "<stem>",
  "outcome": "reviewed | rework | escalated",
  "summary": "<one-line result>",
  "findings": "<plan-review and code-review notes, or the blocker that forced escalation>"
}
```

- **reviewed** — code review passed; the phase is `reviewed` with a `Done:` line on the branch.
- **rework** — code review failed after the bounded retry; the phase is back to `in-progress`.
- **escalated** — a genuine ambiguity in the AC or an architectural/design decision with no clear default blocked progress; the phase is set to `blocked` and the blocker is described in `findings` for a human (surfaced by the escalation protocol).

## Mandatory dispatch — no inline work

Dispatch is a **MUST**, not a suggestion, and it does not degrade based on how capable the current session feels. You (the dispatcher) MUST NOT write the plan, review it, or implement the phase yourself in this context. Each of the three roles — planner (step 4), plan reviewer (step 5), implementer (step 6) — MUST be a separate `Agent` subagent, dispatched synchronously. There is no fallback path where this context does that work directly, for any model, on any phase.

**Failure mode to avoid — inline-collapse:** reading the phase body and then drafting the plan, critiquing it, or writing the code yourself in this same context — treating "dispatch a planning/review/implementer subagent" as narration rather than an actual `Agent` tool call, or reporting a result as though a subagent produced it when none was spawned. This is **inline-collapse**: it silently collapses three independent roles into one, discarding the plan gate and the independent code review this skill exists to guarantee. It MUST NOT happen.

Before each of steps 4, 5, and 6, and again before treating that step's result as final, you MUST complete this checklist:

1. **Declare** which subagent you are about to dispatch and its role (e.g. "Dispatching planner subagent for phase `<stem>`, model `<tier>`").
2. **Call** the `Agent` tool synchronously for that role — never narrate it, actually invoke it.
3. **Block** until it returns; do not advance on any other signal.
4. **Verify** the returned result matches that step's expected shape (a plan document / a verdict / committed code) before moving to the next step. If no `Agent` call actually happened, stop and redo the step — do not substitute your own inline work for the missing subagent's output.

## Steps

1. **Parse `$ARGUMENTS`** as `<roadmap-slug> <phase>` (stem or number). This skill is non-interactive: it never waits for human approval. Launch unattended runs with `--permission-mode auto` (or `bypassPermissions` in a sandbox) so file edits and rdm tool calls don't block on prompts.
2. **Read the phase and its model tier:** use `rdm_phase_show` with `project: {proj_param}, roadmap: "<slug>", phase: "<phase>"`. Capture the full body (context, steps, acceptance criteria) and the assigned **model** tier (also visible via `rdm_phase_list` with `project: {proj_param}, roadmap: "<slug>"`). The body captured here is the **only** phase context you pass downstream — do not pass roadmap-wide or conversation context.
3. **Create/locate the roadmap worktree and mark in-progress:**
   - Use the `{t_worktree_add}` MCP tool with `project: {proj_param}, item: "<slug>"` — the **roadmap** ref (not `<slug>/<phase-stem>`). It is idempotent and returns the worktree `path` and `branch`: the first phase creates the `roadmap/<slug>` worktree; every later phase of the same roadmap reuses it. `cd` into that path (or open it) before editing files. Run the MCP server from your project (code) repo — the tool refuses to run inside the plan repo.
   - Use `rdm_phase_update` with `project: {proj_param}, roadmap: "<slug>", phase: "<phase>", status: "in-progress"`.
4. **Plan (planning subagent):** dispatch a planning subagent **synchronously** with the `Agent` tool — block on its returned result — running on the phase's **model tier** (pass it as the agent's model). Seed it with **only** the phase body from step 2 and the repo — *not* this orchestrator's accumulated context. Ask it to return a **self-contained plan document** — everything the implementer needs written down, because a **new** agent implements it in step 6 with no access to this planner's exploration context. The document must contain:
   - **Steps per AC** — the implementation steps that satisfy every acceptance criterion, mapped one-to-one to the ACs.
   - **File/crate navigation map** — every crate and file it will touch, so the implementer inherits navigation instead of re-deriving it through fresh exploration (this is what avoids paying for discovery twice).
   - **Test list per AC** — the tests to add, each mapped to the AC it covers.
   - **Edge cases / error paths per AC** — for each AC, the edge cases and error paths it implies, not just the happy path.
   - **Cross-phase / cross-crate dependencies** — the other phases, crates, or assumptions the plan depends on (e.g. a type or function an earlier phase must already provide), so they are declared rather than silently assumed.

   Have it return the plan document only — no code yet.

   **Self-check before proceeding to step 5:** confirm the `Agent` call above actually returned a plan document from a dispatched planning subagent. If you drafted the plan yourself instead of dispatching, stop — that is the inline-collapse failure above, and this step MUST be redone with a real dispatch.
5. **Plan gate (separate reviewer subagent):** dispatch a *different*, lightweight plan-review subagent **synchronously** with the `Agent` tool, blocking on its returned verdict. Give it the phase body and the plan document — and nothing else.

   Scale the reviewer's rigor to the phase's difficulty tier (same tier `rdm-estimate` assigned: trivial/easy → small, moderate → medium, hard → large):
   - **Trivial/easy (small)** — a single reviewer subagent, holistic judgment against the checklist.
   - **Moderate (medium)** — a single reviewer subagent, but its verdict must carry per-finding evidence — for every checklist item, cite the specific AC text, plan step, or file/crate the judgment rests on. A verdict with no cited evidence is invalid and must be redone before it counts.
   - **Hard (large)** — in addition to the moderate evidence discipline, the reviewer's verdict goes through a refute pass — a third, fresh subagent (never the planner, never the first reviewer) whose only job is to refute the reviewer's verdict and evidence, in the same adversarial spirit as `rdm-review`'s verify step ("this verdict is NOT correct unless the plan and phase body prove otherwise"). It returns agreement: confirmed | overturned plus its own evidence. Reconciliation rule: the stricter of the two verdicts governs, ranked escalate > revise > approve — a refute pass can only tighten the gate, never loosen it. (This is a deliberate one-directional difference from `rdm-review`'s per-finding refute, which can also drop findings; here the refute can only catch an under-verified approval.)

   The reviewer (and, for hard phases, the refuter) checks the plan document against:
   - **Acceptance criteria → steps** — does every AC map to at least one concrete implementation step? Flag any AC with no mapped step.
   - **Acceptance criteria → tests** — does every AC map to at least one named test in the plan's per-AC test list (test-per-AC)? Flag any AC with no mapped test.
   - **Edge cases / error paths** — for each AC, does the plan enumerate the edge cases or error paths that AC implies, not just its happy path?
   - **Cross-phase / cross-crate dependencies** — does the plan declare the other phases, crates, or assumptions it depends on? Flag any undeclared dependency it silently assumes.
   - **Scope** — does the plan stay in scope, adding no work the phase did not ask for?
   - **Architecture** — does it target the right crates per the core/cli/server separation (core is the source of truth; cli/server are thin)?

   It returns exactly one verdict:
   - **approve** — proceed to implement. The approved plan document — exactly as reviewed, not a paraphrase — is what the implementer receives in step 6.
   - **revise** — concrete, bounded feedback tied to the specific checklist item(s) that failed. Apply it once: the planning subagent revises the plan document a single time. The reviewer then re-checks the revised plan against the same checklist (repeating the tier-scaled rigor above, including the refute pass for hard phases). If the revised plan now satisfies the checklist, approve it and proceed to step 6. If it still does not, do not proceed with a deficient plan — escalate (stage `plan`) per the exhausted-budget rule below. Do not loop — there is at most one revise round, win or lose.
   - **escalate** — either a genuine AC ambiguity or an architectural decision with no clear default on the first pass, or an exhausted plan-revise budget (the one allowed revision, re-checked, still does not satisfy the checklist). Either way, stop and park the phase per the **escalation protocol** (see below): set the phase to `blocked` with a stage-tagged reason (use `rdm_phase_update` with `project: {proj_param}, roadmap: "<slug>", phase: "<phase>", status: "blocked", reason: "[plan] <the decision or ambiguity>"`), and return the **escalated** outcome with it in `findings`.

   The gate is bounded: one review pass plus at most one revise round (with its own re-check), then proceed or escalate. Never an unbounded critique loop. Exhausting the single revise round without converging is itself a `plan`-stage escalation — never proceed to implementation with a plan that still fails the checklist.

   **Self-check before proceeding to step 6:** confirm the plan-review verdict above actually came from a dispatched reviewer subagent (and refuter, for hard phases) — not from your own judgment. A verdict you produced yourself without dispatching MUST be discarded and redone with a real `Agent` call.
6. **Implement (new implementer subagent):** on `approve` (or after the single accepted revision), dispatch a **new** implementer subagent **synchronously** with the `Agent` tool — block on its returned result — on the phase's **model tier**. Seed it with **only** the phase body from step 2 and the approved plan document from steps 4–5 — *not* the planning subagent's accumulated context (its exploration, rejected alternatives, or files it opened but will not touch). The plan document's file/crate navigation map and per-AC test list are what let it skip re-deriving discovery. It implements the approved plan document inside the worktree and commits the changes. Do **not** emit a `Done:` line YET — that is owned by `rdm-review`, which adds it on a passing review (a deferred two-stage protocol, not a contradiction).

   **Self-check before proceeding to step 7:** confirm the implementation above was produced by a dispatched implementer subagent's `Agent` call, not written directly in this context. Code you wrote yourself instead of the dispatched implementer MUST NOT be treated as this phase's implementation — redo the step with a real dispatch.
7. **Code review:** run the `rdm-review` skill on the result (`<slug> <phase>`). It owns the `needs-review` → `reviewed` gate and the `Done:` line, and it speaks the **same** outcome vocabulary this skill returns — `reviewed | rework | escalated`. Map its outcome onto yours one-for-one:
   - **reviewed** → `rdm-review` leaves the phase `reviewed` with a `Done:` line on the branch → return **reviewed**.
   - **rework (fixable defect)** → allow **one** bounded rework pass: dispatch a **fresh** implementer subagent (per step 6, seeded the same way plus the review findings) to address the findings and re-run `rdm-review` — never resume the returned implementer by message. If it still comes back `rework`, `rdm-review` returns the phase to `in-progress` → return **rework**. Never loop indefinitely on rework. The rework implementer MUST also be a dispatched `Agent` subagent — never inline work in this context.
   - **fail (decision/blocker, not a defect)** → if review surfaces a genuine AC ambiguity or architectural decision rather than a fixable defect — or the single rework pass is exhausted without converging — park it as a `code`-stage escalation (use `rdm_phase_update` with `status: "blocked", reason: "[code] <the decision or ambiguity>"`) and return **escalated** instead of retrying.
8. **Return the structured outcome** (the JSON above) as your final message.

## Escalation protocol

This skill follows the shared **escalation protocol** (`docs/escalation-protocol.md`) — the single definition the dispatch flow and the autonomous loop both apply. In short:

- **Routine findings never escalate.** Bugs, missing tests, doc gaps — `rdm-review` fixes them inline or files a task. They never reach the user.
- **Decisions/blockers escalate.** Ambiguous/untestable AC, an architectural decision with no clear default, an exhausted plan-revise or rework budget, or a hard blocker (missing dependency/credential, conflicting requirement).
- **Park, don't interrupt.** On autopilot, record the escalation by setting the phase `blocked` with a stage-tagged `reason` (`[plan]` or `[code]`), then return the **escalated** outcome. The reason is preserved across a later resume. The user reviews the whole queue at once with the `rdm review blocked` command rather than being interrupted mid-run.

## Context isolation

**This isolation is mandatory, not best-effort.** The planner, plan reviewer, and implementer MUST each be a separate dispatched `Agent` subagent — never done inline by the dispatcher, and never merged into one subagent call. This holds regardless of session or model capability; it is the contract, not a situational workaround. See **Mandatory dispatch — no inline work** above for the failure mode this prevents and the required checklist.

The whole point is a *fresh* per-phase agent, and the isolation holds at three boundaries. When you dispatch the planner, the plan reviewer, and the implementer:

- Pass the phase body captured in step 2 **explicitly** as their context.
- Do **not** leak unrelated roadmap phases, other tasks, or this orchestrator's conversation history into any of the subagents.
- **Planner ↔ reviewer:** the plan reviewer is a *separate* agent from the planner — the planner never grades its own plan.
- **Planner ↔ implementer:** the step-6 implementer is a **new** agent, seeded only with the phase body and the approved plan document — never the planning agent's own context (its exploration, dead ends, or files it read but did not act on). The plan document is the *only* channel from planning to implementation, which is why step 4 requires it to be self-contained.

**Subagents never `SendMessage` their orchestrator.** The planner, plan reviewer, and implementer subagents must not attempt to `SendMessage` back to this dispatcher — there is no resolvable parent name (`"claude"` does not resolve). The dispatcher reads each subagent's returned result; that result is the sole channel for outcomes to flow back.

## Side-work

If you discover bugs or unrelated improvements while working, do not fix them inline — create a tagged task instead so the work is findable later:

Use `rdm_task_create` with `project: {proj_param}, slug: "<slug>", title: "Description", body: "Details.", tags: ["<tag1>", "<tag2>"]`.

Use lowercase kebab-case tags and prefer ones already present in the project (check with `rdm_search` `query: "", tags: ["<candidate>"], project: {proj_param}`).
