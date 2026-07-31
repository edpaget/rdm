# Refuter model tiering — the decision, and the harness behind it

## The question

Refuters run on the most expensive tier everywhere in the autonomous lane.
`rdm model resolve review-verify` returns `opus` at the default tier and at
`large`, and `.claude/workflows/plan-review.js` passes **no** models at all, so
its finders and refuters inherit the ambient session model — which is
opus-class. Refutation is also expensive in aggregate: `docs/token-baseline.json`
measures the refuter class at 19.6 % of all lane tokens.

Two things had to be settled:

1. **Can refuters move to a cheaper tier without shipping defects?**
2. **Is `plan-review.js`'s model omission deliberate policy or an oversight?**
   That question was itself contested — an Opus refuter, handed exactly that
   finding, refuted it by citing `f4e89d7` and
   `scripts/verify-workflow-review.sh` §5b-mechanical.

An initial ad hoc A/B (8 findings × {Opus, Sonnet} × 2 replicates) had produced a
suggestive but unusable result: 7/8 majority agreement, no within-model flips,
and — decisively — **Sonnet was not cheaper in tokens** (52.9k mean vs Opus
48.4k). Its one disagreement was the expensive failure mode: a
mechanically-true-but-not-a-defect finding that Opus refuted after 13–20 tool
calls and Sonnet kept after 7–9. Eight findings is not a basis for changing a
gate, so this phase built a reusable instrument instead.

