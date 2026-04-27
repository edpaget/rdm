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
2. **List tasks**: `./target/debug/rdm task list --project rdm` to see open and in-progress tasks. Add `--tag <name>` to narrow by tag (e.g. `--tag bug`).
3. **Show details**: if a task slug was provided in `$ARGUMENTS`, run `./target/debug/rdm task show <slug> --project rdm`. Otherwise, present the task list and ask the user which task to work on.
4. **Mark in-progress**: `./target/debug/rdm task update <slug> --status in-progress --no-edit --project rdm`
5. **Enter plan mode**: use the `EnterPlanMode` tool to switch into planning mode.
6. **Create an implementation plan** using the planning tool. The plan should:
   - Break the task into concrete implementation steps based on the task description
   - Include a final step: "Review changes with user and commit"
7. **Wait for user approval**: the user will review the plan and either accept or request changes. Do not proceed until the plan is accepted.
8. **Exit plan mode**: use the `ExitPlanMode` tool to switch back to execution mode.
9. **Execute the plan**: implement each step, following the plan.
10. **Review with user**: present a summary of the changes and ask the user to confirm they are ready to finalize.
11. **Finalize**: on user acceptance:
    - Commit the implementation changes with a `Done: task/<slug>` line in the commit message so the post-merge hook records the commit SHA and marks the task done automatically.
      **Use the exact task slug from the rdm commands you ran earlier — do NOT invent or paraphrase it.**
    - If the task is also part of a roadmap phase, include a `Done: <roadmap-slug>/<phase-stem>` line as well (using exact slugs/stems from rdm)

When creating a side-work task (during planning or implementation), attach tags so the task is findable later:

```bash
./target/debug/rdm task create <slug> --title "Description" --body "Details." --tags <tag1>,<tag2> --no-edit --project rdm
```

Use lowercase kebab-case tags and prefer ones already present in the project (check with `./target/debug/rdm search "" --tag <candidate> --project rdm`).
