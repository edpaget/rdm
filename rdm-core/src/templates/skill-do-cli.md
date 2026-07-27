---
name: rdm-do
description: Implement an rdm roadmap phase or work on an rdm task — plan, execute, then finalize through the canonical code review
allowed-tools:
  - Read
  - Bash
  - Glob
  - Grep
  - Write
  - Edit
  - EnterPlanMode
  - ExitPlanMode
  - Agent
  - Workflow
---

Implement a roadmap phase or work on a task. One shared flow: find the target → mark in-progress → plan → execute → review with the user → finalize through the canonical code review.
{principles}
## Run modes

`$ARGUMENTS` may include `--auto` to select the run mode:

- **interactive** (default): plan → wait for approval → implement → review with the user → finalize. The approval and review gates pause for human input.
- **`--auto`** (non-interactive): skip the approval and review gates and proceed autonomously. This splits by flow:
  - **phase flow** (`--auto <roadmap-slug> [phase-number]`): after marking the phase in-progress and entering its worktree (steps 1-5 below, unchanged), route straight into the `dispatch-phase` Workflow instead of re-running the prose plan/implement/review steps — see `## Auto phase dispatch` below.
  - **task flow** (`--auto --task <slug>`): after marking the task in-progress and entering its per-task worktree (steps 1-5 below, unchanged), route straight into the `dispatch-phase` Workflow with `{ task: <slug> }` instead of re-running the prose plan/implement/review steps — see `## Auto task dispatch` below.

For unattended Claude Code runs (where no human is present to approve permission prompts), launch with `--permission-mode auto` (or `bypassPermissions` in a sandbox) so worktree edits and bash commands don't block on prompts.

## Argument forms

`$ARGUMENTS` selects the flow:

- `<roadmap-slug> [phase-number]` → **phase flow**.
- `--task <slug>` → **task flow**.
- empty → **discovery**: list in-progress phases and open tasks, then ask the user which to work on:
  ```bash
  rdm search "" --status in-progress --type phase {proj_flag}
  rdm task list {proj_flag}
  ```

## Steps

1. **Parse `$ARGUMENTS`** and pick the flow (phase, task, or discovery — see above).
2. **Find and show the target:**
   - **phase**: if no phase number was given, run `rdm phase list --roadmap <slug> {proj_flag}` and pick the first `not-started` or `in-progress` phase. Then `rdm phase show <phase> --roadmap <slug> {proj_flag}` for full context, steps, and acceptance criteria.
   - **task**: if a slug was provided, `rdm task show <slug> {proj_flag}`. Otherwise present the task list and ask the user which to work on.
3. **Mark in-progress:**
   - phase: `rdm phase update <phase> --status in-progress --no-edit --roadmap <slug> {proj_flag}`
   - task: `rdm task update <slug> --status in-progress --no-edit {proj_flag}`
   - land the status change: `rdm commit -m "chore(plan): start <phase-or-task>"`
