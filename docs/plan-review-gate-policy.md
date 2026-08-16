# The plan-review gate: self-review policy, evidence, and deferral

Owner: `.claude/workflows/lib/plan-review.mjs`'s `plan-review-driver` block (and its
byte-identical copy in `.claude/workflows/rdm-wf-plan-review.js`).
Gated by: `scripts/verify-workflow-review.sh` §§ 5b-drift, 5b-mechanical, 5b-exec,
5b-gate-evidence, 5b-gate-action, 5b-gate-return, 5b-gate-loud, 5b-mut, 6.

This document records a decision that was previously implicit: **may a plan review clear
the `needs-plan-review` gate tag on an item the same session authored?** It exists because
the question was forced by three production blocks, and because "make the classifier
quieter" is not an acceptable answer to it.

---

## The decision

**Yes — plan review MAY clear `needs-plan-review` on an item the same session authored,
because the verdict that opens the gate is not authored by that session.**

The reasoning is structural, not procedural:

1. **The orchestrator never produces the finding set.** Every finding comes from an
   independently dispatched finder agent, one per dimension, fanned out by
   `buildReviewPipeline('plan')`. The session driving the review contributes none of them.
2. **The orchestrator never grades them either.** Each gating finding is graded by a
   *separate* refuter agent — a fresh dispatch per finding, bounded by the refutation
   budget. A finding the refuter refutes is dropped; a low-confidence one is dropped by the
   floor. That grading is deliberately **not total**, and the prompt says so rather than
   claiming otherwise — see "The prompt's grading claim is computed, not asserted" below.
3. **The gate is a table lookup, not a judgment.** `GATE_POLICY.plan` maps
   `reviewed | rework | escalated` onto `{ status: null, clearsPlanReviewTag }`. There is
   no discretionary branch for the orchestrator to exercise. `classifyPlanOutcome` derives
   the outcome mechanically from surviving blocking findings.

So the two-party property holds by *construction*: authorship of the plan and authorship of
the verdict are held by different agents, whoever typed the invocation. Requiring a
different human or session to run the review would add ceremony without adding a second
party that isn't already there.

### The prompt's grading claim is computed, not asserted

A first cut of clause 2 said, flatly, that findings "are graded by a second, independent
refuter agent **per finding**". That is not true of every run, and asserting it would have
reproduced this phase's own defect with the sign flipped: a gate whose factual claims do not
survive checking. Three pipeline behaviors leave a survivor un-graded, and all three can
coexist with a `reviewed` outcome:

| Un-graded survivor | Marker on the finding | Why |
| --- | --- | --- |
| a non-gating `suggestion` | `unrefuted: true`, `unrefutedReason: 'non-gating'` | `NON_GATING_SEVERITIES` — it gates at no tier, so no refuter is ever dispatched for it |
| a gating finding past the cap | `unrefuted: true`, `unrefutedReason: 'budget'` | the per-unit refutation budget cut it for cost; the budget skips GRADING, never FILTERING |
| a crashed refuter | `refuterError: true` | a crash is not proof of refutation, so the finding is kept un-refuted |

None of the three prevents `reviewed` (only a surviving **blocking** finding does), so a
blanket claim would be contradicted a few lines later by the prompt's own EVIDENCE block,
which reports produced-vs-graded honestly. That is precisely the inconsistency a careful
classifier is most likely to catch.

So clause 2 is split in two. Its **fixed** half describes the mechanism and is true of every
run: independently dispatched finders, a fresh separate refuter per gating finding, bounded
by a per-unit refutation budget, and a table-lookup gate. Its **conditional** half is
computed by `gateTwoPartyClause` from `buildGateEvidence`'s `gradedCount` /
`ungradedCount` / `ungradedDetail`, which are derived from **this unit's survivors** — not
from `budget.graded`, which describes the pipeline before any consumer-side post-filter. It
renders one of three sentences: nothing survived, all survivors were graded, or an itemised
`<n> x <severity> (<why>)` split with the explicit note that an un-graded survivor was
*reported, not verified*. Severity strings arrive from finder agents, so they are collapsed
to a closed vocabulary (`blocking | concern | suggestion | other`) before interpolation —
the AUTHORIZATION preamble sits above the delimited quoted region and must stay
finder-uninfluenced.

### The boundary — where this does NOT hold

The argument above is entirely contingent on the fan-out actually running. It does **not**
cover a unit that reached `reviewed` through any path that skipped it:

- a **fetch failure** — already fail-closed (`built.fetchFailed` returns `escalated`,
  `fetchError: true`, and gates nothing);
