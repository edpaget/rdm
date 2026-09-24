# Test and tooling migration inventory

Inventory for the `rust-test-suite-consolidation` roadmap, taken at phase 1
(2026-09-23, base `b4a3782`). It catalogues every verification-script section
and every JavaScript file by what it actually exercises, who owns it, what it
depends on, where it goes, whether CI requires it, and — only where the covered
product behaviour has already been retired — why it can be dropped. Later
phases update the Destination and CI columns as they land.

Destinations: phase 2 — Rust-driven workflow execution tests and review
coverage; phase 3 — remaining Claude workflow coverage; phase 4 — measurement,
corpus and Codex coexistence tooling; phase 5 — distribution and CLI shell
verification; phase 6 — session, commit and race harnesses; phase 7 — nextest
as the single runner and the final coverage audit.

The "Retirement rationale" column also records *candidates* the survey
noticed (duplicates, prose/source greps that the operator's no-grep rule would
retire rather than port). A candidate is not a decision: each owning phase
decides, and coverage is retired only when tied to landed product retirement
or to an operator rule, never because a test uses a mocked host.

## 1. Verification scripts

CI's `Shell harnesses` step (`.github/workflows/ci.yml`) runs `cargo build`, then every `scripts/verify-*.sh` in a loop, so every verify-* row is **required** unless the row says otherwise. `observe-*.sh` scripts are not in that loop and are **opt-in**.

In the Dependencies column, "rdm bin" means a prebuilt `target/debug/rdm` and "common" is defined per script. "Mutant build" means a `cargo build` of a scratch tree taken from `git archive HEAD` with one source line changed by `sed`.

### verify-workflow-review.sh (deleted in phase 2)
**Phase 2 (done).** The script is deleted. Each section below names its Rust equivalent in `rdm-cli/tests/workflow_review/` (module `::` test) or records its retirement. All of those tests run under `cargo nextest run`, so CI requires them. "R-grep" is the rationale for assertions retired rather than ported: *not behavioral coverage: a string-presence check (operator no-grep rule)*. Under the operator's 2026-09-23 no-grep ruling these are outside the roadmap's product-retirement rule and need no confirmation. Where a retired string check stood in for a still-shipping behaviour, that behaviour is ported as an executed assertion, marked "rewritten".