**A measured "keep Opus, change nothing" is a successful outcome of that
instrument, not an unfinished phase.** See [DECISION](#decision).

## Method

### Corpus construction

The corpus is `tests/fixtures/refuter-agreement/corpus.jsonl` — 56 items, one
JSON object per line.

**Mined items (44, 78.6 %) are the primary source.**
`scripts/mine-refuter-corpus.mjs` walks the session sidecars with phase 1's
parser (`defaultProjectsRoot` / `locateSessionDirs` / `findWorkflowRunFiles` /
`buildRecords` / `transcriptPathFor` from `scripts/lib/token-report.mjs`),
filters to `agentClass === 'refute'` over the same six lane workflows
`docs/token-baseline.json`'s `runSet` uses, and reads each refuter's
**full-fidelity `subagents/workflows/<runId>/agent-*.jsonl` transcript**. The
initiating user turn carries the complete `refutePrompt`; the assistant turns
carry the complete `StructuredOutput` verdict. Historical findings are therefore
replayable **verbatim**.

> The 401-character hard truncation applies only to the `promptPreview` /
> `resultPreview` fields of the `wf_*.json` sidecars. It does **not** apply to
> the transcripts. `scripts/verify-refuter-agreement.sh` §3 asserts every mined
> prompt exceeds 401 characters precisely to prove the transcript, not the
> preview, was the source. The median mined prompt is ~9.4k characters.

Over the mining window, 987 refuter records yielded **953 recoverable** ones; 26
had no readable transcript and 8 returned no recoverable verdict. Those 34 are
bucketed with counts and contribute to nothing.

Recovery reuses the already-exported, brace-matched, sentinel-anchored
`matchBrace` / `extractFinding` from `scripts/measure-refuter-severity.mjs`
rather than re-implementing them, so the two instruments cannot drift. This
matters: `refutePrompt` interpolates the target **inline** on the header line,
and the `--implementation-plan` plan-review target is itself a pretty-printed
JSON document — so a naive `indexOf('{')` finds the *target*, and reading "the
rest of the first line" *truncates* a multi-line target. Both hazards are
covered by fixtures in §4 of the harness.

All 953 recovered candidates regenerate **byte-identically** through the real
`refutePrompt` imported from `.claude/workflows/lib/review.mjs`.

**Constructed items (12, 21.4 %) top up only under-covered classes.** Real
production history under-covers three classes: `misread-scope` and
`false-premise` are rare, and — because the repo fixes its defects — most
historically-correct findings are no longer defects in the tree a replay reads.
Each constructed item records `provenance.kind: 'constructed'`, the commit it was
built against, and why it exists.

### Adjudication

**The historical verdict is never ground truth.** Every run on disk was refuted
by an opus-class model; scoring Opus against its own past verdicts would be
circular and would manufacture ~100 % agreement for the baseline tier. The miner
emits `groundTruth: null` on every record and files the historical verdict under
`provenance.historicalVerdict` as a *candidate signal for adjudication only*.

Ground truth was assigned by a separate, evidence-citing pass. For each item:
read the cited location, decide `defect: true|false`, assign a class from the
closed set, and write an `evidence` string naming the concrete artifact that
settles it.

**Ground truth is adjudicated against a PINNED TREE** — recorded per item in
`groundTruth.adjudicatedAgainstCommit`, here `0359e3035d31`. That is the tree a
replay run actually reads, which is *not* the tree the historical refuter read. A
finding that was a real defect when raised and has since been fixed is therefore
`stale-fact` / `defect: false`, because that is the correct answer for a refuter
reading this tree. Fifteen items are exactly that.

The closed class set (`GROUND_TRUTH_CLASSES` in
`scripts/lib/refuter-agreement.mjs`):

| class | meaning |
|---|---|
| `real-defect` | the finding is correct and the code/plan is wrong |
| `mechanically-true-not-a-defect` | the stated fact holds, but a documented exception, deliberate design, or governing artifact means it is not a defect — **the divergence class** |
| `false-premise` | cites a file, symbol, or behavior that does not exist |
| `stale-fact` | was true once, superseded by a later commit |
| `misread-scope` | true somewhere, but not at the cited location |
| `style-preference` | a taste call dressed as a defect |

Each item also carries `groundTruth.authority`:

- **`authoritative`** — settled by a citable artifact (a changelog entry, a
  harness criterion, a commit, a directly-readable line of code). The
  authoritative-only rates are the **decision-grade** figures.
- **`judgement-call`** — a reasonable reviewer could disagree.

`validateCorpusItem` rejects an `authoritative` item whose evidence names no
concrete artifact, and the harness asserts that floor.

### Composition

| property | value | floor |
|---|---:|---:|
| items | 56 | ≥ 45 |
| `mechanically-true-not-a-defect` share | 42.9 % | ≥ 35 % |
| mined share | 78.6 % | ≥ 60 % |
| `authoritative` share | 76.8 % | ≥ 50 % |

| class | items | | authority | items | | provenance | items |
|---|---:|---|---|---:|---|---|---:|
| mechanically-true-not-a-defect | 24 | | authoritative | 43 | | mined | 44 |
| stale-fact | 15 | | judgement-call | 13 | | constructed | 12 |
| real-defect | 7 | | | | | | |
| false-premise | 5 | | | | | | |
| style-preference | 3 | | | | | | |
| misread-scope | 2 | | | | | | |

Severity: 7 `blocking`, 45 `concern`, 4 `suggestion`. Mode: 42 `code`, 14 `plan`.

The divergence class is **deliberately over-represented**, far above its natural
incidence, because that is where the two tiers disagreed. Every aggregate in
this document is therefore over a weighted corpus and is **not** a population
estimate of production finding mix — the per-class rates are the ones to quote.

The four `suggestion`-severity items are recorded as **historical-only** and are
excluded from every headline rate. Phase 6 landed
`NON_GATING_SEVERITIES = ['suggestion']`, so no refuter is ever spawned for one
again and no tiering decision can affect them.

### Replay

`scripts/run-refuter-agreement.mjs` regenerates every prompt through the **real**
`refutePrompt` imported from `.claude/workflows/lib/review.mjs` and compares the
result against the item's recorded `promptSha256`. A mismatch sets
`promptDrift` and is reported rather than silently accepted — that is what
catches a later `refutePrompt` edit invalidating the corpus. At the recorded
commit, zero items drift.

Each trial is one dispatch of `claude -p --model <tier> --output-format json`
with the regenerated prompt on stdin, run from the repo root so the refuter can
read the cited locations. The trial plan is `corpusId × tier × replicate`, in
that order; results are written into indexed slots, so bounded `--concurrency`
never changes the recorded order.

A response with no boolean `refuted` is recorded as `verdict: null` and bucketed
**`ungraded`** — never coerced to `false`, which would silently inflate the
false-positive rate.

## Decision rule

**Stated before the numbers, so it cannot be fitted to them.**

The two error types are not interchangeable:

- A **false negative** — a real defect wrongly refuted — *ships a defect*. It is
  the expensive error, and it is silent.
- A **false positive** — a non-defect kept — *costs one rework round*. It is
  visible and self-correcting.

The cheaper tier is adopted **only if all four hold**:

1. Its **authoritative-only false-negative rate** is no worse than the
   baseline's, within the resolution the corpus supports.
2. Its **authoritative-only false-positive rate** does not increase by more than
   10 percentage points — beyond that, the extra rework rounds cost more agent
   time than the price-per-token saving buys.
3. Its **replicate flip rate** is no worse than the baseline's. A tier that posts
   a good FN rate by coin-flip is not safer, it is lucky.
4. Its **per-class** rate on `mechanically-true-not-a-defect` — the divergence
   class — is not materially worse. That is the class the initial A/B failed on,
   and it is the class this corpus is weighted to measure.

If any of the four fails, the answer is **keep Opus**.

Note what the rule deliberately does *not* rest on: token volume. Re-tiering
changes **price per token**, not token volume, and this harness measures volume.
Any saving argument here is a price argument and is labelled as such.

## Results

Recorded run: label `bounded-8x2x2`, tiers `opus` (baseline) and `sonnet`,
2 replicates, over a stratified 8-item subset of the corpus (3 `real-defect`,
3 authoritative `mechanically-true-not-a-defect`, 1 `false-premise`,
1 `misread-scope`) = 32 dispatches. Raw trials:
`tests/fixtures/refuter-agreement/trials-bounded-8x2x2.json`.

> **Bounded-run limitation, stated up front.** The instrument supports the full
> corpus; this recorded run does not use it. A full
> `--tiers opus,sonnet --replicates 2` pass over all 56 items is 224 dispatches.
> At the means this run actually measured (533k tokens per Opus dispatch, 1.01M
> per Sonnet one — both dominated by boot and file reading), that is ~173M
> tokens. Spending that to decide a token-reduction question would be
> self-defeating. The subset is stratified to
> put items in the classes the decision rule names, and the resulting per-tier
> denominators are small — which is itself an input to the decision (see
> [Limitations](#limitations)). Reproduce or extend with:
>
> ```
> node scripts/run-refuter-agreement.mjs --tiers opus,sonnet --replicates 2 \
>   --concurrency 8 --out tests/fixtures/refuter-agreement/results-full.json
> ```

<!-- RESULTS-BEGIN -->
### False negatives — a real defect wrongly refuted → SHIPS A DEFECT
Denominator: defect-truth trials only. Authoritative-only is the DECISION-GRADE figure.
| tier | authoritative FN | judgement-call FN | all FN | output | uncached input | cache write | cache read | mean tokens/trial | mean tool calls/trial |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| opus | 1/6 (16.7%) | 0/0 (n/a) | 1/6 (16.7%) | 100,465 | 256 | 718,055 | 7,707,930 | 532,919.1 | 10.4 |
| sonnet | 2/5 (40.0%) | 0/0 (n/a) | 2/5 (40.0%) | 98,007 | 432 | 679,527 | 15,352,317 | 1,008,142.7 | 12.6 |

### False positives — a non-defect kept → COSTS A REWORK ROUND
Denominator: non-defect-truth trials only. A DIFFERENT denominator from the block above.
| tier | authoritative FP | judgement-call FP | all FP | ungraded | mean tokens/trial | mean tool calls/trial |
|---|---:|---:|---:|---:|---:|---:|
| opus | 3/7 (42.9%) | 0/0 (n/a) | 3/7 (42.9%) | 3 | 532,919.1 | 10.4 |
| sonnet | 5/10 (50.0%) | 0/0 (n/a) | 5/10 (50.0%) | 1 | 1,008,142.7 | 12.6 |

### Self-consistency — same item, same tier, replicate disagreement
| tier | replicate pairs | flips | flip rate |
|---|---:|---:|---:|
| opus | 5 | 2 | 40.0% |
| sonnet | 7 | 2 | 28.6% |
A tier with a low FN rate but a high flip rate is not safer — it is lucky.

### Per-class breakdown — the aggregate above is over a deliberately weighted corpus
### opus
| class | FN (defect-truth trials) | FP (non-defect-truth trials) | ungraded |
|---|---:|---:|---:|
| false-premise | 0/0 (n/a) | 1/1 (100.0%) | 1 |
| mechanically-true-not-a-defect | 0/0 (n/a) | 2/5 (40.0%) | 1 |
| misread-scope | 0/0 (n/a) | 0/1 (0.0%) | 1 |
| real-defect | 1/6 (16.7%) | 0/0 (n/a) | 0 |

### sonnet
| class | FN (defect-truth trials) | FP (non-defect-truth trials) | ungraded |
|---|---:|---:|---:|
| false-premise | 0/0 (n/a) | 2/2 (100.0%) | 0 |
| mechanically-true-not-a-defect | 0/0 (n/a) | 3/6 (50.0%) | 0 |
| misread-scope | 0/0 (n/a) | 0/2 (0.0%) | 0 |
| real-defect | 2/5 (40.0%) | 0/0 (n/a) | 1 |

### Token volume vs baseline
- opus: baseline (532,919.1 mean tokens/trial, 10.4 mean tool calls/trial).
- sonnet: 1,008,142.7 mean tokens/trial vs 532,919.1 for opus — a delta of 475,223.6 (89.2%).
Re-tiering changes PRICE-PER-TOKEN, not token VOLUME. These are volume figures only.

### Standing caveats
- The corpus is DELIBERATELY WEIGHTED toward mechanically-true-not-a-defect (the class where the tiers diverged). The aggregate is therefore NOT a population estimate of production finding mix — quote the per-class rates.
- False negatives and false positives have asymmetric cost and structurally different denominators. They are reported separately and are never averaged into an accuracy number.
- Token figures here are VOLUME, not price. Re-tiering changes price-per-token, not token volume (the initial A/B measured Sonnet at 52.9k vs Opus 48.4k per refuter). Any cost conclusion is a price-per-token argument and must be labelled as such.
- Historical `suggestion`-severity items are recorded but excluded from every headline rate: phase 6 landed NON_GATING_SEVERITIES = [suggestion], so no refuter is spawned for one again.
- Ground truth is adjudicated against the PINNED TREE recorded in each item's groundTruth.adjudicatedAgainstCommit — the tree a replay run actually reads — not against the tree the historical refuter read. A finding that was real then and is fixed now is `stale-fact`/defect:false, because that is the correct answer for a refuter reading this tree.
<!-- RESULTS-END -->

## Limitations

- **The corpus is deliberately weighted.** `mechanically-true-not-a-defect` is
  42.9 % of items, far above its production incidence. Every aggregate here is a
  measurement of *this* corpus, not an estimate of production error rates. Quote
  the per-class rates.
- **The recorded run is bounded**, not the full corpus — 8 items × 2 tiers × 2
  replicates. Denominators are small enough that a single trial moves a rate by
  a large fraction, so only large differences are readable. The rule's
  thresholds were chosen with that in mind, and the tie-break is conservative:
  when the data cannot resolve a difference, keep the safer tier.
- **Ground truth is adjudicated against one pinned tree** (`0359e3035d31`) by a
  single operator. Thirteen of 56 items are explicitly `judgement-call`; the
  headline figures exclude them.
- **Historical verdicts are excluded by construction**, which is correct (they
  are circular) but means the corpus has no independent second grader.
- **Neither tier refuted the one `false-premise` item that ran** (Opus 1/1,
  Sonnet 2/2 kept). That item is `mined-wf_909dbdd4-a29-a7caffc6c9f254a77`,
  **mined**, and its false premise is one of *commit attribution*, not a missing
  file: the location it cites (`.claude/workflows/dispatch-phase.js:1503-1510`)
  exists, but its claim about which commit introduced it is false — `git show
  --stat 31be47a` touches exactly one file, `scripts/verify-workflow-dispatch.sh`,
  and no `.js` at all. Refuting it therefore required inspecting a commit's
  contents, not checking that a path exists. The result is about the *prompt*,
  not the tiers: a refuter told to "start from the stance that this is not a real
  issue" appears to treat an unverified premise as inconclusive rather than as
  disproof. It is the single most actionable observation in this run and it is
  orthogonal to tiering.
- **The easier existence-check case was never tested.** The two corpus items
  whose findings cite genuinely nonexistent files
  (`constructed-false-premise-tag-policy`,
  `constructed-false-premise-no-drift-gate`) fall outside the bounded subset and
  were not dispatched. The claim that a finding citing a missing file should be
  the *easiest* thing to refute is therefore untested here; establishing it
  requires running those items.
- **Four trials came back ungraded** (3 Opus, 1 Sonnet) — a well-formed response
  with no boolean `refuted`. They are bucketed separately and reach no rate, but
  they shrink already-small denominators, and the baseline tier produced more of
  them.
- **One repo, one operator, one historical window.** Findings are rdm's own, and
  rdm's review dimensions and prose are idiosyncratic.
- **Volume, not price.** Every token figure here is volume. The initial A/B
  already showed the cheaper tier spending *more* tokens (52.9k vs 48.4k) —
  per-agent cost is dominated by boot and file reading, so re-tiering cannot
  reduce the metric this roadmap measures. It can only change price per token.
- **`stale-fact` is a corpus artifact of a healthy repo.** Fifteen items are
  findings that were real when raised and have since been fixed. Under the
  pinned-tree rule they are correctly `defect: false`, but their presence means
  the corpus over-represents "the code moved on" relative to live review traffic.

## The `plan-review.js` model-omission question

**Verdict: oversight** — not deliberate policy.

### The mechanical facts

`.claude/workflows/lib/plan-review.mjs` calls `runPlanReview({ target })` at
**both** call sites — the persisted-unit path and the `--implementation-plan`
path — with no `findModel` and no `verifyModel` in the context object.
`buildReviewPipeline` in `.claude/workflows/lib/review.mjs` reads
`ctx.findModel` / `ctx.verifyModel` and passes them straight to the finder and
refuter `agent()` calls as `model:`. With the keys absent, both resolve to
`undefined`.

`docs/workflow-schemas.md` § "agent() options spike" settles what `undefined`
means, empirically, by reading the model each agent actually ran on out of its
transcript:

> | `model:` passed | Actually ran on | Conclusion |
> |---|---|---|
> | *(key omitted)* | `claude-opus-4-8` (session model) | inherits the session model |
> | `undefined` | `claude-opus-4-8` (session model) | **inert — identical to omitting the key** |

So plan-review's finders and refuters inherit the ambient session model.

The sibling consumer does the opposite. `.claude/workflows/dispatch-phase.js`
builds
`const reviewModels = { findModel: models.review_find, verifyModel: models.review_verify }`
and threads it into the same pipeline. Two consumers of one pipeline therefore
disagree about whether its judgment agents carry a model.

The consequence is not neutral, and it is not a saving. `rdm model resolve
review-find` returns **`sonnet`**; `review-verify` returns **`opus`**. Under
`dispatch-phase`, a plan-mode finder runs on Sonnet. Under `plan-review.js`, the
same finder inherits the opus-class session model. The omission makes
plan-review's finders **more** expensive than the configured policy, not less.

### Why the counter-argument does not cover this case

The refutation that contested this finding cited two artifacts. Both govern the
**mechanical** pin only.

**`f4e89d7` — "pin mechanical agents to the small tier across the autonomous
lane".** Its body reads, verbatim:

> Judgment agents (plan/implement/review, the estimate rater, plan-review's act
> step, backlog's analyzers, document's synthesis step) are left unpinned.

That sentence is scoped by its own commit. The same commit's `dispatch-phase`
half demonstrably *keeps* `review_find` / `review_verify` threaded, so "judgment
agents are left unpinned" cannot mean "judgment sites must carry no model" — it
means "this commit did not pin them to the **mechanical** tier". Reading it as a
general prohibition contradicts the commit's own diff.

**`scripts/verify-workflow-review.sh` §5b-mechanical.** Its negative assertion is:

```
assert_label_not_model "$TMP/mech-blocks" 'act:' '_mechanicalModel'
```

It asserts that `act:` is not pinned to `_mechanicalModel`. It says nothing about
any other model, and nothing about finders or refuters at all — which it could
not, because those are not `agent()` call sites in `plan-review.js`. They live
inside the stamped review block, dispatched by `buildReviewPipeline`.

**And a third artifact points the other way.** `CHANGELOG.md`'s entry on the
review fleet states that the skill surface now sizes review agents via the
`[models]` policy

> instead of inheriting the session's model … Every dispatched agent is now
> given an explicit `model`, closing the session-model-inheritance leak.

That is the same leak, described as a defect and closed elsewhere. It is still
open in `plan-review.js`.

**No artifact anywhere states that a judgment site should be left unpinned
entirely.** Absent such a statement, and with the sibling consumer, the config
policy, and the changelog all pointing the other way, the omission is an
oversight.

### Disposition

The tiering decision below is *keep Opus, change nothing*, so this fix is not
applied inline — changing a model binding here would be an unmeasured change
riding on a measured decision to change nothing. It is filed as a plan-repo
task: **`thread-plan-review-judgment-models`**.

Note that the fix is not simply "pin them to Opus". `review-find` resolves to
`sonnet`, so threading the configured policy would *lower* plan-review's finder
tier while leaving its refuters on `review-verify` → `opus`. That is a
policy-alignment change with its own cost and risk profile, which is exactly why
it belongs in its own unit of work rather than in this decision.

## DECISION

<!-- DECISION-BEGIN -->
**`keep-opus` — change nothing.** No model binding is altered by this phase.

The decision rule was stated before the numbers. Applying it:

| # | criterion | opus | sonnet | verdict |
|---|---|---:|---:|---|
| 1 | authoritative-only **false-negative** rate | **16.7 %** (1/6) | **40.0 %** (2/5) | **FAILS** — Sonnet's FN rate is more than double the baseline's |
| 2 | authoritative-only **false-positive** rate | 42.9 % (3/7) | 50.0 % (5/10) | passes (+7.1 pp, inside the 10 pp allowance) |
| 3 | replicate **flip rate** | 40.0 % (2/5) | 28.6 % (2/7) | passes (Sonnet is more self-consistent) |
| 4 | **divergence-class** FP rate | 40.0 % (2/5) | 50.0 % (3/6) | fails — worse on the class the corpus exists to measure |

Criterion 1 alone settles it, and it fails on the error type that matters most.
A false negative is a real defect wrongly refuted: it is silent, it ships, and
nothing downstream catches it. Sonnet produced two of them on five defect-truth
trials against Opus's one on six. Every one of Sonnet's false negatives landed on
a `real-defect` item — it refuted findings that are still true in the tree it was
reading.

Criterion 4 fails in the same direction: on
`mechanically-true-not-a-defect`, the class this corpus is deliberately weighted
toward and the class the original 8-finding A/B diverged on, Sonnet kept 3 of 6
non-defects against Opus's 2 of 5. The initial anecdote reproduced.

**And the cost argument for switching evaporated entirely.** Sonnet did not
merely fail to save tokens — it spent **89.2 % more** of them (1,008,143 mean per
trial against Opus's 532,919) and made **more** tool calls (12.6 against 10.4).
The original A/B measured Sonnet at 52.9k vs Opus 48.4k, a ~9 % excess; measured
here on a repo-reading refuter, the excess is an order of magnitude larger.
Whatever price-per-token saving a cheaper tier offers is being consumed, and then
some, by volume. There is no version of this trade that is both cheaper and
safer.

**Why this is a successful result, not an unfinished phase.** The deliverable was
a decision backed by an instrument, and the instrument is what makes the decision
re-checkable: the corpus, the adjudicated ground truth, the raw trials, the
scorer, and the gate all land in this commit. Re-run it against a future model
and the answer may change; that is the point of building it rather than asserting
the answer. The phase body states this explicitly: *"an outcome of 'keep Opus' is
a legitimate, successful result."*

Consequently **no `verify-workflow-*.sh` acceptance criterion required an
update** — no model binding changed. `scripts/verify-workflow-review.sh` instead
carries a pointer comment near §5b-mechanical recording that judgment-site model
binding was evaluated and deliberately left as-is, and
`scripts/verify-refuter-agreement.sh` §11 enforces that as an XOR: the moment
`lib/plan-review.mjs` gains a `findModel`, the gate starts demanding a matching
`5b-models` criterion.

**What is NOT closed by this decision.** The `plan-review.js` model omission is a
separate question with a separate answer — it is an oversight, and it is filed as
`thread-plan-review-judgment-models`. That fix is about aligning plan-review with
the configured `[models]` policy (which would move its *finders* to Sonnet, since
`review-find` resolves there), not about moving *refuters* off Opus. This
decision says nothing against it.
<!-- DECISION-END -->

## Sibling question: refuter *shape*

This document A/Bs the refuter's **model**. Its sibling,
[`refuter-batching.md`](refuter-batching.md), A/Bs the refuter's **shape** — one
refuter per gating finding versus one per dimension over that review unit's
gating findings — on the same instrument, the same corpus, and the same
never-blend-FN-and-FP discipline. Its outcome is `no-measurement`: grouped by the
key a real dispatch actually forms (`runId | unitIdent | mode | dim.key`), this
corpus yields exactly 1 qualifying group of 3 items against a pre-registered
floor of 6 groups / 18 items, so no A/B was run and the pipeline is unchanged.
Growing this corpus is what unblocks it — see that document's § Mining headroom,
and note that any growth also invalidates the Composition figures above and the
composition floors in `scripts/verify-refuter-agreement.sh` § 2.

## Refuter-agreement harness

The harness is **on-demand only**. It lives entirely under `scripts/` and
`tests/fixtures/`; it imports *from* `.claude/workflows/lib/review.mjs` and
nothing under `.claude/workflows/` imports it back. `scripts/verify-refuter-agreement.sh`
asserts that directionally with a grep, so it can never be wired into the lane's
hot path.

> **Cost warning.** A real run dispatches **paid agents** — one per corpus item
> per tier per replicate. `--dry-run`, `--dispatch-stub` and `--score-only`
> spend nothing. The gate never dispatches.

### The three scripts

| script | role |
|---|---|
| `scripts/lib/refuter-agreement.mjs` | canonical stdlib-only module: corpus schema and validation, prompt reconstruction, trial construction, the scorer, the renderer, and the `--audit` arithmetic |
| `scripts/mine-refuter-corpus.mjs` | mines real historical findings verbatim from `agent-*.jsonl` transcripts; emits `groundTruth: null` always |
| `scripts/run-refuter-agreement.mjs` | regenerates prompts through the real `refutePrompt`, dispatches trials per tier, scores, renders, and audits |

### Mine → adjudicate → run → score → audit

```bash
# 1. MINE candidates from the real corpus (restricted to this repo's own slugs).
node scripts/mine-refuter-corpus.mjs --until 2026-07-29T00:00:00Z --out candidates.jsonl

# 2. ADJUDICATE by hand: for each candidate, read the cited location and fill in
#    groundTruth {defect, class, authority, evidence, adjudicatedAgainstCommit}.
#    The miner never does this — the historical verdict is circular.

# 3. RUN (dispatches paid agents). --dry-run first to see the plan and spend nothing.
node scripts/run-refuter-agreement.mjs --tiers opus,sonnet --replicates 2 --dry-run
node scripts/run-refuter-agreement.mjs --tiers opus,sonnet --replicates 2 \
  --concurrency 8 --out tests/fixtures/refuter-agreement/results-<label>.json

# 4. SCORE a saved run again without dispatching anything.
node scripts/run-refuter-agreement.mjs --score-only tests/fixtures/refuter-agreement/results-<label>.json

# 5. AUDIT the committed figures, corpus-free, on any machine.
node scripts/run-refuter-agreement.mjs --audit docs/token-baseline.json

# 6. GATE everything.
bash scripts/verify-refuter-agreement.sh
```

### Corpus schema

One JSON object per line in `tests/fixtures/refuter-agreement/corpus.jsonl`.
`validateCorpusItem` requires every field and **rejects unknown top-level keys**,
so a typo'd hand edit cannot pass silently.

| field | meaning |
|---|---|
| `id` | unique; `mined-<runId>-<agentId>` or `constructed-<slug>` |
| `schemaVersion` | `CORPUS_SCHEMA_VERSION` |
| `mode` | `code` \| `plan` |
| `dim.key` | the review dimension the finding came from |
| `target` | the refuter prompt's target, recovered whole (may be multi-line) |
| `finding` | `id`, `concern`, `location`, `severity`, `confidence`, `what_fails` (+ optional `why`, `recommendation`) |
| `promptSha256` | sha256 of the original prompt; the runner regenerates and compares |
| `promptDrift` | true once `refutePrompt` no longer reproduces it byte-identically |
| `provenance` | `{kind: 'mined', projectSlug, sessionId, runId, agentId, workflow, historicalVerdict, historicalModel}` or `{kind: 'constructed', builtAgainstCommit, rationale}` |
| `groundTruth` | `{defect, class, authority, evidence, adjudicatedAgainstCommit}` |

### What the scorer guarantees

- **False negatives and false positives are never averaged.** They are computed
  over structurally different denominators (defect-truth trials vs non-defect
  trials) and rendered as two separate labelled blocks with their consequences
  spelled out inline. `findBlendedAccuracyKeys` walks the report recursively and
  the harness asserts **no** key matching `/accuracy|overallCorrect|combinedRate/i`
  exists at any depth — the mechanical form of "never averaged together".
- **Three parallel rate sets** per tier: `authoritativeOnly` (rendered first and
  labelled decision-grade), `judgementCallOnly`, and `all`. They partition
  exactly, which the harness checks field by field.
- **Cost sits on the agreement row.** Each tier's four token classes, mean tokens
  per trial, and mean tool calls per trial are rendered on the *same* table row
  as its FN figures, so agreement and cost cannot be read apart.
- **Determinism.** No `Date.now(`, no `Math.random(`, no network in the module or
  the miner. The run label defaults to the corpus sha, never the clock.
- **The paid-dispatch path is unit-tested with zero spend.** `--dry-run` and
  `--dispatch-stub` deliberately *bypass* `claudeDispatch`/`parseClaudeResult`,
  yet those are exactly the branches that produced the verdicts and the
  token/tool-call figures the DECISION above was computed from. So
  `scripts/verify-refuter-agreement.sh` §7b drives them directly.
  `parseClaudeResult` is a pure function of a response body and is asserted
  against synthetic `claude -p --output-format json` bodies covering: the
  StructuredOutput shape; **several** StructuredOutput blocks, where the *last*
  must win; a bare-JSON `result` string; a prose/fence-wrapped one (exercising
  `tryParseEmbeddedJson`'s string-aware brace matching); a missing `usage`
  object; the `num_tool_uses` fallback-not-override; and — critically — a
  non-boolean or absent `refuted`, which must bucket as `ungraded` rather than
  coerce to `false` and silently inflate the false-positive rate.
  `countSessionToolUses`/`projectSlugFor` run against a scratch transcript with
  interleaved non-`tool_use` blocks, a user turn, and a malformed line. And
  `claudeDispatch` runs against **PATH-shadowed fake `claude` binaries** — PATH
  is replaced wholesale rather than prepended, so a real `claude` stays
  unreachable — covering the success, non-zero-exit, non-JSON-body, and
  missing-binary branches. §9h–9j plant the three regressions this exists to
  catch (first-block-wins, non-boolean coercion, tool-call miscount) and assert
  §7b fails on each, so the coverage is not vacuous.
- **The miner's skip branches decide the corpus size, so they are gated too.**
  How many historical refuters reach the corpus at all is a function of six
  degradation branches (`no-transcript`, `no-prompt`, `unparseable-finding`,
  `unrecoverable-mode`, `unrecoverable-dim`, `no-verdict`); a regression that made
  one of them fire on healthy transcripts would silently shrink the mined
  majority without failing anything.
  `tests/fixtures/refuter-agreement/mine-sidecars` therefore carries one
  transcript per branch, and §4 asserts each bucket's exact count *plus* an
  accounting identity — `recovered + skipped == refuter records` — so no branch
  can become a silent drop. §4b drives the rest of the miner's CLI (`--severity`
  singly and as a comma-set, `--until` in both directions, `--limit`, `--out`,
  `--help`, and every argument-validation error, each of which must be an
  actionable named message rather than a stack trace). §9k–9l plant the two
  regressions those sections exist to catch — a dropped `unrecoverable-mode`
  guard and an inert `--severity` filter — and assert §4 and §4b fail on each.