- a **zero-dimension coverage run** — a unit whose `coverage.ran` is empty produced no
  findings because nothing looked, not because nothing is wrong. The gate prompt now
  renders `coverage.ran` verbatim, so this is visible rather than implied;
- a **hoisted-payload shortcut** whose content was never validated — `hoistedFetchedOk` is
  a shape guard only, and content validation of a caller-hoisted payload is owned by
  `phase-2-fix-plan-review-gate-tag-clobber`, not by this decision.

In each case the structural argument is void, and the gate's clearance is not justified by
this document.

### The escalation path

An invoking surface that judges itself too close to the plan — or an operator who simply
wants a checkpoint before any plan state changes — passes **`gateMode: 'return'`**.

The driver then computes the gate action and **writes nothing**. Every unit comes back with

```js
gateAction: {
  kind, ident, roadmap,
  clearsPlanReviewTag,          // false on rework/escalated
  commands: [updateCmd, commitCmd],  // [] when the tag is not to be cleared
  remainingTags,                // the exact sibling-preserved list to be written
  removedTags,                  // [] when the item never carried the tag (idempotent no-op)
  applied, deferred, blocked, blockedReason,
}
```

and `gateDeferred: true`. A human, or a distinct surface, applies `gateAction.commands`.
`gateMode` is read from the **structured `args` object only**, never parsed out of the
`$ARGUMENTS` flag string: a target slug literally named `return`, or a prose target
containing `--gate-mode`, must never silently suppress the gate. An illegal value throws at
parse time, before any agent fires, naming both legal values.

Deferral is a **deliberate hand-off, not a failure** — `deferred` and `blocked` are
separate fields, and the deferred clause on the summary reads differently from the blocked
one, so a `'return'`-mode run is never reported as a broken gate.

### What this does not decide

The broader operator-intent question — *when is an agent-initiated write on the operator's
behalf legitimate at all?* — is owned by the **`review-gate-intent`** roadmap. This
document deliberately does not pre-empt it; it decides one narrow case (this gate, this
tag) on the strength of the structural argument above, and must stay consistent with
whatever `review-gate-intent` concludes. Note that one of the three blocked runs below was
itself a review *of* that roadmap.

---

## The evidence: three recorded classifier blocks across two runs

Verbatim, from the phase that produced this change
(`phase-4-plan-review-gate-blocked-by-safety-classifier`).

### Run `wf_1ee517c8-ec2` — target `review-gate-intent`

> `[gate:clear-tag:roadmap:review-gate-intent]` — "[Self-Approval] ... stamping the agent's
> own authored roadmap as reviewed/passed ... bypassing the two-party review gate for the
> agent's own work."

**Rebuttal.** The agent did not stamp anything as reviewed. The `reviewed` outcome was
produced by `classifyPlanOutcome` over a survivor set authored by independent finders and
graded by independent refuters; the gate is a `GATE_POLICY.plan` lookup over that outcome.
The two-party gate is not bypassed — it is the mechanism that produced the verdict. That
said, the objection was *unanswerable from what the sub-agent was handed*: a bare
two-command instruction with none of that chain attached. That is the defect this phase
fixed.

### Run `wf_7e7d554d-452` — target `project-agnostic-lane`, phase 3

> `[gate:clear-tag:phase:phase-3-generic-diff-signals]` — "[CI Bypass] ... mechanically
> clear the gate tag ... with no visible evidence phase 3 individually passed review and no
> user request for this action."

**Rebuttal, per claim.**

- *"No visible evidence phase 3 individually passed review"* — factually true **of the
  prompt**, false of the run. Phase 3 reached `reviewed` with ZERO blocking findings after
  a full find/refute/filter pass, gated independently of its siblings. The evidence existed
  and was simply not shown to the agent asked to act on it. The prompt now renders it.
- *"No user request for this action"* — false. The operator invoked the plan-review
  skill/workflow, and clearing the tag on `reviewed` is that skill's documented, specified
  gate behavior, spelled out in its own generated spec.
- *"CI Bypass"* — the tag is not a CI gate. It is plan metadata; no build, test, or merge
  check reads it.

### Run `wf_7e7d554d-452` — target `project-agnostic-lane`, phase 5

> `[gate:clear-tag:phase:phase-5-operator-intent-gate]` — "[External System Writes] The user
> only asked a question ... mutating plan repo state that was never requested or approved."

**Rebuttal.** "Only asked a question" is false for the same reason as above: the operator
invoked a skill whose specified terminal step is this write. On blast radius: the write
touches **one plan-metadata tag list** — no rdm status, no code, no land-time completion
directive — and is reversible by re-adding the tag with the same `--tags` command. The
prompt now states this explicitly.

