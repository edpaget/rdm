---
name: rdm-review
description: Review implementation of an rdm phase or task
allowed-tools:
  - Read
  - Bash
  - Write
  - Edit
  - Glob
  - Grep
  - Agent
---

Review the implementation of an rdm phase or task. `$ARGUMENTS` should be `<roadmap-slug> <phase-number>` for a phase, or `--task <task-slug>` for a task.
{principles}
The review runs as a pipeline: **find → verify → filter → report → act → gate**. Findings are never surfaced, fixed, or acted on until a *separate* agent has tried to refute them. The agent that finds an issue is never the agent that confirms it.

## Steps

### 1. Setup

1. **Parse arguments**: determine whether this is a phase review or task review from `$ARGUMENTS`.
   - If the first argument is `--task`, the next argument is a task slug.
   - Otherwise, the first argument is a roadmap slug and the second is a phase number.
2. **Read the acceptance criteria**:
   - For a phase: `rdm phase show <phase-number> --roadmap <slug> {proj_flag}`
   - For a task: `rdm task show <slug> {proj_flag}`
   Extract the acceptance criteria, steps, and any other requirements from the body.
3. **Identify the implementation diff**: use `git log --oneline -20` and `git diff` to understand what was recently changed. Identify the commits and files relevant to this phase or task. Note the diff size, which modules it touches, and whether it changes public API, `unsafe` constructs, dependencies, or user-facing behavior — these drive which conditional agents you launch in step 2.

### 2. Find — dispatch an adaptive review fleet (parallel)

Scale the fleet to what the diff actually touches. Always run the **base** agents; add **conditional** agents only when the diff hits their surface. This keeps a 10-line phase cheap while a cross-cutting change still gets full coverage. Each agent is **read-only** — it reviews and reports, it never edits.

**Base (always run):**

- **AC Compliance** — for each acceptance criterion, rate PASS / FAIL / PARTIAL with evidence (file:line, test name). Flag any criterion that is ambiguous or untestable.
- **Correctness & error handling** — logic bugs, edge cases, race conditions, error paths. Check error handling against the project's conventions (CLAUDE.md / AGENTS.md); user-facing errors must be actionable.

**Conditional (add when the trigger is present):**

- **Tests** — *trigger: diff adds/changes any non-trivial logic, or adds no test files.* Do tests exist and cover the key behaviors and edge cases? Was a test-first discipline followed (test describes desired behavior)? Are there untested branches?
- **Public API docs** — *trigger: diff changes public API surface.* Are public items documented per the project's conventions (CLAUDE.md / AGENTS.md)? Are error, panic, and safety conditions called out where the project requires it?
- **Architecture** — *trigger: diff touches more than one module/layer, or moves logic between layers.* Does logic live where the project's architecture says it should, with thin layers on top? No duplicated logic across interfaces?
- **Language safety & conventions** — *trigger: diff contains unsafe/low-level constructs, new dependencies, or non-trivial new modules.* Are unsafe or risky constructs justified per the project's conventions? Commit message, scope, and lint/format discipline followed?
- **Changelog & docs** — *trigger: the change is user-facing (CLI commands, API endpoints, MCP tools, config options, observable behavior).* If the project requires a changelog/docs update for user-facing changes (CLAUDE.md / AGENTS.md), is it present in the same change? Flag a missing or non-user-facing entry as a finding.

Decide triggers from the `git diff` and file list in step 1. When in doubt about a trigger, include the agent — a spurious agent that finds nothing is cheaper than a missed concern. State which agents you launched and why in the report.

**Each agent returns structured findings**, one block per finding:

```
- id: <short-slug>
  concern: <ac|correctness|tests|api-docs|architecture|safety|changelog>
  file: <path>:<line>
  severity: blocking | concern | suggestion
  confidence: 0-100
  what-fails: <the specific problem>
  why: <root cause / which rule or AC it violates>
  impact: <what breaks or degrades>
  recommendation: <concrete fix>
```

