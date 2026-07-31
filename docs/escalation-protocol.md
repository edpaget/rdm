# Escalation protocol

This is the **single shared definition** of when an autonomous run interrupts a
human and how it records what it could not decide. It is referenced by the
`rdm-dispatch-phase` skill (the plan gate and the code-review stage) and by the
autonomous roadmap loop ([`rdm-autopilot`](./autonomous-loop.md)), so every
consumer applies one rule.

Autonomy is only useful if the human is interrupted for the *right* things.
`rdm-review` already absorbs routine code findings, so those must never reach
the user. What is left — genuine decisions and hard blockers — is what escalates.

**Isolation note.** Both escalation stages (`plan` and `code`) are raised and
recorded *inside* the per-phase `Agent` subagent that `rdm-autopilot` dispatches
for each phase — not in the loop's own context. The subagent parks the phase
`blocked` with a stage-tagged reason and returns only its structured outcome, so
the loop never needs to hold the plan or review detail to apply this protocol: it
reads parked escalations back through `rdm phase show` / `rdm review blocked`.

## Taxonomy: routine vs escalation

**Routine findings — never escalate.** Anything `rdm-review` can resolve on its
own: a bug, a missing test, a doc gap, a style fix. These are fixed inline
(small) or filed as a task (large) by the review skill and never surface to the
user.

**Escalations — surface or park.** Four kinds, none of which a dispatched run
can decide for itself:

| Kind | Example |
|------|---------|
| **Ambiguous / untestable AC** | An acceptance criterion that can be read two ways, or that names no observable outcome. |
| **Architectural / design decision with no clear default** | Two defensible designs with materially different consequences and nothing in the phase that picks one. |
| **Exhausted budget** | The bounded plan-revise or rework-retry budget ran out without converging (see *Budgets*). |
| **Hard blocker** | A missing dependency, an external credential, or a requirement that conflicts with another phase — work cannot proceed at all. |

Every escalation is tagged with the **stage** that raised it:

- **`plan`** — raised by the plan gate, before any code is written.
- **`code`** — raised during or after `rdm-review`, once code exists.

## Budgets (the retry triggers)

The dispatch flow is bounded so it never loops indefinitely. Exhausting a budget
is itself an escalation (kind: *exhausted budget*).

