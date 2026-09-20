# Workflow vs. prose: where the autonomous lane's boundary goes

**Status:** Decided, and the migration is done. The rule and the per-script dispositions
below are settled; the migration they described was carried out by the
`prose-autopilot-orchestration` roadmap. Phase 3 retired `.claude/workflows/autopilot.js`
and `lib/autopilot.mjs` in favor of the prose `rdm-autopilot` skill (see
`docs/autonomous-loop.md`) — this document now records where the loop went and why, and
the `autopilot.js` row in the Dispositions table below is kept as a historical record of
the reasoning, not a live inventory entry.

The autonomous lane has two surfaces: deterministic **Workflow-tool scripts** under
`.claude/workflows/`, and **prose skills** under `.claude/skills/`. The decision that
created that split named two categories but was only ever applied to one script, so
there was nowhere to classify a new script without re-litigating the whole question.
This document is that place.

## The rule

A unit belongs in a **workflow** when it has all five of these properties. It belongs in
**prose** when it does not — and in particular when the anti-criterion below applies.

1. **Fan-out-shaped.** It has genuine concurrent fan-out over a data-sized collection —
   dimensions × findings, phases, signal categories — not a handful of sequential steps
   that merely *could* be written as `parallel()`. Fan-out is what the runtime is
   actually good at; a script with none is paying for machinery it never uses.
2. **Mechanism, not policy.** Its steps are a fixed procedure that does not change when
   operational judgment does. Policy — stop-vs-continue, retry budgets, how an outcome
   is interpreted, what the operator is told — is the part that changes most often and
   is cheapest to iterate in prose. A script whose body is mostly policy will be edited
   constantly, and every edit costs a generator run, a byte-identity gate, and a harness
   update.
3. **Headless.** No *mid-run* human gate. A terminal handoff at end-of-run is not a
   mid-run gate: `rdm-wf-backlog` proposes and stops, `rdm-wf-document` writes and stops, and both are
   fine as workflows. A unit that must pause for approval and then continue is not.
4. **Deterministic / resumable.** The same inputs produce the same `agent()` call
   sequence, so the runtime's prefix-cached resume is meaningful. (This is also why
   `Date.now()` / `Math.random()` are forbidden in workflow scripts.)
5. **Hermetically gatable.** Its logic can be single-sourced into a `lib/*.mjs` module
   and driven under injected fakes by a `verify-workflow-*.sh` harness. If the thing
   being tested is a judgment call rather than a call sequence, a harness cannot assert
   it and the gate is theatre.

**Two runtime constraints shape every answer above.** The runtime cannot `import` or
`require`, so shared logic is single-sourced in `lib/*.mjs` and stamped verbatim into
consumers by a `gen-workflow-*.sh` generator under a byte-identity gate. And `workflow()`
nesting is capped at **one level** — a workflow may call another, but that one may not.
A candidate that needs to compose two existing workflows therefore cannot simply nest
them; it either spends the single level, carries a stamped copy (which is why
`autopilot.js` held its own `estimate-core` copy rather than calling `rdm-wf-estimate`), or
belongs at the prose layer, where no such cap applies. Both constraints are recorded in
`docs/workflow-schemas.md`.

**The anti-criterion.** A **low-iteration sequential driver** — on the order of five
iterations per run — whose own fan-out is negligible buys nothing from the workflow
runtime, and pays for it precisely on the part that changes most. Resume is worthless at
five steps, determinism is trivially satisfied, and the harness coverage that remains is
coverage of policy the harness cannot actually judge. That is the autopilot drive loop,
and it is why the loop moves to prose while everything it drives stays a workflow.

## Dispositions

Moving the drive loop to prose left the other seven scripts in place as workflows (six
today — `agent-orchestrated-dispatch` phase 7 retired `rdm-wf-dispatch-phase`), but
it was not a pure surface swap, and the table should not be read as claiming one.
`autopilot.js` made exactly **one** nested `workflow()` call — a dispatch of the
phase engine, then named `dispatch-phase` — and reached the estimate fan-out through a stamped `estimate-core` copy of its own,
not by invoking `rdm-wf-estimate`. The prose `rdm-autopilot` skill called `rdm-wf-dispatch-phase` as
autopilot did **and additionally calls `rdm-wf-estimate` as a workflow**, which was a new call path
rather than a preserved one, and which dropped `gen-workflow-estimate.sh`'s stamped
consumers from three to one. Since `agent-orchestrated-dispatch` phase 6 it no longer calls the
phase engine at all: it enters the prose `rdm-dispatch-phase` orchestrator with `Skill`, into its own
session, so `rdm-wf-estimate` is the one Workflow the drive loop itself still invokes.

