# Collapsing the always-on plan finders — three agents, or one with three lenses?

Sibling of [`refuter-batching.md`](refuter-batching.md) and
[`refuter-model-tiering.md`](refuter-model-tiering.md). Those two A/B the
**refute** half of `buildReviewPipeline`; this one A/Bs the **find** half. All
three follow the same discipline: pre-register a decision rule, measure, copy the
DECISION line out of the instrument, and mechanize a decision/pipeline XOR so a
half-landed change can never coexist with a no-ship figure.

Instrument: `scripts/lib/finder-collapse.mjs` + `scripts/run-finder-collapse.mjs`
+ `scripts/mine-plan-finder-corpus.mjs`, gated by
`scripts/verify-finder-collapse.sh`. Figures live in
[`docs/token-baseline.json`](token-baseline.json) § `planFinderCollapse`.

## The question

`plan` mode's `DIMENSIONS` list (`.claude/workflows/lib/review.mjs`) has **three
always-on dimensions** — `coherence`, `architectural-fit` and `restraint`. Only
`unit-of-work` is triggered (`when: (s) => s.targetType === 'phase'`). Every plan
review therefore dispatches three finder agents unconditionally, each paying its
own agent context floor, and **all three resolve the same `FINDINGS_SCHEMA`** —
so they are mergeable into one finder with no schema change.

> `CLAUDE.md` described this set as "coherence/architectural-fit always-on" for
> some time, omitting `restraint`. That prose was stale — `restraint` carries no
> `when` predicate and has always been always-on. Corrected in the same commit as
> this document.

The multiplier is largest where it is safest. `plan-review.js` fans out per unit
with `parallel()` under `--roadmap`, and `buildReviewUnits` emits the roadmap's
own body as a unit **in addition to** one per phase — so a five-phase roadmap is
six units and **eighteen** always-on plan finders before a single triggered
dimension or refuter. The reference run `wf_55af7324-87c` bears this out: 9 units
× 4 dimensions = 36 find agents. Collapsing the three always-on lenses into one
finder takes eighteen to six.

The risk is **dilution**: one agent asked to hold three review lenses may find
less per lens than three agents each holding one. That is an empirical question,
and this document exists to answer it rather than assert it. **A material
per-lens loss is a legitimate terminal negative** — the phase names it as such —
and ends the work with the pipeline byte-unchanged.

### What is deliberately NOT merged

`code` mode's two always-on dimensions, `ac` and `correctness`, are **not**
symmetric with plan mode's three and are not candidates for this merge. The
reason is recorded in the `//|code|` spec prose in
`.claude/workflows/lib/review.mjs`, so it renders into the shipped skill
templates rather than living as tribal knowledge, and in long form in
[`workflow-schemas.md`](workflow-schemas.md) § "Dimensions and `when` triggers".
In short: `ac` is the ONE dimension resolving `AC_REVIEW_SCHEMA` instead of
`FINDINGS_SCHEMA`, and its `ac` table is the structured side-channel
`classifyOutcome` step 2 consumes directly via `acTableHasGap` — a channel that
never reads finding severity, is never refuted, and is never budgeted. Merging it
into a shared findings stream would route the acceptance-criteria contract
through exactly the path it was deliberately kept out of, and would force a union
schema on the merged agent. Plan mode's three lenses share ONE schema, which is
what makes them mergeable at all.

`unit-of-work` likewise stays its own triggered dimension. It is scoped to phase
units CONSUMER-SIDE by `stripNonPhaseUnitOfWork` in `plan-review.js` (which
filters on `f.concern === 'unit-of-work'`), and folding a conditionally-scoped
lens into the unconditional agent would defeat that scoping.

## Method

### Corpus

`scripts/mine-plan-finder-corpus.mjs` recovers real **review units** from the
full-fidelity `subagents/workflows/<runId>/agent-*.jsonl` transcripts. Plan-mode
`findPrompt` interpolates the target INLINE, so a finder's initiating user turn
carries the complete plan document; its assistant turns carry the complete
`StructuredOutput` findings. Both halves are replayable verbatim.

The unit boundary is `(runId, unitIdent)` — the identical boundary
[`token-baseline.json`](token-baseline.json) § `refuterFanout` documents and
`scripts/lib/refuter-agreement.mjs` § `unitIdentOf` implements, because
`buildReviewPipeline` is invoked once per review unit and never once per run. A
plan-mode target is `<kind> <identity>` followed by a blank line and the
document, so the first line IS the identity. The `--implementation-plan` shape is
raw pretty-printed JSON and is **rejected** (it contains `{` and `"`), its
finders excluded and counted, never bucketed into a fake unit.

**The recorded findings are not an adjudication.** They are what the three-finder
shape produced on that day, on that model. Scoring an arm against a verdict the
same shape produced would be circular, so the miner assigns no ground truth. The
recorded arm-A findings are used for exactly two things: the power analysis, and
a validity cross-check against the LIVE arm-A distribution.

