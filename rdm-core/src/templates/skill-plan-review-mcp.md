---
name: rdm-plan-review
description: Review the plan for an rdm roadmap, phase, or task before implementation begins
allowed-tools:
  - Read
  - Glob
  - Grep
  - Agent
  - {t_phase_show}
  - {t_phase_update}
  - {t_task_show}
  - {t_task_update}
  - {t_task_create}
  - {t_roadmap_show}
  - {t_roadmap_update}
  - {t_commit}
---

Review the *plan* of an rdm roadmap, phase, or task — not its implementation. `$ARGUMENTS` should be `<roadmap-slug> [phase-number]` for a phase, `--task <slug>` for a task, `--roadmap <slug>` for a whole roadmap, or `--implementation-plan` for reviewing an in-progress `rdm-do` implementation plan directly.
{principles}
The review runs as a pipeline: **find → refute → filter → verdict → act → gate**. Findings are never surfaced, fixed, or acted on until a *separate* agent has tried to refute them. The agent that finds an issue is never the agent that confirms it.

The specification of that pipeline — which dimensions run, how findings are graded, and what each outcome means — is **generated from the canonical review source** and is shared with `rdm-review`, which reviews the diff after implementation instead of the plan before it. It appears under "Review specification" below. The steps here wire it to the rdm MCP tools.

## Steps

### 1. Setup

1. **Parse `$ARGUMENTS`**:
   - `--task <slug>` — review a task's plan.
   - `--roadmap <slug>` — review the whole roadmap: its own body plus every phase, gated individually.
   - `<roadmap-slug> [phase-number]` — review a single phase. If `phase-number` is omitted, review the roadmap the same as `--roadmap <slug>`.
   - `--implementation-plan` — review an `rdm-do` plan document handed to you directly in context, ahead of implementation. There is no persisted rdm item backing this mode, so it produces an outcome and findings report only — **no tag-gate step** (skip the Gate step entirely for this mode), and it skips the Act step's fix-application half the same way (see the carve-out there).
2. **Read the target artifact** — this also establishes the **target type**, which is the trigger signal for the `unit-of-work` dimension in the Review specification:
   - Phase (target type `phase`): use `{t_phase_show}` with `project: {proj_param}, roadmap: "<slug>", phase: "<phase-number>"` for the body and `tags`.
   - Task (target type `task`): use `{t_task_show}` with `project: {proj_param}, task: "<slug>"` for the body and `tags`.
   - Roadmap (target type `roadmap`): use `{t_roadmap_show}` with `project: {proj_param}, roadmap: "<slug>"` — returns the roadmap body plus every phase's summary (body, tags) in one call; also fetch each phase's full body with `{t_phase_show}` per phase, since each phase is reviewed as a `phase` target in its own right.
   - `--implementation-plan` (target type `implementation-plan`): read the plan text already provided in context; no tool call is needed.

### 2. Find — dispatch the review fleet (parallel)

Dispatch one **read-only** `Agent` per applicable dimension, per **Review specification § Dimensions** below. Run the always-on dimensions unconditionally; add `unit-of-work` only when the target type from step 1 is a phase — including once per phase when reviewing `--roadmap <slug>`. State which dimensions you launched, and why, in the report.

### 3. Refute — per-finding refute pass (parallel)

Dispatch a **fresh** `Agent` per finding, per **Review specification § Refute**. Run these concurrently; the finder is never the refuter. Suggestions may skip refutation (low stakes) but are still subject to the confidence floor.

### 4. Filter, consolidate & decide the outcome

Apply **Review specification § Filter & consolidate**, then **§ Verdict** to reach exactly one outcome: `reviewed`, `rework`, or `escalated`.

For a `--roadmap <slug>` review, consolidate **per phase** as well as for the roadmap body as a whole — one phase's `rework` does not decide the outcome of a phase that came back clean (see the Gate step's per-phase handling).

### 5. Report

Present a single structured report:
- Surviving findings grouped by severity (blocking → concern → suggestion), each with a location, confidence, and recommendation.
- The outcome: **reviewed**, **rework**, or **escalated**, and the one rule that decided it.

### 6. Act — only the orchestrator edits, never a sub-agent

Apply **Review specification § Act**. The dispatched reviewers never apply fixes; only the orchestrator (this skill) does, and only after refutation.

