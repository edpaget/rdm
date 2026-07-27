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

Review the *plan* of an rdm roadmap, phase, or task — not its implementation. This skill is a thin shim over the **`plan-review` Workflow** (`.claude/workflows/plan-review.js`), which runs the whole pipeline end to end. Invoke that workflow and report its result; the domain notes below exist so a human reader understands what it does and can drive it interactively when the workflow is unavailable.

## Invoke the workflow

Pass `$ARGUMENTS` straight through to the `plan-review` Workflow. It accepts the same four target forms:

- `--task <slug>` — review a task's plan.
- `--roadmap <slug>` — review the whole roadmap: its own body plus **every phase, gated independently**.
- `<roadmap-slug> [phase-number]` — a single phase when the phase arg is present; with no phase arg it behaves exactly like `--roadmap <slug>`.
- `--implementation-plan` — review an `rdm-do` plan document handed over in context, ahead of implementation. There is **no persisted rdm item** behind this mode, so it is report-only (see the carve-out below).

The workflow runs the shared `find → refute → filter → verdict → act → gate` pipeline (`buildReviewPipeline('plan')`) and returns a per-unit outcome (`reviewed` | `rework` | `escalated`) with its findings.

### Gather the target payload yourself — do not let a subagent transcribe it

You are already a running agent with the repo in context; the workflow is not, so anything it has to look up costs it a whole dedicated mechanical subagent — and, for the target artifact specifically, that subagent has **twice corrupted real plan data in production** (runs `wf_e3402021-0af` and `wf_f4be8027-dbb`, recorded in full on task `fix-plan-review-gate-tag-clobber`). Both corrupt returns were *schema-valid*: one transposed the roadmap's real body and tags into `phases[0]` and packed three words lifted from its own prompt into `tags`, losing five of six phases; the other returned `tags: ["plan-target"]` — a phrase from the prompt's own "Return a PLAN_TARGET object". The gate then faithfully wrote that junk over the target's real tags. `agent(..., { schema })` cannot catch this, because the schema constrains shape and never content.

Run the reads yourself and pass the parsed JSON through the workflow `args`. Every one of these is **optional** — the workflow falls back to its in-workflow fetch agent for anything you omit or get wrong — but supplying them is what removes the transcription step entirely:

- **`fetched`** — the target artifact, **verbatim**:
  - `--task <slug>` → `./target/debug/rdm task show <slug> --project rdm --format json`; pass `{ body, tags }` copied straight from that JSON.
  - single phase → `./target/debug/rdm phase show <phase> --roadmap <slug> --project rdm --format json`; pass `{ body, tags }`.
  - `--roadmap <slug>` → one `./target/debug/rdm roadmap show <slug> --project rdm --format json`, **plus one `./target/debug/rdm phase show <stem> --roadmap <slug> --project rdm --format json` per phase**; assemble `{ body, tags, phases: [{ stem, body, tags }, …] }` with one entry per real phase.
  - **Never summarize, paraphrase, or describe what you did.** `body` is the document's own text and `tags` is the exact array the binary printed — not a description of the fetch, not words from this prompt. If you cannot read the target, omit `fetched` entirely and let the workflow's fetch agent run; do not pass a placeholder.
- **`wontFixedTexts`** — the array of prior wont-fix finding texts from `./target/debug/rdm search "" --type review --project rdm` (or omit it).
- **`mechanicalModel`** — the id printed by `./target/debug/rdm model resolve mechanical`, verbatim (or omit it).

`fetched` is read from the structured `args` object only — never parsed out of the `$ARGUMENTS` flag string.

## What the workflow does (domain intent)

The pipeline runs `find → refute → filter → verdict → act → gate`; no finding is surfaced, fixed, or acted on until a *separate* refuter agent has failed to refute it. Its full specification — which dimensions run, how findings are graded, what each outcome means — is **generated from the canonical review source** (shared with `rdm-review`, which reviews the diff after implementation) and appears under "Review specification" below.

Key domain behaviors the workflow implements, worth knowing when reading its output:

- **Target type drives `unit-of-work`.** The `unit-of-work` dimension is scoped to **phase** units only — it is dropped from task, roadmap-body, and `--implementation-plan` units (the workflow strips it post-pipeline via `stripNonPhaseUnitOfWork`).
- **Per-phase independent gating.** Under `--roadmap <slug>` the roadmap body and every phase are reviewed and gated **individually**: one phase's `rework` never holds the tag on a sibling that reached `reviewed`.
- **The gate manages a tag, not a status.** Plan review owns the reserved `needs-plan-review` tag. On `reviewed` it clears the tag by **read-filter-write** (`filterPlanReviewTag` preserves siblings like `depends-unlanded`, since `--tags` replaces the whole list); on `rework`/`escalated` it leaves the tag in place. It **never** writes an rdm status and never writes a land-time completion directive.
- **`--implementation-plan` is report-only.** No persisted item, so the workflow skips both the act half and the gate entirely — it reports the outcome and findings only.
- **Fail-closed.** An unread/empty plan is never silently marked reviewed; the workflow leaves the tag in place and reports the fetch failure.

## Guidelines

- Be objective, and cite evidence for every finding.
- The dispatched sub-agents only review and report — they never edit. Only the orchestrator applies small fixes (whole-`--body` writes) and files large findings as tasks, and only after refutation.
- Never guess intent when the target document is ambiguous or missing — report it as a finding instead.
- A surviving `blocking` finding yields `rework` or `escalated`; concerns and suggestions alone never hold the gate closed.

## Review specification

The generated marker block below contains the plan-mode review dimensions (coherence, architectural-fit, unit-of-work), refutation logic, filtering, verdict rules, and gate policy, rendered from the canonical review source via `scripts/gen-skill-review.sh --mode plan`. It documents exactly the pipeline `plan-review.js` runs. Do not hand-edit it.

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
- There is no acceptance-criteria pass/fail table at plan stage — the quality
  of the plan's own acceptance criteria is judged by the **coherence**
  dimension and surfaces as an ordinary finding.

### Verdict — one outcome vocabulary: `reviewed` | `rework` | `escalated`

Determine the outcome in this strict order — the first matching rule wins:

1. **escalated** — a surviving blocker that needs a *human decision* rather
   than a code change: the goal, approach, or scope is wrong, the work
   violates a stated architectural constraint, or the acceptance criteria
   themselves are missing, contradictory, or unimplementable as written.
2. **rework** — else if any surviving finding is `blocking`. The defect is
   fixable in place; the work goes back for another round.
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
