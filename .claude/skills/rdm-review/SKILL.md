---
name: rdm-review
description: Review implementation of an rdm phase or task
allowed-tools:
  - Read
  - Bash
  - Glob
  - Grep
  - Agent
---

Review the implementation of an rdm phase or task. `$ARGUMENTS` should be `<roadmap-slug> <phase-number>` for a phase, or `--task <task-slug>` for a task.

**IMPORTANT: This is the rdm source repo. Always run `cargo build` first, then use `./target/debug/rdm` — never bare `rdm`.**

## Steps

1. Run `cargo build` to ensure the binary is up to date.
2. **Parse arguments**: determine whether this is a phase review or task review from `$ARGUMENTS`.
   - If the first argument is `--task`, the next argument is a task slug.
   - Otherwise, the first argument is a roadmap slug and the second is a phase number.

3. **Read the acceptance criteria**:
   - For a phase: `./target/debug/rdm phase show <phase-number> --roadmap <slug> --project rdm`
   - For a task: `./target/debug/rdm task show <slug> --project rdm`
   Extract the acceptance criteria, steps, and any other requirements from the body.

4. **Identify the implementation diff**: use `git log --oneline -20` and `git diff` to understand what was recently changed. Identify the commits and files relevant to this phase or task.

5. **Dispatch parallel review agents** using the `Agent` tool. Launch at least two agents concurrently:

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

6. **Collect and consolidate results** from both agents into a single report:
   - List each acceptance criterion with its status (PASS / FAIL / PARTIAL) and evidence
   - List code quality findings grouped by severity (blocking, concern, suggestion)
   - Provide an overall verdict: **PASS**, **PASS WITH CONCERNS**, or **FAIL**

7. **Present the report** to the user in a clear, structured format.

8. **Offer to create rdm tasks** for any actionable issues found:
   ```bash
   ./target/debug/rdm task create <slug> --title "Review finding: description" --body "Details." --no-edit --project rdm
   ```

## Guidelines

- Be objective — evaluate against the stated AC, not personal preferences
- Provide specific evidence (file paths, line numbers) for every finding
- Distinguish between blocking issues (FAIL) and minor concerns (PASS WITH CONCERNS)
- Do not re-implement or fix code — only review and report
- If AC are missing or vague, note this as a finding rather than guessing intent