4. **Get into the roadmap's worktree (one worktree per roadmap, work in place).** Each roadmap gets a *single* worktree on branch `roadmap/<slug>`, and **every phase of that roadmap is implemented in place in it**. Entry therefore happens at most **once** — on the first phase, from the main checkout — so the session never re-enters or nests. Run `rdm worktree current --format json` and compare the current worktree's roadmap to the target `<slug>`. (`worktree current` reports `item` = the roadmap slug for a roadmap worktree, or `<roadmap>/<stem>` for a legacy per-phase worktree — compare the roadmap portion against `<slug>`.)
   - **Match** (current worktree's roadmap == `<slug>`): you are already in the target roadmap's worktree → **work in place**. Skip `worktree add` and entry. This is the common case for every phase after the first.
   - **None** (output is `null` — main checkout or not in a worktree): ensure the roadmap worktree exists with `rdm worktree add <slug> {proj_flag}` (idempotent — reuses it if present), take the `path` it prints, then enter it **once**. Claude Code may use `EnterWorktree({path})` as a one-time convenience; any other host (Pi, web, etc.) `cd`s into / launches in the printed `path`. Because the model never re-enters, plain `cd`/launch is **fully correct** — `EnterWorktree` is a convenience, **not** a correctness dependency.
   - **Mismatch** (current worktree's roadmap is a *different* roadmap): interactive → **ask** the user whether to switch to the target roadmap's worktree; `--auto` → switch to it — ensure it exists with `rdm worktree add <slug> {proj_flag}`, then **relaunch / `cd` in the printed `path`** to enter it. Note `EnterWorktree` is *not* usable here: from inside another worktree its `path` form is rejected unless the target lives under `.claude/worktrees/`, which rdm worktrees do not — so relaunch is the entry path, and it is fully correct. (`EnterWorktree` only applies to the one-time entry from the **main checkout** in the *None* case above.)

   **Tasks keep their own per-task worktree.** For the task flow, run `rdm worktree add task/<slug> {proj_flag}` and enter the printed `path` the same way (one-time `EnterWorktree` convenience, or `cd`/launch).

   Do the rest of the work in that worktree, so your changes are isolated from the live checkout.
5. **For `--auto` (either flow), skip steps 5-10 below and jump straight to `## Auto phase dispatch` / `## Auto task dispatch` — steps 5-10 are the interactive prose path only.** Enter plan mode with the `EnterPlanMode` tool, then **create an implementation plan** _(interactive only; `--auto` skips the approval gate and proceeds to implement)_. The plan should:
   - Break the phase/task into concrete implementation steps based on its description and acceptance criteria.
   - Include a final step: "Review changes with user and finalize".
6. **Review the implementation plan** _(both modes)_: run the `rdm-plan-review` skill with `--implementation-plan` against the plan drafted in the previous step, covering coherence and architectural fit.
   - **interactive**: surface the verdict and findings alongside the plan, before the approval gate.
   - **`--auto`**: never wait on the verdict — fold every surviving **blocking** finding back into the plan text before continuing to the execute step. If a blocking finding can't be resolved by editing the plan (genuine ambiguity or an architectural decision), don't drop it silently: file it via the Side-work convention (`rdm task create ... --tags plan-review --no-plan-review`) — `--no-plan-review` keeps the filed finding from being stamped `needs-plan-review` itself.
7. **Wait for user approval** _(interactive only)_: do not proceed until the plan is accepted. Then use `ExitPlanMode` to switch back to execution mode.
8. **Execute the plan**: implement each step, following the plan and any acceptance criteria.
9. **Review with user** _(interactive only; `--auto` finalizes without waiting)_: present a summary of the changes and ask the user to confirm they are ready to finalize.
10. **Finalize — commit, then actively run the canonical code review.** Finalize never *parks* work for someone else to review later; it drives the review itself. This runs in **both** modes: interactively (after the step-9 confirmation) and under `--auto` (which skips only the human confirmation, never the review).

    1. **Commit the implementation diff** — a plain `git commit` of the code diff in the **source repo**, on the worktree's branch.
    2. **Mark the item `needs-review`** as a transient marker so the review has a well-defined starting state, and land that plan-repo change:
       - phase: `rdm phase update <phase> --status needs-review --no-edit --roadmap <slug> {proj_flag}`
       - task: `rdm task update <slug> --status needs-review --no-edit {proj_flag}`
       - land the plan-repo status change: `rdm commit -m "chore(plan): finalize <phase-or-task>"`

       This `rdm commit` is a **separate, plan-repo** git commit — distinct from the source-repo `git commit` of the implementation diff above. Do not conflate the two: one lands your code, the other lands the plan-repo status update.
    3. **Immediately invoke the `rdm-review` skill** against the item. It is the canonical review — the same find → refute → filter → verdict → gate pipeline the autonomous lane runs — so every finalize is actively reviewed, in either mode.
    4. **The review owns the gate.** It persists the status its outcome maps to — `reviewed`, `in-progress` (rework), or `blocked` with a `[code]`-prefixed reason (escalated) — and on `reviewed` **only**, it amends the land-time completion trailer onto the branch commit.

    **Never hand-type the completion trailer.** Finalize does not write it at all: on the interactive path the review gate writes it, and on the autonomous path `rdm-land` writes it at land time. Both source the exact line from `rdm hook done-line --roadmap <slug> --phase <stem>` (or `--task <slug>`), so the format string has exactly one home. Use the exact roadmap slug / phase stem / task slug from the rdm commands you ran earlier — do NOT invent or paraphrase them. The commit stays on the worktree's branch, which is left for merge to main (the merge hook flips `reviewed` → `done`).

    **Single pass.** If the review returns `rework`, the item is left `in-progress` and you must surface that to the user with its findings — do not silently loop. Re-run this skill to take another pass.

## Auto phase dispatch (--auto, phase flow only)

1. Gather the mechanical values yourself first and hand them to the workflow. You already ran `rdm phase show` in step 2 and stamped the phase `in-progress` in step 3 — reuse both rather than making the workflow re-do them with dedicated subagents. Every one of these args is **optional**; the workflow falls back to its own in-workflow fetch for anything you omit or get wrong.
   - Re-run `rdm phase show <phase> --roadmap <slug> {proj_flag} --format json` if you did not keep the parsed JSON.
   - Resolve the five per-step model ids. Let `T` be the phase JSON's `model` tier: run `rdm model resolve plan --tier T` and `rdm model resolve implement --tier T` **with** the tier hint when `T` is a non-empty string, without it otherwise; then `rdm model resolve review-find`, `rdm model resolve review-verify` and `rdm model resolve mechanical` with **no** `--tier` argument, always.
   - Assemble `phaseMeta` as `{ roadmap, phase, stem, model, body, models: { plan, implement, review_find, review_verify, mechanical } }`, copying the `body` and the model ids **verbatim** — never summarize or invent one. The workflow applies an all-or-nothing guard: a payload missing the body or any one of the five ids is rejected outright and the in-workflow fetch runs instead.
   - Pass `alreadyInProgress: true` **only** because step 3 already ran `rdm phase update <phase> --status in-progress` and it exited 0 — that write happens strictly before the workflow is invoked, so the phase is observably `in-progress` earlier than the workflow's own stamp would make it. Never pass it for a `--plan-only` invocation.
2. Invoke the `dispatch-phase` Workflow (`.claude/workflows/dispatch-phase.js`, provisioned automatically by `rdm agent-config claude --skills`) via the `Workflow` tool with `{ roadmap: <slug>, phase: <stem>, phaseMeta, alreadyInProgress: true }`; block for its returned OUTCOME.
3. Interpret the OUTCOME and persist status. The OUTCOME carries the canonical gate policy **as data** — `outcome.status` (the status the canonical review mapped this outcome to), `outcome.reason` (already carrying its `[code]`/`[plan]` gate tag), and `outcome.writesCompletion` — so read those fields rather than restating the map. dispatch-phase's code-review stage IS the canonical review, so the work is already reviewed by the time you see the OUTCOME.
   - `reviewed` → persist `outcome.status`: `rdm phase update <phase> --status reviewed --no-edit --roadmap <slug> {proj_flag}` then `rdm commit -m "chore(plan): finalize <phase>"`
   - `rework` → this lane is single-pass, so park it in the escalation queue with `outcome.reason` instead of leaving it merely in-progress: `rdm phase update <phase> --status blocked --reason "[code] <outcome.summary>" --no-edit --roadmap <slug> {proj_flag}` then `rdm commit -m "chore(plan): park <phase>"`
   - `escalated` → same, with the plan-gate tag: `rdm phase update <phase> --status blocked --reason "[plan] <outcome.summary>" --no-edit --roadmap <slug> {proj_flag}` then `rdm commit -m "chore(plan): park <phase>"`
   - Do NOT add a `Done:` line here. `outcome.writesCompletion` is `true` only on `reviewed`, and it is `rdm-land` that reads it and synthesizes the trailer via `rdm hook done-line` at land time — no manual rebase, and no pre-step before `rdm-land`.
4. Return the OUTCOME JSON verbatim as the final message.

This section applies only to `--auto` + the phase flow. Interactive `rdm-do` (either flow) is unaffected and keeps the steps above unchanged.

## Auto task dispatch (--auto, task flow only)

The task-flow twin of the phase dispatch above. A task belongs to no roadmap, carries no difficulty/model tier, and lives in its own `task/<slug>` worktree.

1. Gather the mechanical values yourself first and hand them to the workflow. You already ran `rdm task show` in step 2 and stamped the task `in-progress` in step 3 — reuse both rather than making the workflow re-do them with dedicated subagents. Every one of these args is **optional**; the workflow falls back to its own in-workflow fetch for anything you omit or get wrong.
   - Re-run `rdm task show <slug> {proj_flag} --format json` if you did not keep the parsed JSON.
   - Resolve the five per-step model ids. A task carries no tier, so run all five with **no** `--tier` argument: `rdm model resolve plan`, `rdm model resolve implement`, `rdm model resolve review-find`, `rdm model resolve review-verify`, `rdm model resolve mechanical`.
   - Assemble `taskMeta` as `{ task, body, models: { plan, implement, review_find, review_verify, mechanical } }`, copying the `body` and the model ids **verbatim** — never summarize or invent one. The same all-or-nothing guard applies: a payload missing the body or any one of the five ids is rejected and the in-workflow fetch runs instead.
   - Pass `alreadyInProgress: true` **only** because step 3 already ran `rdm task update <slug> --status in-progress` and it exited 0.
2. Invoke the `dispatch-phase` Workflow (`.claude/workflows/dispatch-phase.js`) via the `Workflow` tool with `{ task: <slug>, taskMeta, alreadyInProgress: true }`; block for its returned OUTCOME. The task-mode OUTCOME is keyed by `task` (the slug), not `roadmap`/`phase`.
3. Interpret the OUTCOME and persist status (a true mirror of the phase contract). The task-mode OUTCOME carries the same canonical `outcome.status` / `outcome.reason` / `outcome.writesCompletion` fields — read them rather than restating the map. `escalated` maps to the `blocked` **task** status; it is never downgraded to `in-progress`.
   - `reviewed` -> persist `outcome.status`: `rdm task update <slug> --status reviewed --no-edit {proj_flag}` then `rdm commit -m "chore(plan): finalize <slug>"`
   - `rework` -> single-pass park with `outcome.reason`: `rdm task update <slug> --status blocked --reason "[code] <outcome.summary>" --no-edit {proj_flag}` then `rdm commit -m "chore(plan): park <slug>"`
   - `escalated` -> `rdm task update <slug> --status blocked --reason "[plan] <outcome.summary>" --no-edit {proj_flag}` then `rdm commit -m "chore(plan): park <slug>"`
   - Do NOT add a `Done:` line here. On `reviewed` the OUTCOME's `writesCompletion` is `true` and `rdm-land` is the land-time writer — it synthesizes `Done: task/<slug>` via `rdm hook done-line` before the rebase.
4. Return the OUTCOME JSON verbatim as the final message.

Tasks always dispatch at the fixed `medium` tier (there is no task estimate), so the `large`-tier gate tightening never applies. Task bodies often carry no formal acceptance criteria; the plan/review gates tolerate their absence.

**Single-item scope:** unlike the `autopilot` workflow's advance/park loop, this single-item entry point parks on the first `rework`/`escalated` OUTCOME rather than re-dispatching against a rework budget — re-run `rdm-do --auto <roadmap> <phase>` (or `--auto --task <slug>`) by hand to retry. Skipping the outer rework-retry here is intentional, not an oversight.

## Side-work

If you discover bugs or unrelated improvements while working, do not fix them inline — create a tagged task instead so the work is findable later:

```bash
rdm task create <slug> --title "Description" --body "Details." --tags <tag1>,<tag2> --no-edit {proj_flag}
rdm commit -m "chore(plan): file side-work task <slug>"  # land the batch
```

Use lowercase kebab-case tags and prefer ones already present in the project (check with `rdm search "" --tag <candidate> {proj_flag}`).
