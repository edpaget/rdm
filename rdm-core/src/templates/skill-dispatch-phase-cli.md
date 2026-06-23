---
name: rdm-dispatch-phase
description: Dispatch a single rdm phase end-to-end in an isolated worktree on its assigned model tier — plan, independently review the plan, implement, then code-review — and return a structured outcome
allowed-tools:
  - Read
  - Bash
  - Glob
  - Grep
  - Write
  - Edit
  - Agent
---

Run **one** rdm phase to completion in isolation, then report a structured outcome. This is the per-phase unit of autonomous execution: a fresh subagent implements the phase seeded with **only that phase's context**, in its own worktree, on the model tier chosen during estimation. An orchestrator (the autonomous roadmap loop) calls this skill repeatedly, once per phase.

Unlike interactive `rdm-do`, there is no human to approve the tactical plan before code is written. This skill replaces that gate with a *lightweight, automated second opinion*: a **separate** agent reviews the plan against the phase's acceptance criteria before any code is written. Validating the plan (the intermediate artifact) is far cheaper than reworking code, and the reviewer must be independent — the planner never grades its own plan.
{principles}
## Dispatch contract

**Inputs** (from `$ARGUMENTS` and the phase record):

- `roadmap` — roadmap slug.
- `phase` — phase stem or number.
- `model tier` — read from the phase record (see step 2); the implementer subagent runs on this tier.
- `worktree path` — created or located in step 3.

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

## Steps

1. **Parse `$ARGUMENTS`** as `<roadmap-slug> <phase>` (stem or number). This skill is non-interactive: it never waits for human approval. Launch unattended runs with `--permission-mode auto` (or `bypassPermissions` in a sandbox) so worktree edits and bash commands don't block on prompts.
2. **Read the phase and its model tier:** `rdm phase show <phase> --roadmap <slug> {proj_flag}`. Capture the full body (context, steps, acceptance criteria) and the assigned **Model** tier (also visible via `rdm phase list --roadmap <slug> {proj_flag}`). The body captured here is the **only** phase context you pass downstream — do not pass roadmap-wide or conversation context.
3. **Create/locate the worktree and mark in-progress:**
   - `rdm worktree add <slug>/<phase-stem> {proj_flag}` — idempotent; prints the worktree path. `cd` into it and do all implementation there.
   - `rdm phase update <phase> --status in-progress --no-edit --roadmap <slug> {proj_flag}`
4. **Plan (implementer subagent):** dispatch an implementer subagent with the `Agent` tool, running on the phase's **model tier** (pass it as the agent's model). Seed it with **only** the phase body from step 2 and the repo — *not* this orchestrator's accumulated context. Ask it to produce a concrete tactical plan: the implementation steps that satisfy every acceptance criterion, the crates/files it will touch, and the tests it will add. Have it return the plan only — no code yet.
5. **Plan gate (separate reviewer subagent):** dispatch a *different*, lightweight plan-review subagent with the `Agent` tool. Give it the phase body and the proposed plan — and nothing else. It checks the plan against:
   - **Acceptance criteria** — does the plan satisfy every AC?
   - **Scope** — does it stay in scope, adding no work the phase did not ask for?
   - **Architecture** — does it target the right crates per the core/cli/server separation (core is the source of truth; cli/server are thin)?

   It returns exactly one verdict:
   - **approve** — proceed to implement.
   - **revise** — concrete, bounded feedback. Apply it **once**: the implementer revises the plan a single time, then proceeds. Do **not** loop — there is at most one revise round.
   - **escalate** — genuine AC ambiguity or an architectural decision with no clear default. Stop and park the phase per the **escalation protocol** (see below): record the blocker as a `plan`-stage escalation and return the **escalated** outcome with it in `findings`.
     ```bash
     rdm phase update <phase> --status blocked --reason "[plan] <the decision or ambiguity>" --no-edit --roadmap <slug> {proj_flag}
     ```

   The gate is bounded: one review pass plus at most one revise round, then proceed or escalate. Never an unbounded critique loop. Exhausting the single revise round without converging is itself a `plan`-stage escalation.
6. **Implement:** on `approve` (or after the single accepted revision), the implementer subagent implements the approved plan inside the worktree and commits the changes. Do **not** emit a `Done:` line — that is owned by `rdm-review`.
7. **Code review:** run the `rdm-review` skill on the result (`<slug> <phase>`). It owns the `needs-review` → `reviewed` gate and the `Done:` line. Map its verdict onto the outcome:
   - **pass** → `rdm-review` leaves the phase `reviewed` with a `Done:` line on the branch → return **reviewed**.
   - **fail (fixable defect)** → allow **one** bounded rework pass (the implementer addresses the review findings and re-runs `rdm-review`). If it still fails, `rdm-review` returns the phase to `in-progress` → return **rework**. Never loop indefinitely on rework.
   - **fail (decision/blocker, not a defect)** → if review surfaces a genuine AC ambiguity or architectural decision rather than a fixable defect — or the single rework pass is exhausted without converging — park it as a `code`-stage escalation instead of retrying, and return **escalated**:
     ```bash
     rdm phase update <phase> --status blocked --reason "[code] <the decision or ambiguity>" --no-edit --roadmap <slug> {proj_flag}
     ```
8. **Return the structured outcome** (the JSON above) as your final message.

## Escalation protocol

This skill follows the shared **escalation protocol** (`docs/escalation-protocol.md`) — the single definition the dispatch flow and the autonomous loop both apply. In short:

- **Routine findings never escalate.** Bugs, missing tests, doc gaps — `rdm-review` fixes them inline or files a task. They never reach the user.
- **Decisions/blockers escalate.** Ambiguous/untestable AC, an architectural decision with no clear default, an exhausted plan-revise or rework budget, or a hard blocker (missing dependency/credential, conflicting requirement).
- **Park, don't interrupt.** On autopilot, record the escalation by setting the phase `blocked` with a stage-tagged `--reason` (`[plan]` or `[code]`), then return the **escalated** outcome. The reason is preserved across a later resume. The user reviews the whole queue at once with `rdm review blocked {proj_flag}` rather than being interrupted mid-run.

## Context isolation

The whole point is a *fresh* per-phase agent. When you dispatch the implementer and the plan reviewer:

- Pass the phase body captured in step 2 **explicitly** as their context.
- Do **not** leak unrelated roadmap phases, other tasks, or this orchestrator's conversation history into either subagent.
- The plan reviewer is a *separate* agent from the implementer — the planner never grades its own plan.

## Side-work

If you discover bugs or unrelated improvements while working, do not fix them inline — create a tagged task instead so the work is findable later:

```bash
rdm task create <slug> --title "Description" --body "Details." --tags <tag1>,<tag2> --no-edit {proj_flag}
```

Use lowercase kebab-case tags and prefer ones already present in the project (check with `rdm search "" --tag <candidate> {proj_flag}`).