Skip this step's fix-application half entirely in `--implementation-plan` mode — there is no persisted rdm item to write to or file against.

Small fixes are written back as a whole body (bodies are whole-document-authoritative — there is no patch/diff mechanism):
- phase: use `{t_phase_update}` with `project: {proj_param}, roadmap: "<slug>", phase: "<phase-number>", body: "<full updated body>"`
- task: use `{t_task_update}` with `project: {proj_param}, task: "<slug>", body: "<full updated body>"`
- roadmap: use `{t_roadmap_update}` with `project: {proj_param}, roadmap: "<slug>", body: "<full updated body>"`
- land it: call `{t_commit}` with `message: "chore(plan): address plan review finding on <target>"`

Large findings are filed as tasks instead: use `{t_task_create}` with `project: {proj_param}, slug: "<slug>", title: "Plan review finding: description", body: "Details."`, then land it: call `{t_commit}` with `message: "chore(plan): file plan review finding as task"`.

### 7. Gate — clear or leave `needs-plan-review`

Apply **Review specification § Gate**. Skip this step entirely in `--implementation-plan` mode — there is no persisted rdm item to gate; report the outcome and findings only.

On **reviewed**, read the target's current tags via `{t_phase_show}` / `{t_task_show}` / `{t_roadmap_show}`, filter `needs-plan-review` out by exact string match, and write the complete remaining list back — **the `tags` field replaces the whole list**, so always read-then-filter-then-set:

- phase: use `{t_phase_update}` with `project: {proj_param}, roadmap: "<slug>", phase: "<phase-number>", tags: ["<remaining-tag-1>", "<remaining-tag-2>"]` (or `tags: []` when `needs-plan-review` was the only tag present — the `tags` field is a JSON array of strings, never a comma-joined string)
- task: use `{t_task_update}` with `project: {proj_param}, task: "<slug>", tags: ["<remaining-tag-1>", "<remaining-tag-2>"]` (or `tags: []`)
- roadmap: use `{t_roadmap_update}` with `project: {proj_param}, roadmap: "<slug>", tags: ["<remaining-tag-1>", "<remaining-tag-2>"]` (or `tags: []`)
- land it: call `{t_commit}` with `message: "chore(plan): clear needs-plan-review on <target>"`

Under `--roadmap <slug>`, gate each phase **individually** — a phase whose own outcome is `rework` or `escalated` keeps its `needs-plan-review` tag even when every other phase in the roadmap reaches `reviewed`.

On **rework** and **escalated**, do **not** call the update tool with `tags`. `needs-plan-review` is left unchanged in place. State explicitly in the report that the tag was left and enumerate exactly what must change before the next review pass.

## Guidelines

- Be objective, and cite evidence (a location and a quote or paraphrase) for every finding.
- The dispatched sub-agents only review and report — they never edit. The orchestrator (this skill) applies small fixes and files large ones, and only after refutation.
- Never guess intent when the target document is ambiguous or missing — report it as a finding instead.
- Bodies are whole-document-authoritative: always read-modify-write the entire body, never assume a patch/diff mechanism exists.
- Tags replace the whole list: always read the current tags, filter out `needs-plan-review`, and set the complete remaining list (or empty when it was the only tag).
- A surviving `blocking` finding yields `rework` or `escalated`; concerns and suggestions alone never hold the gate closed.

## Review specification

Generated from the canonical review source (`.claude/workflows/lib/review.mjs`)
by `scripts/gen-skill-review.sh --mode plan` — the single home of the
dimensions, severity scale, refute pass, verdict rules, and gate policy shared
with `rdm-review`. Edit the source, not this region.

<!-- rdm:review-spec:begin (generated by scripts/gen-skill-review.sh --mode plan — edit .claude/workflows/lib/review.mjs, not this region) -->

### Dimensions — the adaptive review fleet

Scale the fleet to what the change actually touches. **Always-on** dimensions
run for every review; **triggered** dimensions run only when the change hits
their surface. This keeps a 10-line change cheap while a cross-cutting change
still gets full coverage. Each dimension is reviewed by its own **read-only**
agent — it reviews and reports, it never edits. When in doubt about a trigger,
include the dimension: a spurious agent that finds nothing is cheaper than a
missed defect. State which dimensions you ran, and why, in the report.

**Confidence floor.** Drop any finding whose post-refutation confidence is
below **70**, even when no refuter knocked it down.