The AC agent additionally returns the per-criterion PASS/FAIL/PARTIAL table. Calibrate for signal: **one strong finding beats five weak ones.** Do not report pure style/formatting nitpicks unless they violate an explicit project rule.

### 3. Verify — per-finding refute pass (parallel)

For every finding with `severity` of blocking or concern (and every AC FAIL/PARTIAL verdict), dispatch a **fresh** `Agent` whose job is to **refute** it. The refute agent:

- Starts from the stance "this is NOT a real issue unless the code proves otherwise."
- Reads the actual code at the cited location and surrounding context.
- Returns `verdict: confirmed | refuted | uncertain`, a corrected `confidence` (0-100), and one line of evidence.

Run these concurrently. The finder is never the verifier. Suggestions may skip verification (low stakes) but are still subject to the confidence filter.

### 4. Filter & consolidate

- **Drop** any finding the refute pass marked `refuted`, or whose post-verification confidence is **below 70**.
- **Dedup** findings that point at the same file:line / same root cause (the fleet covers overlapping ground by design).
- **Rank** survivors by severity, then confidence.
- Keep the AC table intact (it is the contract); surviving AC FAIL/PARTIAL items become findings.

### 5. Report

Present a single structured report:
- The AC table: each criterion with PASS / FAIL / PARTIAL and evidence.
- Surviving findings grouped by severity (blocking → concern → suggestion), each with file:line, confidence, and recommendation.
- An overall verdict: **PASS**, **PASS WITH CONCERNS**, or **FAIL**.

### 6. Act — only on verified findings

Report first, then act. Never fix or file an unverified finding.

- **Small** — localized, low-risk, no new acceptance criteria (a typo, a missing doc comment, a tightened error message, an extra test). Fix it inline: apply with `Edit`/`Write`, run the relevant tests, then fold it into the implementation commit with `git commit --amend --no-edit`.
- **Large** — new modules, cross-cutting changes, or anything that warrants its own acceptance criterion. Do NOT fix inline. File a task:
  ```bash
  rdm task create <slug> --title "Review finding: description" --body "Details." --tags <tag1>,<tag2> --no-edit {proj_flag}
  ```

For each finding, state how it was handled (fixed-inline / filed-as-task `<slug>`).

### 7. Gate — transition by verdict

This skill owns the `needs-review` → `reviewed` gate.

- **Pass** (clean, or clean after small fixes): set the item to `reviewed`, then amend a `Done:` line into the branch commit so the merge-to-main hook flips it to `done` later.
  ```bash
  rdm phase update <phase> --status reviewed --no-edit --roadmap <slug> {proj_flag}
  # or: rdm task update <slug> --status reviewed --no-edit {proj_flag}
  git commit --amend   # add the Done: line to the branch commit message
  ```
  The `Done:` line is `Done: <roadmap-slug>/<phase-stem>` (phase) or `Done: task/<slug>` (task), using the exact slugs/stems from the rdm commands above. Do NOT set the item to `done` directly — that flip is owned by the merge-to-main hook.
- **Rework** (FAIL — substantial changes needed): return the item to `in-progress` and write **no** `Done:` line.
  ```bash
  rdm phase update <phase> --status in-progress --no-edit --roadmap <slug> {proj_flag}
  # or: rdm task update <slug> --status in-progress --no-edit {proj_flag}
  ```

## Guidelines

- Be objective — evaluate against the stated AC, not personal preferences.
- Provide specific evidence (file:line, test name) for every finding.
- **No finding is surfaced, fixed, or filed until a separate refute agent has confirmed it.** The finder never grades its own work.
- Filter hard: drop refuted findings and anything below 70 confidence. One strong finding beats five weak ones.
- Distinguish blocking issues (FAIL) from minor concerns (PASS WITH CONCERNS).
- The dispatched sub-agents only review and report — they never modify code. The orchestrator (this skill) applies small fixes, and only after verification.
- Never fix large changes inline — file them as tasks.
- If AC are missing or vague, note this as a finding rather than guessing intent.
