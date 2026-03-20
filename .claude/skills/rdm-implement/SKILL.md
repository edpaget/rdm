---
name: rdm-implement
description: Implement the next phase of an rdm roadmap
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

Implement a phase from an rdm roadmap. `$ARGUMENTS` should be `<roadmap-slug> [phase-number]`.

**IMPORTANT: This is the rdm source repo. Always run `cargo build` first, then use `./target/debug/rdm` — never bare `rdm`.**

## Steps

1. Run `cargo build` to ensure the binary is up to date.
2. **Parse arguments**: extract the roadmap slug and optional phase number from `$ARGUMENTS`.
3. **Find the phase**: if no phase number was given, run `./target/debug/rdm phase list --roadmap <slug> --project rdm` and pick the first `not-started` or `in-progress` phase.
4. **Read the phase**: `./target/debug/rdm phase show <phase> --roadmap <slug> --project rdm` to get full context, steps, and acceptance criteria.
5. **Mark in-progress**: `./target/debug/rdm phase update <phase> --status in-progress --no-edit --roadmap <slug> --project rdm`
6. **Enter plan mode**: use the `EnterPlanMode` tool to switch into planning mode.
7. **Create an implementation plan** using the planning tool. The plan should:
   - Break the phase into concrete implementation steps based on the phase description and acceptance criteria
   - Include a final step: "Review changes with user and commit"
8. **Wait for user approval**: the user will review the plan and either accept or request changes. Do not proceed until the plan is accepted.
9. **Exit plan mode**: use the `ExitPlanMode` tool to switch back to execution mode.
10. **Execute the plan**: implement each step, following the plan and the phase's acceptance criteria.
11. **Review with user**: present a summary of the changes and ask the user to confirm they are ready to finalize.
12. **Finalize**: on user acceptance, commit the implementation changes with a `Done:` line in the commit message — the post-merge hook will mark the phase done and record the commit SHA.
    **Use the exact roadmap slug and phase stem from the rdm commands you ran earlier — do NOT invent or paraphrase them:**
      ```
      Done: <roadmap-slug>/<phase-stem>
      ```
13. **Handle side-work**: if you discover bugs or unrelated improvements, create tasks instead of fixing them inline:
    ```bash
    ./target/debug/rdm task create <slug> --title "Description" --body "Details." --no-edit --project rdm
    ```
