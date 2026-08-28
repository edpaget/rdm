---
name: roadmap
description: Create an rdm roadmap with phases for a topic
allowed-tools:
  - Read
  - Bash
  - Glob
  - Grep
  - AskUserQuestion
---

Create an rdm roadmap with phases for the topic described in `$ARGUMENTS`.

## Steps

1. **Explore the codebase** to understand the current state relevant to `$ARGUMENTS`. Read key files, search for related code, and build context.
2. **Interview the operator — human-in-the-loop only.** Before designing phases, run a short, bounded interview so the plan is shaped by the operator's actual intent instead of being reconciled against it afterward:
   - Ask at most 3-5 questions, one at a time, selected by impact x uncertainty — only where the answer would change how the work is broken into phases.
   - Each question is closed-form: 2-4 mutually exclusive options with a recommended default, or a short answer with a suggested value, so the operator can reply in one token. Use the question-asking tool available in your environment (e.g. `AskUserQuestion`, granted in this skill's `allowed-tools`).
   - Cover, in priority order: the goal as an observable end state; what is explicitly NOT wanted; and one operator-testable "done looks like" signal. Stop as soon as all three are unambiguous — don't ask a fourth question just to reach the cap.
   - Terminate early the moment the operator signals they're finished ("done", "that's it", "no more").
   - Record every answer **verbatim** under `Interview.` in a `## Intent` section — never a paraphrase.
   - An unresolved high-impact question goes under `Open`, never guessed at.
   - If the operator does not engage, or this skill is running with no human in the loop, write `(not captured)` as the whole `## Intent` section rather than inventing intent.
3. **Design phases** that break the work into independently deliverable increments. Each phase should produce a working, testable result.
4. **Create the roadmap**, including the `## Intent` section (or the literal `(not captured)`) captured above in the body: `rdm roadmap create <slug> --title "Title" --body "Summary.

## Intent
<captured section>" --tags <tag1>,<tag2> --no-edit --project <PROJECT>`

   If the roadmap already exists (e.g. you're re-running this skill against one created earlier), read its current body, splice in the `## Intent` section, and write the whole body back instead — bodies are whole-document-authoritative, there is no patch/diff mechanism: `rdm roadmap update <slug> --body "<full updated body>" --no-edit --project <PROJECT>`.
5. **Create each phase** with context, steps, and acceptance criteria in the body:
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
6. **Land the batch**: `rdm commit -m "feat(plan): add <roadmap> roadmap"` — one commit for the roadmap and all its phases.
7. **Verify** the roadmap looks correct: `rdm roadmap show <slug> --project <PROJECT>`

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
