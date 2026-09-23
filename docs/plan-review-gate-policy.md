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

### The gate returns; the orchestrator applies

**This is no longer an escape hatch — it is how the gate works.** The engine has no
agent that can run a shell command, so it cannot write the tag at any time, for any
caller. `gateMode` is gone with the choice it used to express.

The driver computes the gate action and **writes nothing**. Every unit comes back with

```js
gateAction: {
  kind, ident, roadmap,
  clearsPlanReviewTag,          // false on rework/escalated
  tagsUnknown,                  // true when the caller supplied no tag list
  commands: [updateCmd, commitCmd],  // [] when there is nothing to write
  remainingTags,                // the exact sibling-preserved list to be written
  removedTags,                  // [] when the item never carried the tag (idempotent no-op)
}
```

and the orchestrator applies `gateAction.commands`, in order, reporting the exit
status. `result.gatePendingCount` says how many units are waiting on it.

**The commands can only be built from a tag list the caller supplied.** `--tags`
replaces the whole list, so a unit whose current tags the engine was never shown
gets `commands: []` and `tagsUnknown: true` rather than a `--tags ""` that would
silently drop a sibling tag such as `depends-unlanded`. Refusing to guess is the
only safe branch, and it is visible rather than silent.

Because the hand-off is the whole mechanism, the pending clause carries the
**commands themselves**, not a pointer at the JSON:

```
 [gate pending: needs-plan-review is cleared by running — <update> && <commit>]
```

A surface that reports only the `summary` — a log line, a chat message — is therefore
already reporting exactly what remains to be done.

**There is no `gateBlocked` any more.** It meant "the write was attempted and did not
succeed", and nothing attempts it: an orchestrator whose own `rdm ... update` exits
nonzero reports that itself, with the shell's own message.

If nobody applies the commands, nothing is silently passed: the item simply keeps
`needs-plan-review` and is picked up by the standing
`rdm search "" --tag needs-plan-review --project rdm` sweep.

### What this does not decide

The broader operator-intent question — *when is an agent-initiated write on the operator's
behalf legitimate at all?* — is owned by the **`review-gate-intent`** roadmap. This
document deliberately does not pre-empt it; it decides one narrow case (this gate, this
tag) on the strength of the structural argument above, and must stay consistent with
whatever `review-gate-intent` concludes. Note that one of the three blocked runs below was
itself a review *of* that roadmap.

---

## The `planned` status decision

**Decision: rdm adds NO `planned` status.** Phase statuses stay exactly the
seven they have always been (`not-started`, `in-progress`, `needs-review`,
`reviewed`, `done`, `blocked`, `wont-fix`), and task statuses their seven
(`open` in place of `not-started`).

### What was asked

The `agent-orchestrated-dispatch` roadmap's phase 5 was told to decide, *from
the evidence of running phases 2–4 against a real roadmap*, whether an explicit
`planned` status is needed between `in-progress` and `reviewed` — with a stated
default of no new status, on the grounds that the `reviewed` gate
(`docs/core-enforced-gates.md`) transitively blocks implementation-without-plan
from ever completing.

### The evidence

1. **Plans already carry their own lifecycle.** A plan document has a
   `PlanStatus` of `draft | approved | changes-requested | superseded`, derived
   from reviews rather than set by a flag, and queryable directly:
   `rdm plan list --implements phase/<roadmap>/<stem>`. "This item has been
   planned" is already a fact the plan repo records — on the plan, where it
   belongs.

2. **Phases 2–4 never needed it.** Nothing in running those phases produced a
   moment where "planned but not implemented" was inexpressible. Phase 4
   (`phase-4-change-review-target`) reached `blocked` with its rework budget
   exhausted; that outcome was expressible in the existing vocabulary, and a
   `planned` rung would not have changed what an observer learned from it.

3. **The `reviewed` gate makes the rung unnecessary.** `rdm-core/tests/gate.rs`
   demonstrates that an item with no approved plan *cannot reach `reviewed` at
   all* once `gates.reviewed` is on. Implementation-without-plan is blocked at
   the exit, transitively, so there is nothing left for an intermediate status
   to prevent.

### The counter-argument, and why it does not carry

An observer watching an item sit at `in-progress` cannot tell whether it is
being planned or being implemented. That is a real loss of resolution.

It does not carry, because the signal the observer wants already exists and is
already queryable: the plan document's own `PlanStatus`. Adding a seventh phase
status would make the same fact recoverable from two places that can disagree —
an item stamped `planned` whose plan is still `draft`, or an `in-progress` item
whose plan is `approved`. That is precisely the second-source-of-truth
anti-pattern `PlanStatus`'s own rustdoc rejects for plans ("never set directly
by a status flag: it is **derived from reviews**"). Having rejected it there, it
would be incoherent to introduce it here.

### Enforced by code, not by this prose

Two independent tripwires make reversing this decision a deliberate,
test-visible act rather than a silent one:

- `rdm-core/src/model.rs`'s `phase_status_variants_are_exactly_the_seven_recorded`
  and `task_status_variants_are_exactly_the_seven_recorded` match exhaustively
  over the status enums and assert the recorded seven-element list. A new
  variant fails to compile at the match first, then fails the assertion — and
  both failure messages point back at this section.
