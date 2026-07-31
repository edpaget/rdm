# Refuter batching — one refuter per dimension, or one per finding?

Sibling of [`refuter-model-tiering.md`](refuter-model-tiering.md): that document
A/Bs the refuter's **model**, this one A/Bs its **shape**. Both run on the same
instrument (`scripts/lib/refuter-agreement.mjs` +
`scripts/run-refuter-agreement.mjs`) over the same adjudicated finding corpus,
and both are gated by `scripts/verify-refuter-agreement.sh`.

## The question

Stage 2 of `buildReviewPipeline` (`.claude/workflows/lib/review.mjs`) dispatches
**one refuter agent per gating finding**. Each pays the measured ~38.8k-token
agent context floor and a cold cache to grade a single item. Grading a whole
dimension's findings in ONE agent pays that floor once, and is arguably *better*
review: a batched refuter sees its dimension's whole finding set and can notice
that two findings are the same defect, or that one is refuted by evidence
attached to another — the failure mode Cognition's "Don't Build Multi-Agents"
names in its first principle.

The risk is **anchoring**: a refuter that refutes the first finding may drift
toward refuting the rest. That is an empirical question, and this phase exists to
measure it rather than assert it either way. False negatives (a real defect
wrongly refuted → **ships a defect**) and false positives (a non-defect kept →
**costs a rework round**) have asymmetric cost and structurally different
denominators, so they are reported separately and never averaged.

## Method

### Corpus power

**A batched arm assembled mostly from size-1 "batches" is not evidence.** Before
any dispatch, the corpus must be shown to contain enough real multi-finding
batches for anchoring to be observable at all. That analysis is a first-class,
zero-spend step of the instrument:

```bash
node scripts/run-refuter-agreement.mjs --batch-power
```

**The grouping key must carry the review-unit identity.**
`buildReviewPipeline` is invoked **once per review unit** (one dispatch-phase
code-review stage, one plan-review phase unit) — never once per workflow run. A
real batched dispatch is therefore exactly *one review unit's gating findings for
one dimension*, and the key is:

```
runId | unitIdent | mode | dim.key
```

`unitIdent` is the first line of the corpus item's already-extracted `target`,
accepted only if it passes phase 1's plausibility rule (non-empty, contains no
`{` and no double-quote, ≤ 200 chars). Plan-mode targets are
`phase <roadmap>/<stem>` followed by a blank line and the body, so the first line
*is* the identity; code-mode targets are a single bare line. The
`--implementation-plan` shape is raw pretty-printed JSON and is **rejected**, its
item excluded and counted, never bucketed into a fake unit. This is the identical
policy `refuterFanout.refuterCountsByUnit` used, so the two measurements stay
comparable. (The predicate is deliberately restated inside
`scripts/lib/refuter-agreement.mjs` rather than imported from
`scripts/measure-refuter-severity.mjs`, which is a CLI and would invert the
dependency; § 2c-equivalence of the harness imports **both** and asserts they
agree on every committed item, so the two copies cannot drift.)

Three exclusions are applied before grouping, each separately counted:

| exclusion | why |
|---|---|
| `provenance.kind === 'constructed'` | no run id and no real review unit — cannot belong to any dispatch a production run could produce |
| severity in `HISTORICAL_ONLY_SEVERITIES` (`suggestion`) | never dispatched to a refuter since `workflow-token-reduction` phase 6, so never a batch member |
| unrecoverable unit identity | excluded and counted, never bucketed into a fake unit |

**Pre-registered floors**, fixed before the run:
`MIN_BATCH_GROUP_SIZE = 3`, `MIN_QUALIFYING_BATCH_GROUPS = 6`,
`MIN_QUALIFYING_BATCH_ITEMS = 18`. Their derivation: at the floor the bounded run
is 6 batched dispatches × 2 replicates (12) + 18 per-finding dispatches × 2
replicates (36) = **48 dispatches**, just under the ~55 ceiling the recorded
533k-tokens-per-Opus-dispatch mean allows.

