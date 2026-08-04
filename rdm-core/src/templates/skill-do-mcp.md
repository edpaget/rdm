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
  - Workflow
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
- **`--auto`** (non-interactive): skip the approval and review gates and proceed autonomously. This splits by flow:
  - **phase flow** (`--auto <roadmap-slug> [phase-number]`): after marking the phase in-progress and entering its worktree (steps 1-5 below, unchanged), route straight into the `rdm-wf-dispatch-phase` Workflow instead of re-running the prose plan/implement/review steps — see `## Auto phase dispatch` below.
  - **task flow** (`--auto --task <slug>`): after marking the task in-progress and entering its per-task worktree (steps 1-5 below, unchanged), route straight into the `rdm-wf-dispatch-phase` Workflow with `{ task: <slug> }` instead of re-running the prose plan/implement/review steps — see `## Auto task dispatch` below.

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
5. **For `--auto` (either flow), skip steps 5-10 below and jump straight to `## Auto phase dispatch` / `## Auto task dispatch` — steps 5-10 are the interactive prose path only.** Enter plan mode with the `EnterPlanMode` tool, then **create an implementation plan** _(interactive only; `--auto` skips the approval gate and proceeds to implement)_. The plan should:
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

## Auto phase dispatch (--auto, phase flow only)

1. Pass `alreadyInProgress: true` in the workflow args, because step 3 already ran `{t_phase_update}` with `status: "in-progress"` and it succeeded — that write happens strictly before the workflow is invoked, so the phase is observably `in-progress` earlier than the workflow's own stamp would make it, and the workflow can skip a dedicated subagent for it. The flag is **optional**: omit it and the workflow stamps the phase itself, exactly as before. Never pass it for a `--plan-only` invocation.

   `phaseMeta` is **deliberately not hoisted here**: the workflow applies an all-or-nothing guard requiring the body plus all five resolved per-step model ids, and there is no MCP model-resolve tool — so a partial payload would be rejected outright and the in-workflow fetch remains the path on MCP. Do not invent model ids.
2. Invoke the `rdm-wf-dispatch-phase` Workflow (`.claude/workflows/rdm-wf-dispatch-phase.js`, provisioned automatically by `rdm agent-config claude --skills`) via the `Workflow` tool with `{ roadmap: <slug>, phase: <stem>, alreadyInProgress: true, rdmBin, project: {proj_param} }`; block for its returned OUTCOME. `rdmBin` is the rdm executable the workflow's own Bash agents invoke — still meaningful on MCP, because the workflow shells out even though this shim does not. It is **optional**, defaulting to a plain `rdm` on `PATH` when omitted — resolve an explicit `--rdm-bin <path>` first, then `$RDM_BIN` if set, then let the default apply, using the first that resolves and never probing the filesystem to choose. An explicitly passed value always wins verbatim. `project` is optional and applies only to project-scoped commands.
3. Interpret the OUTCOME and persist status. The OUTCOME carries the canonical gate policy **as data** — `outcome.status` (the status the canonical review mapped this outcome to), `outcome.reason` (already carrying its `[code]`/`[plan]` gate tag), and `outcome.writesCompletion` — so read those fields rather than restating the map. dispatch-phase's code-review stage IS the canonical review, so the work is already reviewed by the time you see the OUTCOME.
   - `reviewed` → persist `outcome.status`: use `{t_phase_update}` with `project: {proj_param}, roadmap: "<slug>", phase: "<phase>", status: "reviewed"` then call `{t_commit}` with `message: "chore(plan): finalize <phase>"`
   - `rework` → this lane is single-pass, so park it in the escalation queue with `outcome.reason` instead of leaving it merely in-progress: use `{t_phase_update}` with `status: "blocked", reason: "[code] <outcome.summary>"` then call `{t_commit}` with `message: "chore(plan): park <phase>"`
   - `escalated` → same, with the plan-gate tag: use `{t_phase_update}` with `status: "blocked", reason: "[plan] <outcome.summary>"` then call `{t_commit}` with `message: "chore(plan): park <phase>"`
   - Do NOT add a `Done:` line here. `outcome.writesCompletion` is `true` only on `reviewed`, and it is `rdm-land` that reads it and synthesizes the trailer via `rdm hook done-line` at land time — no manual rebase, and no pre-step before `rdm-land`.
