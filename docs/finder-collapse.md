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

> `CLAUDE.md` described this set as `coherence`/`architectural-fit` always-on for
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
  incomplete-always-on-lenses=93 no-findings-output=1 no-transcript=8
  not-an-always-on-lens=16 plan-doc-below-floor=19 unrecognized-target-type=8
  unrecoverable-unit-identity=231
```

Every count is in **finder records, not units**, and the accounting closes
exactly: 16 units × 3 always-on lenses = 48 recovered, plus 376 skipped, is 424.
Nothing is silently dropped — an unrecovered finder is an unknown, and an unknown
must never contribute to a rate. (`not-an-always-on-lens` is the `unit-of-work`
finder of a unit that WAS recovered: neither arm uses it, so it is counted rather
than vanishing.) The identity holds under `--limit` as well: a unit past the
limit is classified into a single `beyond-limit` bucket **before** any other
test, so truncating a run never drops the boundary unit's always-on records — or
any later unit's — into no bucket at all while the limit-independent
`finderRecordCount` keeps reporting them.
`unrecoverable-unit-identity=231` is overwhelmingly the
`--implementation-plan` shape, correctly rejected. The corpus is 16 units:
**15 `phase`, 1 `task`, 0 `roadmap`**.

#### Why `MIN_PLAN_DOC_CHARS = 500`, and why no roadmap-body unit qualifies

Five units (19 finder records) were excluded as `plan-doc-below-floor`. Four are
`roadmap <slug> (body)` units and one is a `task` unit; in every case the "plan
document" they were reviewed against is a **fetch-status line**, not the item's
body:

```
roadmap regularize-mechanical-agents (body)

Successfully fetched roadmap regularize-mechanical-agents with all phase details from the rdm project.
```

```
roadmap project-agnostic-lane (body)

ROADMAP_TARGET successfully fetched.
```

```
task task/fix-plan-review-gate-tag-clobber

fix-plan-review-gate-tag-clobber task fetched successfully
```

Reviewing those measures nothing about lens dilution — both arms would find
nothing in either. They are excluded and counted rather than padding the corpus.
The floor sits in the empty band between the largest degenerate target (**102**
chars) and the smallest real plan document in the window (**783**); no unit in
the window falls between them, so the threshold is not a tuned parameter. The
consequence is that the population spans `phase` and `task` units only, which is
why
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

Pre-registration commit: **`65baf92`** — *"feat(scripts): pre-register the
collapsed plan-finder A/B"*. It added this document with the Decision rule
section above, the six-criterion `DECISION_RULE` constants, and the whole
instrument, and changed nothing in the review pipeline. Every figure below was
produced after it by

```bash
node scripts/run-finder-collapse.mjs --dispatch --replicates 2 --run-units 8 \
  --model opus --concurrency 8 --out tests/fixtures/finder-collapse/trials-opus-r2.json