Verbatim output against the committed corpus, at tree `03f55f0`:

```
Batch-size distribution the corpus can actually form (UNIT-SCOPED key: runId|unitIdent|mode|dim)

  corpus items                 56
- constructed (no run/unit)    12
- non-gating (suggestion)       4
- unrecoverable unit identity  5
= groupable items              35  in 29 group(s)

Size histogram (size:groups)   1:24, 2:4, 3:1
  code                        1:16, 2:4, 3:1
  plan                        1:8

Minimum group size for the anchoring measurement: 3
Size-1 groups are EXCLUDED from it (24 item(s) in singleton groups).
Qualifying population: 1 group(s) / 3 item(s) against floors of 6 group(s) / 18 item(s).
No group carries provenance.agentIndex, so every size below is an UPPER BOUND: a REWORK re-review is a second dispatch this key cannot yet split apart.

POWER: INSUFFICIENT
A batched arm built from this population is byte-for-byte a per-finding arm across most of its items, so the anchoring effect would be unobservable. This is a NO-MEASUREMENT outcome, not a passing gate.
```

The single qualifying group is
`wf_c8d0fac2-6d9 | workflow-token-reduction/phase-1-token-measurement-harness | code | correctness`.
**Plan mode forms no group above size 1 at all** — 8 groupable plan items, all
singletons.

#### The superseded naive key, and why it is void

An earlier framing grouped by `(runId, mode, dim.key)`. Under that key the same
56 items report **36 groups {1:24, 2:8, 3:1, 4:2, 5:1}** — apparently four groups
of size ≥ 3. Those figures are **void**, for two independent reasons:

1. **Two of the four are an artifact of the 12 `constructed` items**, which carry
   no `runId` and therefore collapse into one pseudo-run
   (`|code|correctness` size 5, `|code|architecture` size 4). They are not
   dispatches at all.
2. **The only non-constructed size-3 group is not one unit.**
   `wf_ff29f56e-da1|code|tests` holds 2 items targeting
   `unify-code-review/phase-4-workflow-lane-omits-done-trailer` and 1 targeting
   `unify-code-review/phase-5-rdm-review-standalone-shim` — unit-scoped, a pair
   plus a singleton. Measured directly: **4 of the corpus's 19 mined `runId`s
   span more than one target.**

This is the same coarse key phase 1 already measured and **rejected** for
`refuterFanout` (run `wf_55af7324-87c`: 96 refuters at one `phaseIndex` across 9
distinct review units). Under the corrected key the qualifying population is
**1 group / 3 items, not 2 groups** — materially worse than the naive figures
suggested. Both histograms are recorded in
[`docs/token-baseline.json`](token-baseline.json) §
`refuterBatching.corpusPower`, the naive one under `supersededNaiveKey`.

### Mining headroom — what a future attempt must buy

The shortfall is a *corpus* shortfall, not a source-data shortfall. Over the same
pinned window the sibling section uses (`--until 2026-07-29T00:00:00Z`, 953
recovered refuters), the miner's new unit-scoped grouping reports:

```bash
node scripts/mine-refuter-corpus.mjs --until 2026-07-29T00:00:00Z --min-group-size 3 \
  --exclude-corpus tests/fixtures/refuter-agreement/corpus.jsonl \
  --format json --out /tmp/batch-candidates.json
```

| figure | value |
|---|---:|
| qualifying (size ≥ 3) unit-scoped groups available | 82 |
| items in them | 298 |
| of those, **plan-mode** groups / items | 69 / 253 |
| groups already holding ≥ 1 adjudicated corpus member | 13 |
| adjudicated items already inside those 13 groups | 18 |
| **further adjudications needed to clear the floor** | **8** (completing 5 groups → 6 groups / 18 items) |

Mining must target **plan mode**: phase 1's `findingsPerFinder` puts plan finders
at p50 2 / p90 4.4–5 / max 7 findings versus code finders at p50 0–1 / max 3, and
`refuterCountsByUnit` at p50 8.5 refuters per unit — so multi-finding
per-dimension groups exist in volume only there, which is exactly where the
committed corpus is emptiest.

