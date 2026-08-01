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
  - {t_commit}
---

Review the implementation of an rdm phase or task. `$ARGUMENTS` should be `<roadmap-slug> <phase-number>` for a phase, or `--task <task-slug>` for a task.
{principles}
The review runs as a pipeline: **find → refute → filter → verdict → act → gate**. Findings of a **gating** severity are never surfaced, fixed, or acted on until a *separate* agent has tried to refute them; a non-gating `suggestion` is passed through marked `unrefuted: true` and acted on under the un-refuted disposition rule (§ Act). The agent that finds an issue is never the agent that confirms it.

The specification of that pipeline — which dimensions run, how findings are graded, and what each outcome means — is **generated from the canonical review source** and is identical across every rdm surface (the interactive skill, `rdm-dispatch-phase`, and `rdm-autopilot`). It appears under "Review specification" below. The steps here wire it to the rdm MCP tools.

## Steps

### 1. Setup

1. **Parse arguments**: determine whether this is a phase review or task review from `$ARGUMENTS`.
   - If the first argument is `--task`, the next argument is a task slug.
   - Otherwise, the first argument is a roadmap slug and the second is a phase number.
2. **Read the acceptance criteria**:
   - For a phase: use `{t_phase_show}` with `project: {proj_param}, roadmap: "<slug>", phase: "<phase-number>"`
   - For a task: use `{t_task_show}` with `project: {proj_param}, task: "<slug>"`
   Extract the acceptance criteria, steps, and any other requirements from the body.
3. **Identify the implementation diff**: use `git log --oneline -20` and `git diff` to understand what was recently changed. Identify the commits and files relevant to this phase or task. Note the diff size, which modules it touches, and whether it changes public API, `unsafe` constructs, dependencies, or user-facing behavior — these are the **trigger signals** for the conditional dimensions in the Review specification.

   From those same diff signals, derive a **tier hint** for step 2's fleet: `small` (localized, single module, no risky surface — a typo fix, a one-line log message), `medium` (an ordinary change — new logic in one module, a bugfix), or `large` (touches public API, `unsafe`, spans multiple modules/crates, adds a dependency, or is user-facing). This is a read of the **diff's risk**, not the phase's own difficulty rating — a "hard" phase can still land a small, low-risk diff, and vice versa.

### 2. Find — dispatch the review fleet (parallel)

Dispatch one **read-only** `Agent` per applicable dimension, per **Review specification § Dimensions** below. Run the always-on dimensions unconditionally; add each triggered dimension when its trigger fires against the diff from step 1. State which dimensions you launched, and why, in the report.

**Model sizing.** Every dispatched agent in this step runs on an **explicitly resolved** model — never the inherited session model. For each finder agent, resolve `rdm model resolve review-find --tier <hint>` using the tier hint derived in step 1, and pass `model` explicitly when dispatching that agent with the `Agent` tool. Purely mechanical checks (e.g. a scripted presence/lint check with no judgment involved) may instead resolve `rdm model resolve mechanical`, or run inline without a subagent at all. Resolution reads the `[models]` config table (tier→model-id bindings, review floor, and per-step overrides), falling back to built-in defaults (`small`→haiku, `medium`→sonnet, `large`→opus) when unset.

### 3. Refute — per-finding refute pass (parallel)

Dispatch a **fresh** `Agent` per **gating** finding (`blocking` / `concern`), per **Review specification § Refute**. Run these concurrently; the finder is never the refuter. A `suggestion` skips refutation — it gates nothing at any tier — and passes through marked `unrefuted: true`, still subject to the confidence floor.

The refute agent also runs on an explicitly resolved model, never the inherited session model: resolve `rdm model resolve review-verify` once (its default tier is already floored to the top review tier, so no `--tier` hint is needed) and pass `model` when dispatching each refute agent.

### 4. Filter, consolidate & decide the outcome

Apply **Review specification § Filter & consolidate**, then **§ Verdict** to reach exactly one outcome: `reviewed`, `rework`, or `escalated`.

