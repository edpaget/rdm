# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.19.0] - 2026-08-10
### Added

- `rdm-core` can now emit rdm's Claude Code lane as an installable **plugin tree** as well as the existing raw skills tree. New public API in `rdm_core::agent_config`: `generate_plugin_manifest()` returns the `.claude-plugin/plugin.json` manifest (name `rdm`, the crate version, a plugin-facing description, and an author block — deliberately no `workflows` key, since that directory is convention-discovered and the key would replace rather than add to the default), `generate_plugin_skills()` / `generate_plugin_workflows()` return the eleven skills under `skills/<name>/SKILL.md` and the two engines under `workflows/` at the plugin root, and `generate_plugin_files()` returns the whole 14-file tree as `PluginFile { relative_path, content }`. In plugin mode skill names drop their `rdm-` prefix (the `rdm:` plugin namespace already disambiguates, so the skill is `rdm:roadmap` rather than `rdm:rdm-roadmap`), skill bodies name engines by the namespaced form `rdm:rdm-wf-dispatch-phase` instead of by a `.claude/workflows/…` path — **every** mention is rewritten, including the operative "invoke the Workflow with `{ … }`" instructions in the `autopilot`, `dispatch-phase` and `do` shims, so a plugin-installed shim always dispatches through the namespace the runtime actually resolves (engines this distribution does not ship, such as `rdm-wf-estimate`, stay un-namespaced, since namespacing them would name a plugin entry that does not exist) — and every shim that needs an rdm binary carries a note on how to resolve one from a plugin install (`--rdm-bin`, then `RDM_BIN`, then `PATH`, with an actionable error if none resolves). Engine names keep their `rdm-wf-` prefix, which is what keeps the emitted skill names and engine names disjoint. Existing `rdm agent-config claude --skills` output is byte-for-byte unchanged.
- `rdm agent-config claude --plugin --out <dir>` now writes that plugin tree to disk: the manifest at `<dir>/.claude-plugin/plugin.json`, the eleven skills at `<dir>/skills/<name>/SKILL.md`, and the two workflow engines at `<dir>/workflows/<name>.js`. `--plugin` is its own emission mode alongside `--skills` (mutually exclusive with it — pass one or the other, not both) and is Claude-only. It requires `--out`; it cannot be combined with `--user`, since a plugin reaches `~/.claude` by installation (`claude plugin marketplace add` / `claude plugin install`), not by rdm writing into the user directory. `--skills --user` is unaffected and keeps working exactly as before. A new hermetic harness, `scripts/verify-plugin-distribution.sh`, checks the emitted plugin tree's self-consistency (layout, naming, and that every skill's workflow reference resolves to a real file in the same emitted tree).
- rdm now ships as an **installable Claude Code plugin**. The generated plugin tree is checked into the repo at `plugins/rdm/` and a marketplace entry at `.claude-plugin/marketplace.json` points at it, so a consumer can run `claude plugin marketplace add edpaget/rdm` followed by `claude plugin install rdm@rdm` and get the eleven `rdm:<name>` skills plus the two `rdm:rdm-wf-<engine>` Workflow engines — offline, with no auth. `plugins/rdm/` is generator output and must never be hand-edited; regenerate it with `env -u RDM_ROOT -u RDM_PROJECT cargo run -q -- agent-config claude --plugin --out plugins/rdm` (the omitted `--project` is load-bearing — it keeps the generic `--project <PROJECT>` placeholder in every skill body instead of baking in rdm's own project name). Two new harnesses keep it honest. `scripts/verify-plugin-install.sh` is hermetic pure POSIX shell with no external-CLI dependency and runs in CI: it gates the checked-in tree against generator output behind a **version-normalized** drift diff — the manifest `version` is normalized on both sides so a release-time crate bump can never red-light `main`, while a separate assertion against *freshly generated* output is what catches a stale manifest — plus marketplace shape and `source` resolution (with a non-empty-entry floor, since `claude plugin validate --strict` false-passes a dangling `source`), workflow byte-identity, and the eleven-skill inventory with frontmatter validity, each behind a planted-corruption self-test. `scripts/observe-plugin-install.sh` is the developer-run half — it needs the `claude` CLI, so it deliberately sits outside CI's `scripts/verify-*.sh` glob — and performs a real offline `validate --strict` → `marketplace add` → `install rdm@rdm` into an isolated `CLAUDE_CONFIG_DIR`, asserting the installed skill inventory and workflow bytes on the filesystem and proving the invoking user's real `~/.claude` is untouched; it exits `2` with a NOTICE (distinct from both pass and fail) when `claude` is absent. Consumer install instructions, the regeneration rules, and a full run transcript live in `docs/plugin-distribution.md`.

### Fixed

- `README.md`'s installation directions now cover plugin-based distribution. The Installation section gained a "Wire it into your coding assistant" step that leads with `claude plugin marketplace add edpaget/rdm` / `claude plugin install rdm@rdm` as the recommended path, so a reader no longer has to reach the "AI Agent Integration" section further down to learn the plugin exists; the previous "ask your assistant to run `rdm --help`" advice is retained for other assistants and for consumers who cannot use the marketplace. The pinned-release example was refreshed from the long-stale `v0.6.2` to `v0.18.2`.
- Corrected three stale claims about the `rdmBin` runtime argument left behind by the 0.18.2 change that made it optional. `README.md` described it as **required** and told the reader they must "supply one of these options to resolve the binary"; `docs/plugin-distribution.md` § "Decision 4: Runtime Arguments Delivery" still declared it `REQUIRED` with "no ambient fallback; an absent key throws before the first `agent()` call", said the engines "continue to fail fast if `rdmBin` is missing", and carried a decision-table row reading "Required". All now state the shipped contract: omitting `rdmBin` defaults to a plain `rdm` on `PATH`, an explicit value is used verbatim, and only a present-but-non-string value is refused. The `--rdm-bin` flag and `RDM_BIN` environment variable are described as overrides for a binary that is not on `PATH` rather than as setup every consumer must perform, and Decision 4 now defers to `docs/workflow-schemas.md` § "Environment args" as the canonical contract instead of restating it.
- `README.md` no longer tells readers that the skill shims emitted by `rdm agent-config claude --skills --out <dir>` "carry rdm's own dogfood values, so adjust those two arguments for the target repo". They do not: the emitted shims bake in the `--project` supplied at emission time and invoke a bare `rdm`, so no post-emission editing is required. The raw-emission fallback paragraph now says what the fallback actually costs a consumer — 13 loose file copies with no collision protection or namespace prefixing, self-managed workflow discovery, and an assumption that rdm is on `PATH`, since unlike the plugin shims the raw shims carry no binary-resolution section.

## [0.18.2] - 2026-08-03
### Changed

- The `rdmBin` argument accepted by the `rdm-wf-dispatch-phase`, `rdm-wf-review-refute-fix` and `rdm-wf-estimate` Workflow engines is now **optional and defaults to a plain `rdm` on `PATH`**. Previously it was required and fail-closed: a caller that omitted it got an error before the first agent dispatch. That made the plugin-installed lane unusable out of the box, since a consumer who installs the `rdm` plugin has no repo-local build path to supply and `PATH` is the right answer for essentially every consumer. You can now install the plugin and dispatch a phase or task without configuring anything. An explicitly passed path still wins verbatim, the explicit `"rdm"` sentinel still works unchanged, and a present-but-non-string value is still rejected rather than silently falling back to `PATH`. The skills resolve the binary in the order `--rdm-bin` → `$RDM_BIN` → `rdm`, and the `rdm-autopilot` skill no longer refuses to start when `--rdm-bin` is absent.

- Documented rdm's **recommended distribution path** for downstream consumers: plugin marketplace installation (`claude plugin marketplace add edpaget/rdm` then `claude plugin install rdm@rdm`) over raw skills emission (`rdm agent-config claude --skills --out <dir>`). The plugin provides automatic namespace prefixing, collision protection, and workflow discovery; raw emission is retained as a fallback. Updated `README.md` § "AI Agent Integration" and `CLAUDE.md` with distribution guidance, and reconciled stale workflow-count references across documentation (workflow-schemas.md, mechanical-agent-inventory.md, CLAUDE.md) from "three distributed workflows" to the correct count of two (rdm-wf-dispatch-phase and rdm-wf-review-refute-fix). Corrected the documented rdmBin resolution mechanism to reflect the actual implementation: resolve via `--rdm-bin` flag first, then `RDM_BIN` environment variable, then `PATH` lookup (removed non-existent "standard installation locations" fallback). Added `docs/plugin-distribution.md` § "Which copy runs?" to explain the three distribution surfaces: the authoritative templates in `rdm-core/src/templates/`, the emitted plugin tree consumers install, and this repo's local `.claude/` lane, with precise categorization of which `.claude/workflows/` files are generated (with hand-copied trailing driver blocks) versus hand-maintained byte-checked blocks versus pure hand-maintained copies. Documented why this repo never installs its own plugin (development-build requirement and drift-gate root for generated artifacts).

## [0.18.1] - 2026-08-03
### Added

- The autonomous lane `rdm agent-config claude --skills --out <dir>` emits is now **verified to work in an arbitrary consumer repo**, not merely verified to match this repo's own copies byte-for-byte. `scripts/verify-agent-config-distribution.sh` stands up a hermetic non-rdm, **non-Rust** fixture repo — a Python/TypeScript source tree with real feature branches, its own `rdm init`-seeded plan repo, its own project name, and its own rdm executable path — emits the lane into it, extracts importable modules from the **emitted** engine scripts (with an inverse transform proving each copy is byte-identical to the emitted file, and a proof that the untransformed file genuinely does not import), and then executes their pipeline logic there: every review signal fires on the fixture's own diff and all seven code dimensions are selected (against a docs-only control that selects only the always-on pair), every rdm command the engines build names the fixture's binary and honors the project-flag allow-list with zero `./target/debug/rdm` or `--project rdm`, and three of those built commands are really run against the fixture plan repo and must exit 0 with the expected JSON shape. Four planted corruptions in the emitted bytes prove none of it is vacuous. The harness now requires `node` (on `PATH` or via `mise exec node --`).
- `rdm agent-config claude --skills` now cleans up superseded `.claude/workflows/` files left over from an earlier emission of the same lane, once it recognizes a file's exact prior content: it reports `Removed <path>` for a file whose bytes match a known previously-emitted fingerprint, and `Skipped <path> (content modified since emission; left in place)` for a same-named file it does not recognize (user-edited, or from a version of rdm it doesn't know about) — the latter is never deleted. A removal failure (e.g. a permission error) is reported as `Failed to remove <path>: <error>` and never fails the overall emit. This only runs for Claude Code project output (`--out`, not `--user`; Pi has no `.claude/workflows` at all). The shipped superseded-file table now carries the two engines renamed by the `rdm-wf-` prefix change below, plus the retired `autopilot.js` orphan.
- The `plan`-mode review's always-on dimension set is now **correctly documented as three** — `coherence`, `architectural-fit` and `restraint`. `restraint` carries no `when` predicate and has always run on every plan review; `CLAUDE.md`, `docs/workflow-schemas.md`'s dimension table, and the shipped plan-review skill templates' finding template (`concern: <coherence|architectural-fit|restraint|unit-of-work>`) all said otherwise. No behavior changed — only the description of it.
- The shipped **code-review** skill templates now state **why `ac` and `correctness` are not merged into one always-on finder**: `ac` is the only dimension resolving the AC-review schema rather than the findings schema, and its per-criterion table is the structured side-channel the verdict consumes directly — a channel that never reads a finding's severity, is never refuted, and never consumes refutation budget. Folding it into a shared findings stream would route the acceptance-criteria contract through exactly the path it was kept out of. Rendered into the code-mode skills only; the long-form rationale lives in `docs/workflow-schemas.md`.
- A new on-demand **finder-collapse harness** answers, on evidence, whether `plan` mode's three always-on finders can be collapsed into ONE agent holding three lenses. `scripts/mine-plan-finder-corpus.mjs` recovers real plan REVIEW UNITS verbatim from the full-fidelity `subagents/workflows/<runId>/agent-*.jsonl` transcripts — plan-mode prompts interpolate the plan document inline, so both the document and the three-finder output are replayable — keyed on the same `(runId, unitIdent)` boundary `docs/token-baseline.json` § `refuterFanout` documents, and never adjudicates. `scripts/lib/finder-collapse.mjs` + `scripts/run-finder-collapse.mjs` build both arms (arm A through the REAL exported `findPrompt` over the REAL always-on `DIMENSIONS.plan` entries — never a copy; arm B through a collapsed three-lens prompt that lives in the instrument so a no-ship leaves the lane byte-unchanged), dispatch them with replicates via `claude -p`, and score per-lens finding counts, per-lens severity distribution, adjudicated material recall, `concern` attribution validity and per-class token totals — never blending a rate across lenses (enforced by a recursive key assertion). Modes that spend nothing: `--dry-run`, `--dispatch-stub`, `--score`, `--audit`. The miner's accounting identity — every plan-finder record is either recovered as an always-on lens observation or counted in exactly one skip bucket, since an unrecovered finder is an unknown and an unknown must never contribute to a rate — holds under `--limit` too: a unit past the limit is classified into a single `beyond-limit` bucket before any other test, rather than the boundary unit's records and every later unit being abandoned uncounted. `scripts/verify-finder-collapse.sh` gates all of it hermetically with planted-mutation self-tests, including one that reverts that bucket to a bare `break` and must break the under-truncation identity.
- `docs/finder-collapse.md` records the resulting decision — **`no-ship`, and the review pipeline is unchanged** — with its six-criterion rule pre-registered in an earlier commit than the run it judges. Over 8 real review units x 2 replicates x 2 arms (64 paid dispatches, `opus` both arms), the collapsed finder lost adjudicated material findings in **all three lenses independently**: `coherence` -4 (18.2 %), `architectural-fit` -11 (73.3 %), `restraint` -10 (58.8 %), against a tolerance of one finding, and 9 of arm A's 19 material `blocking` findings. Both arms' findings were equally material under hand adjudication (83/83 rows, complete coverage) — the discriminator is **recall, not precision**. Attribution was perfect (57/57 arm-B findings carried a valid lens key) and tokens fell 55.1 %, and neither of those can carry a ship: the rule is an AND over all six criteria and states that the token criterion alone is never a ship. The scorer additionally refuses to let criteria 2-4 pass while adjudication coverage is incomplete, so an empty adjudication yields `no-ship` rather than a vacuous pass. Machine-readable figures land in `docs/token-baseline.json` § `planFinderCollapse`, audited corpus-free by `node scripts/run-finder-collapse.mjs --audit docs/token-baseline.json`, and a decision/pipeline XOR in the harness keeps a half-landed merged dimension from ever coexisting with that figure.

