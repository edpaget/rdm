---
name: rdm-do
description: Implement an rdm roadmap phase or work on an rdm task — plan, execute, then finalize through the canonical code review
allowed-tools:
  - Read
  - Glob
  - Grep
  - Write
  - Edit
  - EnterPlanMode
  - ExitPlanMode
  - Agent
  - {t_phase_list}
  - {t_phase_show}
  - {t_phase_update}
  - {t_task_list}
  - {t_task_show}
  - {t_task_update}
  - {t_task_create}
  - {t_worktree_current}
  - {t_worktree_add}
  - {t_commit}
---

Implement a roadmap phase or work on a task. One shared flow: find the target → mark in-progress → plan → execute → review with the user → finalize through the canonical code review.
{principles}
## Run modes

`$ARGUMENTS` may include `--auto` to select the run mode:

- **interactive** (default): plan → wait for approval → implement → review with the user → finalize. The approval and review gates pause for human input.
- **`--auto`** (non-interactive): skip the approval and review gates and proceed autonomously — build the plan, implement it, and finalize without waiting for a human.

For unattended Claude Code runs (where no human is present to approve permission prompts), launch with `--permission-mode auto` (or `bypassPermissions` in a sandbox) so file edits and rdm tool calls don't block on prompts.

This skill does its work in an isolated git worktree — **one worktree per roadmap**, with every phase implemented in place in it. It detects the current worktree with `{t_worktree_current}` and creates/reuses the roadmap worktree with `{t_worktree_add}` after marking the item in-progress (see the "Get into the roadmap's worktree" step). That keeps the live checkout untouched while you implement, and because the model never re-enters, a plain `cd`/open into the returned path is all any host needs.

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
   - land the status change: call `{t_commit}` with `message: "chore(plan): start <phase-or-task>"`
4. **Get into the roadmap's worktree (one worktree per roadmap, work in place):** each roadmap gets a *single* worktree on branch `roadmap/<slug>`, and **every phase of that roadmap is implemented in place in it**, so entry happens at most **once** (the first phase, from the main checkout) and the session never re-enters or nests. Use `{t_worktree_current}` and compare the current worktree's roadmap to the target `<slug>`. (`{t_worktree_current}` reports `item` = the roadmap slug for a roadmap worktree, or `<roadmap>/<stem>` for a legacy per-phase worktree — compare the roadmap portion against `<slug>`.)
   - **Match** (current worktree's roadmap == `<slug>`): you are already in the target roadmap's worktree → **work in place**. Skip `{t_worktree_add}` and entry. This is the common case for every phase after the first.
   - **None** (output is `null` — main checkout or not in a worktree): ensure the roadmap worktree exists by calling `{t_worktree_add}` with `project: {proj_param}, item: "<slug>"` (idempotent — reuses it if present). Take the returned `path` and `cd` into / open it **once** before editing files. The model never re-enters, so a plain `cd`/open is all any MCP host needs — fully correct, no special worktree-entry tool required.
   - **Mismatch** (current worktree's roadmap is a *different* roadmap): interactive → **ask** the user whether to switch to the target roadmap's worktree; `--auto` → switch to it — call `{t_worktree_add}` with `item: "<slug>"`, then `cd`/open the returned `path` (or instruct a relaunch there).

   **Tasks keep their own per-task worktree.** For the task flow, call `{t_worktree_add}` with `project: {proj_param}, item: "task/<slug>"` and `cd`/open the returned `path`.

   Run the MCP server from your project (code) repo — `{t_worktree_add}` refuses to run inside the plan repo.
5. **Enter plan mode** with the `EnterPlanMode` tool, then **create an implementation plan** _(interactive only; `--auto` skips the approval gate and proceeds to implement)_. The plan should:
   - Break the phase/task into concrete implementation steps based on its description and acceptance criteria.
   - Include a final step: "Review changes with user and finalize".
6. **Review the implementation plan** _(both modes)_: run the `rdm-plan-review` skill with `--implementation-plan` against the plan drafted in the previous step, covering coherence and architectural fit.
   - **interactive**: surface the verdict and findings alongside the plan, before the approval gate.
   - **`--auto`**: never wait on the verdict — fold every surviving **blocking** finding back into the plan text before continuing to the execute step. If a blocking finding can't be resolved by editing the plan (genuine ambiguity or an architectural decision), don't drop it silently: file it via the Side-work convention (use `rdm_task_create` with `tags: ["plan-review"]`).
7. **Wait for user approval** _(interactive only)_: do not proceed until the plan is accepted. Then use `ExitPlanMode` to switch back to execution mode.
8. **Execute the plan**: implement each step inside the worktree, following the plan and any acceptance criteria.
9. **Review with user** _(interactive only; `--auto` finalizes without waiting)_: present a summary of the changes and ask the user to confirm they are ready to finalize.
10. **Finalize — commit, then actively run the canonical code review.** Finalize never *parks* work for someone else to review later; it drives the review itself. This runs in **both** modes: interactively (after the step-9 confirmation) and under `--auto` (which skips only the human confirmation, never the review).

    1. **Commit the implementation diff** — a plain `git commit` of the code diff in the **source repo**, on the worktree's branch. This is a source-repo git operation with no MCP equivalent; delegate it to a Bash-capable subagent.
    2. **Mark the item `needs-review`** as a transient marker so the review has a well-defined starting state, and land that plan-repo change:
       - phase: use `rdm_phase_update` with `project: {proj_param}, roadmap: "<slug>", phase: "<phase>", status: "needs-review"`
       - task: use `rdm_task_update` with `project: {proj_param}, task: "<slug>", status: "needs-review"`
       - land the plan-repo status change: call `{t_commit}` with `message: "chore(plan): finalize <phase-or-task>"`

       This `{t_commit}` call is a **separate, plan-repo** git commit — distinct from the source-repo `git commit` of the implementation diff above. Do not conflate the two: one lands your code, the other lands the plan-repo status update.
    3. **Immediately invoke the `rdm-review` skill** against the item. It is the canonical review — the same find → refute → filter → verdict → gate pipeline the autonomous lane runs — so every finalize is actively reviewed, in either mode.
    4. **The review owns the gate.** It persists the status its outcome maps to — `reviewed`, `in-progress` (rework), or `blocked` with a `[code]`-prefixed reason (escalated) — and on `reviewed` **only**, it amends the land-time completion trailer onto the branch commit (again a source-repo `git commit --amend`, with no MCP equivalent).

    **Never hand-type the completion trailer.** Finalize does not write it at all: on the interactive path the review gate writes it, and on the autonomous path `rdm-land` writes it at land time. Both source the exact line from `rdm hook done-line --roadmap <slug> --phase <stem>` (or `--task <slug>`), so the format string has exactly one home. Use the exact roadmap slug / phase stem / task slug from the rdm tools you used earlier — do NOT invent or paraphrase them. The commit stays on the worktree's branch and is left for merge to main (the merge hook flips `reviewed` → `done`).

    **Single pass.** If the review returns `rework`, the item is left `in-progress` and you must surface that to the user with its findings — do not silently loop. Re-run this skill to take another pass.

## Side-work

If you discover bugs or unrelated improvements while working, do not fix them inline — create a tagged task instead so the work is findable later:

Use `rdm_task_create` with `project: {proj_param}, slug: "<slug>", title: "Description", body: "Details.", tags: ["<tag1>", "<tag2>"]`, then land it with `{t_commit}` (`message: "chore(plan): file side-work task <slug>"`).

Use lowercase kebab-case tags and prefer ones already present in the project (check with `rdm_search` `query: "", tags: ["<candidate>"], project: {proj_param}`).