```

and is replayable with **zero spend** from the committed trials file via
`--score`. The machine-checkable copy is in
[`token-baseline.json`](token-baseline.json) § `planFinderCollapse`, audited
corpus-free by `--audit`.

Run: **8 review units** (7 `phase`, 1 `task`), **2 replicates**, `opus` in both
arms, **64 paid dispatches** (48 arm A, 16 arm B). `POWER: SUFFICIENT`.

Three trials returned no `findings` array and are excluded from their lens's
observation count: **2 in arm A** (one of them the `architectural-fit` finder on
`phase-7-prefix-workflow-engines`) and **1 in arm B**. The error bias therefore
runs *against* overstating arm B's shortfall.

### Per-lens findings

Mean findings per unit-observation, and the severity distribution
(blocking / concern / suggestion). Never blended across lenses.

| lens | arm A mean | arm B mean | arm A b/c/s | arm B b/c/s |
|---|---:|---:|---|---|
| coherence | 2.69 | 2.40 | 14 / 23 / 6 | 15 / 17 / 4 |
| architectural-fit | 2.33 | 0.53 | 22 / 13 / 0 | 4 / 4 / 0 |
| restraint | 2.13 | 0.87 | 2 / 19 / 11 | 0 / 11 / 2 |

Arm A produced **110** findings over 46 lens-observations; arm B produced **57**
over 15 collapsed observations. The shortfall is **not uniform**: `coherence`
survives the collapse nearly intact (−11 %), while `architectural-fit` falls to
**23 %** and `restraint` to **41 %** of the per-observation rate. The collapsed
agent keeps reviewing the lens the plan document most obviously invites and
quietly drops the other two.

The live arm-A per-lens means (2.69 / 2.33 / 2.13) sit inside the recorded
`findingsPerFinder` distribution [`token-baseline.json`](token-baseline.json)
§ `refuterFanout` reports for plan finders (p50 2, p90 4.4–5, max 7), so the live
arm A is not an outlier against the historical three-finder shape.

### Adjudicated material findings (replicate 1)

Coverage **83/83** — every replicate-1 finding in both arms was adjudicated
against tree `65baf92`.

| lens | arm A | arm B | loss | loss % | arm A blocking | arm B blocking |
|---|---:|---:|---:|---:|---:|---:|
| coherence | 22 | 18 | 4 | 18.2 % | 8 | 8 |
| architectural-fit | 15 | 4 | 11 | 73.3 % | 10 | 2 |
| restraint | 17 | 7 | 10 | 58.8 % | 1 | 0 |

Adjudicated material `blocking` findings in total: arm A **19**, arm B **10**.

**Every adjudicated finding in BOTH arms was judged material** — each cited a
checkable location and named a defect the plan document actually exhibits. That
is itself the sharpest result here: *arm B's findings are not worse, there are
simply far fewer of them per lens.* Precision is not the discriminator; recall
is. Arm B also produced **7** material findings arm A missed entirely (for
example that the truncated `diffText` a driver passes cannot support the
"content was read" inference, and that a prescribed guard would turn an existing
harness section red), against **23** the other way — so the collapsed finder is
not merely a subset of arm A, it is a smaller and differently-sampled subset.

### `concern` attribution

57 arm-B findings across both replicates, **57** carrying a valid lens key —
**100.0 %**, with zero claiming `unit-of-work`. The explicit enum instruction and
the `unit-of-work` prohibition both held on every single finding. Attribution is
emphatically not what fails here, and the remap guard the ship path would have
needed was never exercised by a real hallucination.

### Tokens

| | arm A | arm B |
|---|---:|---:|
| dispatches | 48 | 16 |
| output | 531 608 | 259 486 |
| uncached input | 747 | 1 753 |
| cache write | 2 258 183 | 935 627 |
| cache read | 24 343 964 | 10 995 190 |
| **mean per unit-observation** | **1 695 906** | **762 004** |
| mean **input** per dispatch | 554 227 | 745 786 |

Mean tokens per unit-observation fall **55.1 %**. The last row is the one that
matters for reading that number correctly: arm B's mean input per dispatch is
**34.6 % HIGHER** than an arm-A finder's, because the merged prompt carries all
three lenses. The saving is entirely the two eliminated agent context floors, not
a cheaper prompt — exactly the accounting this document pre-registered, and the
reason the figure is normalized to a unit-observation rather than a dispatch.

### The six criteria

| # | criterion | verdict |
|---|---|---|
| 1 | `POWER: SUFFICIENT` | **PASS** |
| 2 | per-lens material recall within 1 finding and 15 pp | **FAIL** — coherence −4 (18.2 %), architectural-fit −11 (73.3 %), restraint −10 (58.8 %); all three lenses fail independently |
| 3 | no lens extinction | PASS — no lens reached zero |
| 4 | material `blocking` within 1 | **FAIL** — arm A 19, arm B 10 (shortfall 9) |
| 5 | attribution validity ≥ 95 % | PASS — 100.0 % |
| 6 | tokens ≥ 20 % lower | PASS — 55.1 % |

## DECISION

**`no-ship`.** Copied from `scoreCollapse`'s output, not hand-reasoned.

Criterion 2 fails in **all three lenses independently**, and criterion 4 fails
alongside it. This is not a marginal per-lens miss that more replicates would
resolve: `architectural-fit` loses **73 %** of its adjudicated material findings
and **8 of its 10** material blockers, and `restraint` loses **59 %**. Even
`coherence`, the lens that survives best, misses the −1-finding tolerance by
four. The dilution the phase set out to measure is real and large.

Criteria 5 and 6 both pass handsomely — attribution is perfect and the token
saving is 55 % — and that is precisely the trap the pre-registered rule exists to
close. **A token pass is never a ship.** The 437.1M-token find class stays as it
is.

The pipeline is therefore **unchanged**: `.claude/workflows/lib/review.mjs` still
runs three always-on plan finders, `DIMENSIONS.plan` still holds
`coherence, architectural-fit, unit-of-work, restraint`, and nothing under
`.claude/workflows/` or `rdm-core/src/templates/workflows/` differs by a byte
except the two prose corrections this phase also owed — the `//|code|` non-merge
rationale, and the `//|plan|` `concern:` enum gaining `restraint`. (`review.mjs`
additionally exports `PLAN_SEVERITY_CALIBRATION` from its Node-only export list,
which sits OUTSIDE every marker, so the instrument can inject the real
calibration paragraph into arm B instead of copying it; no stamped byte and no
rendered skill line changes as a result.) This is the
outcome the phase names as legitimate: *"A collapsed finder that materially loses
findings in any one lens is a legitimate terminal negative: document it and leave
the pipeline unchanged."*