- The refuter-agreement harness now also A/Bs refuter **shape** — one refuter per gating finding versus one per dimension over that review unit's gating findings — alongside the model question it already answered, and `docs/refuter-batching.md` records the outcome: **`no-measurement`, and the review pipeline is unchanged.** A batched dispatch is one *review unit's* findings for one dimension (`buildReviewPipeline` runs once per unit), so the corpus is grouped by `runId | unitIdent | mode | dim.key`; under that key the committed 56-item corpus yields 35 groupable items in 29 groups — 24 singletons, 4 pairs, 1 triple — i.e. exactly **1 qualifying (size >= 3) group / 3 items against pre-registered floors of 6 groups / 18 items**. A batched arm built from that would be byte-for-byte a per-finding arm across most of its items, so the anchoring effect the experiment exists to detect would be unobservable; no paid A/B was run, and the phase reports a no-measurement outcome rather than a pass. (The earlier `(runId, mode, dim.key)` framing reported four size->=3 groups; those figures are void — two are an artifact of the 12 `constructed` items collapsing into one pseudo-run, and the only non-constructed triple spans two different review units. Both histograms are recorded, the naive one under `supersededNaiveKey`.) New zero-spend command: `node scripts/run-refuter-agreement.mjs --batch-power`.
- New refuter-agreement flags supporting the above, all documented in `--help`: `run-refuter-agreement.mjs` gains `--batch-power`, `--min-batch-group <n>`, `--shape per-finding|batched|both`, `--allow-underpowered` (which stamps a `NO MEASUREMENT` banner and suppresses any decision line, so an underpowered arm can never be mistaken for a pass) and `--audit-section refuterModelTiering|refuterBatching`; `mine-refuter-corpus.mjs` gains `--min-group-size <n>` (emit only candidates whose unit-scoped group has at least n members, so a costly hand-adjudication pass buys only power-adding items) and `--exclude-corpus <path>` (skip already-adjudicated ids so a re-mine appends rather than duplicates), and newly mined items carry `provenance.agentIndex` so a REWORK re-review can be split off a first-round batch instead of silently inflating its size. The scorer reports the two shapes as independent arm buckets with `cost.dispatches` counted by unique dispatch id, `cost.meanTokensPerDispatch` and `cost.meanTokensPerGradedFinding`, plus a new `## ANCHORING` block (`allSameVerdictShare` and refutation rate by position within a batch) computed over qualifying groups only — with false negatives and false positives still on their structurally different denominators and still never blended. Machine-readable figures land in `docs/token-baseline.json` § `refuterBatching`, audited corpus-free by `node scripts/run-refuter-agreement.mjs --audit docs/token-baseline.json --audit-section refuterBatching`.
- `docs/workflow-vs-prose-boundary.md` records where the autonomous lane's workflow-vs-prose boundary goes, so a new script can be classified without re-litigating the decision. It states the rule as five criteria a unit must meet to stay a Workflow script (fan-out-shaped, mechanism rather than policy, headless, deterministic/resumable, hermetically gatable) plus the anti-criterion that decides the autopilot drive loop — a low-iteration sequential driver with negligible fan-out buys nothing from the workflow runtime and pays for it on the part that changes most. It records the two runtime constraints that shape every such answer (the runtime cannot `import`/`require`, and `workflow()` nesting is capped at one level), and gives a disposition with a stated reason for all eight `.claude/workflows/*.js` scripts (`autopilot.js` moves to prose; the other seven stay, `spike-agent-type.js` as an explicit exempt spike artifact) on an axis kept separate from distribution — only three of the eight are emitted downstream, and the five local-only ones all reference an `agentType` a downstream tree has no definition for. It notes the known cost — retiring the JS loop loses `scripts/verify-workflow-autopilot.sh`'s hermetic coverage, which is a first-class phase rather than a cleanup afterthought — and records the two arguments that are deliberately NOT part of the decision: this is not `program-driven-orchestration` (deferred, not superseded, because a headless `claude -p` / Agent SDK orchestrator bills against API usage and is incompatible with Claude subscription billing), and it is not justified by "liveness" (refuted by run `wf_52b569c9-9b4`'s transcripts, where the stalled implementers were actively running rather than backgrounded and had wrongly concluded their work was committed). `CLAUDE.md`'s two-surfaces decision rule now cites the criteria and points at the doc, with autopilot appearing only as an in-flight migration note.
- A new on-demand **refuter-agreement harness** decides, on evidence, whether the autonomous lane's refuters must stay on Opus. `scripts/mine-refuter-corpus.mjs` recovers real historical findings **verbatim** from the full-fidelity `subagents/workflows/<runId>/agent-*.jsonl` transcripts (the 401-character truncation applies only to the `wf_*.json` sidecar previews, not the transcripts) and always emits `groundTruth: null`, because scoring Opus against its own past verdicts would be circular. `scripts/run-refuter-agreement.mjs` regenerates every prompt through the REAL `refutePrompt`, sha-verifies it against the mined original, and dispatches replicates per tier via `claude -p` — with `--dry-run`, `--dispatch-stub`, `--score-only` and `--audit` modes that spend nothing. `scripts/lib/refuter-agreement.mjs` holds the corpus schema and a scorer that reports **false-negative and false-positive rates separately**, over structurally different denominators, with no blended-accuracy field anywhere (enforced by a recursive key assertion) and per-tier token classes plus tool-call means on the same report rows, so the already-measured "cheaper model, same tokens" effect stays visible.
- A checked-in, adjudicated finding corpus at `tests/fixtures/refuter-agreement/corpus.jsonl`: 56 items, 78.6 % mined from real production runs and 21.4 % constructed only to top up under-covered classes, each carrying provenance, a ground-truth class from a closed set, an `authoritative`-vs-`judgement-call` authority flag, and the pinned commit it was adjudicated against. The `mechanically-true-not-a-defect` class — where the two tiers actually diverged — is deliberately over-represented at 42.9 %. `scripts/verify-refuter-agreement.sh` gates all of it hermetically, never dispatching a paid agent, with planted-mutation self-tests. That includes the **real paid-dispatch path** that the dry-run and stub modes bypass but the recorded decision was computed from: `parseClaudeResult` is driven as a pure function over synthetic `claude -p --output-format json` bodies (last-StructuredOutput-wins, bare-JSON and prose/fence-wrapped `result` strings, a missing `usage` object, the `num_tool_uses` fallback, and a non-boolean `refuted` that must bucket as `ungraded` instead of coercing to `false` and inflating the false-positive rate), and `claudeDispatch`'s success / non-zero-exit / non-JSON-body / missing-binary branches run against PATH-shadowed fake `claude` binaries. The miner is covered to the same standard, because its skip branches decide how much history reaches the corpus at all: a hermetic sidecar fixture exercises all six degradation paths (`no-transcript`, `no-prompt`, `unparseable-finding`, `unrecoverable-mode`, `unrecoverable-dim`, `no-verdict`) with an accounting identity asserting `recovered + skipped == refuter records` so none can become a silent drop, plus its full CLI surface (`--severity` singly and as a comma-set, `--until` in both directions, `--limit`, `--out`, `--help`, and every argument-validation error, each of which must now be an actionable named message rather than a raw stack trace).
- `docs/refuter-model-tiering.md` records the resulting decision — **keep Opus, change nothing** — with its supporting numbers and its decision rule stated before those numbers. On the recorded run — a stratified 8-item × 2-tier × 2-replicate subset of the corpus, 16 trials per tier, not a corpus-wide estimate — Sonnet's authoritative-only false-negative rate was 2/5 (40.0 %) against Opus's 1/6 (16.7 %) (every Sonnet false negative landing on a still-true `real-defect`), it was worse on the divergence class, and it spent **89.2 % more tokens per trial** with more tool calls — so the cheaper tier is neither safer nor cheaper in volume. No model binding changed, so no `verify-workflow-*.sh` criterion needed updating; `scripts/verify-workflow-review.sh` gains a pointer comment near §5b-mechanical and the new gate enforces that as an XOR. The doc carries per-class and authoritative-only breakdowns, self-consistency flip rates, and an explicit `## Limitations` section. It also answers the long-contested `plan-review.js` model-omission question: the omission is an **oversight**, not policy — `f4e89d7` and `scripts/verify-workflow-review.sh` §5b-mechanical govern only the MECHANICAL pin, while the sibling `dispatch-phase.js`, the `[models]` config policy, and `CHANGELOG.md`'s own "closing the session-model-inheritance leak" entry all point the other way. Machine-readable figures land in `docs/token-baseline.json` § `refuterModelTiering`, audited corpus-free by `node scripts/run-refuter-agreement.mjs --audit docs/token-baseline.json`.
- A new dev tool, `scripts/measure-lane-tokens.mjs` (over `scripts/lib/token-report.mjs`), measures token usage across Claude Code Workflow lane runs. It locates every `wf_*.json` session sidecar under a `--root` (default `~/.claude/projects`, searching every project-slug directory including `--worktrees-`-named ones), joins each run's agents with their `agent-*.jsonl` transcripts (deduping usage by `requestId`), and reports totals broken out by token class (output / uncached input / cache write / cache read) grouped by agent class, full label, model, and workflow — plus an explicit, never-reconciled discrepancy line between the sidecar's own `totalTokens` and the deduped sum. Invoke it with `--since <iso-date>`, `--workflow <name>` (repeatable, OR'd), and `--format text|json`. It is the measurement tool later phases of the `workflow-token-reduction` roadmap use to substantiate their token-saving claims. Stdlib-only Node, no packages; hermetic regression in `scripts/verify-token-report.sh`.
- `scripts/measure-lane-tokens.mjs` now also reports a **per-agent-class first-request floor** (`floorByAgentClass`): the n/min/p10/median/mean of each measured agent's first transcript request only (uncached input + cache write + cache read, before any tool use) — the same quantity `docs/token-baseline.json`'s whole-corpus `agentContextFloor.measuredFloor` is defined over, now broken out per agent class instead of only as a single global median. `cached`/sidecar-only-fallback records (no per-class split recoverable) are excluded from the floor rather than counted as zero, and a class with no eligible records is omitted from the output rather than reported with `n: 0`. Surfaced in both `--format json` (a new `floorByAgentClass` key) and `--format text` (a new "-- Per-agent-class first-request floor --" section). `scripts/verify-token-report.sh` gained fixture-backed assertions and two new planted-mutation self-tests covering it.
- `docs/token-baseline.md` and its machine-readable twin `docs/token-baseline.json` commit a measured "before" snapshot of the six autonomous lanes' (autopilot, dispatch-phase, plan-review, backlog, estimate, document) real token spend, broken down per agent class and per model with cache reads included. It reconciles the corrected per-class figures against the roadmap body's original sidecar-`tokens`-field survey (the review-vs-implementation ratio narrows from ~11:1 to ~1.67:1 once cache reads are counted), reports a directly-measured ~38.8k-token per-agent context floor attributed between `CLAUDE.md` and tool schemas/system prompt, and documents the confounds (roadmap size, phase difficulty, rework rounds) that make raw per-run totals non-comparable across lane runs. Later phases of the `workflow-token-reduction` roadmap diff their savings claims against this baseline.
- `docs/token-baseline.json` gained a `floorByAgentClass` addendum: the same per-agent-class first-request floor now surfaced by `scripts/measure-lane-tokens.mjs`, regenerated over the existing on-disk corpus (44 runs / 2,110 records — no fresh dispatch) with every mechanical agent class (`fetch`, `stamp`, `model`, `diff`, `gate`, `advance`, `park`) plus the mixed mechanical/judgment `act` and `estimate` classes represented, so the roadmap's later mechanical-elimination phase has real, correctly-caveated per-class "before" evidence instead of only a whole-corpus median. The existing `byAgentClass`/`runSet`/`agentContextFloor`/`totalsDiscrepancy`/`warnings` sections are unchanged; `methodology.ac5RegenerabilityStatus`/`regenerateCommandScope` are updated to note `floorByAgentClass` is now CLI-reproducible while the whole-corpus `agentContextFloor` still is not.
- A companion dev tool, `scripts/measure-hoist-delta.mjs`, measures the
  mechanical-subagent reduction directly rather than by applying an elimination
  rule on paper: it executes the real, post-change `dispatch-phase` driver under
  a recording fake `agent` — once with a pre-change caller's arguments and once
  with the arguments the post-change skill shim passes — and counts the
  subagents each run actually spawns, then prices them using
  `docs/token-baseline.json`'s own measured per-class figures. It reports both a
  raw and a fresh (ex-cache-read) token column, since cache reads dominate the
  raw totals and are the cheapest token there is. Its `--check <doc>` mode
  asserts the figures it computes appear verbatim in
  `docs/mechanical-agent-inventory.md`, so that document cannot drift into a
  stale hand-transcription; `scripts/verify-workflow-dispatch.sh` section 8 runs
  that check in CI with planted-mutation self-tests. Stdlib-only Node, no
  packages.
- A new dev tool, `scripts/measure-refuter-severity.mjs` (over the same
  `scripts/lib/token-report.mjs`), breaks refuter token spend out by the
  **severity of the finding each refuter graded** — the dimension the single
  `refute` agent-class bucket cannot see. It recovers each finding from the
  refuter's own transcript (brace-matching the embedded finding JSON, so a
  review target that itself contains braces still parses) and its verdict from
  the forced `StructuredOutput` call, then reports agent count, verdict tally,
  refutation rate, and all four token classes per severity, plus the projected
  drop from skipping non-gating refutation. `--until` pins the measurement
  window so a committed figure cannot be silently re-baselined by a later lane
  run; `--check <doc>` recomputes over the corpus and asserts a document's
  recorded figures match; `--audit <doc>` checks a document's numbers for
  internal consistency without reading any sidecars, so the committed figures
  are gated on any machine. `scripts/verify-token-report.sh` section 6 gates it
  against a hermetic fixture with two planted-mutation self-tests. The measured
  result is recorded in `docs/token-baseline.json` §
  `nonGatingRefutationSkip` and summarized in `docs/token-baseline.md`.
- `scripts/measure-refuter-severity.mjs` gained two more descriptive
  distributions — **review fanout** — over the same corpus: findings-per-finder
  (n/min/p50/p90/max, split by mode and dimension, read from each finder's own
  `StructuredOutput` output rather than inferred from refuter counts, since a
  `suggestion` finding is never dispatched to a refuter at all) and
  refuters-dispatched-per-review-unit (n/min/p50/p90/max plus a recovery
  rate). Both a refuter's dimension and its review-unit identity are parsed
  from the same prompt header line the severity extractor already reads
  (`A prior reviewer raised this <dim> finding against <target>:`) via a new
  `extractRefuterContext()`, never from the refuter's own
  `refute:<mode>:(f.id|dim.key:idx)` label, which a finder-supplied `f.id`
  routinely displaces the dimension out of. The review-unit boundary is
  deliberately **not** `phaseTitle`/`phaseIndex` — those are the workflow's
  own declared pipeline stages and collapse an entire plan-review run's
  distinct review units into one bucket (measured directly: 96 refuters at one
  `phaseIndex` across 9 real units on a reference run) — so the unit key comes
  exclusively from the target embedded in each refuter's own prompt, with a
  target that is itself pretty-printed JSON (the `--implementation-plan`
  shape) rejected rather than captured as a fake identity. A retried
  dispatch's Workflow-runtime-suffixed label (`dim (retry N)`) is normalized
  before grouping so a retry pools into its non-retried siblings' row instead
  of fragmenting into its own zero-`refutersDispatched` row. Recorded in
  `docs/token-baseline.json` § `refuterFanout` (48-run window ending
  2026-07-29, 2,208 agent records, not subtractable against the per-agent-class
  baseline above — the corpus grew between measurements) and summarized in a
  new `docs/token-baseline.md` § "Phase 1: review fanout". Both `--check` and
  `--audit` now validate this section alongside `nonGatingRefutationSkip` in
  one pass. `scripts/verify-token-report.sh` section 6 gained a fixture
  extension (two finders and two refuters, including a dimension-shadow
  refuter whose `f.id` names a different real dimension than its finding's
  own — proving dimension resolution reads the prompt, not the label) and two
  more planted-mutation self-tests.
- `scripts/measure-refuter-severity.mjs` gained a third distribution —
  **determining-finding rank** — which answers the one question a refutation
  cap lives or dies on: where in a ranked candidate list does the finding that
  actually determined the outcome sit? It replays the live pipeline's own rule
  by **importing** `rankFindings` / `survives` / `hasBlocking` from
  `.claude/workflows/lib/review.mjs` (read-only; no lane file is modified and
  no local severity table or confidence floor is kept), so the measurement
  cannot drift from the behavior it predicts. Ranking is over each unit's full
  **candidate** list, not its survivor list: severity sorts first, so
  rank-among-survivors would be a constant 1, whereas a cap truncates the
  candidate list. The review-unit key is phase 1's, unchanged — a new
  `extractFinderContext()` reads the identical prompt-embedded `context.target`
  from the finder side (`Review target: <target>.`), with no `phaseTitle` /
  `phaseIndex` anywhere. Each unit resolves to exactly one of `determining`
  (with a rank), `non-determining` (fully resolved, nothing gated — a distinct
  row) or `unrecoverable` under a closed, strictly **per-unit** reason
  vocabulary; an agent whose unit identity cannot be resolved is attributable
  to no unit, so it invalidates none and is instead counted as an
  `orphanAgents` diagnostic with a stated residual-risk bound. Nothing is
  imputed, unrecoverable units are excluded from every within-top-N numerator
  and denominator, and the recoverable share is restated beside every headline.
  Also reports candidate-set sizes, a `largeTier` sensitivity variant (the tier
  is embedded in neither prompt and is therefore not recoverable), and an
  `acTableGapUnits` diagnostic for the AC-table side channel. The
  supports/kills conclusion is **derived**, not asserted: an exported
  `CAP_VERDICT_RULE` + `deriveCapVerdict()` produce `supports-cap` /
  `kills-cap` / `inconclusive`, and `--audit` re-derives the verdict from the
  doc's own numbers. Over the same 48-run window ending 2026-07-29 (2,208
  agent records) the result is recorded in `docs/token-baseline.json` §
  `determiningFindingRank` and read in prose in a new
  `docs/token-baseline.md` § "Phase 2: rank of the determining finding":
  **the evidence supports a cap at N = 5**. `scripts/verify-token-report.sh`
  gained a new section 7 over a purpose-built fixture tree
  (`tests/fixtures/token-determining-rank`), plus a direct drive of the
  reason-resolution logic covering the whole closed vocabulary — including the
  three reasons the corpus fixture cannot reach (`multi-round-unit`,
  `ambiguous-finding-join`, `unreadable-finder-transcript`), the load-bearing
  precedence order between them and `dimension-coverage-gap`, and the
  retry-supersession rule that keeps a re-dispatched dimension from being read
  as a second review round — with twelve planted-mutation self-tests.
  The walk checks **severity-eligibility before disposition**: a candidate
  outside `hasBlocking`'s blocker set for the tier being walked (a
  `suggestion` at either tier, a `concern` at the default tier) can never be
  the determining finding whatever its verdict turns out to be, so an
  unreadable verdict on it leaves the unit `non-determining` instead of
  poisoning it to `unrecoverable` — the distinction the section exists to
  keep, and one that really bites on this pre-phase-6 window where
  suggestions were still dispatched to refuters. This moves three units out
  of `unrecoverable`, raising the recoverable share from 82.1 % to
  **85.7 %**; the `supports-cap` verdict at N = 5 is unchanged.
  `deriveCapVerdict` is additionally driven over a synthetic branch table so
  the `kills-cap` outcome — which neither the fixture nor the real corpus
  produces, and which the measurement explicitly treats as a legitimate
  terminal finding — is verified rather than merely reachable.

- `rdm task create` gained a `--no-plan-review` flag: it skips the automatic
  `needs-plan-review` stamp even when the `plan_review` config flag is
  enabled. Intended for tasks filed from a plan-review finding itself, so the
  gate's own output is never fed back into itself as new input to review. The
  standalone plan-review workflow and `rdm-do --auto`'s side-work task filing
  both now pass it automatically.
- The plan-review gate gained a new always-on dimension, **restraint** — the
  counterweight to `unit-of-work`: it flags a plan that specifies an
  implementation decision better left to whoever carries it out, or whose
  level of detail has grown past the point where more of it reduces risk.
- A new local-only agent registry, `.claude/agents/`, holding one definition:
  `rdm-mechanical.md`, a trimmed transcribe-one-command agent with a two-tool
  allowlist. The mechanical `agent()` call sites of the four local-only
  workflows (`document.js`, `backlog.js`, `estimate.js`, `plan-review.js`) now
  dispatch under it, so those lanes' command-transcribing steps run with a far
  smaller context — a measured 8,907 tokens per agent (−23%) less, confirmed by a
  live lane dispatch. Judgment steps (finders, refuters, planners, implementers)
  are deliberately unchanged.
  The definition is **not distributed**: `rdm agent-config` emits skills and
  workflows only, so the three distributed workflows do not reference it.
  Full evidence — including the measured 19,320-token per-agent cost of loading
  `CLAUDE.md` into any subagent, 60% above the previously recorded `chars/4`
  estimate — is in `docs/workflow-schemas.md` § "agentType / effort options
  spike" and `docs/token-baseline.json` → `mechanicalContextTrim`.
- New guards in `scripts/verify-workflow-review.sh`, each with a
  planted-mutation self-test: §2b — no workflow script may pass `effort:`, and no
  *distributed* workflow template may reference `agentType` (an unresolvable one
  raises rather than degrading silently, so it would hard-break every downstream
  lane on first dispatch); §2c — a bidirectional assertion that every mechanical
  call site carries `agentType: 'rdm-mechanical'` and no judgment site does, with
  a completeness sweep that fails if a site is added or removed without updating
  the asserted list.
- `scripts/verify-token-report.sh` gained coverage for `percentile()`'s linear
  interpolation branch, which every existing fixture short-circuited by giving
  each agent class only one record.

### Fixed

- `scripts/verify-token-report.sh` resolves symlinks in its scratch directory.
  On macOS `mktemp -d` returns a path under the `/var` → `/private/var`
  symlink, and every instrument it exercises gates its CLI on
  `path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)`; Node
  realpaths `import.meta.url` but not `process.argv[1]`, so a script copied
  into that scratch directory silently did nothing when run. Every
  planted-mutation self-test in sections 5 and 6 was therefore passing
  vacuously — the fixture comparison failed because the mutant emitted no
  report at all, not because the mutation was caught. The new section 7 also
  asserts each mutant produced a non-empty report before requiring its check
  to flip, so vacuity cannot return silently.

- `autopilot`'s `fetch:next` interpretation (`interpretNext`) no longer silently
  reports a malformed or double-wrapped `rdm next` result as a completed run.
  An agent transcribing `rdm next`'s JSON output was observed re-encoding the
  whole payload as a string inside its own `result` field; that shape fell
  through the old catch-all and was misclassified as "nothing to do", so a run
  with actionable phases remaining stopped early and reported a clean finish.
  `interpretNext` now defensively unwraps a string-encoded `result` (bounded,
  applied uniformly to all three `rdm next` shapes), and any genuinely
  malformed/unrecognized/empty return is classified as a distinct `unparseable`
  stop reason instead of `nothing`. The run summary flags any non-well-known
  stop reason with a loud `*** ABNORMAL TERMINATION` marker (via a new
  allowlist, `isAbnormalStop`, so a future unrecognized reason is fail-safe
  flagged too), and an `unparseable` `fetch:next` failure is also recorded in
  the summary's escalations section (tagged `[fetch]`) — though, since no
  phase stem is known at that point, it is summary-only and does not appear in
  `rdm review blocked`; the summary now says so explicitly right next to the
  `rdm review blocked` pointer, so a reader isn't misled into expecting the
  queue to list it. Making it queue-visible would require an rdm-core
  schema/status-model change (`blocked_phases` is phase-only by design), which
  is out of scope here; tracked as follow-up task
  `surface-fetch-next-escalations-in-blocked-queue`.
- The plan-review round-note reader (`parseRoundNotes`) accepted only
  `blocking` and `concern` bullets while the writer emitted every severity, so
  the first `suggestion` bullet in a previous round's note truncated the rest
  of that round's bullet list — making repeat-finding detection re-list them
  verbatim every pass. The reader now accepts every severity the writer can
  produce. (Reporting-only; outcomes were classified from the live survivor
  set, never the parsed one.)

### Changed

- **BREAKING — every Workflow engine under `.claude/workflows/` is now named
  `rdm-wf-<name>.js`.** The engines used to surface in the skill/slash-command
  listing under bare names that read as siblings of their `rdm-*` skill front
  doors, so `dispatch-phase` and `rdm-dispatch-phase` were indistinguishable
  without reading both files. All six moved at once — a prefix applied to a
  subset would be worse than none, because the *absence* of a prefix would stop
  meaning anything:
  `dispatch-phase.js` -> `rdm-wf-dispatch-phase.js`,
  `review-refute-fix.js` -> `rdm-wf-review-refute-fix.js`,
  `backlog.js` -> `rdm-wf-backlog.js`,
  `document.js` -> `rdm-wf-document.js`,
  `estimate.js` -> `rdm-wf-estimate.js`,
  `plan-review.js` -> `rdm-wf-plan-review.js`.
  Each engine's `meta.name` matches its new filename stem, so the listing entry
  moves with the file.
  **No `rdm-*` skill was renamed** — all 11 front doors (`rdm-do`,
  `rdm-dispatch-phase`, `rdm-autopilot`, `rdm-review`, `rdm-plan-review`,
  `rdm-estimate`, `rdm-backlog`, `rdm-document`, `rdm-land`, `rdm-revise`,
  `rdm-roadmap`) keep their names and invocations; only the engines behind them
  moved. `.claude/workflows/lib/*.mjs` filenames are likewise unchanged.
  **What you must do:** if you invoke a Workflow by name from your own
  automation or prose — a `Workflow` tool call, a custom skill, a script — update
  that name to its `rdm-wf-` form. Invoking a bare engine name now resolves
  nothing.
  **What you need not do:** no manual `rm`. Re-running
  `rdm agent-config claude --skills --out <dir>` **removes the superseded**
  `dispatch-phase.js` and `review-refute-fix.js` from a previously-emitted tree
  as part of the same emit, using the fingerprint-gated cleanup mechanism, and
  reports each as `Removed <path>`. The same emit also removes the long-retired
  `autopilot.js` orphan, which had no successor and no other cleanup path. A
  file you have edited yourself is never removed — it is reported as `Skipped`
  and left in place.

- The `review-refute-fix` and `estimate` workflows no longer hardcode this
  repo's `./target/debug/rdm` binary or `--project rdm` either — the same change
  `dispatch-phase` already landed, applied to the other two engines, so all
  three now honor ONE contract rather than each inventing its own. Both take
  `rdmBin` (**required**, fail-closed, with no PATH fallback — pass the explicit
  sentinel `"rdm"` to opt into PATH resolution) and an optional `project`,
  applied only to project-scoped subcommands: `rdm model resolve` and
  `rdm commit` never carry the flag, while `phase list/show/update`,
  `task update` and `worktree add` do. Both args are validated at parse time, so
  a mis-invocation throws before the first agent is dispatched and costs zero
  tokens.
  - `review-refute-fix`'s two legacy survivors-only shapes — `mode: "plan"`, and
    `mode: "code"` with no item identifiers — are the one documented carve-out:
    they emit no rdm invocations at all, so they keep working with no `rdmBin`
    and return their existing `{ mode, survivors, budget }` result unchanged.
  - The `rdm-review` and `rdm-estimate` skills now pass both args, so nothing
    that works today stops working. One bounded, temporary exception: until the
    prose `rdm-autopilot` loop is parameterized, its `estimate` pre-pass throws —
    **non-fatally**, because that skill already logs a warning and continues
    into its drive loop on an estimate error. Phases then dispatch at whatever
    tier `rdm next` reports rather than a freshly-rated one; autopilot's
    `dispatch-phase` payload is unaffected.
- The `dispatch-phase` workflow no longer hardcodes this repo's
  `./target/debug/rdm` binary or `--project rdm`, so a downstream repo can drive
  the autonomous lane with its own executable and its own project. It now takes
  a **required** `rdmBin` argument — the exact rdm executable to invoke; pass
  `"rdm"` to opt into `PATH` resolution explicitly — and an **optional**
  `project` argument, applied only to project-scoped subcommands (`rdm model
  resolve` and `rdm commit` never receive a project flag; omitting `project`
  emits no flag at all, so rdm's own `RDM_PROJECT`/`default_project` chain
  applies). A `Workflow` invocation that omits `rdmBin` now fails fast with an
  actionable error, before spending a single token, instead of silently running
  whichever `rdm` happens to be first on `PATH`; there is deliberately no
  existence preflight, because a stale global `rdm` would pass one. A `project`
  value that is not a plain name is rejected rather than interpolated into an
  agent's shell prompt. The `rdm-dispatch-phase`, `rdm-do --auto` (both the
  phase and task flows) and `rdm-autopilot` skills all pass the new arguments,
  in both the CLI and MCP variants.

- The local `rdm-autopilot` dogfood skill (`.claude/skills/rdm-autopilot/SKILL.md`)
  no longer hardcodes this repo's `./target/debug/rdm` binary or `--project rdm`
  either — closing the fourth (prose) propagation channel the
  `project-agnostic-lane` roadmap identified, the one no generator, byte-identity
  gate, or `*.js` grep could reach. It now accepts a **required** `--rdm-bin
  <path>` (accepts the literal sentinel `rdm` to opt into `PATH` resolution) and
  an **optional** `--project <name>`, and threads both through: `rdm model
  resolve` still carries no project flag, while `rdm next` / `phase update` /
  `phase show` all do, and both its `estimate` and `dispatch-phase` Workflow
  payloads now pass `rdmBin`/`project` as bare keys instead of this repo's
  literal values. The shipped `skill-autopilot-{cli,mcp}.md` templates were
  already project-agnostic (threaded by the `dispatch-phase` payload change
  above) and needed no change — they invoke no `estimate` pre-pass by design.

- The shipped code-review pipeline now decides **which conditional dimensions
  run from the CONTENT of the diff rather than from rdm-specific file paths**, so
  `api-docs`, `changelog` and `security` fire correctly in any repository and any
  language. Previously these were derived from path lists that were either
  hard-coded to this project's crate layout or matched on a spelling
  coincidence — and because a review that skips a dimension still reports clean,
  the failure was silent. Concretely: `api-docs` now fires when an added line
  introduces an exported or public symbol in any language (`export` /
  `export default`, `module.exports`, `pub` / `pub(crate)`, `public`, a
  capitalized Go identifier, `__all__`) instead of only on a `+pub` line under a
  specific crate path; `changelog` fires when an added line registers a CLI
  subcommand/argument/flag, an attached help or usage string, an HTTP/RPC route
  or tool, or user-visible printed output — instead of on any path merely spelled
  `config` or `mcp`, so a change to `vite.config.ts` no longer trips it; and
  `security` fires on sink-shaped content (process execution, filesystem access,
  environment and secret reads, deserialization or `eval`, raw memory — including
  every Rust `unsafe` shape, the inline `unsafe { … }` expression as well as the
  `unsafe fn` / `unsafe impl` / `unsafe trait` / `unsafe extern` declarations)
  instead of on a security-sounding path name, so a `child_process` sink in
  `src/lib/runner.js` is caught while `src/auth/session.js` with no sink content
  is not. Only ADDED diff lines are scanned, so a *removed* line never trips a
  signal.
- Review coverage is never silently dropped when the diff cannot be read: if a
  change touches code files but their content is unavailable, all three
  conditional signals are set to `true` — an explicit value, never an omitted
  key — so their dimensions still run. A docs-only change, or a readable diff
  that matches nothing, remains a genuine negative and runs only the always-on
  dimensions. The signal-derivation input shape is unchanged
  (`{ targetType, changedFiles, diffText }`) and no configuration option was
  added, so no downstream caller needs to change.
- The `security` dimension of the review skills emitted by `rdm agent-config
  claude --skills` now reviews on a **threat-model** basis instead of matching
  language-specific API and keyword patterns. A finding is a claim that an
  attacker can do something they should not be able to do, backed by the code
  that grants it — explicitly not lint, not style, not "consider using a safer
  API" — and it is worked through five language-neutral categories: injection,
  authorization, memory, crypto, and exposure. Severity is rated on impact
  rather than certainty and maps onto the existing `blocking` / `concern` /
  `suggestion` contract rather than adding a second ladder. Findings may now
  carry an optional `category` slug (`command-injection`, `path-traversal`,
  `unsafe-ffi`, `hardcoded-secret`, `info-disclosure`, …). The dimension is now
  triggered purely by the paths a change touches; the previous
  language-specific diff-content triggers, which could never fire outside one
  language, are gone.
- Every review finder prompt — both code review and plan review, every
  dimension — now carries **prompt-injection hygiene**: the repository under
  review is untrusted data and cannot issue instructions to a reviewer, and
  text telling a reviewer to skip a file, ignore a finding, stop reviewing, or
  claiming the code is already verified or approved is itself reportable as a
  finding. Applies to the review and plan-review skills `rdm agent-config
  claude --skills` emits and to the workflow scripts it ships alongside them.
- The shipped code-review dimensions no longer hardcode rdm's own Rust
  conventions. `correctness`, `architecture`, `api-docs`, `changelog` and
  `security` now state generic intent — the error-handling conventions the
  project states, its layering contract, the documentation its public API
  requires, its changelog rule, and how it requires a use of the language's
  safety escape hatch to be justified — and direct the reviewing agent to read
  the consuming project's own principles document (`docs/principles.md`,
  falling back to `CLAUDE.md` / `AGENTS.md` in the project root) for the
  specifics. A repo in any language now gets a reviewer that enforces that
  repo's rules instead of applying rustdoc, crate-layout and `// SAFETY:` rules
  to it. This is a prose change only: no config key, CLI
  flag, or pipeline input changed, and pointing the reviewer at a different
  project still requires no code change.
