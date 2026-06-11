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
---

Implement a roadmap phase or work on a task. One shared flow: find the target → mark in-progress → plan → execute → review with the user → finalize into `needs-review`.
{principles}
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
4. **Enter plan mode** with the `EnterPlanMode` tool, then **create an implementation plan**. The plan should:
   - Break the phase/task into concrete implementation steps based on its description and acceptance criteria.
   - Include a final step: "Review changes with user and finalize".
5. **Wait for user approval**: do not proceed until the plan is accepted. Then use `ExitPlanMode` to switch back to execution mode.
6. **Execute the plan**: implement each step, following the plan and any acceptance criteria.
7. **Review with user**: present a summary of the changes and ask the user to confirm they are ready to finalize.
8. **Finalize:** on user acceptance, commit the implementation changes, then transition the item to `needs-review`:
   - phase: `rdm phase update <phase> --status needs-review --no-edit --roadmap <slug> {proj_flag}`
   - task: `rdm task update <slug> --status needs-review --no-edit {proj_flag}`

   **Do NOT emit a `Done:` line in the commit message.** The `rdm-review` skill produces the `Done:` line when review passes; an item sitting in `needs-review` is the sentinel that signals a review is pending. Use the exact roadmap slug / phase stem / task slug from the rdm commands you ran earlier — do NOT invent or paraphrase them.

## Side-work

If you discover bugs or unrelated improvements while working, do not fix them inline — create a tagged task instead so the work is findable later:

```bash
rdm task create <slug> --title "Description" --body "Details." --tags <tag1>,<tag2> --no-edit {proj_flag}
```

Use lowercase kebab-case tags and prefer ones already present in the project (check with `rdm search "" --tag <candidate> {proj_flag}`).