There are **four** distinct budgets. Two are *in-run* (inside a single
`dispatch-phase` run) and two are *roadmap-level* (autopilot's own).

### 1. Plan-revise budget = 2 — in-run, per dispatch

The plan gate allows at most **two** `revise` rounds. The attempt sequence is:

```
original plan → review → revise 1 → review → revise 2 → review → escalate
```

A budget of N means N revisions **after** the original attempt, i.e. N + 1 plan
attempts in total. The loop breaks early the moment a review comes back with no
blocking findings. If the plan still does not satisfy the AC after the last
revision, escalate (stage `plan`) rather than revising again.

### 2. Code-rework budget = 2 — in-run, per dispatch

A failing code review allows at most **two** rework passes, counted
**independently** of the plan budget — a plan that took two revisions consumes no
code-rework budget, and vice versa. The attempt sequence is:

```
implement → review → rework 1 → review → rework 2 → review → rework outcome
```

Same shape: budget N = N + 1 implementation attempts, with an early break on a
clean review. If review still fails, the phase returns for rework (routine) — but
if the *reason* review keeps failing is a decision/blocker rather than a fixable
defect, escalate (stage `code`) instead of retrying.

**0 is legal and meaningful** for either budget: no reworks at all — terminate on
the first blocking review. It is never confused with "unset".

**Early exit other than a clean review.** A plan or revise agent that resolves to
`null` (an unknown or unavailable model id) short-circuits the whole dispatch to
`escalated` via the `fetchError` path, consuming no further budget. The null
document is never reviewed as an empty plan and never replaces the last good one.

**Per-run overrides.** Both in-run budgets are overridable per run via the
`dispatch-phase` workflow args `maxPlanRevise` and `maxCodeRework` (non-negative
integers; anything else is rejected at parse time, before any agent runs). They
are threaded from autopilot's `--max-plan-revise` / `--max-code-rework`. An
absurdly large override is accepted by validation but bounded in practice by the
global step budget below.

### 3. Autopilot rework re-dispatch budget = 1 — roadmap-level

`DEFAULT_MAX_REWORK = 1`, stated in `.claude/skills/rdm-autopilot/SKILL.md` (the
prose autopilot loop's own internal constant — there is no `lib/autopilot.mjs`
anymore): how many times a `rework` OUTCOME is re-dispatched before the phase is
parked `blocked [code]`. **Unchanged** by the in-run budget raise.

### 4. Autopilot global step budget = 50 — roadmap-level

`DEFAULT_GLOBAL_BUDGET = 50`, stated in the same skill: the maximum total phase
dispatches per autopilot run, so a pathological roadmap can never loop forever.
**Unchanged.**

### Composed worst case

At both in-run budgets = 2, a single dispatch runs up to **3 plan attempts and 3
code attempts**, and autopilot may re-dispatch a `rework` outcome once more — so
at most **2 dispatches per phase**, i.e. ≤ 2 × phase-count dispatches per run.
`DEFAULT_GLOBAL_BUDGET = 50` therefore still bounds a 25-phase roadmap even in
the fully-pathological case, and is confirmed as the intended backstop at the new
rates rather than silently inherited.

### Which lane these numbers describe

**2 / independent / per-run-overridable describes the workflow lane**
(`.claude/workflows/`: `dispatch-phase.js` and its lib; autopilot itself is now
the prose `rdm-autopilot` skill, not a workflow file). The shipped prose skill
templates under `rdm-core/src/templates/`
(`skill-dispatch-phase-cli.md` and its MCP twin, which hardcode "at most one
revise round") **remain at 1** pending the distribution follow-up, so
`agent-config` consumers still get 1/1 until those templates are updated.

## Decision rule: auto-handle vs park vs raise

For each finding or blocker, apply the first matching rule:

1. **Auto-handle** — it is a routine finding. Let `rdm-review` fix it inline or
   file it as a task. Do not interrupt the user. (This is the common case.)
2. **Park as `blocked`** — it is an escalation, but the run is unattended (no
   human is waiting) *or* other phases can still make progress. Record it (below)
   and move on, so the user can answer it in a batch later.
3. **Raise to the user now** — it is an escalation **and** a human is interactively
   present **and** it blocks all further progress. Stop and ask.

On autopilot the default is **park** — batching decisions is what keeps the loop
from interrupting the user for every phase. Raising mid-run is reserved for the
interactive case where stopping is cheap and progress is otherwise blocked.

## Recording and resuming an escalation

A parked escalation is a phase in the `blocked` status with a recorded reason:

```bash
rdm phase update <phase> --status blocked \
  --reason "[plan] AC 2 is ambiguous: which crate owns parsing — core or cli?" \
  --no-edit --roadmap <slug> --project <project>
```

Prefix the reason with the stage tag (`[plan]` or `[code]`) so the queue shows
where it was raised. The reason is stored in the phase's frontmatter
(`blocked_reason`), is visible in `rdm phase show` (human and `--format json`),
and is **preserved across a later resume** — moving the phase back to
`in-progress` does not erase why it stalled. Clear it explicitly with
`--clear-reason` once the decision has been applied.

## Listing the queue (batch review)

One command surfaces every parked escalation so the user can answer them
together instead of being interrupted mid-run:

```bash
rdm review blocked --project <project>            # human-readable
rdm review blocked --project <project> --format json
```

Each entry carries the phase identifier (`roadmap/stem`), title, and recorded
reason. JSON emits an array of `{identifier, project, title, reason}`.
