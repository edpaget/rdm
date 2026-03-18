---
name: rdm-roadmap
description: Create an rdm roadmap with phases for a topic
allowed-tools:
  - Read
  - Bash
  - Glob
  - Grep
---

Create an rdm roadmap with phases for the topic described in `$ARGUMENTS`.

**IMPORTANT: This is the rdm source repo. Always run `cargo build` first, then use `./target/debug/rdm` — never bare `rdm`.**

## Steps

1. Run `cargo build` to ensure the binary is up to date.
2. **Explore the codebase** to understand the current state relevant to `$ARGUMENTS`. Read key files, search for related code, and build context.
3. **Design phases** that break the work into independently deliverable increments. Each phase should produce a working, testable result.
4. **Create the roadmap**: `./target/debug/rdm roadmap create <slug> --title "Title" --body "Summary." --no-edit --project rdm`
5. **Create each phase** with context, steps, and acceptance criteria in the body:
   ```bash
   ./target/debug/rdm phase create <slug> --title "Phase title" --number <n> --no-edit --roadmap <roadmap-slug> --project rdm <<'EOF'
   ## Context
   Why this phase exists and what it builds on.

   ## Steps
   1. First step
   2. Second step

   ## Acceptance Criteria
   - [ ] Criterion one
   - [ ] Criterion two
   EOF
   ```
6. **Verify** the roadmap looks correct: `./target/debug/rdm roadmap show <slug> --project rdm`

## Guidelines

- Aim for 2–6 phases per roadmap
- Each phase should be independently deliverable and testable
- Include Context, Steps, and Acceptance Criteria in every phase body
- Order phases so each builds on the previous one
- Use clear, descriptive slugs (e.g., `add-caching`, `migrate-auth`)
