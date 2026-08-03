---
name: roadmap
description: Create an rdm roadmap with phases for a topic
allowed-tools:
  - Read
  - Bash
  - Glob
  - Grep
---

Create an rdm roadmap with phases for the topic described in `$ARGUMENTS`.

## Steps

1. **Explore the codebase** to understand the current state relevant to `$ARGUMENTS`. Read key files, search for related code, and build context.
2. **Design phases** that break the work into independently deliverable increments. Each phase should produce a working, testable result.
3. **Create the roadmap**: `rdm roadmap create <slug> --title "Title" --body "Summary." --tags <tag1>,<tag2> --no-edit --project <PROJECT>`
4. **Create each phase** with context, steps, and acceptance criteria in the body:
   ```bash
   rdm phase create <slug> --title "Phase title" --number <n> --tags <tag> --no-edit --roadmap <roadmap-slug> --project <PROJECT> <<'EOF'
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

   Pass a bare slug like `hook-commit-bug` — rdm prepends `phase-<number>-` automatically. Do **not** include `phase-N-` in the slug; you'll get a doubled prefix like `phase-1-phase-1-hook-commit-bug`.
5. **Land the batch**: `rdm commit -m "feat(plan): add <roadmap> roadmap"` — one commit for the roadmap and all its phases.
6. **Verify** the roadmap looks correct: `rdm roadmap show <slug> --project <PROJECT>`

## Guidelines

- Aim for 2–6 phases per roadmap
- Each phase should be independently deliverable and testable
- Include Context, Steps, and Acceptance Criteria in every phase body
- Order phases so each builds on the previous one
- Use clear, descriptive slugs (e.g., `add-caching`, `migrate-auth`)
- Tag the roadmap and phases so related work is findable. Use lowercase
  kebab-case (`auth`, `tech-debt`); prefer existing tags — check with
  `rdm search "" --tag <candidate> --project <PROJECT>` before inventing a new one.
- If `plan_review` is enabled, every roadmap and phase created above already
  carries a `needs-plan-review` tag — leave it in place, don't strip it by
  hand. It's cleared only by manually running the `plan-review` skill
  once the plan passes review — there is no automated reprompt.