- The shipped review pipeline now grades **at most 5 findings per review unit**
  by default. It ranks a unit's gating findings by severity, then confidence,
  and dispatches a refuter only for the top 5; the rest are still reported, but
  pass through un-refuted and marked `unrefutedReason: 'budget'`. The confidence
  floor still applies to them — the budget skips *grading*, never *filtering* —
  so a bounded review can only ever report MORE work to do, never less. Override
  it per run with `maxRefutations` on `dispatch-phase`, `plan-review`, or
  `review-refute-fix`; `0` is legal and means "grade nothing". There is no
  "uncapped" value — pass a large number instead. The default of 5 is measured,
  not guessed: over the recorded run corpus the finding that actually determined
  the outcome was within the top 5 for 100 % of units at the default tier and
  98.2 % at the `large` tier (`docs/token-baseline.md` § "Phase 4: the chosen
  refutation budget").
- A bounded review now says so, everywhere you would look. The pipeline logs how
  many findings it produced, graded, and passed through; the dispatch OUTCOME
  carries a `reviewBudget` field and appends a short
  `[review budget hit: N produced, M graded, K ungraded]` clause to its summary
  (and therefore to the reason recorded on a parked or blocked item, visible in
  `rdm review blocked`); and an autopilot run summary suffixes a `[budget]` tag
  onto that phase's entry in its `phases completed (...)` line. A review that
  stayed under budget reads exactly as it did before. A bound hit on an *early*
  round stays reported even after a later revision resolves it — for plan-revise
  rounds exactly as for code-rework rounds — and when both gates hit, the clause
  reports the later (code) round's counts, not the earlier plan gate's.
- Review findings now carry an explicit provenance marker, so a report can tell
  four cases apart that used to blur together: graded-and-survived (no marker),
  deliberately skipped as non-gating (`unrefutedReason: 'non-gating'`), cut for
  budget (`unrefutedReason: 'budget'`), and **grading crashed**
  (`refuterError: true`) — the last of which previously carried no marker at all
  and was indistinguishable from a verified survivor.
- `rdm-autopilot` no longer delegates the roadmap-driving loop to
  `.claude/workflows/autopilot.js`. Per the `prose-autopilot-orchestration`
  roadmap's phase 2, the skill now drives the loop itself in prose: it invokes
  `estimate` and `dispatch-phase` as `Workflow`-tool calls and runs every other
  step (`rdm next`, `rdm phase update`, `rdm phase show` read-backs, `rdm model
  resolve mechanical`) as direct Bash commands in its own context instead of
  dispatched mechanical `agent()` subagents — those subagents existed only
  because the headless workflow runtime cannot run Bash itself, a limitation a
  live prose skill does not have. The estimate pre-pass now runs through a real
  `workflow('estimate', …)` call for the first time; previously `autopilot.js`
  reached the same fan-out only via a stamped `estimate-core` copy embedded in
  `lib/autopilot.mjs`, never a genuine `estimate` invocation. `autopilot.js`,
  `lib/autopilot.mjs`, and their `scripts/verify-workflow-autopilot.sh` harness
  are left in place and unexecuted by this skill, pending a later phase's
  retirement decision.

- The review pipeline no longer spawns a refuter for a **`suggestion`**
  finding. Severity is the only thing that turns a finding into an outcome, and
  a `suggestion` gates nothing at any tier, so a refuter's verdict on one could
  never change anything. Such findings now pass straight through marked
  `unrefuted: true` — still subject to the same confidence floor — and both act
  steps handle them under an explicit disposition rule: incorporate the ones
  that improve readability or clarity where the change is not major, file the
  ones worth keeping as follow-up tasks, and skip only the rest with a stated
  reason (recordable as a new `skipped` action, with a `reason`, in the
  code-lane `CODE_ACT` schema) — so a real observation can never evaporate into
  a transient skip reason. `blocking` and `concern` keep their refuter
  — over the measured corpus a `concern` is overturned *more* often than a
  `blocking` finding — and the rule is fail-safe: a finding whose severity is
  missing or unrecognized is still refuted. Measured effect over the recorded
  corpus: 239 of 989 refuters (24.2 %, 20.7 % of refuter tokens) would not have
  been spawned. A refuter that *crashes* still keeps its finding and is
  deliberately not marked `unrefuted`.

- The autonomous-lane Workflow scripts now accept **optional caller-supplied
  arguments** so they no longer spawn a dedicated mechanical subagent for work
  the invoking skill already did: `dispatch-phase` takes
  `phaseMeta`/`taskMeta`/`alreadyInProgress`, `autopilot` takes
  `mechanicalModel`/`phaseList`/`next`, `estimate` takes
  `mechanicalModel`/`phaseList`, `plan-review` takes
  `fetched`/`wontFixedTexts`/`mechanicalModel`, `backlog` takes
  `mechanicalModel`/`report`, `document` takes `mechanicalModel`/`roadmapMeta`,
  and `review-refute-fix` takes `diff`. `dispatch-phase` additionally absorbs the
  branch diff into its implementer instead of running a separate diff agent.
  **Every one of these is optional and behaviour-neutral** — invoking a workflow
  directly via the `Workflow` tool with the previous argument shape produces the
  same outcome as before, exercising the unchanged in-workflow fetch. Across the
  measured corpus this removes 115 of 304 mechanical subagents (~38%); see
  `docs/mechanical-agent-inventory.md` for the full census, the classification
  rule, and the measured delta.
- A caller-supplied `phaseMeta` payload for `dispatch-phase` must now carry the
  phase's non-empty `model` difficulty tier (alongside its body and all five
  resolved model ids) or it is rejected and the in-workflow fetch runs instead —
  so the `rdm-dispatch-phase` and `rdm-do --auto` skills now gather and pass the
  tier. The tier is the sole input to the code-review gate's strictness, and an
  absent one silently fell back to `medium`: a caller that supplied everything
  *but* the tier would have had a `large` phase reviewed at `medium` strictness,
  letting a blocking finding that should have forced a rework round pass
  straight to `reviewed`. A task payload carries no tier and is unaffected.
  Rejection is a fallback, never an error — the run proceeds exactly as it does
  with no payload at all.
