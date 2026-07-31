# Token baseline — `workflow-token-reduction` phase 2

**Measurement date:** 2026-07-26
**Measured by:** `scripts/measure-lane-tokens.mjs` (over `scripts/lib/token-report.mjs`), phase 1 of this roadmap.
**Companion machine-readable file:** [`docs/token-baseline.json`](token-baseline.json).

This is the committed "before" reading. Phases 3-7 of this roadmap must diff their
token-saving claims against this snapshot, using the comparison unit named in
[Confounds](#confounds) below — not raw per-run totals.

## Methodology

The instrument locates every `workflows/wf_*.json` session-sidecar file under
`~/.claude/projects/**` (searching every project-slug directory, including
`--worktrees-`-named ones), joins each run's `workflow_agent` entries with their
`subagents/workflows/<runId>/agent-*.jsonl` transcripts, and dedupes usage by
`requestId` (a request streams across several transcript lines; only the last
line for a given `requestId` carries the final `output_tokens`). Each agent's
usage is broken into four token classes — **output**, **uncached input**,
**cache write**, **cache read** — and rolled up by agent class (the label's
leading segment, e.g. `refute:plan:coherence-1` → `refute`), by full label, by
model, and by workflow name.

**Command run:**

```
node scripts/measure-lane-tokens.mjs --format json \
  --workflow autopilot --workflow dispatch-phase --workflow plan-review \
  --workflow backlog --workflow estimate --workflow document
```

against the default root (`~/.claude/projects`), at git SHA `9ec1d881142cf95e4ff370c153940a2c3182f724`
(the commit that landed the phase-1 instrument on this roadmap's branch — not
yet on `main`; see [Known limitations](#known-limitations-and-follow-ups)).

A "run" is one `wf_*.json` sidecar file, i.e. one `Workflow` tool invocation.
An "agent" is one `workflow_agent` entry inside a run's `workflowProgress[]`.

**Known limits carried over from phase 1, still true here:**

- `totalTokens` / `workflowProgress[].tokens` (the "sidecar `tokens` field")
  are **not a cost basis** — see [Totals discrepancy](#totals-discrepancy-sidecar-vs-deduped)
  and [Reconciliation](#reconciliation-vs-the-tokens-field-survey) below, which
  quantify exactly how far off they are and why.
- A `cached: true` agent is genuinely zero-cost (served from the harness's own
  result cache); it contributes 0 to every class, correctly.
- An agent with no `agentId`, no transcript file, or an empty transcript falls
  back to the sidecar-only `tokens` scalar attributed entirely to `output` —
  this is a lossy fallback used for 10 agents across the in-scope run set (see
  the `warnings` array in the JSON twin); none of them belong to a class this
  document draws a headline conclusion from.

**A new limit found while producing this document:** the ratio and floor
figures below required run-level enumeration (which run belongs to which
lane) and a per-agent floor regression, and the CLI's `--format json` output
does not expose either — it only exposes pre-aggregated group totals. Both
were derived by importing `scripts/lib/token-report.mjs`'s exported functions
directly in ad hoc analysis scripts (not committed; every figure they
produced was independently cross-checked against the committed instrument's
own `--format json` output and matches exactly — see
[Known limitations](#known-limitations-and-follow-ups)), reading the exact
same on-disk data the CLI reads. This is a real gap in the CLI's
regenerability surface; it is filed as follow-up task
`measure-lane-tokens-regenerability-gap` rather than fixed inline in this
measurement-only phase.

## Run set

### Included (six in-scope lanes)

The exclusion rule: a run's `workflowName` field must exact-match one of the
six lane names below. Everything else — deep-research runs, one-off spikes,
and workflow names from earlier/experimental review architectures that
predate or sit alongside the current six-lane set — is excluded.

| lane | runs found | notes |
| --- | ---: | --- |
| `autopilot` | 19 | 12 `completed`, 3 `killed` (partial agent sets: 31, 42, 20 agents), 4 `failed` (0 agents each) |
| `dispatch-phase` | 10 | all `completed` |
| `plan-review` | 10 | all `completed` |
| `backlog` | 1 | `completed` |
| `estimate` | 0 | **no runs on disk as of this measurement** — see note below |
| `document` | 0 | **no runs on disk as of this measurement** — see note below |
| **Total** | **40** | 1,943 agent records |

`estimate` and `document` have never been invoked as standalone
`Workflow` runs in the captured history (an `estimate:*`-labelled *agent class*
does appear, but only nested inside `autopilot`'s estimate pre-pass — see the
`estimate` row of the per-agent-class table below, which is real data drawn
from those nested invocations, not from a standalone `estimate` workflow run).
This baseline therefore cannot speak to the token cost of a standalone
`estimate` or `document` workflow run; any phase 3-7 change to those two
lanes will be unmeasured against a real "before" until they have run at least
once in production.

The killed/failed `autopilot` runs are included, matching
`scripts/measure-lane-tokens.mjs`'s default behavior (it has no status
filter) and phase 2's own instruction to measure "every lane run currently on
disk." Each individual agent inside a `killed` run still completed its own
turn and produced real, valid usage data — only the outer multi-phase loop
was interrupted before advancing further — so those agents are legitimate
per-agent-class data points. The 4 `failed` runs contributed 0 agents and are
harmless to include.

`docs/token-baseline.json`'s `runSet` key records this same breakdown in
machine-readable form: `runSet.included.byLane` (per-lane run counts and a
`byStatus` breakdown, matching the table above) and `runSet.excluded.byWorkflowName`
(per-workflow-name counts, matching the table below), plus the two totals
(40 included, 14 excluded). **It does not carry per-run provenance** — no
`runId`, `timestamp`/`startTime`, or `filePath` for any individual run, only
these lane/status/workflow-name aggregates. Individual-run enumeration is
one of the two figures this document's CLI instrument cannot regenerate on
its own (see [Known limitations](#known-limitations-and-follow-ups) and
follow-up task `measure-lane-tokens-regenerability-gap`).

### Excluded (non-lane workflow names)

| workflow name | runs | reason |
| --- | ---: | --- |
| `deep-research` | 5 | not part of the six-lane autonomous set |
| `adversarial-sync-tdd-review` | 1 | earlier/experimental review architecture, predates the six-lane set |
| `adversarial-sync-tdd-review-v2` | 1 | earlier/experimental review architecture, predates the six-lane set |
| `roadmap-review-refute` | 1 | earlier/experimental review architecture, predates the six-lane set |
| `tdd-review-refute` | 1 | earlier/experimental review architecture, predates the six-lane set |
| `plan-review-sync-roadmaps` | 1 | earlier/experimental review architecture, predates the six-lane set |
| `review-refute-fix` | 1 | invoked standalone outside `dispatch-phase`/`plan-review`; not itself one of the six named lanes |
| `spike-agent-model` | 1 | spike run |
| `import-spike` | 1 | spike run |
| `import-hack-spike` | 1 | spike run |
| **Total excluded** | **14** | |

54 runs were found in total on disk (40 included + 14 excluded).

## Per-agent-class token breakdown (cache reads included)

All figures are deduped token counts (last-write-wins per `requestId`),
summed across the 40 included runs.

| agent class | agents | deduped requests | output | uncached input | cache write | cache read | **total** |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| find | 568 | 7,544 | 4,633,806 | 63,388 | 34,256,476 | 398,176,594 | **437,130,264** |
| implement | 48 | 3,216 | 1,553,266 | 12,613 | 7,102,147 | 394,293,558 | **402,961,584** |
| refute | 886 | 5,065 | 4,222,775 | 17,113 | 37,097,110 | 194,651,548 | **235,988,546** |
| plan | 69 | 785 | 1,156,882 | 29,682 | 4,957,839 | 49,731,584 | **55,875,987** |
| act | 51 | 506 | 388,464 | 1,012 | 2,441,044 | 24,141,501 | **26,972,021** |
| diff | 33 | 365 | 401,324 | 1,084 | 2,202,050 | 16,722,669 | **19,327,127** |
| fetch | 105 | 331 | 281,778 | 1,578 | 3,645,545 | 8,496,678 | **12,425,579** |
| estimate | 95 | 259 | 88,870 | 1,374 | 2,619,891 | 5,257,652 | **7,967,787** |
| advance | 28 | 75 | 13,079 | 442 | 783,241 | 1,399,130 | **2,195,892** |
| gate | 17 | 40 | 7,808 | 278 | 262,891 | 1,038,374 | **1,309,351** |
| stamp | 20 | 38 | 4,616 | 174 | 647,214 | 637,115 | **1,289,119** |
| model | 14 | 30 | 2,952 | 60 | 417,104 | 565,878 | **985,994** |
| analyze | 3 | 14 | 23,324 | 28 | 141,811 | 443,345 | **608,508** |
| park | 6 | 18 | 3,700 | 128 | 188,835 | 348,505 | **541,168** |
| **Total** | **1,943** | **18,286** | **12,782,644** | **128,954** | **96,763,198** | **1,095,904,131** | **1,205,578,927** |

This table's totals row was cross-footed by summing the 14 class rows above
column-by-column and independently against the tool's own per-record grand
total (`1,205,578,927` in the raw `--format json` output); both agree
exactly.

Cache reads dominate: **1,095,904,131** of the **1,205,578,927**-token grand
total (91%) is cache read, confirming phase 1's premise that the sidecar
`tokens` field (which is blind to cache reads) badly understates real
traffic. See the next two sections for how much this changes the roadmap's
headline numbers.

## Per-model totals

| model | agents | deduped requests | output | uncached input | cache write | cache read | total |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `claude-sonnet-5` | 436 | 8,446 | 4,650,985 | 78,618 | 31,452,582 | 623,089,521 | 659,271,706 |
| `claude-opus-5[1m]` | 767 | 5,320 | 4,893,548 | 21,675 | 35,624,009 | 229,334,821 | 269,874,053 |
| `claude-opus-4-8[1m]` | 610 | 3,768 | 2,792,506 | 22,417 | 25,385,521 | 214,031,939 | 242,232,383 |
| `claude-haiku-4-5-20251001` | 119 | 752 | 445,605 | 6,244 | 4,301,086 | 29,447,850 | 34,200,785 |
| `haiku` (generic alias, no usage data) | 6 | 0 | 0 | 0 | 0 | 0 | 0 |
| `sonnet` (generic alias, no usage data) | 5 | 0 | 0 | 0 | 0 | 0 | 0 |

**Model-identity edge case:** 11 agent records carry a generic alias
(`"haiku"`, `"sonnet"`) instead of a resolved snapshot ID, and all 11 have
zero deduped requests and zero tokens in every class. Cross-checking these
11 records directly against `buildRecords()`'s per-record flags (not against
the `warnings` array alone) splits them into two distinct populations, not
one:

- **6 are genuinely `cached: true` agents** (1 `haiku`, 5 `sonnet`) — real
  zero-cost cache hits (`agent.cached`), not fallback warnings at all.
- **5 are `haiku`-alias agents with no `agentId`** — these hit the
  no-transcript fallback described above, and in this data the sidecar
  `tokens` scalar they fall back to also happens to be `0`, so the fallback
  still produces an all-zero row.

All **10 of the 10** entries in the JSON twin's `warnings` array are "no
`agentId`" cases (every string in that array contains the phrase "has no
agentId"; an earlier draft of this document miscounted this as "6 of the
10"). Only 5 of those 10 warnings are accounted for above — the other 5
carry a *resolved* `claude-opus-5[1m]` model id rather than a generic
alias, so their (also-zero) sidecar-fallback contribution folds invisibly
into `claude-opus-5[1m]`'s large non-zero row instead of producing a
visible all-zero row of its own. This is consistent with, not in tension
with, the "lossy fallback attributed entirely to output" behavior described
under [Known limits](#methodology) above: the fallback did attribute
sidecar `tokens` to `output` for all 10 no-`agentId` agents; it is only
coincidental to this data set that every one of those 10 agents' sidecar
`tokens` value was itself `0`, so no non-zero `output` appears anywhere in
this table as a result of the fallback.

The 6 genuinely-cached and 5 no-`agentId`-fallback alias records are kept
as **separate rows** from the resolved-snapshot rows for the same model
family (not merged), per the "model version drift" risk this phase was
asked to guard against — merging them would silently under-attribute a
resolved snapshot's real total to the alias, or vice versa.

## Reconciliation vs the `tokens`-field survey

The roadmap body's "Why" section claims, from the original scratchpad survey:

> Review outweighs implementation ~8.6:1 [agent-count share: find 37% +
> refute 36% = 73% vs implement 8%]. A linear fit of tokens against tool
> calls gives an intercept of ~36.8k tokens per agent before any work is
> done, and the cheapest agent observed cost 27k.

That survey used the non-cost-basis `tokens` field. To make an apples-to-apples
comparison, both numbers below are computed on the **same 40-run in-scope set**
measured for this phase — the only difference is which fields are summed.

### Review vs. implementation, by tokens

| methodology | review share (find + refute) | implement share | ratio (review : implement) |
| --- | ---: | ---: | ---: |
| **Old** — sidecar `tokens` field only | 74.89% | 6.75% | **11.1 : 1** |
| **New** — deduped, cache-read-inclusive | 55.83% | 33.42% | **1.67 : 1** |
| **Delta** | −19.06 pp | +26.67 pp | ratio narrows **~6.6x** |

(Old-methodology totals: find 37,594,107 + refute 45,387,233 = 82,981,340 of
110,812,684 total `tokens`-field sum; implement 7,476,962. New-methodology
totals: find 437,130,264 + refute 235,988,546 = 673,118,810 of 1,205,578,927
deduped total; implement 402,961,584.)

**This moves materially.** The old-methodology figure on this larger run set
(74.89% / 6.75%, ~11.1:1) closely reproduces the roadmap's original claim
(~73% / ~8%, ~8.6:1) — the original survey's numbers were internally
consistent for what they measured, and this reconciliation does not
contradict the survey's arithmetic. What it contradicts is the survey's
*conclusion*: once cache reads are counted, `implement`'s share jumps from
6.75% to 33.42% (a 5x increase), driven almost entirely by cache reads —
`implement`'s cache-read total is 394,293,558, which is **98%** of
`implement`'s corrected total and **53x** larger than `implement`'s entire
old-methodology sum (7,476,962). `implement` agents run long
(3,216 deduped requests across only 48 agents — 67 requests/agent on
average), and every one of those ~67 turns pays a cache read for the entire
growing conversation so far. The old `tokens`-field survey was structurally
blind to this because it doesn't carry cache-read numbers at all.

**Roadmap body updated.** Because this moves materially, the roadmap body
(`workflow-token-reduction`) was updated via `rdm roadmap update` to append
a "Phase 2 correction" section carrying the corrected reconciliation figures
alongside the original claim, with a pointer to this document as the source
of truth. The update is staged in the plan repo and lands with this phase's
commit batch.

**Caveat on scope:** this reconciliation is agent-class-vs-agent-class,
computed over a different (larger) run set than the original 20-run survey,
not a strict re-measurement of the exact same runs. The direction and
magnitude of the shift (cache reads reweighting `implement` upward
relative to `find`/`refute`) is the material finding; the exact percentages
will move again as more runs accumulate.

## Agent context floor

### What "floor" means here, and why the regression approach breaks

The roadmap body's claim ("linear fit of tokens against tool calls gives an
intercept of ~36.8k... cheapest agent observed cost 27k") is a regression:
`tokens ≈ a + b·toolCalls`, with the floor read off as the intercept `a`.
Reproducing that exact regression on this phase's run set, using the same
non-cost-basis `tokens` field the original survey used (excluding 6 agents
with a `tokens` value of exactly 0 — orphaned zero-usage transcript entries,
not real observations), gives:

| | intercept (floor) | slope | r² | n |
| --- | ---: | ---: | ---: | ---: |
| **Legacy regression** (sidecar `tokens` field vs. `toolCalls`) | **37,552** | 1,725 / tool call | 0.693 | 1,920 |

This closely reproduces the roadmap's ~36.8k figure (validating the
methodology replication) and its ~27k "cheapest observed" claim (this run
set's cheapest sidecar-`tokens` agent: 21,867).

Running the **same regression** against the deduped, cache-read-inclusive
per-agent total instead of the sidecar `tokens` field breaks it:

| | intercept (floor) | slope | r² | n |
| --- | ---: | ---: | ---: | ---: |
| **Corrected regression** (deduped total vs. `toolCalls`) | **−718,826** (negative — not usable as a floor) | 115,218 / tool call | 0.778 | 1,920 |

The fit quality is actually *better* (r² 0.778 vs 0.693), but the intercept
goes negative because the true relationship is not linear: cache reads
compound turn-over-turn (each additional turn re-reads the entire growing
conversation so far, not a fixed increment), so per-agent total tokens grow
super-linearly with tool-call count. A linear model forced through that
curvature produces a nonsensical negative "floor." **This is itself a
material methodology finding**: the original survey's regression approach,
while internally valid for the field it measured, cannot be reused once
cache reads are in scope.

### The corrected floor: measured directly

The floor — the fixed cost paid before any work happens — is more directly
measured as each agent's **first transcript request only** (uncached input +
cache write + cache read on that single request, before any tool use),
rather than inferred from a regression over an agent's full multi-turn total.
This is a **measured** quantity, taken directly from real per-agent
transcripts (excluding the same 6 zero-usage orphaned entries):

| statistic | value (tokens) |
| --- | ---: |
| n (real agents) | 1,920 |
| minimum | 21,371 |
| 10th percentile | 30,224 |
| **median** | **38,838** |
| mean | 38,452 |

The median (**38,838 tokens**) is this document's reported floor. It lands
within the same order of magnitude as the roadmap's original ~36.8k
regression-intercept claim and this phase's own legacy-regression
reproduction (37,552) — so the floor's **numeric value is not materially
revised** by this phase, even though the *method* used to derive it
(direct first-request measurement, not a total-vs-toolCalls regression) is.

### Attribution: `CLAUDE.md` vs. tool schemas / system prompt

| component | tokens | basis |
| --- | ---: | --- |
| Measured floor (median, first request only) | 38,838 | **measured** — real per-agent transcript data |
| Project `CLAUDE.md` (this roadmap's worktree copy, 1 line ahead of `main`'s 47,015-char copy — a single bullet inserted at line 173 — see [Confounds](#confounds)) | 48,207 chars → ≈ 12,052 tokens | **estimated** — chars ÷ 4 (see below) |
| User-global `CLAUDE.md` (`~/.claude/CLAUDE.md`) | 13,040 chars → ≈ 3,260 tokens | **estimated** — chars ÷ 4 |
| `CLAUDE.md` subtotal (project + user-global) | 61,247 chars → ≈ 15,312 tokens | **estimated** |
| Remainder: tool schemas + system prompt + skill/agent-config overhead | ≈ 38,838 − 15,312 = **23,526** | **estimated** (derived: measured floor minus the estimated `CLAUDE.md` figure) |

No local Anthropic tokenizer was available in this sandbox to produce a true
token count for the `CLAUDE.md` files (no network-independent tokenizer
package is vendored in this stdlib-only repo, and no API credential was
available to call a token-counting endpoint). The `CLAUDE.md` figures above
therefore use the standard ~4-characters-per-token approximation for English
prose/Markdown, applied to a directly measured (`wc -c`) character count —
**the character count is measured; the token conversion is estimated.**
This estimate cross-checks well against the roadmap body's own prior figures
(project ~11.7k, user-global ~3.3k — the estimate above is within 2-3% of
both), which is the best available confidence signal in the absence of a
real tokenizer pass.

The remainder (≈23,526 tokens, tool schemas + system prompt + whatever else
loads before an agent's first tool call) is therefore also labeled
**estimated**, since it is a subtraction against the estimated `CLAUDE.md`
figure, not an independent measurement. Isolating it directly would require
either a real tokenizer or a controlled experiment across agent classes with
different tool-schema surfaces — neither was in scope for this
measurement-only phase.

### Addendum: per-agent-class floor (`floorByAgentClass`)

The median above is a **single whole-corpus figure** — it says nothing about
whether a `fetch` agent's fixed cost differs from a `refute` agent's. A later
phase (`add-per-agent-class-floor`) closed exactly that gap: `firstRequestTokens`
(the same uncached-input + cache-write + cache-read-of-the-first-request
quantity `measuredFloor` above is defined over) is now a field on every
measured `buildRecords` record, and `floorByAgentClass` aggregates it — n,
min, p10, median, mean — **per agent class** instead of across the whole
corpus. `cached`/sidecar-only-fallback records (no per-class split
recoverable) are excluded from the population entirely, not counted as zero.
It is surfaced by `scripts/measure-lane-tokens.mjs` in both `--format json`
and `--format text`, and regenerates exactly via this document's own
`regenerateCommand` (see `docs/token-baseline.json`'s `methodology` block).

The figures were regenerated over the on-disk corpus **as it stood when that
later phase ran** (44 runs / 2,110 records) — a larger corpus than this
baseline's own 40 runs / 1,943 records, since dogfooding continued in the
interim. This is corpus growth between two measurements, not a re-baseline:
every section above (`byAgentClass`, `runSet`, `agentContextFloor`,
`totalsDiscrepancy`, `warnings`) is byte-unchanged from this phase's own
commit. See `docs/token-baseline.json`'s `floorByAgentClass.populationCaveat`
for the exact reconciliation.

Every mechanical agent class this roadmap's phase 3 acts on (`fetch`,
`stamp`, `model`, `diff`, `gate`, `advance`, `park`) has a recorded pre-change
floor. Two classes carry an important caveat before being read as a clean
mechanical figure:

- **`estimate`** mixes mechanical (`estimate:list`/`write:*`/`tier:*`) and
  judgment (`estimate:rate:*`) sub-labels under one `agentClass` bucket,
  because the class is derived by splitting a label on its first colon only.
- **`act`** mixes mechanical (`act:round-note:*`, irreducible per
  [`docs/mechanical-agent-inventory.md`](mechanical-agent-inventory.md)) and
  judgment (`act:code`, `act:<kind>:<ident>`) sub-labels under the same
  one-bucket-per-colon-split rule.

Neither `estimate`'s nor `act`'s floor may be read as a clean mechanical-only
figure; a phase comparing a mechanical-elimination target against either
bucket must apply this caveat rather than treat every listed class as
equally clean.

## Totals discrepancy: sidecar vs. deduped

| | tokens |
| --- | ---: |
| Sidecar `totalTokens` sum (all 40 included runs) | 110,812,684 |
| Deduped per-class sum (all 40 included runs, all agents) | 1,205,578,927 |
| Delta | −1,094,766,243 |

The deduped total is **10.9x** the sidecar-reported total. This is the same
finding phase 1 and `autopilot-run-accounting` established from a single run;
this phase confirms it holds across the full in-scope run set, not just the
one run originally sampled.

## Confounds

Runs are not directly comparable to each other, for several independent
reasons:

- **Roadmap size** — a roadmap with more phases dispatches more
  `dispatch-phase`/`plan-review`/`estimate` agent invocations inside a single
  `autopilot` run; a bigger roadmap does not imply a more expensive *phase*.
- **Phase difficulty / model tier** — `rdm-estimate` assigns each phase a
  difficulty and model tier; a `high`-tier phase's `implement`/`find`/`refute`
  agents run on a more expensive model and, per the "Evidence already
  gathered" section of the roadmap body, do not necessarily use fewer tokens
  for it. Sonnet was *not* cheaper in tokens than Opus: the original 8-finding
  A/B measured 52.9k vs 48.4k, and that finding has since been re-measured
  against a 56-item adjudicated corpus — see
  [`refuter-model-tiering.md`](refuter-model-tiering.md), which supersedes the
  8-finding A/B as the reference for this confound. Per-agent cost is dominated
  by boot and file reading, so re-tiering changes price per token, not token
  volume.
- **Rework rounds** — a phase that bounces through plan-review or code-review
  rework rounds re-runs `find`/`refute`/`plan`/`implement` agents multiple
  times; its total is not comparable to a phase that passed review on the
  first pass.
- **User-specific `CLAUDE.md`** — the user-global `CLAUDE.md` measured for
  the [Agent context floor](#agent-context-floor) section is this operator's
  own machine (`~/.claude/CLAUDE.md`, 13,040 chars). It is not a universal
  constant; a different operator's global instructions file will shift the
  floor by a different amount. This is a genuine confound for anyone
  replaying this baseline on a different machine.
- **Project `CLAUDE.md` drifts between `main` and a roadmap's worktree** — the
  project `CLAUDE.md` measured above is this roadmap's shared worktree copy
  (48,207 chars, 429 lines), which is already ahead of `main`'s copy
  (47,015 chars, 428 lines — a single bullet inserted at line 173 documenting
  the phase-1 instrument's own harness) by 1,192 characters. Every in-flight
  roadmap worktree carries its own small `CLAUDE.md` delta from `main`, so the
  project-`CLAUDE.md` component of the floor will shift slightly depending on
  which worktree (or `main`) an agent runs in.
- **Model-identity drift** — as noted above, some agent records carry a
  generic alias (`haiku`, `sonnet`) rather than a resolved snapshot ID and
  contribute zero measurable usage; per-model comparisons must key on the
  exact snapshot ID, not a display name, to avoid conflating different model
  versions.
- **Run completeness** — 3 of the 19 `autopilot` runs in scope are `killed`
  and 4 are `failed` (0 agents); see [Run set](#run-set) for how this
  document handled them.

**Because of all of the above, per-run totals are not a valid unit of
comparison for later phases' before/after claims.** The comparable units are
**per-agent-class figures** (the tables above) and **per-phase-dispatch
figures** (one `dispatch-phase` invocation's token cost, independent of which
roadmap or autopilot run it was dispatched from) — later phases must diff
against those, not against a raw sum of tokens across runs.

## No behavior change

This phase is measurement and documentation only. No file under
`.claude/workflows/`, `.claude/workflows/lib/`, `.claude/skills/`, or
`scripts/measure-lane-tokens.mjs`/`scripts/lib/token-report.mjs` was
modified. The only files this phase's commit touches are this document and
its machine-readable twin, `docs/token-baseline.json` (plus the
plan-repo roadmap-body update described above, which is a separate mutation
surface, not a change to this git repo). The lane's behavior is
byte-identical before and after this phase.

## Known limitations and follow-ups

- **AC5 ("`docs/token-baseline.json` ... is regenerable by re-running the
  phase-1 CLI") is only PARTIALLY met, and this document says so explicitly
  rather than leaving a reader to discover the gap.** Re-running the exact
  `regenerateCommand` above reproduces `byAgentClass`/`byLabel`/`byModel`/
  `byWorkflow`/`totalsDiscrepancy`/`warnings` byte-for-byte — the bulk of the
  committed JSON's figures. It does **not** reproduce two sections: `runSet`
  (run-level enumeration/inclusion accounting) and `agentContextFloor` (the
  first-request/regression floor analysis) — both were derived by importing
  `scripts/lib/token-report.mjs`'s exported functions directly in ad hoc,
  uncommitted analysis scripts, reading the same on-disk data the CLI reads,
  but not through a documented CLI output mode. Closing this gap for real
  means adding output modes to `scripts/measure-lane-tokens.mjs`, which this
  phase's own acceptance criteria forbid (AC7: "no reduction change is made
  in this phase" — the approved plan scopes `scripts/measure-lane-tokens.mjs`
  as no-change, and AC7's own check asserts the commit touches only the two
  docs files). Fixing AC5 in full and satisfying AC7 in full are mutually
  exclusive within this phase's scope; this phase keeps AC7 (zero behavior
  change to the shipped instrument) and reports AC5 as partially met, with
  the remainder tracked as follow-up task
  `measure-lane-tokens-regenerability-gap` rather than fixed inline here.
- **A real bug was found in the committed phase-1 instrument while producing
  this document**: `scripts/lib/token-report.mjs`'s `parseWorkflowRun`
  accepts a syntactically-valid `wf_*.json` that is missing its `runId`
  field, and `buildRecords()` then crashes (rather than warn-and-skip, the
  pattern every other malformed-input path follows) when it tries to build
  that run's transcript directory path from an `undefined` runId. A local,
  uncommitted guard against this was present in the shared roadmap worktree
  at the start of this phase's work; it was reverted (this phase's scope is
  measurement-only — no edits to the phase-1 instrument, per its own
  acceptance criteria) and the fix refiled as follow-up task
  `measure-lane-tokens-missing-runid-crash` instead. The measurement above
  was re-run against the reverted (committed-only, un-patched) instrument to
  confirm this: **the figures are unchanged** — no run in the 40-run included
  set actually triggers the guard (the `warnings` array in the JSON twin
  contains no "missing or invalid runId" entries), so the bug's presence or
  absence has no bearing on this document's numbers.
- **No local Anthropic tokenizer was available**, so the `CLAUDE.md`
  attribution in [Agent context floor](#agent-context-floor) uses a
  character-count heuristic rather than a true tokenization pass. See that
  section for the cross-check that gives confidence in the estimate.
- **`estimate` and `document` have no standalone on-disk runs** as of this
  measurement (see [Run set](#run-set)) — this baseline cannot speak to
  their per-run cost, only to the `estimate:*`-labelled agent class as
  observed nested inside `autopilot` runs.

## Phase 3: mechanical-agent elimination

Phase 3 of this roadmap removes mechanical subagents by **never spawning them** —
hoisting the command to the caller that already has the repo in context, absorbing
it into an adjacent running agent, or suppressing it as redundant. The full call-site
census, the classification rule, the irreducible set that scopes phase 4, and the
measured delta are in
[`docs/mechanical-agent-inventory.md`](mechanical-agent-inventory.md).

Headline: **115 of 304 mechanical agents (37.8%)** observed across the measured
corpus would not have been spawned had phase 3's code been live. The `model` and
`diff` agent classes go to **zero** on the shim-driven paths; `diff` goes to zero
everywhere for `dispatch-phase`, because absorption needs no caller.

Two measurement caveats that constrain how that number may be compared with the
baseline above:

- **`docs/token-baseline.json` carries no `byLabel` section.** The JSON comparison
  is therefore done at **agent-class** granularity, and per-label figures are read
  from the CLI's own `byLabel` output.
- **The corpus has grown since this baseline was taken**, and every run in it
  executed pre-change code. A raw `byAgentClass` subtraction between two JSON
  snapshots measures corpus growth, not the change. The inventory doc reports a
  **replay delta** instead — the observed per-label counts with phase 3's
  elimination rules applied — and states which caller surface each elimination
  depends on. A fresh post-change dispatch is the natural confirmation and should
  be taken on the next real lane run.

The measured delta reflects the **local dogfood shim callers**. The distributed
side of `plan-review`, `backlog`, `document`, `review` and `estimate` stays on the
in-workflow fallback until task
`convert-remaining-skill-templates-to-workflow-shims` lands.

## Phase 6: non-gating refutation

Phase 6 stops spawning a refuter for a finding whose verdict cannot change the
outcome. `hasBlocking` gates on `blocking` (and, at the `large` tier, `concern`);
the acceptance-criteria table is a separate structured channel that never reads
finding severity. `suggestion` therefore gates nothing at any tier, and its
refuter is pure cost. Such findings now pass through marked `unrefuted: true`,
still subject to the confidence floor, and act steps handle them under an
explicit disposition rule rather than as confirmed defects.

The per-severity breakdown that justifies this — a dimension the single `refute`
agent-class bucket above cannot see — is measured by
`scripts/measure-refuter-severity.mjs` and recorded in full in
[`docs/token-baseline.json`](token-baseline.json) § `nonGatingRefutationSkip`.
Headline figures, over the 48-run window ending 2026-07-29 (989 refuters):

| severity | agents | graded | refuted | rate | all tokens | fresh tokens |
|---|---:|---:|---:|---:|---:|---:|
| blocking | 197 | 197 | 75 | 38.1 % | 58,706,344 | 9,913,110 |
| concern | 527 | 522 | 263 | 50.4 % | 147,254,974 | 25,364,063 |
| suggestion | 239 | 236 | 175 | 74.2 % | 54,310,983 | 10,334,840 |
| unrecoverable (no transcript) | 26 | 0 | 0 | — | 1,724,157 | 1,724,157 |

**239 refuters (24.2 % of all refuters, 20.7 % of refuter tokens, 4.1 % of all
lane tokens) would not have been spawned** had this change been live —
54,310,983 tokens, or 10,334,840 excluding cache reads.

`concern` deliberately keeps its refuter: it is overturned *more* often than a
`blocking` finding (50.4 % vs 38.1 %), so the refuter is doing real work, and it
gates outright at the `large` tier. That also makes the pass-through set
tier-independent, so no tier has to be threaded into the pipeline.

Two caveats, the same shape as phase 3's:

- **The window is wider than the run set above** (48 runs / 2208 agent records vs
  40 / 1943): the corpus grew between the two measurements. These rows are not
  subtractable against the per-agent-class table — compare only within the
  section.
- **No post-change lane corpus exists.** Every run in the window executed
  pre-change code, so this is an exact accounting of what non-gating refutation
  actually cost, not a forecast. The natural confirmation is the next real lane
  run: the `suggestion` row should go to zero agents.

Gating: every figure above is `--check`-gated against the real corpus
(`node scripts/measure-refuter-severity.mjs --check docs/token-baseline.json`,
run by hand since it needs the sidecars) and, corpus-free, `--audit`-gated by
`scripts/verify-token-report.sh` on any machine. The prose framing (the caveats,
the decision rationale) is provenance-only.

## Phase 1: review fanout

A cap on refuter spend cannot be sized honestly without knowing how many
findings a finder actually emits, or how many refuters a single review unit
dispatches — the single `refute` agent-class bucket, and even the
per-severity breakdown directly above, cannot see either. This section
extends the SAME instrument (`scripts/measure-refuter-severity.mjs`) with two
more descriptive distributions, over the SAME 48-run window ending
2026-07-29 (2,208 agent records) as § "Phase 6: non-gating refutation" above
— the two sections were regenerated together, though a future regeneration
of either one is not required to keep the windows aligned; check each
section's own `measurementWindow` before comparing them.

**Findings per finder**, split by mode and dimension, read from each
finder's own transcript output (never inferred from refuter counts — since
phase 6 a `suggestion` is never dispatched to a refuter, so refuter count is
strictly less than finding count). The Workflow runtime suffixes a retried
dispatch's label with `(retry N)` (e.g. `code:ac (retry 1)`); that suffix
names the attempt, not the dimension, so it is stripped before grouping and
retried dispatches pool into their non-retried siblings' row rather than
fragmenting into their own — the table below is therefore the complete,
un-split breakdown (11 rows), not a readability trim of a larger one. The
same rows, keyed identically, are in
[`docs/token-baseline.json`](token-baseline.json) § `refuterFanout.findingsPerFinder`.

| mode | dim | n | min | p50 | p90 | max | refuters dispatched |
|---|---|---:|---:|---:|---:|---:|---:|
| code | ac | 51 | 0 | 0 | 2 | 2 | 28 |
| code | api-docs | 4 | 0 | 0 | 0 | 0 | 0 |
| code | architecture | 53 | 0 | 0 | 1.8 | 2 | 25 |
| code | changelog | 17 | 0 | 0 | 1 | 1 | 4 |
| code | correctness | 51 | 0 | 1 | 2 | 3 | 52 |
| code | security | 19 | 0 | 0 | 1 | 1 | 5 |
| code | tests | 51 | 0 | 1 | 2 | 3 | 57 |
| plan | architectural-fit | 117 | 0 | 2 | 4.4 | 6 | 266 |
| plan | coherence | 118 | 0 | 2 | 5 | 7 | 301 |
| plan | restraint | 26 | 0 | 2 | 3 | 4 | 53 |
| plan | unit-of-work | 118 | 0 | 1 | 3 | 4 | 172 |

35 finder transcripts were unreadable (no `StructuredOutput` call ever seen)
and are excluded from every row's `n` rather than counted as zero findings;
no finder label failed to resolve to a `(mode, dim)` pair.
`refutersDispatched` counts only refuters whose OWN prompt-derived dimension
matches the row — never the refuter's label, which a finder-supplied `f.id`
routinely displaces (see Method below) — so it can undercount against `n`
where a dimension's refuters fell outside this window or its identity could
not be resolved.

**Refuters dispatched per review unit** — one `dispatch-phase` code-review
stage or one `plan-review` phase unit, keyed by the unit identity embedded in
each refuter's own initiating prompt turn, NEVER by `phaseTitle`/`phaseIndex`
(see Method below for why that key silently collapses a whole plan-review
run into one unit):

| units | min | p50 | p90 | max |
|---:|---:|---:|---:|
| 80 | 1 | 8.5 | 13 | 21 |

The unit was recovered for **639 of 989 refuters (64.6 %)**; the remaining
350 are reported as `unrecoverableRefuterCount` and are never bucketed into
any unit's count. Unrecovered cases are chiefly the `--implementation-plan`
target, which is itself pretty-printed JSON and is rejected by construction
rather than captured as a fake identity, plus refuters with no transcript at
all.

**Method.** Both a refuter's dimension and its unit identity are parsed from
the same header line § "Phase 6" already reads for severity (`A prior
reviewer raised this <dim> finding against <target>:`), never from the
refuter's `refute:<mode>:(f.id|dim.key:idx)` label — a finder-supplied `f.id`
(the common case) displaces the dimension out of the label entirely. The
review-unit boundary is deliberately not `phaseTitle`/`phaseIndex`: those are
the workflow's own declared pipeline stages, and in a plan-review run they
are IDENTICAL across every review unit in the run — measured directly on run
`wf_55af7324-87c` (`--roadmap project-agnostic-lane`): 152 agents, all 96
refuters sitting at the SAME `phaseIndex` across 9 distinct review units,
which that key would silently collapse into "one unit with 96 refuters".
`context.target`, embedded inline in the prompt, supplies the real boundary
instead.

Two caveats:

- **These rows are not subtractable against § "Per-agent-class token
  breakdown" or § "Phase 6" above** — the corpus has grown since the run-set
  measurement (40 runs / 1,943 agent records vs 48 / 2,208 here), the same
  caveat phases 3 and 6 of the `workflow-token-reduction` roadmap already
  carry. Compare figures only within this section.
- **The per-unit distribution is a lower bound, not an exact accounting.**
  Only 64.6 % of refuters resolved to a unit; the rest are excluded from
  every unit's count rather than guessed at, so the true per-unit fan-out is
  at least as high as reported here.

Gating: every figure above is `--check`-gated against the real corpus
(`node scripts/measure-refuter-severity.mjs --check docs/token-baseline.json`)
and, corpus-free, `--audit`-gated by `scripts/verify-token-report.sh` on any
machine. The prose framing (this section's caveats and method paragraph) is
provenance-only.

**Continued in § "Phase 2: rank of the determining finding"**, immediately
below. That section is the second half of this one and is kept adjacent rather
than nested: it reuses this section's prompt-derived review-unit key verbatim,
reports over this same 48-run window, and is bounded by the 64.6 % recovery
rate stated just above. Read the two together; neither restates the other's
figures.

## Phase 2: rank of the determining finding

Phase 1 sized the fan-out. This section answers the single question a
refutation cap lives or dies on: **where in a ranked list does the finding
that actually determined the outcome sit?** A cap that grades only the top N
candidates is free if that finding is almost always near the top, and sheds
real signal if the rank is spread out.

A **determining finding** is the highest-ranked candidate that both survives
its refutation (`survives`) and makes `hasBlocking` true — the one finding
that carried the unit's outcome. The ranking and the gating rule are not
reimplemented here: `scripts/measure-refuter-severity.mjs` **imports**
`rankFindings` / `survives` / `hasBlocking` directly from
[`.claude/workflows/lib/review.mjs`](../.claude/workflows/lib/review.mjs), so
the measurement cannot drift from the behavior it is predicting. Same window
as § "Phase 1: review fanout" and § "Phase 6": **48 runs ending
2026-07-29T00:00:00Z, 2,208 agent records** — all three sections are
regenerated in one pass over one filtered corpus.

**Ranked over the candidate list, not the survivor list.** This choice is the
whole measurement. Ranking among *survivors* is degenerate: `SEVERITY_RANK`
orders `blocking`(0) before `concern`(1) before `suggestion`(2), so the
top-ranked survivor **is** by construction the one that makes `hasBlocking`
true, and the answer would be a constant 1 for every determining unit — a
tautology dressed up as a finding. A refutation budget truncates the
**candidate** list, which is what the finders emitted before anything is
graded, so that is the ranking a cap would actually apply and the one measured
here.

**Severity is checked before disposition**, and that order is load-bearing. A
candidate whose severity sits outside `hasBlocking`'s blocker set for the tier
being walked — a `suggestion` at either tier, a `concern` at the default tier —
can never be the determining finding *whatever its verdict turns out to be*, so
the walk skips it outright and neither its verdict nor a **missing** verdict can
decide the unit. Reading the disposition first would silently reclassify a
genuinely non-determining unit (one blocking candidate that *was* refuted, plus
a suggestion whose refuter transcript is unparseable) as unrecoverable,
collapsing the very distinction this section exists to keep — and this window
predates `workflow-token-reduction` phase 6, when suggestions were still
dispatched to refuters, so such units really occur in it.

**Rank distribution** (determining units only):

| rank | units |
|---:|---:|
| 1 | 23 |
| 2 | 6 |

Of the **84** review units in the window, **72 were recoverable (85.7 %)** —
29 determining, 43 non-determining — and 12 were not. Against that recoverable
share of 85.7 %:

| within top N | units | % of determining (29) | % of recoverable (72) |
|---:|---:|---:|---:|
| 3 | 29 | 100 % | 40.3 % |
| 5 | 29 | 100 % | 40.3 % |

Both denominators are labelled deliberately: the second is smaller because 43
recoverable units had **no** gating finding at all, and those are
non-determining, not rank-absent. Nothing is imputed for the 12 unrecoverable
units — they appear in no numerator and in no denominator.

The cap is not vacuous at either N: candidate lists over the same recoverable
units run **n 72, min 0, p50 8.5, p90 13, max 15**, so a top-3 or top-5 budget
would genuinely truncate the median unit rather than never binding.

**Unrecoverable units, by reason:**

| reason | units |
|---|---:|
| unreadable-finder-transcript | 1 |
| multi-round-unit | 11 |

`unknown-disposition-above-determining` does not appear at the default blocker
set, and that absence is the recoverability rule doing its job rather than a
gap in the corpus. The rule is: the walk stops at the first *blocking-eligible*
candidate whose disposition cannot be read, because an ungraded finding ranked
**above** the determining one could itself have been the determining finding at
a better rank. An ungraded finding ranked strictly **below** it, or one that
could never have gated at all, cannot change the answer and is harmless — that
double asymmetry is what makes 85.7 % of units recoverable rather than far
fewer. (At the `large` tier, where `concern` joins the blocker set, two units
*do* land under this reason, taking that variant's unrecoverable count to 14;
see the tier variant below.)

**Recoverability scoping: contamination is per unit, never run-wide.** Every
reason above is decidable from the unit's own records. An agent whose unit
identity does not resolve — chiefly the `--implementation-plan` target, which
is itself pretty-printed JSON and is rejected by construction rather than
captured as a fake identity — is attributable to **no** unit, so it marks no
unit unrecoverable. It is counted separately: **252 orphan finders and 350
orphan refuters across 26 runs** (that refuter figure reconciles exactly with
§ Phase 1's `unrecoverableRefuterCount`, the same 350 counted the same way).
This is phase 1's own precedent applied to units — it already sends an
unresolved refuter to its own bucket and never lets it invalidate the units
that *did* resolve in the same run. A run-wide rule would not be conservative
but destructive: real runs mix many named units with an occasional orphan
(phase 1's method note cites a 9-unit run), so a single bad target could zero
out the recoverable share and manufacture a verdict out of a corpus artefact
rather than out of the data. The missing-candidate hazard such a rule reaches
for is caught locally instead, by `dimension-coverage-gap` — the unit's own
finder set failing to cover the dimensions that were always-on for the whole
window (code: `ac` + `correctness`; plan: `coherence` +
`architectural-fit` — `restraint` is always-on today but shipped part-way
through this window, so requiring it would flag pre-`restraint` units for an
artefact of release timing), or the unit holding a refuter for a dimension it
has no finder for. The residual risk this accepts is stated rather than
imputed: an orphan **finder** could in principle have belonged to a named unit
whose coverage nonetheless looks complete (an unlabeled retry, say), leaving
that unit's candidate list short and its rank understated. The 252 orphan
finders are the explicit upper bound on how many units could be understated
that way; it is bounded and reported, never repaired by guesswork, and it
never invalidates a unit that did resolve.

**Tier is not recoverable.** It is threaded through `context` at runtime and
embedded in neither prompt, so no unit can be attributed to one. The headline
uses the default blocker set (`['blocking']`); the `largeTier` variant re-runs
the identical walk with the set widened to `['blocking','concern']` rather
than guessing a tier from an agent's model id. It yields 55 determining units
with ranks 1 (37), 2 (14), 3 (1), 4 (2) and 7 (1) — **52 within top 3
(94.5 %)** and **54 within top 5 (98.2 %)**. Widening the blocker set can only
move a determining finding earlier in the same ranking, so both counts are
monotone above the default tier's; `--audit` asserts that directly.

**The `ac`-table side channel** is a second outcome path this measurement
deliberately does not score. In code mode `classifyOutcome` routes a
FAIL/PARTIAL AC table through `acTableHasGap` directly, never through finding
severity or refutation, so a unit can have been `rework` with no gating finding
at all. **3** units carried such a table; they are reported as their own
diagnostic and are *not* folded into the non-determining count. This phase's
question is scoped to `hasBlocking`.

**Verdict: the evidence SUPPORTS a cap.** Phase 4 should cap refutation, and
should choose **N = 5**. The threshold rule that produces this — exported as
`CAP_VERDICT_RULE` and re-derived from the committed figures by `--audit`, so
the conclusion cannot drift from the data it reads — is: `supports-cap` iff the
top-5 share is ≥ 95 % of determining units **and** the recoverable share is
≥ 50 % **and** there are ≥ 20 determining units; `kills-cap` iff the top-5
share is below 80 %; `inconclusive` otherwise, explicitly including "too few
recoverable units to speak to it". Here: 100 %, 85.7 %, and 29 units. N = 3 is
supported at the default blocker set (100 %) but slips to 94.5 % at the `large`
tier, which the corpus cannot rule out per unit; N = 5 clears the bar at both
(100 % and 98.2 %), so N = 5 is the choice the evidence actually backs.

Had the distribution not concentrated, recording that would have been this
phase's terminal finding and the evidence for phase 4 to conclude "do not cap";
`kills-cap` and `inconclusive` are first-class outcomes of the same derived
rule, not failure modes.

Two caveats carried forward:

- **Not subtractable against § "Per-agent-class token breakdown" or § "Phase
  6"** — the corpus has grown since the run-set measurement (40 runs / 1,943
  agent records vs 48 / 2,208 here). Compare only within this section.
- **Phase 1's recovery rate bounds this one.** Phase 1 recovered a unit for
  64.6 % of refuters over this same window, which bounds how much of the corpus
  this section can speak to; 85.7 % of *units* resolved within that bound. Two
  further limits on the disposition half: the corpus cannot distinguish a
  refuter that crashed (which the live pipeline keeps as un-refuted) from one
  whose transcript is simply absent, so both are read as `unknown` —
  conservative, and possibly divergent from what the pipeline actually did; and
  the window straddles `workflow-token-reduction` phase 6, before which
  `suggestion` findings were refuted and after which they pass through
  ungraded, so a missing refuter for a suggestion is legitimate while a missing
  refuter for a gating finding is unknown.

**This phase changes no lane behavior.** Nothing under `.claude/workflows/`,
`.claude/skills/` or `rdm-core/src/templates/` is modified — `review.mjs` is
imported read-only. The cap this measurement informs is phase 4's to build;
**the value it chose, and why, is in § "Phase 4: the chosen refutation budget"
immediately below.**

Gating: every figure above is `--check`-gated against the real corpus
(`node scripts/measure-refuter-severity.mjs --check docs/token-baseline.json`)
and, corpus-free, `--audit`-gated by `scripts/verify-token-report.sh` section 7
on any machine — including the supports/kills verdict, which `--audit`
re-derives from the doc's own numbers. The prose framing is provenance-only.

## Phase 4: the chosen refutation budget

Phase 2 measured; this section decides. **The cap ships at N = 5**, as
`DEFAULT_MAX_REFUTATIONS` in `.claude/workflows/lib/review.mjs`.

This section records a DECISION, not a new measurement — it adds no key to
`docs/token-baseline.json` and changes none of the figures above. It reads them.

### Both escape hatches are closed

The phase was explicitly allowed to conclude "no cap needed". It cannot:

- **Phase 2's own pre-registered rule returns `supports-cap`.**
  `capVerdict.verdict` is `"supports-cap"`, re-derived corpus-free by
  `--audit`. The rule (`withinTop` n=5 as a percentage of DETERMINING units;
  supports at ≥ 95 %, kills below 80 %, ≥ 50 % recoverable, ≥ 20 determining
  units) is satisfied on every input: 100 % within top 5, 85.7 % recoverable,
  29 determining units.
- **Phase 3 shipped no pipeline change, so batching did not moot the cap.**
  `refuterBatching.corpusPower.meetsMinimum` is `false` (1 qualifying group
  against a floor of 6), and `git diff main..HEAD --stat` on
  `roadmap/bound-review-fan-out` touches no file under `.claude/workflows/` —
  `review.mjs` was still byte-identical to `main` when this phase began. The
  "phase 3 may have made refutation cheap enough that the tail no longer
  matters" branch therefore does not apply.

### Why 5, and why not 3

| tier (blocker set) | determining units | rank histogram | p90 | max | within top 3 | within top 5 |
| --- | ---: | --- | ---: | ---: | ---: | ---: |
| default (`['blocking']`) | 29 | `{1: 23, 2: 6}` | 2 | 2 | **100 %** | **100 %** |
| `large` (`['blocking','concern']`) | 55 | `{1: 37, 2: 14, 3: 1, 4: 2, 7: 1}` | 2 | 7 | 94.5 % | **98.2 %** |

**N = 3 is rejected.** It is free at the default tier, but its large-tier
coverage is 94.5 % — below phase 2's own *pre-registered*
`supportsCapAtOrAbovePercent` of 95. Choosing 3 would contradict the rule the
evidence was graded under. N = 5 clears it at both tiers.

**The cap is not a no-op.** Candidate-set size over the 72 recoverable units is
p50 8.5, p90 13, max 15 — so a cap of 5 bites on more than half of all units and
buys real refuter spend, which is the whole point of bounding the fan-out.

**The residual is one unit.** Exactly 1 of 55 large-tier determining units (the
rank-7 unit in the histogram above) would have its determining finding go
ungraded under N = 5.

### Why the residual is safe by construction

Not by luck — by a monotonicity argument that holds for every N, including 0:

1. `survives(finding, verdict)` reads the **finding's** confidence, never the
   verdict's. The only effect a verdict can have is `refuted === true ⇒ drop`.
2. Therefore `survives(f, null) === true` whenever `survives(f, v) === true`:
   skipping refutation is monotone-**increasing** in the survivor set.
3. Over the same candidate list, the budgeted survivor set is a **superset** of
   the unbudgeted one — the top N behave identically, and the overflow can only
   gain survivors that grading would have removed.
4. `hasBlocking` is an existential over that set, and `classifyOutcome` step 3
   returns `rework` iff `hasBlocking(lastRound, tier)`. A superset can only ADD
   a blocking survivor.

So a budget hit can only move `reviewed → rework`, never `rework → reviewed`.
The rank-7 unit would still have been `rework` under the cap; it would simply
have arrived there on an ungraded finding. The AC table is never budgeted, so
`classifyOutcome` step 2 is bit-identical under every N, and plan findings feed
step 1 through the same monotone `hasBlocking`, so `escalated` is likewise only
ever reachable more often.

This is encoded EXECUTABLY, not only here: `scripts/verify-workflow-review.sh`
§ 9 runs an exhaustive property test over every refuted-subset × N × tier
combination of a planted candidate set, plus the named rank-7 scenario and the
below-floor inverse.

### What the cap does NOT change

The overflow reuses the pass-through `workflow-token-reduction` phase 6 already
built (`unrefuted: true`, `verdict: null`, `skipped: true`) with a new
`unrefutedReason: 'budget'` discriminator. `survives` is untouched: an
over-budget finding below the 70-point confidence floor is still dropped, and
one at or above it still gates. The budget skips **grading**, never
**filtering**.

## Refuter model tiering

Refuters run on the most expensive tier everywhere (`review-verify` resolves to
`opus`; `plan-review.js` passes no models and inherits the opus-class session
model). Whether they must was settled against a 56-item adjudicated finding
corpus rather than the original 8-finding A/B, with false-negative and
false-positive rates reported separately — a false negative ships a defect, a
false positive costs a rework round — alongside per-tier token volume and
tool-call counts.

The headline: **token volume is not the lever here.** The cheaper tier does not
spend fewer tokens; per-agent cost is dominated by boot and file reading. Any
saving from re-tiering is a price-per-token argument, and this roadmap's metric
is volume.

The full method, the corpus composition, the per-class and authoritative-only
tables, the self-consistency flip rates, the answer to the `plan-review.js`
model-omission question, and the decision itself are in
[`refuter-model-tiering.md`](refuter-model-tiering.md). Machine-readable figures
live in `docs/token-baseline.json` § `refuterModelTiering` and are audited
corpus-free by `node scripts/run-refuter-agreement.mjs --audit
docs/token-baseline.json`. Tables are not restated here.

## Refuter batching — one refuter per dimension, or one per finding?

The sibling **shape** question to the model question above, on the same
instrument and the same adjudicated corpus:
[`refuter-batching.md`](refuter-batching.md).

The answer is **`no-measurement`**, and the pipeline is unchanged. Grouped by the
key a real dispatch actually forms — `runId | unitIdent | mode | dim.key`,
because `buildReviewPipeline` runs once per *review unit* — the committed 56-item
corpus yields 35 groupable items in 29 groups: **24 singletons, 4 pairs and 1
triple**, i.e. exactly **1 qualifying (size ≥ 3) group / 3 items** against
pre-registered floors of 6 groups / 18 items. A batched arm built from that is
byte-for-byte a per-finding arm across most of its items, so the anchoring effect
the phase exists to detect would be unobservable and any A/B would report
dilution rather than evidence. `POWER: INSUFFICIENT`, no dispatch, no decision.

The earlier `(runId, mode, dim.key)` framing reported 36 groups with four of size
≥ 3; those figures are void (two are an artifact of the 12 `constructed` items
collapsing into one pseudo-run, and the only non-constructed triple spans two
different review units). See the doc's § Corpus power for the correction.

Reproduce the power analysis with zero spend:

```bash
node scripts/run-refuter-agreement.mjs --batch-power
```

Machine-readable figures live in `docs/token-baseline.json` § `refuterBatching`
(including the superseded naive-key histogram and the mining headroom a future
attempt must buy) and are audited corpus-free by `node
scripts/run-refuter-agreement.mjs --audit docs/token-baseline.json
--audit-section refuterBatching`. Tables are not restated here.