**Severity scale** (drives the verdict):

- `blocking` — the work must not advance as-is: a logic error, an unmet
  acceptance criterion, or a mandatory process violation (e.g. a missing
  required changelog entry).
- `concern` — recorded but non-gating; it never by itself holds the work back.
- `suggestion` — minor optional improvement (subject to the confidence floor).

Rank survivors most-severe first, then by confidence descending, then by id.

**Plan review dimensions:**

- **coherence** — *always.* Internal consistency and completeness: are
  the steps and acceptance criteria concrete and actionable? An empty or
  ambiguous plan is itself a `blocking` finding — never guess intent. A
  plan step citing a file or behavior as existing, where it was actually
  introduced by another in-flight (not-yet-landed) roadmap or task, is
  only `blocking` when the target item does **not** carry the
  `depends-unlanded` tag and does not state the dependency explicitly;
  when already annotated, downgrade it to a `concern` (or omit it).
- **architectural-fit** — *always.* Read the project's principles
  (falling back to `CLAUDE.md` / `AGENTS.md` in the project root when no
  principles note is configured — architectural fit must never go
  silently unchecked). Flag any plan step that would violate a stated
  convention or constraint: a violated constraint is what makes a finding
  `blocking`; stylistic preferences alone are not.
- **unit-of-work** — *trigger: the target is a phase.* Skipped for
  tasks, standalone roadmap bodies, and `--implementation-plan`; run once
  per phase under `--roadmap <slug>` (this can fan out to many parallel
  agents on a large roadmap — no hard cap is required, but be mindful of
  the cost). Is the phase independently deliverable and testable —
  neither too large to land safely nor too trivial to warrant its own
  phase?

**Plan target types.** A plan review targets a `roadmap` (its own body,
plus every phase gated individually), a `phase`, a `task`, or an
`implementation-plan` — an `rdm-do` plan document handed over in context
ahead of implementation. `implementation-plan` has **no persisted rdm
item** behind it, so it is report-only: no body edit, no filed task, and
no gate (see § Gate).

### Find — one read-only agent per applicable dimension, in parallel

Each finder agent is told: you are a READ-ONLY reviewer, do not edit any
files; review exactly one dimension; report only findings you can back with
concrete evidence — **one strong finding beats five weak ones**; return an
empty finding list if the dimension is clean. Do not report pure
style/formatting nitpicks unless they violate an explicit project rule.

Each finding is reported as:

```
- id: <short-slug>
  concern: <coherence|architectural-fit|unit-of-work>
  location: <section/heading or phase stem>
  severity: blocking | concern | suggestion
  confidence: 0-100
  what-fails: <the specific problem>
  why: <root cause / which rule or AC it violates>
  recommendation: <concrete fix>
```

### Refute — a FRESH agent per finding, in parallel

For every finding, dispatch a **separate** read-only refuter. The agent that
found an issue is never the agent that confirms it. The refuter starts from
the stance *"this is NOT a real issue unless the code proves otherwise"*,
reads the actual cited location and its surrounding context, and returns
`refuted` (boolean), a corrected `confidence` (0-100), and a rationale.

### Filter & consolidate

- **Drop** any finding a refuter refuted, and any whose post-refutation
  confidence is below the confidence floor (70).
- A refuter that *crashes* is not proof of refutation — keep such a finding as
  un-refuted rather than silently dropping it.
- **Dedup** findings pointing at the same location / same root cause (the
  fleet covers overlapping ground by design).
- **Rank** survivors by severity, then confidence, then id.
- Keep the AC table intact; surviving AC FAIL/PARTIAL items become findings.

### Verdict — one outcome vocabulary: `reviewed` | `rework` | `escalated`

Determine the outcome in this strict order — the first matching rule wins:

1. **escalated** — a surviving blocker that needs a *human decision* rather
   than a code change: the goal, approach, or scope is wrong, the work
   violates a stated architectural constraint, or the acceptance criteria
   themselves are missing, contradictory, or unimplementable as written.
2. **rework** — else if any surviving finding is `blocking`, or the AC table
   contains any FAIL or PARTIAL criterion. The defect is fixable in place; the
   work goes back for another round.
3. **reviewed** — else. Clean, or clean after small fixes. Surviving
   `concern` and `suggestion` findings are recorded and do **not** gate.

