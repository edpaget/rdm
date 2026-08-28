---
name: rdm-roadmap
description: Create an rdm roadmap with phases for a topic
allowed-tools:
  - Read
  - Glob
  - Grep
  - AskUserQuestion
  - {t_roadmap_create}
  - {t_phase_create}
  - {t_roadmap_show}
  - {t_roadmap_update}
  - {t_commit}
---

Create an rdm roadmap with phases for the topic described in `$ARGUMENTS`.
{principles}
## Steps

1. **Explore the codebase** to understand the current state relevant to `$ARGUMENTS`. Read key files, search for related code, and build context.
2. **Interview the operator — human-in-the-loop only.** Before designing phases, run a short, bounded interview so the plan is shaped by the operator's actual intent instead of being reconciled against it afterward:
   - Ask at most 3-5 questions, one at a time, selected by impact x uncertainty — only where the answer would change how the work is broken into phases.
   - Each question is closed-form: 2-4 mutually exclusive options with a recommended default, or a short answer with a suggested value, so the operator can reply in one token. Use the question-asking tool available in your environment (e.g. `AskUserQuestion`, granted in this skill's `allowed-tools`).
   - Cover, in priority order: the goal as an observable end state; what is explicitly NOT wanted; and one operator-testable "done looks like" signal. Stop as soon as all three are unambiguous — don't ask a fourth question just to reach the cap.
   - Terminate early the moment the operator signals they're finished ("done", "that's it", "no more").
   - Record every answer **verbatim** under `Interview.` in a `## Intent` section — never a paraphrase.
   - Structure the `## Intent` section using this canonical grammar — the labels are literal, filled in verbatim:
     ```markdown
     ## Intent

     **Goal.** <the outcome wanted, as an observable end state — not the mechanism>

     **Non-goals.**
     - <explicitly out of scope>

     **Done looks like.**
     - <WHEN <situation> THEN <observable outcome>>

     **Interview.** (captured YYYY-MM-DD)
     - Q: <question asked> → A: <operator's answer, verbatim>
     ```
     `Non-goals`, `Interview`, and an optional `Open` list may be absent. `Goal` and `Done looks like` are what make a section count as captured rather than present-but-empty.
   - An unresolved high-impact question goes under `Open`, never guessed at.
   - If the operator does not engage, or this skill is running with no human in the loop, write `(not captured)` as the whole `## Intent` section rather than inventing intent.
3. **Design phases** that break the work into independently deliverable increments. Each phase should produce a working, testable result.
4. **Create the roadmap**, including the `## Intent` section (or the literal `(not captured)`) captured above in the body: use `rdm_roadmap_create` with `project: {proj_param}, slug: "<slug>", title: "Title", body: "Summary.\n\n## Intent\n<captured section>", tags: ["<tag1>", "<tag2>"]`

   If the roadmap already exists (e.g. you're re-running this skill against one created earlier), read its current body, splice in the `## Intent` section, and write the whole body back instead — bodies are whole-document-authoritative, there is no patch/diff mechanism: use `{t_roadmap_update}` with `project: {proj_param}, roadmap: "<slug>", body: "<full updated body>"`.
5. **Create each phase** with context, steps, and acceptance criteria in the body:
   Use `rdm_phase_create` with `project: {proj_param}, roadmap: "<roadmap-slug>", slug: "<slug>", title: "Phase title", number: <n>, body: "<markdown body>", tags: ["<tag>"]`

   The body should include:
   ```
   ## Context
   Why this phase exists and what it builds on.

   ## Steps
   1. First step
   2. Second step

   ## Acceptance Criteria
   - [ ] Criterion one
   - [ ] Criterion two
   ```

   Pass a bare slug like `hook-commit-bug` for `slug:` — rdm builds the final stem as `phase-<number>-<slug>`. Do **not** include `phase-N-` in `slug:` or you'll get a doubled prefix like `phase-1-phase-1-hook-commit-bug`.
6. **Land the batch**: call `{t_commit}` with `message: "feat(plan): add <roadmap> roadmap"` — one commit for the roadmap and all its phases.
7. **Verify** the roadmap looks correct: use `rdm_roadmap_show` with `project: {proj_param}, roadmap: "<slug>"`

## Guidelines

- Aim for 2–6 phases per roadmap
- Each phase should be independently deliverable and testable
- Include Context, Steps, and Acceptance Criteria in every phase body
- Order phases so each builds on the previous one
- Use clear, descriptive slugs (e.g., `add-caching`, `migrate-auth`)
- Tag the roadmap and phases so related work is findable. Use lowercase
  kebab-case (`auth`, `tech-debt`); prefer existing tags — check with
  `rdm_search` `query: "", tags: ["<candidate>"], project: {proj_param}`
  before inventing a new one.
- If `plan_review` is enabled, every roadmap and phase created above already
  carries a `needs-plan-review` tag — leave it in place, don't strip it by
  hand. It's cleared only by manually running the `rdm-plan-review` skill
  once the plan passes review — there is no automated reprompt.
