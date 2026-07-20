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
  - EnterWorktree
  - Agent
  - Workflow
---

Implement a roadmap phase or work on a task. One shared flow: find the target → mark in-progress → plan → execute → review with the user → finalize through the canonical code review.

**IMPORTANT: This is the rdm source repo. Always run `cargo build` first, then use `./target/debug/rdm` — never bare `rdm`. If you modify any rdm source, `cargo build` again before running it.**

## Run modes

`$ARGUMENTS` may include `--auto` to select the run mode:

- **interactive** (default): plan → wait for approval → implement → review with the user → finalize. The approval and review gates pause for human input.
- **`--auto`** (non-interactive): skip the approval and review gates and proceed autonomously. This splits by flow:
  - **phase flow** (`--auto <roadmap-slug> [phase-number]`): after marking the phase in-progress and entering its worktree (steps 1-5 below, unchanged), route straight into the `dispatch-phase` Workflow instead of re-running the prose plan/implement/review steps — see `## Auto phase dispatch` below.
  - **task flow** (`--auto --task <slug>`): after marking the task in-progress and entering its per-task worktree (steps 1-5 below, unchanged), route straight into the `dispatch-phase` Workflow with `{ task: <slug> }` instead of re-running the prose plan/implement/review steps — see `## Auto task dispatch` below.

For unattended Claude Code runs (where no human is present to approve permission prompts), launch with `--permission-mode auto` (or `bypassPermissions` in a sandbox) so worktree edits and bash commands don't block on prompts.

**Dogfood-only note:** the `--auto` phase-flow → `dispatch-phase` Workflow wiring above is a local-only edit to this file (`.claude/skills/rdm-do/SKILL.md`), not propagated to the distributed `rdm-core/src/templates/skill-do-cli.md` / `skill-do-mcp.md` templates this roadmap ships — those templates stay prose-only for now, to be updated in a future distribution follow-up. The divergence is intentional and recorded in the `workflow-orchestration` roadmap body (phase 4).

## Argument forms

`$ARGUMENTS` selects the flow:

- `<roadmap-slug> [phase-number]` → **phase flow**.
- `--task <slug>` → **task flow**.
- empty → **discovery**: list in-progress phases and open tasks, then ask the user which to work on:
  ```bash
  ./target/debug/rdm search "" --status in-progress --type phase --project rdm
  ./target/debug/rdm task list --project rdm
  ```

## Steps

1. Run `cargo build` to ensure the binary is up to date.
2. **Parse `$ARGUMENTS`** and pick the flow (phase, task, or discovery — see above).
3. **Find and show the target:**
   - **phase**: if no phase number was given, run `./target/debug/rdm phase list --roadmap <slug> --project rdm` and pick the first `not-started` or `in-progress` phase. Then `./target/debug/rdm phase show <phase> --roadmap <slug> --project rdm` for full context, steps, and acceptance criteria.
   - **task**: if a slug was provided, `./target/debug/rdm task show <slug> --project rdm`. Otherwise present the task list and ask the user which to work on.
4. **Mark in-progress:**
   - phase: `./target/debug/rdm phase update <phase> --status in-progress --no-edit --roadmap <slug> --project rdm`
   - task: `./target/debug/rdm task update <slug> --status in-progress --no-edit --project rdm`
   - land the status change: `./target/debug/rdm commit -m "chore(plan): start <phase-or-task>"`
