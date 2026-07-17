---
name: rdm-do
description: Implement an rdm roadmap phase or work on an rdm task — plan, execute, then finalize into needs-review for rdm-review
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
---

Implement a roadmap phase or work on a task. One shared flow: find the target → mark in-progress → plan → execute → review with the user → finalize into `needs-review`.

**IMPORTANT: This is the rdm source repo. Always run `cargo build` first, then use `./target/debug/rdm` — never bare `rdm`. If you modify any rdm source, `cargo build` again before running it.**

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
6. **Enter plan mode** with the `EnterPlanMode` tool, then **create an implementation plan** _(interactive only; `--auto` skips the approval gate and proceeds to implement)_. The plan should:
   - Break the phase/task into concrete implementation steps based on its description and acceptance criteria.
   - Include a final step: "Review changes with user and finalize".
7. **Review the implementation plan** _(both modes)_: run the `rdm-plan-review` skill with `--implementation-plan` against the plan drafted in the previous step, covering coherence and architectural fit.
   - **interactive**: surface the verdict and findings alongside the plan, before the approval gate.
   - **`--auto`**: never wait on the verdict — fold every surviving **blocking** finding back into the plan text before continuing to the execute step. If a blocking finding can't be resolved by editing the plan (genuine ambiguity or an architectural decision), don't drop it silently: file it via the Side-work convention (`./target/debug/rdm task create ... --tags plan-review`).
8. **Wait for user approval** _(interactive only)_: do not proceed until the plan is accepted. Then use `ExitPlanMode` to switch back to execution mode.
9. **Execute the plan**: implement each step, following the plan and any acceptance criteria.
10. **Review with user** _(interactive only; `--auto` finalizes without waiting)_: present a summary of the changes and ask the user to confirm they are ready to finalize.
11. **Finalize:** on user acceptance, commit the implementation changes — a plain `git commit` of the code diff in the **source repo**, on the worktree's branch — then transition the item to `needs-review`:
    - phase: `./target/debug/rdm phase update <phase> --status needs-review --no-edit --roadmap <slug> --project rdm`
    - task: `./target/debug/rdm task update <slug> --status needs-review --no-edit --project rdm`
    - land the plan-repo status change: `./target/debug/rdm commit -m "chore(plan): finalize <phase-or-task>"`

    This `rdm commit` is a **separate, plan-repo** git commit — distinct from the source-repo `git commit` of the implementation diff above. Do not conflate the two: one lands your code, the other lands the plan-repo status update.

    **Do NOT emit a `Done:` line in the commit message YET** — `rdm-review` adds it on a passing review as the final step. This is a deferred two-stage `Done:` protocol, not a contradiction: finalize defers the `Done:` line, review completes it. An item sitting in `needs-review` is the sentinel that signals a review is pending. Use the exact roadmap slug / phase stem / task slug from the rdm commands you ran earlier — do NOT invent or paraphrase them. The commit stays on the worktree's branch, which is left for merge to main (review takes it `needs-review` → `reviewed`, and the merge hook flips it to `done`).

## Side-work

If you discover bugs or unrelated improvements while working, do not fix them inline — create a tagged task instead so the work is findable later:

```bash
./target/debug/rdm task create <slug> --title "Description" --body "Details." --tags <tag1>,<tag2> --no-edit --project rdm
./target/debug/rdm commit -m "chore(plan): file side-work task <slug>"  # land the batch
```

Use lowercase kebab-case tags and prefer ones already present in the project (check with `./target/debug/rdm search "" --tag <candidate> --project rdm`).
