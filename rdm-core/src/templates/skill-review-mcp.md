---
name: rdm-review
description: Review implementation of an rdm phase or task
allowed-tools:
  - Read
  - Write
  - Edit
  - Glob
  - Grep
  - Agent
  - {t_phase_show}
  - {t_phase_update}
  - {t_task_show}
  - {t_task_update}
  - {t_task_create}
---

Review the implementation of an rdm phase or task. `$ARGUMENTS` should be `<roadmap-slug> <phase-number>` for a phase, or `--task <task-slug>` for a task.
{principles}
## Steps

1. **Parse arguments**: determine whether this is a phase review or task review from `$ARGUMENTS`.
   - If the first argument is `--task`, the next argument is a task slug.
   - Otherwise, the first argument is a roadmap slug and the second is a phase number.

2. **Read the acceptance criteria**:
   - For a phase: use `rdm_phase_show` with `project: {proj_param}, roadmap: "<slug>", phase: "<phase-number>"`
   - For a task: use `rdm_task_show` with `project: {proj_param}, task: "<slug>"`
   Extract the acceptance criteria, steps, and any other requirements from the body.

3. **Identify the implementation diff**: use `git log --oneline -20` and `git diff` to understand what was recently changed. Identify the commits and files relevant to this phase or task.

4. **Dispatch parallel review agents** using the `Agent` tool. Launch at least two agents concurrently:

   **Agent 1 — AC Compliance Reviewer**:
   - For each acceptance criterion, evaluate whether it is met
   - Provide evidence: file paths, line numbers, test names
   - Rate each criterion: PASS, FAIL, or PARTIAL
   - Note any criteria that are ambiguous or untestable

   **Agent 2 — Code Quality Reviewer**:
   - Check adherence to CLAUDE.md conventions (error handling, doc comments, test coverage, unsafe policy)
   - Review architecture: does the implementation follow the core/cli/server separation?
   - Check for common issues: missing error context, untested edge cases, public API without docs
   - Verify tests exist and cover the key behaviors

5. **Collect and consolidate results** from both agents into a single report:
   - List each acceptance criterion with its status (PASS / FAIL / PARTIAL) and evidence
   - List code quality findings grouped by severity (blocking, concern, suggestion)
   - Provide an overall verdict: **PASS**, **PASS WITH CONCERNS**, or **FAIL**

6. **Categorize each finding by size** and act on it:
   - **Small** — localized, low-risk, no new acceptance criteria (a typo, a missing doc comment, a tightened error message, an extra test). Fix it inline: apply the change with `Edit`/`Write`, run the relevant tests, then fold it into the implementation commit with `git commit --amend --no-edit`.
   - **Large** — new modules, cross-cutting changes, or anything that warrants its own acceptance criterion. Do NOT fix inline. File a task with `rdm_task_create`: `project: {proj_param}, slug: "<slug>", title: "Review finding: description", body: "Details."`

7. **Present the report** to the user in a clear, structured format. For each finding, state how it was handled (fixed-inline / filed-as-task `<slug>`), and give the overall verdict.

8. **Transition the item** by verdict (this skill owns the `needs-review` → `reviewed` gate):
   - **Pass** (clean, or clean after small fixes): set the item to `reviewed`, then amend a `Done:` line into the branch commit so the merge-to-main hook flips it to `done` later.
     - phase: use `rdm_phase_update` with `project: {proj_param}, roadmap: "<slug>", phase: "<phase>", status: "reviewed"`
     - task: use `rdm_task_update` with `project: {proj_param}, task: "<slug>", status: "reviewed"`
     - then `git commit --amend` to add the `Done:` line to the branch commit message: `Done: <roadmap-slug>/<phase-stem>` (phase) or `Done: task/<slug>` (task), using the exact slugs/stems from the rdm tools above. Do NOT set the item to `done` directly — that flip is owned by the merge-to-main hook.
   - **Rework** (FAIL — substantial changes needed): return the item to `in-progress` and write **no** `Done:` line.
     - phase: use `rdm_phase_update` with `project: {proj_param}, roadmap: "<slug>", phase: "<phase>", status: "in-progress"`
     - task: use `rdm_task_update` with `project: {proj_param}, task: "<slug>", status: "in-progress"`

## Guidelines

- Be objective — evaluate against the stated AC, not personal preferences
- Provide specific evidence (file paths, line numbers) for every finding
- Distinguish between blocking issues (FAIL) and minor concerns (PASS WITH CONCERNS)
- The dispatched sub-agents only review and report — they never modify code. The orchestrator (this skill) applies small fixes per the categorization step.
- Never fix large changes inline — file them as tasks. Only small, localized fixes are amended into the implementation commit.
- If AC are missing or vague, note this as a finding rather than guessing intent
