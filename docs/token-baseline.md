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
  for it (Sonnet was *not* cheaper in tokens than Opus in the controlled A/B).
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