Pinned window: `--until 2026-07-31T00:00:00Z`, this repo's own project slugs, the
six lane workflows. Verbatim mining output at that window:

```
mine-plan-finder-corpus: 16 review unit(s) from 424 plan-finder record(s) — skipped:
  incomplete-always-on-lenses=31 no-findings-output=1 no-transcript=8
  plan-doc-below-floor=5 unrecognized-target-type=2 unrecoverable-unit-identity=231
```

`unrecoverable-unit-identity=231` is overwhelmingly the `--implementation-plan`
shape, correctly rejected. The corpus is 16 units: **15 `phase`, 1 `task`, 0
`roadmap`**.

#### Why `MIN_PLAN_DOC_CHARS = 500`, and why no roadmap-body unit qualifies

Five units were excluded as `plan-doc-below-floor`. Every one of them is a
`roadmap <slug> (body)` unit whose "plan document" is a **fetch-status line**,
not a roadmap body:

```
roadmap regularize-mechanical-agents (body)

Successfully fetched roadmap regularize-mechanical-agents with all phase details from the rdm project.
```

```
roadmap project-agnostic-lane (body)

ROADMAP_TARGET successfully fetched.
```

Reviewing those measures nothing about lens dilution — both arms would find
nothing in either. They are excluded and counted rather than padding the corpus.
The floor sits in the empty band between the largest degenerate target (147
chars) and the smallest real plan document in the window (783). The consequence
is that the population spans `phase` and `task` units only, which is why
`POWER_FLOORS.minTargetTypes` is **2** rather than 3: requiring 3 would make the
floor unmeetable for a reason that has nothing to do with lens dilution. This is
a scope limitation of the measurement and is recorded as one below; the
underlying fetch defect is filed separately as an rdm task.

### Power

Pre-registered floors, fixed before any dispatch (`POWER_FLOORS`):

| floor | value | why |
|---|---:|---|
| `minUnits` | 8 | below this, one unit's stochastic miss moves a per-lens count by more than the decision rule's own tolerance |
| `minReplicates` | 2 | finder output is stochastic; a one-shot A/B reads a random miss as a per-lens loss |
| `minTargetTypes` | 2 | see above |
| `requiredLenses` | all three | a lens that never fired in the population is untestable by construction |

`buildCollapseTrials` **throws** on an underpowered population.
`--allow-underpowered` stamps `noMeasurement: true`, which forces `formatReport`
to print a `NO MEASUREMENT` banner and **suppress the decision line** — the same
mechanism `buildBatchTrials` uses in the sibling instrument.

### The run population

Chosen deterministically and BEFORE dispatch, by `selectRunUnits`: sort every
qualifying unit by `id`, bucket by target type, then take units round-robin
across the buckets in `phase, task, roadmap` order until `n` are chosen.
Round-robin is what stops the single `task` unit being crowded out by the 15
`phase` units. No post-hoc selection, no clock, no RNG.

### The two arms

**Arm A** is the current shape: three separate finders, each prompt built by the
**REAL exported `findPrompt`** over the **REAL always-on `DIMENSIONS.plan`
entries**, imported from `.claude/workflows/lib/review.mjs`. The instrument never
carries its own copy of the production prompt — a drifted copy would measure a
strawman. `scripts/verify-finder-collapse.sh` § 3 gates that fidelity.

**Arm B** is the candidate: one finder built by `buildCollapsedPlanPrompt`, a
**minimal delta** from `findPrompt` — same READ-ONLY stance, same `Review target`
line, same plan-mode inspect hint, the same `PLAN_SEVERITY_CALIBRATION` paragraph
injected exactly once, the same evidence instruction, the same FINDINGS-schema
closing instruction. The only changes are the ones the merge makes necessary:

1. the single-dimension sentence becomes an enumerated list of the three lenses,
   carrying each lens's `title` and `focus` byte-for-byte;
2. an instruction to review every lens **independently** and not trade one off
   against another;
3. an instruction that each finding's `concern` MUST be exactly one of the three
   lens keys;
4. an instruction never to emit `concern: unit-of-work` (that lens belongs to a
   separate reviewer);
5. "return an empty `findings` array ONLY when ALL of the lenses are clean".

Any wording change beyond that would be a confound: the A/B varies the number of
agents, and that only.

`buildCollapsedPlanPrompt` lives in the **instrument** during the experiment, so
a no-ship decision leaves `.claude/workflows/lib/review.mjs` byte-unchanged. On a
ship decision it moves into `review.mjs`, the instrument imports it from there,
and the harness asserts the two renders are byte-identical.

### Replicates, model, and what is adjudicated

Both arms run on the **same model** in the **same session window**, with
**R = 2** replicates per unit per arm. Hand adjudication covers
**replicate 1 in full, for both arms**; replicate 2 supplies the variance and
stability figures on the mechanical measures (per-lens counts, severity
distribution, attribution validity, tokens). That scope is a deliberate bound on
hand work and is restated under Limitations.

