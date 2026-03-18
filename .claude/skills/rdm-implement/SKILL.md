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
---

Implement a phase from an rdm roadmap. `$ARGUMENTS` should be `<roadmap-slug> [phase-number]`.

**IMPORTANT: This is the rdm source repo. Always run `cargo build` first, then use `./target/debug/rdm` — never bare `rdm`.**

## Steps

1. Run `cargo build` to ensure the binary is up to date.
2. **Parse arguments**: extract the roadmap slug and optional phase number from `$ARGUMENTS`.
3. **Find the phase**: if no phase number was given, run `./target/debug/rdm phase list --roadmap <slug> --project rdm` and pick the first `not-started` or `in-progress` phase.
4. **Read the phase**: `./target/debug/rdm phase show <phase> --roadmap <slug> --project rdm` to get full context, steps, and acceptance criteria.
5. **Mark in-progress**: `./target/debug/rdm phase update <phase> --status in-progress --no-edit --roadmap <slug> --project rdm`
6. **Plan the implementation** and present your approach to the user for approval before writing code.
7. **Execute the plan**: implement the work described in the phase, following the steps and acceptance criteria.
8. **Mark done**: `./target/debug/rdm phase update <phase> --status done --no-edit --roadmap <slug> --project rdm`
9. **Handle side-work**: if you discover bugs or unrelated improvements, create tasks instead of fixing them inline:
   ```bash
   ./target/debug/rdm task create <slug> --title "Description" --body "Details." --no-edit --project rdm
   ```