### The batched shape (built, gated, unexercised on real dispatches)

`buildBatchPrompt` is a **minimal delta** from the real `refutePrompt`: same
READ-ONLY stance, same "NOT a real issue unless the code/plan proves otherwise"
sentence. The only change is that the dimension's whole gating finding set is
rendered as one JSON array with an explicit `refute_id` per entry, and the
closing instruction asks for `{ verdicts: [{ id, refuted, confidence, rationale }] }`
with one entry per `refute_id`. Any wording change beyond that would be a
confound — the A/B varies shape, and shape only.

`refutePrompt` and `VERDICT_SCHEMA` are **byte-unchanged**: all 56 recorded
`promptSha256` values were taken against the current text, and § 3 of the harness
fails on drift. The batched prompt is a sibling, never a mutation.

Three resilience rules are implemented in `expandBatchResults` /
`parseClaudeBatchResult` and gated:

- a verdict for an id the dispatch did **not** contain is dropped and recorded
  under `unknownVerdictIds`;
- an id the response **omits** keeps `verdict: null` (ungraded) — never coerced
  to `refuted: false`, which would silently inflate the false-positive rate;
- a **crashed or malformed** dispatch (including one with no `verdicts` array at
  all) leaves every one of its ids ungraded, exactly like a crash — never read as
  "every finding omitted".

## Decision rule

Fixed **before** any dispatch. SHIP the batched shape if and only if **all four**
hold:

1. `POWER: SUFFICIENT` under the unit-scoped key.
2. The batched arm's **authoritative-only false-negative count** over the same
   items exceeds the per-finding arm's by at most 1, and its FN **rate** by at
   most 10 points.
3. `allSameVerdictShare` for the batched arm is at most 25 points above the
   per-finding arm over the same qualifying groups, **and**
   `refutationRateByPosition` shows no monotone rise after position 1.
4. `meanTokensPerGradedFinding` is materially lower.

The false-positive rate is reported and weighed, but is explicitly the **softer**
criterion — a rework round, not a shipped defect. **A pass on (4) alone is not a
ship.** Shipping on the token argument alone is not permitted.

The prohibition is mechanized, not asserted: `buildBatchTrials` **throws** on an
underpowered population, and `--allow-underpowered` stamps
`noMeasurement: true`, which forces `formatReport` to print a
`NO MEASUREMENT — batched arm was underpowered` banner and suppresses any
decision line.

## Results

Criterion 1 failed. No dispatch was made, so there are no arm figures to report —
that is the correct outcome, not a gap.

### False negatives

Not measured. The qualifying population (1 group / 3 items) is 5 groups and 15
items below the pre-registered floor, so a batched arm built from it would be
byte-for-byte a per-finding arm across 24 of 29 groups. Any FN comparison drawn
from it would reflect **dilution rather than evidence**.

### False positives

Not measured, for the same reason. FP is reported on a structurally different
denominator from FN and is never blended with it; the scorer's
`findBlendedAccuracyKeys` gate holds over the new anchoring block too.

### Self-consistency

Not measured. `scoreTrials` computes `selfConsistency.flipRate` per **bucket**
(`tier` for the per-finding arm, `tier|arm` for the batched one), so the two
shapes never share a flip denominator once a run is made.

### Anchoring

Not measured. `scoreAnchoring` is implemented and gated: over qualifying groups
only, it reports `allSameVerdictShare` per arm on the same group set and
`refutationRateByPosition` (position 1 versus positions 2..n, plus a
`risesAfterFirst` flag). A higher all-same share plus a rising by-position
refutation rate is the anchoring signature. Size-1 groups are excluded from it —
a size-1 "batch" is byte-for-byte a per-finding dispatch and can exhibit no
anchoring, so including it would dilute the very effect being measured.

### Token volume

Not measured. `cost.dispatches` (counted by unique `dispatchId`, so an expanded
batched row set is not read as N separate dispatches),
`cost.meanTokensPerDispatch`, `cost.gradedFindings` and
`cost.meanTokensPerGradedFinding` are implemented and audited. The token argument
alone could never have carried the decision in any case.

