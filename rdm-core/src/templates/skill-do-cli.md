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
---

Implement a roadmap phase or work on a task. One shared flow: find the target → mark in-progress → plan → execute → review with the user → finalize through the canonical code review.
{principles}
## Run modes

`$ARGUMENTS` may include `--auto` to select the run mode:

- **interactive** (default): plan → wait for approval → implement → review with the user → finalize. The approval and review gates pause for human input.
- **`--auto`** (non-interactive): skip the approval and review gates and proceed autonomously — build the plan, implement it, and finalize without waiting for a human.

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
5. **Enter plan mode** with the `EnterPlanMode` tool, then **create an implementation plan** _(interactive only; `--auto` skips the approval gate and proceeds to implement)_. The plan should:
   - Break the phase/task into concrete implementation steps based on its description and acceptance criteria.
   - Include a final step: "Review changes with user and finalize".
6. **Review the implementation plan** _(both modes)_: run the `rdm-plan-review` skill with `--implementation-plan` against the plan drafted in the previous step, covering coherence and architectural fit.
   - **interactive**: surface the verdict and findings alongside the plan, before the approval gate.
   - **`--auto`**: never wait on the verdict — fold every surviving **blocking** finding back into the plan text before continuing to the execute step. If a blocking finding can't be resolved by editing the plan (genuine ambiguity or an architectural decision), don't drop it silently: file it via the Side-work convention (`rdm task create ... --tags plan-review`).
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

## Side-work

If you discover bugs or unrelated improvements while working, do not fix them inline — create a tagged task instead so the work is findable later:

```bash
rdm task create <slug> --title "Description" --body "Details." --tags <tag1>,<tag2> --no-edit {proj_flag}
rdm commit -m "chore(plan): file side-work task <slug>"  # land the batch
```

Use lowercase kebab-case tags and prefer ones already present in the project (check with `rdm search "" --tag <candidate> {proj_flag}`).
