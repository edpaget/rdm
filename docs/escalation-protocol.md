# Escalation protocol

This is the **single shared definition** of when an autonomous run interrupts a
human and how it records what it could not decide. It is referenced by the
`rdm-dispatch-phase` skill (the plan gate and the code-review stage) and by the
autonomous roadmap loop ([`rdm-autopilot`](./autonomous-loop.md)), so every
consumer applies one rule.

Autonomy is only useful if the human is interrupted for the *right* things.
`rdm-review` already absorbs routine code findings, so those must never reach
the user. What is left — genuine decisions and hard blockers — is what escalates.

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

The dispatch flow is bounded so it never loops indefinitely. Exhausting either
budget is itself an escalation (kind: *exhausted budget*):

- **Plan-revise budget = 1.** The plan gate allows at most one `revise` round.
  If the plan still does not satisfy the AC after the single revision, escalate
  (stage `plan`) rather than revising again.
- **Rework-retry budget = 1.** A failing `rdm-review` allows one rework pass. If
  review still fails, the phase returns to `in-progress` (routine rework) — but
  if the *reason* review keeps failing is a decision/blocker rather than a
  fixable defect, escalate (stage `code`) instead of retrying.

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