- The `rdm-plan-review` skill now reads its target with
  `rdm task show|phase show|roadmap show --format json` itself and passes the
  parsed JSON verbatim, instead of asking a subagent to transcribe it. That
  transcription step had twice written junk over a target's real tag list, and
  schema validation could not catch it because both bad returns were
  schema-valid. A caller-supplied payload must carry the target's `tags` array
  (and, for a roadmap, each phase's `stem`/`body`/`tags`) or it is rejected and
  the in-workflow fetch runs instead — the gate replaces an item's whole tag
  list with `--tags`, so an incomplete payload would silently clear it.
- The distributed `rdm-plan-review`, `rdm-backlog`, `rdm-document`, `rdm-review`
  and `rdm-estimate` skills are **not** yet Workflow shims, so they continue to
  use each workflow's own in-workflow fetch — correct, just not yet cheaper.
  Converting them is tracked by task
  `convert-remaining-skill-templates-to-workflow-shims`. MCP skill variants
  likewise omit the model-derived arguments, since there is no MCP
  model-resolve tool.

- The plan-review gate's `coherence` dimension no longer blocks a plan merely
  because it leaves an implementation decision undecided. A plan may delegate
  decisions to whoever carries it out; an undecided point is now a `concern`
  unless the undecided branches would lead to different goals or outcomes.
  Coherence is `blocking` only when an implementer following the plan as
  written would build the wrong thing.
- The standalone plan-review workflow now caps repeated review rounds on the
  same item: each non-`reviewed` pass records a `## Plan Review Round <N> —
  <outcome>` audit note on the item's body. A finding that is still genuinely
  unresolved keeps the item in `rework`/`escalated` on round 2 exactly as on
  round 1 — repeats are only de-duplicated in the human-facing note, never in
  the pass/fail decision — and a third round escalates to a human instead of
  looping further. The cap only fires on findings that are still unresolved: a
  plan that is genuinely clean by the third pass is reported `reviewed`, not
  escalated. A finding already resolved `wont-fix` on a prior pass is dropped
  from both the report and the outcome, and is never re-raised.

### Changed

- `rdm agent-config claude --skills`/`--mcp` no longer emits
  `.claude/workflows/autopilot.js`: the autopilot loop is now the prose
  `rdm-autopilot` skill, which orchestrates the `dispatch-phase` and
  `estimate` Workflows directly instead of nesting through a stamped
  `autopilot.js`/`lib/autopilot.mjs` copy. The emitted workflow-file count
  drops from 3 to 2 (`dispatch-phase.js`, `review-refute-fix.js`).
- The shipped `rdm-autopilot` skill templates (`rdm agent-config claude
  --skills`/`--mcp`) now carry the same full prose-parity orchestration
  content as the local dogfood `rdm-autopilot` skill, replacing the interim
  "full prose-parity documentation lands in a follow-up phase" placeholder:
  the budget-check semantics (a shared, rework-inclusive dispatch counter
  defaulting to 50), the fetch/classify/work/park drive loop, the
  advance/park read-back confirmation retried up to 2 times, and the exact
  summary format with its known-good stop-reason allowlist (`nothing`,
  `blocked-on-dependencies`, `budget`, `plan-only-exhausted`,
  `mechanical-model-unresolved`). The MCP variant gains a new
  `rdm_phase_show` tool (surfaced via a new `{t_phase_show}` template
  placeholder) so it can read a phase back after an advance/park write,
  mirroring the CLI variant's `rdm phase show --format json` call — MCP has
  no equivalent of that command otherwise.
- The shipped `rdm-autopilot` skill templates (`rdm agent-config claude
  --skills`/`--mcp`) no longer run an `estimate` Workflow pre-pass before
  dispatching phases downstream: every downstream phase now dispatches at
  whatever tier `rdm next` already reports (default `medium`) rather than
  being freshly rated first. `generate_workflows()` never emitted
  `.claude/workflows/estimate.js` in the first place — it references
  `agentType: 'rdm-mechanical'`, which a downstream repo's (nonexistent)
  `.claude/agents/` registry has no definition for and which raises rather
  than degrading silently — so the templates' prior instruction to invoke it
  was a dangling reference the very first dispatch would have hit. The step
  numbering shrinks accordingly (the phase-cursor fetch, drive loop, and
  summary steps renumber down by one), the `mechanicalModel`/`phaseList`
  hoists (and, on MCP, the `rdm_phase_list` tool) are dropped entirely, and
  `mechanical-model-unresolved` is removed from the known-good stop-reason
  allowlist. The local dogfood `.claude/skills/rdm-autopilot` copy is
  unaffected and still invokes the real `estimate` Workflow — only the
  shipped/downstream lane changes.
  `scripts/verify-agent-config-distribution.sh`'s Workflow-invocation check
  is also generalized from an autopilot-only literal-string check to
  `check_workflow_invocations_resolve`, which asserts, across every emitted
  skill, that any "Invoke(ing) the `<name>` Workflow" instruction names a
  Workflow that actually resolves to a file in the same emitted
  `.claude/workflows/` tree.

### Fixed

- The MCP variant of the shipped `rdm-autopilot` skill template
  (`rdm agent-config claude --skills --mcp`) called its own new
  advance/park read-back steps with the wrong argument shape: `stem: S` and
  no `project` field at all, on both the `rdm_phase_update` and
  `rdm_phase_show` calls. The real MCP server's `PhaseUpdateParams`/
  `PhaseParams` require `phase` (not `stem`) plus a mandatory `project`,
  matching every other MCP skill template in this repo — so every phase
  the loop tried to advance to `reviewed` or park as `blocked` would have
  failed at exactly the read-back confirmation mechanism this same phase
  introduced. Both call sites now pass
  `project: {proj_param}, roadmap: "<slug>", phase: S`, and both the
  generated-skill unit tests and `scripts/verify-agent-config-distribution.sh`
  gained a regression assertion pinning the correct argument shape at those
  two call sites and rejecting a reintroduced `stem:`-keyed call.
- The shipped `rdm-autopilot` skill templates (`rdm agent-config claude
  --skills`/`--mcp`) no longer instruct invoking a Workflow literally named
  `autopilot`. The same change that dropped `.claude/workflows/autopilot.js`
  from `generate_workflows()`'s emitted output left the templates'
  "What to do" steps still telling a downstream agent to invoke it — a
  contradiction that would have broken the very first dispatch of the
  emitted skill in any repo that ran `rdm agent-config claude --skills`.
  The steps now describe driving the loop directly and composing the real
  `dispatch-phase` Workflow it still relies on, and both
  `rdm-core`'s generated-skill tests and
  `scripts/verify-agent-config-distribution.sh` gained a planted-mutation-backed
  assertion that the emitted template can never again claim to invoke a
  Workflow named `autopilot`.
- The autonomous code review (`rdm-dispatch-phase`, `rdm-autopilot`, the
  standalone `review-refute-fix` workflow) now mechanically forces `rework`
  whenever the acceptance-criteria table it reports carries a FAIL or PARTIAL
  criterion, regardless of finding severity or refutation — previously this
  guarantee held only to the extent the `ac` dimension happened to emit its
  gaps as `blocking` findings that survived refutation and the confidence
  floor. It also now incorporates any surviving non-blocking (concern or
  suggestion) finding on an otherwise-clean review by size instead of
  silently dropping it: small fixes are applied inline before landing, and
  large ones are filed as a follow-up task tagged `code-review`. When an
  AC-only gap (no blocking finding) triggers a rework pass, the implementer is
  now actually told which acceptance criteria failed and why — previously the
  rework prompt carried only the (empty) findings array, so the retry had no
  signal to act on and would very likely reproduce the same gap. The rework
  summary in every surface that can classify an AC-only-gap rework
  (`dispatch-phase`'s phase and task outcomes, and `review-refute-fix`'s own
  standalone summary) now names the real cause ("unmet acceptance criteria in
  AC table") instead of the misleading "no surviving findings".

## [0.18.0] - 2026-07-25
### Fixed

- The dogfood autonomous-lane workflows' mechanical (fetch/exec) agents now
  resolve and pin to the small/mechanical tier instead of inheriting the
  reviewer or session model. `dispatch-phase`'s `stamp:in-progress` and
  `diff:signals` steps now resolve `models.mechanical` (added to Stage-0's
  batch model resolution, alongside plan/implement/review-find/review-verify)
  instead of borrowing `models.review_find`. The four headless workflows
  (`estimate`, `plan-review`, `backlog`, `document`) each gained a small
  `rdm model resolve mechanical` bootstrap step whose resolved id is threaded
  into every mechanical Bash agent in that file (list/writeback/tier-read;
  fetch/gate-tag-clear; report fetch; roadmap/phase fetch and per-phase
  gather/write) — closing the gap the earlier autopilot-only mechanical-tier
  fix left open (e.g. a live `backlog` run's `fetch:report` agent previously
  ran on `claude-opus-4-8`). Judgment agents (plan/implement/review, the
  estimate rater, the plan-review orchestrator's `act` step, backlog's
  analyzers, document's synthesis step) are unaffected.

### Changed

- The dogfood `autopilot` Workflow's estimate pre-pass now records a
  `## Estimate <difficulty> — <justification>` audit note on each phase it
  rates (previously it persisted `--difficulty` only and dropped the rater's
  justification). The behavior — and the underlying difficulty-writeback logic
  — is now single-sourced with the new standalone `estimate` workflow via the
  shared `estimate-core` block, so both surfaces write the note identically.
- **Breaking:** the shipped `rdm-autopilot`, `rdm-dispatch-phase`, and the
  `--auto` branch of `rdm-do` skill templates (`rdm agent-config claude
  --skills`, both `cli` and `mcp` variants) are rewritten as thin shims that
  invoke the autonomous-lane Workflow scripts (`.claude/workflows/autopilot.js`
  / `dispatch-phase.js`, now emitted alongside the skills — see the prior
  `[Added]` entry) via the `Workflow` tool, instead of re-narrating an 8-step
  plan/plan-review/implement/code-review loop in prose with no runnable
  backing. Every real behavioral guardrail carries forward unchanged: the
  `--permission-mode auto` safety rules (`Edit`-not-`Write`; never `git stash
  -u`/`reset --hard`/`clean -fdx`) now live in the shipped
  `skill-dispatch-phase-{cli,mcp}.md` templates, not just the local dogfood
  copy; `rdm-autopilot`'s four run-mode flags (`--max-phases`, `--plan-only`,
  `--max-plan-revise`, `--max-code-rework`) are documented; `rdm-do`'s new
  `## Auto phase dispatch` / `## Auto task dispatch` sections route `--auto`
  runs into `dispatch-phase`, reading `outcome.status` / `outcome.reason` /
  `outcome.writesCompletion` as data instead of restating the gate policy.
  Existing consumers of the old prose templates should regenerate
  (`rdm agent-config claude --skills --out <dir>`) rather than hand-patch.
- The `rdm-plan-review` skill's documentation has been reorganized and clarified.
  The review pipeline steps (Setup → Find → Consolidate → Categorize & act → Gate)
  now have clear, permanent hand-authored sections that document plan-review's
  domain-specific logic (argument parsing, verdict-determination, and
  `needs-plan-review` tag-clearing gating). The Gate section is now more explicit
  about its fundamentally different role compared to code-review's status-transition
  gate: plan-review gates by clearing or leaving the `needs-plan-review` tag, never
  writing rdm status or land-time completion directives. The review dimensions and
  verdict rules are no longer hand-authored here at all: they are generated into the
  skill from the canonical review source (`.claude/workflows/lib/review.mjs`, via
  `scripts/gen-skill-review.sh --mode plan`), so the skill and `rdm-review` now share
  one specification. Setup, Find, Consolidate, Categorize & act, and Gate remain
  hand-authored **by design** — they carry plan-review's domain-specific logic, not
  review-spec content, and are not pending future automation. **No behavior change**;
  all verdict categories, gating logic, and per-phase handling remain unchanged.

### Removed

- The now-superseded "Mandatory dispatch — no inline work" / inline-collapse
  self-check checklists are gone from the shipped `rdm-autopilot` and
  `rdm-dispatch-phase` templates (both `cli` and `mcp` variants) — they existed
  only to stop an LLM from narrating orchestration it should have dispatched to
  a subagent, and a thin shim that hands off to the `Workflow` tool cannot
  inline-collapse that way. No template documents a `--land` flag on
  `rdm-autopilot` any longer (it never had one); landing to `main` stays
  `rdm-land`'s exclusive, explicit-invocation job.
- **Breaking:** the plan-review Stop hook (`rdm agent-config claude --hooks`'s
  `.claude/hooks/rdm-plan-review-on-create.sh`) and its Pi `agent_end`
  extension (`rdm agent-config pi --hooks`'s
  `.pi/extensions/rdm-plan-review.ts`) are no longer generated, and the
  `--hooks` flag itself is removed from `rdm agent-config claude`/`rdm
  agent-config pi` entirely. It is superseded by plan review now running
  in-flow on the ephemeral implementation-plan lane (`dispatch-phase`,
  `rdm-do`'s `--implementation-plan` review). **That in-flow review is a
  distinct mechanism from, and does not clear, the persisted
  `needs-plan-review` tag** stamped onto roadmaps/phases/tasks by `roadmap
  create` / `phase create` / `task create` when `plan_review` is enabled — it
  reviews an ephemeral plan draft, not the persisted item. With the Stop hook
  retired, clearing `needs-plan-review` on items created via `rdm-roadmap`,
  ad hoc create commands, or `rdm-do` side-task filing has **no automated
  reprompt left and is manual-only** until a filed follow-up task
  (`wire-active-plan-review-tag-gate`) lands: run the `rdm-plan-review` skill
  against the item (`--roadmap`/`--task`/`<roadmap> <phase>`), or
  periodically sweep with `rdm search "" --tag needs-plan-review`. Projects
  that installed the retired hook/extension via `--hooks` should remove
  `.claude/hooks/rdm-plan-review-on-create.sh` and its `hooks.Stop` entry in
  `.claude/settings.json` (or `.pi/extensions/rdm-plan-review.ts`) manually —
  `agent-config` no longer manages them.

### Added

- New dogfood `estimate` Workflow (`.claude/workflows/estimate.js`): rating an
  rdm roadmap's phase difficulties now runs headlessly. Given a roadmap slug
  (optionally narrowed to a single phase number) it lists the phases, filters
  to the ones whose difficulty is unset, rates each in a parallel fan-out, and
  writes the rating back — persisting the difficulty AND appending a
  `## Estimate <difficulty> — <justification>` audit note to the phase body —
  then reads the model tier back from rdm-core for its summary. When narrowed to
  a single phase, the summary reports the other still-unestimated phases as
  `deferred` (not targeted this run) rather than mislabeling them as already
  estimated; only phases that genuinely carry a difficulty are reported as
  skipped. It never passes
  `--model` and never reimplements the difficulty→tier mapping: rdm-core
  (`Difficulty::model_tier`) stays the single home for that policy. Its pure
  core lives once in `.claude/workflows/lib/estimate.mjs` (the `estimate-core`
  marker region) and is stamped byte-identical into every consumer by the new
  `scripts/gen-workflow-estimate.sh` (with a `--check` drift gate); the local
  `rdm-estimate` skill is re-authored as a thin shim over this workflow. New
  dogfood harness `scripts/verify-workflow-estimate.sh` gives hermetic
  estimate-core DRIFT + BEHAVIOR coverage. The shipped `rdm-estimate` skill
  template is unchanged; this workflow is dogfood-only for now.
- New dogfood `backlog` Workflow (`.claude/workflows/backlog.js`): the
  read-only, propose-only `rdm-backlog` grooming pass now runs headlessly. It
  runs `rdm backlog report` once, fans one READ-ONLY analyzer agent out per
  populated signal category (`stale_tasks`, `duplicate_clusters`,
  `tag_clusters`, `archivable_roadmaps`) in parallel, and consolidates the
  results into one ordered, reviewable batch of `{command, rationale}`
  proposals grouped by category plus a merged `## Open questions` section —
  or short-circuits to "Nothing to groom" when the report carries no signals.
  The non-mutation guarantee is structural: the only Bash-executing agent in
  the whole run is the read-only report fetch, and every analyzer is
  explicitly told to propose text only, never to execute a mutating command
  — mirroring `review-refute-fix`'s "READ-ONLY reviewer" framing. The local
  `rdm-backlog` skill is re-authored as a thin shim over this workflow. New
  dogfood harness `scripts/verify-workflow-backlog.sh` asserts the batch
  shape over a seeded report, the empty-report short-circuit, and — against a
  real seeded plan repo — that a run leaves the plan repo's git state
  byte-identical before and after. The shipped `rdm-backlog` skill templates
  are unchanged; this workflow is dogfood-only for now.
- New dogfood `plan-review` Workflow (`.claude/workflows/plan-review.js`): a
  standalone plan-mode review over all four target types — `--task <slug>`,
  `--roadmap <slug>`, a positional `<slug> [phase]`, and
  `--implementation-plan`. It reuses the one canonical review core
  (`buildReviewPipeline('plan')` and `GATE_POLICY.plan`) with no new review
  logic. For `--roadmap` it reviews the roadmap body plus every phase and gates
  each **independently** (a `parallel()` per-phase fan-out): a phase that
  reworks keeps its `needs-plan-review` tag while sibling phases that reach
  `reviewed` have theirs cleared. Clearing is a sibling-preserving
  read-filter-write (a reserved tag like `depends-unlanded` survives), the gate
  never persists an rdm status, and `--implementation-plan` — which has no
  persisted rdm item — is report-only (no body edit, no filed task, no gate).
  An unread or empty plan fails closed (the tag is left in place). The local
  `rdm-plan-review` skill is re-authored as a thin shim over this workflow. The
  shipped `rdm-plan-review` skill templates are unchanged.
- The standalone `review-refute-fix` Workflow tool now returns a full
  `reviewed` / `rework` / `escalated` verdict — with the mapped rdm status,
  `writesCompletion`, and a summary — instead of just a list of surviving
  findings, when invoked as `{ mode: 'code', roadmap, phase }` or
  `{ mode: 'code', task }`: it derives real diff signals from the item's
  worktree (falling open to every review dimension when the diff is
  unavailable, exactly like `dispatch-phase`'s code gate) and runs the same
  canonical review pipeline. An optional `gate: true` persists the mapped
  status via a mechanical `rdm phase update` / `rdm task update` call, for
  headless or ad hoc callers — it never runs `rdm commit` and never writes the
  land-time completion trailer. Existing ad hoc invocations (`mode: 'plan'`, or
  `mode: 'code'` with no roadmap/phase or task) are unaffected and keep
  returning the original `{ mode, survivors }` shape. The interactive
  `rdm-review` skill now delegates its dimension-finding/refuting mechanics to
  this same workflow (invoked with `gate: false`) instead of re-deriving them
  by hand, while keeping its own human-in-the-loop report/act/gate steps
  — including persisting status and amending the completion trailer — exactly
  as before.
- New dogfood harness `scripts/verify-workflow-review-outcome.sh` covers the
  above: the full OUTCOME shape for a clean and a blocking seed, the
  diff-signals fail-open contract, the mutual-exclusion guard on `task` vs
  `roadmap`/`phase`, both legacy backward-compatible shapes, the optional
  headless gate, and that the `rdm-review` skill shim still references the
  workflow while retaining its interactive report/act/gate prose and the
  completion-trailer mechanism.
- New dogfood harness `scripts/verify-agent-config-distribution.sh` proves
  that `rdm agent-config claude --skills` (both the plain CLI and `--mcp`
  variants) emits a self-consistent, working autonomous lane into a
  downstream repo: the 3 workflow scripts land byte-identical to their
  `.claude/workflows/*.js` sources, all 11 skills land with valid
  frontmatter, and every literal `.claude/workflows/<name>.js` reference
  inside an emitted skill resolves to a real file in the same emitted tree —
  with planted-corruption self-tests proving neither gate is vacuous.
- `rdm agent-config claude --skills --out <dir>` now also emits the
  autonomous-lane Workflow-tool scripts (`autopilot.js`, `dispatch-phase.js`,
  `review-refute-fix.js`) under `<dir>/.claude/workflows/`, byte-identical to
  this repo's own dogfood copies in `.claude/workflows/`. Emission is
  Claude-only (Pi has no Workflow-tool runtime) and `--out`-only, not
  `--user` — the scripts still hardcode this repo's own
  `./target/debug/rdm` binary path and `--project rdm` invocation and are
  not yet parameterized for a downstream target repo (tracked as a
  follow-up).
- `rdm hook done-line` prints the land-time `Done:` commit trailer for a phase
  (`--roadmap <slug> --phase <stem>`) or a task (`--task <slug>`). It is now the
  single home of that format string, so the review gate and `rdm-land` amend its
  output onto the branch commit instead of hand-typing the format. It rejects
  both-or-neither of `--phase`/`--task`, an embedded `/`, and `task` used as a
  roadmap slug (a reserved prefix), each with an actionable error.
- The `rdm-review` skill gains a **security** review dimension, triggered when a
  change touches auth, input parsing/validation, path or file handling,
  subprocess/shell invocation, secrets and credentials, deserialization, network
  code, or `unsafe` blocks. It reviews injection, path traversal, secret leakage,
  missing authorization, and unsafe-invariant violations. The pre-existing
  dimensions (`ac`, `correctness`, `tests`, `architecture`, `api-docs`,
  `changelog`) are unchanged, so this is strictly added coverage.

- Tag filtering across the list surfaces. `rdm roadmap list --tag <tag>` and the
  top-level `rdm list --tag <tag>` are new; `rdm task list --tag <tag>` and
  `rdm search --tag <tag>` now accept the flag **repeatedly**, and repeats combine
  with AND (`--tag bug --tag ui` keeps only items carrying both). Matching is
  exact and case-sensitive, passing no `--tag` imposes no constraint, and a filter
  that matches nothing prints the usual empty-state line and exits 0. `--tag`
  composes with the existing `--status`/`--priority`/`--sort` filters, and is
  allowed alongside `roadmap list --archived` (unlike `--sort`/`--priority`).
- List output now surfaces tags when any listed item has them. `rdm task list`
  (text, table, and Markdown) and the Markdown `rdm roadmap list` gain a trailing
  `Tags` column; the default (human) `rdm roadmap list` / `rdm list` view is
  paragraph-shaped and instead appends a ` [tags: bug, ui]` suffix after the
  priority suffix on each tagged line. Untagged items render an empty cell / no
  suffix, and the column is omitted entirely when nothing is tagged. Because
  these renderers are shared, this also changes the MCP `rdm_task_list` and
  `rdm_roadmap_list` tool results. JSON output is unchanged.

- New `rdm tag list --project <name>` command: a read-only inventory of every tag
  in use across a project's roadmaps and tasks, printed most-used-first as
  `cli (3)` / `web-ui (5)` lines (or `No tags in use.` when there are none).
  Counts cover roadmaps and tasks of every status — including `done` and
  `wont-fix` — but not phases or archived roadmaps, and tags are compared
  verbatim, so `CLI` and `cli` are listed separately. Pass `--format json` for a
  machine-readable array of `{"tag", "count", "roadmaps", "tasks"}` objects for
  agent consumption. This answers "which tags exist?"; `rdm search "" --tag <name>`
  answers "what carries this tag?".

- New read-only MCP tool `rdm_backlog_report`, a thin wrapper over the same
  `rdm_core::ops::backlog::report` the CLI's `rdm backlog report` already
  calls, returning the identical four-array JSON shape (`stale_tasks`,
  `duplicate_clusters`, `tag_clusters`, `archivable_roadmaps`). This backs a
  new `rdm-backlog` MCP skill (`rdm agent-config claude --mcp --skills`),
  closing the last cli/mcp skill-set gap: both platforms now emit the same 11
  skills, asserted by a new relative-path parity test
  (`generate_skills_cli_mcp_name_parity`) so a future one-sided addition
  fails CI instead of silently shipping asymmetric skill sets.
- New dogfood `document` Workflow (`.claude/workflows/document.js`): the
  `rdm-document` doc-generation pass now runs headlessly. It validates every
  phase of a roadmap is `done` (aborting with the incomplete-phase list
  otherwise), fans a per-phase git-gather step out in `parallel()` (`git log`
  / `git diff --stat` over each phase's recorded commit SHA, falling back to
  phase-body-only when a phase has no SHA), runs one synthesis agent to draft
  the doc, and a mechanical Bash agent to write it to `--out` (default
  `docs/<slug>.md` — the runtime has no filesystem of its own), returning
  `{ roadmap, aborted, incompletePhases, path, draft }`. It performs no
  status mutation and no approval step of its own: the workflow produces an
  artifact, not a completion signal, and the terminal human review is the
  local `rdm-document` skill's job alone. That skill is re-authored as a thin
  shim over this workflow. New dogfood harness
  `scripts/verify-workflow-document.sh` asserts the draft is produced at the
  expected default/`--out` path, the all-done validation aborts on an
  incomplete roadmap, and a phase with no recorded commit falls back to
  body-only — including against a real seeded plan repo and source repo, not
  just fabricated inputs. The shipped `rdm-document` skill templates are
  unchanged; this workflow is dogfood-only for now.

### Fixed

- The workflow lane's `dispatch-phase` now stamps a phase (or task) `in-progress`
  (best-effort) when it begins working it, right after fetching the item and
  resolving models and before planning starts. Previously a direct `Workflow`
  invocation of `dispatch-phase` — and therefore every autopilot-driven phase —
  jumped straight from `not-started` to `reviewed`/`blocked` with no observable
  in-progress signal in `rdm phase list`, a status search, or the TUI while the
  run was executing, so a crashed or cancelled run looked untouched instead of
  attempted. A `--plan-only` pass is unaffected — it never implements, so it
  never stamps.

- An autonomously produced branch now reaches `rdm-land` ready to land, with no
  manual rebase. `rdm-dispatch-phase` / `rdm-autopilot` deliberately never write
  the `Done:` commit trailer, and their outcome now says so explicitly — it
  carries `writesCompletion: true` on a clean review — so `rdm-land` synthesizes
  the trailer from `rdm hook done-line` and amends it onto the branch tip
  *before* the rebase and fast-forward. Previously the missing trailer had to be
  noticed and repaired by hand after the fact.

- The workflow lane's dispatch outcome classifier no longer marks a phase
  `reviewed` on a failing code review when no rework round ran. It previously
  hard-coded a two-slot "first pass + exactly one rework" shape, so with a
  code-rework budget of 0 the empty post-rework slot made a blocking first-pass
  review classify clean. The classifier now judges the last review round that
  actually ran, however many there were (including zero).

- Removed dangling instructions from the shipped `rdm-review` / `rdm-plan-review`
  skill templates (both `cli` and `mcp` variants) telling the reader to "edit
  `.claude/workflows/lib/review.mjs` and run `scripts/gen-skill-review.sh`" to
  regenerate the review specification section. Those paths are dogfood-only
  tooling in rdm's own source repo and never ship to a consumer repo, so
  following the instruction there was impossible. The generated region is now
  described as fixed content, rendered from rdm's own canonical review source
  at release time, with upstream changes arriving via the next
  `rdm agent-config` regeneration rather than a local edit.

### Changed

- The shipped `rdm-plan-review` skill now carries the same **generated review
  specification** as `rdm-review`: the same severity scale, confidence floor,
  finding format, and `reviewed` / `rework` / `escalated` outcome vocabulary,
  replacing the old PASS / PASS WITH CONCERNS / REWORK verdicts. Its three plan
  dimensions are unchanged (coherence and architectural fit always run;
  unit-of-work runs only for a phase), and the `needs-plan-review` gate is
  unchanged in effect — what used to PASS or PASS WITH CONCERNS now reports
  `reviewed` and clears the tag, and what used to REWORK now reports `rework` or
  `escalated` and leaves it. Per-phase gating under `--roadmap <slug>` and the
  report-only `--implementation-plan` mode (no gate, no mutations) are preserved.
- Plan review now includes a **refute pass**: every finding is graded by a fresh,
  separate read-only agent before it is reported, and refuted or low-confidence
  (<70) findings are dropped. This is deliberate parity with the code review, and
  it means some weakly-evidenced findings that previously held the
  `needs-plan-review` gate closed will no longer do so.
- `rdm-do`'s finalize step now runs an automated code review in **both** modes.
  Previously, interactive `rdm-do` parked the item in `needs-review` for a
  separate review pass to pick up, and `rdm-do --auto --task` finalized with no
  automated review at all. Finalize now invokes the `rdm-review` skill directly
  once the work is committed; the human confirmation gate still decides *whether
  to finalize*, but a review always happens. The review owns the status gate
  (`reviewed` / `in-progress` / `blocked`) and the completion trailer, so
  `needs-review` is now only a transient marker rather than a parking state.
- The autonomous lane's code review now scales to the change. It selects its
  review dimensions from the real branch diff (`git diff main...HEAD`), so
  `tests`, `architecture`, `api-docs`, `changelog`, and `security` run when the
  change actually touches their surface — re-derived on every rework round, so a
  fix that newly touches a public API is reviewed for public API docs. If the
  diff cannot be read, every dimension runs, so coverage only ever fails open.
- The `rdm-review` skill now reports a single outcome — **reviewed**, **rework**,
  or **escalated** — retiring the old PASS / PASS WITH CONCERNS / BLOCKED / FAIL
  quartet. PASS and PASS WITH CONCERNS collapse to `reviewed`, FAIL becomes
  `rework`, and BLOCKED becomes `escalated`; this is the same vocabulary
  `rdm-dispatch-phase` and `rdm-autopilot` already returned, so every surface now
  speaks one language. An **escalated task** is set to `blocked` (with a `[code]`
  reason prefix) rather than being downgraded to `in-progress` — tasks have
  supported `blocked` for some time, and the skill's claim to the contrary was
  stale. Phase behavior is unchanged.
- The `rdm-review` skill's wording is trimmed: the review dimensions, severity
  scale, and verdict rules now live *only* inside the generated "Review
  specification" section (stamped by `scripts/gen-skill-review.sh` from the
  same canonical source `rdm-dispatch-phase`/`rdm-autopilot` use). The
  surrounding Setup/Report/Act/Gate steps no longer restate those definitions
  — they reference the generated section by name instead. No behavior change;
  this only removes duplicated prose that could drift out of sync with the
  canonical definitions.
- `rdm-land` no longer aborts when a reviewed branch is missing its `Done:`
  trailer. Autonomous runs deliberately never write one, so landing now
  synthesizes it via `rdm hook done-line` from the item's identifiers and amends
  it onto the branch tip before rebasing — aborting only when the identifiers are
  unknown or the tip is not the un-landed reviewed commit.

- The autonomous workflow lane's two in-run retry budgets are raised from 1 to 2
  and are now overridable per run. `dispatch-phase` accepts `maxPlanRevise` and
  `maxCodeRework` (default 2 each, counted independently: budget N means N
  reworks after the original attempt, i.e. N + 1 attempts), and autopilot
  forwards them from `--max-plan-revise N` / `--max-code-rework N`. A budget of
  `0` is legal and means "terminate on the first blocking review"; a negative or
  non-integer budget is rejected up front with an actionable message. Autopilot's
  own roadmap-level rework re-dispatch budget (1) and global step budget (50) are
  unchanged. `docs/escalation-protocol.md` § Budgets now documents all four
  budgets, the attempt sequences, and notes that the shipped
  `rdm-core/src/templates/` prose skills remain at 1 pending a follow-up — so
  `agent-config` consumers are unaffected by this change.
- `rdm agent-config` output (both the CLI and `--mcp` variants) now teaches
  tagging as a create-time habit: it tells agents to always pass `--tags` /
  `tags: [...]` when creating a roadmap, phase, or task, warns that `--tags` on
  update *replaces* the existing list, suggests a starting vocabulary of `bug`,
  `enhancement`, `cli`, `core`, `server`, `web-ui`, and `docs` (framed as
  suggestions, not a closed set), and points at `rdm tag list` — instead of the
  previous `rdm search "" --tag <candidate>` — for discovering the tags a project
  already uses before inventing a new one.
- The `rdm-plan-review` skill's Coherence reviewer (CLI and MCP variants) now
  treats a cross-item dependency on unlanded work as non-blocking when the
  target item is annotated: a plan step citing a file or behavior introduced by
  another in-flight (not-yet-landed) roadmap or task is only `blocking` if the
  item does not carry the new reserved `depends-unlanded` tag and does not
  state the dependency explicitly. This closes a false-positive REWORK mode
  where side-tasks filed from inside a roadmap's shared worktree described
  branch-local files as if they already existed on `main`.

### Added

- Tasks can now be `blocked`. `TaskStatus` gains a `blocked` variant, mirroring
  phases: `rdm task update <slug> --status blocked --reason "…"` parks a task with
  a recorded reason (stored in the task's `close_reason`, preserved across later
  status changes and cleared with `--clear-reason`). `blocked` is non-terminal, so
  a blocked task still shows in the default `rdm task list` "active work" view, and
  it is accepted anywhere a task status is parsed (CLI `--status` filter, MCP
  `rdm_task_update`/`rdm_task_list`, and the web status dropdown). This lets the
  autonomous workflow lane park a task the same way it parks a phase.

### Added

- New `autopilot` workflow (`.claude/workflows/autopilot.js`): the active driver
  of the autonomous lane. Given one roadmap slug it runs an estimate pre-pass over
  the roadmap's unestimated phases (one `parallel()` fan-out, persisting each
  difficulty so the model tier auto-derives), then loops `rdm next` → the
  `dispatch-phase` workflow (via the one allowed level of `workflow()` nesting) →
  interpret the OUTCOME → PERSIST status: a `reviewed` phase advances
  (`rdm phase update --status reviewed`), a `rework` phase re-dispatches against a
  per-phase budget and then parks `blocked [code]`, and an `escalated` phase parks
  `blocked [plan]`. It bounds the run with a global step budget and `--max-phases`,
  supports `--plan-only` (each dispatch stops after its plan gate, guarded against
  re-vetting), always ends with a batched summary (phases completed, escalations
  tagged plan/code pointing at `rdm review blocked`, stop reason), and never
  touches `main` — landing stays the separate `rdm-land` skill. `dispatch-phase`
  now accepts a `planOnly` arg and returns early once the plan gate passes. The
  dogfood `rdm-autopilot` skill is rewritten as a thin shim that parses the
  invocation and hands off to the workflow. Gated by
  `scripts/verify-workflow-autopilot.sh` (pure helpers + a state-backed driven
  loop: drive-to-reviewed, rework→park, escalated, budget stops, the estimate
  pre-pass, `--plan-only`, and mid-tier defaulting). Dogfood-only — not emitted by
  `agent-config`.
- New `dispatch-phase` workflow (`.claude/workflows/dispatch-phase.js`): the
  keystone per-phase unit of autonomous execution, a deterministic 4-stage
  pipeline (plan → plan-review → implement → code-review). It seeds a fresh
  implementer with only the phase body plus an independently reviewed plan
  document, runs both review gates through Phase 1's stamped
  `buildReviewPipeline`, bounds itself to at most one plan-revise and one
  code-rework pass, and returns an `{ roadmap, phase, outcome, summary, findings }`
  OUTCOME with `outcome` one of `reviewed` | `rework` | `escalated`. It never
  emits a `Done:` line (landing is a separate, later step). Gated by
  `scripts/verify-workflow-dispatch.sh` (all three outcome branches) plus
  `scripts/verify-workflow-review.sh`. Dogfood-only — not emitted by
  `agent-config`.
- New `rdm-backlog` Claude Code skill: a propose-only backlog grooming pass. It
  reads `rdm backlog report` and emits a single batched, human-reviewable plan
  that pairs each proposed consolidate / merge / retire / archive action with the
  literal `rdm` command that would carry it out and a one-line rationale, framing
  proposed roadmaps to be `/rdm-autopilot`-ready. It performs zero mutations
  (no create/update/merge/archive, no staging, no `rdm commit`), handles the
  empty-backlog case, and surfaces ambiguous or destructive-if-wrong decisions as
  open questions rather than acting on them. `rdm agent-config claude --skills`
  and `rdm agent-config pi --skills` now emit 10 skill files instead of 9; the
  `--mcp` variant is unchanged at 9 pending a dedicated `rdm_backlog_report` MCP
  tool.
- rdm now automatically configures a git merge driver so conflicts on auto-generated `INDEX.md`/`projects/*/INDEX.md` files are resolved by regeneration instead of requiring manual `rdm resolve`. `rdm init` writes the tracked `.gitattributes` entries (which travel with clones); every command that opens the plan repo adds the untracked, repo-local `.git/config` driver definition if missing (best-effort — a read-only `.git/config` warns instead of failing the command). (The previously-documented but never-implemented `rdm install-merge-driver` command reference has been removed from the docs — nothing to migrate.)

- rdm now emits a warning to stderr when the global config (~/.config/rdm/config.toml) or repo config (rdm.toml) contains invalid TOML, instead of silently falling back to defaults.
- New `[models]` config table schema (`small`/`medium`/`large` model ids,
  `review_floor`, and per-step `[models.steps]` overrides) in `rdm.toml` and
  the global config, with repo-over-global merge semantics. Not yet consumed
  by any command — this lays the storage foundation for upcoming model-tier
  resolution.
- `rdm-core::model_policy` module with a `ModelPolicy` resolver that turns a
  dispatch step (`plan`, `implement`, `review-find`, `review-verify`,
  `mechanical`) plus an optional caller tier hint into a concrete model id,
  applying the `[models]` config from `rdm.toml`/global config with built-in
  defaults (`small`→`haiku`, `medium`→`sonnet`, `large`→`opus`, review floor
  `medium`). Review-reasoning steps are clamped up to the review floor;
  `mechanical` is exempt. Not yet wired into any CLI command or skill — this
  is the internal sizing engine upcoming phases will consume.
- `rdm model resolve <step> [--tier <tier>]` and `rdm model show` (`--format
  json` supported) — CLI porcelain over the `[models]` sizing policy,
  resolving a dispatch step (`plan`, `implement`, `review-find`,
  `review-verify`, `mechanical`) to a concrete model id, or inspecting the
  full resolved policy (tier bindings, review floor, per-step models).
- `rdm config set server.quick_filters "Label1:tag1,Label2:tag2"` (plus matching `rdm config get`/`rdm config list` support) configures HTML quick-filter chips without hand-editing `rdm.toml`. An empty value clears all chips; the key is repo-only (`--global` is rejected).
- `rdm backlog report [--older-than <days>] [--tag <tag>] [--project <project>]` — a read-only backlog grooming sensor. Prints stale tasks (`open`/`in-progress` tasks past a staleness threshold, default 60 days), likely-duplicate task clusters (via the existing fuzzy `search` matcher — no new similarity engine), thematic tag clusters among active tasks, and archivable roadmaps (every phase terminal, not yet archived). Supports `--format json` for structured output alongside the default human/markdown rendering. Performs zero writes and requires no plan-repo schema change.
- `rdm bootstrap` gained a `--print-root` flag (prints only the resolved
  plan-repo path to stdout, with all narration moved to stderr) and
  `--format json` support (`{"path", "status", "commits_merged"}`), for
  reliable scripting in session-start hooks instead of parsing the human
  success banner. `--print-root` takes precedence over `--format` when
  both are given. `templates/claude-code-web/.claude/hooks/SessionStart.sh`
  now uses `--print-root` instead of a `sed`-based parse of bootstrap's
  stdout.
- `rdm promote <task-slug> --into <existing-roadmap-slug>` consolidates a task
  into an already-existing roadmap as a new trailing phase (auto-numbered),
  carrying over the task's body (prefixed with a provenance line naming the
  source task) and tags, instead of creating a brand-new 1:1 roadmap. `--body`
  and `--no-edit` are supported to override the phase body content. The
  source task is not deleted — it is closed as `done` with a pointer note
  naming the new roadmap and phase, so it drops out of the active `task
  list` while remaining inspectable. Consolidating into a nonexistent
  roadmap, or a task that is already `done`/`wont-fix`, fails with an
  actionable error. The existing `--roadmap-slug` (create-new-roadmap) form
  is unchanged.
- `rdm task merge <survivor> --from <a> --from <b>` (comma-separated or
  repeated) folds one or more duplicate tasks into a survivor: it unions the
  sources' tags into the survivor, appends each source body under a `## Merged
  from task <slug>` heading (in `--from` order), and closes every source
  `wont-fix` with a `superseded by task/<survivor>` pointer note. Merged
  sources drop out of the active `task list` but stay inspectable with their
  provenance intact. The merge is idempotent — re-running it is a no-op — and
  self-merges, unknown survivors, unknown sources, and empty `--from` all fail
  with actionable errors before any change is written.
- `rdm task update --reason <text>` / `--clear-reason` records (or clears) a
  persisted close reason on a task — e.g. why it was retired as `wont-fix`.
  The reason survives read-back, is shown by `task show` and included in
  `task show --format json` (`close_reason`), and is preserved across later
  status changes until explicitly cleared.
- New `plan_review` config key (`rdm.toml` and global config, `RDM_PLAN_REVIEW`
  env override, default `false`). When enabled, `rdm roadmap create`, `rdm
  phase create`, and `rdm task create` stamp a reserved `needs-plan-review`
  tag onto new items alongside any user-supplied `--tags`, so pending items
  can be listed with `rdm search "" --tag needs-plan-review --type
  phase|task --format json`. Not yet consumed by any skill or hook — this
  lays the sentinel-tag foundation an upcoming plan-review skill will act on.
- New `rdm-plan-review` agent skill (CLI and MCP variants), generated by
  `rdm agent-config --skills` alongside the existing nine skills. Reviews the
  plan of a roadmap, phase, or task (or an in-progress `rdm-do` implementation
  plan via `--implementation-plan`) before implementation begins: it dispatches
  parallel read-only sub-agents for coherence, architectural fit (reading the
  configured principles file, falling back to `CLAUDE.md`/`AGENTS.md`), and,
  for phases, unit-of-work sizing, then consolidates their findings into a
  single **PASS** / **PASS WITH CONCERNS** / **REWORK** verdict. Small findings
  are applied inline to the plan document; large ones are filed as
  `rdm task create ... --tags plan-review` tasks. On PASS or PASS WITH
  CONCERNS it clears the `needs-plan-review` tag (Phase 1's sentinel); on
  REWORK it leaves the tag in place and reports what must change.
- `rdm agent-config claude --hooks` and `rdm agent-config pi --hooks` now also
  emit a plan-review Stop hook/extension (`.claude/hooks/rdm-plan-review-on-create.sh`
  registered in `.claude/settings.json`, or `.pi/extensions/rdm-plan-review.ts`
  for Pi) that reprompts the agent to run the `rdm-plan-review` skill while any
  roadmap, phase, or task carries the `needs-plan-review` sentinel tag (stamped
  when `plan_review` is enabled). It honors `stop_hook_active` the same way the
  existing `needs-review` hook does, and fails open on any query error.
  `merge_stop_hook_into_settings` now composes multiple Stop hooks
  non-destructively and idempotently, so both hooks coexist in the same
  `settings.json`.
- Dogfood the `rdm-plan-review` skill and Stop hook:
  `.claude/skills/rdm-plan-review/SKILL.md` and
  `.claude/hooks/rdm-plan-review-on-create.sh` (registered in
  `.claude/settings.json` alongside the existing
  `rdm-review-on-finalize.sh` Stop hook). Enabled `plan_review = true` for
  rdm's own plan repo, so newly created roadmaps/phases/tasks are tagged
  `needs-plan-review` and gated by `rdm-plan-review` before implementation
  begins, composing with the existing post-implementation
  `rdm-review`/`needs-review` gate.

### Changed

- The generated `rdm-review` skill (CLI and MCP variants) and the dogfood
  `.claude/skills/rdm-review/SKILL.md` now size the review fleet via the
  `[models]` policy instead of inheriting the session's model: step 1 derives
  a `small`/`medium`/`large` tier hint from the diff's blast radius, step 2
  resolves `rdm model resolve review-find --tier <hint>` (or `mechanical` for
  scripted checks) for each dispatched finder agent, and step 3 resolves `rdm
  model resolve review-verify` for the refute pass. Every dispatched agent is
  now given an explicit `model`, closing the session-model-inheritance leak.
- `rdm-autopilot` and `rdm-dispatch-phase` skills now state an explicit **synchronous dispatch contract**: every subagent (per-phase dispatch, planner, plan reviewer, implementer) is spawned synchronously and its returned result is the sole channel back — no background-and-poll, no resume-by-message, and no `SendMessage` to a parent (`"claude"` does not resolve); rework always spawns a fresh subagent.
- `rdm-autopilot` and `rdm-dispatch-phase` skills now make subagent dispatch a
  non-skippable **MUST**: a new "Mandatory dispatch — no inline work" section
  in each skill explicitly prohibits doing the planning/implementation/review
  inline, requires a pre-action declaration of which subagent/role is being
  dispatched, and adds a "Self-check before proceeding" checkpoint restated
  at each dispatch point (`rdm-autopilot` step 2; `rdm-dispatch-phase` steps
  4-6), plus a "mandatory, not best-effort" lead-in on `rdm-dispatch-phase`'s
  Context isolation section and a named negative example
  ("inline-collapse") of the failure this closes. The CLI and MCP-variant
  templates under `rdm-core/src/templates/` are brought into sync with both
  this change and phase 1's synchronous-dispatch wording, which they had
  been missing. See `docs/subagent-dispatch-enforcement.md` for the
  evaluated techniques and rationale.
- `rdm-dispatch-phase` and `rdm-autopilot` skills now include explicit guidance on safe operations under `--permission-mode auto`: use `Edit` (surgical) rather than `Write` (whole-file overwrite) when modifying existing tracked files, and never run destructive git operations (`git stash -u`, `git reset --hard`, `git clean -fdx`) that trigger the auto-mode permission classifier and stall unattended runs. The guidance points to the per-roadmap worktree isolation as the alternative (commit a WIP commit instead of stashing; clean up the worktree after the phase is done).
- `rdm worktree add` without `--base` now defaults the new branch to the invoking checkout's current branch instead of always basing off `main`'s `HEAD`, so a worktree created while on a feature branch builds on that branch's work. Detached HEAD (or the invoking branch matching the item's own target branch) still falls back to the prior `HEAD` default; an explicit `--base <ref>` always takes precedence.
- The `rdm-do` skill now reviews its own drafted implementation plan before execution: after the plan is drafted and before the approval gate, it runs the `rdm-plan-review` skill's new `--implementation-plan` mode (coherence + architectural fit) and surfaces the findings. In `--auto` mode it never waits on the verdict — it folds surviving blocking findings back into the plan text, or files them as a side-work task when they can't be resolved by editing the plan alone. `rdm-plan-review --implementation-plan` is now fully specified: it reviews a plan document handed to it directly (no persisted rdm item), and skips both the tag-gate step and the fix-application half of categorize-and-act, since there is nothing to write to or file against in that mode. The `rdm-roadmap` skill now notes that, when `plan_review` is enabled, newly created roadmaps/phases carry a `needs-plan-review` tag that should be left in place until `rdm-plan-review` clears it.

### Fixed

- The `autopilot` Workflow's estimate pre-pass now persists per-phase difficulty
  and model tiers instead of silently no-opping. Its `[estimate:list]` agent was
  handed a StructuredOutput schema with a top-level `type: 'array'`, which the
  Anthropic tool API rejects (`input_schema.type` must be `'object'`); every call
  400'd, so no tiers were recorded and all phases dispatched at the default
  `medium` tier. The phase-list schema now wraps the array under a `phases` key
  and the pre-pass unwraps it, so a live autopilot run over an unestimated
  roadmap records real Difficulty/Model tiers.
- `rdm roadmap update`, `rdm phase update`, and `rdm task update` no longer read
  stdin or open the interactive editor when neither `--body` nor `--clear-body`
  is given. A tags-only or status-only update (e.g. `rdm task update <slug>
  --status done --no-edit`) previously blocked indefinitely reading stdin when
  stdin was an open pipe that never closed — the exact environment the installed
  `Done:` post-merge/post-commit hooks run in as git subprocesses. Now the body
  is consulted only when explicitly requested: `--body` sets it, `--clear-body`
  clears it, otherwise it is left untouched. **Behavior change:** the previously
  legal `update --tags x < body.md` form (piping a body into an update via stdin)
  is retired — pass `--body` to set body content on an update, which composes
  with `--tags`, `--status`, and the other update flags. `create` is unaffected
  and still accepts a piped-stdin body. The bundled skills and the templates
  emitted by `rdm agent-config --skills` (`rdm-revise`, `rdm-estimate`) that
  previously set a body by piping a heredoc into `update` now pass `--body`
  instead, and the generated agent instructions document the create-vs-update
  body-input difference.
- `rdm serve` now resolves `?at=<sha>` historical-revision requests against the plan repo's real git history by default (when the plan root is a git repository), instead of always returning 404 "the store has no history available." Non-git plan roots keep the previous behavior. Note: this applies to the git-featured build shipped via `rdm-cli` (the default); a standalone `rdm-server` build without the new `git` cargo feature keeps FsStore-only (always-404) behavior.
- `rdm bootstrap doctor` now correctly detects rdm on PATH on Windows by checking for `.exe`, `.bat`, and `.cmd` extensions in addition to the unextended name.
- When running a non-init command against an uninitialized plan repo (no
  `rdm.toml` at the resolved root), the error now clearly guides users to
  `rdm init`: "no plan repo found at {path} — run `rdm init` to create one",
  instead of the opaque "failed to open git repository" error. Commands that
  do not require a repo (`rdm init`, `rdm describe`, `rdm agent-config`, `rdm
  model`, and `rdm bootstrap` and `rdm hook` when git is enabled) proceed
  normally without requiring `rdm.toml`.

### Removed

- **Breaking:** the auto-review Stop hook (`rdm agent-config claude --hooks`'s
  `.claude/hooks/rdm-review-on-finalize.sh`) and its Pi `agent_end` extension
  (`rdm agent-config pi --hooks`'s `.pi/extensions/rdm-review.ts`) are no
  longer generated. Both were a *passive* safety net that re-prompted an agent
  only when an item was left in `needs-review` after implementation; they are
  superseded by *active* review now running on every finalize path
  (`rdm-do`, `dispatch-phase`, and `autopilot` all invoke the canonical review
  before an item can be left in `needs-review`), so the net has nothing left
  to catch. Projects that installed the retired hook/extension via `--hooks`
  should remove `.claude/hooks/rdm-review-on-finalize.sh` and its
  `hooks.Stop` entry in `.claude/settings.json` (or
  `.pi/extensions/rdm-review.ts`) manually — `agent-config` no longer manages
  them. The sibling plan-review Stop hook / Pi extension
  (`rdm-plan-review-on-create.sh` / `rdm-plan-review.ts`), which gates a
  different, still-live concern (the plan, before implementation begins), is
  unaffected and continues to be generated by `--hooks`.

## [0.16.0] - 2026-07-04

### Security

- Upgraded the MCP server's `rmcp` dependency to 2.0.0, which fixes an OAuth
  resource-spoofing vulnerability, a metadata SSRF, and a streamable-HTTP
  session leak, and aligns tool-response content encoding with the MCP
  2025-11-25 specification.

### Changed

- `rdm task list` with no `--status` now shows all active tasks (open,
  in-progress, needs-review, reviewed) instead of only open/in-progress,
  matching the REST server's default. Done and wont-fix remain hidden by default.
- The `post-merge`/`post-commit` hooks now apply all `Done:` directives from a
  single hook invocation as one plan-repo commit (with a single `INDEX.md`
  regeneration) instead of one commit per directive. The commit message
  enumerates every applied `Done: <target>` directive alongside its source
  commit SHA, so per-directive provenance is preserved without a per-directive
  commit. Note: if the shared index-regeneration/commit step fails, no
  directive in that batch is committed (per-directive log lines plus a
  `batch-commit-error` event still record what happened) — previously an
  unrelated failure could not undo an already-committed directive's commit.
- Clarified in `--help` (for `roadmap`/`phase`/`task` `create`/`update`) and in
  the agent-facing docs (`CLAUDE.md`, `rdm agent-config`'s CLI instructions
  template) that `--body` accepts any text verbatim — backticks, em-dashes,
  and other Unicode/punctuation included — and always takes precedence over
  stdin, which is never read once `--body` is set. No behavior changed;
  `--body` was already authoritative (fixed in a prior release) and does not
  hang on special-character content — this is documentation-only, backed by
  new regression tests.
- **BREAKING:** The opt-in staging mode is gone — every mutating `rdm`
  command (`roadmap`/`phase`/`task`/`review`/`promote`/etc. create, update,
  delete) now stages its change to disk and defers the git commit
  **unconditionally**; there is no longer a way to auto-commit per mutation.
  The `--stage` flag, the `RDM_STAGE` environment variable, and the `stage`
  field in `rdm.toml` (both repo-level and global config) are removed —
  passing `--stage` or setting `RDM_STAGE`/`stage` is now a plain unknown-flag
  / ignored-config-key situation rather than a behavior toggle. **Migration:**
  after a batch of mutating commands, run `rdm commit -m "..."` to land them
  as one git commit; `rdm status` shows what's pending and `rdm discard
  --force` reverts it. `rdm hook post-commit`/`post-merge` and `rdm bootstrap`
  are unaffected — they already committed unconditionally through an internal
  always-commit pathway and continue to do so. See the MCP bullet below for
  the equivalent change on the MCP server, which is now stage-only and
  exposes `rdm_status`/`rdm_commit`/`rdm_discard`.
- MCP mutation tools now stage changes to disk and require an explicit
  `rdm_commit` to land them — no more auto-commit per mutation. `*_update`
  responses no longer carry a `Commit:` trailer and
  `rdm_review_address_comment` no longer auto-defaults `applied_commit`;
  thread the SHA from `rdm_commit`'s response instead.
- The `rdm-autopilot` agent skill (CLI and MCP variants) now dispatches each
  phase — including the `rdm-estimate` step when a difficulty is unset — as a
  single isolated `Agent` subagent instead of invoking `rdm-estimate` and
  `rdm-dispatch-phase` inline via the `Skill` tool. Only the structured
  `{roadmap, phase, outcome, summary, findings}` outcome crosses back into the
  loop, so the loop's context stays flat across a multi-phase run instead of
  accumulating every phase's plan/plan-review/implementation/code-review detail.
  No change to how the loop interprets `reviewed`/`rework`/`escalated`, the
  per-phase rework-retry budget, or blocked-parking. See
  [`docs/autonomous-loop.md`](docs/autonomous-loop.md).
- The `rdm-dispatch-phase` agent skill (CLI and MCP variants) now splits the
  planning agent from the implementing agent. The planning subagent (step 4)
  returns a **self-contained plan document** — steps mapped to each acceptance
  criterion, a file/crate navigation map, and a per-AC test list — and the
  independent plan gate (step 5) approves that exact document. Implementation
  (step 6) is then handed to a **new** implementer subagent seeded only with the
  phase body and the approved plan document, rather than reusing the planner's
  accumulated exploration context. The plan document carries the navigation map
  forward so the implementer inherits it instead of re-discovering it. The skill's
  "Context isolation" section now names the planner→implementer boundary
  explicitly alongside the existing planner→reviewer one.
- The `rdm-dispatch-phase` plan gate (step 5, CLI and MCP variants) now scales
  its rigor to the phase's difficulty tier instead of applying one fixed level
  of scrutiny: trivial/easy gets a holistic single-reviewer judgment, moderate
  requires the reviewer to cite per-finding evidence (the specific AC text,
  plan step, or file/crate each checklist judgment rests on), and hard adds a
  refute pass — a third, fresh subagent whose only job is to refute the
  reviewer's verdict, adapted from (not a faithful mirror of) `rdm-review`'s
  verify step: it is deliberately one-directional and can only tighten the
  gate's verdict, never loosen it. The checklist itself is sharpened for every
  tier: acceptance-criteria→step and acceptance-criteria→test mappings, edge
  cases/error paths per AC, and declared cross-phase/cross-crate dependencies,
  alongside the existing scope and architecture checks. The `revise` round now
  explicitly re-checks the revised plan against the same checklist, and an
  un-converged revise round escalates (stage `plan`) rather than silently
  proceeding with a deficient plan.

### Fixed

- The CLI's "staged — run `rdm commit` to persist" and "N uncommitted
  change(s)" hints now print to stderr instead of stdout. Since staging is
  the only workflow (no more opt-in `--stage`), these hints previously fired
  on every mutating and read command, corrupting machine-readable stdout
  (`--format json` and any piped/captured output).
- Roadmap status now treats `needs-review` and `reviewed` phases as active work: a roadmap whose phases are only in review states (no `in-progress` phase) is reported as `in-progress` instead of `not-started`, in the CLI and the server UI's roadmap status badge.
- REST API 400/422 responses for invalid status values in request filters and
  updates now list the complete status set including `needs-review` and
  `reviewed`, derived from the core `ParseError` instead of hand-maintained
  literals. This affects `/projects/:project/tasks?status=` (GET), `PATCH
  /projects/:project/tasks/:task`, `/projects/:project/roadmaps/:roadmap/phases?status=`
  (GET), and `PATCH /projects/:project/roadmaps/:roadmap/phases/:phase`.

### Added

- New `hook_timeout_secs` config option (repo `rdm.toml` and global config,
  same precedence as `default_branch`) bounds how long `rdm hook post-merge`
  / `rdm hook post-commit` may run before giving up. Defaults to 30 seconds
  when unset (a configured `0` is treated the same as unset — an unbounded
  timeout would defeat the point of the guard). A hook that hits its
  deadline logs a `timeout` event and still exits 0, so it can never block
  the invoking `git commit`/`git merge` indefinitely.
- New `rdm_status`, `rdm_commit`, and `rdm_discard` MCP tools mirroring the
  CLI's `status`/`commit`/`discard`: inspect staged changes (path + change
  kind), land a batch as one commit with an explicit or auto-generated
  message, or discard staged changes (requires `confirm: true`). Gated on
  the `git` feature.
- The MCP server now exposes the LLM revision workflow, so an agent can
  discover and act on document reviews entirely over MCP:
  - `rdm_review_requests` — the change-request queue (submitted reviews
    with verdict `request-changes`), each entry carrying its target,
    author, summary, `created_commit`, and `open_comment_count`, with
    optional `target_kind`/`target_id` filters.
  - `rdm_review_show` — the full review in one call: summary, every
    comment with its tagged-union anchor (`anchor_type`) and computed
    resolution (`resolved`/`drifted`/`unresolved` with byte range and
    which body it indexes), plus a `documents` array inlining each
    referenced document's body at the review's `created_commit` and at
    HEAD. Unknown anchor types round-trip verbatim and resolve as
    `unresolved` (whole-document treatment).
  - `rdm_review_address_comment` — flips a comment to `addressed` or
    `wont-fix`, records the `applied_commit` provenance SHA, and stores
    the agent's reply; omit `status` to leave the comment open with a
    clarification reply. `applied_commit` is passed through verbatim
    (including for `wont-fix`) and left `null` if omitted — thread the
    `Commit:` value the `rdm_commit` tool reports for the batch that
    applied the fix.
  - `rdm_review_complete` — closes a review as `addressed`, refusing
    while any comment is open and listing the offending comment ids.
- New `rdm-revise` Claude Code / Pi skill (generated by
  `rdm agent-config --skills` in both CLI and MCP variants, and invocable
  as `/rdm-revise`): walks an agent through the revision loop — read the
  review summary for intent first, dispatch each comment on its
  `anchor_type` (resolved span, drifted span, or whole-document), apply
  edits through `rdm ... update`, record `applied_commit` + reply per
  comment, ask for clarification (leaving the comment open) when an
  anchor drifted beyond recovery, `wont-fix` with reasoning as the escape
  hatch, and close the review once nothing remains open.
- `rdm agent-config` instructions (CLI and MCP variants) now document the
  document-review agent loop and the new MCP review tools.
- New core helper `rdm_core::ops::reviews::change_requests` — the single
  definition of the change-request queue shared by `rdm review requests`
  and the MCP `rdm_review_requests` tool.
- New hermetic regression harness
  `scripts/verify-review-revision-loop.sh` covering the resolved-anchor,
  whole-document, drifted-anchor-clarification (blocks close), wont-fix,
  and completion paths of the revision loop end to end.
- With JavaScript enabled, `rdm-server` detail pages now support
  GitHub-style select-to-anchor review comments: while your draft review
  is open, highlighting text in a rendered roadmap, phase, or task body
  pops an "Add review comment" affordance that attaches a
  text-quote-anchored comment to the draft. The selection is mapped back
  to the markdown source (formatting spans, inline code, lists, tables,
  and multi-byte text included) and re-validated server-side before
  anything is stored, so the anchor always re-resolves to the exact
  selected span and later renders as an inline highlight; selections that
  cannot be mapped confidently degrade to a general comment carrying the
  selected text as a blockquote plus a visible "no anchor attached" note —
  a wrong anchor is never stored. The draft panel now updates in place
  without a full page reload and shows a quote preview on pending
  anchored comments. The plain-HTML review flow keeps working unchanged
  with JavaScript disabled.
- The roadmap detail page now renders each phase's body inside a
  collapsed, keyboard-accessible disclosure (native `<details>` — the
  phase list replaces the old table). Expanding a phase and selecting
  text in its body attaches the comment to the open roadmap draft scoped
  to that phase, so phase-level feedback can be authored from the roadmap
  page. Phase bodies are omitted when viewing a pinned `?at=` revision,
  and inline review highlights still live on the phase pages (the
  existing cross-links).
- `rdm-server`'s roadmap, phase, and task detail pages now support
  authoring reviews entirely through plain HTML forms — no JavaScript
  required. A "Start review" form begins (or resumes, if one is already
  open on the document by the same author) a draft; a server-rendered
  draft panel lets you add whole-document comments (with an optional
  phase-scoped dropdown on roadmap pages), edit or remove pending
  comments, and submit with a required verdict (Comment / Approve /
  Request changes) plus a summary. Draft comments stay private to the
  draft panel and never appear in the public Reviews section until
  submitted. Submitted reviews gain an inline Dismiss control, and drafts
  a Delete button. Author identity comes from the form and is remembered
  across visits via an `rdm_author` cookie so the panel resumes your own
  open draft, not someone else's. Validation and lifecycle errors
  (missing verdict, blank comment, editing after submit, out-of-scope
  phase scope) redirect back to the page with a readable inline banner
  instead of a raw Problem+JSON body.
- `rdm-server`'s roadmap, phase, and task detail pages now render a Reviews
  section: every non-draft review of the document (submitted, addressed, or
  dismissed) with its state and verdict badges, author, relative timestamp,
  summary, and comments in order — drafts are never shown. Anchored
  comments show a quote preview; on the current body, hovering or
  keyboard-focusing the preview highlights the resolved span inline in the
  rendered body (a small new `/static/review-highlight.js` script; pages
  stay fully readable with JavaScript disabled, degrading to the quote
  preview). Anchors that no longer resolve (or, once a history-aware
  backend lands, have drifted) get an "outdated" badge and show the
  original quote instead of a highlight. Roadmap-review comments scoped to
  a phase (`doc`) link through to that phase — where they render and
  highlight — and link back to the roadmap review from there. The roadmaps
  and tasks list pages gain a Reviews column with open-review/open-comment
  counts per item (phase-targeted reviews rolled up into their roadmap,
  matching `INDEX.md`), linking to the item's Reviews section.
- `rdm-core`: `ops::reviews::count_open_reviews` (and its slice-based
  sibling `count_open_reviews_in`) is the single open-review/open-comment
  counting pass shared by `INDEX.md` generation and the web list pages, so
  the two surfaces can never report different numbers.

- `rdm-server` now exposes a REST API for document reviews under
  `/projects/:project/reviews`: `GET` lists reviews as lightweight metadata
  summaries (filterable by `?on=<kind>/<id>`, `?state=`, `?verdict=`, and
  `?author=`); `POST` starts a draft (`{"target": "<kind>/<id>"}` plus
  optional `author` and initial `summary`); `GET /:id` returns the full
  detail — summary, comments, and each comment's anchor resolution
  (resolved / drifted / unresolved, with the quoted text, byte range, and
  which body the range indexes) so clients can highlight without extra
  calls; `POST /:id/comments` adds a comment (optionally anchored via a
  tagged `anchor` object or scoped to a roadmap phase via `doc`); `PATCH
  /:id/comments/:n` edits a draft comment's `body`/`anchor`/`doc` — omit a
  field to keep it, send `null` to clear it, or send a value to replace it
  — or, once submitted, records `status`/`applied_commit`/`reply`; `POST
  /:id/submit` stamps a `verdict` (optionally replacing the `summary`);
  `PATCH /:id` transitions to `addressed` or `dismissed`; and `DELETE /:id`
  removes drafts (submitted reviews are part of the record and return 409).
  All lifecycle rules are enforced by `rdm-core` and surface as RFC 9457
  Problem+JSON with actionable detail, and an anchor with an unrecognized
  `anchor_type` round-trips through the API untouched.
- `rdm-core::anchor::resolve_comments` runs the per-comment anchor
  resolution pass for a whole review in one call — the shared helper behind
  both the CLI's review rendering and the new server review endpoints.

- The full review-authoring loop is now available on the CLI, joining the
  existing needs-review queue commands under `rdm review` (whose `--help`
  now groups the two families): `start --on <kind>/<id>` creates a draft
  review of a roadmap, phase (`phase/<roadmap>/<stem-or-number>`), or task;
  `comment <id>` appends a comment — with `--quote "<text>"` the quoted
  text is located in the document **as of the review's `created_commit`**
  and a text-quote anchor (with ~32 chars of surrounding context) is
  derived automatically, an ambiguous quote fails with a 1-based occurrence
  list to disambiguate via `--occurrence <n>`, and `--doc
  phase/<stem-or-number>` scopes a roadmap-review comment to one of its
  phases; `submit <id> --verdict approve|request-changes|comment` finalizes
  the draft (optionally replacing the summary with `--body`); `list`
  filters by `--on`/`--state`/`--verdict`/`--author`; `show <id>` renders
  the summary and each comment with its anchor quote and resolution state
  (resolved / drifted / unresolved), with `--no-body` to suppress bodies;
  `update <id>` records comment resolutions (`--comment <n> --status
  addressed|wont-fix [--applied-commit <sha>] [--reply "..."]`) and closes
  the review (`--state addressed|dismissed`), validated by the lifecycle
  state machine; `delete <id>` removes drafts (submitted reviews require
  `--force`); and `requests` is the agent work queue (submitted reviews
  requesting changes). `--format json` on `list`, `show`, and `requests`
  includes each comment's full anchor and its resolution (with the quoted
  text and whether the range indexes the original or current body), so
  agents need no second call. All review mutations respect staging mode and
  the `--project`/`RDM_PROJECT`/`default_project` resolution chain.

- `rdm search` now indexes reviews: summaries and comment bodies are
  searchable, and `--type review` narrows results to them. `rdm describe
  review` documents the review entity, and `rdm agent-config` output
  teaches agents the new review commands.

- Anchored review comments can now be located within a target's body and
  re-located after the body is edited: exact-quote matching with
  prefix/suffix disambiguation for repeated text, and a fuzzy context
  fallback that recovers a drifted span from its surviving surrounding
  context. History-aware resolution finds the span in the body the reviewer
  originally saw (at the review's recorded commit), flags whether it has
  since drifted, degrades to the current body when history is unavailable,
  and reports unresolved (never failing) for unknown revisions, deleted
  targets, and unrecognized anchor types. Library-only groundwork
  (`rdm_core::anchor`); no CLI or API surface yet.

- Review operations in `rdm-core`: `create_review`, `add_comment`,
  `update_comment`, `remove_comment`, `submit_review`, `update_review`,
  `get_review`, `delete_review`, and review filtering (by target, state,
  verdict, and author) enforce the review lifecycle (`draft` → `submitted` →
  `addressed` | `dismissed`). Submitting requires a verdict and a non-empty
  review (at least one comment or a summary); comment structure locks after
  submission (only a comment's status, applied commit, and reply may still
  change); `addressed` requires every comment resolved; and terminal states
  reject further transitions. Creating a review validates the target exists
  and stamps the plan-repo HEAD the reviewer saw (`created_commit`), even
  under staging mode. `INDEX.md` now shows, per roadmap and per task, the
  count of open (submitted) reviews and the open comments within them —
  phase-targeted reviews roll up into their roadmap's row. Library-only
  groundwork: no CLI or API surface yet.

- A foundational Review data model in `rdm-core`. Reviews of a roadmap, phase,
  or task are stored as markdown files under a project's `reviews/` directory
  (`reviews/<id>.md`) with the review summary as the body and all metadata —
  state, verdict, timestamps, and the full list of inline comments — in the
  frontmatter. Comments can be anchored to a quoted span of the target's body
  (text-quote anchors with surrounding context), and anchor types this build
  does not recognize round-trip losslessly, so an older rdm never corrupts
  reviews written by a newer one. This is groundwork only: reviews can be
  written, loaded, and listed through the core library, with no CLI or API
  surface yet.

- `roadmap update`, `phase update`, and `task update` now accept `--title
  <TITLE>` to rename an item in place. Only the frontmatter title changes — the
  slug (for roadmaps/tasks) and the stem/number (for phases) are never touched,
  and `INDEX.md` is regenerated as part of the same mutation. An empty or
  whitespace-only `--title` is rejected with an actionable error (titles are
  required and cannot be cleared); omit `--title` to leave the existing title
  unchanged.

### Fixed

- Fixed a class of hangs that could leave a plan-repo mutation (or `rdm hook
  post-merge`/`post-commit`) stuck indefinitely, requiring a manual
  `SIGTERM`, in `--auto`/agent-driven flows:
  - Every git subprocess rdm spawns is now hardened to be strictly
    non-interactive — `GIT_EDITOR`/`GIT_SEQUENCE_EDITOR` are forced to a
    no-op so a merge that would otherwise open an interactive commit-message
    editor can't block on it, and `GIT_TERMINAL_PROMPT`/`GIT_ASKPASS` are
    forced so a `fetch`/`push` against an authentication-required remote
    fails fast instead of hanging on a credential or host-key prompt. This
    holds regardless of the invoking user's `core.editor`/`GIT_EDITOR`/
    `VISUAL`/credential-helper configuration.
  - `rdm remote pull`'s diverged-history merge now also explicitly passes
    `--no-edit` (defense-in-depth alongside the blanket editor hardening
    above).
  - `rdm hook post-merge`/`post-commit` now detect when they were themselves
    spawned as a git subprocess by rdm (as can happen if a real `git commit`
    made by `rdm resolve` re-triggers the plan repo's own installed hooks)
    and short-circuit immediately instead of re-running the `Done:`-directive
    pipeline.
  - `rdm hook post-merge`/`post-commit` execution is now bounded by the new
    `hook_timeout_secs` deadline (see Added, above) as a last-resort backstop.

## [0.15.0] - 2026-07-01

### Added

- A regression harness for the auto-review Stop hook loop,
  `scripts/verify-auto-review-hook-loop.sh`. It drives the real hook scripts
  end-to-end in hermetic temp dirs and asserts all four contract states against
  the concrete `.claude/hooks/rdm-review-on-finalize.sh` — fires
  `{"decision":"block",...}` when an item is `needs-review` on the current
  branch, stays silent on an unrelated branch, stays silent under the
  `stop_hook_active` loop guard, and stays silent once the item is `reviewed` —
  plus the shipped `rdm-core/src/templates/hook-review-on-finalize.sh`'s
  previously-untested loop-guard and reviewed-cleared cases. Auto-picked-up by
  the existing `scripts/verify-*.sh` CI glob.

- `phase update`/`task update --status needs-review` now warns on stderr when
  HEAD carries no committed changes worth reviewing, surfacing items that would
  otherwise be silently stranded in `needs-review` with nothing to review. For a
  phase, the baseline is the previous finalized phase in the same roadmap (so the
  long-lived `roadmap/<slug>` branch in the one-worktree-per-roadmap model is
  handled correctly): it warns when HEAD has not advanced past that phase's
  committed work. For the first phase of a roadmap and for standalone tasks, the
  baseline is the configured `default_branch`: it warns when HEAD has no commits
  beyond it. The transition is not blocked and the check fails open on any git
  error — this is a non-destructive data-integrity nicety, not a gate.

- `rdm review restamp` refreshes `review_sha`/`review_branch` on every in-scope
  `needs-review` item to the current source-repo HEAD and branch. Run it after
  amending or rebasing a commit while an item is still `needs-review`: the
  original stamp would otherwise point at a now-dangling commit and the item
  could silently drop out of `rdm review pending` scope (via the
  SHA-reachability fallback), suppressing the auto-review reprompt. Scope
  matches `review pending` exactly, and it is idempotent (items already stamped
  at the current HEAD/branch are left untouched). The Claude Stop hook, the Pi
  `agent_end` extension, the generated `hook-review-on-finalize.sh` template,
  and the shipped `.claude/hooks/rdm-review-on-finalize.sh` now call it
  automatically before checking `review pending`, so this self-heals
  transparently in the normal finalize → review loop.

- `rdm worktree prune` removes every worktree whose plan item is already `done`
  in one command. Resolves each rdm-managed worktree's item status (phase, task,
  or whole roadmap) and removes the done ones; dirty worktrees are skipped unless
  `--force`, `--delete-branch` also deletes their merged branches, and
  `--dry-run` reports what would be removed without changing anything. See
  [`docs/landing.md`](docs/landing.md).
- `rdm agent-config --skills` now emits an `rdm-land` skill (CLI and MCP
  variants) that lands a `reviewed` item to `main` with **linear history**
  (rebase onto `main`, then `git merge --ff-only` — never a merge commit),
  re-running the CI-equivalent checks on the rebased branch first. The
  fast-forward flips the item to `done` via the existing post-commit hook, and
  the skill then cleans up the worktree (`rdm worktree remove --delete-branch`,
  or `rdm worktree prune` for batch cleanup). On rebase conflict or failing
  checks it aborts cleanly and escalates per `docs/escalation-protocol.md`
  instead of force-merging. Landing runs only on explicit invocation (or
  autopilot's opt-in `--land`); it never auto-lands. See
  [`docs/landing.md`](docs/landing.md).
- `rdm-autopilot` agent skill (shipped by `rdm agent-config --skills`, in both
  CLI and MCP variants). It drives **one named roadmap** from `not-started` to
  `reviewed` unattended: each iteration asks `rdm next` for the next actionable
  phase, estimates it (`rdm-estimate`) if needed, dispatches it on its model
  tier through `rdm-dispatch-phase` (plan gate, implementation, `rdm-review`),
  interprets the `reviewed`/`rework`/`escalated` outcome, and advances —
  parking a phase `blocked` when its rework budget is exhausted so the loop
  steps past it. Decisions and blockers are **batched, not raised mid-run**
  (review them with `rdm review blocked`), and the run is bounded by a global
  step budget. Opt-in `--land` (default OFF — `main` is never touched without
  it) and bounded `--plan-only` / `--max-phases` dry-run modes. See
  [`docs/autonomous-loop.md`](docs/autonomous-loop.md).

### Fixed

- `rdm worktree prune --delete-branch` now reports partial success when a done
  item's worktree is removed but its branch is retained because the branch is not
  merged into HEAD (and `--force` was not passed). Previously the whole operation
  was reported as `failed` even though the worktree removal succeeded, so the
  `removed`/`failed` counts misled and the orphaned branch could not be
  re-cleaned by a later prune. There is now a distinct `removed-branch-kept`
  action with a per-result `reason`, a top-level `branch_kept` count in
  `--format json`, and a matching `N branch kept` figure and `removed, branch
  kept (…)` note in the text summary.
- `rdm worktree prune` now reports a worktree that became dirty between the
  initial scan and its removal as `skipped-dirty` rather than `failed`.
- `rdm worktree add`/`list`/`remove` now work when the project's canonical repo
  is **bare** (no working tree of its own) — whether invoked from a linked
  worktree of the bare repo or from inside the bare directory itself. Previously
  every subcommand failed with "not inside a git repository". Sibling worktrees
  are placed under `<parent>/<repo-name>__worktrees/`, with any `.git`/`.bare`
  suffix stripped from the anchor name so the layout matches the normal-repo
  case.
- `rdm worktree` commands run against a working directory that does not exist (or
  is not a directory) now report an actionable "directory does not exist" error
  instead of the misleading "git is not installed".

### Changed

- The `rdm-do`, `rdm-review`, and `rdm-dispatch-phase` skills emitted by `rdm
  agent-config` (CLI and MCP variants) now describe the split `Done:` line as a
  single deferred two-stage protocol. The finalize step says the `Done:` line is
  withheld _YET_ because `rdm-review` adds it on a passing review, and the review
  gate's `git commit --amend` step says it is _completing_ that deferred
  directive — so neither stage reads as contradicting the other. This stops
  auto-mode agent permission classifiers from denying the review-time amend as a
  violation of the finalize-time "no `Done:` line" instruction.
- The `rdm-review` skill emitted by `rdm agent-config` now runs the full
  **find → verify → filter → report → act → gate** pipeline: an adaptive review
  fleet (base AC-compliance + correctness agents, plus conditional agents gated
  on what the diff touches), a per-finding adversarial refute/verify pass where
  the agent that finds an issue is never the one that confirms it, and a
  confidence filter that drops refuted or low-confidence findings before
  anything is fixed or filed. This brings the generated skill to parity with the
  in-repo dogfooding `rdm-review` skill (previously it shipped an older
  fixed-two-agent find → report → act flow). Both the CLI and MCP flavors are
  updated.
- The `rdm-review` skill emitted by `rdm agent-config` now escalates a review to
  a **BLOCKED** verdict when any surviving finding is `blocking`, matching the
  in-repo skill. It adds an explicit severity scale, a strict verdict-order
  (BLOCKED → FAIL → PASS WITH CONCERNS → PASS), and a gate that routes a BLOCKED
  phase to `blocked` and a BLOCKED task to `in-progress` (tasks have no `blocked`
  status) instead of silently downgrading blockers to "pass with concerns". Both
  the CLI and MCP flavors are updated.

## [0.14.0] - 2026-06-29

### Changed

- _Development:_ the shared `.githooks/pre-commit` gate is now driven by
  [`hk`](https://hk.jdx.dev/) (declared in `hk.pkl`, provisioned by `mise
  install`) instead of a hand-rolled cargo script. The repo's shell scripts are
  now linted (`shellcheck`) and formatted (`shfmt`, 4-space / indented case via
  `.editorconfig`) the same way Rust is, enforced in both pre-commit and CI. CI
  additionally runs the `scripts/verify-*.sh` integration harnesses (on pushes
  to `main` and on pull requests). `post-commit` / `post-merge` (the rdm `Done:`
  hooks) are unchanged.
- The `rdm-dispatch-phase` agent skill now targets **one worktree per roadmap**,
  reused across phases: step 3 creates (or idempotently reuses) the roadmap's
  shared `roadmap/<slug>` worktree via `rdm worktree add <slug>` instead of a
  per-phase `<slug>/<phase-stem>` worktree, so an autonomous roadmap run no
  longer spins up a fresh worktree for every phase. The auto-review Stop hook
  and Pi extension are unchanged in behavior but now document the one-worktree
  model (they fire from the roadmap worktree on the `roadmap/<slug>` branch, so
  the branch-scoped review filter resolves exactly that roadmap's items), and
  the README worktree docs cover roadmap-scoped worktrees.
- Finalizing a phase or task into `needs-review` now also stamps the branch of
  the checkout that produced it (`review_branch`), and `rdm review pending`
  scopes the queue to the current checkout's branch: it keeps only items whose
  stamped branch matches, so a roadmap's review trigger can never pick up
  another roadmap's items even when it fires from a different checkout. Legacy
  items finalized before this change carry no branch and fall back to the
  previous SHA-reachability behavior (fail open), so nothing pre-stamp is
  dropped. The `rdm review pending --format json` output now includes a
  `branch` field.

### Added

- A cross-host worktree-review regression harness,
  `scripts/verify-worktree-review-loop.sh`. It drives the full
  do → finalize → trigger → review loop for the one-worktree-per-roadmap model
  across two roadmaps in hermetic temp dirs and asserts roadmap isolation — a
  roadmap's review trigger fires that roadmap's review and stays silent about
  the other — across both supported host paths (the Claude Stop hook template
  and the Pi `agent_end` contract), including that a trigger from the `main`
  checkout never misfires for an in-flight roadmap review.
- `rdm worktree add <roadmap-slug>` now accepts a bare roadmap reference and
  creates (or idempotently reuses) a single worktree on a `roadmap/<slug>`
  branch — one worktree per roadmap, shared by all its phases — at
  `<repo>__worktrees/roadmap-<slug>`. `rdm worktree current` reports that
  roadmap context (via marker or by inverting the `roadmap/<slug>` branch name),
  and `rdm worktree remove` accepts the bare roadmap form too. Also exposed
  through the `rdm_worktree_add` / `rdm_worktree_current` MCP tools. This is the
  isolation unit for the one-worktree-per-roadmap rdm-do flow.
- `rdm worktree current` reports the plan item the current checkout corresponds
  to — the rdm worktree marker if present, otherwise the item inferred from the
  branch name (`phase/<roadmap>/<stem>` or `task/<slug>`), so a hand-made
  worktree or the main checkout sitting on an item branch is also recognized. It
  prints `Not in an rdm worktree.` (text) / `null` (JSON) and exits 0 when the
  checkout is on neither (e.g. the main checkout on `main`). Exposed as the
  `rdm_worktree_current` MCP tool as well. This is the detection primitive the
  `rdm-do` skill will use to reuse the current worktree instead of creating a
  redundant nested one.
- `rdm-dispatch-phase` agent skill (shipped by `rdm agent-config --skills`, in
  both CLI and MCP variants). It runs a single roadmap phase end-to-end in an
  isolated worktree on the phase's assigned model tier and returns a structured
  outcome (`reviewed` | `rework` | `escalated`) for an orchestrator to act on. A
  fresh implementer subagent is seeded with only that phase's body and the repo,
  drafts a tactical plan, and — because autopilot has no human to approve the
  plan — a *separate*, lightweight reviewer gates the plan against the phase's
  acceptance criteria, scope, and the core/cli/server separation before any code
  is written, returning approve / revise / escalate. The plan gate is bounded
  (one review pass plus at most one revise round); code review is delegated to
  `rdm-review`, and a genuine AC/architecture ambiguity parks the phase as
  `blocked` rather than guessing.
- A phase can now be parked as `blocked` with a recorded escalation reason.
  `rdm phase update <phase> --status blocked --reason "<why>"` stores the reason
  in the phase's frontmatter (`blocked_reason`); `--clear-reason` removes it. The
  reason is shown by `rdm phase show` (human and `--format json`) and is
  preserved across a later resume — moving a phase back to `in-progress` no
  longer loses why it stalled. The MCP `rdm_phase_update` tool gains matching
  `reason` / `clear_reason` parameters.
- `rdm review blocked` lists every phase parked as `blocked` — the escalation
  queue awaiting a human decision — with its recorded reason, so decisions can be
  answered in a batch instead of interrupting a run mid-flight. `--format json`
  emits an array of `{identifier, project, title, reason}`; `--project` selects
  the project.
- Escalation protocol documentation (`docs/escalation-protocol.md`): the single
  shared definition of when an autonomous run interrupts a human versus parks a
  decision. It distinguishes routine findings (never escalate; handled by
  `rdm-review`) from decisions/blockers (escalate), tags each escalation with its
  stage (`plan` vs `code`), specifies the plan-revise and rework-retry budget
  triggers, and defines the auto-handle / park-as-blocked / raise-to-user
  decision rule.

### Changed

- The `rdm-do` skill now uses a **one-worktree-per-roadmap, work-in-place**
  model. A roadmap gets a single worktree (`roadmap/<slug>` branch) and every
  phase is implemented in place in it: the skill reads `rdm worktree current`,
  compares the current worktree's roadmap to the target, and **works in place**
  on a match (the common case for every phase after the first), creates/enters
  the roadmap worktree once from the main checkout on a miss, or switches on a
  mismatch (interactively asking, or automatically under `--auto`). Because entry
  happens at most once and the session never re-enters or nests, `EnterWorktree`
  is now a one-time convenience rather than a correctness dependency — non-Claude
  hosts (Pi, web, MCP) get a fully correct entry path via plain `cd`/launch. The
  MCP variant additionally renders the `rdm_worktree_current` tool. Tasks keep
  their existing per-task worktree.

### Fixed

- `rdm hook post-commit` / `post-merge` now always commit the `Done:`
  phase/task updates they apply, even when staging mode is enabled via
  `--stage`, `RDM_STAGE`, or `stage = true` in `rdm.toml`. Previously the hook
  inherited the resolved staging preference, so the update could be written to
  disk without a commit and silently lost (leaving the plan repo dirty and the
  `Done:` directive unapplied).

## [0.13.0] - 2026-06-19

### Added

- `rdm review pending` lists the `needs-review` phases and tasks that are in
  scope for the current source-repo branch — those whose source-repo SHA
  (stamped when the item entered `needs-review`) is reachable from the current
  HEAD, plus any unstamped/legacy items (which fail open). It is the single
  shared source of truth for the auto-review Stop hook and the `rdm-review`
  skill, so they never disagree about what to review. `--format json` emits an
  array of `{kind, identifier, project, title}`; `--project` selects the
  project. Available when built with the `git` feature.

- `rdm-estimate` agent skill (shipped by `rdm agent-config --skills`, in both
  CLI and MCP variants). Given a roadmap slug or a single phase, it reads each
  phase body, rates its difficulty (`trivial` | `easy` | `moderate` | `hard`)
  with a one-line justification, records that note in the phase body, and sets
  the difficulty via `rdm phase update` — the model tier is assigned
  automatically from the difficulty. Phases that already have a difficulty are
  skipped, so re-running is idempotent and never overwrites a human-set value;
  clear a phase's difficulty to re-estimate it.
- `rdm next --roadmap <slug>` prints the next actionable phase in a roadmap
  (text and JSON): the lowest-numbered phase that is `not-started` or
  `in-progress`, skipping phases under review, done, blocked, or won't-fix.
  Roadmap dependencies are honored — if a dependency roadmap is not yet
  complete, the command reports a distinct `blocked-on-dependencies` result
  listing the unmet slugs; when nothing is actionable it reports `nothing`. All
  three outcomes exit 0. `--roadmap` is required (scope is one roadmap at a
  time; there is no project-wide scan).
- Phases now carry optional `difficulty` (`trivial` | `easy` | `moderate` |
  `hard`) and `model` tier (`small` | `medium` | `large`) fields. Set them at
  creation with `rdm phase create --difficulty <d> --model <m>` or later with
  `rdm phase update --difficulty <d>` / `--model <m>` (and `--clear-difficulty`
  / `--clear-model` to remove them). Both are surfaced in `rdm phase show` and
  `rdm phase list` (text and JSON) and reported by `rdm describe phase`. These
  fields are foundational metadata for upcoming difficulty-aware model
  selection.
- The rdm MCP server now exposes worktree lifecycle tools — `rdm_worktree_add`,
  `rdm_worktree_list`, and `rdm_worktree_remove` — mirroring the `rdm worktree`
  CLI commands. They run against the project (code) repo discovered from the
  server's working directory (refusing to run inside the plan repo) and are
  available when the server is built with the `git` feature. With them, the MCP
  `rdm-do` skill now creates and works inside an isolated git worktree via the
  MCP tool (no Bash), matching the CLI skill's behavior.

- `rdm agent-config pi --hooks` ships the Pi auto-review extension to end-user
  projects. It writes `.pi/extensions/rdm-review.ts`, which Pi auto-discovers
  (no settings registration). The extension subscribes to Pi's `agent_end`
  lifecycle event and re-prompts the agent to run the `rdm-review` skill while
  any item is in `needs-review`; it calls `rdm` on `PATH` with standard project
  resolution (no hard-coded project). The flag is composable with `--skills` and
  honors `--out <dir>` (project `.pi/`) and `--user` (`~/.pi/agent/`).
- `rdm agent-config claude --hooks` ships the auto-review Stop hook to end-user
  projects. It writes a generalized `.claude/hooks/rdm-review-on-finalize.sh`
  (executable; calls `rdm` on `PATH` and uses standard project resolution
  instead of a hard-coded project) and registers it under `hooks.Stop` in
  `.claude/settings.json`, merging non-destructively into any existing settings
  (other keys preserved; re-running is idempotent). The flag is claude-only and
  composable with `--skills`; it honors `--out <dir>` and `--user` (`~/.claude/`).
- `rdm worktree` command family (`add` / `list` / `remove`) for managing git
  worktrees in your project (code) repo, keyed to plan items. `add <item>`
  creates (or idempotently reuses) a worktree and branch for a phase
  (`<roadmap>/<phase-stem-or-number>`) or task (`task/<slug>`); branches are
  named `phase/<roadmap>/<stem>` or `task/<slug>`, and worktrees live as
  siblings of the repo under `<repo>__worktrees/`. `--base <ref>` chooses the
  branch point (default current HEAD); `--format json` emits the item, branch,
  path, and created flag. `list` shows item/branch/path and a dirty flag.
  `remove <item|path>` deletes a worktree (refusing a dirty tree without
  `--force`), with `--delete-branch` to drop the branch too (refusing unmerged
  commits without `--force`). Commands run against the repo discovered from the
  current directory and refuse to run inside the plan repo. Only rdm-created
  worktrees (tracked via an internal marker) are listed or removable.
- New `rdm-tui` crate: a terminal UI binary (`rdm-tui`) that opens an
  interactive screen listing the projects in your plan repo. It resolves the
  plan repo the same way the CLI does (`RDM_ROOT`, global config `root`, then
  the XDG data dir), shows a hint when no projects exist yet, and quits on `q`,
  `Esc`, or `Ctrl-C` while always restoring the terminal — even on a panic.
  This is the foundation for richer roadmap and task browsing in later
  releases; it is read-only and does not yet open roadmaps or tasks.
- The `rdm-tui` terminal UI now navigates: press `Enter` on a project to open
  its roadmap list (showing each roadmap's status, slug, title, priority, and
  `done/total` phase progress), and `Enter` on a roadmap to open its detail
  view (the roadmap body plus its phases with status badges). `Esc`/`h` go back
  one screen — restoring the previous cursor position — and `Esc`/`h` from the
  project list quits. Statuses are shown as text labels with ASCII symbols so
  they remain distinguishable without color.
- The `rdm-tui` terminal UI now opens a phase to a detail screen: press `Enter`
  on a phase to see a metadata block (status, completion date, short commit SHA,
  tags) above the phase body rendered as terminal markdown — headings, bold/
  italic/strikethrough, lists, code blocks, GFM tables, and block quotes are all
  visually distinct without color. Cycle between phases with `n`/`p` (or the
  arrow keys), and scroll long bodies with `j`/`k`, `PageUp`/`PageDown`, and
  `Ctrl-u`/`Ctrl-d`. `Esc`/`h` returns to the phase list with its cursor intact.
- The `rdm-tui` terminal UI now opens a per-project task list: press `t` from the
  project list to browse a project's tasks in columns (slug, title, status,
  priority, tags), and use `t`/`r` to switch between the task and roadmap lists.
  Quick-filter the list by cycling the status filter with `s`
  (`all`→`open`→`in-progress`→`done`→`wont-fix`) and toggling a tag-filter popup
  with `f` (`space` to toggle tags, `enter` to apply, `esc` to cancel). Press
  `Enter` on a task to open a scrollable detail screen with its metadata (status,
  priority, tags, created/completed dates, short commit SHA) above its markdown
  body; `Esc`/`h` returns to the list with the cursor and filters intact.
- `rdm_core::ops::task::filter_tasks` (and the `TaskFilter`/`task_matches`
  building blocks): a reusable task-filtering op over status, priority, and tags
  (AND), now shared by the CLI's `task list` and the TUI's task browser.
### Changed

- `rdm phase update --difficulty <d>` now auto-derives `--model` from the
  difficulty→tier mapping (`trivial`/`easy` → small, `moderate` → medium,
  `hard` → large) when `--model` is omitted and no model is already set. An
  explicit `--model` / `--clear-model` and any previously set model are
  respected — the derive only fills an empty model.
- Each plan-repo mutation (create/update/delete/promote/archive/split of a
  roadmap, phase, task, or project) now records a **single** git commit that
  bundles the entity change with the regenerated `INDEX.md`, instead of two
  separate commits. `INDEX.md` regeneration is now an inherent part of every
  mutation, so the index can no longer drift out of date. `--no-index` still
  skips regeneration (committing the entity change alone) as before.
- Roadmap aggregate-status computation (the overall `not-started` /
  `in-progress` / `done` derived from a roadmap's phases) now lives in
  `rdm-core` so every interface shares one implementation. No behavior change
  to the server or web UI.
- `rdm-do` (the in-repo Claude Code skill plus the shipped CLI and MCP skill
  templates) now does its work in an isolated git worktree created after marking
  the item in-progress instead of the live checkout. The CLI variants use
  `rdm worktree add <item>`; the MCP variant drives the equivalent
  `rdm_worktree_add` MCP tool (it no longer works in the live checkout). All
  three variants (dogfood, CLI, MCP) also gain two run modes: interactive
  (default — plan, approval gate, review-with-user) and `--auto` non-interactive
  (skips the approval and review gates and finalizes autonomously). For
  unattended Claude Code runs, launch with `--permission-mode auto` (or
  `bypassPermissions` in a sandbox) so file edits and bash/tool calls don't
  block on prompts. The finalize contract is unchanged (commit on the branch,
  set `needs-review`); the branch is left for merge to main.
- The MCP server and REST API no longer emit CLI-specific navigation hints
  (`rdm phase show …`) in `roadmap show` / `phase show` output. These
  `Hint:` / `Prev:` / `Next:` lines now live only in the `rdm` CLI, where the
  output is unchanged. Markdown table separators (`--format markdown` and
  generated `INDEX.md`) now render with fixed-width `---` / `---:` cells; column
  alignment is unchanged.

### Fixed

- The auto-review Stop hook (and the `rdm-review` skill) no longer misfire
  across worktrees: an item finalized to `needs-review` on one branch no longer
  reprompts a session finishing an unrelated branch, where that item's diff
  isn't even checked out. The `needs-review` transition now stamps the
  source-repo HEAD SHA, and `rdm review pending` scopes the prompt to items
  reachable from the current HEAD (unstamped/legacy items still fail open).
- Marking a task `wont-fix` now stamps a `completed` date (and records the
  optional commit SHA), matching the behavior of marking it `done`. Previously
  `wont-fix` tasks were left with no completion date.
- `rdm worktree add` run from inside a linked worktree now creates the new
  worktree as a sibling of the main repo instead of nesting it under the current
  worktree. Discovery resolves the repository's main working tree rather than the
  current working tree's top-level.
- Merge-conflict output now shows roadmap/phase context for conflicted phase
  files. Phase conflicts were previously misclassified as generic ("Other")
  paths because the classifier expected a layout the tool never writes, so the
  conflict listing dropped their roadmap and phase names.

## [0.12.0] - 2026-06-12

### Added

- Dogfood Claude Code Stop hook (`.claude/hooks/rdm-review-on-finalize.sh`)
  that reprompts the agent to run the `rdm-review` skill while any rdm item is
  in `needs-review`. The status is the sentinel — there is no marker file — so
  once review moves the item out of `needs-review` the next stop is allowed,
  and `stop_hook_active` prevents reprompt loops. Wires `.claude/` in this repo
  only; shipping equivalent config from `rdm agent-config` is tracked
  separately.
- `needs-review` and `reviewed` statuses for both phases and tasks, accepted
  everywhere statuses are (CLI `--status` on `create`/`update`/`list`/`search`,
  the REST server status selects and request parsing, the MCP tools, the
  `rdm describe` schema, and the web UI status badges).
  `needs-review` means implementation is finalized and awaiting review;
  `reviewed` means review passed and the item is awaiting merge to main (where
  the existing `Done:` merge hook flips it to `done`). Both are non-terminal,
  so they never stamp a completion date. The intended review lifecycle
  (`in-progress → needs-review → reviewed → done`) is documented in the agent
  instructions; status transitions remain unconstrained.

### Changed

- Removed the unmaintained transitive dependency `proc-macro-error2`
  (RUSTSEC-2026-0173) by building CLI tables with tabled's `Builder` API
  instead of its derive feature (and bumping tabled to 0.21). Table output is
  unchanged.
- `rdm-review` (the in-repo Claude Code skill plus the shipped CLI and MCP
  skill templates) now categorizes findings by size and owns the status
  transition. Small findings (localized, low-risk) are fixed inline and amended
  into the implementation commit; large findings (new modules, cross-cutting)
  are filed as rdm tasks instead of being fixed inline. On a passing review the
  skill sets the item to `reviewed` and writes the `Done:` line (the merge hook
  later flips it to `done`); on substantial rework it returns the item to
  `in-progress` with no `Done:` line.
- `rdm agent-config --skills` now emits a single `rdm-do` skill (CLI and MCP)
  in place of the previous `rdm-implement` and `rdm-tasks` skills, so the
  shipped skill set drops from 5 to 4 (`rdm-roadmap`, `rdm-do`, `rdm-review`,
  `rdm-document`). The merged `rdm-do` skill handles both roadmap phases
  (`<roadmap-slug> [phase-number]`) and tasks (`--task <slug>`), and finalizes
  by transitioning the item to `needs-review` (no `Done:` line); `rdm-review`
  produces the `Done:` line on a passing review.
- The in-repo Claude Code skills `rdm-implement` and `rdm-tasks` are merged
  into a single `rdm-do` skill that handles both roadmap phases
  (`<roadmap-slug> [phase-number]`) and tasks (`--task <slug>`). Finalize no
  longer commits a `Done:` line straight to `done`; instead it commits the
  implementation and transitions the item to `needs-review`, leaving it for
  the `rdm-review` skill to produce the `Done:` line on a passing review.

### Fixed

- `rdm search --status <status>` without `--type` now matches both phases and
  tasks for statuses shared by both kinds (`in-progress`, `needs-review`,
  `reviewed`, `done`, `wont-fix`); previously it silently returned phases only.
  Applies to the CLI, the server's `?status=` filter, and the MCP `search`
  tool's `status` argument.

## [0.11.0] - 2026-06-05

### Added

- `rdm agent-config` now supports the `pi` coding agent platform. Writes to
  `.pi/AGENTS.md` for project-local config or `~/.pi/agent/AGENTS.md` with
  `--user`.
- `rdm agent-config --skills` now supports the `pi` platform, writing skill
  files to `.pi/skills/` (project) or `~/.pi/agent/skills/` (user).
- HTTP server now serves the project logo as an SVG favicon at `/favicon.ico`.
  The favicon is linked in the base template, displayed in browser tabs, and
  sent with a `Cache-Control: public, max-age=86400` header so browsers can
  cache it for a day.
- `--clear-body` flag on `phase update`, `task update`, and `roadmap update`.
  Use it to intentionally empty an existing body (it is mutually exclusive
  with `--body`).
- `clear_body` field on `PATCH /projects/:project/roadmaps/:roadmap`,
  `PATCH /projects/:project/roadmaps/:roadmap/phases/:phase`, and
  `PATCH /projects/:project/tasks/:task` (mirrors the new CLI flag).

### Changed

- `rdm agent-config claude --skills --out <dir>` now writes skill files under
  `<dir>/.claude/skills/rdm-*/SKILL.md` (treating `<dir>` as a project root).
  Previously the files were placed directly under `<dir>/rdm-*/SKILL.md`. The
  `--user` path is unchanged.
- Roadmap detail page now lays out its metadata (status, priority, last
  changed, dependencies, tags) in the same definition-list style as the
  task and phase pages, with badges for status and priority, instead of a
  loose run of inline paragraphs.
- `--body` is now authoritative on `phase`, `task`, and `roadmap`
  create/update: rdm no longer reads stdin when `--body` is provided,
  eliminating a fragile interaction with background and other
  non-interactive runners that could overwrite or mix bodies.
- `rdm agent-config pi --mcp` now exits with an actionable error
  pointing users to `--skills` (or omitting `--mcp` for the AGENTS.md
  integration). Pi has no native MCP support, so the previous behavior
  silently produced unusable output.

### Fixed

- `phase update`, `task update`, and `roadmap update` no longer silently
  overwrite a non-empty body with empty content. Passing an explicit
  `--body ""` against an existing body is now rejected with an actionable
  error; use `--clear-body` to confirm.

## [0.10.3] - 2026-05-15

### Fixed

- npm publish step now runs from inside the extracted package directory
  instead of passing the path as an argument, so npm no longer
  misinterprets the relative path as a GitHub `<owner>/<repo>`
  shorthand and fails with `Permission denied (publickey)`.

## [0.10.2] - 2026-05-15

### Fixed

- npm publish workflow now uses Node 24 instead of Node 22 to avoid the
  broken bundled npm in the 22.22.2 runner toolcache image (missing
  `promise-retry` module), which was causing the `npm install -g
  npm@latest` step to crash and prevent the npm package from being
  published on release.

## [0.10.1] - 2026-05-15

## [0.10.0] - 2026-05-15

### Added

- post-commit and post-merge hooks now write a diagnostic log to
  `<git_dir>/rdm-hook.log` on every invocation, recording entry, branch and
  directive decisions, per-directive apply outcomes, and any errors. The file
  is auto-truncated past 256 KB and lives inside `.git/`, so it is never
  committed. Re-run `rdm hook install --force` to pick up the matching shim
  update that also captures native git/binary errors into the same file.
### Added

- Prebuilt binaries for two additional target platforms —
  `x86_64-apple-darwin` (Intel macOS) and `aarch64-unknown-linux-gnu`
  (ARM64 Linux) — generated by cargo-dist on every release, alongside the
  existing `aarch64-apple-darwin` and `x86_64-unknown-linux-gnu` binaries.
- npm install method: `npx -y @edpaget/rdm mcp` (or `npm install -g
  @edpaget/rdm`) downloads the matching prebuilt rdm binary from the
  GitHub Release on install. Generated via the cargo-dist npm installer
  and published from CI on each release tag via
  [npm trusted publishing (OIDC)](https://docs.npmjs.com/trusted-publishers/),
  so no long-lived `NPM_TOKEN` secret is needed. Covers macOS arm64/x64
  and Linux arm64/x64.
- MCP Registry metadata at `io.github.edpaget/rdm` — `server.json`
  committed at the repo root, and the npm publish workflow now injects
  `mcpName: io.github.edpaget/rdm` into every published `package.json`
  so the MCP Registry can verify ownership. README adds Claude Code
  (`claude mcp add rdm -- npx -y @edpaget/rdm mcp`) and Cursor install
  snippets.

## [0.9.0] - 2026-05-15

### Added

- Revision-scoped reads on the storage layer: `Store::head_sha` and
  `Store::fetch_body_at(path, sha)` let callers read a target's body at a
  specific git revision. Backed by `git show` in the git store and a
  snapshot map (synthetic `mem-N` SHAs) in the memory store. Surfaces
  typed errors for unknown SHAs, paths missing at a SHA, and backends with
  no notion of history.
- `--at <sha>` flag on `rdm roadmap show`, `rdm phase show`, and
  `rdm task show` reads the body as it was at the given git revision while
  keeping current metadata. The same capability is exposed as the `?at=<sha>`
  query parameter on the matching `GET /projects/...` detail routes (HTML,
  HAL+JSON, Problem+JSON 404 on unknown/missing-at-revision SHAs). Text and
  Markdown output prepend a `Revision: <sha>` line; HAL+JSON includes a
  `revision` field; HTML detail pages render an `aria-live` "Viewing
  revision …" badge near the title.
- A small embedded JavaScript client (`/static/edit.js`) wired into every
  rdm-server page that intercepts `<form data-rdm-edit>` submissions, PATCHes
  the resource as JSON, reloads on success, and surfaces server validation
  errors inline.
- Inline status editor on the phase- and task-detail HTML pages: a `<select>`
  next to the status badge submits a `PATCH` (via `/static/edit.js`) and
  reloads the page with the new status. Phases include `wont-fix`; tasks
  include all four statuses.
- Inline body editor on roadmap-, phase-, and task-detail HTML pages: a
  collapsible `<details>` block exposes the raw markdown in a `<textarea>`
  and PATCHes the resource (via `/static/edit.js`) on submit. Clearing the
  textarea and saving is supported and round-trips. The rendered HTML body
  remains the default read view.
- Inline tag editor on roadmap-, phase-, and task-detail HTML pages: a
  collapsible `<details>` block exposes the current tags as a
  comma-separated `<input>` with "Save tags" and "Clear tags" buttons;
  submits PATCH the resource (via `/static/edit.js`) and reload.
  Whitespace, empty entries, and duplicates are normalized client-side.
  The phase detail page now also shows tags in the read view (parity with
  task/roadmap).
- Phase status `wont-fix`, treated like `done` for roadmap completion.
  Agent-facing surface area (`Describe` schema, agent-config instruction
  templates, and the CLI's combined-status error message) now lists
  `wont-fix` as a valid phase status and documents the
  `not-started`/`in-progress` → `wont-fix` transitions.

### Changed

- Quick-filter chips render right-aligned on the breadcrumb row as a
  horizontal group with vertical separators on desktop, and stack below
  the breadcrumb on narrow screens. The same placement is used on the
  roadmap list, roadmap detail, and task list pages.
- HTML pages now load their stylesheet from `/static/styles.css` instead
  of an inline `<style>` block, making each rendered page significantly
  smaller and allowing the browser to cache the stylesheet across
  navigations.

### Fixed

- Roadmap, phase, and task detail pages now render GFM pipe tables (and
  strikethrough, task lists, and GitHub-style `[!NOTE]` callouts) as proper
  HTML instead of passing the source syntax through as literal text.

## [0.8.0] - 2026-04-27

### Added

- Tags on roadmaps and phases. `--tags <csv>` on `roadmap create`, `roadmap
  update`, `phase create`, and `phase update` sets/replaces tags. Tags appear
  in `roadmap show`, `phase show`, and JSON output. Promoting a task
  preserves its tags onto the seed phase.
- `rdm search --tag <name>` filters results to items carrying the given tag.
  The flag is repeatable (`--tag bug --tag ui`) and ANDs together — an item
  must carry every listed tag to match. Items with no tags are excluded by
  any non-empty tag filter. Combine with `--type`, `--status`, etc., or use
  `--tag` with an empty query (`rdm search "" --tag bug`) to list every
  item carrying the tag. JSON results include a `tags` field.
- HTTP server tag filtering for roadmaps and phases:
  `GET /projects/<p>/roadmaps?tag=<t>` and the new
  `GET /projects/<p>/roadmaps/<r>/phases?tag=<t>` endpoint return only items
  with the given tag. The roadmap detail page also honors `?tag=<t>` to
  filter the embedded phases section. JSON responses include a `tags` field
  on roadmap and phase summaries/details. `POST` and `PATCH` bodies for
  roadmaps and phases now accept `tags: [...]` (and `clear_tags: true` on
  PATCH) to set, replace, or clear tags.
- `[server.quick_filters]` in `rdm.toml` defines named tag presets that
  render as clickable chips on the roadmap, phase, and task list HTML
  pages. Each chip links to the same page with `?tag=<value>`; the active
  chip is highlighted and an "All" link clears the filter. Override per-run
  via `RDM_SERVER_QUICK_FILTERS="Bugs:bug,UI:ui"` (env) or
  `rdm serve --quick-filter Bugs:bug --quick-filter UI:ui` (repeatable CLI
  flag). CLI flags > env > toml; higher-precedence sources fully replace
  lower ones rather than merging.
- MCP server tag support. `rdm_roadmap_create` and `rdm_phase_create` now
  accept a `tags` array; `rdm_roadmap_update` and `rdm_phase_update`
  accept `tags` (replace) and `clear_tags: true` (remove all). The
  `rdm_roadmap_list` and `rdm_phase_list` tools accept an optional `tag`
  filter, and `rdm_search` accepts a `tags` array (AND semantics) matching
  the CLI `--tag` flag.

### Changed

- `rdm agent-config` instructions (CLI and MCP variants) now demonstrate
  tagging: `--tags`/`tags: [...]` on create/update, `--tag`/`tag: "..."`
  on list, and the `--tag`/`tags: [...]` filter on search. Includes a
  short tagging convention note (lowercase kebab-case; check existing
  tags before inventing one). The `rdm-tasks` and `rdm-roadmap` skills
  (and their embedded templates) inherit the same examples.

## [0.7.1] - 2026-04-24

### Fixed

- `rdm hook post-merge` / `post-commit` no longer panic when a commit message contains a line starting with a multi-byte UTF-8 character (e.g. an em dash). The `Done:` prefix check now operates on bytes instead of slicing the string at a non-char-boundary.

## [0.7.0] - 2026-04-20

### Added

- Claude Code web sandbox template under `templates/claude-code-web/`: a `SessionStart` hook script, a `.claude/settings.json` snippet, and a `devcontainer.json` fragment that together install rdm and bootstrap a plan repo on session start. Drop them into a source repo with `scripts/install-claude-code-web-template.sh <target>` (idempotent; prompts before overwriting differing files). Full setup in `docs/claude-code-web.md`.
- `rdm bootstrap --token <token>` (also `RDM_PLAN_REPO_TOKEN` env) injects an access token into HTTPS clone URLs for private plan repos. SSH URLs are cloned as-is with a warning; plain `http://` URLs with a token are rejected. The token is never echoed to stdout or stderr, including on clone failures.
- `rdm bootstrap doctor` subcommand diagnoses sandbox readiness: rdm on PATH, configured plan-repo root, plan-repo URL, token presence, and — for GitHub HTTPS URLs — token scopes via `GET /repos/:owner/:repo`. Exits non-zero on critical failures so CI can gate on it.
- `docs/claude-code-web.md` now has a "Credentials" section covering fine-grained PATs (minimum scopes) and SSH deploy keys.
- `scripts/verify-claude-code-web-loop.sh` — hermetic end-to-end regression harness for the Claude Code web sandbox loop. Uses temp dirs and bare clones in place of GitHub; confirms the template, bootstrap, and source-repo `Done:` → plan-repo phase update all work together. Exits non-zero on any regression.

## [0.6.2] - 2026-04-12
### Added

- `rdm bootstrap --plan-repo <url> [--path <dir>] [--branch <name>] [--init]` clones a plan repo into a target directory (defaulting to `$XDG_DATA_HOME/rdm/plan-repo`) and fast-forwards it on subsequent runs. Designed for Claude Code web session-start hooks and other sandbox bootstrap scripts that need an idempotent "get me a plan repo" command.
- `install.sh` at repo root: `curl -fsSL https://github.com/edpaget/rdm/releases/latest/download/install.sh | sh` downloads a prebuilt rdm binary for the current platform. Supports `--version <tag>` to pin a specific release and `--dir <path>` to override the install location. Wraps the cargo-dist shell installer, which handles OS/arch detection and sha256 verification.
- CI workflow `install-test.yml` exercises `install.sh` on `ubuntu-latest` and `macos-latest` whenever `install.sh` changes.
- CI workflow `attach-install-sh.yml` attaches `install.sh` to each GitHub Release so the stable `releases/latest/download/install.sh` URL always resolves to the tagged version of the wrapper.

### Changed

- Upgraded rmcp dependency from 0.16 to 1.4
- `GitStore::clone_remote` now takes an optional `branch: Option<&str>` argument to clone a specific branch via `git clone --branch`
- Releases now publish an `x86_64-unknown-linux-gnu` tarball and a cargo-dist `rdm-cli-installer.sh` alongside the existing `aarch64-apple-darwin` tarball and Homebrew formula.

## [0.6.1] - 2026-03-31

## [0.6.0] - 2026-03-31

### Added

- Roadmap priority support in REST API: list/detail responses include priority, create accepts optional priority, new PATCH endpoint for updating priority, `?sort=priority` and `?priority=<level>` query params on list
- Roadmap priority support in MCP tools: `rdm_roadmap_create` accepts optional priority, `rdm_roadmap_list` supports sort and priority filter, new `rdm_roadmap_update` tool for setting/clearing priority and body
- Roadmap priority badges in HTML views: list page shows a Priority column and detail page displays priority next to status

## [0.5.0] - 2026-03-26

### Added

- `rdm_create_project` MCP tool to create new projects from within MCP clients
- Search results are now capped by relevance score, filtering out low-quality matches
- Optional `priority` field on roadmaps (`low`, `medium`, `high`, `critical`) — reuses the existing priority model from tasks
- `rdm roadmap create --priority <level>` and `rdm roadmap update` command for setting/clearing priority via CLI
- `rdm roadmap list --sort priority` sorts roadmaps by priority descending; `--priority <level>` filters by priority level
- `rdm roadmap show` displays priority when set

### Fixed

- `default_branch` is now recognized as a valid config key for `rdm config get` and `rdm config set`
- MCP server logs errors when store construction silently falls back instead of swallowing them

## [0.4.0] - 2026-03-24
### Added

- `rdm agent-config --user` writes agent config to the user-level config directory (e.g. `~/.claude/`) instead of a project directory, enabling global agent integration

## [0.3.1] - 2026-03-21

### Added

- `rdm agent-config --mcp` now generates MCP-oriented agent instructions referencing MCP tool names instead of CLI commands
- `rdm agent-config --mcp --skills` generates MCP-aware Claude Code skills that use `mcp__rdm__*` tools in `allowed-tools`
- When `--mcp --out` is used, `.mcp.json` is written alongside the instructions or skills
- MCP agent instructions include a Searching section with `rdm_search` tool

### Changed

- `--mcp` flag is no longer mutually exclusive with `--skills`; it is now a modifier that switches output to MCP tool references
- Restructured README to lead with installation and quick start, added "Core Workflow: Plan, Implement, Done" section showcasing the plan-implement-done cycle, and moved reference material (architecture, REST API endpoints) to dedicated docs

## [0.3.0] - 2026-03-21

### Added

- `rdm hook post-commit` subcommand: parses `Done:` directives from HEAD on the default branch, enabling automatic phase/task completion for fast-forward merges
- `rdm hook install` now installs both `post-merge` and `post-commit` hooks
- `rdm hook uninstall` now removes both hooks
- `default_branch` config key in both repo (`rdm.toml`) and global config — sets the branch name used by the post-commit hook (defaults to `main`)
- `current_branch_at()` public function in `rdm-store-git` for querying the current branch name
- `rdm_init` MCP tool to initialize a plan repo from within an MCP client (e.g. Cursor); accepts an optional `default_project` parameter to create a project during init
- `auto_init` global config option — when `true`, the MCP server automatically initializes the plan repo on first tool call if not already set up
- Improved MCP error messages for uninitialized repos: errors now mention the `rdm_init` tool instead of the CLI `rdm init` command

## [0.2.0] - 2026-03-20

### Added

- `rdm init --remote <url>` to clone an existing shared plan repo instead of creating an empty one; sets `remote.default = "origin"` and validates the cloned repo has `rdm.toml`
- `GitStore::clone_remote(url, root)` static constructor for cloning remote git repositories
- `rdm init --default-project <name>` flag to set `default_project` in repo config and create the project directory
- `rdm init --default-format <fmt>` flag to set `default_format` in global config
- `rdm init` with `--stage` persists `stage = true` to repo config
- `rdm init` now creates parent directories recursively, creates the global config file, and prints a summary with paths, settings, and next steps
- `PlanRepo::init_with_config()` in rdm-core for initializing with a custom `Config`

- `rdm config get <key>` command to view a config value with its source (CLI flag, env var, repo config, global config, or default)
- `rdm config set <key> <value> [--global]` command to set config values in repo or global config with validation
- `rdm config list` command to display all known config keys with resolved values and sources
- `default_format` config key in both repo (`rdm.toml`) and global config — sets the default output format (human, json, table, markdown)
- Format resolution chain: `--format` flag > `RDM_FORMAT` env var > `default_format` in config > `human`
- `InvalidConfigValue` error variant in rdm-core with actionable error messages
- `ConfigSource` and `ResolvedValue<T>` types in rdm-core for tracking where config values come from
- Config validation: invalid `default_format` values are rejected at parse time with clear error messages

### Changed

- `--format` flag no longer defaults to `human` at the clap level; the default is now resolved through the config hierarchy, allowing `default_format` in config files to take effect

### Fixed

- `--root` and `RDM_ROOT` now expand `~` to the home directory and resolve `.`/`..` segments, fixing silent failures when paths are set in config files like `.mise.toml` where the shell doesn't perform tilde expansion

### Added

- `Done: task/<slug>` directive support in post-merge hook — tasks can now be marked done via commit messages, just like phases
- `commit` and `completed` fields on the Task model — automatically set when a task transitions to done
- `--commit` flag on `rdm task update` for manually associating a commit SHA with a task

- XDG-compliant default paths: `rdm` now works out of the box without `RDM_ROOT` by resolving a plan repo root from `~/.config/rdm/config.toml` (global config) or `$XDG_DATA_HOME/rdm` (default data dir)
- `GlobalConfig` struct in rdm-core for parsing global config files with `root`, `default_project`, `stage`, and `remote` fields
- Config merging: CLI flags > env vars > repo config (`rdm.toml`) > global config (`~/.config/rdm/config.toml`) for project, staging, and remote resolution
- `rdm-review` skill for independent post-implementation review with parallel AC compliance and code quality agents
- `skill_review()` generator function in `rdm-core::agent_config` for generating the review skill via `rdm agent-config --skills`
- `rdm-document` Claude Code skill for generating user documentation from completed roadmaps using phase descriptions and commit SHAs
- `rdm agent-config --skills` now generates the `rdm-document` skill alongside the existing three
- `Done:` commit message convention documented in generated agent configs, `rdm-implement` and `rdm-tasks` skills
- `rdm hook install` / `rdm hook uninstall` to manage the post-merge git hook in the plan repo
- `rdm hook post-merge` subcommand: parses `Done: roadmap/phase` directives from the HEAD commit and marks matching phases done with the commit SHA
- `update_phase` is now idempotent for Done→Done transitions: re-marking a done phase with a new commit SHA updates the SHA while preserving the completed date; omitting `--commit` is a safe no-op
- `HeadCommitInfo`, `head_commit_info()`, `git_dir()`, and `default_branch_name()` on `GitStore`
- `rdm_core::hook` module with `DoneDirective` and `parse_done_directives()` for parsing `Done:` directives from commit messages

### Removed

- `.githooks/post-merge` bash script (replaced by `rdm hook` subcommands)
- `--commit <sha>` flag on `rdm phase update` to associate a git commit SHA with phase completion (requires `--status done`)
- `commit` field in phase frontmatter, phase detail display, and JSON output

### Added

- Merge conflict detection during `rdm remote pull` with rdm-aware item context (roadmap, phase, task classification)
- `rdm conflicts` command to list unresolved merge conflicts with item context
- `rdm resolve <file>` command to mark conflicts resolved and auto-complete merge with INDEX.md regeneration
- `rdm discard --force` now aborts an in-progress merge before discarding changes
- `rdm status` shows merge-in-progress state with conflict count
- `MergeConflictResult`, `PullOutcome`, `ResolveResult` structs and `git_list_unmerged`, `git_is_merge_in_progress`, `git_merge_abort`, `git_resolve_conflict` methods on `GitStore`
- `MergeConflict`, `NoMergeInProgress`, `NotConflicted` error variants in rdm-core
- `classify_path` function and `ConflictItem`/`ConflictItemKind` types in new `rdm-core::conflict` module

### Changed

- `rdm remote pull` now attempts a real merge when branches have diverged instead of rejecting with `BranchesDiverged`; non-conflicting concurrent edits merge cleanly

- Top-level `INDEX.md` now shows a lightweight summary table linking to each project's `INDEX.md` instead of inlining all project details

### Added

- `rdm remote push [name]` command to push local commits to a remote (supports `--force`)
- `rdm remote pull [name]` command to fetch and fast-forward merge from a remote, with automatic INDEX.md regeneration
- `PushResult`, `PullResult` structs and `git_push`/`git_pull` methods on `GitStore`
- `PushRejected` and `BranchesDiverged` error variants with actionable messages
- `rdm remote add <name> <url>` command to register a git remote on the plan repo
- `rdm remote remove <name>` command to remove a git remote
- `rdm remote list` command to display all configured remotes with their URLs
- `rdm remote fetch [name]` command to fetch from a git remote (defaults to `remote.default` in `rdm.toml`)
- `rdm status --fetch` flag to fetch from the default remote before showing sync status
- Sync status display on `rdm status` showing ahead/behind commit counts relative to the default remote's tracking branch
- `SyncStatus` struct and `git_fetch`/`git_sync_status` methods on `GitStore` for programmatic fetch and ahead/behind detection
- `RemoteInfo` struct and `git_remote_add/remove/list` methods on `GitStore` for programmatic remote management
- `RemoteConfig` struct in `rdm-core::config` with `[remote]` section support in `rdm.toml`
- `RemoteNotFound` and `DuplicateRemote` error variants in rdm-core
- `format_top_level_index` function in `rdm-core::display` for the new summary-style root index
- Per-project `INDEX.md` files at `projects/<name>/INDEX.md` with relative links, generated alongside the root index
- `format_project_index` function in `rdm-core::display` for standalone per-project index rendering
- `PlanRepo::generate_project_index` method and `project_index_path` path builder in `rdm-core`
- Web UI hides completed roadmaps by default; toggle link (`?show_completed=true`) reveals them
- `rdm tree` command — hierarchical overview of a project's roadmaps, phases, and tasks with statuses (human, JSON, and Markdown formats)
- `rdm-core::tree` module with `TreeNode` types, `build_tree()`, and formatting functions
- Navigation hints in `roadmap show` output — shows how to drill into individual phases
- Prev/next phase navigation in `phase show` output — human and Markdown formats show commands for adjacent phases; JSON includes `prev_phase`/`next_phase` fields
- `rdm describe` command for model introspection — lists entity types or shows fields for a specific entity (project, roadmap, phase, task)
- `rdm-core::describe` module with `Describe` trait, `EntityInfo`/`FieldInfo` types, and formatting functions
- End-to-end agent workflow integration tests validating the full project → roadmap → phase → body discovery path, JSON parity, schema coverage, and programmatic navigation
- Drift tests that compare serde keys against `Describe` field names to catch struct/describe mismatches at compile time
- `project show` command with `--format human/json/markdown` support
- `--format json` support on all read commands: `roadmap list/show`, `phase list/show`, `task list/show`, `project list/show`, `search`, and top-level `list`
- `rdm-core::json` module with serializable JSON output structs (`RoadmapJson`, `PhaseJson`, `TaskJson`, `ProjectJson`, `SearchResultJson`, and summary variants) for stable machine-readable output

### Changed

- `roadmap show --format json` now nests phase summaries (without body) instead of full phase objects; use `phase show --format json` for full phase content
- `search --format json` now outputs via `SearchResultJson` types from the `json` module for a consistent contract
- `--mcp` flag on `rdm agent-config` to generate `.mcp.json` configuration for MCP-aware clients
- `generate_mcp_config` function in `rdm-core::agent_config` for programmatic MCP config generation
- End-to-end MCP workflow integration test covering the full agent lifecycle
- MCP Server section in README with tool table, config generation, and usage instructions
- `--format markdown` option for clean Markdown output on list, show, and search commands
- `--format table` option for pretty terminal tables on list and search commands (powered by `tabled` crate)
- Global `--format` flag on all read commands (defaults to `human`; `text` accepted as alias for backward compatibility)
- 6 mutation MCP tools: `rdm_roadmap_create`, `rdm_phase_create`, `rdm_phase_update`, `rdm_task_create`, `rdm_task_update`, `rdm_task_promote`
- 8 read-only MCP tools: `rdm_project_list`, `rdm_roadmap_list`, `rdm_roadmap_show`, `rdm_phase_list`, `rdm_phase_show`, `rdm_task_list`, `rdm_task_show`, `rdm_search`
- `rdm roadmap archive <slug>` command with `--force` flag to archive completed roadmaps
- `rdm roadmap list --archived` flag to show archived roadmaps
- `rdm roadmap unarchive <slug>` command to restore archived roadmaps to active status
- `RoadmapHasIncompletePhases` error variant in rdm-core for archive validation
- `rdm roadmap split <slug> --phases <stems-or-numbers>... --into <new-slug> --title "Title"` command to extract selected phases from an existing roadmap into a new one, with automatic renumbering and optional `--depends-on` flag
- `PlanRepo::split_roadmap` method in rdm-core for programmatic roadmap splitting
- `InvalidPhaseSelection` error variant in rdm-core for phase selection validation
- Dark mode support for the web UI with toggle button and system-preference detection
- Theme preference persists to `localStorage` across sessions
- Computed overall status badge (done / in-progress / not-started) on roadmap list and detail pages
- Last-changed timestamp on roadmap list and detail pages, derived from file modification times
- `--stage` global flag and `RDM_STAGE` env var for deferred git commits — files are written to disk but the git commit is skipped until explicitly requested
- `rdm status` command to show uncommitted changes in the plan repo
- `rdm commit -m "message"` command for explicit git commits (auto-generates message if `-m` is omitted)
- `rdm discard --force` command to reset working directory to HEAD state
- `stage` option in `rdm.toml` for persistent staging mode
- `staging_mode` on `GitStore` with `git_commit()`, `git_status()`, and `git_discard()` public methods
- `FileChange` enum and `FileStatus` struct in `rdm-store-git` for working directory status reporting
- Uncommitted changes hint on read-only commands (list, show, search) when staging mode is active
- `rdm-store-git` crate — git-backed Store with automatic commits via gitoxide; every `commit()` builds a tree from the working directory and creates a git commit with an auto-generated message
- `git` feature flag on `rdm-cli` (default-on) — enables `GitStore` for automatic git commits on all plan repo mutations
- `Error::Git(String)` variant in rdm-core for git-specific errors
- `rdm-store-fs` crate: filesystem-backed `Store` with in-memory staging — writes buffer in memory, `commit()` flushes to disk using write-to-temp + rename for best-effort atomicity, `discard()` drops the buffer
- `PlanRepo` mutation methods now auto-commit staged changes, so callers don't need explicit `commit()` calls
- `rdm mcp` subcommand: stdio MCP server (scaffold, no tools yet)
- `mcp` feature flag in rdm-cli (default-enabled)

### Changed

- Refactored all inline CSS colors in `base.html` to use CSS custom properties
- Bump `headers-accept` from 0.1 to 0.3
- Bump `mediatype` from 0.19 to 0.21

## [0.1.1] - 2026-03-18

### Added

- Homebrew tap (`edpaget/homebrew-rdm`) with auto-updated formula on release via cargo-dist
- `sign-release.yml` workflow: Sigstore cosign keyless signing of release artifacts with verification instructions appended to GitHub Release notes
- `prepare-release.yml` workflow: one-click version bump, changelog update, commit, tag, and push via `workflow_dispatch`
- cargo-dist configuration for automated binary releases (`rdm` binary for `aarch64-apple-darwin`)
- GitHub Actions release workflow (`.github/workflows/release.yml`) triggered by version tags
- `[profile.dist]` with thin LTO for optimized release builds

### Changed

- Workspace version centralized in root `Cargo.toml`; all crates now use `version.workspace = true`
- Rust version bumped from 1.87 to 1.94
- `repository` field added to workspace package metadata

### Changed

- `FsStore` moved from `rdm-core::store::FsStore` to `rdm_store_fs::FsStore`; import path updated in `rdm-cli` and `rdm-server`

- `rdm phase update` no longer requires `--status`; omitting it preserves the existing status, enabling content-only updates
- `PlanRepo::update_phase` now accepts `Option<PhaseStatus>` instead of `PhaseStatus`
- Server `PATCH /phases/:phase` endpoint accepts optional `status` field in request body

### Added

- `rdm roadmap delete <slug> --force` command to delete a roadmap and all its phases, with automatic cleanup of dependency references from other roadmaps
- `PlanRepo::delete_roadmap` method in rdm-core for programmatic roadmap deletion
- `rdm-implement` and `rdm-tasks` skills now use plan mode (`EnterPlanMode`/`ExitPlanMode`) for a deliberate plan-then-execute workflow with explicit user approval before finalizing
- Generated skills from `rdm agent-config --skills` include the same plan mode workflow
- `rdm roadmap depend <slug> --on <other>` to add a dependency between roadmaps
- `rdm roadmap undepend <slug> --on <other>` to remove a dependency
- `rdm roadmap deps` to display the dependency graph for all roadmaps in a project
- Circular dependency detection rejects cycles with a clear error message
- `CyclicDependency` error variant in rdm-core for dependency cycle detection
- `add_dependency`, `remove_dependency`, and `dependency_graph` methods on `PlanRepo`
- `format_dependency_graph` display function in rdm-core
- `--skills` flag on `rdm agent-config claude` to generate Claude Code skill files (`rdm-roadmap`, `rdm-implement`, `rdm-tasks`) as reusable slash commands
- `rdm-core::agent_config::SkillFile`, `SkillOptions`, and `generate_skills` public API for skill generation
- `rdm agent-config` command to generate AI agent instruction files for Claude Code, Cursor, GitHub Copilot, and AGENTS.md
- Supports `--project` to embed project name in examples and `--out` to write to platform-conventional file paths
- `rdm-core::agent_config` module with `Platform` enum, `AgentConfigOptions`, and `generate_agent_config` function
- "Planning workflow" section in agent config output teaching agents when and how to use rdm commands
- "Status transitions" section documenting valid phase and task status transitions
- `--principles-file` flag on `rdm agent-config` to reference a project principles file in generated instructions
- CLAUDE.md "Searching the plan" subsection documenting `rdm search` usage for AI agents
- `rdm search <query>` CLI command with fuzzy matching across roadmaps, phases, and tasks
- Search flags: `--type` (roadmap|phase|task), `--status`, `--project`, `--limit`, `--format` (text|json)
- Text output displays ranked table with type, title, identifier, and snippet columns
- JSON output (`--format json`) for agent/programmatic consumption
- `format_search_results()` display function in rdm-core for text table formatting
- `Serialize` derives on `SearchResult` and `ItemKind` for JSON serialization
- `search` module in rdm-core: fuzzy search across roadmaps, phases, and tasks by title and body content using `nucleo-matcher`
- `SearchFilter` for narrowing results by item kind, project, or status
- `SearchResult` with kind, identifier, project, title, snippet, and score
- `rdm serve` command with `--port`, `--bind`, and `--root` options
- Graceful shutdown on SIGINT/SIGTERM for `rdm serve` and `rdm-server` binary
- `server` feature flag on `rdm-cli` (enabled by default; disable with `--no-default-features`)
- Integration tests for all server endpoints using reqwest against real TCP server
- Accessibility smoke tests verifying WCAG landmark structure, heading hierarchy, and ARIA attributes
- POST endpoints for creating projects, roadmaps, phases, and tasks (201 Created + Location header)
- PATCH endpoints for updating phase status and task fields (status, priority, tags, body)
- POST endpoint for promoting tasks to roadmaps (`/projects/{project}/tasks/{task}/promote`)
- Automatic index regeneration after all write operations
- Content negotiation for write responses: HAL+JSON returns resource, HTML returns 303 See Other redirect
- 422 Unprocessable Content for invalid request bodies (RFC 9457 Problem Details format)
- `hal_created_response` and `see_other_response` helpers in `rdm-server::extract`
- `validation_error` and `json_rejection_response` helpers in `rdm-server::error`
- HTML rendering for all endpoints with content negotiation: browsers get accessible HTML pages, API clients get HAL+JSON
- WCAG 2.1 AA accessibility: skip-to-content link, breadcrumb navigation with `aria-label` and `aria-current`, proper `<th scope>`, status conveyed by text (not color alone), focus outlines, sufficient color contrast
- Markdown-to-HTML rendering for phase and task body content using pulldown-cmark (raw HTML stripped)
- Format-aware error pages: HTML requests get styled error pages, HAL+JSON requests get RFC 9457 Problem Details
- Askama compile-time templates for all pages: index, roadmaps, roadmap detail, phase detail, task list, task detail, error
- Read-only HAL+JSON endpoints: `GET /` (root with project links), `GET /projects`, `GET /projects/:project/roadmaps`, `GET /projects/:project/roadmaps/:roadmap` (with embedded phases), `GET /projects/:project/roadmaps/:roadmap/phases/:phase` (with prev/next sibling links), `GET /projects/:project/tasks` (with `?status=`, `?priority=`, `?tag=` filters), `GET /projects/:project/tasks/:task`
- `load_project()` method on `PlanRepo` for loading project documents
- HAL+JSON response helpers (`require_hal_json`, `hal_response`) in `rdm-server::extract`
- Server foundation: `rdm-server` binary with axum, health check endpoint (`GET /healthz`), and shared `AppState`
- HAL (Hypertext Application Language) response types in `rdm-core`: `HalLink` and `HalResource<T>` with builder API
- RFC 9457 Problem Details type in `rdm-core` with mappings from all `rdm-core::Error` variants
- Content negotiation extractor parsing `Accept` header for `application/hal+json` and `text/html` (defaults to HTML)
- `AppError` wrapper in `rdm-server` converting core errors to Problem Details HTTP responses
- `phase remove` command to delete a phase from a roadmap (accepts stem or number)
- Interactive `$EDITOR` fallback when no `--body` or stdin is provided (checks `$VISUAL`, then `$EDITOR`, then `vi`)
- `--no-edit` flag on all `create` and `update` commands to suppress interactive editor
- `--body` flag on all `create` and `update` commands for roadmaps, phases, and tasks
- Piped stdin support: body content can be provided via stdin (e.g., `cat notes.md | rdm task create ...`)
- `rdm roadmap show` now displays document body content after the phase table
- `--no-body` flag on `roadmap show`, `phase show`, and `task show` to suppress body output
- `RDM_PROJECT` environment variable for session-level default project (resolution order: `--project` flag > `RDM_PROJECT` env var > `default_project` in `rdm.toml`)
- Body parameter on core create and update functions for roadmaps, phases, and tasks
- `rdm roadmap list --project P` command to list all roadmaps with phase progress
- `rdm index` command to generate `INDEX.md` from current repo state
- `PlanRepo::generate_index` in rdm-core for full index generation (projects, roadmaps with progress, tasks sorted by priority)
- `format_index` display function with `ProjectIndex` and `RoadmapIndexEntry` structs
- `--no-index` global flag to suppress automatic INDEX.md regeneration after mutations
- Auto-regenerate INDEX.md after all mutation commands (project/roadmap/phase/task create, phase/task update, promote)
- `Ord`/`PartialOrd` derive on `Priority` enum (Low < Medium < High < Critical)
- Integration tests for index generation, idempotency, sorting, dependency graphs, auto-index, and `--no-index`

### Removed

- `require_hal_json()` guard — all endpoints now support both HTML and HAL+JSON via content negotiation

### Changed

- `load_roadmap` and `load_task` now return `RoadmapNotFound`/`TaskNotFound` (404) instead of `Io` error (500) when the resource file does not exist
- `task list --status` now uses `TaskStatusFilter` enum for proper clap validation instead of raw string
- `promote` preserves task metadata (priority, created date, tags) in the roadmap body
- `list_tasks` returns `ProjectNotFound` for nonexistent projects instead of an empty list

### Added

- `TaskStatusFilter` type with `Display`/`FromStr` for type-safe status filtering (accepts `all` or any `TaskStatus`)
- `rdm task create`, `rdm task show`, `rdm task update`, and `rdm task list` CLI commands
- `rdm promote` command to convert a task into a roadmap with an initial phase
- `task list` defaults to showing `open` + `in-progress` tasks; `--status all` shows everything
- `task list` supports `--status`, `--priority`, and `--tag` filters
- `PlanRepo::create_task`, `list_tasks`, `update_task`, `promote_task` in rdm-core
- `Display` and `FromStr` impls for `TaskStatus` and `Priority` (enables CLI arg parsing via clap)
- `format_task_detail` and `format_task_list` display functions in rdm-core
- `TaskNotFound` error variant in rdm-core
- Integration tests for all task CLI commands and promote
- `rdm phase list` command to show phases in a roadmap with number, title, status, and stem
- Phase commands (`phase show`, `phase update`) accept phase number as alternative to stem
- `rdm project create` and `rdm project list` CLI commands
- `rdm roadmap create` and `rdm roadmap show` CLI commands
- `rdm phase create`, `rdm phase show`, and `rdm phase update` CLI commands
- `rdm list` command with `--project` and `--all` flags for roadmap progress summaries
- Project resolution: `--project` flag > `default_project` in `rdm.toml` > actionable error
- `PlanRepo::create_project`, `list_projects` for project management
- `PlanRepo::create_roadmap`, `list_roadmaps` for roadmap management
- `PlanRepo::create_phase`, `list_phases`, `update_phase` for phase management
- Auto-numbering for phases (next available number) with explicit `--number` override
- Auto-set `completed` date when phase status transitions to `Done`; auto-clear on non-`Done`
- `Display` and `FromStr` impls for `PhaseStatus` (enables `--status` CLI arg via clap)
- `rdm-core::display` module with `format_roadmap_summary`, `format_phase_detail`, `format_roadmap_list`
- Error variants: `RoadmapNotFound`, `PhaseNotFound`, `DuplicateSlug`, `ProjectNotSpecified`
- Integration tests for all new CLI commands (`cli_project`, `cli_roadmap`, `cli_phase`, `cli_list`, `cli_project_resolution`)
- Cargo workspace with `rdm-core`, `rdm-cli`, and `rdm-server` crates
- Data model types: `PhaseStatus`, `TaskStatus`, `Priority`, `Phase`, `Task`, `Roadmap`, `Project`
- Markdown frontmatter parsing and rendering (`split_frontmatter`, `join_frontmatter`)
- Generic `Document<T>` wrapper with `parse()` and `render()` methods
- Plan repo configuration (`Config` struct, `rdm.toml` parsing)
- `PlanRepo` with path builders, load/write operations for roadmaps, phases, and tasks
- `PlanRepo::load_config` to read and parse `rdm.toml` from an opened repo
- `PlanRepo::init` to initialize a new plan repo with `rdm.toml`, `projects/`, and `INDEX.md`
- `rdm init` CLI command with `--root` flag and `RDM_ROOT` env var support
- Hand-written error types in `rdm-core` with `Display`/`Error` impls
- `rdm-server` stub binary

### Changed

- `create_project` now returns `Document<Project>` for consistency with other create methods
- `Config::to_toml` now returns `crate::error::Result` instead of leaking `toml::ser::Error`

### Fixed

- `Config::from_toml` now returns `crate::error::Error` instead of leaking `toml::de::Error`
- `rdm list` now propagates phase-loading errors instead of silently swallowing them
- CLI integration tests use `tempfile::TempDir` instead of `.tmp/` in project root
