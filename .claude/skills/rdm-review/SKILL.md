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

**IMPORTANT: This is the rdm source repo. Always run `cargo build` first, then use `./target/debug/rdm` — never bare `rdm`.**

The review runs as a pipeline: **find → refute → filter → verdict → act → gate**. Findings are never surfaced, fixed, or acted on until a *separate* agent has tried to refute them. The agent that finds an issue is never the agent that confirms it.

The specification of that pipeline — which dimensions run, how findings are graded, and what each outcome means — is **generated from the canonical review source** and is identical across every rdm surface (the interactive skill, `rdm-dispatch-phase`, and `rdm-autopilot`). It appears under "Review specification" below. The steps here wire it to the CLI.

## Steps

### 1. Setup

0. Run `cargo build` to ensure the binary is up to date.
1. **Parse arguments**: determine whether this is a phase review or task review from `$ARGUMENTS`.
   - If the first argument is `--task`, the next argument is a task slug.
   - Otherwise, the first argument is a roadmap slug and the second is a phase number.
2. **Read the acceptance criteria**:
   - For a phase: `./target/debug/rdm phase show <phase-number> --roadmap <slug> --project rdm`
   - For a task: `./target/debug/rdm task show <slug> --project rdm`
   Extract the acceptance criteria, steps, and any other requirements from the body.
3. **Identify the implementation diff**: use `git log --oneline -20` and `git diff` to understand what was recently changed. Identify the commits and files relevant to this phase or task. Note the diff size, which modules it touches, and whether it changes public API, `unsafe` constructs, dependencies, or user-facing behavior — these are the **trigger signals** for the conditional dimensions in the Review specification.

   From those same diff signals, derive a **tier hint** for step 2's fleet: `small` (localized, single module, no risky surface — a typo fix, a one-line log message), `medium` (an ordinary change — new logic in one module, a bugfix), or `large` (touches public API, `unsafe`, spans multiple modules/crates, adds a dependency, or is user-facing). This is a read of the **diff's risk**, not the phase's own difficulty rating — a "hard" phase can still land a small, low-risk diff, and vice versa.

### 2. Find — dispatch the review fleet (parallel)

Dispatch one **read-only** `Agent` per applicable dimension, per **Review specification § Dimensions** below. Run the always-on dimensions unconditionally; add each triggered dimension when its trigger fires against the diff from step 1. State which dimensions you launched, and why, in the report.

**Model sizing.** Every dispatched agent in this step runs on an **explicitly resolved** model — never the inherited session model. For each finder agent, resolve:
```bash
model=$(./target/debug/rdm model resolve review-find --tier <hint>)
```
using the tier hint derived in step 1, and pass `model` explicitly when dispatching that agent with the `Agent` tool. Purely mechanical checks (e.g. a scripted presence/lint check with no judgment involved) may instead resolve `./target/debug/rdm model resolve mechanical`, or run inline without a subagent at all. Resolution reads the `[models]` config table (tier→model-id bindings, review floor, and per-step overrides), falling back to built-in defaults (`small`→haiku, `medium`→sonnet, `large`→opus) when unset.

### 3. Refute — per-finding refute pass (parallel)

Dispatch a **fresh** `Agent` per finding, per **Review specification § Refute**. Run these concurrently; the finder is never the refuter. Suggestions may skip refutation (low stakes) but are still subject to the confidence floor.

The refute agent also runs on an explicitly resolved model, never the inherited session model: resolve `model=$(./target/debug/rdm model resolve review-verify)` once (its default tier is already floored to the top review tier, so no `--tier` hint is needed) and pass `model` when dispatching each refute agent.

### 4. Filter, consolidate & decide the outcome

Apply **Review specification § Filter & consolidate**, then **§ Verdict** to reach exactly one outcome: `reviewed`, `rework`, or `escalated`.

### 5. Report

Present a single structured report:
- The AC table: each criterion with PASS / FAIL / PARTIAL and evidence.
- Surviving findings grouped by severity (blocking → concern → suggestion), each with file:line, confidence, and recommendation.
- The outcome: **reviewed**, **rework**, or **escalated**, and the one rule that decided it.

### 6. Act