---

## What changed

1. **The gate prompt carries its authorization and its evidence.** Four clauses, each
   answering one recorded objection above: REQUESTED (specified behavior, operator-invoked),
   TWO-PARTY (independent finders, separate refuters, table-lookup gate), BLAST RADIUS (one
   reversible tag list, no status/code/trailer), and EVIDENCE (outcome and round, the
   dimension finders that ran and any that did not, findings produced, findings graded by a
   separate refuter, survivors at blocking severity, and the exact tag list to be written).
   The evidence renders deterministically — sorted dimension lists, fixed key order — and
   degrades to an explicit "unavailable" sentence rather than printing `null`/`undefined`
   when the pipeline reported no coverage or budget.

   The reviewer summary is finder-authored text, so it renders **last**, inside a delimited
   region labelled as data. It can never precede or override the fixed clauses.

   TWO-PARTY's grading claim is **computed per unit, never asserted blanket** — the
   pipeline deliberately leaves non-gating, over-budget, and refuter-crashed survivors
   un-graded, and the clause reports that split honestly instead of overclaiming. See "The
   prompt's grading claim is computed, not asserted" above.

2. **The gate is returnable.** `gateMode: 'return'` computes `gateAction` and writes
   nothing (see above). `gateAction.commands` come from the same `planGateCommands` helper
   the prompt prints, so an action applied by hand is byte-identical to the write the agent
   was asked to make.

3. **A blocked gate is loud.** Previously a refused write left
   `clearsPlanReviewTag: true, tagCleared: false` discoverable only by reading the JSON, and
   the `ack.ok !== true` path logged nothing at all. Now:
   - the unit's `summary` (and the flattened top-level `summary`) gains
     ` [GATE BLOCKED: needs-plan-review NOT cleared despite a reviewed outcome — apply
     manually: <update> && <commit>]`;
   - the unit carries `gateBlocked: true` and `gateAction.blockedReason`, distinguishing
     `ack-not-ok` (a refusal) from `agent-error: <message>` (a crash);
   - a dedicated `GATE BLOCKED` log line is emitted on **both** failure paths;
   - the run-level `gateBlockedCount` counts them, and appears on the final log line when
     non-zero;
   - the skill prose requires a blocked unit to be surfaced at the TOP of the report.

   A healthy run's summary is **byte-unchanged** — the clause is empty, following the same
   discipline as `formatUnitBudget` and `coverageSummaryClause`. A `rework`/`escalated`
   unit, whose `tagCleared` is legitimately false, never trips it.

---

## NON-GOAL

**This change does not, and cannot, guarantee that a safety classifier stops blocking the
gate write.** Classifier behavior is non-deterministic and external; no hermetic harness can
reproduce it, and none in this repo claims to. Any assertion of the form "the gate is no
longer blocked" would be untestable and therefore dishonest.

What the change *does* guarantee, and what the harness gates:

- the gate write is accompanied by its justification and its evidence, deterministically
  rendered;
- the write is **skippable by contract** (`gateMode: 'return'`), so a caller that cannot or
  should not write is never stuck;
- a failure to write is **loud** — in the summary string, in the log, and in a run-level
  count — so nobody reads `clearsPlanReviewTag: true` and assumes the tag is gone.

Making the classifier quieter was explicitly rejected as a solution. The underlying concern
it raised (single-party review of self-authored work) is legitimate, which is why it is
answered above with a decision and a boundary rather than suppressed.

---

## Observed after the change

Recorded here in the same spirit as the three blocked runs above — a single real,
non-hermetic run against a phase that legitimately reaches `reviewed`, since no harness can
substitute for it.

**Status: not yet observed.** The change landed with the hermetic gates green
(`scripts/verify-workflow-review.sh` §§ 5b-gate-evidence / 5b-gate-action / 5b-gate-return /
5b-gate-loud, plus the existing 5b-drift, 5b-mechanical, 5b-exec, 5b-mut). The next real
`rdm-wf-plan-review` run over a unit that reaches `reviewed` should be recorded below with
its run id and whether the `gate:clear-tag` agent was blocked.

| run id | target | outcome | `gate:clear-tag` blocked? | notes |
|---|---|---|---|---|
| _(pending)_ | | | | |

If a run IS still blocked, record it here too and use `gateMode: 'return'` as the supported
path — the AC is the contract (evidence-carrying, skippable, loud), not the classifier's
behavior.