## DECISION

**`no-measurement`.**

The committed corpus cannot answer the question. The pipeline is **unchanged**:
`.claude/workflows/lib/review.mjs` still dispatches one refuter per gating
finding, the `workflow-token-reduction` phase 6 non-gating pass-through is
untouched, and nothing under `.claude/workflows/` or
`rdm-core/src/templates/workflows/` differs by a byte. This is the outcome the
phase names as legitimate: *"An A/B whose qualifying batch population failed step
1's minimum is not a result — it is a no-measurement outcome, and must be
reported as one rather than as a pass."*

The durable output is the **instrument**: the unit-scoped grouping and power
analysis, the batched prompt/trial/expansion/scoring/anchoring code, the two
miner flags, `provenance.agentIndex` stamping, and the harness sections and
planted mutations that gate all of it. `scripts/verify-refuter-agreement.sh`
additionally enforces the decision/pipeline XOR: while
`refuterBatching.decision !== 'ship-batched'`,
`.claude/workflows/lib/review.mjs` must contain neither `batchRefutePrompt` nor
`BATCH_VERDICT_SCHEMA`, so a half-landed pipeline change cannot coexist with a
no-ship decision.

## Limitations

- **Group sizes are an UPPER BOUND.** Within one `(runId, unit, mode, dim)` a
  REWORK re-review is a **second** dispatch that the four-part key silently
  merges. No committed item carries an ordering field, so nothing splits them
  today. Newly mined items now carry `provenance.agentIndex`, and
  `groupCorpusForBatching` splits a group at a discontinuity (`ROUND_SPLIT_GAP`)
  once every member of a group has one — reported as `roundSplits`. The gap
  threshold is an explicit heuristic, not a measurement.
- **The saving is chiefly plan-mode, not a uniform win.** Code finders sit at p50
  0–1 findings and max 3, so a code-mode batch is frequently size 1 and saves
  nothing. Any future write-up must say this rather than quoting a blended
  per-finding saving.
- **Post-ship runs become unminable.** `scripts/measure-refuter-severity.mjs` and
  `scripts/mine-refuter-corpus.mjs` recover dimension, severity **and** unit
  identity by parsing a SINGLE-finding prompt header. If batching ever ships,
  both `nonGatingRefutationSkip` and `refuterFanout` lose their per-finding basis
  for new runs, and the corpus stops growing from production. Tracked as rdm task
  `batched-refuters-break-corpus-mining` — a **pre-condition on shipping**, not a
  bug today.
- **The agent-label vocabulary would change** from `refute:<mode>:<findingId>` to
  `refute:<mode>:<dimKey>`. `verify-workflow-dispatch.sh`'s `JUDGMENT_LABELS`
  prefix list still matches, but any downstream analysis keyed on the id suffix
  does not.
- **Phase 5 interaction.** `phase-5-collapse-always-on-finders` collapses plan
  finders into one agent, which changes the `(runId, unitIdent, mode, dim)`
  structure this power analysis and the batched shape are keyed on. A corpus
  mined after that phase lands is not directly comparable with the figures here,
  and the decision rule would need re-registering.
- **Adjudication is a separate, evidence-citing pass.** The 8 further
  adjudications the floor needs must each read the cited location at a pinned
  tree and record `groundTruth {defect, class, authority, evidence,
  adjudicatedAgainstCommit}`. The historical verdict is circular and is never
  ground truth. Growing the corpus also invalidates
  `refuterModelTiering.corpus`'s counts and derived shares, the Composition table
  in [`refuter-model-tiering.md`](refuter-model-tiering.md), and the composition
  floors in `scripts/verify-refuter-agreement.sh` § 2 — all of which must be
  regenerated in the same commit. The `tiers` rows are per-item trial figures and
  must not be recomputed. Tracked as rdm task `complete-refuter-batching-ab`,
  which carries the exact 8-item target and the bounded run command.