- `tests/golden/describe.json` enumerates the same seven `enum_values` for
  `phase.status` and `task.status`, gated by `scripts/verify-golden-json.sh`.

(No test asserts on this document's prose. The decision is enforced by the code;
the reasoning is enforced by review.)

### If a later phase reverses it

Should `planned` ever be adopted, these are the touch points — named here so
the contingent half of the question is answered rather than deferred:

- `rdm-core/src/ops/next.rs` — `NextPhase.status` and the actionability
  predicate in `next_actionable` must decide whether a `planned` phase is the
  next actionable one.
- `rdm-core/src/hook.rs` plus `rdm hook post-merge` / `post-commit` — the
  `Done:` directive's terminal write, and whether `planned` is a legal
  predecessor of `done`.
- `rdm-tui` — status rendering and the status filters.

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

2. **The gate does not write; it returns.** `gateAction.commands` come from the same
   `planGateCommands` helper, and the orchestrator runs them. There is no `gateMode`,
   because there is no second behaviour to select.

3. **A gate the caller has not yet applied is loud.** The unit's `summary` (and the
   flattened top-level `summary`) gains
   ` [gate pending: needs-plan-review is cleared by running — <update> && <commit>]`;
   `result.gatePendingCount` counts them, appears on the final log line when non-zero,
   and the skill prose requires a pending unit to be surfaced at the TOP of the report
   and never described as cleanly reviewed until the commands have run. A unit whose
   tag list the caller did not supply carries `tagsUnknown: true` and gets **no**
   commands, because `--tags` replaces the whole list and guessing would drop a sibling.

   A `rework`/`escalated` unit never trips the clause — it is not supposed to clear the
   tag — so a healthy run's summary stays byte-unchanged, following the same discipline
   as `formatUnitBudget` and `coverageSummaryClause`.

   **Quoting hazard, deliberately contained.** The pending clause embeds an exact rdm
   command containing double quotes (`--tags "a,b"`) — unlike `coverageSummaryClause`,
   which is documented as quote-free *because* it is interpolated into Bash prompts. In
   plan mode `summary` and `reason` are returned data, never prompt inputs, and
   `lib/plan-review.mjs` builds no prompt at all any more.

4. **The policy is stated on every plan surface; the driver's field names are not.**
   `.claude/workflows/lib/review.mjs`'s `//|plan|` spec is stamped into **four** plan-review
   consumers — the local dogfood shim `.claude/skills/rdm-plan-review/SKILL.md`, the two
   shipped templates `rdm-core/src/templates/skill-plan-review-cli.md`, and
   `plugins/rdm/skills/plan-review/SKILL.md` — and only the first of those is driven by
   `rdm-wf-plan-review.js`. That workflow is **local-only**:
   `rdm-core/src/templates/workflows/` ships `rdm-wf-review-refute-fix.js` and nothing else
   (it also shipped `rdm-wf-dispatch-phase.js` until `agent-orchestrated-dispatch` phase 7
   retired that engine). The shipped and plugin skills run the
   gate themselves, in hand-authored prose that shells out to `rdm … update --tags …`
   directly; they have no driver to read a returned `gateAction` off.

   So the shared spec states the policy in terms of **the write** — state its evidence, be
   loud about a clear that has not happened yet, and report the exact commands — which
   every plan surface can act on whatever mechanism it uses. `gateAction` appears only in
   the local shim's **hand-authored** prose, above the generated marker. Stamping it into
   the shared spec would emit an uninstructable instruction into every downstream tree,
   which is the failure this point exists to prevent.

   Both halves used to be gated: `verify-workflow-review.sh` § 1d-gate-policy required the
   shared spec and both shipped templates to be free of those driver names *and* required the
   local shim to carry `gateAction`, with a § 1g self-test planting a field name in the
   `//|plan|` region and regenerating to prove the detector fired. That section was retired
   by the operator amendment to `task/retire-static-grep-harnesses` (2026-09-23); the scoping
   above is now held by convention and code review, not an automated check.

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
5b-gate-loud, plus the existing 5b-drift, 5b-mechanical, 5b-exec, 5b-mut).

The reason it is still pending is structural, not an oversight: the observation requires
dispatching the real `rdm-wf-plan-review` Workflow, and the implementing session had no
`Workflow` tool in its surface — a sub-agent cannot invoke one. It has to be run from a
session that can, against a unit that actually reaches `reviewed` (a plan-only rehearsal
proves nothing, because the gate half only executes on a real `reviewed` outcome). Tracked
as task `observe-plan-review-gate-after-evidence-change`.

Record the run in this table, whichever way it goes:

| run id | target | outcome | `gate:clear-tag` blocked? | notes |
|---|---|---|---|---|
| _(pending)_ | | | | |

Alongside the row, note whether the unit's `tagCleared` matched its `clearsPlanReviewTag`,
and — if it did not — whether the run's `summary` carried the `[GATE BLOCKED: …]` clause and
`gateBlockedCount` was non-zero, since that loud path is the half of the change a blocked
run actually exercises.

If a run IS still blocked, that is a recordable outcome, not a failed phase: record it here
and use `gateMode: 'return'` as the supported path. The AC is the contract
(evidence-carrying, skippable, loud), not the classifier's behavior — see NON-GOAL above.