| Script § | Actual behaviour exercised | Owning code | Dest. (named Rust tests) | CI | Retired sub-assertions |
|---|---|---|---|---|---|
| 1 | `gen-workflow-review.sh --check` on the real tree | `scripts/gen-workflow-review.sh`, `lib/gen-workflow-block.sh`, `lib/review.mjs`, both consumers and their copies | `generators::review_block_in_sync_in_every_consumer` | nextest | — |
| 1b | Scratch tree: planted block drift fails `--check`; regeneration heals it | same | `generators::review_block_drift_detected_then_healed`. The drift is a line inserted after the `review-refute-fix:begin` marker, located structurally. The heal is checked byte-exact | nextest | — |
| 1c | `gen-skill-review.sh --check` in code and plan modes; planted `//\|` drift detected, then healed | `scripts/gen-skill-review.sh`, review.mjs `//\|` regions, shipped skill templates | `generators::skill_projection_in_sync_code`, `…_plan`, `generators::skill_projection_drift_detected_then_healed` (mutates the first shared `//\| ` line; both modes) | nextest | — |
| 1e | One generator; `DIMENSIONS` keys are exactly code, plan | review.mjs `DIMENSIONS` | Rewritten: `pipeline::review_modes_are_code_and_plan` runs both modes and requires every other mode name to throw | nextest | "no `scripts/gen-plan-review*` exists": a file-absence listing. R-grep |
| 1g | `--target local` check in both modes; unknown target rejected; consumer-side edit detected and healed; local override consumed locally, never leaked into the shipped render | gen-skill-review.sh, `.claude/skills/rdm-{review,plan-review}` | `generators::local_skill_projection_in_sync_{code,plan}`, `generators::unknown_target_rejected` (exit status plus stderr names the target), `generators::local_consumer_edit_detected_then_healed_{code,plan}`, `generators::local_override_does_not_leak_into_shipped_render` | nextest | — |
| 2d | Engine filename set; frozen lib filename set; shipped template byte identity | `.claude/workflows/`, `rdm-core/src/templates/workflows/` | Retired as a duplicate: shipped template byte identity is `rdm-core/src/agent_config.rs::generate_workflows_are_byte_identical_to_source` (every emitted workflow against `.claude/workflows/`), plus the `gen-workflow-review.sh --check` case `generators::review_block_in_sync_in_every_consumer` for the review consumers' embedded copies | nextest (existing) | Engine and lib filename-set listings. R-grep. The engine set that `rdm agent-config` emits is phase-5 distribution behaviour |
| 3 | Pipeline battery, both modes | review.mjs `buildReviewPipeline`, `survives`, `rankFindings`, `findPrompt`, `classifyOutcome`, schemas | `pipeline::code_mode_drops_refuted_and_low_confidence_findings` (+ `…_mutant_floor_comparison`), `pipeline::survival_rule_boundaries`, `pipeline::rank_total_order`, `pipeline::plan_mode_battery`, `pipeline::output_deterministic_{code,plan}`, `pipeline::thrown_finder_drops_only_its_dimension`, `pipeline::crashed_refuter_keeps_finding`, `pipeline::unknown_mode_throws`, `pipeline::finder_schema_accepts_category` (asserted on the recorded `opts.schema`), `pipeline::architectural_blocker_ranked_ahead_of_nit`, `pipeline::restraint_finding_survives`, `pipeline::all_null_finders_reject_{with,without}_model`, `outcome::classify_outcome_and_status_mapping`, `outcome::ac_table_gap_rules`, `outcome::ac_table_channel_forces_rework`, `outcome::gate_policy_table`. Rewritten: `pipeline::ac_finder_dispatched_with_ac_review_schema` replaces the `'AC_REVIEW'`-in-prompt check with the recorded `opts.schema` of the `ac` call, and checks that an `ac` array becomes `acTable` | nextest | Each R-grep: (i) the AC2 byte-pinned `findPrompt` baseline (already deleted before phase 2); (ii) AC2 "no `PLAN_SEVERITY_CALIBRATION` in any code prompt"; (iii) AC2c injection-hygiene phrase presence and shared-const inclusion; (iv) AC2e security-focus prose (attacker framing, category and severity words, no language APIs, no HIGH/MEDIUM); (v) AC2b/AC5 forbidden project tokens and the principles-document pointer; (vi) AC1 plan calibration keyphrases; (vii) the coherence stopping-rule phrase pair and the empty/ambiguous-plan phrase. `GATE_POLICY.code === STATUS_MAPPING` identity became deep equality, because identity does not cross the JSON boundary |
| 3c | Finder retry, participation record, absent vs clean AC table, model-independent guards, no gating, projection helpers | review.mjs `buildReviewCoverage`, `coverageSummaryClause`, `acTableHasGap`, `resolveReviewers` | `coverage::finder_retried_exactly_once`, `coverage::empty_payload_not_retried`, `coverage::dead_dimensions_recorded_in_order` (with the summary clause), `coverage::absent_vs_clean_ac_table`, `coverage::no_models_dead_finder_recorded_not_fatal`, `coverage::dead_dimension_does_not_gate` (with the frozen legacy `classifyOutcome` inputs), `coverage::projection_helpers` | nextest | — |
| 3c-mut | Planted mutants turn 3c red | same | `coverage::mutant_retry_deleted`, `coverage::mutant_complete_hardcoded`, `coverage::mutant_model_conditional_guard` | nextest | — |
| 4 | Calibration phrase presence and its strip mutant | review.mjs | Retired | — | R-grep. The fake host has no model, so the phrase's effect cannot be observed in a component test |
| 4a, 4a-guard | Forbidden and required tokens in dimension prose, refuter guard and ac-contract spans; M1–M3 and planted-token mutants | review.mjs prose | Retired | — | R-grep. No executable behaviour depends on those tokens |
| 5 | `filterPlanReviewTag`, `classifyPlanOutcome`, independent per-unit gates, implementation-plan skips the gate | review.mjs, `rdm-wf-plan-review.js` | `plan::filter_plan_review_tag_preserves_siblings`, `plan::classify_plan_outcome`, `plan::per_unit_independent_gates`, `plan::implementation_plan_skips_gate` | nextest | Export-presence checks are subsumed: the tests call the functions |
| 5b-drift | `plan-review-driver` block byte identity | `lib/plan-review.mjs`, `rdm-wf-plan-review.js` | Retired as a duplicate of `rdm-core/tests/workflow_plan_review_driver.rs::plan_review_driver_block_is_byte_identical` | nextest (existing) | — |
| 5f | `formatRoundNote` → `parseRoundNotes` round trip; narrowed-regex mutant | `lib/plan-review.mjs` | `plan::round_note_severity_round_trip`, `plan::mutant_round_note_regex_narrowed` | nextest | — |
| 8 | Non-gating skip: no refuter for a suggestion, marked pass-through, no budget consumed; a crash is not a skip | review.mjs `NON_GATING_SEVERITIES`/`needsRefutation` | `budget::needs_refutation_fail_safe` (also covers the `NON_GATING_SEVERITIES` value behaviourally), `budget::non_gating_suggestion_passes_through_{code,plan}`, `budget::crashed_refuter_keeps_gating_finding` | nextest | `UNREFUTED_DISPOSITION` prose checks ("reported, not verified", "not major", `FILE`, "evaporate"). R-grep |
| 8c | Mutants: skip widened to `concern`; marker renamed | same | `budget::mutant_non_gating_widened_to_concern`, `budget::mutant_unrefuted_marker_renamed` | nextest | (c) was already deleted with its subject |
| 9 | Refutation budget | review.mjs `resolveRefutationBudget`, `rankBudgetCandidates` | `budget::default_budget_and_resolution` (9a), `budget::rank_budget_candidates_order` (9b), `budget::refutation_budget_under_at_over_bound` (9c-run/9d/9e, including log text), `budget::budget_states_distinct` (9f), `budget::budget_ranking_deterministic_and_monotone` (9g plus the 9j property test), `budget::budget_zero_grades_nothing`, `budget::suggestions_never_consume_budget`, `budget::crashed_finder_contributes_no_candidates`, `budget::per_run_override_reaches_pipeline`, `budget::invalid_override_throws_before_any_agent` (9h), `budget::floor_not_bypassed_over_budget` (9i), `budget::over_budget_blockers_still_gate`, `budget::ac_table_never_budgeted`, `budget::budget_skipped_blocker_still_gates` | nextest | The test-side reference `refCodeGate` driver is not ported. Its assertion reduces to review.mjs's own `hasBlocking` gating on a budget-skipped blocker and not on a non-gating survivor, which `budget_skipped_blocker_still_gates` executes directly. The 9j property test now calls the real `survives` once per finding and verdict shape, and caches `classifyOutcome` per distinct input |
| 9c | Seven mutants | same | `budget::mutant_i_source_order_tiebreak`, `…_ii_confidence_ascending`, `…_iii_budget_reason_dropped`, `…_iv_floor_bypassed`, `…_v_refuter_error_marker_dropped`, `…_vi_budget_cut_off_by_one`, `…_vii_default_budget` | nextest | Mutant (ii) now flips only `rankBudgetCandidates`; the legacy sed also flipped `rankFindings`. `replace_once` refuses an ambiguous anchor |
| 13 | Refuter laundering guard | review.mjs `refutePrompt` | (c), a refuted finding being dropped, is covered by `pipeline::code_mode_drops_refuted_and_low_confidence_findings` | nextest | (a) guard phrase in `refutePrompt` output; (b) a fake refuter whose verdict depends on searching the prompt for that phrase; (d) the strip-the-phrase mutant. R-grep. The guard's real effect is on a model |
| 14 | Deferred-AC contract | review.mjs, `classifyOutcome` | `outcome::blocking_ac_finding_forces_rework_with_all_pass_table` (with its negative control) | nextest | Focus-phrase and `findPrompt`-phrase presence, and both strip-prose mutants. R-grep |
| 15 | Pure persist writer | review.mjs persist helpers; `lib/plan-review.mjs` | `persist::verdict_map_and_unknown_outcome_throws`, `persist::comment_header_round_trip`, `persist::parse_comment_header_rejects_non_headers`, `persist::opaque_and_bare_ref_guards`, `persist::refs_emitted_shell_quoted` (input-dependent, plus determinism), `persist::resolve_persist_arg`, `persist::parse_plan_args_never_parses_persist`, `persist::persist_target_for`, `persist::prior_round_and_findings_from_reviews`, `persist::quote_ok_preserves_or_clears_quote`, `persist::strip_quote_is_pure` (output contract), `persist::quote_changes_refute_prompt`. Rewritten: `persist::ladder_commits_only_its_own_changeset` replaces "never `commit --all`, never `discard`"; `persist::ambiguous_quote_not_silently_anchored` replaces "no pre-emitted `--occurrence`". Command order, one comment per survivor, literal quote ride-through, the mapped verdict and the session-scoped commit are all proven by executing the ladder (15b rows); "no `--doc`" is proven by the target-kind assertions there | nextest | The literal `refutePrompt` baseline; "`ac` prompt never mentions quote"; "prompt names `quote`"; "quote-less refuter prompt carries no `quote_ok` clause"; "blank quote earns no clause"; "no heredoc"; "the ladder contains `RDM_PERSIST_QUOTE='`"; schema-shape checks for `quote`/`quote_ok` optionality. All R-grep. `stripQuote` not mutating its argument is not observable through a JSON value boundary; its output contract is ported |
| 15b | Real-binary round trip | review.mjs persist; `rdm review` CLI | `persist::ladder_lands_anchored_whole_doc_and_cleared_comments`, `persist::verdict_mapping_via_real_binary` (reviewed, rework, escalated, and the `[plan]`/`[code]` prefixes), `persist::phase_stem_and_numeric_refs_resolve`, `persist::bare_ref_rejected` (now through the emitted ladder). All run against `CARGO_BIN_EXE_rdm` in a per-test plan repo | nextest | The direct CLI stale-quote and `--occurrence 1` probes are covered by the named equivalents `rdm-cli/tests/cli_review.rs::review_comment_quote_not_found_names_created_commit` and `::review_comment_ambiguous_quote_lists_occurrences_then_occurrence_selects` |
| 15d-path | `pathFromLocation` guards | review.mjs | `persist::path_from_location_guards` | nextest | — |
| 15g | Crafted target is shell-injection safe; reverting the quoting fires a marker | review.mjs `shellQuote(target)` | `persist::ladder_is_shell_injection_safe`, `persist::mutant_target_unquoted` (run under `sh` against the real binary) | nextest | — |
| 15h / 15h-mut | Scratch-file hygiene | review.mjs persist ladder | Rewritten as behaviour, running the emitted ladder under `sh` against the real binary with a per-test `TMPDIR`: `persist::ladder_scratch_file_not_at_predictable_path` (a symlink planted at `$TMPDIR/rdm-persist-start.$$.json` by the ladder's own shell; the victim is untouched), `persist::concurrent_ladders_do_not_collide`, `persist::ladder_leaves_no_scratch_file`; mutants `persist::mutant_scratch_path_predictable` (victim clobbered) and `persist::mutant_scratch_rm_removed` (file left) | nextest | Regexes over the ladder text (mktemp template, no `$$`, no `/tmp` redirect, rm in the same entry). R-grep, replaced by the executed checks. The change-target ladder shape is not run here; it needs a real source checkout and is exercised by `scripts/lib/review-driver.test.mjs` (phase 3) |
| (named gaps) | §§2a, 2b-fid, 2c, 2c(v), 5b-exec | — | Already deleted before phase 2 | — | — |

**Totals over the 27 live sections.** 16 ported in full: 1, 1b, 1c, 1g, 3c, 3c-mut, 5, 5f, 8c, 9, 9c, 15b, 15d-path, 15g, 15h, 15h-mut. 5 split, with the behaviour ported and the string checks retired: 1e, 3, 8, 14, 15. 4 retired in full as string checks: 4, 4a, 4a-guard, 13. 2 retired as duplicates with a named equivalent: 2d (its filename-set listings retired as string checks), 5b-drift.

### verify-workflow-review-outcome.sh (deleted in phase 2)
| Script § | Actual behaviour exercised | Dest. (named Rust tests) | CI | Retired sub-assertions |
|---|---|---|---|---|
| `cargo build -p rdm-cli` | Build step | none needed: nextest builds `CARGO_BIN_EXE_rdm` | — | — |
| `verify-review-source.mjs` (deleted) | `classifyOutcome` completeness; engine driver legacy shapes; mixed identity; source and template checks | Ported: `outcome::classify_outcome_completeness_cases` (all nine inputs), `engine::legacy_survivors_only_shape_{code,plan}` and `engine::mixed_task_phase_identity_rejected` (both execute the driver). New: `engine::emitted_template_helpers_run_without_driver` (extraction from `rdm-core/src/templates/workflows/rdm-wf-review-refute-fix.js`). Rewritten: `engine::driver_deterministic_and_emits_no_done_trailer` replaces the `Done:\|Date.now\|Math.random` source-absence check by running the source-bound driver twice and checking its returned commands | nextest | The driver-region counts of `classifyOutcome(` and `buildReviewPipeline('code')`. R-grep. Engine/template byte identity is covered by `rdm-core/src/agent_config.rs::generate_workflows_are_byte_identical_to_source` |
| Five `gen-*` `--check` calls | Generator drift | Duplicates of `generators::review_block_in_sync_in_every_consumer`, `generators::skill_projection_in_sync_{code,plan}` and `generators::local_skill_projection_in_sync_{code,plan}` | nextest | — |

### verify-workflow-backlog.sh
| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| 1 / 1b | Heredoc `behavior.mjs`: `parseBacklogArgs`, category order, schema, `parseBacklogReport`, `selectCategories`, analyzer prompt rules, `normalizeAnalysis`, `consolidateBatch`. Driven `buildBacklogPipeline`: an empty report makes 0 analyzer calls, a full report gives 4 subsections, a fetch error propagates, a crashed analyzer degrades, output is deterministic | `.claude/workflows/lib/backlog.mjs` | node, rdm bin (checked at start) | 3 | required | — |
| 2 | Heredoc `zero-mutation.mjs`: real `rdm backlog report` JSON from a seeded repo drives the pipeline to 4 subsections; HEAD, status and file checksums unchanged | `lib/backlog.mjs`, `rdm-core/src/ops/backlog.rs` | node, rdm bin, git, shasum | 3 | required | — |
| 3 / 3b | `backlog-groom` region is byte-identical between the lib and `rdm-wf-backlog.js`; planted-sed self-test | `lib/backlog.mjs`, `rdm-wf-backlog.js` | awk, diff | 3 | required | Copy-drift guard (the no-guards rule may retire it) |
| 5 / 5b | `node --check --input-type=module` parses the engine; a planted duplicate-`meta` self-test | `rdm-wf-backlog.js` | node | 3 | required | — |

### verify-workflow-document.sh
| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| 2 / 2b | `document-core` region is byte-identical between the lib and the engine; planted-drift self-test | `lib/document.mjs`, `rdm-wf-document.js` | awk, diff | 3 | required | Copy-drift guard |
| 3 | Heredoc `test.mjs`: `parseDocumentArgs` (`rdmBin`/`project`), `defaultOutPath`/`resolveOutPath`, `computeIncompletePhases`, `buildGitRangeCommands` with and without a SHA | `lib/document.mjs` | node | 3 | required | — |
| 4 (4a–4c) | Heredoc `test-real.mjs`: real `roadmap`/`phase show --format json` output from three seeded roadmaps (done with SHAs, incomplete, done without a commit) plus real commits, fed through the same functions | `lib/document.mjs`; rdm-core show JSON | node, git, rdm bin | 3 | required | — |

### verify-workflow-estimate.sh
| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| 1 | Heredoc `behavior.mjs`: `parseEstimateArgs`, `selectUnestimated`, list-command text, estimator prompt requires a justification, writeback is `--difficulty` only (no `--model`, no body), no land/merge directive, summary text, determinism | `.claude/workflows/lib/estimate.mjs` | node, rdm bin (checked at start) | 3 | required | (§1b driven pipeline already deleted, phase 34) |
| 2 / 2b | `gen-workflow-estimate.sh --check`; a planted sed edit of the engine goes red, then is restored | `scripts/gen-workflow-estimate.sh`, `rdm-wf-estimate.js` | sh | 3 | required | Drift gate |
| 3 | `node --check` parses the engine as a module | `rdm-wf-estimate.js` | node | 3 | required | — |
| 5 | Heredoc `real.mjs`: real `phase list --format json` output (3 phases, one pre-estimated) → `selectUnestimated` picks 1 and 3 | `lib/estimate.mjs`; rdm-core phase-list JSON | node, rdm bin, git | 3 | required | — |
| 9 | Banner only | — | — | — | — | §9a grep and §9d already retired |
| 9b | Heredoc `paramz.mjs`: the whole engine is wrapped and driven with a capturing fake agent; every emitted `rdm` call uses the injected `rdmBin`; `--project` is threaded per the allow-list | `rdm-wf-estimate.js`, `lib/estimate.mjs` | node | 3 | required | — |
| 9c | Heredoc `rdmbin.mjs`: `resolveRdmBin` defaults and fails closed on a bad type; `projectFlag`/`parseProjectArg` on both lib and engine | same | node | 3 | required | — |

### verify-skill-autopilot.sh
| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| 1 | (header only) | — | — | — | — | Retired in adfd31a (static-grep); stale header remains |
| 2 (2a–2c) | Hermetic plan repo: `phase update --status reviewed` reads back; `--status blocked --reason …` reads back `blocked_reason` | rdm-cli/rdm-core `phase update`/`show` | rdm bin, git | 3 (arguably 5; no JS) | required | — |
| 3 | Runs `bash verify-workflow-estimate.sh` as a sibling gate | as that script | bash, node, rdm bin | 3 | required | Pure duplication; drop |
| 4 | `rdm hook done-line` is amended onto a trailer-less branch tip; after an ff-merge, `rdm hook post-commit` marks the phase done with the landed SHA; `done-line` rejects both-or-neither of `--phase`/`--task` | `rdm_core::hook` (`format_done_directive`, post-commit), rdm-cli `hook` | rdm bin, git | 3 (arguably 5) | required | — |

### verify-token-report.sh
Dependencies for every row: node, plus sed for the mutants. No rdm bin, claude or network.

| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| 1 | `node --check` on the lib and CLI; no `package.json`/`node_modules` under `scripts/` | `scripts/lib/token-report.mjs`, `scripts/measure-lane-tokens.mjs` | node | 4 | required | The stdlib-only guard is moot once the tool is in Rust |
| 2 | CLI over `tests/fixtures/token-sidecar`: per-class and per-group totals vs `expected-totals.json`; `--worktrees-` slug; discrepancy line; first-request floor | same plus fixture | node | 4 | required | — |
| 3 | Lib direct: `requestId` dedupe is last-write-wins; unreadable/empty transcripts give distinct warnings; `locateSessionDirs` handles worktree slugs; `firstRequestTokens` is null for cached/sidecarOnly | `token-report.mjs` | node | 4 | required | — |
| 4 | CLI rejects a missing flag value, a flag taken as a value, and `--out` into a missing directory | `measure-lane-tokens.mjs` | node | 4 | required | — |
| 5 | Four planted mutants each fail §2 | `token-report.mjs` | node, sed | 4 | required | — |
| 6 | `measure-refuter-severity.mjs` over `tests/fixtures/token-refuter-severity`: severity counts, verdicts, token classes, fanout distributions; `--check`; corpus-free `--audit` of `docs/token-baseline.json` (`nonGatingRefutationSkip`, `refuterFanout`); `--until`; six mutants | `scripts/measure-refuter-severity.mjs`, `token-report.mjs`, imports `review.mjs` | node, sed | 4 | required | `refuterFanout` backs the unadopted batching shape (trim candidate) |
| 7 | Determining-finding rank over `tests/fixtures/token-determining-rank`: deep block compare, `unitIdent` parity, closed reason vocabulary, the four `deriveCapVerdict` branches, `--check`/`--audit`, a prose-twin check of `docs/token-baseline.md`; mutants (a)–(l) | `measure-refuter-severity.mjs`, review.mjs `rankFindings`, `docs/token-baseline.{json,md}` | node, git, sed | 4 | required | The prose-twin `.md` string check asserts on doc text; drop it |

### verify-refuter-agreement.sh
Dependencies for every row: node, plus sed for the mutants. No real claude or network.

| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| 1 | `node --check` on the three scripts; `--help` documents every flag and carries the cost warning | `scripts/lib/refuter-agreement.mjs`, `scripts/mine-refuter-corpus.mjs`, `scripts/run-refuter-agreement.mjs` | node | 4 | required | Help-text grep is a string assert |
| 2 | Corpus loads cleanly; size/divergence/mined/authoritative floors; enum checks; adjudication commit on each item | `refuter-agreement.mjs`, `tests/fixtures/refuter-agreement/corpus.jsonl` | node | 4 | required | — |
| 2c (+equivalence, guard) | Batch-group power under the unit-scoped key; the unit-identity check matches the canonical rule; an underpowered arm throws or forces NO MEASUREMENT | `refuter-agreement.mjs`, `run-refuter-agreement.mjs`, `measure-refuter-severity.mjs` | node | 4 | required | Batching recorded as no-measurement (`docs/refuter-batching.md`); retirement candidate |
| 3 | Every item regenerates through the REAL `refutePrompt` with a matching `promptSha256`/`promptDrift` | `refuter-agreement.mjs`, review.mjs `refutePrompt` | node | 4 | required | — (goes red on any `refutePrompt` edit) |
| 4 / 4b | Miner over the `mine-sidecars` fixture: 6 degradation paths, no silent drops, slug filter; CLI `--severity`/`--until`/`--limit`/`--out`/bad-argument messages | `mine-refuter-corpus.mjs` | node | 4 | required | — |
| 5 | Scorer over `trials-sample.json`: separate FN/FP denominators, ungraded bucket, flip rate, per-class and authoritative splits, token/tool columns | `refuter-agreement.mjs` | node | 4 | required | — |
| 5c | Batched scoring: batched prompt, id expansion, arm buckets, dispatch counting | `refuter-agreement.mjs`, review.mjs | node | 4 | required | Batching retirement candidate |
| 6 | No blended accuracy field anywhere in JSON or text output | both | node | 4 | required | — |
| 7 | `--dry-run` dispatches nothing; `--dispatch-stub` drives the full path | `run-refuter-agreement.mjs` | node | 4 | required | — |
| 7b | Pure parsers; `claudeDispatch` against fake `claude` stubs on PATH (ok, fail, garbage, empty, ENOENT) | `run-refuter-agreement.mjs` | node, fake claude stubs | 4 | required | — |
| 7c | `parseClaudeBatchResult`: unknown ids, non-boolean verdicts, missing array | same | node | 4 | required | Batching retirement candidate |
| 8 / 8b | `--audit` arithmetic of the committed tiering figures; `--audit-section refuterBatching` | same, `docs/token-baseline.json` | node | 4 | required | 8b: batching candidate |
| 9 (9a–9t) | About 20 planted mutants prove the sections above can fail | all three | node, sed | 4 | required | — |
| 10, 11 | (removed) | — | — | — | — | Already deleted (10: CHANGELOG assert, a8b4284; 11: AC9 XOR, phase 34). The header still lists them |

### verify-agent-config-distribution.sh
| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| 0 / 8 | Repo `git status --porcelain` unchanged across the run | harness | git, rdm bin | 5 | required | — |
| 1 | `agent-config claude --skills --out` emits a tree | `rdm-core/src/agent_config.rs` (`generate_skills`/`_workflows`/`_agents`), rdm-cli `--out` | rdm bin | 5 | required | — |
| 2 / 2b | 11 skills, 1 workflow and 1 agent exist with valid frontmatter; no unsubstituted `{proj_flag}`-style placeholders | agent_config.rs `render_skill`, `templates/skill-*.md` | rdm bin | 5 | required | — |
| 3 / 3a | Emitted workflow and agent are byte-identical to `.claude/workflows`/`.claude/agents` | agent_config.rs, templates/workflows | rdm bin | 5 | required | — |
| 3b | Re-emit is idempotent and leaves an unrelated user file alone | agent_config.rs `SUPERSEDED_WORKFLOWS` cleanup | rdm bin, shasum | 5 | required | — |
| 3c (i–iii) | Every emitted `agentType:` resolves to an emitted agent; count floor; 3 planted self-tests | agent_config.rs `generate_agents` | rdm bin | 5 | required | No workflow threads `agentType` any more; retirement candidate |
| 4 | Every `.claude/workflows/<name>.js` reference and "invoke the X Workflow" instruction in a skill resolves (floors ≥2) | `templates/skill-*.md` | rdm bin | 5 | required | Prose-grep (no-grep rule) |
| 5a–5f | Planted self-tests: corrupted byte, typo'd shim, planted placeholder, bogus invocation, bare pre-rename name | as §2b–4 | rdm bin | 5 | required | 5d/5f pin the finished `rdm-wf-` rename and the retired `autopilot.js` |
| 5g | This repo's `.claude/skills/` directory set equals the 11 names | dogfood tree | — | 5 | required | Rename guard over the dogfood tree; candidate |
| 5h / 5i | (gaps) | — | — | — | — | Already deleted (include_str count; CHANGELOG assert) |
| 5j / 5k | A stale downstream tree seeded from real pre-removal engine bodies (git history) is cleaned on re-emit via both `--skills` and `--plugin`; user files survive; re-plant self-test | agent_config.rs fingerprint cleanup, `generate_plugin_*` | rdm bin, git | 5 | required | — |
| 6a–6c | pi `--skills` writes no `.claude/workflows`; `claude --skills --user` writes no workflows; emission works with a missing `RDM_ROOT` | agent_config.rs platform/scope gates | rdm bin | 5 | required | — |
| 7a | Builds a non-Rust fixture repo, an `rdm init` plan repo (`acme-web`), a relocated rdm binary; resolves node | fixture | git, rdm bin, node | 5 | required | — |
| 7b | Heredoc `downstream.mjs extract`: the emitted engine becomes importable and the inverse transform is byte-identical; `meta.name` equals the file stem; imports stay in scratch | emitted `rdm-wf-review-refute-fix.js` | node, rdm bin | 5 | required | — |
| 7c | `downstream.mjs logic`: `resolveReviewers` on the emitted engine; ≥8 built commands name the fixture binary; `--project acme-web` threading; `resolveRdmBin`/`projectFlag` | review.mjs (stamped, emitted) | node | 2 or 5 (ambiguous) | required | Header still names the removed `deriveSignals`/`selectDimensions` |
| 7d | `downstream.mjs exec`: two engine-built persist ladders run against the fixture plan repo; the review reads back submitted/request-changes with one resolved anchored comment; a ladder with `--verdict` dropped fails | review.mjs persist, `rdm review` CLI | node, rdm bin, git | 2 or 5 (ambiguous) | required | — |
| 7e | Three sed mutants of the emitted engine turn §7c red | review.mjs | node, sed | 2 or 5 | required | Mutant D already deleted |
| 7g / 7h | Emitted instruction/skill text: no retired whole-tree staging sentence, "changeset" present, every quoted flag is accepted by the real binary, no pointers into this repo's docs; self-tests | templates, rdm-cli arg surface | rdm bin | 5 | required | Mostly prose-grep; only flag acceptance is behavioural |
| 7i (Codex distribution) | `agent-config codex` (+`--skills`/`--user`): AGENTS.md, 4 skills under `.agents/skills`, 7 withheld with a notice, `CODEX_HOME` placement, `--plugin` rejected; emitted `roadmap list`/`phase update` run against the fixture (`needs-review` stamped); local `.agents/skills` equals generator output | agent_config.rs Codex adapter, `scripts/gen-codex-skills.sh` | rdm bin, git | 5 | required | — |
| 7i (`rdm-dev.sh`) | `scripts/rdm-dev.sh session id` from a foreign cwd keeps an explicit `RDM_SESSION`; refuses a missing session or plan repo | `scripts/rdm-dev.sh`, rdm session | rdm bin (wrapper may cargo build) | 5 (or 6; ambiguous) | required | — |
| 7i (`node --test scripts/lib/codex-smoke-process.test.mjs`) | Codex smoke-process lifecycle and cleanup tests | `scripts/lib/codex-smoke-process.mjs` | node | **phase 1 (done)** | removed | Deleted with the JS helper; replaced by the `rdm-devtools` nextest tests (see §4) |
| 7i (`verify-codex-coexistence.mjs`) | Real Codex coexistence run in an isolated temp home | `scripts/verify-codex-coexistence.mjs` | node, codex CLI, optional auth/network | 4 | **opt-in: only when `RDM_CODEX_BIN` is set** (`RDM_CODEX_AUTH_FILE` optional) | — |

### verify-plugin-distribution.sh
| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| 1 / 1b / 7 | Hermeticity baseline; `--plugin --out` emit; status unchanged | agent_config.rs `generate_plugin_*`, rdm-cli `--plugin` | git, rdm bin | 5 | required | — |
| 2 | Manifest fields present and no `workflows` key; 11 skills and 5 workflows laid out correctly; `.claude-plugin/` holds only the manifest | `generate_plugin_manifest` | rdm bin | 5 | required | — |
| 3 | Naming transform: skill dirs drop `rdm-`, engines keep `rdm-wf-` (hardcoded lists) | `PLUGIN_SKILL_NAMES` | rdm bin | 5 | required | — |
| 4 | Every `rdm:<engine>` reference resolves (floor ≥5) | skill templates (plugin render) | rdm bin | 5 | required | Prose-grep |
| 5a–5d, 5f, 5g | Each rejected flag combination has its own distinct message; positive control `--skills --user` | rdm-cli agent-config validation | rdm bin | 5 | required | — |
| 6a–6e | Planted self-tests for §§2–5 | harness | — | 5 | required | — |

### verify-plugin-install.sh
| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| 1 / 1b / 8 | Hermeticity; fresh `--plugin` emit with no `--project` | agent_config.rs | git, rdm bin | 5 | required | — |
| 2 | Checked-in `plugins/rdm/` equals fresh output, version normalized on both sides | `plugins/rdm/`, agent_config.rs | rdm bin | 5 | required | — |
| 3 | Fresh manifest version equals the Cargo.toml crate version | manifest, Cargo.toml | rdm bin | 5 | required | — |
| 4 / 4b | `marketplace.json` shape, non-empty entries, each `source` resolves to a plugin | `.claude-plugin/marketplace.json` | — | 5 | required | — |
| 5 / 6 | Workflows byte-identical and name sets equal; skill inventory and frontmatter | `plugins/rdm/{workflows,skills}` | rdm bin | 5 | required | — |
| 7a–7l | Planted self-tests: dangling source, empty entries, renamed skill, stripped frontmatter, mutated workflow, version bump, stray file, blank fields, isolated floors | harness | — | 5 | required | — |

### observe-plugin-install.sh (not in CI)
| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| 0 | Exits 2 with a NOTICE when `claude` is absent | harness | — | 5 | opt-in | — |
| 1 / 1b / 2 | Snapshots the real `~/.claude`; isolated `CLAUDE_CONFIG_DIR`; temp marketplace | harness, `plugins/rdm` | rdm bin | 5 | opt-in | — |
| 3 / 3b | `claude plugin validate --strict` on the committed tree and the temp marketplace | `plugins/rdm`, marketplace.json | claude CLI | 5 | opt-in | — |
| 4 / 4b | Offline `marketplace add` then `install rdm@rdm` | same | claude CLI | 5 | opt-in | — |
| 5–5d | `plugin list --json` shows one entry at the crate version; installed skills equal emitted; workflows checked on disk; `details` is corroboration only | same | claude CLI | 5 | opt-in | — |
| 6 / 6b | Real `~/.claude` byte-unchanged; the write landed in the isolated dir | harness | claude CLI | 5 | opt-in | — |

### observe-workflow-listing.sh (not in CI, no sections)
| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| (whole) | A live `claude -p` rooted at the repo lists Skill entries; asserts every engine and `rdm-*` front door renders, no bare pre-rename name, no double prefix | `.claude/workflows/*.js` meta, `.claude/skills` frontmatter | claude CLI (authenticated), network | 5 or 7 (ambiguous) | opt-in | Only guards the finished `rdm-wf-` rename; its hermetic half was retired 2026-09-23; candidate for full retirement |

### verify-golden-json.sh
| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| 1 | Fresh capture of the 20-command `--format json` inventory, redacted and diffed byte-for-byte against `tests/golden/*.json` | rdm-cli JSON surfaces; `scripts/lib/golden-capture.sh`, `scripts/lib/rdm-plan-fixture.sh` | rdm bin, git | 5 | required | — |
| 2 | Two independent same-day captures match after redaction; no raw `/tmp` path leaks | redaction rules | rdm bin, git | 5 | required | — |
| 2b | `estimate_snapshot` digest is redacted with the key kept; no bare 64-hex; planted self-test | rdm-core `content_digest`, phase show JSON | rdm bin | 5 | required | — |
| 3 / 4 | A mutated golden copy trips the drift detector; the real goldens re-diff clean | harness | sh | 5 | required | — |

### verify-plugin-loop.sh
| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| 1 | task create (stdin body) → list → update → show → fuzzy-typo search → `info --format json` | rdm-cli task/search/info | rdm bin, git | 5 | required | — |
| 2 | `--body` beats stdin; update never reads stdin; `--body ""` is refused; `--clear-body` works | rdm-cli `resolve_body`/`map_body_clobber` | rdm bin | 5 | required | Already duplicated by the `rdm-cli/tests/cli_task.rs` body tests |
| 3 | FIFO watchdog: `create` blocks on stdin until EOF, then returns promptly; `</dev/null` returns at once | rdm-cli create stdin path | rdm bin, mkfifo | 5 | required | — |
| 4 | Bad slug: non-zero exit, actionable stderr, empty stdout; `needs_review_warning` goes to stderr only and JSON stdout stays clean | rdm-cli error mapping, `commands/{task,phase}.rs` | rdm bin, git | 5 | required | — |

### verify-claude-code-web-loop.sh
| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| step 1 | Seed a plan repo and bare-clone it as the fake remote | rdm create/commit | rdm bin, git | 5 | required | — |
| step 2 | `templates/claude-code-web/.claude/hooks/SessionStart.sh` in a fake sandbox clones the repo and sets the global `root` | SessionStart template, `rdm bootstrap` | rdm bin, git, bash (`file://`, no network) | 5 | required | — |
| step 3 | `roadmap list`/`phase show` work against the bootstrapped clone | rdm read path | rdm bin | 5 | required | — |
| steps 4–6 | Source commit with a `Done:` line → `rdm hook post-commit` → phase done with the SHA | `rdm hook post-commit` | rdm bin, git | 5 | required | — |
| step 7 | Push the plan update to the bare origin | git | git | 5 | required | Trivial; fold into 4–6 |
| step 8 | `rdm review pending` lists only items finalized on the current branch | `review pending` scoping | rdm bin, git worktree | 5 | required | Overlaps worktree-review B (the Stop hook it once drove is retired) |

### verify-git-config-isolation.sh
| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| 1 | rdm-git and the cli_{worktree,gate,verify,review_change} suites give identical results under a clean and a hostile git config | `rdm-git/src/git_test_support.rs`, `rdm-cli/tests/git_test_support.rs`, `rdm-git/src/source.rs` `unified_diff_argv` | cargo nextest ×2, git ≥2.32 | 5 | required | — |
| 1b | python3 strips the isolation from both support files in place; results must diverge; files restored and `cmp`-checked | same | cargo nextest rebuild, python3 | 5 | required | Edits tracked source; needs a different mechanism in Rust |
| 2 | No `*__worktrees` directory escapes into scratch TMPDIR in either run; fixture floor | `rdm-git/src/worktree.rs` | the §1 runs | 5 | required | Duplicates temp-hygiene §1 |

### verify-worktree-temp-hygiene.sh
| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| 1 | Full nextest of `-p rdm-git -p rdm-cli` with TMPDIR redirected leaves no `*__worktrees` | `rdm-git/src/worktree.rs` `worktree_path`/`add`; fixtures in `rdm-cli/tests/cli_{worktree,gate,verify}.rs`, `rdm-git/tests/worktree.rs` | cargo nextest (~86s) | 5 | required | — |
| 1b | python3 re-roots a fixture at its TempDir; the leak must appear; file restored | same | cargo rebuild, python3 | 5 | required | Edits tracked source |

### verify-review-revision-loop.sh
| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| setup | Submitted request-changes review with 4 comments; comment 3's span reworded after submit | `rdm-core/src/ops/reviews.rs` | rdm bin, git | 5 | required | — |
| A | Resolved anchor → `--status addressed --applied-commit` recorded | reviews.rs update, anchor resolution | rdm bin, git | 5 | required | — |
| B | Whole-document comment reports `unresolved` and is addressed | reviews.rs | rdm bin | 5 | required | — |
| C | `drifted` anchor; a clarification reply keeps it open; `--state addressed` is refused | reviews.rs drift and close guard | rdm bin, git | 5 | required | — |
| D | wont-fix with a reply and no applied commit | reviews.rs | rdm bin | 5 | required | — |
| E | Close to `addressed`; the review leaves `review requests` | reviews.rs, `review requests` | rdm bin | 5 | required | — |

### verify-backlog-groom-loop.sh
| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| setup | Seeds a duplicate pair, tag cluster, stale task, consolidate target and terminal roadmap | rdm create | rdm bin, git | 5 | required | — |
| 1 | `rdm backlog report` shows all four signal kinds | `rdm-core/src/ops/backlog.rs` | rdm bin | 5 | required | — |
| 2 | `promote --into` folds a task into a roadmap as a phase | `ops/task.rs` `consolidate_task_into_roadmap` | rdm bin | 5 | required | — |
| 3 | `task merge` folds dup-b into dup-a | `ops/task.rs` `merge_tasks` | rdm bin | 5 | required | — |
| 4 | `task update --status wont-fix --reason` | task update | rdm bin | 5 | required | — |
| 5 | `roadmap archive` on the terminal roadmap | roadmap archive | rdm bin | 5 | required | — |

### verify-worktree-review-loop.sh
| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| setup | Two roadmaps, `rdm worktree add` for each, phases finalized to needs-review (branch and SHA stamped) | `rdm worktree`, needs-review stamping | rdm bin, git worktree | 5 | required | — |
| A | Replays the retired Pi `agent_end` inject rule in sh over `review pending --format json` | `review pending` | rdm bin, git | 5 | required | Models the retired Pi review-on-finalize extension (unify-code-review phases 6–7); B covers the live scoping, so drop |
| B | Each worktree's `review pending` lists only its own roadmap | branch-scoped `review pending` | rdm bin, git worktree | 5 | required | — |
| C | Pending is empty from `main`; the query still exits cleanly after the worktree and branch are removed | same | rdm bin, git | 5 | required | — |
| D | `rdm review restamp` after an amend updates `review_sha`, is idempotent, and the item stays in scope | `review restamp` | rdm bin, git | 5 | required | — |

### verify-rdm-plan-fixture.sh
| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| AC1 | `fixture_setup`/`fixture_code_repo`/`fixture_teardown` build and remove an isolated plan repo and code repo with the documented seeds | `scripts/lib/rdm-plan-fixture.sh` | rdm bin, git | 5 | required | Tests shell test-support only; retires when golden-json and plugin-loop move to a Rust fixture |
| AC1b–e | Guard clauses; HOME/XDG preserved and restored exactly; caller `RDM_*` never leaks in; a failing seed fails loudly | fixture lib | rdm bin | 5 | required | Same (carry the env-scrub into the Rust fixture) |
| AC2 | Two same-day runs are byte-identical after documented redactions | fixture lib, rdm JSON | rdm bin, git | 5 | required | Overlaps golden-json §2 |
| AC3 | A stand-in real `RDM_ROOT` stays byte- and mtime-unchanged; plus a static grep that the lib never names it | fixture lib | rdm bin, stat | 5 | required | Static grep breaks the no-grep rule: keep the sentinel only |
| AC4 | Optional shellcheck/shfmt over the lib and harness | — | shellcheck, shfmt (optional) | 7 | required (skips if absent) | Duplicates CI's blanket shellcheck/shfmt |

### verify-session-identity.sh
Dependencies ("common") for every row: rdm bin, git, POSIX sh, no network.

| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| A | Same session id within a session, different ids across two sessions (real processes) | `rdm-core/src/session/{mod,lease,process}.rs`, `rdm-cli/src/commands/session.rs` | common | 6 | required | — |
| B | Bare env resolves at rung 2 (parent-pid lease) and writes a lease file | `session/lease.rs`, `process.rs` | common | 6 | required | — |
| C | `RDM_SESSION` overrides everything; rung 3 (`CLAUDE_CODE_SESSION_ID`) writes no lease | `session/mod.rs` | common | 6 | required | — |
| D | Lease for a live pid with a stale start time is not adopted | `lease.rs`, `process.rs` | common, sh wrapper | 6 | required | — |
| E / F | Journal holds exactly the session's paths; concurrent sessions' journals are disjoint | `session/journal.rs`, rdm-store-git wiring | common | 6 | required | — |
| G | Session state stays out of `rdm status` and a whole-tree commit; decoy self-test | journal.rs, `commands/status.rs`, `rdm-store-git/src/commit.rs` | common | 6 | required | — |
| H / H2 | Max resolve cost over 20 runs stays under 250 ms; real `hook post-commit` finishes within `hook_timeout_secs` | `session/mod.rs`, rdm-core hook | common | 6 | required | — |
| J (+self-test) | A harness-published id beats an inherited ancestor lease; a mutant restores the merge bug | `session/mod.rs`, `lease.rs` | common, **mutant build** (~15s) | 6 | required | — |
| K0–K5 | Per-call `sh -c` wrappers under a long-lived driver: fragmentation happens; `rdm commit` exits 0 and names the cause and remedy; dead leases are swept and bounded; two drivers never merge; `RDM_HARNESS_SESSION_ID` yields one changeset | `lease.rs` create-path sweep, `commands/commit.rs` advisory | common, background drivers | 6 | required | — |
| K self-tests 1–2 | Mutants with the advisory silenced or the sweep removed fail K2 and K3 | same | **mutant builds** | 6 | required | — |

### verify-journal-truncation-race.sh
Dependencies ("common") for every row: rdm bin, git, POSIX sh, no network. One shared mutant `cargo build` (~1–2 min cold) serves §§1b, 2b, 5b, 6b and 7b.

| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| 1 | 40 parallel mutations and 6 concurrent commits under one `RDM_SESSION`: nothing stranded, journal folds to empty | `session/journal.rs` `record`/`truncate`, rdm-store-git `commit_changeset_id` | common | 6 | required | — |
| 1b | The same fan-out with a commit pinned at the barrier: clean under the fix, stranded under the mutant | journal.rs `truncate` | `RDM_HARNESS_JOURNAL_BARRIER`, shared mutant | 6 | required | — |
| 2 / 2b | Commit parked inside `truncate` while create/update land; records survive, including a rewrite of a path being landed; the read-modify-write mutant loses them | journal.rs, store-git commit | `RDM_HARNESS_JOURNAL_BARRIER`, **shared mutant build** | 6 | required | — |
| 3 | Planted corruptions prove the grep, cleanliness and emptiness assertions can fail | harness | common | 6 | required | — |
| 4 | "Empty" is judged by `read_journal` folding to zero entries, not by file absence | journal.rs `read_journal` | common | 6 | required | — |
| 5 / 5b | `session gc` from an unrelated process is excluded while an append holds the shared lock; the lockless mutant loses the record | journal.rs `lock_journal`/`compact`/`gc_changesets` | `RDM_HARNESS_APPEND_BARRIER`, shared mutant | 6 | required | — |
| 6 / 6b | An append arriving while gc is parked between its compare-and-swap and its rename waits and lands in the rewritten journal; the mutant loses it | journal.rs `compact` | `RDM_HARNESS_COMPACT_BARRIER`, shared mutant | 6 | required | — |
| 7 / 7b | A record appended during a parked `session discard --force` survives; the bare `remove_file` mutant loses it | journal.rs `discard_changeset` | `RDM_HARNESS_JOURNAL_BARRIER`, shared mutant | 6 | required | — |

### verify-scoped-commit.sh
Dependencies ("common") for every row: rdm bin, git, POSIX sh, no network.

| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| A / B / B2 | Two sessions produce disjoint commits, with and without a session id; a rung-2 changeset continues across processes in one shell | `rdm-store-git/src/{commit,lib}.rs`, `commands/commit.rs`, `lease.rs` | common | 6 | required | — |
| B3 | Rung-4: an unattributable dirty tree is reported, never swept | store-git status/commit | common | 6 | required | — |
| C | The `Done:` hook commits only its own changeset; whole-tree stand-in self-test | rdm-core hook, store-git commit | common | 6 | required | — |
| E1 / E2 / E2b / E3 | `init --remote` lands its commit; a legacy repo gets no rdm dirt and `.git/config` is untouched; a server-shaped write is attributable | `rdm-store-git/src/{remote,repo}.rs`, rdm-server | common, bare remote | 6 | required | — |
| F | A scoped commit holds exactly its authored paths; `commit --all` self-test | store-git commit | common | 6 | required | Partial: the seeded-`INDEX.md` sub-assert targets the retired generated index |
| G / G3 / G4 | Scoped discard leaves another session's work intact and skips paths another session overwrote or recreated; stand-in self-tests | store-git discard digest guard | common | 6 | required | — (G5/G6 already retired with `rdm index`) |
| H | Reads are shared across sessions | rdm-store-fs | common | 6 | required | — |
| I | B commits under a project A created but never committed; converges | store-git commit | common | 6 | required | Partial: the dangling-row and orphan-index checks guard the retired `INDEX.md`; the convergence property is still live |

### verify-lost-update.sh
Dependencies ("common") for every row: rdm bin, git, POSIX sh, no network.

| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| 1 | (static grep) | — | — | — | — | Already retired (2026-09-23); §6 covers it |
| 2 / 2b | Two processes interleaved mid-flush: the loser is refused, with and without `RDM_SESSION` | `rdm-store-fs/src/lib.rs` baseline/flush precondition | `RDM_HARNESS_FLUSH_BARRIER` | 6 | required | — |
| 2c | Mutant with the baseline check neutered brings the lost update back | same | **mutant build**, barrier | 6 | required | — |
| 3 / 3b / 3c | A session's own sequential writes, and a two-directive `Done:` flush, never trip the check; self-test | rdm-store-fs, hook | common | 6 | required | — |
| 4 | Committing a path another session overwrote is refused (`ChangesetPathOverwritten`) | store-git `create_scoped_commit`, `JournalEntry::digest`, `rdm_core::paths::describe_path` | common | 6 | required | — |
| 5 | Hook-path rejection is logged and exits 0 | hook, store-git | common | 6 | required | — |
| 6 / 6b / 6c | A delayed delete of a path another session recreated is refused and the file survives; no-recreate control; short-circuit mutant | store-git `build_changeset_tree` delete guard | common, **mutant build** | 6 | required | — |

**Cross-cutting notes for sections 1 and 2**
- **Phase 6 mutant arms.** Every mutant arm builds from `git archive HEAD`, overlays working-tree files, then `sed`-patches a named source line, with a guard that fails if that line has moved. That guard is a source-grep. A Rust port needs a cfg- or feature-gated fault-injection seam instead of a literal port.
- **Tracked-file edits.** git-config §1b and temp-hygiene §1b edit tracked `.rs` files in place.
- **Stale headers:**
  - verify-skill-autopilot.sh (§1)
  - verify-workflow-estimate.sh (§1b)
  - verify-refuter-agreement.sh (§§10–11)
  - verify-rdm-plan-fixture.sh ("no consumer yet" is false: golden-json and plugin-loop use it)
  - verify-agent-config-distribution.sh §7c (names the removed `deriveSignals`/`selectDimensions`)
- **Generator/drift gates** (review §1/1b/1c/1g/2d/5b-drift, backlog §3, document §2, estimate §2, outcome shim). Phase 2 or 3 must decide: port each to nextest, or drop it under the operator's no-guards/no-grep rules.

## 2. JavaScript inventory

### (a) Production Claude workflow code: kept
| File(s) | Role |
|---|---|
| `.claude/workflows/lib/{review,plan-review,estimate,backlog,document}.mjs` | Single-source canonical modules, stamped or copied into the engines |
| `.claude/workflows/rdm-wf-{review-refute-fix,plan-review,estimate,backlog,document}.js` | Local engines, the Workflow-tool entry points |
| `rdm-core/src/templates/workflows/rdm-wf-*.js` (5) | Embedded emission copies (`agent-config claude --skills`), byte-identical |
| `plugins/rdm/workflows/rdm-wf-*.js` (5) | Checked-in plugin-tree copies |

### (b) Workflow behavioural tests with mocked host primitives: preserved
| Test | Rust runner (nextest) | What it proves | Dest. |
|---|---|---|---|
| `scripts/lib/review-driver.test.mjs` | `rdm-cli/tests/workflow_review_driver.rs` | Drives the real `rdm-wf-review-refute-fix.js` code path under a fake reviewer fleet against real worktrees. It executes the returned `gateCommands`/`persistScript` with the built binary, then asserts on plan state (status per outcome, anchored `--path` comments, each survivor persisted once) | **3**. Phase 2 deferred it: it is already nextest-gated, it already executes the real engine, and the phase-2 body names only the two shell suites. Phase 3 rewrites it on the `rdm_devtools::workflow` binding |
| `scripts/lib/review-changelog-range.test.mjs` (deleted) | `rdm-core/tests/workflow_review_changelog_range.rs` (deleted) | `buildReviewPipeline('code')` with a recording agent; the `changelog` finder prompt grades the range, not each commit | **phase 2 (done)**: retired with its wrapper. Its assertions were regexes over fixed prompt wording, so they are not behavioral coverage: a string-presence check (operator no-grep rule) |
| `scripts/lib/plan-review-hoist.test.mjs` | `rdm-core/tests/workflow_plan_review_driver.rs` (`plan_review_driver_hoist_behavior`, plus the Rust byte-identity test `plan_review_driver_block_is_byte_identical`) | Suites A–E: `runPlanReviewDriver` and the shipped engine dispatch only finder/refuter labels for every target kind; implementation-plan path; caller reviewer set against `DIMENSIONS`; env-arg half | **3**. Same rationale as `review-driver.test.mjs`: already individually nextest-gated and executing the real driver, so phase 3 rewrites it on the phase-2 binding |
| `scripts/lib/estimate-writeback.test.mjs` | `rdm-cli/tests/workflow_estimate_writeback.rs` | Real `phase list` JSON → `buildEstimatePipeline` → the returned `phase update --difficulty` commands run in a shell → difficulty and derived tier read back | 3 |
| `scripts/lib/workflow-env-args.test.mjs` | `rdm-core/tests/workflow_env_args.rs` | Backlog and document engines' builders honour runtime `rdmBin`/`project` (no dogfood binary or project literals) | 3 |
| `scripts/verify-review-source.mjs` (deleted; was run by verify-workflow-review-outcome.sh) | none | `classifyOutcome` completeness (escalated on incomplete coverage, missing/invalid AC, budget pass-through, refuter error; reviewed on non-gating/`coverage.last`). Loads the engine as an `AsyncFunction` with fake primitives: the legacy survivors-only shape carries no outcome, and a mixed task+phase identity is rejected. Also string-structural asserts and byte-identity with the template | **phase 2 (done)**: behaviour ported to `rdm-cli/tests/workflow_review/{outcome,engine}.rs`. Its greps were retired; see the verify-workflow-review-outcome.sh table |
| Heredocs in verify-workflow-review.sh §3–§15h (deleted) | `rdm-cli/tests/workflow_review/` | See the verify-workflow-review.sh table: pipeline, budget, non-gating, laundering, persist ladder, shell-injection | **phase 2 (done)** |
| Heredoc `downstream.mjs` in verify-agent-config-distribution.sh §7b–7e | none | The emitted engine made importable; reviewer resolution and binary/project agnosticism; persist ladders executed against a foreign plan repo; mutants | 2 or 5 (ambiguous) |
| Heredocs in verify-workflow-{backlog,document,estimate}.sh | none | See those tables (`behavior.mjs`, `zero-mutation.mjs`, `test.mjs`, `test-real.mjs`, `real.mjs`, `paramz.mjs`, `rdmbin.mjs`, `node --check` parse gates) | 3 |

### (c) Codex runtime/spike code and tests
Owner: the `codex-agent-support` roadmap (docs/codex-support.md, codex-runtime.md, codex-orchestration-spike.md).

**How CI runs them.** A separate step runs `mise exec node -- node --test --test-concurrency=1 scripts/lib/codex-spike-*.test.mjs scripts/lib/codex-runtime*.test.mjs` with no credentials. Those tests use fake codex stubs on PATH and the prebuilt `target/debug/rdm` passed through `RDM_BIN`. `codex-smoke-process.test.mjs` is NOT in that step; it runs from verify-agent-config-distribution §7i.

| File | Role | Product vs tooling | Tests | Dest. |
|---|---|---|---|---|
| `scripts/rdm-codex.mjs` | 9-line CLI: `node scripts/rdm-codex.mjs run-spec.json` → `runRuntime` | Product surface (phase-3 Codex runtime, documented entrypoint in `docs/codex-runtime.md`) | via `codex-runtime.test.mjs` (CLI invalid specs) | 4 (phase body says "codex tooling"; ambiguous because it is a documented, if repo-only, product entrypoint) |
| `scripts/lib/codex-runtime.mjs` | Explicit Codex host: plan-review, code-review and estimate over the canonical `review.mjs`/`plan-review.mjs`; model resolution through `rdm model resolve` | Product (runtime) | `codex-runtime.test.mjs` (14): tier bindings, role rejection, drift/HEAD/phase-body rejection, incomplete coverage | 4 |
| `scripts/lib/codex-runtime-state.mjs` | Run manifest, owned session, direct-argv mutations, `safeGit` | Product (runtime) | `codex-runtime-state.test.mjs` (16): durable intent, uncertainty, cancellation reaping, output limits, git-override isolation | 4 |
| `scripts/lib/codex-runtime-estimate.mjs` | Estimate preview/apply over `lib/estimate.mjs` with an atomic precondition and scoped commit | Product (runtime) | `codex-runtime-estimate.test.mjs` (11, includes a real plan repo); `codex-runtime-estimate-interruption.test.mjs` (SIGTERM after update/commit, reconciliation); `codex-runtime-queue.test.mjs` (bounded concurrency, cancellation); `codex-runtime-review-process.test.mjs` (finder barrier, refuter JSON rejection) | 4 |
| `scripts/lib/codex-process.mjs` | Shared Codex subprocess transport (read-only sandbox, ignore user config, bounded parallel) | Product (runtime transport) | `codex-spike-process.test.mjs` (12) | 4 |
| `scripts/run-codex-orchestration-spike.mjs` | Opt-in research runner that uses saved Codex auth | Tooling / experimental | none | 4 |
| `scripts/lib/codex-spike-estimate.mjs`, `codex-spike-review.mjs`, `codex-spike-process.mjs` (2-line re-export) | Phase-2 spike host adapters | Experimental tooling | `codex-spike-estimate.test.mjs` (real CLI; skips done phases), `codex-spike-review.test.mjs` (4), `codex-spike-process.test.mjs` | 4 |
| `scripts/lib/codex-smoke-process.mjs` + `.test.mjs` | Test-only live-process lifecycle and cleanup | Tooling | run by verify-agent-config-distribution §7i | **phase 1 (done)**: deleted; replaced by `rdm-devtools` (`process` module, `rdm-smoke` entrypoint, nextest tests) |
| `scripts/verify-codex-coexistence.mjs` | Opt-in real-Codex coexistence check in a temp home; since phase 1 its live `codex exec` call runs under the Rust `rdm-smoke` entrypoint (`RDM_SMOKE_BIN` or a cargo build) | Tooling | self | 4 (port the whole flow to Rust and delete the .mjs) |

### (d) Measurement, corpus and evaluation tooling: phase 4
| File | What it does |
|---|---|
| `scripts/lib/token-report.mjs` | Reads Workflow sidecars and agent transcripts; `requestId` dedupe; token-class breakdown; grouping; first-request floor |
| `scripts/measure-lane-tokens.mjs` | CLI over token-report |
| `scripts/measure-refuter-severity.mjs` | Refuter spend by graded severity; refuter fanout; determining-finding rank; `--check`/`--audit` of `docs/token-baseline.json` |
| `scripts/lib/refuter-agreement.mjs` | Corpus loading, `refutePrompt` replay and drift check, FN/FP scoring, batching power analysis |
| `scripts/mine-refuter-corpus.mjs` | Mines historical refuter findings verbatim from transcripts |
| `scripts/run-refuter-agreement.mjs` | Dispatches the corpus through the real refuter prompt on multiple tiers (via the `claude` CLI); scoring and audits |
| `scripts/verify-review-source.mjs` | Listed here by the phase body, but it was a review-driver test. Ported and deleted in phase 2; see (b) |
| `scripts/verify-codex-coexistence.mjs` | See (c) |

### (e) Browser assets: out of scope
- `rdm-server/assets/edit.js`, `review-anchor.js` and `review-highlight.js` (466 lines in total).
- They are served by `rdm-server/src/handlers/static_assets.rs` and referenced from `templates/base.html`.
- This is existing product UI scope, not part of this roadmap.

### (f) Already retired: finder-collapse family
All are absent from the tree; `git ls-files` finds no match. `docs/finder-collapse.md` is kept as the historical record.
- **b89cedf** "test: retire the finder-collapse instrument, fix stale gate claims" deleted:
  - `scripts/verify-finder-collapse.sh`
  - `scripts/lib/finder-collapse.mjs`
  - `scripts/mine-plan-finder-corpus.mjs`
  - `scripts/run-finder-collapse.mjs`
  - `tests/fixtures/finder-collapse/`
- **533f4f5** "refactor: retire the rdm-wf-dispatch-phase Workflow engine" deleted `scripts/measure-hoist-delta.mjs`.

### Other `.js`/`.mjs`/`.cjs` files
None. Every file from `git ls-files '*.js' '*.mjs' '*.cjs'` is covered in (a)–(e) above.

## 3. Evidence limits

Two kinds of workflow evidence exist and neither substitutes for the other.

- **Workflow unit/component tests with mocked host primitives** — the
  `scripts/lib/*.test.mjs` suites run by nextest, the Node heredocs in
  `verify-workflow-*.sh`, the Rust `workflow_review` tests (phase 2), and the
  emitted-engine `downstream.mjs` arm. They execute the *actual* workflow code (canonical
  `lib/*.mjs` modules, stamped engines, or emitted copies) against scripted
  `agent`/`pipeline`/`parallel` primitives, and some run the commands the
  workflow returns against a real rdm binary and plan repo. They prove the
  workflow's own logic, prompts, generated commands and gate policy for the
  scripted inputs. They do **not** prove behaviour inside Claude's real
  Workflow runtime or that a model follows the instructions. They are
  meaningful coverage and must be preserved (phases 2–3 move their scenarios
  and assertions into Rust while still executing the real JS).
- **Actual host observations** — `observe-plugin-install.sh`,
  `observe-workflow-listing.sh`, the `RDM_CODEX_BIN` arm of
  `verify-codex-coexistence.mjs`, and the opt-in Codex spike runner. They prove
  what a real Claude Code or Codex host does with the shipped artefacts (plugin
  install, skill listing, skill selection, live model invocation). They need
  the host CLI, sometimes an account and network, stay opt-in, and report
  "not run" rather than passing when prerequisites are missing. Mocked tests
  never stand in for them, and they never stand in for the mocked tests.

The Rust `rdm-devtools` process tests added in phase 1 are a third, narrower
kind: real OS processes (fixture executables, real signals, real process
groups) exercising the smoke runner's lifecycle. They prove the runner's
cleanup contract, not anything about Codex.

## 4. Phase 1: smoke-process slice

`scripts/lib/codex-smoke-process.mjs` and its `node:test` suite were replaced
by the non-published `rdm-devtools` workspace crate (`publish = false`,
`dist = false`):

- `rdm_devtools::process::run_bounded` — bounded runner: SIGINT/SIGTERM
  interception, `prepare`, spawn in a new process group, 8 MiB stdout cap,
  stderr drained, group `SIGKILL` on every post-spawn path, reap, `cleanup`
  exactly once.
- `rdm-smoke` — entrypoint used by `verify-codex-coexistence.mjs`
  (`RDM_SMOKE_BIN` overrides the on-demand cargo build).
- `rdm-devtools-fixture` — interpreter-free fixture executable driving the
  tests.
- `rdm-devtools/tests/process_lifecycle.rs`, `tests/smoke_cli.rs` and
  `tests/signal_during_prepare.rs` — one test per lifecycle outcome,
  including a panicking hook, SIGINT/SIGTERM to the real runner process and a
  broken-teardown control (`broken_cleanup_mutant_is_detected`). The
  signal-during-prepare test raises a process-wide SIGINT, so it sits alone
  in its own binary and is safe under plain `cargo test` as well as nextest.

Usable invocation:

```sh
cargo run -q -p rdm-devtools --bin rdm-smoke -- run --timeout-secs 120 \
  [--cwd DIR] [--private-copy SRC:DEST] [--stdout-file PATH] -- <program> [args...]
cargo nextest run -p rdm-devtools
```

### Timings (recorded evidence, not asserted)

Host: macOS (Darwin 27.0.0, arm64), 2026-09-23, base `b4a3782`.

| Suite | Runs (wall, seconds) | Notes |
|---|---|---|
| `node --test scripts/lib/codex-smoke-process.test.mjs` (Node v24.18.0), before deletion | 3.49, 5.98, 0.81 (spec reporter); 2.24, 3.38, 3.90 (tap reporter; `duration_ms` 2158, 2863, 3490) | 3 tests: success/failure/timeout combined, SIGINT, SIGTERM |
| `cargo nextest run -p rdm-devtools`, warm build | 8.12, 8.05, 7.26 (nextest summary 6.26, 6.17, 6.25) | 17 tests when timed; the suite now has 22 |
| `cargo nextest run -p rdm-devtools -E 'test(/sig/)' --test-threads 8`, with hostile `RDM_ROOT`, `RDM_PROJECT`, `RDM_SESSION`, `NODE_OPTIONS`, `GIT_CONFIG_GLOBAL` inherited | 1.23 (summary 0.45) | 2 signal tests, concurrent |
| full `-p rdm-devtools --test-threads 16` under the same hostile env | summary 6.16 | all 17 then-present tests pass |

Per-test (nextest, warm): lifecycle tests 0.05–0.45 s each; the timeout tests
1.4–1.6 s (1–1.5 s timeouts); SIGINT/SIGTERM 0.45–0.48 s;
`broken_cleanup_mutant_is_detected` 6.2 s, dominated by its two 1.5 s timeout
runs and the 3 s bounded wait that proves the leaked grandchild survives.

## 5. Phase 2: review workflow tests

`scripts/verify-workflow-review.sh`, `scripts/verify-workflow-review-outcome.sh`,
`scripts/verify-review-source.mjs`, `scripts/lib/review-changelog-range.test.mjs`
and `rdm-core/tests/workflow_review_changelog_range.rs` are deleted. Their
coverage is mapped section by section in § 1.

### Mechanism decision: a Node child process

This is a desk comparison; no spike was committed.

- **Embedded engines** (`rquickjs`/QuickJS-ng, `boa`) would add a C toolchain or
  a heavy pure-Rust engine build to every `cargo nextest run`. Rust would need
  its own module loader and promise-job pump, and more bespoke binding code.
  They also diverge from the V8/Node family the Claude Workflow host runs on,
  and a spec gap risks false failures on the 3.4k-line `review.mjs`.
- **Node** (pinned in `.mise.toml`) was already a dependency of several
  nextest tests and of CI through mise. It adds no crates, and it runs the same
  engine family as the host.
- **Chosen: Node**, driven as a bounded child over a line-delimited JSON
  protocol.

**Node is an explicit prerequisite.** `rdm_devtools::workflow::resolve_node`
looks at `RDM_TEST_NODE` (read, never set by tests), then `node` on `PATH`,
then `mise which node`. A missing runtime is an error naming `mise install`,
`PATH` and `RDM_TEST_NODE`. It is never a skip.

### The binding

- **`rdm_devtools::process::Session`** is the interactive sibling of phase 1's
  `run_bounded`. It uses the same `ProcessSpec`, spawns in a fresh process
  group, and installs the same SIGINT/SIGTERM interception, so a nextest
  slow-timeout SIGTERM still tears the child down. On `Drop` and on every error
  it sweeps the group with SIGKILL and reaps the child. On top of that it pipes
  stdin through a writer thread (so sending never blocks), splits stdout into
  lines with a per-line cap, keeps an 8 KiB stderr tail for diagnostics, and
  enforces one overall deadline on every receive. The phase-1 tests are
  unchanged and green.
- **`rdm_devtools::workflow`** is the Rust binding. `workflow_host.mjs` is the
  only test-side JavaScript: it is embedded with `include_str!` and written into
  each host's private temp directory. The glue imports modules, compiles
  function bodies, encodes and decodes values, keeps a handle table, forwards
  callbacks, and serializes thrown errors (name, message, stack, plus the module
  path for a failed import, which Node leaves off ESM syntax errors). It routes
  every `console` method to stderr. It contains no tests, scenarios, expected
  values, assertions, fixture branches or review logic; the module rustdoc
  states that contract.
- **Protocol.** One JSON object per line. Rust sends `import`, `compile`,
  `get`, `call`, `reply` and `shutdown`. Node answers with `ok`, `err`, or
  `callback`. Values are JSON plus these single-key tags:
  - `{"$fn":n}` and `{"$ref":n}`: glue-held functions and opaque values;
  - `{"$callback":n}`: a Rust handler;
  - `{"$host":"parallel"|"pipeline"}`: a fake host primitive;
  - `{"$undefined":true}`.

  Rust records each callback's arrival sequence, so barrier and fresh-refuter
  properties are observed. A handler may also hold a reply and release it
  later (`Agent::hold_then_release`), which is how a fixed refuter completion
  order is planted for the determinism case.
- **Fake host primitives**, documented in rustdoc and limited to what these
  tests need:
  - `parallel(thunks)` is an order-preserving `Promise.all`; a thunk that throws
    resolves to `null`.
  - `pipeline` is deliberately not implemented. No ported case reaches a
    `pipeline()` call: the review core demands the dependency but composes with
    `parallel` only. Calling it throws `unsupported host primitive: pipeline`.
  - `log` is a recorded Rust callback.

  There is no schema validation, no model and no Claude host. These are
  component tests; they do not establish behaviour in Claude's real Workflow
  host (see § 3).
- **Script loading.** Both transforms live in Rust:
  - `driver_source` drops the `export ` of the single column-0
    `export const meta`, and the body is compiled with
    `(args, agent, pipeline, parallel, log)`. This executes the driver.
  - `helper_source` injects `return { names }` before the unique
    `// --- Driver` sentinel. The helpers are returned and the driver never
    runs.

  Either transform fails actionably when the meta line or the sentinel is
  missing or duplicated. The child runs with `NODE_OPTIONS` and every `RDM_*`
  variable removed, with its temp directory as the working directory.
- **Mutants.** `MutantTree` copies real sources into a per-test temp directory
  and keeps their relative layout, so the `plan-review.mjs → review.mjs` import
  still resolves. `replace_once` refuses a missing or ambiguous anchor and names
  the mutant. A mutant test requires its scenario to fail on a check or a
  JavaScript exception; an infrastructure failure never counts as catching a
  mutant.

### Tests

- **`rdm-devtools/tests/workflow_host.rs`** (9 cases) exercises the binding
  with the real `review.mjs` only:
  - callbacks and returned functions;
  - the parallel finder barrier;
  - rejection and throw propagation;
  - missing runtime;
  - invalid module (a truncated copy yields Node's `SyntaxError` naming the
    file, and the host still shuts down with its temp directory removed);
  - malformed protocol (`rdm-devtools-fixture print garbage`: the error quotes
    the line, the child is reaped and the temp directory removed);
  - child timeout (`rdm-devtools-fixture sleep`: the error names
    `request #1 (import of …)` and the group is killed);
  - the per-line cap;
  - four concurrent isolated hosts.
- **`rdm-cli/tests/workflow_review/`** is one test binary with 109 cases. By
  module: pipeline 17, outcome 6, coverage 10, budget 27, plan 6, persist 26,
  engine 5, generators 12. Seventeen of those cases are planted-logic mutant
  controls. Every scenario, scripted agent reply and expected value is Rust.
  The persist cases run the emitted ladder under `sh` against
  `CARGO_BIN_EXE_rdm` in a per-test plan repo, with a hermetic environment: no
  inherited `RDM_*`, and global/system git and rdm config isolated. Git
  inspection goes through `rdm-cli/tests/git_test_support.rs`. The first case
  built was the vertical slice
  `pipeline::code_mode_drops_refuted_and_low_confidence_findings` with its
  floor-comparison mutant.
- **Deferred to phase 3**, both already individually nextest-gated and both
  already executing the real engines:
  - `scripts/lib/review-driver.test.mjs`
  - `scripts/lib/plan-review-hoist.test.mjs`

### Timings (recorded evidence, not asserted)

Host: macOS (Darwin 27.0.0, arm64, 18 cores), 2026-09-23, Node v24.18.0.

| Suite | Runs (wall, seconds) | Notes |
|---|---|---|
| `sh scripts/verify-workflow-review.sh`, before deletion | 3.62, 3.62, 3.64 | warm `target/debug/rdm` |
| `sh scripts/verify-workflow-review-outcome.sh`, before deletion | 7.61, 0.41, 0.40 | the first run includes its `cargo build -p rdm-cli` relink |
| `cargo nextest run -p rdm-cli --test workflow_review`, warm | 1.46, 1.44, 1.46 (summary 1.230, 1.207, 1.225) | 109 cases; per case 0.008–0.820 s (the persist cases that seed a plan repo are the slowest) |
| same, `--test-threads 1` | 8.17 (summary 7.93) | serial |
| `cargo nextest run -p rdm-devtools`, warm | 10.29, 9.30, 9.27 (summary 9.09, 9.09, 9.07) | 31 cases, dominated by phase 1's `broken_cleanup_mutant_is_detected`; `workflow_host` cases 0.007–0.812 s (the timeout case waits out an 0.8 s deadline) |

When the `workflow_host` cases run alongside phase 1's short process tests on
macOS, nextest occasionally marks one of those phase-1 tests "leaky": the test
passes, but its output pipe took longer than the leak timeout to close. This
never happened with `workflow_host` excluded, or under unrelated CPU load.
Phase 2 did not investigate it further; nothing fails.