Adjudication is written into
`tests/fixtures/finder-collapse/adjudication.jsonl`, one row per finding in
either arm:

```json
{"unitId":"…","arm":"A","replicate":1,"lens":"coherence","findingId":"…",
 "material":true,"matchedInOtherArm":true,"matchedFindingId":"…",
 "rationale":"…","adjudicatedAgainstCommit":"<sha>"}
```

A **paraphrase that names the same defect counts as matched**; a different defect
does not. Equivalence is never asserted from counts alone.

### What is counted, and how

Everything per-lens is reported **per lens and never blended** — a single
cross-lens mean is exactly what would hide an extinct lens behind two healthy
ones. `findBlendedLensKeys` gates that structurally over both the report and the
committed `planFinderCollapse` section.

Tokens are reported **by class** (output / uncached input / cache-write /
cache-read), and normalized to a **unit-observation** — one `(unit, replicate)`
pair, the thing a production run actually pays for. Reporting per-dispatch would
flatter arm B by construction, since it makes one dispatch where arm A makes
three. Mean input tokens per dispatch are reported separately, because the merged
prompt is longer than any single-lens prompt and part of the predicted saving can
evaporate into it.

## Decision rule

**Pre-registered before any paid dispatch**, in the same commit that added this
document — see the Results section for that commit. Encoded as constants in
`DECISION_RULE` (`scripts/lib/finder-collapse.mjs`) and evaluated mechanically by
`scoreCollapse`, which emits `decision` plus a per-criterion pass/fail table. The
DECISION line below is **copied from the instrument's output**, never
hand-reasoned.

SHIP the collapsed finder if and only if **ALL SIX** hold:

1. `POWER: SUFFICIENT`.
2. For **every lens independently**, arm B's adjudicated material-finding count
   is at most **1 finding** and at most **15 percentage points** below arm A's.
3. **No extinction**: no lens goes to zero material findings in arm B where arm A
   produced ≥ 2 across the corpus.
4. **No systematic severity downgrade**: arm B's adjudicated-material `blocking`
   count is at least arm A's minus 1.
5. Arm B's `concern` attribution is valid (∈ {`coherence`, `architectural-fit`,
   `restraint`}) on **≥ 95 %** of its findings.
6. Mean tokens per unit-observation are materially lower (**≥ 20 %**).

**Criterion 6 alone is NEVER a ship.** Shipping on the token argument alone is
not permitted, and the rule is mechanized as an AND over all six so it cannot be
reached by argument. `auditCollapseDoc` additionally rejects a committed
`ship-collapsed` decision whose only passing criterion is 6.

### The decision/pipeline XOR

`scripts/verify-finder-collapse.sh` § 9 enforces the invariant that a half-landed
pipeline can never coexist with a no-ship figure, in **both** directions:

- while `planFinderCollapse.decision !== 'ship-collapsed'`,
  `.claude/workflows/lib/review.mjs` must contain **no** merged-plan-dimension
  symbol (`PLAN_LENSES`, `lenses:`, `attributeConcern`, `lensDimFor`) and
  `DIMENSIONS.plan` must still hold exactly
  `coherence, architectural-fit, unit-of-work, restraint`;
- when it **is** `ship-collapsed`, the inverse must hold.

## Results

**PENDING.** Nothing has been dispatched yet. This section is filled in from
`scripts/run-finder-collapse.mjs`'s own output after the run; the DECISION line
below is copied from the instrument, never hand-reasoned, and no figure appears
here that the committed `tests/fixtures/finder-collapse/trials-*.json` cannot
reproduce under `--score`.

## DECISION

**PENDING** — see above. Until a decision is recorded, the decision/pipeline XOR
holds the pipeline byte-unchanged.

## Limitations

- **Adjudication covers replicate 1 only** (both arms, in full). Replicate 2
  supplies variance on the mechanical measures. A *marginal* result would need
  both replicates adjudicated before it could be recorded as a decision.
- **No roadmap-body unit is in the population.** Every `roadmap <slug> (body)`
  unit in the pinned window was dispatched with a fetch-status line instead of
  the roadmap body, so all five fall below `MIN_PLAN_DOC_CHARS`. The measurement
  is therefore scoped to `phase` and `task` documents. The underlying fetch
  defect is filed as an rdm task; it is a real plan-review coverage hole
  independent of this experiment.
- **One model, one window.** Both arms run on one model inside one session
  window. A different model might hold three lenses better; nothing here will
  license a claim about any other tier.
- **The prompt is a minimal delta, not an optimized one.** A materially different
  merged prompt (for example one that forces a per-lens section in the output, or
  runs three sequential passes inside one agent) is a *different* candidate, and
  would need its own pre-registration and its own run.
- **The corpus is this repo's own plan documents only**, obtained from recorded
  prompts, never by reading files under `$RDM_ROOT` directly.
