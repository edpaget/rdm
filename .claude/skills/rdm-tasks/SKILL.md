---
name: rdm-tasks
description: Work on rdm tasks
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

Work on rdm tasks. `$ARGUMENTS` is an optional task slug.

**IMPORTANT: This is the rdm source repo. Always run `cargo build` first, then use `./target/debug/rdm` — never bare `rdm`.**

## Steps

1. Run `cargo build` to ensure the binary is up to date.
2. **List tasks**: `./target/debug/rdm task list --project rdm` to see open and in-progress tasks.
3. **Show details**: if a task slug was provided in `$ARGUMENTS`, run `./target/debug/rdm task show <slug> --project rdm`. Otherwise, present the task list and ask the user which task to work on.
4. **Mark in-progress**: `./target/debug/rdm task update <slug> --status in-progress --no-edit --project rdm`
5. **Enter plan mode**: use the `EnterPlanMode` tool to switch into planning mode.
6. **Create an implementation plan** using the planning tool. The plan should:
   - Break the task into concrete implementation steps based on the task description
   - Include a final step: "Review changes with user, commit, and mark task done"
7. **Wait for user approval**: the user will review the plan and either accept or request changes. Do not proceed until the plan is accepted.
8. **Exit plan mode**: use the `ExitPlanMode` tool to switch back to execution mode.
9. **Execute the plan**: implement each step, following the plan.
10. **Review with user**: present a summary of the changes and ask the user to confirm they are ready to finalize.
11. **Finalize**: on user acceptance:
    - Commit the implementation changes
    - Mark the task done: `./target/debug/rdm task update <slug> --status done --no-edit --project rdm`