### 5. Report

Present a single structured report:
- The AC table: each criterion with PASS / FAIL / PARTIAL and evidence.
- Surviving findings grouped by severity (blocking → concern → suggestion), each with file:line, confidence, and recommendation.
- The outcome: **reviewed**, **rework**, or **escalated**, and the one rule that decided it.

### 6. Act

Apply **Review specification § Act**. File large findings as tasks with `{t_task_create}`: `project: {proj_param}, slug: "<slug>", title: "Review finding: description", body: "Details."`

### 7. Gate — transition by outcome

This skill owns the `needs-review` → `reviewed` gate. Persist the status from **Review specification § Gate** — the mapped status is `reviewed`, `in-progress`, or `blocked`:

- phase: use `{t_phase_update}` with `project: {proj_param}, roadmap: "<slug>", phase: "<phase>", status: "<status>"`. On `escalated`, pass `status: "blocked", reason: "[code] <the decision or blocker>"` — the recorded `reason` field, not the commit message, is what the blocked queue reads.
- task: use `{t_task_update}` with `project: {proj_param}, task: "<slug>", status: "<status>"`. On `escalated`, pass `status: "blocked", reason: "[code] <the decision or blocker>"` the same way.
- land the plan-repo status change: call `{t_commit}` with `message: "chore(plan): <outcome> <phase-or-task>"`.

On `reviewed` **only**, add the completion trailer to the branch commit with `git commit --amend` — a **separate**, source-repo operation, with no MCP equivalent. Source the trailer line from `rdm hook done-line --roadmap <slug> --phase <stem>` (or `rdm hook done-line --task <slug>`), using the exact slugs/stems from the rdm tools above. Do NOT set the item to `done` directly — that flip is owned by the merge-to-main hook.

## Review specification

