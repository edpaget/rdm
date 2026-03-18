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
---

Work on rdm tasks. `$ARGUMENTS` is an optional task slug.

**IMPORTANT: This is the rdm source repo. Always run `cargo build` first, then use `./target/debug/rdm` — never bare `rdm`.**

## Steps

1. Run `cargo build` to ensure the binary is up to date.
2. **List tasks**: `./target/debug/rdm task list --project rdm` to see open and in-progress tasks.
3. **Show details**: if a task slug was provided in `$ARGUMENTS`, run `./target/debug/rdm task show <slug> --project rdm`. Otherwise, present the task list and ask the user which task to work on.
4. **Mark in-progress**: `./target/debug/rdm task update <slug> --status in-progress --no-edit --project rdm`
5. **Implement** the work described in the task.
6. **Mark done**: `./target/debug/rdm task update <slug> --status done --no-edit --project rdm`