Criterion 4 (determinism) is not tabulated because it held by construction for all eight
while `autopilot.js` still existed, and continues to hold for the six live scripts
today: the `Date.now(` / `Math.random(` bans are already grepped by the verify harnesses,
so no script can violate it and stay green.

**Distribution is a separate axis from disposition.** Only `rdm-wf-review-refute-fix.js` is
emitted downstream by `generate_workflows` (`autopilot.js` was
too, until phase 3 retired it in favor of the prose `rdm-autopilot` skill, and
`rdm-wf-dispatch-phase.js` was until `agent-orchestrated-dispatch` phase 7 retired it — see the
retirement record below); `rdm-wf-plan-review.js`,
`rdm-wf-estimate.js`, `rdm-wf-backlog.js`, `rdm-wf-document.js`, and
`spike-agent-type.js` are local-only, and every one of the local-only five references
`agentType: 'rdm-mechanical'`, which a downstream tree has no definition for and which
*raises* rather than degrading silently. So the phase 4 rewrite of the **distributed**
`skill-autopilot-cli.md` cannot simply mirror the local prose skill by pointing at
`rdm-wf-estimate` — it needs an explicit answer (ship `rdm-wf-estimate.js` with the `agentType`
stripped, inline the pre-pass in the shipped prose, or drop the pre-pass downstream).
**Decided (phase 4): drop the pre-pass downstream.** The distributed `rdm-autopilot`
template dispatches every phase at whatever tier `next.model` already reports,
defaulting to `medium`, and never invokes `rdm-wf-estimate` at all.
Shipping `rdm-wf-estimate.js` stays blocked on lifting the `agentType`-downstream rule (owned by
`ship-mechanical-agent-type-downstream`, not this phase), and inlining the pre-pass in
prose would duplicate `estimate.mjs`'s filtering/rating/writeback logic outside its
single-sourced home and risk silent drift. The local dogfood `rdm-autopilot` skill is
unaffected and still invokes the real `rdm-wf-estimate` Workflow.

**Decided (`agent-orchestrated-dispatch` phase 6): omit the plan-review call downstream.**
The per-phase driver moved to prose in that phase — `.claude/skills/rdm-dispatch-phase/SKILL.md`
is now the orchestrator, loaded into the main session with `Skill`, and it invokes **two**
Workflows: `rdm-wf-plan-review` on the `plan/<slug>` document and `rdm-wf-review-refute-fix`
on `change/<sha>`. Only the second of those is emitted downstream (`SHIPPED_WORKFLOWS` in
`rdm-core/src/agent_config.rs` emits `rdm-wf-review-refute-fix.js`, nothing else), so a
distributed skill naming
`rdm-wf-plan-review.js` would reference a file absent from its own tree and fail
`scripts/verify-agent-config-distribution.sh`'s shim-reference check. This is the identical
hazard as the `rdm-wf-estimate` case above, and it takes the identical answer: the
distributed `skill-dispatch-phase-cli.md` **omits the plan-review invocation** and waits on a
human-submitted `rdm review submit --verdict approve` on the plan at that step, while this
repo's local `.claude/skills/` copy makes the real Workflow call. The two are the *same read*
— rdm flips plan status on any `review submit` against a `plan/<slug>` target — so the
distributed procedure is not a degraded variant of the gate, only a different author of the
approving review. Shipping the plan-review engine downstream stays `ship-plan-review-workflow`'s
job.

