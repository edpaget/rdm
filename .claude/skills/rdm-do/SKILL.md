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
5. **Get into the item's isolated worktree.** `<item>` is the same ref form used elsewhere (`<roadmap>/<phase-stem>` for a phase, `task/<slug>` for a task). A plain `cd` into a sibling worktree does **not** persist across tool calls in Claude Code (cwd resets each call), and the auto-review Stop hook runs from the launch dir — so you must move the session durably with `EnterWorktree`, not `cd`. Run `./target/debug/rdm worktree current --format json` and branch on its `item`:
   - **Match** (`item` == the target `<item>`): you are already in the right worktree. Reuse the current dir — skip `worktree add` and `EnterWorktree`. Run `cargo build` here.
   - **None** (output is `null` — you are in the main checkout or not in a worktree): run `./target/debug/rdm worktree add <item> --project rdm`, take the `path` it prints, then move the session in with `EnterWorktree({path})`.
   - **Mismatch** (`item` is a *different* item — you are in another item's worktree): interactive → **ask** the user whether to reuse the current worktree or create and enter the target's worktree; `--auto` → run `worktree add <item>`, `EnterWorktree({path})`, and note that you moved to the target's worktree.

   After entering, run `cargo build` in the worktree and use **that worktree's** `./target/debug/rdm` for all later rdm commands — this exercises your changes where you made them. Do the rest of the work in this worktree.
6. **Enter plan mode** with the `EnterPlanMode` tool, then **create an implementation plan** _(interactive only; `--auto` skips the approval gate and proceeds to implement)_. The plan should:
   - Break the phase/task into concrete implementation steps based on its description and acceptance criteria.
   - Include a final step: "Review changes with user and finalize".
7. **Wait for user approval** _(interactive only)_: do not proceed until the plan is accepted. Then use `ExitPlanMode` to switch back to execution mode.
8. **Execute the plan**: implement each step, following the plan and any acceptance criteria.
9. **Review with user** _(interactive only; `--auto` finalizes without waiting)_: present a summary of the changes and ask the user to confirm they are ready to finalize.
10. **Finalize:** on user acceptance, commit the implementation changes, then transition the item to `needs-review`:
    - phase: `./target/debug/rdm phase update <phase> --status needs-review --no-edit --roadmap <slug> --project rdm`
    - task: `./target/debug/rdm task update <slug> --status needs-review --no-edit --project rdm`

    **Do NOT emit a `Done:` line in the commit message.** The `rdm-review` skill produces the `Done:` line when review passes; an item sitting in `needs-review` is the sentinel that signals a review is pending. Use the exact roadmap slug / phase stem / task slug from the rdm commands you ran earlier — do NOT invent or paraphrase them. The commit stays on the worktree's branch, which is left for merge to main (review takes it `needs-review` → `reviewed`, and the merge hook flips it to `done`).

## Side-work

If you discover bugs or unrelated improvements while working, do not fix them inline — create a tagged task instead so the work is findable later:

```bash
./target/debug/rdm task create <slug> --title "Description" --body "Details." --tags <tag1>,<tag2> --no-edit --project rdm
```

Use lowercase kebab-case tags and prefer ones already present in the project (check with `./target/debug/rdm search "" --tag <candidate> --project rdm`).
