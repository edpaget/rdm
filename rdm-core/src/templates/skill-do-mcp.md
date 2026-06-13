---
name: rdm-do
description: Implement an rdm roadmap phase or work on an rdm task — plan, execute, then finalize into needs-review for rdm-review
allowed-tools:
  - Read
  - Glob
  - Grep
  - Write
  - Edit
  - EnterPlanMode
  - ExitPlanMode
  - {t_phase_list}
  - {t_phase_show}
  - {t_phase_update}
  - {t_task_list}
  - {t_task_show}
  - {t_task_update}
  - {t_task_create}
---

Implement a roadmap phase or work on a task. One shared flow: find the target → mark in-progress → plan → execute → review with the user → finalize into `needs-review`.
{principles}
## Run modes

`$ARGUMENTS` may include `--auto` to select the run mode:

- **interactive** (default): plan → wait for approval → implement → review with the user → finalize. The approval and review gates pause for human input.
- **`--auto`** (non-interactive): skip the approval and review gates and proceed autonomously — build the plan, implement it, and finalize without waiting for a human.

For unattended Claude Code runs (where no human is present to approve permission prompts), launch with `--permission-mode auto` (or `bypassPermissions` in a sandbox) so file edits and rdm tool calls don't block on prompts.

Worktree isolation (working in a dedicated git worktree) is not available in this MCP variant — `rdm worktree` is CLI-only and there is no MCP tool for it yet, so this skill works in the live checkout. Use the CLI `rdm-do` skill if you want worktree isolation.

## Argument forms

`$ARGUMENTS` selects the flow:

- `<roadmap-slug> [phase-number]` → **phase flow**.
- `--task <slug>` → **task flow**.
- empty → **discovery**: list in-progress phases and open tasks, then ask the user which to work on:
  - use `rdm_search` with `project: {proj_param}, query: "", status: "in-progress", type: "phase"`
  - use `rdm_task_list` with `project: {proj_param}`

## Steps

1. **Parse `$ARGUMENTS`** and pick the flow (phase, task, or discovery — see above).
2. **Find and show the target:**
   - **phase**: if no phase number was given, use `rdm_phase_list` with `project: {proj_param}, roadmap: "<slug>"` and pick the first `not-started` or `in-progress` phase. Then use `rdm_phase_show` with `project: {proj_param}, roadmap: "<slug>", phase: "<phase>"` for full context, steps, and acceptance criteria.
   - **task**: if a slug was provided, use `rdm_task_show` with `project: {proj_param}, task: "<slug>"`. Otherwise present the task list and ask the user which to work on.
3. **Mark in-progress:**
   - phase: use `rdm_phase_update` with `project: {proj_param}, roadmap: "<slug>", phase: "<phase>", status: "in-progress"`
   - task: use `rdm_task_update` with `project: {proj_param}, task: "<slug>", status: "in-progress"`
4. **Enter plan mode** with the `EnterPlanMode` tool, then **create an implementation plan** _(interactive only; `--auto` skips the approval gate and proceeds to implement)_. The plan should:
   - Break the phase/task into concrete implementation steps based on its description and acceptance criteria.
   - Include a final step: "Review changes with user and finalize".
5. **Wait for user approval** _(interactive only)_: do not proceed until the plan is accepted. Then use `ExitPlanMode` to switch back to execution mode.
6. **Execute the plan**: implement each step, following the plan and any acceptance criteria.
7. **Review with user** _(interactive only; `--auto` finalizes without waiting)_: present a summary of the changes and ask the user to confirm they are ready to finalize.
8. **Finalize:** on user acceptance, commit the implementation changes, then transition the item to `needs-review`:
   - phase: use `rdm_phase_update` with `project: {proj_param}, roadmap: "<slug>", phase: "<phase>", status: "needs-review"`
   - task: use `rdm_task_update` with `project: {proj_param}, task: "<slug>", status: "needs-review"`

   **Do NOT emit a `Done:` line in the commit message.** The `rdm-review` skill produces the `Done:` line when review passes; an item sitting in `needs-review` is the sentinel that signals a review is pending. Use the exact roadmap slug / phase stem / task slug from the rdm tools you used earlier — do NOT invent or paraphrase them. The commit is left for merge to main (review takes it `needs-review` → `reviewed`, and the merge hook flips it to `done`).

## Side-work

If you discover bugs or unrelated improvements while working, do not fix them inline — create a tagged task instead so the work is findable later:

Use `rdm_task_create` with `project: {proj_param}, slug: "<slug>", title: "Description", body: "Details.", tags: ["<tag1>", "<tag2>"]`.

Use lowercase kebab-case tags and prefer ones already present in the project (check with `rdm_search` `query: "", tags: ["<candidate>"], project: {proj_param}`).
