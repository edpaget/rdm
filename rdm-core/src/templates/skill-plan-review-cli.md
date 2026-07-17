---
name: rdm-plan-review
description: Review the plan for an rdm roadmap, phase, or task before implementation begins
allowed-tools:
  - Read
  - Bash
  - Glob
  - Grep
  - Agent
---

Review the *plan* of an rdm roadmap, phase, or task — not its implementation. `$ARGUMENTS` should be `<roadmap-slug> [phase-number]` for a phase, `--task <slug>` for a task, `--roadmap <slug>` for a whole roadmap, or `--implementation-plan` for reviewing an in-progress `rdm-do` implementation plan directly.
{principles}
The review runs as a pipeline: **find → consolidate → categorize & act → gate**. Three read-only sub-agents review the plan in parallel, one per dimension; the orchestrator (this skill) — never a sub-agent — consolidates their reports into a single verdict and applies any fixes.

## Steps

### 1. Setup

1. **Parse `$ARGUMENTS`**:
   - `--task <slug>` — review a task's plan.
   - `--roadmap <slug>` — review the whole roadmap: its own body plus every phase, gated individually.
   - `<roadmap-slug> [phase-number]` — review a single phase. If `phase-number` is omitted, review the roadmap the same as `--roadmap <slug>`.
   - `--implementation-plan` — review an `rdm-do` plan document handed to you directly in context, ahead of implementation. There is no persisted rdm item backing this mode, so it produces a verdict and findings report only — **no tag-gate step** (skip step 5 entirely for this mode), and skips step 4's fix-application half the same way (see the carve-out there).
2. **Read the target artifact**:
   - Phase: `rdm phase show <phase-number> --roadmap <slug> {proj_flag}` for the body, and `rdm phase show <phase-number> --roadmap <slug> --format json {proj_flag}` for its `tags`.
   - Task: `rdm task show <slug> {proj_flag}` for the body, and `rdm task show <slug> --format json {proj_flag}` for its `tags`.
   - Roadmap: `rdm roadmap show <slug> --format json {proj_flag}` returns the roadmap body plus every phase's summary (body, tags) in one call; also fetch each phase's full body with `rdm phase show <n> --roadmap <slug> {proj_flag}` for the unit-of-work reviewer.
   - `--implementation-plan`: read the plan text already provided in context; no `rdm` command is needed.

### 2. Find — dispatch parallel read-only sub-agents, one per dimension

Dispatch the applicable reviewers **in parallel**. Each is **read-only** — it reviews and reports, it never edits.

- **Coherence reviewer** — runs for every target type (phase, task, roadmap, implementation plan). Checks internal consistency, completeness, and whether steps and acceptance criteria are concrete and actionable. An empty or ambiguous plan document is itself a REWORK-worthy finding — never guess intent on the orchestrator's behalf.
- **Architectural-fit reviewer** — runs for every target type. Reads the project's principles file via the mechanism above, and — since architectural fit must not silently go unchecked just because `--principles-file` was never configured — falls back to reading `CLAUDE.md` or `AGENTS.md` directly in the project root when no principles note is present. Flags any plan step that would violate a stated convention.
- **Unit-of-work reviewer** — *phases only.* Skipped for `--task`, `--implementation-plan`, and for a standalone roadmap-body review; run once per phase when reviewing `--roadmap <slug>` (this can fan out to many parallel agents on a large roadmap — no hard cap is required, but be mindful of the cost). Judges whether the phase is independently deliverable and testable — neither too large to land safely nor too trivial to warrant its own phase.

Each dispatched agent returns one finding block per issue:

```
- id: <short-slug>
  concern: <coherence|architecture|unit-of-work>
  location: <section/heading or phase stem>
  severity: blocking | concern | suggestion
  confidence: 0-100
  what-fails: <the specific problem>
  why: <root cause / which rule, AC, or principle it violates>
  impact: <what breaks or degrades if this ships unaddressed>
  recommendation: <concrete fix>
```

### 3. Consolidate — the orchestrator determines a single verdict

The **orchestrator** (this skill itself, not a sub-agent) merges the dimension reports into exactly one verdict. Determine it in this strict order — the first matching rule wins:

1. **REWORK** — if any dimension raised a `blocking` finding.
2. **PASS WITH CONCERNS** — else if any dimension raised a surviving `concern` or `suggestion` finding.
3. **PASS** — else (no blocking findings, no concerns, no suggestions).