5. **Get into the roadmap's worktree (one worktree per roadmap, work in place).** Each roadmap gets a *single* worktree on branch `roadmap/<slug>`, and **every phase of that roadmap is implemented in place in it**. Entry happens at most **once** — on the first phase, from the main checkout — so the session never re-enters or nests. Run `./target/debug/rdm worktree current --format json` and compare the current worktree's roadmap to the target `<slug>`. (`worktree current` reports `item` = the roadmap slug for a roadmap worktree, or `<roadmap>/<stem>` for a legacy per-phase worktree — compare the roadmap portion against `<slug>`.)
   - **Match** (current worktree's roadmap == `<slug>`): you are already in the target roadmap's worktree → **work in place**. Skip `worktree add` and entry. Run `cargo build` here. This is the common case for every phase after the first.
   - **None** (output is `null` — main checkout or not in a worktree): ensure the roadmap worktree exists with `./target/debug/rdm worktree add <slug> --project rdm` (idempotent — reuses it if present), take the `path` it prints, then enter it **once** with `EnterWorktree({path})`. `EnterWorktree` is a **one-time entry convenience**, not required for correctness — the model never re-enters, so a plain `cd`/launch into `path` works just as well on any host.
   - **Mismatch** (current worktree's roadmap is a *different* roadmap): interactive → **ask** the user whether to switch to the target roadmap's worktree; `--auto` → switch to it — run `./target/debug/rdm worktree add <slug> --project rdm`, then **relaunch / `cd` in the printed `path`** to enter it, and note that you moved to the target roadmap's worktree. `EnterWorktree` is *not* usable from inside another worktree — its `path` form is rejected unless the target is under `.claude/worktrees/`, which rdm worktrees are not; relaunch is the correct entry here (`EnterWorktree` applies only to the one-time entry from the **main checkout** in the *None* case above).

   **Tasks keep their own per-task worktree.** For the task flow, run `./target/debug/rdm worktree add task/<slug> --project rdm` and enter the printed `path` the same way (one-time `EnterWorktree` convenience, or `cd`/launch).

   After entering, run `cargo build` in the worktree and use **that worktree's** `./target/debug/rdm` for all later rdm commands — this exercises your changes where you made them. Do the rest of the work in this worktree.
6. **For `--auto` (either flow), skip steps 6-11 below and jump straight to `## Auto phase dispatch` / `## Auto task dispatch` — steps 6-11 are the interactive prose path only.** Enter plan mode with the `EnterPlanMode` tool, then **create an implementation plan** _(interactive only; `--auto` skips the approval gate and proceeds to implement)_. The plan should:
   - Break the phase/task into concrete implementation steps based on its description and acceptance criteria.
   - Include a final step: "Review changes with user and finalize".
7. **Review the implementation plan** _(both modes)_: run the `rdm-plan-review` skill with `--implementation-plan` against the plan drafted in the previous step, covering coherence and architectural fit.
   - **interactive**: surface the verdict and findings alongside the plan, before the approval gate.
   - **`--auto`**: never wait on the verdict — fold every surviving **blocking** finding back into the plan text before continuing to the execute step. If a blocking finding can't be resolved by editing the plan (genuine ambiguity or an architectural decision), don't drop it silently: file it via the Side-work convention (`./target/debug/rdm task create ... --tags plan-review`).
8. **Wait for user approval** _(interactive only)_: do not proceed until the plan is accepted. Then use `ExitPlanMode` to switch back to execution mode.
9. **Execute the plan**: implement each step, following the plan and any acceptance criteria.
10. **Review with user** _(interactive only; `--auto` finalizes without waiting)_: present a summary of the changes and ask the user to confirm they are ready to finalize.
11. **Finalize — commit, then actively run the canonical code review.** The human gate at step 10 decides **whether to finalize**, not **whether a review happens**: once accepted, this step *drives* the review rather than parking the item in `needs-review` for a Stop hook to pick up. Review is therefore active in **both** lanes — interactive here, and inside `dispatch-phase` under `--auto`.

    1. **Commit the implementation diff** — a plain `git commit` of the code diff in the **source repo**, on the worktree's branch.
    2. **Mark the item `needs-review`** as a transient marker so the review has a well-defined starting state, and land that plan-repo change:
       - phase: `./target/debug/rdm phase update <phase> --status needs-review --no-edit --roadmap <slug> --project rdm`
       - task: `./target/debug/rdm task update <slug> --status needs-review --no-edit --project rdm`
       - land the plan-repo status change: `./target/debug/rdm commit -m "chore(plan): finalize <phase-or-task>"`

       This `rdm commit` is a **separate, plan-repo** git commit — distinct from the source-repo `git commit` of the implementation diff above. Do not conflate the two: one lands your code, the other lands the plan-repo status update.
    3. **Immediately invoke the `rdm-review` skill** against the item. `rdm-review` is the generated projection of `.claude/workflows/lib/review.mjs` — the *same* canonical review the autonomous lane stamps into `dispatch-phase`. There is no second review implementation.
    4. **The review owns the gate.** It persists the status the canonical mapping assigns — `reviewed`, `in-progress` (rework), or `blocked` with a `[code]`-prefixed reason (escalated) — and, on `reviewed` **only**, amends the land-time completion trailer onto the branch commit.

    **Never hand-type the completion trailer.** Finalize does not write it at all: on this interactive path the review gate writes it; on the autonomous path `rdm-land` writes it at land time. Both source the exact line from `./target/debug/rdm hook done-line --roadmap <slug> --phase <stem>` (or `--task <slug>`), so the format string has exactly one home. Use the exact roadmap slug / phase stem / task slug from the rdm commands you ran earlier — do NOT invent or paraphrase them. The commit stays on the worktree's branch, which is left for merge to main (the merge hook flips `reviewed` → `done`).

    **Single pass.** If the review returns `rework`, the item is left `in-progress` and you must surface that to the user with its findings — there is no automatic rework loop here. Re-run this skill to take another pass.

    Because finalize now drives the item **out of** `needs-review` before the session stops, nothing is ever left parked in `needs-review` — the once-passive needs-review Stop hook / Pi `agent_end` extension that used to catch a dropped finalize has been retired as redundant.

## Auto phase dispatch (--auto, phase flow only)

1. Invoke the `dispatch-phase` Workflow (`.claude/workflows/dispatch-phase.js`) via the `Workflow` tool with `{ roadmap: <slug>, phase: <stem> }`; block for its returned OUTCOME.
2. Interpret the OUTCOME and persist status. The OUTCOME now carries the canonical gate policy **as data** — `outcome.status` (the status the canonical review mapped this outcome to), `outcome.reason` (already carrying its `[code]`/`[plan]` gate tag), and `outcome.writesCompletion` — so read those fields rather than restating the map. dispatch-phase's code-review stage IS the canonical review, so the work is already reviewed by the time you see the OUTCOME.
   - `reviewed` → persist `outcome.status`: `./target/debug/rdm phase update <phase> --status reviewed --no-edit --roadmap <slug> --project rdm` then `./target/debug/rdm commit -m "chore(plan): finalize <phase>"`
   - `rework` → this lane is single-pass, so park it in the escalation queue with `outcome.reason` instead of leaving it merely in-progress: `./target/debug/rdm phase update <phase> --status blocked --reason "[code] <outcome.summary>" --no-edit --roadmap <slug> --project rdm` then `./target/debug/rdm commit -m "chore(plan): park <phase>"`
   - `escalated` → same, with the plan-gate tag: `./target/debug/rdm phase update <phase> --status blocked --reason "[plan] <outcome.summary>" --no-edit --roadmap <slug> --project rdm` then `./target/debug/rdm commit -m "chore(plan): park <phase>"`
   - Do NOT add a `Done:` line here. `outcome.writesCompletion` is `true` only on `reviewed`, and it is `rdm-land` that reads it and synthesizes the trailer via `rdm hook done-line` at land time — no manual rebase, and no pre-step before `rdm-land`.
3. Return the OUTCOME JSON verbatim as the final message.

This section applies only to `--auto` + the phase flow. Interactive `rdm-do` (either flow) is unaffected and keeps the steps above unchanged.

## Auto task dispatch (--auto, task flow only)

The task-flow twin of the phase dispatch above. A task belongs to no roadmap, carries no difficulty/model tier, and lives in its own `task/<slug>` worktree.

1. Invoke the `dispatch-phase` Workflow (`.claude/workflows/dispatch-phase.js`) via the `Workflow` tool with `{ task: <slug> }`; block for its returned OUTCOME. The task-mode OUTCOME is keyed by `task` (the slug), not `roadmap`/`phase`.
2. Interpret the OUTCOME and persist status (a true mirror of the phase contract). The task-mode OUTCOME carries the same canonical `outcome.status` / `outcome.reason` / `outcome.writesCompletion` fields — read them rather than restating the map. `escalated` maps to the `blocked` **task** status; it is never downgraded to `in-progress`.
   - `reviewed` -> persist `outcome.status`: `./target/debug/rdm task update <slug> --status reviewed --no-edit --project rdm` then `./target/debug/rdm commit -m "chore(plan): finalize <slug>"`
   - `rework` -> single-pass park with `outcome.reason`: `./target/debug/rdm task update <slug> --status blocked --reason "[code] <outcome.summary>" --no-edit --project rdm` then `./target/debug/rdm commit -m "chore(plan): park <slug>"`
   - `escalated` -> `./target/debug/rdm task update <slug> --status blocked --reason "[plan] <outcome.summary>" --no-edit --project rdm` then `./target/debug/rdm commit -m "chore(plan): park <slug>"`
   - Do NOT add a `Done:` line here. On `reviewed` the OUTCOME's `writesCompletion` is `true` and `rdm-land` is the land-time writer — it synthesizes `Done: task/<slug>` via `rdm hook done-line` before the rebase.
3. Return the OUTCOME JSON verbatim as the final message.

Tasks always dispatch at the fixed `medium` tier (there is no task estimate), so the `large`-tier gate tightening never applies. Task bodies often carry no formal acceptance criteria; the plan/review gates tolerate their absence.

**Single-item scope:** as with the phase path, this parks on the first `rework`/`escalated` OUTCOME rather than re-dispatching against a rework budget — re-run `rdm-do --auto --task <slug>` by hand to retry.

**Single-item scope:** unlike the `autopilot` workflow's advance/park loop, this single-item entry point parks on the first `rework`/`escalated` OUTCOME rather than re-dispatching against a rework budget — re-run `rdm-do --auto <roadmap> <phase>` by hand to retry. Skipping the outer rework-retry here is intentional, not an oversight.

## Side-work

If you discover bugs or unrelated improvements while working, do not fix them inline — create a tagged task instead so the work is findable later:

```bash
./target/debug/rdm task create <slug> --title "Description" --body "Details." --tags <tag1>,<tag2> --no-edit --project rdm
./target/debug/rdm commit -m "chore(plan): file side-work task <slug>"  # land the batch
```

Use lowercase kebab-case tags and prefer ones already present in the project (check with `./target/debug/rdm search "" --tag <candidate> --project rdm`).