<!-- rdm:review-spec:begin (fixed content shipped with this skill — rendered at release time from rdm's own canonical review source; do not hand-edit this region, pick up upstream changes via the next rdm agent-config regeneration) -->

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

**Code review dimensions:**

- **ac** — *always.* For each acceptance criterion, rate PASS / FAIL /
  PARTIAL with evidence (file:line, test name). Flag any criterion that is
  unmet, ambiguous, or untestable. The per-criterion table is the contract
  and is reported intact.
- **correctness** — *always.* Logic bugs, edge cases, race conditions, and
  error paths, judged against the error-handling conventions the project
  states in its principles document (`docs/principles.md` if present,
  otherwise `CLAUDE.md` / `AGENTS.md` in the project root) — which error
  type each layer must use, and where context may be added. User-facing
  errors must be actionable.
- **tests** — *trigger: the diff adds or changes non-trivial logic, or adds
  no test files.* Do tests exist and cover the key behaviors and edge
  cases? Was a test-first discipline followed? Are there untested branches?
- **architecture** — *trigger: the diff touches more than one module/layer,
  or moves logic between layers.* Does logic live where the project's
  stated layering contract puts it, with the interaction layers on top
  staying thin? No duplicated logic across interfaces? Read the project's
  principles document (`docs/principles.md` if present, otherwise
  `CLAUDE.md` / `AGENTS.md`) for the layering contract and the commit-scope
  convention, and flag any change that violates one.
- **api-docs** — *trigger: the diff changes a public API item.* Do public
  items carry the documentation the project's principles document requires
  (`docs/principles.md` if present, otherwise `CLAUDE.md` / `AGENTS.md`)?
  Read it for which items are in scope and which sections each kind of item
  must carry — failure modes, abort conditions, safety invariants,
  examples.
- **changelog** — *trigger: the diff makes a user-facing change (CLI
  commands, API endpoints, MCP tools, config options, observable
  behavior).* A user-facing change MUST carry a changelog entry in the
  same commit; a missing entry is **blocking**. Read the project's
  principles document (`docs/principles.md` if present, otherwise
  `CLAUDE.md` / `AGENTS.md`) for the changelog file, its format, and its
  categories. The entry must read from a user's perspective, not describe
  internals.
- **security** — *trigger: the diff touches auth, input parsing or
  validation, path/file handling, subprocess or shell invocation, secrets
  and credentials, deserialization, network code, or a construct that
  opts out of the language's memory- or type-safety guarantees.*
  Injection, path traversal, secret leakage, missing authorization, and
  violated safety invariants. Where the language offers an escape hatch
  out of its own safety guarantees, every use must be justified in the
  form the project requires and must state the invariant the caller
  upholds — read the project's principles document
  (`docs/principles.md` if present, otherwise `CLAUDE.md` /
  `AGENTS.md`) for that requirement. An unjustified or
  invariant-violating use is a finding.

**Why `ac` and `correctness` are NOT merged into one always-on finder.**
Plan mode's always-on lenses all resolve the SAME findings schema, which is
what makes merging them into one agent even conceivable. Code mode's two are
not symmetric with them. `ac` is the ONE dimension that resolves the
AC-review schema instead of the findings schema, and its per-criterion `ac`
table is the structured side-channel the verdict consumes **directly** — a
channel that never reads a finding's severity, is never refuted, and never
consumes refutation budget. Folding `ac` into a shared findings stream would
route the acceptance-criteria contract through exactly the path it was
deliberately kept out of, and would force a union schema on the merged
agent. So the two stay separate agents, and this is a decision rather than
an oversight.

### Find — one read-only agent per applicable dimension, in parallel

Each finder agent is told: you are a READ-ONLY reviewer, do not edit any
files; review exactly one dimension; report only findings you can back with
concrete evidence — **one strong finding beats five weak ones**; return an
empty finding list if the dimension is clean. Do not report pure
style/formatting nitpicks unless they violate an explicit project rule.

Each finding is reported as:

```
- id: <short-slug>
  concern: <ac|correctness|tests|architecture|api-docs|changelog|security>
  location: <path>:<line>
  severity: blocking | concern | suggestion
  confidence: 0-100
  what-fails: <the specific problem>
  why: <root cause / which rule or AC it violates>
  recommendation: <concrete fix>
```

### Refute — a FRESH agent per GATING finding, in parallel

For every finding whose severity can gate the outcome, dispatch a **separate**
read-only refuter. The agent that found an issue is never the agent that
confirms it. The refuter starts from the stance *"this is NOT a real issue
unless the code proves otherwise"*, reads the actual cited location and its
surrounding context, and returns `refuted` (boolean), a corrected `confidence`
(0-100), and a rationale.

**Non-gating pass-through.** A `suggestion` gates nothing at any tier — the
verdict consults only `blocking` (and `concern`, at the `large` tier), and the
acceptance-criteria channel never reads a finding's severity at all — so a
refuter's verdict on one cannot change the outcome either way. No refuter is
dispatched for it. It passes straight through, marked `unrefuted: true`, and
is still subject to the confidence floor. `suggestion` is the ONLY severity
treated this way, and the rule is fail-safe: a finding whose severity is
missing or unrecognized is refuted like a gating one.

`concern` is deliberately **not** passed through, even though it does not gate
at the default tier. Measured over the whole recorded refuter corpus (989
refuters; `scripts/measure-refuter-severity.mjs`, recorded in
`docs/token-baseline.json` § `nonGatingRefutationSkip`):

| severity | graded | refuted | rate |
|---|---:|---:|---:|
| blocking | 197 | 75 | 38.1 % |
| concern | 522 | 263 | 50.4 % |
| suggestion | 236 | 175 | 74.2 % |

A `concern` is overturned MORE often than a `blocking` one, so its refuter is
doing real work — and it gates outright at the `large` tier. Skipping only
`suggestion` drops 239 refuters (24.2 % of all refuters, 20.7 % of refuter
tokens) with no severity that can gate losing its counter-check.

**Refutation budget.** At most **5** gating findings per review unit are
graded. The unit's whole candidate list is assembled first, the gating half is
ranked severity-then-confidence, and only the top 5 get a refuter; everything
past the cut takes the SAME un-refuted pass-through, marked `unrefuted: true`
with `unrefutedReason: 'budget'`. Non-gating `suggestion` findings never
consume budget. The budget skips **grading**, never **filtering** — an
over-budget finding faces the same confidence floor, and one that survives it
still gates. The default of 5 is measured, not guessed: replaying this
pipeline's own ranking over the recorded corpus
(`docs/token-baseline.json` § `determiningFindingRank`) put the
outcome-determining finding within the top 5 for **100 %** of determining
units at the default tier and **98.2 %** at the `large` tier. It is
overridable per run via `maxRefutations` (`0` is legal and means grade
nothing); there is no "uncapped" sentinel — express that as a large N. When
the bound is hit, the run reports how many findings were produced, how many
were graded, and how many were passed through for budget, so a bounded run is
never read as complete coverage.

**Four states, four markers.** Every finding that reaches you is in exactly
one of these, and they are told apart by markers alone:

| State | Markers |
|---|---|
| graded and survived | no `unrefuted`, no `refuterError` |
| skipped as non-gating | `unrefuted: true`, `unrefutedReason: 'non-gating'` |
| passed over for budget | `unrefuted: true`, `unrefutedReason: 'budget'` |
| grading crashed | `refuterError: true`, and never `unrefuted` |

### Filter & consolidate

- **Drop** any finding a refuter refuted, and any whose post-refutation
  confidence is below the confidence floor (70).
- A refuter that *crashes* is not proof of refutation — keep such a finding as
  un-refuted rather than silently dropping it. It is **not** marked
  `unrefuted: true`: that marker means "deliberately never graded", not
  "grading failed".
- A finding passed through un-refuted carries `unrefuted: true` and faces the
  **same confidence floor** as everything else: the refuter is skipped, the
  floor is not.
- **Dedup** findings pointing at the same location / same root cause (the
  fleet covers overlapping ground by design).
- **Rank** survivors by severity, then confidence, then id.
- The AC table is returned as **structured data**, separate from the
  findings list — never folded into a finding. A surviving FAIL/PARTIAL
  criterion is checked directly against that table, never through finding
  severity or the refute/confidence-floor path, so the guarantee cannot be
  silently defeated by a refuter or the 70-point floor. Trade-off: this also
  means an AC-table FAIL bypasses refutation entirely — a hallucinated FAIL
  from the single `ac` finder can force a spurious rework with no
  counter-check. The AC table and any `ac`-dimension `findings` entry about
  the same criterion are two independent channels, not deduplicated against
  each other.

### Verdict — one outcome vocabulary: `reviewed` | `rework` | `escalated`

Determine the outcome in this strict order — the first matching rule wins:

1. **escalated** — a surviving blocker that needs a *human decision* rather
   than a code change: the goal, approach, or scope is wrong, the work
   violates a stated architectural constraint, or the acceptance criteria
   themselves are missing, contradictory, or unimplementable as written.
2. **rework** — else if any surviving finding is `blocking`, or the structured
   AC table (returned by the `ac` dimension alongside its findings — see
   § Refute above) contains any FAIL or PARTIAL criterion. The AC-table check
   is direct and mechanical: it never routes through finding severity or
   refutation, so it cannot be silently defeated by a refuter or the
   confidence floor. The defect is fixable in place; the work goes back for
   another round.
3. **reviewed** — else. Clean, or clean after small fixes. Surviving
   `concern` and `suggestion` findings are recorded and do **not** gate.

Never downgrade a surviving `blocking` finding to "reviewed with concerns" —
a blocker always yields `rework` or `escalated`.

### Act — verified findings by size, un-refuted ones by disposition

Report first, then act. Findings reach this step with two different
provenances, and they are handled differently:

- A finding a refuter **graded and failed to refute** is acted on by SIZE —
  small or large, below.
- A finding marked `unrefuted: true` was **reported, not verified** — no
  refuter graded it (it is a non-gating severity; see § Refute), so treat it
  as an observation, never as a confirmed defect. Incorporate the ones that
  improve readability or clarity where the change is **not major**. "Major"
  means anything that would alter the approach, widen scope, or touch code
  outside the diff under review — that is follow-up material, not an
  in-flight edit. For each one you do not incorporate: **file** it as a task
  if it is worth keeping (a low-severity security or correctness note is),
  otherwise skip it and state why. An observation must never evaporate into
  a skip reason just because no refuter graded it.
- Read the `unrefutedReason` to tell WHY it went ungraded. `'non-gating'`
  means its severity could not have changed the outcome, so grading it was
  pointless. `'budget'` means the per-unit refutation budget was hit and it
  was cut for COST — prefer FILING that one over skipping it.
- A finding carrying `refuterError: true` is a THIRD case: a refuter was
  dispatched for it and CRASHED. That is not proof of refutation and not a
  deliberate skip, so it is never marked `unrefuted`; treat it as still
  ungraded and say so.

Never fix or file a finding that carries neither provenance.

- **Small** — localized, low-risk, no new acceptance criteria (a typo, a
  missing doc comment, a tightened error message, an extra test). Fix it
  inline, run the relevant tests, then fold it into the implementation commit.
- **Large** — new modules, cross-cutting changes, or anything that warrants
  its own acceptance criterion. Do **NOT** fix inline: file it as a task.

For each finding, state how it was handled (fixed-inline / filed-as-task /
skipped, with a reason). These three are exactly the actions the code lane's
`CODE_ACT` schema accepts — `skipped` exists for an un-refuted observation
that is neither worth incorporating in flight nor worth filing.

### Gate — status mapping

The review owns the `needs-review` → `reviewed` gate. Persist the status the
outcome maps to, for the item's kind:

| Outcome | When | Phase status | Task status | Completion trailer |
|---|---|---|---|---|
| **reviewed** | clean, or clean after small fixes | `reviewed` | `reviewed` | write it |
| **rework** | a fixable defect, or an unmet acceptance criterion | `in-progress` | `in-progress` | do **not** write it |
| **escalated** | a blocker needing a human decision | `blocked` | `blocked` | do **not** write it |

Tasks and phases map identically — `blocked` is a valid task status, so an
escalated task is *not* downgraded to `in-progress`. On `escalated`, prefix
the recorded reason with `[code]` so the blocked queue shows which gate
escalated it.

Never set the item to `done` directly — that flip is owned by the
merge-to-main hook.

**The completion trailer.** On `reviewed` only, amend the land-time
completion trailer into the branch commit; this completes the directive
deliberately deferred by the finalize step, so the merge-to-main hook flips
the item `reviewed → done` later. Never hand-type the trailer format — ask rdm
for it, so the format string has exactly one home:

```bash
rdm hook done-line --roadmap <slug> --phase <stem>   # prints: Done: <slug>/<stem>
rdm hook done-line --task <slug>                     # prints: Done: task/<slug>
```

On `rework` and `escalated`, write **no** trailer.

### Guidelines

- Be objective — evaluate against the stated acceptance criteria, not personal
  preferences.
- Provide specific evidence (file:line, test name) for every finding.
- **No finding of a GATING severity is surfaced, fixed, or filed until a
  separate refuter agent has failed to refute it.** The finder never grades
  its own work. Non-gating `suggestion` findings are the one exception: they
  pass through un-refuted, marked `unrefuted: true`, and are acted on under
  the disposition rule above rather than fixed as verified defects.
- Filter hard: drop refuted findings and anything below 70 confidence. One
  strong finding beats five weak ones.
- The dispatched sub-agents only review and report — they never modify code.
  The orchestrator applies small fixes, and only after refutation or under the
  un-refuted disposition rule.
- Never fix large changes inline — file them as tasks.
- If acceptance criteria are missing or vague, report it as a finding rather
  than guessing intent.

<!-- rdm:review-spec:end -->