The durable output is the **instrument**: the plan-review-unit miner, the two-arm
trial builder, the collapsed prompt, the per-lens scorer with its six-criterion
table and its adjudication-coverage gate, the corpus-free `--audit`, and the
hermetic harness with its planted mutations and the decision/pipeline XOR.

### What a future attempt would have to change

Not the attribution, and not the token argument — both already pass. It would
have to change the *prompt*, and re-register:

- force a **per-lens section in the output** (an agent asked for three labelled
  lists cannot silently return one lens's worth of findings), or
- run the three lenses as **sequential passes inside one agent**, paying one
  context floor but three reasoning passes.

Either is a materially different candidate from the minimal delta measured here,
and would need its own pre-registration and its own run. **This result refutes the
cheap collapse, not every possible collapse.**

## Limitations

- **Adjudication covers replicate 1 only** (both arms, in full — 83/83).
  Replicate 2 supplies the mechanical measures. Criterion 2's margins here
  (−4 / −11 / −10 against a tolerance of −1) are far too large for a
  replicate-2 adjudication to flip, but a *marginal* future result would need
  both replicates adjudicated before it could be recorded as a decision. The
  scorer refuses to let this be skipped: criteria 2–4 **cannot pass** while
  `adjudicationCoverage.complete` is false, so an empty adjudication produces
  `no-ship`, never a vacuous `ship-collapsed`.
- **Materiality was judged by one adjudicator**, against the plan text and the
  repo at `65baf92`. Both arms were judged by the same standard and in the same
  pass, and every finding in both arms cleared it — so an adjudicator bias would
  have to be *lens-specific* to manufacture this result, which is not a shape a
  single standard produces.
- **No roadmap-body unit is in the population.** Every `roadmap <slug> (body)`
  unit in the pinned window was dispatched with a fetch-status line instead of
  the roadmap body, so all five fall below `MIN_PLAN_DOC_CHARS`. The measurement
  is scoped to `phase` and `task` documents. The underlying fetch defect is filed
  as the rdm task `plan-review-roadmap-body-fetch-status-line`; it is a real
  plan-review coverage hole independent of this experiment.
- **One model, one window.** Both arms ran on `opus` inside one session window.
  A different model might hold three lenses better; nothing here licenses a claim
  about any other tier.
- **Three dispatches returned no findings array** (arm A 2, arm B 1) and were
  excluded from their lens's observation count rather than read as "clean". Two
  of the three are arm A, so the error bias runs against arm B's shortfall being
  overstated, not for it.
- **The corpus is this repo's own plan documents only**, obtained from recorded
  prompts, never by reading files under `$RDM_ROOT` directly.
- **Token figures are for this corpus's document sizes** (783–21 518 chars). A
  much smaller document shifts the ratio between the context floor and the
  document, and with it criterion 6's margin — though not criterion 2's, which is
  what decided this.