4. Return the OUTCOME JSON verbatim as the final message.

This section applies only to `--auto` + the phase flow. Interactive `rdm-do` (either flow) is unaffected and keeps the steps above unchanged.

## Auto task dispatch (--auto, task flow only)

The task-flow twin of the phase dispatch above. A task belongs to no roadmap, carries no difficulty/model tier, and lives in its own `task/<slug>` worktree.

1. Pass `alreadyInProgress: true` in the workflow args, because step 3 already ran `{t_task_update}` with `status: "in-progress"` and it succeeded. As on the phase path, `taskMeta` is **not** hoisted on MCP — there is no model-resolve tool, and the workflow's all-or-nothing guard would reject a payload missing the five model ids.
2. Invoke the `rdm-wf-dispatch-phase` Workflow (`.claude/workflows/rdm-wf-dispatch-phase.js`) via the `Workflow` tool with `{ task: <slug>, alreadyInProgress: true, rdmBin, project: {proj_param} }`; block for its returned OUTCOME. The task-mode OUTCOME is keyed by `task` (the slug), not `roadmap`/`phase`. `rdmBin` is the rdm executable the workflow's own Bash agents invoke — still meaningful on MCP, because the workflow shells out even though this shim does not. It is **optional**, defaulting to a plain `rdm` on `PATH` when omitted — resolve an explicit `--rdm-bin <path>` first, then `$RDM_BIN` if set, then let the default apply, using the first that resolves and never probing the filesystem to choose. An explicitly passed value always wins verbatim. `project` is optional and applies only to project-scoped commands.
3. Interpret the OUTCOME and persist status (a true mirror of the phase contract). The task-mode OUTCOME carries the same canonical `outcome.status` / `outcome.reason` / `outcome.writesCompletion` fields — read them rather than restating the map. `escalated` maps to the `blocked` **task** status; it is never downgraded to `in-progress`.
   - `reviewed` -> persist `outcome.status`: use `{t_task_update}` with `project: {proj_param}, task: "<slug>", status: "reviewed"` then call `{t_commit}` with `message: "chore(plan): finalize <slug>"`
   - `rework` -> single-pass park with `outcome.reason`: use `{t_task_update}` with `status: "blocked", reason: "[code] <outcome.summary>"` then call `{t_commit}` with `message: "chore(plan): park <slug>"`
   - `escalated` -> use `{t_task_update}` with `status: "blocked", reason: "[plan] <outcome.summary>"` then call `{t_commit}` with `message: "chore(plan): park <slug>"`
   - Do NOT add a `Done:` line here. On `reviewed` the OUTCOME's `writesCompletion` is `true` and `rdm-land` is the land-time writer — it synthesizes `Done: task/<slug>` via `rdm hook done-line` before the rebase.
4. Return the OUTCOME JSON verbatim as the final message.

Tasks always dispatch at the fixed `medium` tier (there is no task estimate), so the `large`-tier gate tightening never applies. Task bodies often carry no formal acceptance criteria; the plan/review gates tolerate their absence.

**Single-item scope:** unlike the prose `rdm-autopilot` skill's advance/park loop, this single-item entry point parks on the first `rework`/`escalated` OUTCOME rather than re-dispatching against a rework budget — re-run `rdm-do --auto <roadmap> <phase>` (or `--auto --task <slug>`) by hand to retry. Skipping the outer rework-retry here is intentional, not an oversight.

## Side-work

If you discover bugs or unrelated improvements while working, do not fix them inline — create a tagged task instead so the work is findable later:

Use `rdm_task_create` with `project: {proj_param}, slug: "<slug>", title: "Description", body: "Details.", tags: ["<tag1>", "<tag2>"]`, then land it with `{t_commit}` (`message: "chore(plan): file side-work task <slug>"`).

Use lowercase kebab-case tags and prefer ones already present in the project (check with `rdm_search` `query: "", tags: ["<candidate>"], project: {proj_param}`).
