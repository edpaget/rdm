# Workflow vs. prose: where the autonomous lane's boundary goes

**Status:** Decided. The rule and the per-script dispositions below are settled; the
migration they describe is tracked by the `prose-autopilot-orchestration` roadmap, whose
phases 2–5 are still in flight. Until phase 3 lands, `.claude/workflows/autopilot.js`
still exists and still runs — this document records where the loop is *going*, not where
it already is.

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
   mid-run gate: `backlog` proposes and stops, `document` writes and stops, and both are
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
`autopilot.js` holds its own `estimate-core` copy rather than calling `estimate`), or
belongs at the prose layer, where no such cap applies. Both constraints are recorded in
`docs/workflow-schemas.md`.

**The anti-criterion.** A **low-iteration sequential driver** — on the order of five
iterations per run — whose own fan-out is negligible buys nothing from the workflow
runtime, and pays for it precisely on the part that changes most. Resume is worthless at
five steps, determinism is trivially satisfied, and the harness coverage that remains is
coverage of policy the harness cannot actually judge. That is the autopilot drive loop,
and it is why the loop moves to prose while everything it drives stays a workflow.

## Dispositions

Moving the drive loop to prose leaves the other seven scripts in place as workflows, but
it is not a pure surface swap, and the table should not be read as claiming one.
`autopilot.js` makes exactly **one** `workflow()` call today — `workflow('dispatch-phase',
…)` — and reaches the estimate fan-out through a stamped `estimate-core` copy of its own,
not by invoking `estimate`. The prose orchestrator will call `dispatch-phase` as autopilot
does today **and additionally call `estimate` as a workflow**, which is a new call path
rather than a preserved one, and which drops `gen-workflow-estimate.sh`'s stamped
consumers from three to one.

Criterion 4 (determinism) is not tabulated because it holds by construction for all eight:
the `Date.now(` / `Math.random(` bans are already grepped by the verify harnesses, so no
script can violate it and stay green.

**Distribution is a separate axis from disposition.** Only `autopilot.js`,
`dispatch-phase.js`, and `review-refute-fix.js` are emitted downstream by
`generate_workflows`; `plan-review.js`, `estimate.js`, `backlog.js`, `document.js`, and
`spike-agent-type.js` are local-only, and every one of the local-only five references
`agentType: 'rdm-mechanical'`, which a downstream tree has no definition for and which
*raises* rather than degrading silently. So the phase 4 rewrite of the **distributed**
`skill-autopilot-{cli,mcp}.md` cannot simply mirror the local prose skill by pointing at
`estimate` — it needs an explicit answer (ship `estimate.js` with the `agentType`
stripped, inline the pre-pass in the shipped prose, or drop the pre-pass downstream).
**Decided (phase 4): drop the pre-pass downstream.** The distributed `rdm-autopilot`
template dispatches every phase at whatever tier `next.model` (or `{t_next}` on the MCP
variant) already reports, defaulting to `medium`, and never invokes `estimate` at all.
Shipping `estimate.js` stays blocked on lifting the `agentType`-downstream rule (owned by
`ship-mechanical-agent-type-downstream`, not this phase), and inlining the pre-pass in
prose would duplicate `estimate.mjs`'s filtering/rating/writeback logic outside its
single-sourced home and risk silent drift. The local dogfood `rdm-autopilot` skill is
unaffected and still invokes the real `estimate` Workflow.

| Script | Fan-out | Shape | Mid-run gate | Disposition |
|---|---|---|---|---|
| `autopilot.js` | only its estimate pre-pass — and that is a stamped `estimate-core` copy (single-sourced in `lib/estimate.mjs`), not a call to `estimate.js` | policy: advance/park, retry budgets, stop conditions, operator summary; sequential `while` loop, ~5 iterations | no | **MOVE to prose** (`rdm-autopilot` skill) — fails criteria 1 and 2, and is the anti-criterion exactly |
| `dispatch-phase.js` | two review stages — plan (4 dimensions) then code (up to 7, narrowed by diff signals) — each fanning `parallel()` over its findings | mechanism: fixed 4-stage plan → plan-review → implement → code-review | no | **STAY** — real fan-out over a fixed procedure |
| `review-refute-fix.js` | same review core: dimensions → findings | mechanism: find → refute → filter → verdict | no | **STAY** — the canonical review pipeline, already single-sourced in `lib/review.mjs` |
| `plan-review.js` | the review core **plus** an outer `parallel()` over phase units | mechanism | no | **STAY** — two nested levels of genuine fan-out |
| `estimate.js` | `parallel()` rate over unestimated phases | mechanism | no | **STAY** — the pre-pass fan-out, which the prose loop will now depend on *newly*, as a real `workflow()` call rather than autopilot's stamped copy |
| `backlog.js` | `parallel()` over ≤4 signal categories | mechanism; propose-only, zero mutation | no — the handoff to a human is terminal | **STAY** |
| `document.js` | `parallel()` git-gather over completed phases | mechanism; zero rdm mutation | no — approval is terminal | **STAY** |
| `spike-agent-type.js` | none (its cases are dispatched sequentially on purpose) | neither — it is a spike artifact that exercises the Workflow runtime itself, not a lane | n/a | **STAY, exempt** — kept as the executable record of the spike; it would not be authored as a lane workflow today |

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

`scripts/verify-workflow-autopilot.sh` currently gates the drive loop hermetically —
drive-to-reviewed, rework/park, escalation, budget stops, the estimate pre-pass, and
`--plan-only` — plus a byte-identity drift gate against `lib/autopilot.mjs`. Prose cannot
be gated that way. Retiring the JS loop therefore **loses real regression coverage**, and
deciding what of that coverage survives (and in what form) is a first-class phase of
`prose-autopilot-orchestration` — phase 3 — not a cleanup afterthought. Criterion 5 above
cuts both ways: the loop is a poor fit for a hermetic harness, but "poor fit" is not
"zero value", and what it does catch has to be replaced or consciously given up.

## Coupling

`autopilot.js` has eight touchpoints, which is why this is a roadmap rather than a task:
the two verify harnesses that drive it, the generator that stamps `estimate-core` into
it, the distribution byte-identity gate, `agent_config.rs`'s emission, both
`skill-autopilot-{cli,mcp}.md` shims, and the distributed template copy. They are
enumerated in full under "Coupling to be unwound" in the roadmap body
(`rdm roadmap show prose-autopilot-orchestration --project rdm`); that enumeration is
canonical and is not duplicated here.