Apply **Review specification § Act**. File large findings as tasks with:
```bash
./target/debug/rdm task create <slug> --title "Review finding: description" --body "Details." --tags <tag1>,<tag2> --no-edit --project rdm
```

### 7. Gate — transition by outcome

This skill owns the `needs-review` → `reviewed` gate. Persist the status from **Review specification § Gate**, then land the plan-repo change:

```bash
# <status> is the mapped status for the outcome: reviewed | in-progress | blocked
./target/debug/rdm phase update <phase> --status <status> --no-edit --roadmap <slug> --project rdm
# or, for a task:
./target/debug/rdm task update <slug> --status <status> --no-edit --project rdm
./target/debug/rdm commit -m "chore(plan): <outcome> <phase-or-task>"
```

On `escalated` **only**, record the escalation on the item itself with `--reason` — that recorded field, not the commit message, is what `rdm review blocked` reads:

```bash
./target/debug/rdm phase update <phase> --status blocked --reason "[code] <the decision or blocker>" --no-edit --roadmap <slug> --project rdm
# or, for a task:
./target/debug/rdm task update <slug> --status blocked --reason "[code] <the decision or blocker>" --no-edit --project rdm
```

On `reviewed` **only**, add the completion trailer to the branch commit — a **separate**, source-repo operation:

```bash
git commit --amend -m "$(git log -1 --pretty=%B)

$(./target/debug/rdm hook done-line --roadmap <slug> --phase <stem>)"
# or, for a task: $(./target/debug/rdm hook done-line --task <slug>)
```

Use the exact slugs/stems from the `rdm` commands above. Do NOT set the item to `done` directly — that flip is owned by the merge-to-main hook.

## Review specification

<!-- rdm:review-spec:begin (generated by scripts/gen-skill-review.sh — edit .claude/workflows/lib/review.mjs, not this region) -->

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
  error paths, judged against the project's error-handling conventions
  (CLAUDE.md / AGENTS.md). User-facing errors must be actionable.
- **tests** — *trigger: the diff adds or changes non-trivial logic, or adds
  no test files.* Do tests exist and cover the key behaviors and edge
  cases? Was a test-first discipline followed? Are there untested branches?
- **architecture** — *trigger: the diff touches more than one module/layer,
  or moves logic between layers.* Does logic live where the project's
  architecture says it should, with thin layers on top? No duplicated logic
  across interfaces?
- **api-docs** — *trigger: the diff changes a public `rdm-core` item.* Are
  public items documented per the project's conventions
  (`#![warn(missing_docs)]`)? Are `# Errors`, `# Panics`, and `# Safety`
  sections present where the project requires them?
- **changelog** — *trigger: the diff makes a user-facing change (CLI
  commands, API endpoints, MCP tools, config options, observable
  behavior).* A user-facing change MUST carry a `CHANGELOG.md` entry in the
  same commit; a missing entry is **blocking**, per the project's
  conventions. The entry must read from a user's perspective, not describe
  internals.
- **security** — *trigger: the diff touches auth, input parsing or
  validation, path/file handling, subprocess or shell invocation, secrets
  and credentials, deserialization, network code, or `unsafe` blocks.*
  Injection, path traversal, secret leakage, missing authorization, and
  unsafe-invariant violations. Every `unsafe` block needs a `// SAFETY:`
  comment stating the invariant it upholds; an unjustified or risky
  construct is a finding.

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

### Act — only on verified findings

Report first, then act. Never fix or file an unverified finding.

- **Small** — localized, low-risk, no new acceptance criteria (a typo, a
  missing doc comment, a tightened error message, an extra test). Fix it
  inline, run the relevant tests, then fold it into the implementation commit.
- **Large** — new modules, cross-cutting changes, or anything that warrants
  its own acceptance criterion. Do **NOT** fix inline: file it as a task.

For each finding, state how it was handled (fixed-inline / filed-as-task).

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
./target/debug/rdm hook done-line --roadmap <slug> --phase <stem>   # prints: Done: <slug>/<stem>
./target/debug/rdm hook done-line --task <slug>                     # prints: Done: task/<slug>
```

On `rework` and `escalated`, write **no** trailer.

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