Never downgrade a surviving `blocking` finding to "reviewed with concerns" —
a blocker always yields `rework` or `escalated`.

**Plan-stage reading of the three outcomes.**

- `escalated` — the plan needs a **human product decision**: the goal,
  approach, or scope is wrong, or it violates a stated architectural
  constraint that cannot simply be rewritten in place.
- `rework` — the plan document itself needs a fixable rewrite (an ambiguous
  step, a missing prerequisite, an untestable acceptance criterion).
- `reviewed` — clean, or clean with recorded concerns/suggestions.

`rework` and `escalated` both leave the gate **closed**, so this is exactly
the outcome the retired PASS / PASS WITH CONCERNS / REWORK vocabulary
produced: PASS and PASS WITH CONCERNS both collapse to `reviewed` (they
cleared the tag), and REWORK splits into `rework` and `escalated` (both
leave it).

**Plan-stage severity calibration.** `blocking` means the goal, approach, or
scope is wrong, or the plan violates a stated architectural constraint. A
defect in a specific proposed line of code or shell (e.g. an off-by-one in
proposed pseudo-code) is a `concern` that rides along as an implementation
note for the implementing agent — not a gate. An empty or ambiguous plan is
still `blocking`.

### Act — only on verified findings

Report first, then act. Never fix or file an unverified finding.

- **Small** — a localized wording, typo, or missing-detail fix to the plan
  document itself. Apply it directly: the body is whole-document-authoritative,
  so read the current body, apply the change, and write the **entire** modified
  body back — there is no patch/diff mechanism.
- **Large** — a structural concern: a missing prerequisite, scope too big for
  one phase, or a conflicting design decision. Do **NOT** edit the plan
  document for these: file it as a task.

For each finding, state how it was handled (fixed-inline / filed-as-task).

In `--implementation-plan` mode the *act* half is skipped entirely — there is
no persisted rdm item to write to or file against. Findings are still
reported; folding them back into the plan text is left to the caller.

### Gate — clear or leave `needs-plan-review`

The plan review owns the reserved `needs-plan-review` tag. It **never**
persists an rdm status — the item's status is the implementation lane's to
own — and it never writes a land-time completion directive.

| Outcome | `needs-plan-review` | rdm status written |
|---|---|---|
| **reviewed** | cleared | none |
| **rework** | left in place | none |
| **escalated** | left in place | none |

On **reviewed**:

1. Read the target's current tags (the `tags` array is present in every JSON
   summary).
2. Filter `needs-plan-review` out of that array by **exact string match**.
   This is idempotent: a target that already lacks the tag is a safe no-op.
3. Write the **complete remaining list** back. Tags **replace** the whole list
   — there is no remove-one-tag operation — so always read-filter-write, or a
   sibling tag (e.g. the reserved `depends-unlanded`) is silently dropped.
   When `needs-plan-review` was the only tag, write an **empty** list.
4. Land it with a `chore(plan): clear needs-plan-review on <target>` commit.

On **rework** and **escalated**: do **not** touch the tags. `needs-plan-review`
is left unchanged in place. State explicitly in the report that the tag was
left, and enumerate exactly what must change before the next review pass. On
`escalated`, say what human decision is required; prefix a recorded reason
with `[plan]` so it is attributable to this gate.

Scope of the gate by target type:

- **`--roadmap <slug>`** — gate each phase **individually**, and the roadmap
  body separately. One phase's `rework` must not hold the tag on phases that
  reached `reviewed`, and the roadmap body's own outcome is independent of any
  phase's.
- **`--implementation-plan`** — **no gate at all.** There is no persisted rdm
  item, so there is no tag to clear and nothing to mutate; report the outcome
  and findings only.

### Guidelines

- Be objective — evaluate against the stated acceptance criteria, not personal
  preferences.
- Provide specific evidence (file:line, test name) for every finding.
- **No finding is surfaced, fixed, or filed until a separate refuter agent has
  failed to refute it.** The finder never grades its own work.
- Filter hard: drop refuted findings and anything below 70 confidence. One
  strong finding beats five weak ones.
- The dispatched sub-agents only review and report — they never modify code.
  The orchestrator applies small fixes, and only after refutation.
- Never fix large changes inline — file them as tasks.
- If acceptance criteria are missing or vague, report it as a finding rather
  than guessing intent.

<!-- rdm:review-spec:end -->