For a `--roadmap <slug>` review, consolidate **per phase** as well as for the roadmap body as a whole — a single phase's blocking finding produces a REWORK verdict for that phase without necessarily failing phases that passed independently (see the Gate step's per-phase handling).

### 4. Categorize & act — only the orchestrator edits, never a sub-agent

Report first, then act. The dispatched reviewers never apply fixes; only the orchestrator does, and only after consolidation.

**`--implementation-plan` mode**: findings are still reported using the finding-block format from step 2, but the *act* half below is skipped entirely — there is no persisted rdm item to write to or file against in this mode. Resolving or folding surviving findings back into the plan text is left to the caller (e.g. `rdm-do`'s own `--auto`-mode step).

For each surviving finding, classify it:

- **Small** — a localized wording, typo, or missing-detail fix to the plan document itself. Apply it directly: read the current body, apply the change, and pass the **entire modified body** back — `--body` is whole-document-authoritative, so there is no patch/diff mechanism to rely on:
  ```bash
  rdm phase update <phase-number> --roadmap <slug> --body "<full updated body>" --no-edit {proj_flag}
  # or: rdm task update <slug> --body "<full updated body>" --no-edit {proj_flag}
  # or: rdm roadmap update <slug> --body "<full updated body>" --no-edit {proj_flag}
  rdm commit -m "chore(plan): address plan review finding on <target>"
  ```
- **Large** — a structural concern: a missing prerequisite, scope that's too big for one phase, or a conflicting design decision. Do **not** edit the plan document for these. File it as a task instead:
  ```bash
  rdm task create <slug> --title "Plan review finding: description" --body "Details." --tags plan-review --no-edit {proj_flag}
  rdm commit -m "chore(plan): file plan review finding as task"
  ```

### 5. Gate — clear or leave `needs-plan-review` by verdict

Skip this step entirely in `--implementation-plan` mode — there is no persisted rdm item to gate; report the verdict and findings only.

- **PASS or PASS WITH CONCERNS** (concerns and suggestions alone never block — the same grouping `rdm-review` uses for its own pass/pass-with-concerns gate):
  1. Read the target's current tags: `rdm phase show <n> --roadmap <slug> --format json {proj_flag}` / `rdm task show <slug> --format json {proj_flag}` / `rdm roadmap show <slug> --format json {proj_flag}` — the `tags` array is present in every JSON summary.
  2. Remove `needs-plan-review` from that array by exact string match. This is the CLI-porcelain equivalent of `rdm_core::tags::clear_plan_review_tag` — there is no `rdm tags clear` command, so replicate its effect here: idempotent (a target that already lacks the tag is a safe no-op) and collapsing to no tags when it was the only one present.
  3. Set the filtered list back — remember **`--tags` replaces the whole list**, it never removes one tag in place, so always read-then-filter-then-set the complete remaining list:
     ```bash
     rdm phase update <phase-number> --roadmap <slug> --tags <comma-joined-remaining-tags> --no-edit {proj_flag}
     # or, when needs-plan-review was the only tag present:
     rdm phase update <phase-number> --roadmap <slug> --tags "" --no-edit {proj_flag}
     ```
  4. `rdm commit -m "chore(plan): clear needs-plan-review on <target>"`.
  - **`--roadmap <slug>` mode**: gate each phase **individually** — a phase whose own consolidated verdict is REWORK keeps its `needs-plan-review` tag even when every other phase in the roadmap passes.
- **REWORK**: do **not** call `update --tags`. `needs-plan-review` is left unchanged in place. State explicitly in the report that the tag was left and enumerate exactly what must change before the next review pass.

## Guidelines

- Be objective, and cite evidence (a location and a quote or paraphrase) for every finding.
- The dispatched sub-agents only review and report — they never edit. The orchestrator (this skill) applies small fixes and files large ones, and only after consolidating the verdict.
- Never guess intent when the target document is ambiguous or missing — report it as a finding instead.
- `--body` is whole-document-authoritative: always read-modify-write the entire body, never assume a patch/diff mechanism exists.
- `--tags` replaces the whole list: always read the current tags, filter out `needs-plan-review`, and set the complete remaining list (or `--tags ""` when it was the only tag).
- Treat PASS and PASS WITH CONCERNS the same for gating purposes; only a surviving `blocking` finding forces REWORK.