**How the prose orchestrator is validated.** By dogfooding, and improved iteratively from what
a real drive surfaces (operator, 2026-09-20). No harness greps its prose: a test asserting that
particular strings are present in a static file is not evidence that the procedure behaves, so
`scripts/verify-skill-dispatch.sh` is deliberately not written, and there is **no
live-smoke-run gate** anywhere in this lane. What does gate it is real-binary machinery —
`scripts/verify-agent-config-distribution.sh` and `scripts/verify-plugin-install.sh` over the
emitted templates, `scripts/verify-workflow-review.sh` over the review engines, and
`cargo nextest run` over the plan, review and gate surfaces the prose drives. The same phase
deleted `scripts/verify-workflow-do-auto.sh` and `scripts/verify-workflow-do-auto-task.sh`
(their subject, the `--auto` → engine wiring inside `rdm-do`'s prose, no longer exists) and
narrowed `scripts/verify-skill-autopilot.sh` to its real-binary sections plus the one
`Skill`-entry contract nothing else covers. The wider sweep of that harness class is owned by
`task/retire-static-grep-harnesses`.

| Script | Fan-out | Shape | Mid-run gate | Disposition |
|---|---|---|---|---|
| `autopilot.js` | only its estimate pre-pass — and that was a stamped `estimate-core` copy (single-sourced in `lib/estimate.mjs`), not a call to `rdm-wf-estimate.js` | policy: advance/park, retry budgets, stop conditions, operator summary; sequential `while` loop, ~5 iterations | no | **MOVED to prose** (`rdm-autopilot` skill) — failed criteria 1 and 2, and was the anti-criterion exactly *(historical row — retired to prose in phase 3 of `prose-autopilot-orchestration`; the file no longer exists)* |
| `rdm-wf-dispatch-phase.js` | two review stages — plan (4 dimensions) then code (up to 7, narrowed by diff signals) — each fanning `parallel()` over its findings | mechanism: fixed 4-stage plan → plan-review → implement → code-review | no | **retired (`agent-orchestrated-dispatch` phase 7)** — phase 6 replaced this engine with the prose `rdm-dispatch-phase` orchestrator and no lane called it any more; phase 7 deleted the file, `lib/dispatch-phase.mjs`, the shipped template, the plugin-tree copy and the emission registration. Retirement rests on the operator's design principle — Workflows are for extremely deterministic *mechanism* (the review-refute cycle); agent *judgment* above the review gate lives in prose — and on the replacement's **functional acceptance, not measured superiority**: no cost, speed, context-ceiling or performance-parity claim was produced or is implied. *(historical row — the file no longer exists)* |
| `rdm-wf-review-refute-fix.js` | same review core: dimensions → findings | mechanism: find → refute → filter → verdict | no | **STAY** — the canonical review pipeline, already single-sourced in `lib/review.mjs` |
| `rdm-wf-plan-review.js` | the review core **plus** an outer `parallel()` over phase units | mechanism | no | **STAY** — two nested levels of genuine fan-out |
| `rdm-wf-estimate.js` | `parallel()` rate over unestimated phases | mechanism | no | **STAY** — the pre-pass fan-out, which the prose loop now depends on *newly* (in the local dogfood skill only — the distributed template drops the pre-pass, see "Decided (phase 4)" above), as a real `workflow()` call rather than autopilot's former stamped copy |
| `rdm-wf-backlog.js` | `parallel()` over ≤4 signal categories | mechanism; propose-only, zero mutation | no — the handoff to a human is terminal | **STAY** |
| `rdm-wf-document.js` | `parallel()` git-gather over completed phases | mechanism; zero rdm mutation | no — approval is terminal | **STAY** |
| `spike-agent-type.js` | none (its cases are dispatched sequentially on purpose) | neither — it is a spike artifact that exercises the Workflow runtime itself, not a lane | n/a | **STAY, exempt** — kept as the executable record of the spike; it would not be authored as a lane workflow today |

## Retirement record: `rdm-wf-dispatch-phase` (`agent-orchestrated-dispatch` phase 7)

The engine was retired on the **functional acceptance** of its prose replacement. No
comparative benchmark, token baseline, context-ceiling estimate or performance-parity verdict
was produced, and none is implied here: nothing in this record claims the prose orchestrator
is cheaper, faster, or better-scaling than the engine it replaces. What it rests on is the
operator's design principle (2026-09-19) that Workflows are for extremely deterministic
mechanism while judgment above the review gate lives in prose, plus the two independent
approvals below.

**Phase 6's approval — and the gap in it.** Phase 6 (`phase-6-prose-phase-orchestrator`,
status `reviewed`) carries **no persisted review**: `rdm review list --on
phase/agent-orchestrated-dispatch/phase-6-prose-phase-orchestrator --project rdm` returns
nothing. Its code review lived only inside Workflow run **`wf_6197ebcd-598`**, the
`rdm-wf-dispatch-phase` dispatch that drove it, whose verdict that engine returned as OUTCOME
data and never wrote to any document. That missing trail is not incidental to this
retirement — it is one of the reasons for it: an engine that decides a phase is reviewed and
leaves no reviewable record behind is precisely the judgment-above-the-gate work that belongs
in a lane which persists its reviews. No review id is invented here to fill the gap.

**This phase's own trail — the replacement's dogfooded acceptance.** Phase 7 is the first
phase driven end to end by phase 6's prose orchestrator, and that drive is what accepts it.
Unlike phase 6, it persisted every step:

- Plan document: **`plan/phase-7-parity-and-retirement`** (status `approved`, implements
  `rdm:phase/agent-orchestrated-dispatch/phase-7-parity-and-retirement`).
- Approving plan review: **`2026-09-20-1412-7da6`** (author `edward`, `submitted` /
  `approve`), transcribing plan-review run **`wf_f4ea10da-078`**'s clean verdict after the
  engine misrouted it — see `task/plan-review-engine-ignores-rdmbin`.
- Change review: **`2026-09-20-1631-c691`**, on
  `change/512767e0d268f3de9722dff831039cb76a4a83e9` (branch
  `roadmap/agent-orchestrated-dispatch`), recorded by the orchestrator on this phase's head.
  Look it up with `rdm plan show phase-7-parity-and-retirement --project rdm --format json` →
  `change_reviews[]`. **Not** with `rdm review list --on phase/…`: `ReviewTarget::same_item`
  matches only same-kind targets, so a `change/<sha>` review never surfaces under a `phase/`
  filter.

Read either approval back with `rdm review show <id> --project rdm`, and the plan with
`rdm plan show phase-7-parity-and-retirement --project rdm --format json`.

**Accepted loss.** Deleting `scripts/verify-workflow-dispatch.sh` gave up its
static-invariant net (greps over prose and templates) along with the engine it tested. Every
*behavioral* protection it carried survives elsewhere — wrong-checkout selection and gate
override in `rdm-core/tests/gate.rs` + `rdm-cli/tests/cli_gate.rs` + `scripts/verify-reviewed-gate.sh`,
required review coverage in `scripts/verify-workflow-review.sh` §3c, persist-side anchor
accounting in `scripts/verify-workflow-review-outcome.sh` (and `scripts/verify-workflow-review.sh`
§ 9a), the verification gate in `rdm-cli/tests/cli_verify.rs`,
and the no-completion-trailer-before-land rule in `scripts/verify-skill-autopilot.sh`. The
grep-only half was dropped deliberately; that class is owned by
`task/retire-static-grep-harnesses`, which this phase does not close.

## Non-goals

Two arguments are deliberately **not** part of this decision. Both were considered and
set aside, and neither should be reintroduced as justification for it.

- **This is not `program-driven-orchestration`.** That roadmap explores the *opposite*
  direction — moving control out of prompts and into code, via hooks and a headless
  `claude -p` / Agent SDK orchestrator. It is **deferred, not superseded**: the
  headless-orchestrator path bills against API usage and is incompatible with Claude
  subscription billing. Revisit it later, or as a mode for an enterprise work account.
  Nothing from this boundary work should be filed against it.

- **This is not justified by "liveness".** The rejected claim was that prose wins
  because a prose orchestrator receives task notifications and can pull a *backgrounded*
  subagent forward, which the headless workflow runtime cannot. The transcripts from run
  `wf_52b569c9-9b4` **refute** it: the stalled implementers were not backgrounded and
  waiting for supervision — they were actively running (27 `Bash` calls in one sampled
  agent, transcripts of 100–276 KB each) and had wrongly concluded their work was
  already committed. Supervision does not fix a confident wrong inference. The case for
  this boundary is architectural and does not depend on the claim.

## Known cost

`scripts/verify-workflow-autopilot.sh` used to gate the drive loop hermetically —
drive-to-reviewed, rework/park, escalation, budget stops, the estimate pre-pass, and
`--plan-only` — plus a byte-identity drift gate against `lib/autopilot.mjs`. Prose cannot
be gated that way. Retiring the JS loop therefore **lost real regression coverage**, and
deciding what of that coverage survived (and in what form) was a first-class phase of
`prose-autopilot-orchestration` — phase 3 — not a cleanup afterthought. Criterion 5 above
cuts both ways: the loop was a poor fit for a hermetic harness, but "poor fit" is not
"zero value", so what it caught was replaced rather than dropped: phase 3 landed
`scripts/verify-skill-autopilot.sh`, which gates the surviving loop policies (static text
invariants for drive-to-reviewed, rework-retry-once-then-park, escalated→park, budget
stops, estimate-pre-pass-always-runs, `--plan-only` dedup) plus a dynamic advance/park
write+read-back contract against the real binary — there is no `lib/autopilot.mjs`
anymore, so there is no byte-identical-copy drift gate to run.

## Coupling

`autopilot.js` had eight touchpoints, which is why this was a roadmap rather than a task:
the two verify harnesses that drove it, the generator that stamped `estimate-core` into
it, the distribution byte-identity gate, `agent_config.rs`'s emission, both
`skill-autopilot-cli.md` shims, and the distributed template copy. They were
enumerated in full under "Coupling to be unwound" in the roadmap body
(`rdm roadmap show prose-autopilot-orchestration --project rdm`); that enumeration is
canonical and is not duplicated here.
