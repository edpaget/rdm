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
as the single runner and the final coverage audit; phase 8 — the remaining
Codex runtime process suites (§ 11).

The "Retirement rationale" column also records *candidates* the survey
noticed (duplicates, prose/source greps that the operator's no-grep rule would
retire rather than port). A candidate is not a decision: each owning phase
decides, and coverage is retired only when tied to landed product retirement
or to an operator rule, never because a test uses a mocked host.

## 1. Verification scripts

At phase 1, CI's `Shell harnesses` step (`.github/workflows/ci.yml`) ran `cargo build`, then every `scripts/verify-*.sh` in a loop, so every verify-* row was **required** unless the row says otherwise; `observe-*.sh` scripts were never in that loop and are **opt-in**. Phase 7 removed that step once the loop had no members: every row below is now owned by `cargo nextest run` (or the required `suite-hygiene` profile), and § 10 holds the consolidated map.

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
| 15h / 15h-mut | Scratch-file hygiene | review.mjs persist ladder | Rewritten as behaviour, running the emitted ladder under `sh` against the real binary with a per-test `TMPDIR`: `persist::ladder_scratch_file_not_at_predictable_path` (a symlink planted at `$TMPDIR/rdm-persist-start.$$.json` by the ladder's own shell; the victim is untouched), `persist::concurrent_ladders_do_not_collide`, `persist::ladder_leaves_no_scratch_file`; mutants `persist::mutant_scratch_path_predictable` (victim clobbered) and `persist::mutant_scratch_rm_removed` (file left) | nextest | Regexes over the ladder text (mktemp template, no `$$`, no `/tmp` redirect, rm in the same entry). R-grep, replaced by the executed checks. The change-target ladder shape is not run here; it needs a real source checkout and is owned by `driver::persist_ladder_records_change_review_anchors` (phase 3) |
| (named gaps) | §§2a, 2b-fid, 2c, 2c(v), 5b-exec | — | Already deleted before phase 2 | — | — |

**Totals over the 27 live sections.** 16 ported in full: 1, 1b, 1c, 1g, 3c, 3c-mut, 5, 5f, 8c, 9, 9c, 15b, 15d-path, 15g, 15h, 15h-mut. 5 split, with the behaviour ported and the string checks retired: 1e, 3, 8, 14, 15. 4 retired in full as string checks: 4, 4a, 4a-guard, 13. 2 retired as duplicates with a named equivalent: 2d (its filename-set listings retired as string checks), 5b-drift.

### verify-workflow-review-outcome.sh (deleted in phase 2)
| Script § | Actual behaviour exercised | Dest. (named Rust tests) | CI | Retired sub-assertions |
|---|---|---|---|---|
| `cargo build -p rdm-cli` | Build step | none needed: nextest builds `CARGO_BIN_EXE_rdm` | — | — |
| `verify-review-source.mjs` (deleted) | `classifyOutcome` completeness; engine driver legacy shapes; mixed identity; source and template checks | Ported: `outcome::classify_outcome_completeness_cases` (all nine inputs), `engine::legacy_survivors_only_shape_{code,plan}` and `engine::mixed_task_phase_identity_rejected` (both execute the driver). New: `engine::emitted_template_helpers_run_without_driver` (extraction from `rdm-core/src/templates/workflows/rdm-wf-review-refute-fix.js`). Rewritten: `engine::driver_deterministic_and_emits_no_done_trailer` replaces the `Done:\|Date.now\|Math.random` source-absence check by running the source-bound driver twice and checking its returned commands | nextest | The driver-region counts of `classifyOutcome(` and `buildReviewPipeline('code')`. R-grep. Engine/template byte identity is covered by `rdm-core/src/agent_config.rs::generate_workflows_are_byte_identical_to_source` |
| Five `gen-*` `--check` calls | Generator drift | Duplicates of `generators::review_block_in_sync_in_every_consumer`, `generators::skill_projection_in_sync_{code,plan}` and `generators::local_skill_projection_in_sync_{code,plan}` | nextest | — |

### verify-workflow-backlog.sh (deleted in phase 3)
**Phase 3 (done).** Every row names its Rust equivalent in `rdm-cli/tests/workflow_passes/backlog.rs` (`backlog::…`, filter `-E 'binary(workflow_passes) and test(/^backlog::/)'`) or records its retirement. "NR" means *not behavioral coverage (operator no-grep rule, 2026-09-23)*.

| Script § | Actual behaviour exercised | Dest. (named Rust tests) | CI | Retired sub-assertions |
|---|---|---|---|---|
| 1 | `parseBacklogArgs` (`olderThan: 0`, empty tag, string/`'null'` payloads, refusals) | `backlog::parse_backlog_args` | nextest | — |
| 1 | `CATEGORY` order, `ANALYSIS_SCHEMA` | `backlog::category_order_and_schema` | nextest | — |
| 1 | `parseBacklogReport`, `selectCategories`, `normalizeAnalysis`, `consolidateBatch` | `backlog::{parse_backlog_report, select_categories, normalize_analysis, consolidate_batch}` | nextest | — |
| 1 | `backlogReportCommand` text | Rewritten, executed (rule 2): `backlog::report_command_executes_with_threaded_filters`. The returned command runs under a bare `PATH` on a decoy-default repo and equals a direct `rdm backlog report` with the same filters; the tag filter is shown to change the report; an empty tag gives the unfiltered report; the omitted-axes arm runs a plain `rdm` through the `PATH` shim on the default project | nextest | `includes('--older-than 0')`-style text checks, replaced by execution |
| 1 | Analyzer prompt grooming rules | Retired | — | NR: "READ-ONLY", "NEVER execute", `--status wont-fix`, `--roadmap-slug`, `--into`, "task merge", "survivor", "roadmap list", "mutually exclusive", "roadmap archive", "Never add --force" |
| 1b | Driven `buildBacklogPipeline` | `backlog::{empty_report_short_circuits, full_report_one_analyzer_per_category, fetch_error_propagates, crashed_analyzer_degrades, output_deterministic, missing_fetch_report_throws}`: zero calls on an empty report; four labels, each with `opts.schema` equal to `ANALYSIS_SCHEMA`; each analyzer's proposal and question consolidated into the returned batch; the rejection message; the crashed category becoming an open question | nextest | — |
| 2 | Zero mutation over a real report | Strengthened: `backlog::engine_zero_mutation_over_real_report` runs the real `rdm-wf-backlog.js` driver (the shell drove the lib pipeline only). With no `report` it dispatches nothing and returns `fetchError` plus `reportCommand`; that command is executed and its JSON fed back; four subsections, every analyzer in the declared `Analyze` phase; the plan repo's HEAD, status and every file are identical before and after | nextest | — |
| 3 / 3b | Block byte identity (private awk diff) and its sed self-test | `backlog::generator_in_sync` (real `gen-workflow-backlog.sh --check`), `backlog::generator_drift_detected_then_healed` (scratch tree: drift inside the engine's block, and in the template and plugin copies the awk diff never checked, each goes red and heals byte-exact) | nextest | The private awk extraction, superseded by the generator's own check |
| 5 | Module parse | Duplicate of `backlog::engine_zero_mutation_over_real_report` (loading the driver compiles it) | nextest | — |
| 5b | Planted duplicate-`meta` self-test | `backlog::engine_duplicate_meta_fails_to_load` (negative control: `load_driver` fails with a `SyntaxError`) | nextest | — |

### verify-workflow-document.sh (deleted in phase 3)
**Phase 3 (done).** Rust tests are in `rdm-cli/tests/workflow_passes/document.rs` (`document::…`).

| Script § | Actual behaviour exercised | Dest. (named Rust tests) | CI | Retired sub-assertions |
|---|---|---|---|---|
| 2 / 2b | Block byte identity and planted-drift self-test | `document::generator_in_sync`, `document::generator_drift_detected_then_healed` (as for backlog, including the template and plugin copies) | nextest | The private awk extraction |
| 3 | `parseDocumentArgs`, `defaultOutPath`/`resolveOutPath`, `computeIncompletePhases`, `buildGitRangeCommands` | `document::{parse_document_args, out_path_resolution, compute_incomplete_phases, git_range_commands}` | nextest | The exact command text for a fabricated `abc123`, replaced by execution (next rows) |
| 4 (4a–4c) | Real `roadmap`/`phase show` JSON from three seeded roadmaps through the functions | `document::real_roadmap_json_drives_decisions` | nextest | — |
| 4 (strengthened) | — | `document::git_range_commands_execute_in_source_repo`: the returned `log`/`diffStat` run in the real source repo and name exactly the phase's commit and file | nextest | — |
| (new) | The shell never ran the driver | `document::engine_aborts_incomplete_before_any_agent`, `document::engine_gathers_then_synthesizes_and_writes` (see § 6) | nextest | — |

### verify-workflow-estimate.sh (deleted in phase 3)
**Phase 3 (done).** Rust tests are in `rdm-cli/tests/workflow_passes/estimate.rs` (`estimate::…`).

| Script § | Actual behaviour exercised | Dest. (named Rust tests) | CI | Retired sub-assertions |
|---|---|---|---|---|
| 1 | `parseEstimateArgs`, `selectUnestimated`, summary text | `estimate::{parse_estimate_args, select_unestimated, summary_text}` | nextest | — |
| 1 | `estimateListCommand` text | Rewritten, executed: `estimate::list_command_executes` (bare `PATH`, decoy default; returns the three seeded phases; omitted-axes arm through the `PATH` shim) | nextest | Text equality, replaced by execution |
| 1 | Estimator prompt | `estimate::estimator_prompt_embeds_phase_body` (propagation of an injected body) | nextest | NR: the difficulty vocabulary and justification wording |
| 1 | Writeback: one command, `--difficulty`, no `--model`, `--roadmap`; no land/merge/`Done:` directive, with a detector self-test | Rewritten, executed: `estimate::writeback_sets_difficulty_and_core_tier`. Real `phase list` JSON goes into `buildEstimatePipeline`; exactly one command per phase is executed; the read-back shows `difficulty` and the core-derived `model` for two difficulties (easy → small, moderate → medium), an untouched body, and an unchanged plan-repo HEAD (the writeback commits nothing) | nextest | NR: the "no `--model`" and forbidden-directive text scans and their detector self-test; the read-back and unchanged HEAD are the behavioural form |
| 2 / 2b | `gen-workflow-estimate.sh --check`; a planted edit of the **real** engine goes red, then is restored | `estimate::generator_in_sync`, `estimate::generator_drift_detected_then_healed` (in a scratch tree: the real tree is never edited) | nextest | — |
| 3 | Module parse | Duplicate of `estimate::engine_commands_use_injected_axes` (loading the driver compiles it) | nextest | — |
| 5 | Real `phase list` JSON → `selectUnestimated` | `estimate::real_phase_list_selects_unestimated` | nextest | — |
| 9 | Banner only | — | — | — |
| 9b | Driven engine capture: injected binary and project on every emitted invocation, allow-list, determinism | Rewritten, executed: `estimate::engine_commands_use_injected_axes` (the engine's own `listCommand` is executed and fed back; the returned writebacks run under a bare `PATH` on the decoy repo and the phases read back as estimated; the rater prompt carries the injected binary and its stem), `estimate::engine_output_deterministic` | nextest | NR: the prompt invocation-token scan and allow-list regex |
| 9c | `rdmBin` defaults and refusals; project validation; the engine with no `rdmBin` | `estimate::rdm_bin_resolution_and_refusals`, `estimate::project_arg_validation`, `estimate::engine_without_rdm_bin_uses_plain_rdm` (the returned commands fail under a bare `PATH` and succeed through the `PATH` shim, so a leaked repo-local path could not resolve) | nextest | NR: the "no `./target/debug/rdm` in any prompt" absence scan |

### verify-skill-autopilot.sh (deleted in phase 3)
**Phase 3 (done).** No JavaScript was left in it; its surviving sections were plain `rdm` CLI round trips and moved to CLI tests.

| Script § | Actual behaviour exercised | Dest. (named Rust tests) | CI | Retired sub-assertions |
|---|---|---|---|---|
| 1 | (header only) | Retired with the file | — | Its assertions were retired as static greps in `adfd31a` |
| 2 (2a–2c) | Keyless repo: `--status reviewed` reads back; `--status blocked --reason …` reads back `blocked_reason` | `rdm-cli/tests/cli_phase.rs::reviewed_and_blocked_reason_read_back_as_json` (new; `cli_gate::the_gate_is_off_by_default` sets the key explicitly, so it did not cover the keyless case) | nextest | — |
| 3 | Runs `verify-workflow-estimate.sh` as a sibling gate | Retired as a duplicate: the estimate family runs directly under nextest | — | — |
| 4 | `hook done-line` amended onto a trailer-less tip, no rebase, ff-merge, `hook post-commit` → done with the landed SHA; malformed `done-line` requests rejected | `rdm-cli/tests/cli_hook.rs::done_line_amended_onto_branch_tip_completes_after_ff_merge` (new). The negatives are duplicates of `cli_hook::done_line_rejects_roadmap_without_phase` and `cli_hook::done_line_rejects_task_combined_with_phase` | nextest | — |

### verify-token-report.sh (deleted in phase 4)
Was: node, plus sed for the mutants; no rdm bin, claude or network. Now: `rdm-devtools` nextest tests (§ 7); none needs Node except those marked [node] there.

| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| 1 | `node --check` on the lib and CLI; no `package.json`/`node_modules` under `scripts/` | `scripts/lib/token-report.mjs`, `scripts/measure-lane-tokens.mjs` | node | **phase 4**: retired | — | Moot: the tool is Rust |
| 2 | CLI over `tests/fixtures/token-sidecar`: per-class and per-group totals vs `expected-totals.json`; `--worktrees-` slug; discrepancy line; first-request floor | same plus fixture | node | **phase 4 (done)**: `measure_lane_tokens::{fixture_totals_match_hand_computed, json_matches_golden, text_matches_golden, worktree_slug_sessions_included, discrepancy_reported_never_reconciled, floor_by_agent_class_excludes_cached_and_omits_empty}` | required (nextest) | — |
| 3 | Lib direct: `requestId` dedupe is last-write-wins; unreadable/empty transcripts give distinct warnings; `locateSessionDirs` handles worktree slugs; `firstRequestTokens` is null for cached/sidecarOnly | `token-report.mjs` | node | **phase 4 (done)**: unit `measure::sidecar::tests::{request_id_dedupe_last_write_wins, unreadable_and_empty_transcripts_warn_distinctly, first_request_tokens_null_for_cached_and_sidecar_only, floor_interpolates_across_a_multi_record_class}`, `measure::jsnum::tests::percentile_linear_interpolation`; `measure_lane_tokens::worktree_slug_sessions_included` | required (nextest) | — |
| 4 | CLI rejects a missing flag value, a flag taken as a value, and `--out` into a missing directory | `measure-lane-tokens.mjs` | node | **phase 4 (done)**: `measure_lane_tokens::{missing_flag_value_rejected, flag_taken_as_value_rejected, out_into_missing_directory_fails, unknown_argument_and_bad_format_rejected}` | required (nextest) | — |
| 5 | Four planted mutants each fail §2 | `token-report.mjs` | node, sed | **phase 4 (done)**: see § 7 mutant map | required (nextest) | The sed form is retired; each mutant maps to a named discriminating test |
| 6 | `measure-refuter-severity.mjs` over `tests/fixtures/token-refuter-severity`: severity counts, verdicts, token classes, fanout distributions; `--check`; corpus-free `--audit` of `docs/token-baseline.json` (`nonGatingRefutationSkip`, `refuterFanout`); `--until`; six mutants | `scripts/measure-refuter-severity.mjs`, `token-report.mjs`, imports `review.mjs` | node, sed | **phase 4 (done)**: `measure_refuter_severity::{severity_counts_verdicts_and_token_classes_match_fixture, fanout_distributions_match_fixture, check_accepts_fixture_doc, audit_committed_baseline_ok, until_pins_run_set_at_both_edges, until_rejects_unparseable_date, check_applies_doc_window_and_until_overrides}` | required (nextest) | — |
| 7 | Determining-finding rank over `tests/fixtures/token-determining-rank`: deep block compare, `unitIdent` parity, closed reason vocabulary, the four `deriveCapVerdict` branches, `--check`/`--audit`, a prose-twin check of `docs/token-baseline.md`; mutants (a)–(l) | `measure-refuter-severity.mjs`, review.mjs `rankFindings`, `docs/token-baseline.{json,md}` | node, git, sed | **phase 4 (done)**: `measure_refuter_severity::determining_rank_block_matches_fixture` plus the unit tests in § 7; the prose twin is retired | required (nextest) | Prose-twin `.md` string check: not behavioural coverage |

### verify-refuter-agreement.sh (deleted in phase 4)
Was: node, plus sed for the mutants; no real claude or network. Now: `rdm-devtools/tests/measure_refuter_agreement.rs` plus in-crate unit tests (§ 7).

| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| 1 | `node --check` on the three scripts; `--help` documents every flag and carries the cost warning | `scripts/lib/refuter-agreement.mjs`, `scripts/mine-refuter-corpus.mjs`, `scripts/run-refuter-agreement.mjs` | node | **phase 4**: retired | — | `node --check` is moot; the help-text grep is not behavioural coverage |
| 2 | Corpus loads cleanly; size/divergence/mined/authoritative floors; enum checks; adjudication commit on each item | `refuter-agreement.mjs`, `tests/fixtures/refuter-agreement/corpus.jsonl` | node | **phase 4 (done)**: `corpus_loads_with_floors_enums_and_adjudication_commits`, `invalid_corpus_items_rejected_with_named_errors` | required (nextest) | — |
| 2c (+equivalence, guard) | Batch-group power under the unit-scoped key; the unit-identity check matches the canonical rule; an underpowered arm throws or forces NO MEASUREMENT | `refuter-agreement.mjs`, `run-refuter-agreement.mjs`, `measure-refuter-severity.mjs` | node | **phase 4 (done)**: `batch_power_under_unit_scoped_key_matches_golden`, `batch_power_dispatches_nothing`, `underpowered_batched_arm_throws_or_forces_no_measurement`; 2c-equivalence retired as moot (one Rust `refuter_severity::extract::unit_ident` serves both tools, pinned by `json_target_is_not_a_unit_identity`) | required (nextest) | Batching arm kept (`no-measurement` is a result, not a retirement) |
| 3 | Every item regenerates through the REAL `refutePrompt` with a matching `promptSha256`/`promptDrift` | `refuter-agreement.mjs`, review.mjs `refutePrompt` | node | **phase 4 (done)** [node]: `corpus_prompts_regenerate_as_recorded`, `mined_prompts_exceed_sidecar_preview_length`, `refute_prompt_edit_is_reported_as_drift` | required (nextest) | — (goes red on any `refutePrompt` edit, by design) |
| 4 / 4b | Miner over the `mine-sidecars` fixture: 6 degradation paths, no silent drops, slug filter; CLI `--severity`/`--until`/`--limit`/`--out`/bad-argument messages | `mine-refuter-corpus.mjs` | node | **phase 4 (done)**: `miner_matches_goldens`, `miner_six_skip_reasons_and_accounting_identity`, `miner_slug_filter_excludes_foreign_project`, `miner_severity_until_limit_out_flags`, `miner_min_group_size_and_exclude_corpus`, `miner_bad_arguments_rejected` | required (nextest) | Help grep retired |
| 5 | Scorer over `trials-sample.json`: separate FN/FP denominators, ungraded bucket, flip rate, per-class and authoritative splits, token/tool columns | `refuter-agreement.mjs` | node | **phase 4 (done)**: `score_sample_matches_goldens`, `fn_fp_on_separate_denominators_with_ungraded_bucket`, `flip_rate_per_class_and_authoritative_splits`, `token_and_tool_columns` | required (nextest) | — |
| 5c | Batched scoring: batched prompt, id expansion, arm buckets, dispatch counting | `refuter-agreement.mjs`, review.mjs | node | **phase 4 (done)**: `batched_scoring_expansion_arm_buckets_and_dispatch_count`, `anchoring_over_qualifying_groups_only` | required (nextest) | — |
| 6 | No blended accuracy field anywhere in JSON or text output | both | node | **phase 4 (done)**: `no_blended_accuracy_key_in_any_report` | required (nextest) | — |
| 7 | `--dry-run` dispatches nothing; `--dispatch-stub` drives the full path | `run-refuter-agreement.mjs` | node | **phase 4 (done)** [node]: `dry_run_matches_golden_and_dispatches_nothing`, `fake_claude_drives_full_path` (`--dispatch-stub` became `--claude-bin` + a fake `claude`) | required (nextest) | — |
| 7b | Pure parsers; `claudeDispatch` against fake `claude` stubs on PATH (ok, fail, garbage, empty, ENOENT) | `run-refuter-agreement.mjs` | node, fake claude stubs | **phase 4 (done)**: unit `parse_claude_result_last_structured_output_wins_fenced_bare_and_non_boolean_ungraded`, `slug_and_tool_recount`; `claude_dispatch_ok_fail_garbage_empty_enoent`; [node] `concurrency_does_not_change_output_order` | required (nextest) | — |
| 7c | `parseClaudeBatchResult`: unknown ids, non-boolean verdicts, missing array | same | node | **phase 4 (done)**: unit `parse_claude_batch_result_unknown_ids_non_boolean_missing_array` | required (nextest) | — |
| 8 / 8b | `--audit` arithmetic of the committed tiering figures; `--audit-section refuterBatching` | same, `docs/token-baseline.json` | node | **phase 4 (done)**: `audit_committed_tiering_ok`, `audit_committed_batching_ok`, `audit_rejects_edited_tiering_figure`, `audit_rejects_edited_batching_figure` | required (nextest) | — |
| 9 (9a–9t) | About 20 planted mutants prove the sections above can fail | all three | node, sed | **phase 4 (done)**: see § 7 mutant map | required (nextest) | The sed form is retired |
| 10, 11 | (removed) | — | — | — | — | Already deleted (10: CHANGELOG assert, a8b4284; 11: AC9 XOR, phase 34) |

### verify-agent-config-distribution.sh (deleted in phase 5)
Now: the `rdm-cli` nextest binary `distribution` (`rdm-cli/tests/distribution/`); the per-section map with final test names is § 8.

| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| 0 / 8 | Repo `git status --porcelain` unchanged across the run | harness | git, rdm bin | **phase 5 (done)** — § 8 | required (nextest) | — |
| 1 | `agent-config claude --skills --out` emits a tree | `rdm-core/src/agent_config.rs` (`generate_skills`/`_workflows`/`_agents`), rdm-cli `--out` | rdm bin | **phase 5 (done)** — § 8 | required (nextest) | — |
| 2 / 2b | 11 skills, 1 workflow and 1 agent exist with valid frontmatter; no unsubstituted `{proj_flag}`-style placeholders | agent_config.rs `render_skill`, `templates/skill-*.md` | rdm bin | **phase 5 (done)** — § 8 | required (nextest) | — |
| 3 / 3a | Emitted workflow and agent are byte-identical to `.claude/workflows`/`.claude/agents` | agent_config.rs, templates/workflows | rdm bin | **phase 5 (done)** — § 8 | required (nextest) | — |
| 3b | Re-emit is idempotent and leaves an unrelated user file alone | agent_config.rs `SUPERSEDED_WORKFLOWS` cleanup | rdm bin, shasum | **phase 5 (done)** — § 8 | required (nextest) | — |
| 3c (i–iii) | Every emitted `agentType:` resolves to an emitted agent; count floor; 3 planted self-tests | agent_config.rs `generate_agents` | rdm bin | **phase 5 (done)**: retired — § 8 | — | No workflow threads `agentType` any more; retirement candidate |
| 4 | Every `.claude/workflows/<name>.js` reference and "invoke the X Workflow" instruction in a skill resolves (floors ≥2) | `templates/skill-*.md` | rdm bin | **phase 5 (done)**: retired — § 8 | — | Prose-grep (no-grep rule) |
| 5a–5f | Planted self-tests: corrupted byte, typo'd shim, planted placeholder, bogus invocation, bare pre-rename name | as §2b–4 | rdm bin | **phase 5 (done)**: retired — § 8 | — | 5d/5f pin the finished `rdm-wf-` rename and the retired `autopilot.js` |
| 5g | This repo's `.claude/skills/` directory set equals the 11 names | dogfood tree | — | **phase 5 (done)**: retired — § 8 | — | Rename guard over the dogfood tree; candidate |
| 5h / 5i | (gaps) | — | — | — | — | Already deleted (include_str count; CHANGELOG assert) |
| 5j / 5k | A stale downstream tree seeded from real pre-removal engine bodies (git history) is cleaned on re-emit via both `--skills` and `--plugin`; user files survive; re-plant self-test | agent_config.rs fingerprint cleanup, `generate_plugin_*` | rdm bin, git | **phase 5 (done)** — § 8 | required (nextest) | — |
| 6a–6c | pi `--skills` writes no `.claude/workflows`; `claude --skills --user` writes no workflows; emission works with a missing `RDM_ROOT` | agent_config.rs platform/scope gates | rdm bin | **phase 5 (done)** — § 8 | required (nextest) | — |
| 7a | Builds a non-Rust fixture repo, an `rdm init` plan repo (`acme-web`), a relocated rdm binary; resolves node | fixture | git, rdm bin, node | **phase 5 (done)** — § 8 | required (nextest) | — |
| 7b | Heredoc `downstream.mjs extract`: the emitted engine becomes importable and the inverse transform is byte-identical; `meta.name` equals the file stem; imports stay in scratch | emitted `rdm-wf-review-refute-fix.js` | node, rdm bin | **phase 5 (done)** — § 8 | required (nextest) | — |
| 7c | `downstream.mjs logic`: `resolveReviewers` on the emitted engine; ≥8 built commands name the fixture binary; `--project acme-web` threading; `resolveRdmBin`/`projectFlag` | review.mjs (stamped, emitted) | node | **phase 5 (done)** — § 8 | required (nextest) | Header still names the removed `deriveSignals`/`selectDimensions` |
| 7d | `downstream.mjs exec`: two engine-built persist ladders run against the fixture plan repo; the review reads back submitted/request-changes with one resolved anchored comment; a ladder with `--verdict` dropped fails | review.mjs persist, `rdm review` CLI | node, rdm bin, git | **phase 5 (done)** — § 8 | required (nextest) | — |
| 7e | Three sed mutants of the emitted engine turn §7c red | review.mjs | node, sed | **phase 5 (done)** — § 8 | required (nextest) | Mutant D already deleted |
| 7g / 7h | Emitted instruction/skill text: no retired whole-tree staging sentence, "changeset" present, every quoted flag is accepted by the real binary, no pointers into this repo's docs; self-tests | templates, rdm-cli arg surface | rdm bin | **phase 5 (done)** — § 8 | required (nextest) | Mostly prose-grep; only flag acceptance is behavioural |
| 7i (Codex distribution) | `agent-config codex` (+`--skills`/`--user`): AGENTS.md, 4 skills under `.agents/skills`, 7 withheld with a notice, `CODEX_HOME` placement, `--plugin` rejected; emitted `roadmap list`/`phase update` run against the fixture (`needs-review` stamped); local `.agents/skills` equals generator output | agent_config.rs Codex adapter, `scripts/gen-codex-skills.sh` | rdm bin, git | **phase 5 (done)** — § 8 | required (nextest) | — |
| 7i (`rdm-dev.sh`) | `scripts/rdm-dev.sh session id` from a foreign cwd keeps an explicit `RDM_SESSION`; refuses a missing session or plan repo | `scripts/rdm-dev.sh`, rdm session | rdm bin (wrapper may cargo build) | **phase 5 (done)** — § 8 | required (nextest) | — |
| 7i (`node --test scripts/lib/codex-smoke-process.test.mjs`) | Codex smoke-process lifecycle and cleanup tests | `scripts/lib/codex-smoke-process.mjs` | node | **phase 1 (done)** | removed | Deleted with the JS helper; replaced by the `rdm-devtools` nextest tests (see §4) |
| 7i (Codex coexistence) | Real Codex coexistence run in an isolated temp home | was `scripts/verify-codex-coexistence.mjs`; now `rdm_devtools::codex_coexistence` behind `rdm-smoke codex-coexistence` | codex CLI, optional auth/network | **phase 4 (done)**: the arm runs `cargo run -p rdm-devtools --bin rdm-smoke -- codex-coexistence`; hermetic coverage in `rdm-devtools/tests/codex_coexistence.rs` (§ 7) | **opt-in: only when `RDM_CODEX_BIN` is set** (`RDM_CODEX_AUTH_FILE` optional) | — |

### verify-plugin-distribution.sh (deleted in phase 5)
Now: the `rdm-cli` nextest binary `distribution` (`rdm-cli/tests/distribution/`); the per-section map with final test names is § 8.

| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| 1 / 1b / 7 | Hermeticity baseline; `--plugin --out` emit; status unchanged | agent_config.rs `generate_plugin_*`, rdm-cli `--plugin` | git, rdm bin | **phase 5 (done)** — § 8 | required (nextest) | — |
| 2 | Manifest fields present and no `workflows` key; 11 skills and 5 workflows laid out correctly; `.claude-plugin/` holds only the manifest | `generate_plugin_manifest` | rdm bin | **phase 5 (done)** — § 8 | required (nextest) | — |
| 3 | Naming transform: skill dirs drop `rdm-`, engines keep `rdm-wf-` (hardcoded lists) | `PLUGIN_SKILL_NAMES` | rdm bin | **phase 5 (done)** — § 8 | required (nextest) | — |
| 4 | Every `rdm:<engine>` reference resolves (floor ≥5) | skill templates (plugin render) | rdm bin | **phase 5 (done)**: retired — § 8 | — | Prose-grep |
| 5a–5d, 5f, 5g | Each rejected flag combination has its own distinct message; positive control `--skills --user` | rdm-cli agent-config validation | rdm bin | **phase 5 (done)** — § 8 | required (nextest) | — |
| 6a–6e | Planted self-tests for §§2–5 | harness | — | **phase 5 (done)**: retired — § 8 | — | — |

### verify-plugin-install.sh (deleted in phase 5)
Now: the `rdm-cli` nextest binary `distribution` (`rdm-cli/tests/distribution/`); the per-section map with final test names is § 8.

| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| 1 / 1b / 8 | Hermeticity; fresh `--plugin` emit with no `--project` | agent_config.rs | git, rdm bin | **phase 5 (done)** — § 8 | required (nextest) | — |
| 2 | Checked-in `plugins/rdm/` equals fresh output, version normalized on both sides | `plugins/rdm/`, agent_config.rs | rdm bin | **phase 5 (done)** — § 8 | required (nextest) | — |
| 3 | Fresh manifest version equals the Cargo.toml crate version | manifest, Cargo.toml | rdm bin | **phase 5 (done)** — § 8 | required (nextest) | — |
| 4 / 4b | `marketplace.json` shape, non-empty entries, each `source` resolves to a plugin | `.claude-plugin/marketplace.json` | — | **phase 5 (done)** — § 8 | required (nextest) | — |
| 5 / 6 | Workflows byte-identical and name sets equal; skill inventory and frontmatter | `plugins/rdm/{workflows,skills}` | rdm bin | **phase 5 (done)** — § 8 | required (nextest) | — |
| 7a–7l | Planted self-tests: dangling source, empty entries, renamed skill, stripped frontmatter, mutated workflow, version bump, stray file, blank fields, isolated floors | harness | — | **phase 5 (done)** — § 8 | required (nextest) | — |

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

### verify-golden-json.sh (deleted in phase 5)
Now: the `rdm-cli` nextest binary `golden_json` (`rdm-cli/tests/golden_json/`); the per-section map with final test names is § 8.

| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| 1 | Fresh capture of the 20-command `--format json` inventory, redacted and diffed byte-for-byte against `tests/golden/*.json` | rdm-cli JSON surfaces; `scripts/lib/golden-capture.sh`, `scripts/lib/rdm-plan-fixture.sh` | rdm bin, git | **phase 5 (done)** — § 8 | required (nextest) | — |
| 2 | Two independent same-day captures match after redaction; no raw `/tmp` path leaks | redaction rules | rdm bin, git | **phase 5 (done)** — § 8 | required (nextest) | — |
| 2b | `estimate_snapshot` digest is redacted with the key kept; no bare 64-hex; planted self-test | rdm-core `content_digest`, phase show JSON | rdm bin | **phase 5 (done)** — § 8 | required (nextest) | — |
| 3 / 4 | A mutated golden copy trips the drift detector; the real goldens re-diff clean | harness | sh | **phase 5 (done)** — § 8 | required (nextest) | — |

### verify-plugin-loop.sh (deleted in phase 5)
Now: the `rdm-cli` nextest binary `cli_loops` (`rdm-cli/tests/cli_loops/`); the per-section map with final test names is § 8.

| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| 1 | task create (stdin body) → list → update → show → fuzzy-typo search → `info --format json` | rdm-cli task/search/info | rdm bin, git | **phase 5 (done)** — § 8 | required (nextest) | — |
| 2 | `--body` beats stdin; update never reads stdin; `--body ""` is refused; `--clear-body` works | rdm-cli `resolve_body`/`map_body_clobber` | rdm bin | **phase 5 (done)** — § 8 | required (nextest) | Already duplicated by the `rdm-cli/tests/cli_task.rs` body tests |
| 3 | FIFO watchdog: `create` blocks on stdin until EOF, then returns promptly; `</dev/null` returns at once | rdm-cli create stdin path | rdm bin, mkfifo | **phase 5 (done)** — § 8 | required (nextest) | — |
| 4 | Bad slug: non-zero exit, actionable stderr, empty stdout; `needs_review_warning` goes to stderr only and JSON stdout stays clean | rdm-cli error mapping, `commands/{task,phase}.rs` | rdm bin, git | **phase 5 (done)** — § 8 | required (nextest) | — |

### verify-claude-code-web-loop.sh (deleted in phase 5)
Now: the `rdm-cli` nextest binary `cli_loops` (`rdm-cli/tests/cli_loops/`); the per-section map with final test names is § 8.

| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| step 1 | Seed a plan repo and bare-clone it as the fake remote | rdm create/commit | rdm bin, git | **phase 5 (done)** — § 8 | required (nextest) | — |
| step 2 | `templates/claude-code-web/.claude/hooks/SessionStart.sh` in a fake sandbox clones the repo and sets the global `root` | SessionStart template, `rdm bootstrap` | rdm bin, git, bash (`file://`, no network) | **phase 5 (done)** — § 8 | required (nextest) | — |
| step 3 | `roadmap list`/`phase show` work against the bootstrapped clone | rdm read path | rdm bin | **phase 5 (done)** — § 8 | required (nextest) | — |
| steps 4–6 | Source commit with a `Done:` line → `rdm hook post-commit` → phase done with the SHA | `rdm hook post-commit` | rdm bin, git | **phase 5 (done)** — § 8 | required (nextest) | — |
| step 7 | Push the plan update to the bare origin | git | git | **phase 5 (done)** — § 8 | required (nextest) | Trivial; fold into 4–6 |
| step 8 | `rdm review pending` lists only items finalized on the current branch | `review pending` scoping | rdm bin, git worktree | **phase 5 (done)** — § 8 | required (nextest) | Overlaps worktree-review B (the Stop hook it once drove is retired) |

### verify-git-config-isolation.sh (deleted in phase 7)
Destination corrected to phase 7 in phase 6: phase 5 did not take it and no phase body names it. It is a suite-hygiene meta-gate — a nested `cargo nextest run` of whole crates under a hostile `~/.gitconfig` — not session or commit behaviour. **Phase 7 (done):** the `rdm-cli` binary `suite_hygiene` (`rdm-cli/tests/suite_hygiene/`), run by the required `cargo nextest run --profile suite-hygiene`; P/D/R and reasons in § 10.

| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| 1 (clean arm) | rdm-git and the cli_{worktree,gate,verify,review_change} suites pass under a clean git config | `rdm-git/src/git_test_support.rs`, `rdm-cli/tests/git_test_support.rs`, `rdm-git/src/source.rs` `unified_diff_argv` | cargo nextest, git ≥2.32 | **phase 7 (done)**: D `temp_hygiene::worktree_suites_leave_no_worktree_in_tmpdir` (a superset of crates, clean env) and the default `cargo nextest run` | required (suite-hygiene, nextest) | — |
| 1 (hostile arm, identical results) | The same suites give the same results under a hostile git config | same | cargo nextest | **phase 7 (done)**: P `git_config::hostile_git_config_changes_no_result` (exit 0 ⇔ every selected test passes, which is the clean result set; also asserts the `diff.external` marker is absent) | required (suite-hygiene) | The per-test summary parse and `diff` are harness mechanics, retired |
| 1 (no-results guard) | A run that parsed no results fails as broken | harness | — | **phase 7**: R | — | nextest's exit status replaces it: no tests run (4), a build failure (101) or a setup error is `Failure::Infra` in `suite_hygiene/nested.rs` |
| 1b | python3 strips the isolation from both support files in place; results must diverge | same | cargo nextest rebuild, python3 | **phase 7 (done)**: P `mutants::stripped_git_config_isolation_fails_the_hostile_run` (M2, in a working-tree mirror; requires exit 100) | required (suite-hygiene) | — |
| 1b (restore + `cmp`) | Files restored byte-for-byte | harness | — | **phase 7**: R | — | Moot: the mirror never writes the checkout |
| 2 (hostile leak count) | No `*__worktrees` leak under the hostile config | `rdm-git/src/worktree.rs` | the §1 runs | **phase 7 (done)**: P, inside `git_config::hostile_git_config_changes_no_result` | required (suite-hygiene) | — |
| 2 (clean leak count) | No leak under the clean config | same | the §1 runs | **phase 7 (done)**: D `temp_hygiene::worktree_suites_leave_no_worktree_in_tmpdir` | required (suite-hygiene) | — |
| 2 (fixture floor) | The clean summary contains a test named `*worktree*` | harness | — | **phase 7**: R | — | Not behavioral coverage: a name grep over the result list; non-vacuity comes from M1 (`mutants::tempdir_rooted_fixture_leaks_a_worktree`) |

### verify-worktree-temp-hygiene.sh (deleted in phase 7)
Destination corrected to phase 7 in phase 6, for the reason given for `verify-git-config-isolation.sh` above. **Phase 7 (done):** `rdm-cli/tests/suite_hygiene/`; § 10.

| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| 1 | Full nextest of `-p rdm-git -p rdm-cli` with TMPDIR redirected leaves no `*__worktrees` | `rdm-git/src/worktree.rs` `worktree_path`/`add`; fixtures in `rdm-cli/tests/cli_{worktree,gate,verify}.rs`, `rdm-git/tests/worktree.rs` | cargo nextest | **phase 7 (done)**: P `temp_hygiene::worktree_suites_leave_no_worktree_in_tmpdir` | required (suite-hygiene) | — |
| 1b | python3 re-roots a fixture at its TempDir; the leak must appear | same | cargo rebuild, python3 | **phase 7 (done)**: P `mutants::tempdir_rooted_fixture_leaks_a_worktree` (M1, in a working-tree mirror, neutral git config) | required (suite-hygiene) | — |
| 1b (restore + `cmp`) | File restored byte-for-byte | harness | — | **phase 7**: R | — | Moot: the mirror never writes the checkout |

### verify-review-revision-loop.sh (deleted in phase 5)
Now: the `rdm-cli` nextest binary `cli_loops` (`rdm-cli/tests/cli_loops/`); the per-section map with final test names is § 8.

| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| setup | Submitted request-changes review with 4 comments; comment 3's span reworded after submit | `rdm-core/src/ops/reviews.rs` | rdm bin, git | **phase 5 (done)** — § 8 | required (nextest) | — |
| A | Resolved anchor → `--status addressed --applied-commit` recorded | reviews.rs update, anchor resolution | rdm bin, git | **phase 5 (done)** — § 8 | required (nextest) | — |
| B | Whole-document comment reports `unresolved` and is addressed | reviews.rs | rdm bin | **phase 5 (done)** — § 8 | required (nextest) | — |
| C | `drifted` anchor; a clarification reply keeps it open; `--state addressed` is refused | reviews.rs drift and close guard | rdm bin, git | **phase 5 (done)** — § 8 | required (nextest) | — |
| D | wont-fix with a reply and no applied commit | reviews.rs | rdm bin | **phase 5 (done)** — § 8 | required (nextest) | — |
| E | Close to `addressed`; the review leaves `review requests` | reviews.rs, `review requests` | rdm bin | **phase 5 (done)** — § 8 | required (nextest) | — |

### verify-backlog-groom-loop.sh (deleted in phase 5)
Now: the `rdm-cli` nextest binary `cli_loops` (`rdm-cli/tests/cli_loops/`); the per-section map with final test names is § 8.

| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| setup | Seeds a duplicate pair, tag cluster, stale task, consolidate target and terminal roadmap | rdm create | rdm bin, git | **phase 5 (done)** — § 8 | required (nextest) | — |
| 1 | `rdm backlog report` shows all four signal kinds | `rdm-core/src/ops/backlog.rs` | rdm bin | **phase 5 (done)** — § 8 | required (nextest) | — |
| 2 | `promote --into` folds a task into a roadmap as a phase | `ops/task.rs` `consolidate_task_into_roadmap` | rdm bin | **phase 5 (done)** — § 8 | required (nextest) | — |
| 3 | `task merge` folds dup-b into dup-a | `ops/task.rs` `merge_tasks` | rdm bin | **phase 5 (done)** — § 8 | required (nextest) | — |
| 4 | `task update --status wont-fix --reason` | task update | rdm bin | **phase 5 (done)** — § 8 | required (nextest) | — |
| 5 | `roadmap archive` on the terminal roadmap | roadmap archive | rdm bin | **phase 5 (done)** — § 8 | required (nextest) | — |

### verify-worktree-review-loop.sh (deleted in phase 5)
Now: the `rdm-cli` nextest binary `cli_loops` (`rdm-cli/tests/cli_loops/`); the per-section map with final test names is § 8.

| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| setup | Two roadmaps, `rdm worktree add` for each, phases finalized to needs-review (branch and SHA stamped) | `rdm worktree`, needs-review stamping | rdm bin, git worktree | **phase 5 (done)** — § 8 | required (nextest) | — |
| A | Replays the retired Pi `agent_end` inject rule in sh over `review pending --format json` | `review pending` | rdm bin, git | **phase 5 (done)**: retired — § 8 | — | Models the retired Pi review-on-finalize extension (unify-code-review phases 6–7); B covers the live scoping, so drop |
| B | Each worktree's `review pending` lists only its own roadmap | branch-scoped `review pending` | rdm bin, git worktree | **phase 5 (done)** — § 8 | required (nextest) | — |
| C | Pending is empty from `main`; the query still exits cleanly after the worktree and branch are removed | same | rdm bin, git | **phase 5 (done)** — § 8 | required (nextest) | — |
| D | `rdm review restamp` after an amend updates `review_sha`, is idempotent, and the item stays in scope | `review restamp` | rdm bin, git | **phase 5 (done)** — § 8 | required (nextest) | — |

### verify-rdm-plan-fixture.sh (deleted in phase 5)
Now: the `rdm-cli` nextest binary `golden_json` (`rdm-cli/tests/golden_json/`); the per-section map with final test names is § 8. `common/seeded_plan.rs` replaces its shell library.

| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| AC1 | `fixture_setup`/`fixture_code_repo`/`fixture_teardown` build and remove an isolated plan repo and code repo with the documented seeds | `scripts/lib/rdm-plan-fixture.sh` | rdm bin, git | **phase 5 (done)**: retired — § 8 | — | Tests shell test-support only; retires when golden-json and plugin-loop move to a Rust fixture |
| AC1b–e | Guard clauses; HOME/XDG preserved and restored exactly; caller `RDM_*` never leaks in; a failing seed fails loudly | fixture lib | rdm bin | **phase 5 (done)**: retired — § 8 | — | Same (carry the env-scrub into the Rust fixture) |
| AC2 | Two same-day runs are byte-identical after documented redactions | fixture lib, rdm JSON | rdm bin, git | **phase 5 (done)** — § 8 | required (nextest) | Overlaps golden-json §2 |
| AC3 | A stand-in real `RDM_ROOT` stays byte- and mtime-unchanged; plus a static grep that the lib never names it | fixture lib | rdm bin, stat | **phase 5 (done)**: retired — § 8 | — | Static grep breaks the no-grep rule: keep the sentinel only |
| AC4 | Optional shellcheck/shfmt over the lib and harness | — | shellcheck, shfmt (optional) | **phase 5 (done)** — § 8 | required (nextest) | Duplicates CI's blanket shellcheck/shfmt |

### verify-session-identity.sh (deleted in phase 6)
Now: the `rdm-cli` nextest binary `concurrency` (`rdm-cli/tests/concurrency/`); the per-section map with final test names is § 9.
Dependencies ("common") for every row: rdm bin, git, POSIX sh, no network.

| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| A | Same session id within a session, different ids across two sessions (real processes) | `rdm-core/src/session/{mod,lease,process}.rs`, `rdm-cli/src/commands/session.rs` | common | **phase 6 (done)** — § 9 | required (nextest) | — |
| B | Bare env resolves at rung 2 (parent-pid lease) and writes a lease file | `session/lease.rs`, `process.rs` | common | **phase 6 (done)** — § 9 | required (nextest) | — |
| C | `RDM_SESSION` overrides everything; rung 3 (`CLAUDE_CODE_SESSION_ID`) writes no lease | `session/mod.rs` | common | **phase 6 (done)** — § 9 | required (nextest) | — |
| D | Lease for a live pid with a stale start time is not adopted | `lease.rs`, `process.rs` | common, sh wrapper | **phase 6 (done)** — § 9 | required (nextest) | — |
| E / F | Journal holds exactly the session's paths; concurrent sessions' journals are disjoint | `session/journal.rs`, rdm-store-git wiring | common | **phase 6 (done)** — § 9 | required (nextest) | — |
| G | Session state stays out of `rdm status` and a whole-tree commit; decoy self-test | journal.rs, `commands/status.rs`, `rdm-store-git/src/commit.rs` | common | **phase 6 (done)** — § 9 | required (nextest) | — |
| H / H2 | Max resolve cost over 20 runs stays under 250 ms; real `hook post-commit` finishes within `hook_timeout_secs` | `session/mod.rs`, rdm-core hook | common | **phase 6 (done)** — § 9 | required (nextest) | — |
| J (+self-test) | A harness-published id beats an inherited ancestor lease; a mutant restores the merge bug | `session/mod.rs`, `lease.rs` | common, **mutant build** (~15s) | **phase 6 (done)** — § 9 | required (nextest) | — |
| K0–K5 | Per-call `sh -c` wrappers under a long-lived driver: fragmentation happens; `rdm commit` exits 0 and names the cause and remedy; dead leases are swept and bounded; two drivers never merge; `RDM_HARNESS_SESSION_ID` yields one changeset | `lease.rs` create-path sweep, `commands/commit.rs` advisory | common, background drivers | **phase 6 (done)** — § 9 | required (nextest) | — |
| K self-tests 1–2 | Mutants with the advisory silenced or the sweep removed fail K2 and K3 | same | **mutant builds** | **phase 6 (done)** — § 9 | required (nextest) | — |

### verify-journal-truncation-race.sh (deleted in phase 6)
Now: the `rdm-cli` nextest binary `concurrency` (`rdm-cli/tests/concurrency/`); the per-section map with final test names is § 9.
Dependencies ("common") for every row: rdm bin, git, POSIX sh, no network. One shared mutant `cargo build` (~1–2 min cold) serves §§1b, 2b, 5b, 6b and 7b.

| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| 1 | 40 parallel mutations and 6 concurrent commits under one `RDM_SESSION`: nothing stranded, journal folds to empty | `session/journal.rs` `record`/`truncate`, rdm-store-git `commit_changeset_id` | common | **phase 6 (done)** — § 9 | required (nextest) | — |
| 1b | The same fan-out with a commit pinned at the barrier: clean under the fix, stranded under the mutant | journal.rs `truncate` | `RDM_HARNESS_JOURNAL_BARRIER`, shared mutant | **phase 6 (done)** — § 9 | required (nextest) | — |
| 2 / 2b | Commit parked inside `truncate` while create/update land; records survive, including a rewrite of a path being landed; the read-modify-write mutant loses them | journal.rs, store-git commit | `RDM_HARNESS_JOURNAL_BARRIER`, **shared mutant build** | **phase 6 (done)** — § 9 | required (nextest) | — |
| 3 | Planted corruptions prove the grep, cleanliness and emptiness assertions can fail | harness | common | **phase 6 (done)** — § 9 | required (nextest) | — |
| 4 | "Empty" is judged by `read_journal` folding to zero entries, not by file absence | journal.rs `read_journal` | common | **phase 6 (done)** — § 9 | required (nextest) | — |
| 5 / 5b | `session gc` from an unrelated process is excluded while an append holds the shared lock; the lockless mutant loses the record | journal.rs `lock_journal`/`compact`/`gc_changesets` | `RDM_HARNESS_APPEND_BARRIER`, shared mutant | **phase 6 (done)** — § 9 | required (nextest) | — |
| 6 / 6b | An append arriving while gc is parked between its compare-and-swap and its rename waits and lands in the rewritten journal; the mutant loses it | journal.rs `compact` | `RDM_HARNESS_COMPACT_BARRIER`, shared mutant | **phase 6 (done)** — § 9 | required (nextest) | — |
| 7 / 7b | A record appended during a parked `session discard --force` survives; the bare `remove_file` mutant loses it | journal.rs `discard_changeset` | `RDM_HARNESS_JOURNAL_BARRIER`, shared mutant | **phase 6 (done)** — § 9 | required (nextest) | — |

### verify-scoped-commit.sh (deleted in phase 6)
Now: the `rdm-cli` nextest binary `concurrency` (`rdm-cli/tests/concurrency/`); the per-section map with final test names is § 9.
Dependencies ("common") for every row: rdm bin, git, POSIX sh, no network.

| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| A / B / B2 | Two sessions produce disjoint commits, with and without a session id; a rung-2 changeset continues across processes in one shell | `rdm-store-git/src/{commit,lib}.rs`, `commands/commit.rs`, `lease.rs` | common | **phase 6 (done)** — § 9 | required (nextest) | — |
| B3 | Rung-4: an unattributable dirty tree is reported, never swept | store-git status/commit | common | **phase 6 (done)** — § 9 | required (nextest) | — |
| C | The `Done:` hook commits only its own changeset; whole-tree stand-in self-test | rdm-core hook, store-git commit | common | **phase 6 (done)** — § 9 | required (nextest) | — |
| E1 / E2 / E2b / E3 | `init --remote` lands its commit; a legacy repo gets no rdm dirt and `.git/config` is untouched; a server-shaped write is attributable | `rdm-store-git/src/{remote,repo}.rs`, rdm-server | common, bare remote | **phase 6 (done)** — § 9 | required (nextest) | — |
| F | A scoped commit holds exactly its authored paths; `commit --all` self-test | store-git commit | common | **phase 6 (done)** — § 9 | required (nextest) | Partial: the seeded-`INDEX.md` sub-assert targets the retired generated index |
| G / G3 / G4 | Scoped discard leaves another session's work intact and skips paths another session overwrote or recreated; stand-in self-tests | store-git discard digest guard | common | **phase 6 (done)** — § 9 | required (nextest) | — (G5/G6 already retired with `rdm index`) |
| H | Reads are shared across sessions | rdm-store-fs | common | **phase 6 (done)** — § 9 | required (nextest) | — |
| I | B commits under a project A created but never committed; converges | store-git commit | common | **phase 6 (done)** — § 9 | required (nextest) | Partial: the dangling-row and orphan-index checks guard the retired `INDEX.md`; the convergence property is still live |

### verify-lost-update.sh (deleted in phase 6)
Now: the `rdm-cli` nextest binary `concurrency` (`rdm-cli/tests/concurrency/`); the per-section map with final test names is § 9.
Dependencies ("common") for every row: rdm bin, git, POSIX sh, no network.

| Script § | Actual behaviour exercised | Owning code | Dependencies | Dest. | CI | Retirement rationale |
|---|---|---|---|---|---|---|
| 1 | (static grep) | — | — | — | — | Already retired (2026-09-23); §6 covers it |
| 2 / 2b | Two processes interleaved mid-flush: the loser is refused, with and without `RDM_SESSION` | `rdm-store-fs/src/lib.rs` baseline/flush precondition | `RDM_HARNESS_FLUSH_BARRIER` | **phase 6 (done)** — § 9 | required (nextest) | — |
| 2c | Mutant with the baseline check neutered brings the lost update back | same | **mutant build**, barrier | **phase 6 (done)** — § 9 | required (nextest) | — |
| 3 / 3b / 3c | A session's own sequential writes, and a two-directive `Done:` flush, never trip the check; self-test | rdm-store-fs, hook | common | **phase 6 (done)** — § 9 | required (nextest) | — |
| 4 | Committing a path another session overwrote is refused (`ChangesetPathOverwritten`) | store-git `create_scoped_commit`, `JournalEntry::digest`, `rdm_core::paths::describe_path` | common | **phase 6 (done)** — § 9 | required (nextest) | — |
| 5 | Hook-path rejection is logged and exits 0 | hook, store-git | common | **phase 6 (done)** — § 9 | required (nextest) | — |
| 6 / 6b / 6c | A delayed delete of a path another session recreated is refused and the file survives; no-recreate control; short-circuit mutant | store-git `build_changeset_tree` delete guard | common, **mutant build** | **phase 6 (done)** — § 9 | required (nextest) | — |

**Cross-cutting notes for sections 1 and 2**
- **Phase 6 mutant arms.** Decided in phase 6 (§ 9): no cfg- or feature-gated fault seam, which would put test-only alternate implementations in production source. Each negative control rebuilds `rdm` from an isolated mirror of the working tree (`git ls-files --cached --others --exclude-standard`, read-only) under `$CARGO_TARGET_TMPDIR`, with its family's anchored exactly-once edits applied in memory; an anchor that has moved fails the control as not-run (fixture construction, like `MutantTree::replace_once`), never as a pass.
- **Tracked-file edits.** git-config §1b and temp-hygiene §1b edit tracked `.rs` files in place.
- **Stale headers:**
  - verify-skill-autopilot.sh (§1), verify-workflow-estimate.sh (§1b): deleted in phase 3
  - verify-rdm-plan-fixture.sh ("no consumer yet" is false: golden-json and plugin-loop use it)
  - verify-agent-config-distribution.sh §7c (names the removed `deriveSignals`/`selectDimensions`)
- **Generator/drift gates** (review §1/1b/1c/1g/2d/5b-drift, backlog §3, document §2, estimate §2, outcome shim). Decided: phase 2 ported the review ones to `workflow_review::generators::*`; phase 3 ported backlog/document/estimate to `workflow_passes::{backlog,document,estimate}::generator_*`, each running the real generator's `--check` plus a scratch-tree drift → red → heal control.

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
| `scripts/lib/review-driver.test.mjs` (deleted) | `rdm-cli/tests/workflow_review_driver.rs` (deleted) | Drives the real `rdm-wf-review-refute-fix.js` code path under a fake reviewer fleet against real worktrees. It executes the returned `gateCommands`/`persistScript` with the built binary, then asserts on plan state (status per outcome, anchored `--path` comments, each survivor persisted once) | **phase 3 (done)**: `rdm-cli/tests/workflow_review/driver.rs` (`driver::…`); case map in § 6 |
| `scripts/lib/review-changelog-range.test.mjs` (deleted) | `rdm-core/tests/workflow_review_changelog_range.rs` (deleted) | `buildReviewPipeline('code')` with a recording agent; the `changelog` finder prompt grades the range, not each commit | **phase 2 (done)**: retired with its wrapper. Its assertions were regexes over fixed prompt wording, so they are not behavioral coverage: a string-presence check (operator no-grep rule) |
| `scripts/lib/plan-review-hoist.test.mjs` (deleted) | `rdm-core/tests/workflow_plan_review_driver.rs` (`plan_review_driver_hoist_behavior` deleted; the byte-identity test `plan_review_driver_block_is_byte_identical` kept) | Suites A–F: `runPlanReviewDriver` and the shipped engine dispatch only finder/refuter labels for every target kind; implementation-plan path; caller reviewer set against `DIMENSIONS`; env-arg half; source pin | **phase 3 (done)**: `rdm-cli/tests/workflow_review/plan_driver.rs` (`plan_driver::…`); case map in § 6 |
| `scripts/lib/estimate-writeback.test.mjs` (deleted) | `rdm-cli/tests/workflow_estimate_writeback.rs` (deleted) | Real `phase list` JSON → `buildEstimatePipeline` → the returned `phase update --difficulty` commands run in a shell → difficulty and derived tier read back | **phase 3 (done)**: test 1 → `workflow_passes::estimate::writeback_sets_difficulty_and_core_tier`; test 2 → `estimate::estimated_phase_left_untouched`; test 3 → `estimate::second_pass_idempotent` (each seeds its own state) |
| `scripts/lib/workflow-env-args.test.mjs` (deleted) | `rdm-core/tests/workflow_env_args.rs` (deleted) | Backlog and document engines' builders honour runtime `rdmBin`/`project` (no dogfood binary or project literals) | **phase 3 (done)**: case map in § 6 |
| `scripts/verify-review-source.mjs` (deleted; was run by verify-workflow-review-outcome.sh) | none | `classifyOutcome` completeness (escalated on incomplete coverage, missing/invalid AC, budget pass-through, refuter error; reviewed on non-gating/`coverage.last`). Loads the engine as an `AsyncFunction` with fake primitives: the legacy survivors-only shape carries no outcome, and a mixed task+phase identity is rejected. Also string-structural asserts and byte-identity with the template | **phase 2 (done)**: behaviour ported to `rdm-cli/tests/workflow_review/{outcome,engine}.rs`. Its greps were retired; see the verify-workflow-review-outcome.sh table |
| `scripts/lib/review-effort.test.mjs` (deleted; landed on main in `0f83454`, after this inventory was taken) | `rdm-core/tests/workflow_review_effort.rs` (deleted) | `buildReviewPipeline` in both modes and the plan-review driver with a recording agent: a supplied `findEffort`/`verifyEffort` reaches every finder, the retry and every refuter; absent adds no `effort` key; invalid is refused before any dispatch; `parsePlanArgs` normalises effort | **phase 7 (done)**: `rdm-cli/tests/workflow_review/effort.rs` (`effort::…`); case map in § 10 |
| Heredocs in verify-workflow-review.sh §3–§15h (deleted) | `rdm-cli/tests/workflow_review/` | See the verify-workflow-review.sh table: pipeline, budget, non-gating, laundering, persist ladder, shell-injection | **phase 2 (done)** |
| Heredoc `downstream.mjs` in verify-agent-config-distribution.sh §7b–7e | none | The emitted engine made importable; reviewer resolution and binary/project agnosticism; persist ladders executed against a foreign plan repo; mutants | 2 or 5 (ambiguous) |
| Heredocs in verify-workflow-{backlog,document,estimate}.sh (deleted) | none | See those tables (`behavior.mjs`, `zero-mutation.mjs`, `test.mjs`, `test-real.mjs`, `real.mjs`, `paramz.mjs`, `rdmbin.mjs`, `node --check` parse gates) | **phase 3 (done)**: `rdm-cli/tests/workflow_passes/` |

### (c) Codex runtime/spike code and tests
Owner: the `codex-agent-support` roadmap (docs/codex-support.md, codex-runtime.md, codex-orchestration-spike.md).

**How CI runs them.** At inventory time a separate step ran `mise exec node -- node --test --test-concurrency=1 scripts/lib/codex-spike-*.test.mjs scripts/lib/codex-runtime*.test.mjs` with no credentials, using fake codex stubs on PATH and `scripts/rdm-dev.sh` as `RDM_BIN`. Phase 4 (on main) moved the review/estimate/spike suites to Rust; phase 8 moved the last four (`codex-spike-process`, `codex-runtime-state`, `codex-runtime-review-process`, `codex-runtime-queue`) to the `rdm-cli` binary `codex_process` and deleted the step (§ 11). Every Codex runtime test now runs in `mise exec node -- cargo nextest run`, with no credentials.

**Phase 4 ruling: the Codex runtime is out of scope here.** `scripts/rdm-codex.mjs`, `scripts/lib/codex-runtime*.mjs`, `codex-process.mjs`, `codex-spike-*.mjs` and `scripts/run-codex-orchestration-spike.mjs` are production Codex runtime code owned by `codex-agent-support` (on main they now ship under `rdm-core/src/templates/codex-runtime/`, and main's `docs/codex-test-migration.md` keeps four suites — `codex-spike-process`, `codex-runtime-state`, `codex-runtime-review-process`, `codex-runtime-queue` — in a "remaining legacy" CI step awaiting their owning migration). Phase 4 ports only the coexistence smoke check. The roadmap-level "no JavaScript test files" criterion is owned by this roadmap's phase 8 (`phase-8-port-remaining-codex-runtime-process-suites`, folded in from the former task), which ports the four remaining suites and deletes their CI step (§ 10).

| File | Role | Product vs tooling | Tests | Dest. |
|---|---|---|---|---|
| `scripts/rdm-codex.mjs` | 9-line CLI: `node scripts/rdm-codex.mjs run-spec.json` → `runRuntime` | Product surface (phase-3 Codex runtime, documented entrypoint in `docs/codex-runtime.md`) | via `codex-runtime.test.mjs` (CLI invalid specs) | 4 (phase body says "codex tooling"; ambiguous because it is a documented, if repo-only, product entrypoint) |
| `scripts/lib/codex-runtime.mjs` | Explicit Codex host: plan-review, code-review and estimate over the canonical `review.mjs`/`plan-review.mjs`; model resolution through `rdm model resolve` | Product (runtime) | `codex-runtime.test.mjs` (14): tier bindings, role rejection, drift/HEAD/phase-body rejection, incomplete coverage | 4 |
| `scripts/lib/codex-runtime-state.mjs` | Run manifest, owned session, direct-argv mutations, `safeGit` | Product (runtime) | `codex-runtime-state.test.mjs` (16): durable intent, uncertainty, cancellation reaping, output limits, git-override isolation — now `codex_process::state` | 4; **phase 8 (done)** for the suite (§ 11) |
| `scripts/lib/codex-runtime-estimate.mjs` | Estimate preview/apply over `lib/estimate.mjs` with an atomic precondition and scoped commit | Product (runtime) | `codex-runtime-estimate.test.mjs` (11, includes a real plan repo); `codex-runtime-estimate-interruption.test.mjs` (SIGTERM after update/commit, reconciliation); `codex-runtime-queue.test.mjs` (bounded concurrency, cancellation); `codex-runtime-review-process.test.mjs` (finder barrier, refuter JSON rejection) — the last two now `codex_process::runner` | 4; **phase 8 (done)** for the queue and review-process suites (§ 11) |
| `scripts/lib/codex-process.mjs` | Shared Codex subprocess transport (read-only sandbox, ignore user config, bounded parallel) | Product (runtime transport) | `codex-spike-process.test.mjs` (12 test declarations, 26 cases with the parameterised rejections) — now `codex_process::transport` | 4; **phase 8 (done)** for the suite (§ 11) |
| `scripts/run-codex-orchestration-spike.mjs` | Opt-in research runner that uses saved Codex auth | Tooling / experimental | none | 4 |
| `scripts/lib/codex-spike-estimate.mjs`, `codex-spike-review.mjs`, `codex-spike-process.mjs` (2-line re-export) | Phase-2 spike host adapters | Experimental tooling | `codex-spike-estimate.test.mjs` (real CLI; skips done phases), `codex-spike-review.test.mjs` (4), `codex-spike-process.test.mjs` (the re-export: now `codex_process::transport::spike_entrypoint_runs_the_runtime_transport`, phase 8) | 4 |
| `scripts/lib/codex-smoke-process.mjs` + `.test.mjs` | Test-only live-process lifecycle and cleanup | Tooling | run by verify-agent-config-distribution §7i | **phase 1 (done)**: deleted; replaced by `rdm-devtools` (`process` module, `rdm-smoke` entrypoint, nextest tests) |
| `scripts/verify-codex-coexistence.mjs` (deleted) | Opt-in real-Codex coexistence check in a temp home | Tooling | now `rdm-devtools/tests/codex_coexistence.rs` | **phase 4 (done)**: whole flow ported to `rdm-smoke codex-coexistence` (§ 7) |

### (d) Measurement, corpus and evaluation tooling: phase 4 (done)
All deleted; each is replaced by a subcommand of the repository-only `rdm-measure` binary (crate `rdm-devtools`). See § 7.

| File (deleted) | What it did | Replacement |
|---|---|---|
| `scripts/lib/token-report.mjs` | Reads Workflow sidecars and agent transcripts; `requestId` dedupe; token-class breakdown; grouping; first-request floor | `rdm_devtools::measure::sidecar` |
| `scripts/measure-lane-tokens.mjs` | CLI over token-report | `rdm-measure lane-tokens` (`measure::lane_tokens`) |
| `scripts/measure-refuter-severity.mjs` | Refuter spend by graded severity; refuter fanout; determining-finding rank; `--check`/`--audit` of `docs/token-baseline.json` | `rdm-measure refuter-severity` (`measure::refuter_severity`) |
| `scripts/lib/refuter-agreement.mjs` | Corpus loading, `refutePrompt` replay and drift check, FN/FP scoring, batching power analysis | `measure::refuter_agreement::{corpus, prompt, trials, score, report, audit, dispatch}` |
| `scripts/mine-refuter-corpus.mjs` | Mines historical refuter findings verbatim from transcripts | `rdm-measure mine-refuter-corpus` (`measure::refuter_agreement::miner`) |
| `scripts/run-refuter-agreement.mjs` | Dispatches the corpus through the real refuter prompt on multiple tiers (via the `claude` CLI); scoring and audits | `rdm-measure refuter-agreement` |
| `scripts/verify-review-source.mjs` | Listed here by the phase body, but it was a review-driver test. Ported and deleted in phase 2; see (b) | — |
| `scripts/verify-codex-coexistence.mjs` | See (c) | `rdm-smoke codex-coexistence` |

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

- **Workflow unit/component tests with mocked host primitives** — the Rust
  `workflow_review` (phases 2–3) and `workflow_passes` (phase 3) tests, the
  remaining `codex-*` `.test.mjs` suites (phase 4), and the emitted-engine
  `downstream.mjs` arm. (The other `scripts/lib/*.test.mjs` suites and the
  `verify-workflow-*.sh` Node heredocs were retired in phases 2–3.) They execute the *actual* workflow code (canonical
  `lib/*.mjs` modules, stamped engines, or emitted copies) against scripted
  `agent`/`pipeline`/`parallel` primitives, and some run the commands the
  workflow returns against a real rdm binary and plan repo. They prove the
  workflow's own logic, prompts, generated commands and gate policy for the
  scripted inputs. They do **not** prove behaviour inside Claude's real
  Workflow runtime or that a model follows the instructions. They are
  meaningful coverage and must be preserved (phases 2–3 move their scenarios
  and assertions into Rust while still executing the real JS).
- **Actual host observations** — `observe-plugin-install.sh`,
  `observe-workflow-listing.sh`, the `RDM_CODEX_BIN` arm running
  `rdm-smoke codex-coexistence` (and its ignored `codex_coexistence_live`
  test), and the opt-in Codex spike runner. They prove
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
  (`RDM_SMOKE_BIN` overrides the on-demand cargo build). Since phase 4 the
  whole coexistence flow is the `rdm-smoke codex-coexistence` subcommand and
  the `.mjs` is deleted (§ 7).
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
  already executing the real engines, and ported there (§ 6):
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

## 6. Phase 3: remaining workflow tests

Deleted: `scripts/verify-workflow-{backlog,document,estimate}.sh`,
`scripts/verify-skill-autopilot.sh`,
`scripts/lib/{review-driver,plan-review-hoist,estimate-writeback,workflow-env-args}.test.mjs`,
`rdm-cli/tests/workflow_review_driver.rs`,
`rdm-cli/tests/workflow_estimate_writeback.rs`,
`rdm-core/tests/workflow_env_args.rs`, and the Node runner plus three source
string asserts in `rdm-core/tests/workflow_plan_review_driver.rs` (its
byte-identity half is kept). `git ls-files 'scripts/lib/*.test.mjs'` now lists
only `codex-*` files (phase 4). No `.js`/`.mjs` file was added, and
`rdm-devtools/src/workflow_host.mjs` is unchanged: no case calls `pipeline()`,
so phase 2's throw-if-called placeholder suffices.

### Classification rules (applied uniformly)

1. **Behavioural, ported**: assertions on values a workflow function or driver
   returns (including rendered summary, batch or round-note text whose content
   comes from the scenario's inputs); thrown errors; the recorded agent call
   sequence, labels and `opts`; and the effect of executing a returned command
   against the real binary.
2. **Generated CLI commands are executed wherever they are meant to be run.**
   The command runs under `sh` (or `/bin/bash`) with `PATH=/usr/bin:/bin`, so no
   ambient `rdm` resolves, against a per-test plan repo whose default project is
   a decoy; the resulting state is read back. A dropped `rdmBin` fails to
   resolve and a dropped `--project` hits the decoy, so the fixture is its own
   negative control (both were confirmed by temporarily dropping each axis from
   the driver and plan-driver arguments: the executing tests go red). The
   omitted-axes arms run with a per-test `bin/` directory holding an `rdm`
   symlink to `CARGO_BIN_EXE_rdm`, on a repo whose default project is the target.
3. **Prompt text counts only as input propagation**: a test-chosen sentinel
   supplied as input must reach the dispatched prompt. Checks for fixed wording
   written in source are retired as *not behavioral coverage (operator no-grep
   rule, 2026-09-23)* — "READ-ONLY", "NEVER execute", the plan/code source-pin
   sentences, vocabulary lists, forbidden-directive lists, absence scans for
   hard-coded binaries or projects in prompt prose, and source-text checks such
   as "no import/require".
4. **Byte identity through regenerate-and-compare is kept**
   (`gen-workflow-*.sh --check` plus a scratch-tree drift → red → heal
   control). No new drift guard was added.
5. **No Rust mirror**: expected values are literals per scenario; no workflow
   decision is recomputed in Rust.

### Layout

| Family | Binary / module | Filter | Tests |
|---|---|---|---|
| review driver | `rdm-cli/tests/workflow_review/driver.rs` | `-E 'binary(workflow_review) and test(/^driver::/)'` | 33 |
| plan-review driver | `rdm-cli/tests/workflow_review/plan_driver.rs` | `-E 'binary(workflow_review) and test(/^plan_driver::/)'` | 45 |
| backlog | `rdm-cli/tests/workflow_passes/backlog.rs` | `-E 'binary(workflow_passes) and test(/^backlog::/)'` | 19 |
| document | `rdm-cli/tests/workflow_passes/document.rs` | `-E 'binary(workflow_passes) and test(/^document::/)'` | 12 |
| estimate | `rdm-cli/tests/workflow_passes/estimate.rs` | `-E 'binary(workflow_passes) and test(/^estimate::/)'` | 16 |
| autopilot's CLI round trips | `rdm-cli/tests/cli_phase.rs`, `rdm-cli/tests/cli_hook.rs` | `-E 'test(=reviewed_and_blocked_reason_read_back_as_json) \| test(=done_line_amended_onto_branch_tip_completes_after_ff_merge)'` | 2 |

Each module's rustdoc names the file it executes: the canonical
`.claude/workflows/lib/*.mjs` module, or the `.claude/workflows/rdm-wf-*.js`
engine loaded with `Host::load_driver`. Nothing loads a JavaScript file a test
wrote. `workflow_passes` has the same nextest slow-timeout override as
`workflow_review`.

**Shared support**, extracted from phase 2 and included with `#[path]` by both
workflow binaries:

- `rdm-cli/tests/common/workflow_support.rs`: the generic half of
  `workflow_review/support.rs` (`Failure`, `Outcome`, `check!`/`check_eq!`,
  `load`, `split`, `Lib::real`/`Lib::mutant_of`, `run_real`, `mutant_verdict`,
  `run_mutant`, the recording scripted `Agent` with `install`), plus `Module`
  (one imported module) and `run_driver` (moved from `engine.rs`).
  `workflow_review/support.rs` keeps the review-specific part (`Js`,
  `Agent::planted`, label parsing, survivor lookups, `Lib::mutant`) and
  re-exports the rest.
- `rdm-cli/tests/common/plan_fixture.rs`: `hermetic()` and `PlanRepo` moved out
  of `workflow_review/persist.rs` (which now uses them), generalised with the
  decoy-default seed, `run` (explicit shell, `PATH`, `TMPDIR`, extra env),
  `run_open_stdin`, `rdm_on_path`, `snapshot` (HEAD, porcelain status, every
  non-`.git` file's bytes), and `SourceRepo` (`git init -b main` at
  `<TempDir>/src`, so `src__worktrees/` stays inside the `TempDir`, plus
  `add_worktree` through `rdm worktree add` and `pin`). All git calls go
  through `git_test_support.rs`.
- Both live under `rdm-cli/tests/common/`, not beside `git_test_support.rs`:
  cargo builds every top-level `tests/*.rs` as its own test target, and these
  two files are not self-contained (they name `crate::…`).

**Isolation and bounds.** Every test owns its `TempDir`, plan repo and `Host`;
no state is shared, which removes `review-driver.test.mjs`'s ordering
dependence (L1263 and L1379 relied on earlier cases). The two held-open-stdin
cases run through `rdm_devtools::process::Session` (stdin piped and never
written, one overall deadline of 8 s, or 1.5 s for the mutant, process group
swept on every exit path); no timeout is hand-rolled.

### Case map: `scripts/lib/review-driver.test.mjs` → `workflow_review::driver`

Fixture per test: plan repo (default `decoy`, project `rev-verify`, roadmap
`rm-rev` with `phase-1-clean`/`phase-2-dirty`, task `standalone-task`, plan
`rev-verify-plan`), a source repo with the roadmap and task worktrees
registered by `rdm worktree add`, and a pinned commit on each.

| JS case | Rust test | Notes |
|---|---|---|
| L366 | `driver::clean_review_gates_phase_to_reviewed` | `not-started` before, `reviewed` after the executed gate |
| L383 | `driver::rework_gates_phase_to_in_progress` | |
| L399 | `driver::task_target_gates_via_task_update` | |
| L411 | `driver::refused_gate_write_fails_plain_shell` | fault injection kept (HEAD moved on); also reads back that nothing stamped `reviewed` |
| L435 | `driver::persist_ladder_records_change_review_anchors` | owner of phase 2's § 15h change-target ladder shape: `target.kind = change`, file-quote anchor, degraded anchor, whole-document finding, note body, `persistDegraded`, `anchorsDegraded=` |
| L554 | `driver::apostrophe_finding_runs_under_system_bash` | `/bin/bash` (3.2 on macOS) |
| L591 | `driver::path_with_line_suffix_lands_anchor` | |
| L652, L752, L822, L876, L935, L1005 | `driver::runtime_refusal_{outside_hunk_blocking_parks, outside_hunk_suggestion_no_park, quote_not_found_parks, spoofed_does_not_touch_parks, unmodified_file_benign, path_absent_at_head_parks}` | spoofed-quote negative kept |
| L1059 | `driver::mixed_anchor_run_reports_partial` | |
| L1156 | `driver::whole_document_by_design_never_degrades` | its absence-of-clause check on the persisted body is the negative control for L435's clause |
| L1204 | `driver::all_anchors_degraded_sets_all` | |
| L1263 | `driver::gate_refuses_reviewed_when_park_required`, `driver::park_guard_is_what_refuses` | yes/no/unset executed; guard-strip control kept. Retired: the `gateScript` contains-variable and contains-guard checks (NR); the stdout "no `status: reviewed`" check is subsumed by the status read-back |
| L1318 | `driver::refused_review_start_fails_persist_plain_shell` | stub-rdm fault injection kept |
| L1340 | `driver::no_ac_reviewer_escalates` | |
| L1362 | `driver::unresolvable_source_escalates_without_dispatch` | five broken pins, zero agent calls |
| L1379 | `driver::gate_false_writes_nothing` | seeds its own state; asserts a byte-identical plan-repo snapshot |
| main `0f83454` (added after the port, carried over on the rebase onto main) | `driver::find_and_verify_effort_reach_every_finder_and_refuter` | effort args reach every finder/refuter; none dispatched without them |
| L1390 | duplicate of `engine::mixed_task_phase_identity_rejected` | |
| L1397 | duplicate of `engine::legacy_survivors_only_shape_{code,plan}` | |
| L1417 | `driver::anchor_refusal_classification_uses_real_error_text` | each refusal text is `rdm_core::error::Error`'s own `Display`, not a hand-copied string; spoofed-quote negatives kept |
| L1513 | `driver::has_blocking_in_scope_matrix` | |
| L1535 | `driver::refute_prompt_plan_command_follows_mode` | propagation: an injected `planCommand` sentinel reaches the code refuter; absent/undefined/`''` give byte-identical prompts; supplying a plan adds lines in both modes, and code mode adds strictly more (the scope clause). The fixed "IN SCOPE" phrase check is NR |
| L1572 | `driver::out_of_scope_blocker_reviewed_in_scope_reworks` | |
| L1623 | `driver::comment_header_in_scope_round_trip` | the header-line regexes are NR; the parse round trip is the behaviour |
| L1639, L1678 | `driver::parse_comment_header_legacy_{seven,six}_key` | |
| L1708, L1740 | `driver::persist_anchor_invalid_path_falls_back`, `driver::persist_anchor_strips_slashed_suffix` | no-fallback negative kept |
| L1758 | `driver::persist_ladder_completes_under_open_stdin` | through `Session` |
| L1787 | `driver::dev_null_redirects_prevent_stdin_hang` | stub `cat`-first rdm; redirect-strip mutant: the `Session` deadline expires and the group is killed |

(34 cases: 32 ported into 33 tests, 2 duplicates.)

### Case map: `scripts/lib/plan-review-hoist.test.mjs` → `workflow_review::plan_driver`

Three drives: LIB (the real `runPlanReviewDriver` with a Rust `runPlanReview`
that records every context), REAL (over the real `buildReviewPipeline('plan')`),
SHIP (the real `rdm-wf-plan-review.js` driver). Executed commands run against a
decoy-default repo with project `prv` (roadmap `r` with `phase-1-x`/`phase-2-y`,
task `t`, plan `p`).

| JS case(s) | Rust test | Notes |
|---|---|---|
| A1 | `plan_driver::every_target_dispatches_only_finders_and_refuters` | four target kinds, non-empty floor |
| A2 | retired | NR: absence of fixed wording in prompts |
| A3 | `plan_driver::unit_commands_read_their_own_document` | `itemCommand`/`roadmapCommand` executed, returning the named item's JSON |
| A4 | `plan_driver::roadmap_sweep_reviews_caller_named_stems` | |
| A5 | `plan_driver::terminal_stems_skipped_and_reported` | |
| A6 | `plan_driver::gate_commands_clear_only_the_plan_review_tag` | executed: tags read back `["depends-unlanded"]`; HEAD advances by exactly one commit |
| A7 | `plan_driver::gate_without_tags_refuses_visibly` | |
| A8 | `plan_driver::rework_keeps_tag_and_renders_round_note` | |
| A9, A9b | `plan_driver::round_from_caller_prior_reviews` | persist off and on |
| A9c | `plan_driver::repeat_detection_reads_prior_comments` | |
| A9d | `plan_driver::absent_vs_empty_prior_reviews` | |
| A10 | `plan_driver::persist_ladder_lands_unit_review` | executed: a submitted review on `phase/r/phase-1-x` is read back; the persist-off arm also asserts no `persistScript` |
| A11 | `plan_driver::wont_fix_texts_suppress_matching_finding` | |
| A12 | `plan_driver::build_review_units_is_pure` | |
| B0, B1, B1b | `plan_driver::shipped_engine_dispatches_only_finders_and_refuters`, `plan_driver::shipped_engine_string_payload_equivalent` | `compileWorkflow`'s source checks (no import/require, no leftover `export`) are NR; `load_driver` compiling and running is the behaviour |
| B2 | retired | file existence; every SHIP test loads the file |
| C1 | `plan_driver::implementation_plan_grades_plan_by_slug` | `plan show p` executed |
| C2 | `plan_driver::plan_slug_persists_to_plan_target` | the ladder is executed and a review on `plan/p` read back; the free-form arm logs `persist ignored` |
| C2b | `plan_driver::free_form_plan_read_by_quoted_path` | the returned `cat --` command runs on a real file whose path contains an apostrophe and prints it verbatim |
| C2c | `plan_driver::free_form_path_reaches_every_reviewer` | propagation of a sentinel path |
| C2d, C2e, C3, C4, C5 | `plan_driver::{implementation_plan_without_document_refused_before_agents, plan_naming_modes_mutually_exclusive, implementation_plan_is_report_only, parse_plan_args_refusals, retired_transport_keys_not_parsed}` | |
| D1–D4, D7 | `plan_driver::reviewer_set_{absent_runs_all, declaration_order, unknown_dropped, only_empty_refused, no_selection_predicate}` | keys read from `DIMENSIONS`, never transcribed |
| D5, D6, D8, D9, D10 | `plan_driver::{caller_set_reaches_pipeline, caller_set_narrows_real_finders, empty_set_refused_every_target, refusal_before_any_agent, failed_unit_named_not_dropped}` | |
| E1, E2, E3 | `plan_driver::{injected_axes_reach_executed_commands, omitted_axes_use_plain_rdm_and_default_project, gate_ladder_executes_for_every_item_kind}` | executed under rule 2 (every read, gate and persist command of every target kind); the invocation-token scans over command text are replaced |
| E4, E5 | `plan_driver::{invalid_axes_refused_at_parse, axes_not_read_from_flag_string}` | |
| F1, F1b, F2, F3 | `plan_driver::source_pin_{task_form_reaches_every_prompt, phase_form_binds_phase, sweep_binds_each_unit, single_target_binds_itself}` | propagation of the injected pin values (source path, SHAs, branch); each unit binds its own item and never a sibling's; the roadmap unit carries none; at least one refuter ran. The mode-wording helpers are NR |
| F1c, F1d | retired | NR: fixed prompt wording |
| F4 | `plan_driver::unpinned_prompts_deterministic` | the absence-of-wording half is NR |
| F5 | `plan_driver::malformed_source_pin_refused_before_agents` | |

`rdm-core/tests/workflow_plan_review_driver.rs`: `plan_review_driver_block_is_byte_identical`
is kept for its byte-identity half; its three source string asserts (the
`return await runPlanReviewDriver` presence in the engine, its absence in the
lib, the `export {` offset) are deleted as NR; `plan_review_driver_hoist_behavior`
and `node_command()` are deleted.

### Case map: `scripts/lib/workflow-env-args.test.mjs`

| JS test | Rust test | Notes |
|---|---|---|
| 1 (backlog: axes reach the report command and every analyzer prompt) | report half → `workflow_passes::backlog::report_command_executes_with_threaded_filters` (executed); prompt half → `backlog::analyzer_prompts_carry_injected_axes_and_items` (propagation of the sentinel binary, `--project demo` and each report item slug) | the invocation-token scans and floors are NR |
| 2 (backlog: omitted axes) | the omitted-axes arm of `backlog::report_command_executes_with_threaded_filters` | the prompt absence arm is NR |
| 3 (backlog: invalid axes) | `backlog::invalid_axes_refused_at_parse` | |
| 4, 5 (document: both read commands, with and without axes) | `workflow_passes::document::read_commands_execute_with_injected_axes` | executed under rule 2, plus the omitted-axes arm |
| 6 (document: invalid axes) | `document::invalid_axes_refused_at_parse` | |

The estimate-writeback case map is in § 2(b). The two document engine cases
the shells never had: `document::engine_aborts_incomplete_before_any_agent`
(real `roadmap show` JSON as `roadmapMeta`, obtained by executing the engine's
own `roadmapCommand`; zero agent calls; `incompletePhases` returned) and
`document::engine_gathers_then_synthesizes_and_writes` (every `gather:<stem>`
precedes `synthesize:draft`; a thrown gather still hands the synthesizer that
phase's read command, which is executed; the gathered account reaches the
synthesizer; the returned `writeScript` is executed and the file holds the
scripted draft verbatim, quotes and `$` included).

### Shells already absent at phase 3: retirement with evidence

| Shell | Deleting commit | Product retired | Retained replacement evidence |
|---|---|---|---|
| `verify-workflow-dispatch.sh` | `533f4f5` | The `rdm-wf-dispatch-phase` engine, `lib/dispatch-phase.mjs`, and their template and plugin copies (agent-orchestrated-dispatch phase 7; `docs/workflow-vs-prose-boundary.md` § "Retirement record") | The prose `rdm-dispatch-phase` skill (validated by dogfooding, deliberately no grep harness); its gated terminal write, `rdm-cli/tests/cli_gate.rs`; the review engine it invokes, `workflow_review::*`; downstream orphan removal, `rdm-core/src/agent_config.rs::{superseded_workflows_table_names_the_renamed_and_retired_engines, resolve_superseded_workflows_removes_fingerprint_match}` |
| `verify-workflow-do-auto.sh`, `verify-workflow-do-auto-task.sh` | `b3946da` | The `rdm-do --auto` → engine wiring; `buildTaskOutcome` went with the engine in `533f4f5` | § 1 was static greps: NR. § 2 was the phase/task outcome → status write plus read-back, retained as CLI behaviour: `cli_phase::reviewed_and_blocked_reason_read_back_as_json` (new), `cli_review::blocked_lists_parked_phases_with_recorded_reasons`, `cli_task::{task_update_wont_fix_reason_shown, task_update_clear_reason}`. § 3 was a sibling gate, gone with the engine |
| `verify-skill-intent-interview.sh` | `e4716a8` | None: the whole file was prose/template greps, including its § 3 greps over the `agent-config` emission | NR. Its sibling gate is a duplicate of the phase-2 review tests |

### Shared distribution cases: named owners

- Engine and template byte identity: `rdm-core/src/agent_config.rs::generate_workflows_are_byte_identical_to_source`.
- Stamped block, embedded template copy and plugin copy for backlog, document and estimate: `workflow_passes::{backlog,document,estimate}::generator_*`.
- Review and plan-review stamped blocks: `workflow_review::generators::*` (phase 2).
- Plan-review driver block: `rdm-core/tests/workflow_plan_review_driver.rs::plan_review_driver_block_is_byte_identical`.
- Emitted-engine downstream behaviour, including estimate § 9's cited § 7c contract for the review engine: `distribution::downstream::*` in the `rdm-cli` nextest binary `distribution` (phase 5; § 8 maps each section).
- Superseded-engine removal downstream: `distribution::superseded::*` (phase 5, § 5j/5k), plus the `agent_config.rs` unit tests above.

### Negative controls and provenance kept

- Review driver: the L1263 guard-strip mutant, the L1787 redirect-strip mutant
  (now killed by the `Session` deadline), the L411 and L1318 fault injections,
  the L876 and L1417 spoofed quotes, the L1156 no-degradation negative, the
  L1708 no-fallback arm.
- Generators: backlog/document/estimate drift → red → heal, each in its own
  scratch tree, now also covering the template and plugin copies.
- Load failure: `backlog::engine_duplicate_meta_fails_to_load`.
- Fixtures: the decoy default project and bare-`PATH` design (rule 2) replace
  the retired string-scan floors.

### Timings (recorded evidence, not asserted)

Host: macOS (Darwin 27.0.0, arm64), 2026-09-23/24, Node v24.18.0, warm build.

| Suite | Runs | Notes |
|---|---|---|
| `sh scripts/verify-workflow-backlog.sh`, before deletion | 0.98, 0.53, 0.53 s wall | |
| `sh scripts/verify-workflow-document.sh`, before deletion | 0.43, 0.42, 0.41 s wall | |
| `sh scripts/verify-workflow-estimate.sh`, before deletion | 0.46, 0.37, 0.37 s wall | |
| `sh scripts/verify-skill-autopilot.sh`, before deletion | 0.86, 0.86, 0.87 s wall | includes its nested estimate run |
| `cargo nextest run --test workflow_review_driver`, before deletion | summary 8.33, 8.22, 8.76 s | one test wrapping 34 serial `node --test` cases |
| `cargo nextest run --test workflow_estimate_writeback`, before deletion | summary 0.61, 0.62, 0.62 s | |
| `cargo nextest run --test workflow_env_args`, before deletion | summary 0.07, 0.07, 0.07 s | |
| `cargo nextest run --test workflow_plan_review_driver`, before | summary 0.085, 0.086, 0.086 s | two tests (byte identity plus the `node --test` wrapper) |
| `cargo nextest run -p rdm-cli -E 'binary(workflow_passes)'`, after | summary 3.09 (first, cold host start), 1.51, 1.45 s | 47 cases; backlog 0.83 s, document 1.12 s, estimate 0.78 s alone |
| same, `--test-threads 1` | summary 7.23 s | serial |
| `-E 'binary(workflow_review) and test(/^driver::/)'`, after | summary 3.41, 3.30, 3.27 s | 33 cases; `--test-threads 1`: 17.69 s |
| `-E 'binary(workflow_review) and test(/^plan_driver::/)'`, after | summary 1.54, 1.55, 1.58 s | 45 cases; `--test-threads 1`: 4.93 s |
| `cargo nextest run -p rdm-cli --test workflow_review`, after | summary 4.10 s | 187 cases (109 phase 2 + 78 phase 3) |
| the two autopilot CLI tests, after | summary 0.65 s | |

As in phase 2, nextest occasionally marks a passing case "leaky" (its output
pipe closed after the leak timeout); seen on 1 of 3 `driver::` runs and 1 of 3
`plan_driver::` runs. Nothing failed.

### Evidence statement

Everything in this phase is **component-test evidence**: the real workflow
JavaScript under Node with a fake host (scripted agents, the phase-2 `parallel`
primitive), and the real `rdm` binary and real git for every executed command.
**No Claude-host observation** was made or claimed in this phase.
`observe-workflow-listing.sh` and `observe-plugin-install.sh` stay opt-in and
untouched.

## 7. Phase 4: measurement, corpus and Codex coexistence tooling

Every in-scope JavaScript tool and assertion entrypoint is deleted and replaced
by Rust in the repository-only `rdm-devtools` crate (nothing is added to the
shipped `rdm` binary):

| Deleted | Replacement |
|---|---|
| `scripts/lib/token-report.mjs`, `scripts/measure-lane-tokens.mjs` | `rdm-measure lane-tokens` (`measure::{sidecar, lane_tokens}`) |
| `scripts/measure-refuter-severity.mjs` | `rdm-measure refuter-severity` (`measure::refuter_severity`) |
| `scripts/mine-refuter-corpus.mjs` | `rdm-measure mine-refuter-corpus` (`measure::refuter_agreement::miner`) |
| `scripts/lib/refuter-agreement.mjs`, `scripts/run-refuter-agreement.mjs` | `rdm-measure refuter-agreement` (`measure::refuter_agreement`) |
| `scripts/verify-codex-coexistence.mjs` | `rdm-smoke codex-coexistence` (`codex_coexistence`) |
| `scripts/verify-token-report.sh`, `scripts/verify-refuter-agreement.sh` | named nextest tests (tables in § 1 and the mutant map below) |

Already retired before this phase and not resurrected: the finder-collapse
family (b89cedf) and `scripts/measure-hoist-delta.mjs` (533f4f5); see 2(f).

**Canonical versus measurement-owned.** Measurement logic (sidecar and
transcript parsing, the unit-identity rule, aggregation, percentiles, the joins,
dispositions, the cap-verdict rule, the doc check/audit arithmetic, the corpus
schema, scorer, anchoring, batching power, the experiment's own batched prompt,
the `claude -p` result parsers, the miner) is Rust. The canonical workflow
decisions a measurement replays — `rankFindings`, `survives`, `hasBlocking`,
`acTableHasGap`, `refutePrompt`, `DIMENSIONS` and `NON_GATING_SEVERITIES` — stay
in `.claude/workflows/lib/review.mjs` and are called through the phase-2 binding
behind the `measure::review_rules::ReviewRules` trait (`NodeReviewRules`, one
long-lived host; `--review-lib` points it at another copy, which is how the
mutant tests run). No Rust clone of a canonical function exists, which also
retires the JS tool's `NON_GATING_SEVERITIES` "copy drift" pin: there is no copy.
`measure::{jsnum, jsjson, jsdate}` reproduce JavaScript number printing,
`toFixed`/`toLocaleString`, `Math.round`, JSON key order and `Date.parse`, and
the workflow binding gained `Host::call_with_json_args` so a finding's key order
reaches `refutePrompt` intact.

**JavaScript runtime.** `--audit`, `--score-only`, `--batch-power`,
`lane-tokens`, `mine-refuter-corpus` and `rdm-smoke codex-coexistence` need none
(several tests run the binary with an empty `PATH` to prove it). Measuring
(`refuter-severity` without `--audit`) and `refuter-agreement --dry-run`/real
runs need Node, reached only through the binding.

**Golden evidence.** Commit 9ed3c11 ran each JS tool once over the checked-in
fixtures before deletion and committed the outputs (absolute fixture paths as
`<ROOT>`): `tests/fixtures/token-sidecar/expected/`,
`tests/fixtures/token-refuter-severity/expected/`,
`tests/fixtures/token-determining-rank/expected/` and
`tests/fixtures/refuter-agreement/expected/` (miner JSONL/JSON variants and its
stderr accounting line, `--score-only` over both trial files, `--batch-power`,
both dry runs), plus `tests/fixtures/refuter-agreement/exclude-one.jsonl`. The
Rust output is compared structurally for JSON (the `instrument` field
normalized) and byte for byte for text and JSONL; every golden matched the Rust
port byte for byte apart from `instrument`. The hand-computed files
(`expected-totals.json`, `expected-nonGatingRefutationSkip.json`,
`expected-determiningFindingRank.json`, `trials-*.json`) stay as independent
ground truth.

### Tests

| File | Tests | Needs Node |
|---|---|---|
| `rdm-devtools/tests/measure_lane_tokens.rs` | 13 | none |
| `rdm-devtools/tests/measure_refuter_severity.rs` | 12 | every test that measures a corpus (listed in the file's header); not `audit_committed_baseline_ok`, `until_rejects_unparseable_date` |
| `rdm-devtools/tests/measure_refuter_agreement.rs` | 31 | `corpus_prompts_regenerate_as_recorded`, `mined_prompts_exceed_sidecar_preview_length`, `refute_prompt_edit_is_reported_as_drift`, `dry_run_matches_golden_and_dispatches_nothing`, `fake_claude_drives_full_path`, `concurrency_does_not_change_output_order`, and the CLI arm of `claude_dispatch_ok_fail_garbage_empty_enoent` (a real run that stops at the missing binary) |
| `rdm-devtools/tests/codex_coexistence.rs` | 18 + 1 ignored (`codex_coexistence_live`) | none |
| in-crate `measure::` unit tests | 40 | `refuter_severity::rank::tests::{inert_candidate_cannot_decide_unit, unknown_disposition_is_never_imputed}` |

`claude`, `codex` and `rdm` are never real in the default suite: the
`rdm-devtools-fixture` binary plays each when started under that name (tests
symlink it), answering from a scenario file beside the symlink and recording
argv, cwd and environment names. The real Codex check is `codex_coexistence_live`
(`#[ignore]`; set `RDM_CODEX_BIN` and `RDM_BIN`, optionally
`RDM_CODEX_AUTH_FILE`, run with `--run-ignored only`), which fails rather than
passing when its prerequisites are missing.

### Mutant map

Each sed mutant of the deleted harnesses is replaced by a named test that feeds
the input exercising the branch the mutant broke, so the same regression in the
Rust code fails that test.

| Harness mutant | Regression | Failing Rust test |
|---|---|---|
| token-report §5 #1 | requestId dedupe → first-write-wins | `measure::sidecar::tests::request_id_dedupe_last_write_wins`; `measure_lane_tokens::json_matches_golden` (the fixture plants the duplicate) |
| token-report §5 #2 | discrepancy delta reconciled to 0 | `discrepancy_reported_never_reconciled` |
| token-report §5 #3 | floor read from the LAST request | `first_request_tokens_null_for_cached_and_sidecar_only`; `floor_by_agent_class_excludes_cached_and_omits_empty` |
| token-report §5 #4 | cached/sidecar-only floor as 0, not null | `first_request_tokens_null_for_cached_and_sidecar_only`; `floor_by_agent_class_excludes_cached_and_omits_empty` |
| token-report §6 window | `--until` comparison inverted | `until_pins_run_set_at_both_edges` |
| token-report §6 (a) | edited `projected` doc figure | `check_and_audit_reject_edited_severity_figure` |
| token-report §6 (b) | sentinel search never matches | `finding_extraction_requires_sentinel_anchor` |
| token-report §6 (c) | header marker never matches | `unit_and_dimension_come_from_prompt_header` |
| token-report §6 (d) | refutersDispatched joined on the label | `refuters_join_on_prompt_dimension_not_label` |
| token-report §7 (a) | edited `units.determining` | `check_and_audit_reject_edited_rank_figure` |
| token-report §7 (b) | identity ranking instead of `rankFindings` | `rank_uses_canonical_rank_findings` (a `MutantTree` of the real `review.mjs`) |
| token-report §7 (c) | unknown disposition imputed as not-refuted | `unknown_disposition_is_never_imputed` |
| token-report §7 (d) | a `CAP_VERDICT_RULE` threshold mutated | `cap_verdict_four_branches_and_thresholds`; `measure::refuter_severity::doc::tests::committed_verdict_rederives_only_under_the_real_rule` |
| token-report §7 (e) | JSON target captured as a unit | `json_target_is_not_a_unit_identity`; `determining_rank_block_matches_fixture` |
| token-report §7 (f) | run-wide orphan reason accepted | `run_wide_orphan_reason_refused_by_audit` |
| token-report §7 (g) | precedence chain reordered | `reason_vocabulary_is_closed_and_precedence_holds` |
| token-report §7 (h) | ambiguous-join detection dropped | `ambiguous_finding_join_is_unrecoverable` |
| token-report §7 (i) | unreadable finder transcript read as empty | `unreadable_finder_transcript_is_not_empty` |
| token-report §7 (j) | a retry counted as a second round | `retry_is_not_a_second_round` |
| token-report §7 (k) | disposition read before eligibility | `inert_candidate_cannot_decide_unit` |
| token-report §7 (l) | kills-cap comparison inverted | `cap_verdict_four_branches_and_thresholds` |
| refuter-agreement 9a | divergence class relabelled away | `corpus_loads_with_floors_enums_and_adjudication_commits` |
| refuter-agreement 9b | illegal authority | `invalid_corpus_items_rejected_with_named_errors` |
| refuter-agreement 9c | uncited authoritative evidence | `invalid_corpus_items_rejected_with_named_errors`; `corpus::tests::authoritative_evidence_matches_the_js_rule` |
| refuter-agreement 9d | corrupted `promptSha256` | `corpus_prompts_regenerate_as_recorded` |
| refuter-agreement 9e | FN/FP denominators swapped | `fn_fp_on_separate_denominators_with_ungraded_bucket` |
| refuter-agreement 9f | blended accuracy field | `no_blended_accuracy_key_in_any_report` |
| refuter-agreement 9g | miner not reading the transcript | `miner_matches_goldens` (edits a fixture finding and requires the mined record to move) |
| refuter-agreement 9h | first StructuredOutput wins | `parse_claude_result_last_structured_output_wins_fenced_bare_and_non_boolean_ungraded` |
| refuter-agreement 9i | non-boolean `refuted` coerced | same |
| refuter-agreement 9j | tool calls miscounted | same; `fake_claude_drives_full_path` |
| refuter-agreement 9k | `unrecoverable-mode` guard dropped | `miner_six_skip_reasons_and_accounting_identity`; `miner_matches_goldens` |
| refuter-agreement 9l | `--severity` made inert | `miner_severity_until_limit_out_flags` |
| refuter-agreement 9m | grouping key lost the unit identity | `batch_power_under_unit_scoped_key_matches_golden` |
| refuter-agreement 9n | always SUFFICIENT | `batch_power_under_unit_scoped_key_matches_golden`; `underpowered_batched_arm_throws_or_forces_no_measurement` |
| refuter-agreement 9o | constructed items grouped | `batch_power_under_unit_scoped_key_matches_golden` |
| refuter-agreement 9p | omitted id coerced to `refuted: false` | `batched_scoring_expansion_arm_buckets_and_dispatch_count` |
| refuter-agreement 9q | expanded rows counted as dispatches | same |
| refuter-agreement 9r | blended field inside the anchoring block | `no_blended_accuracy_key_in_any_report` |
| refuter-agreement 9s | batch parser accepts an unknown id | `parse_claude_batch_result_unknown_ids_non_boolean_missing_array` |
| refuter-agreement 9t | missing `verdicts` read as a clean grade | same |

### Intentional CLI and schema changes

1. Entrypoints: `node scripts/<tool>.mjs …` → `cargo run -q -p rdm-devtools --bin rdm-measure -- <lane-tokens|refuter-severity|mine-refuter-corpus|refuter-agreement> …`, and `rdm-smoke codex-coexistence`.
2. The JSON `instrument` field names the Rust command (`rdm-measure refuter-severity`, `rdm-measure mine-refuter-corpus`, `rdm-measure refuter-agreement`). Everything else in the report, miner and results schemas is unchanged.
3. `refuter-agreement --dispatch-stub <module>` is removed (an injected JS module is a JS extension point); `--claude-bin <path>` (default `claude` on `PATH`) replaces it.
4. Coexistence positionals `<rdm> <codex> [auth.json]` → `--rdm`/`--codex`/`--copy-auth-from`; timeouts are `--discovery-timeout-secs` (30) and `--exec-timeout-secs` (120); exit codes 0/1/124/130/143. Children also lose `GIT_DIR`-style repository variables, so the private `git init` works when invoked from a git hook.
5. Argument errors still exit 1, but clap generates the wording (it always names the flag). A value starting with `-` is refused except for `--project-slug` (slugs start with `-`), which still refuses a `--flag`.
6. `--review-lib <path>` and `--host-timeout-secs` (default 600) are new; `NON_GATING_SEVERITIES` and the always-on `DIMENSIONS` are read from `review.mjs` at run time.
7. `refuter-severity --audit` re-derives the projected drop with the severity set the doc records under `projected.severities` (the JS audit re-read the live constant, which would need a JavaScript runtime).
8. `Date.parse` accepts the ECMAScript date-time forms V8 parses (plus V8's space separator and colon-less offset); V8's legacy fallback formats (`"Jul 24 2026"`) are refused as unparseable.
9. A sidecar missing `runId` no longer crashes the report (the JS crashed; tracked as `measure-lane-tokens-missing-runid-crash`): the run id reads `undefined` and its agents degrade to sidecar-only with a warning.
10. Unreadable-transcript warnings carry the OS error text rather than Node's `EACCES: …` wording.

Experiment retirements: none. The batching arm, the tiering arm,
`refuterFanout` and `determiningFindingRank` are all kept.

### JavaScript left in the tree

`git ls-files '*.js' '*.mjs' '*.cjs'` after this phase lists only: the Claude
workflow code of 2(a); `rdm-devtools/src/workflow_host.mjs` (the binding glue);
the browser assets of 2(e); and the Codex runtime/spike files and their
`.test.mjs` suites, which are out of scope (2(c) ruling; follow-up task
`port-remaining-codex-runtime-process-suites`). This phase added no `.js`/`.mjs`
file, and no JavaScript source is embedded in a Rust string.

### Timings (recorded evidence, not asserted)

Host: macOS (Darwin 27.0.0, arm64), 2026-09-24, warm build.

| Suite | Wall | Notes |
|---|---|---|
| `cargo nextest run -p rdm-devtools` | 11.90 s, 9.84 s (nextest summary 9.70 s, 9.63 s) | 149 tests (+1 skipped); the phase-1 process tests dominate |
| the four new integration binaries | summary 1.80 s | 74 tests (+1 ignored) |
| in-crate unit tests | summary 0.06 s | 40 tests |

Slowest new tests: `concurrency_does_not_change_output_order` 1.45 s (its fake
`claude` sleeps up to 240 ms per trial, serially and at `--concurrency 4`),
`discovery_timeout_kills_and_reaps_app_server` and
`exec_timeout_kills_group_and_removes_auth_copy` 1.3 s each (1 s deadlines);
everything else is under 0.35 s.

## 8. Phase 5: distribution and CLI shell verification

The ten phase-5 harnesses and the three shell helpers that served only them are
deleted: `scripts/verify-{agent-config-distribution,plugin-distribution,plugin-install,plugin-loop,claude-code-web-loop,rdm-plan-fixture,golden-json,backlog-groom-loop,review-revision-loop,worktree-review-loop}.sh`,
`scripts/lib/rdm-plan-fixture.sh`, `scripts/lib/golden-capture.sh` and
`scripts/capture-golden.sh`. Rust owns every fixture, command sequence,
assertion and teardown. The only scripts a test executes are products, each
from a scratch or sandbox context: `scripts/gen-codex-skills.sh` (a scratch
copy), `scripts/rdm-dev.sh` (in place; it writes nothing) and
`templates/claude-code-web/.claude/hooks/SessionStart.sh` (sandbox `HOME`). The
live observers `scripts/observe-{plugin-install,workflow-listing}.sh` are
untouched and stay distinct from the hermetic tests.

### Layout

| Binary | Modules | Tests | Needs Node | Filter |
|---|---|---|---|---|
| `rdm-cli/tests/distribution/` | `support`, `claude_skills`, `superseded`, `plugin`, `codex`, `downstream` | 29 | `downstream` only (through `rdm_devtools::workflow`; a missing Node is an actionable error, never a skip) | `cargo nextest run -p rdm-cli --test distribution` |
| `rdm-cli/tests/cli_loops/` | `plugin_loop`, `claude_code_web`, `backlog_groom`, `review_revision`, `worktree_review` | 7 | none | `cargo nextest run -p rdm-cli --test cli_loops` |
| `rdm-cli/tests/golden_json/` | `capture`, `redact` | 5 (+1 ignored: `bless`) | none | `cargo nextest run -p rdm-cli --test golden_json` |

Shared support: `rdm-cli/tests/common/seeded_plan.rs` (new; the port of
`rdm-plan-fixture.sh`: project `fixture-proj`, `sample-roadmap` with three
phases done/in-progress/not-started, `fixture-task-{open,active,done}`, one
seed commit, an optional code repo at `<TempDir>/code`, identity
`fixture-bot <fixture@example.invalid>`, a failed step naming its command);
`plan_fixture::Sandbox` (new); `workflow_support::{Lib::at, Lib::mutant_at}`
(new) and `run_real`/`run_mutant`/`mutant_verdict` generalized to closures;
`rdm_devtools::workflow::invert_helper_source` (new, tested by
`workflow_host::{invert_helper_source_round_trips_the_real_engine,
invert_helper_source_rejects_a_tampered_transform}`). `.config/nextest.toml`
adds `binary(distribution)` to the workflow slow-timeout override; `rdm-cli`
gains the dev-dependency `regex` (already in `Cargo.lock`).

### Isolation rules

- Every process runs under `plan_fixture::Sandbox`: temp `HOME` and
  `XDG_{CONFIG,DATA,STATE}_HOME` under the test's `TempDir`; every inherited
  `RDM_*`, `CODEX_HOME`, `CLAUDE_CONFIG_DIR` and `CLAUDE_CODE_SESSION_ID`
  removed; `GIT_CONFIG_GLOBAL`/`GIT_CONFIG_SYSTEM=/dev/null`; a fixed git
  identity. Tests never call `std::env::set_var`.
- Every emission goes to `--out <TempDir>`; every `--user` emission lands in
  the sandbox `HOME`.
- Generators run only from a scratch copy: `gen-codex-skills.sh` and
  `docs/principles.md` are copied to `<TempDir>/scratch/{scripts,docs}` and run
  with `cwd` there, a fake `cargo` first on `PATH` and `PATH` otherwise
  `/usr/bin:/bin`. The script derives its output root from its own location, so
  it can only write into scratch; the test also asserts the recorded
  `--manifest-path` is the scratch copy and that scratch holds nothing else.
- No test writes into the source checkout; the one exception is the ignored
  `golden_json::bless`, run deliberately.
- The checkout-wide `git status` before/after check (§0/§8, §1/§7, §1/§8 of the
  three distribution harnesses) is dropped: it is racy under parallel nextest
  and would red-light a developer's own edits. It is replaced by construction
  (the rules above) and by per-test output-location assertions
  (`claude_skills::emitted_skills_and_agent_have_structured_frontmatter`,
  `plugin::emitted_plugin_layout_and_manifest`,
  `codex::local_skills_match_gen_codex_skills_output`,
  `plugin::rejection_messages_are_pairwise_distinct`).
- Commands a test executes run with `Run::bare()` (`PATH=/usr/bin:/bin`) plus
  explicit additions, and the downstream plan repo's default project is a
  decoy, so a dropped `rdmBin` or `--project` fails by behaviour.

### The fake `cargo`

`gen-codex-skills.sh` and `rdm-dev.sh` end in `cargo run … -- <args>`. The plan
called for a `cargo` role in `rdm-devtools-fixture`; implemented, it turned out
to be unreachable from `rdm-cli`'s tests, because Cargo exposes
`CARGO_BIN_EXE_*` only for the test's own package, and locating the fixture
next to `CARGO_BIN_EXE_rdm` would be a hard-coded target layout that can also
run a stale binary under `cargo test -p rdm-cli`. `codex.rs` instead writes a
two-line exec stub into the test's temp tree that records the call (cwd and
argv) and execs `CARGO_BIN_EXE_rdm` on the arguments after `--`. It holds no
assertions; the role was not kept, since nothing would use it.

### Deleted-template fixture strategy (§5j/§5k)

Provenance-documented fixtures; no test reads git history, so a shallow clone
runs them. The cleanup is fingerprint-gated (`SUPERSEDED_WORKFLOWS` in
`rdm-core/src/agent_config.rs`), so one genuine historical body per retired
name is committed under `tests/fixtures/superseded-workflows/`, extracted once
with `git show <commit>:<path>`:

| Fixture | Source commit:path | Bytes | sha256 |
|---|---|---|---|
| `dispatch-phase.js.body` | `0fb2bcaf07c3f5a05aa753a28eaf66833760a452:rdm-core/src/templates/workflows/dispatch-phase.js` | 80823 | `e7644f1718c9f6690cd8136bbf668c26cc19fd7b6a2a93fd97add25a27604522` |
| `review-refute-fix.js.body` | `0fb2bcaf07c3f5a05aa753a28eaf66833760a452:rdm-core/src/templates/workflows/review-refute-fix.js` | 41803 | `032ffd9ea22dee0eeef54ea8433b9bf25955174bf564094e65f33e80ba72229a` |
| `autopilot.js.body` | `0fb2bcaf07c3f5a05aa753a28eaf66833760a452:rdm-core/src/templates/workflows/autopilot.js` | 35572 | `817c8ecde65ba7d614620927cc86bc9743b10511b4a85e3249ede7d0324be7e4` |
| `rdm-wf-dispatch-phase.js.body` | `0868c2a10c3fd2ca3c31c33248db0813e04fd1ff:rdm-core/src/templates/workflows/rdm-wf-dispatch-phase.js` | 170545 | `e6b506a97a54504aee6194aa7f381d8524d1a56100ccdc11359e4e71617dbd75` |

This table is documentation, not a test: no test re-checks a fixture against
`SUPERSEDED_WORKFLOWS` (that would be a guard over something else still being
true). A fixture that were not a genuine body would survive the re-emit and
fail the removal assertion, and `superseded::a_tampered_superseded_body_survives_the_cleanup`
plants a genuine body with one byte appended under a retired name and requires
it to survive (fingerprint gating, not name gating). A new
`SUPERSEDED_WORKFLOWS` entry needs its body added here the same way.

### Downstream evidence (§7)

`distribution::downstream` builds one foreign fixture (`Downstream::new`): a
Python/TypeScript repo on `feature/checkout`, its own plan repo (default
project a decoy, project `acme-web`, roadmap `checkout-revamp` with
`checkout-form`/`order-summary`, task `tidy-cli`), its own rdm path
`tools/acme-rdm` (a symlink to `CARGO_BIN_EXE_rdm`), and the lane that binary
emits into the repo. Three layers, kept apart:

1. **Helper extraction that bypasses the driver**: the emitted engine's helpers
   are compiled out of the emitted bytes (`helper_source`, proven byte-exact by
   `invert_helper_source`); the raw emitted file fails to import with a
   `SyntaxError`; the resolver, the environment guards and the persist ladders
   the helpers build are executed, the ladders against the fixture plan under
   the bare `PATH`.
2. **Mocked-driver execution**: the emitted engine's driver runs under a
   Rust-scripted clean fleet with item identity `{ task }` and the pinned source
   fields, `persist: true`; its `persistScript` runs against the fixture plan
   and the review is read back. Two additions to the documented shape come from
   the real binary, not the engine: the pinned source is the task's
   `rdm worktree add` checkout (a change review binds to a registered
   checkout), and `implements` names a plan seeded for the task (a change
   review records the plan it implements).
3. **Planted corruptions in the emitted bytes**: mutants A (binary literal), B
   (project literal) and C (reviewer selection ignored), each through
   `Lib::mutant_at` and each caught by a failed check or a JavaScript throw,
   never by an infrastructure failure.

These are component tests: real emitted JavaScript under Node with a fake host,
real rdm and real git. No Claude host is observed;
`scripts/observe-workflow-listing.sh` remains the live observer.

### Golden JSON

`golden_json::capture_matches_committed_goldens` reproduces all 24 committed
`tests/golden/*.json` byte for byte (they are unchanged in this phase), naming
every drifted, missing or uncaptured file and the bless command. The six
redaction rules are ported to `regex` over the raw text. Re-bless:
`cargo nextest run -p rdm-cli --test golden_json --run-ignored only -E 'test(=bless)'`.

### Case map

P = ported (named test), D = duplicate of an existing test, R = retired (with
reason). Totals: **P 50, D 9, R 20** (rows counted as grouped below).

**verify-agent-config-distribution.sh** (P 15 / D 4 / R 11)

| § | Disposition |
|---|---|
| 0/8 checkout status unchanged | P: by construction (isolation rules) plus per-test output-location assertions |
| 1 emit | P: fixture of the `claude_skills::*` tests |
| 2 inventory + frontmatter | P `claude_skills::emitted_skills_and_agent_have_structured_frontmatter` |
| 2b placeholders | R: string-presence check (operator no-grep rule) |
| 3 workflow byte identity | D `cli_agent_config::agent_config_workflows_are_byte_identical_to_source`, rdm-core `generate_workflows_are_byte_identical_to_source` |
| 3a agent byte identity | D `cli_agent_config::agent_config_agents_are_byte_identical_to_source` |
| 3b re-emit idempotent | P `claude_skills::reemit_is_idempotent_and_spares_user_files` |
| 3c (i–iii) agentType resolution | R: guards a reference set that is empty today (no emitted workflow passes `agentType`), and its sweep is a grep over emitted JS |
| 4 shim/invocation references | R: prose grep over emitted skill text |
| 5a byte-corruption self-test | R: harness-internal self-test; Rust `assert_eq!` cannot be vacuous |
| 5b, 5c, 5d, 5e, 5f | R: self-tests of the retired §2b/§4 greps |
| 5g dogfood `.claude/skills` set | R: guard over the deliberately divergent local lane; the shipped inventory is §2 |
| 5j | P `superseded::skills_reemit_removes_fingerprinted_orphans`; negative control `superseded::a_tampered_superseded_body_survives_the_cleanup` replaces the re-plant self-test |
| 5k | P `superseded::plugin_reemit_removes_fingerprinted_orphans` (seeds all four bodies, not just the retired dispatch engine) |
| 6a, 6b | P `claude_skills::pi_and_user_emissions_write_no_runtime_and_run_no_cleanup` (directory absence also D `agent_config_pi_skills_does_not_write_workflows`/`agent_config_user_skills_does_not_write_workflows`) |
| 6c | P `claude_skills::skills_emission_ignores_a_missing_rdm_root` |
| 7a fixture | P `downstream::Downstream::new` (the docs-only/CHANGELOG-only branches and the Rust-token scan served the retired `deriveSignals`/`selectDimensions`) |
| 7b | P `downstream::{emitted_engine_helpers_round_trip_to_the_emitted_bytes, raw_emitted_engine_does_not_import}`; its shim/invocation re-checks R (no-grep) |
| 7c | P `downstream::emitted_resolvers_and_env_guards_execute`; the prompt-text tokenizer R (prompt-text grep), replaced by execution under the bare `PATH` and decoy project |
| 7d | P `downstream::{emitted_persist_ladders_run_against_the_foreign_plan, emitted_engine_driver_persists_through_the_foreign_fixture}`; the dropped-`--verdict` string-edit control R, replaced by mutants A and B |
| 7e A/B/C | P `downstream::{mutant_a_hardcoded_binary_is_caught, mutant_b_hardcoded_project_is_caught, mutant_c_ignored_reviewer_selection_is_caught}` |
| 7g/7h emitted instruction text | R: prose greps; the quoted-flag acceptance half is D `cli_commit.rs` (`--changeset`, `--all`), `cli_status.rs` (`status --all`), `cli_session.rs` (`session id/list/journal`) |
| 7i Codex distribution | D `cli_agent_config::codex_{project_emits_instructions_and_supported_skills,user_paths_separate_config_and_skills,plugin_rejection_names_the_supported_channel}`; P `codex::{every_withheld_skill_is_named_in_the_notice, emitted_commands_execute_against_a_foreign_plan, local_skills_match_gen_codex_skills_output}` |
| 7i `rdm-dev.sh` | P `codex::{dev_wrapper_runs_from_a_foreign_cwd_and_keeps_the_session, dev_wrapper_refuses_missing_session_or_plan_repo}` |
| 7i codex-smoke-process / coexistence arm | D: phase 1 `rdm-devtools` lifecycle tests; phase 4 `codex_coexistence.rs` plus the ignored `codex_coexistence_live` |

**verify-plugin-distribution.sh** (P 4 / D 1 / R 3)

| § | Disposition |
|---|---|
| 1/1b/7 | P: by construction |
| 2 | P `plugin::emitted_plugin_layout_and_manifest` |
| 3 naming transform | P `plugin::naming_transform_relates_to_the_skills_emission` (both sides from real emissions) |
| 4 `rdm:<engine>` refs | R: prose grep |
| 5a–5d, 5f | D `cli_agent_config::agent_config_plugin_{and_skills_conflict,requires_out,and_user_rejected_with_distinct_message,rejected_on_agents_md,rejected_on_cursor,rejected_on_copilot,rejected_on_pi}`, `agent_config_skills_and_user_still_works_unaffected_by_plugin` |
| 5g distinct messages | P `plugin::rejection_messages_are_pairwise_distinct` (the `--skills` destination error is read from the CLI, not hard-coded) |
| 6a–6d | R: harness-internal checker self-tests (6b/6c test a retired grep) |
| 6e | R: self-test of a comparator Rust expresses as `assert_ne!` |

**verify-plugin-install.sh** (P 7 / D 0 / R 1)

| § | Disposition |
|---|---|
| 1/1b/8 | P: by construction (fresh emission, every `RDM_*` removed, no `--project`) |
| 2 drift, version-normalized | P `plugin::checked_in_tree_matches_the_generator` (full recursive set-and-byte comparison; only the manifest `version` value, read from parsed JSON, is normalized) |
| 3 version | P `plugin::fresh_manifest_version_is_the_crate_version` |
| 4/4b marketplace | P `plugin::marketplace_shape_and_sources_resolve` |
| 5/6 workflow identity + skill inventory | P: §2's set-and-byte drift plus `plugin::skill_frontmatter_names_match_their_dirs` |
| 7a–7c, 7k | P `plugin::marketplace_checker_rejects_planted_corruptions` |
| 7g–7j | P `plugin::drift_normalization_is_surgical` |
| 7d–7f, 7l | R: floor/vacuity self-tests, superseded by exact set equality against a fresh emission |

**verify-plugin-loop.sh** (P 3 / D 1 / R 0)

| § | Disposition |
|---|---|
| 1 | P `plugin_loop::editor_round_trip` |
| 2 | D `cli_task::{body_flag_beats_stdin, task_update_body_flag_beats_stdin, task_update_tags_ignores_stdin, task_update_status_ignores_stdin, task_update_empty_body_refuses_clobber, task_update_clear_body_succeeds}` (each checked: same property, including the `--clear-body` pointer and the untouched body) |
| 3 | P `plugin_loop::create_waits_for_stdin_eof` |
| 4 | P `plugin_loop::errors_and_warnings_stay_on_stderr` |

**verify-claude-code-web-loop.sh** (P 1 / D 1 / R 0)

| § | Disposition |
|---|---|
| steps 1–7 | P `claude_code_web::session_start_bootstraps_and_done_line_completes_the_phase` |
| step 8 | D `cli_review::pending_scopes_to_current_branch_and_fails_open`, plus `worktree_review::roadmap_worktrees_scope_pending_and_restamp` |

**verify-rdm-plan-fixture.sh** (P 0 / D 2 / R 3) — it tested shell test support only.

| § | Disposition |
|---|---|
| AC1, AC1b, AC1c | R: the shell library is deleted; `common/seeded_plan.rs` replaces it |
| AC1d, AC3 | R: isolation holds by construction (explicit `--root`, `hermetic_removals`, `Sandbox`); AC3's static grep is also a no-grep violation |
| AC1e | R: a failed seed step returns a `Failure` naming the command |
| AC2 | D `golden_json::same_day_captures_are_identical_and_leak_no_temp_path` |
| AC4 | D: CI shellcheck/shfmt (and the linted file is deleted) |

**verify-golden-json.sh** (P 4 / D 0 / R 1)

| § | Disposition |
|---|---|
| 1 | P `golden_json::capture_matches_committed_goldens` |
| 2 | P `golden_json::same_day_captures_are_identical_and_leak_no_temp_path` |
| 2b | P `golden_json::digest_fields_are_redacted_not_frozen` |
| 3 | P `golden_json::{comparator_names_a_mutated_golden, redactor_redacts_a_planted_digest_and_every_volatile_field}` |
| 4 heal after self-test | R: harness-internal; no Rust test mutates the real goldens |

**verify-backlog-groom-loop.sh** (P 6 / D 0 / R 0): setup and §1–5 are one
sequential scenario, `backlog_groom::groom_loop_end_to_end`, reading parsed
`--format json` fields; the four confirmation lines stay CLI-stdout checks.

**verify-review-revision-loop.sh** (P 6 / D 0 / R 0): setup and A–E are one
sequential scenario, `review_revision::revision_loop_end_to_end`. Fidelity
fix: the shell read the plan HEAD before committing, so every
`applied_commit` it recorded was the seed commit; the port commits each body
edit and asserts each comment records that edit's own commit.

**verify-worktree-review-loop.sh** (P 4 / D 0 / R 1)

| § | Disposition |
|---|---|
| setup, B, C, D | P `worktree_review::roadmap_worktrees_scope_pending_and_restamp` |
| A | R: models the retired Pi `agent_end` review-on-finalize extension (unify-code-review phases 6–7); B covers the live scoping |

### Timings (recorded evidence, not asserted)

Host: macOS 27.0 (Darwin 27.0.0, arm64, Apple M5 Max, 18 cores), 2026-09-24,
warm build, a shared machine under concurrent load (so run-to-run variance is
high).

Before (the shells, `sh scripts/verify-<name>.sh` with `target/debug/rdm`
built; three warm runs each):

| Harness | Wall (s) |
|---|---|
| verify-agent-config-distribution.sh (with Node) | 3.15, 3.06, 3.05 |
| verify-plugin-distribution.sh | 0.78, 0.36, 0.36 |
| verify-plugin-install.sh | 1.15, 1.16, 1.15 |
| verify-plugin-loop.sh | 2.64, 2.63, 2.63 |
| verify-claude-code-web-loop.sh | 0.52, 0.51, 0.51 |
| verify-rdm-plan-fixture.sh | 2.25, 2.21, 2.23 |
| verify-golden-json.sh | 2.04, 2.06, 2.05 |
| verify-backlog-groom-loop.sh | 0.44, 0.44, 0.44 |
| verify-review-revision-loop.sh | 0.81, 0.81, 0.81 |
| verify-worktree-review-loop.sh | 0.50, 0.50, 0.50 |
| **sum (one run each, serial)** | **≈ 14.3** |

After (nextest summary time, three warm runs, then `--test-threads 1` once):

| Binary | Parallel (s) | Serial (s) |
|---|---|---|
| `distribution` (29 tests) | 1.26, 1.67, 3.99 | 7.74 |
| `cli_loops` (7 tests) | 1.82, 2.65, 2.77 | 6.00 |
| `golden_json` (5 tests, 1 ignored) | 1.66, 5.17, 2.73 | 2.48 |
| `rdm-devtools --test workflow_host` (14 tests, 2 new) | 0.82, 0.82, 0.82 | — |

nextest intermittently reports a test as `LEAK` on this host, including
`golden_json::comparator_names_a_mutated_golden`, which spawns no process at
all; it is host noise, not a leaked child.

## 9. Phase 6: session, commit and race harnesses

`scripts/verify-{session-identity,scoped-commit,lost-update,journal-truncation-race}.sh`
are deleted. Every behavioural section is a Rust test in the `rdm-cli` nextest
binary `concurrency` (`rdm-cli/tests/concurrency/`), or a demonstrated
duplicate of an existing test. The tests run real, separate `rdm` processes —
`CARGO_BIN_EXE_rdm`, or an isolated mutant build of it — against per-test temp
plan repos. Rust owns every fixture, spawn, barrier release, wait, assertion
and teardown; `sh` appears only as the process topology under test (the
long-lived parent that anchors a rung-2 lease, and the per-call wrapper). The
scripts had no helper libraries and no caller but CI's `scripts/verify-*.sh`
loop, which keeps running the two remaining scripts, so `ci.yml` and
`CLAUDE.md` are unchanged.

### Layout

| Module | Owns | Tests |
|---|---|---|
| `support` | `Plan` (repo + `Sandbox` + binary), `Proc` (guarded child), `Parked`, `ShellDriver`/`Driver`, readers | — |
| `mutant` | working-tree mirror, anchored edits, locked isolated builds | — |
| `session_identity` | rung 1–3, leases, § G/H/H2, § J, § K; scenario functions shared with `mutants` | 13 |
| `journal_race` | the fan-out and the four journal interleavings; scenario functions | 6 |
| `lost_update` | the flush and commit content checks; scenario functions | 7 |
| `scoped_commit` | session-scoped commit/status/discard | 7 |
| `mutants` | one negative control per planted regression | 10 |

Filter: `cargo nextest run -p rdm-cli --test concurrency`. No new crate
dependency: the build lock is `std::fs::File::lock` (stable since 1.89; MSRV
1.94). One existing test gained an assertion:
`cli_commit::the_owning_changeset_lands_its_manifest_and_nothing_else` now also
requires `rdm status --all` to report no changed path after both commits (the
convergence half of the scoped-commit harness's § I).

### Isolation rules

- `git_test_support::REPO_REDIRECT_VARS` (new) names every git variable that
  would redirect a child away from the repo a fixture names: `GIT_DIR`,
  `GIT_WORK_TREE`, `GIT_INDEX_FILE`, `GIT_COMMON_DIR`, `GIT_OBJECT_DIRECTORY`,
  `GIT_ALTERNATE_OBJECT_DIRECTORIES`, `GIT_PREFIX`, `GIT_NAMESPACE`,
  `GIT_CONFIG`, `GIT_CONFIG_PARAMETERS`, `GIT_CONFIG_COUNT`;
  `repo_redirect_removals()` adds any inherited numbered
  `GIT_CONFIG_KEY_<n>`/`GIT_CONFIG_VALUE_<n>`. `git_test_support::git` (which
  removed only the first three) and `plan_fixture::hermetic_removals` — hence
  `Sandbox::removals` — now remove the full list, strengthening every binary
  that uses them. `Sandbox::removals` also removes `CLAUDE_SESSION_ID` (it is
  in `HARNESS_SESSION_VARS`; an inherited value would silently move a rung-2
  case to rung 3).
- Every `rdm`, `git` and `sh` child is built from the `Sandbox`. Tests never
  mutate their own environment. Driver scripts run with `PATH=/usr/bin:/bin`
  and name the binary by absolute path, so no ambient `rdm` can resolve.
- Every test has its own `Plan` (its own temp repo and sandbox) and
  test-unique session ids. Invocations pin `RDM_SESSION` unless a test
  deliberately takes rung 2 from its own process (`Plan::run_bare`): a bare call
  leases the test's pid in that repo, and every later driver in the repo would
  ascend to it. The only shared identities (the journal tests'
  `shared-changeset`, lost-update § 2b's common parent, § J's planted ancestor
  lease) exist inside the one test about them.
- `Proc` sends output to files, stays in the test's process group, and kills
  and reaps its child on drop, so every assertion or panic path tears parked
  children down; scenario structs own finished results, never live children.
  Confirmed on the pinned nextest (0.9.130) with a deliberately hung test in a
  scratch crate: on `terminate-after` nextest killed the test, a `sleep 300`
  child it had spawned, and a `sh -c 'sleep 300 & wait'` grandchild.
- Waits are events with deadlines — a readiness file, a product-written file,
  a driver's step-status file, or a process exit — never a sleep taken as proof.
  The shells' "still alive after N sleeps" park checks and journal § 6's
  `sleep 1` are gone.
- Mutant state lives under `$CARGO_TARGET_TMPDIR/rdm-mutants/<family>/` and
  the checkout is only read (below).

### Barrier readiness (the one production change)

The four seams (`RDM_HARNESS_FLUSH_BARRIER` in `rdm-store-fs`;
`RDM_HARNESS_JOURNAL_BARRIER`, `RDM_HARNESS_APPEND_BARRIER`,
`RDM_HARNESS_COMPACT_BARRIER` in `rdm-core/src/session/journal.rs`) now share
one documented `pub fn rdm_core::session::harness_barrier(var)`, with the 60 s
ceiling kept (`HARNESS_BARRIER_CEILING`). When `var` names a non-empty path
`<m>`, it first creates the empty sibling `<m>.parked`
(`HARNESS_BARRIER_PARKED_SUFFIX`; best effort — a failed write changes
nothing) and then polls for `<m>` exactly as before. `journal.rs` imports it
under the same name, so its call sites and the mutant bodies are unchanged;
`FsStore`'s private barrier calls it with `HARNESS_FLUSH_BARRIER`, removing the
duplicate loop. The append and compaction seams park with the journal lock
already held (shared; exclusive, after the compare-and-swap), so there the
readiness file also means "the lock is held". No new variable, no bypass,
inert when unset. The raw-write audit
(`rdm-core/tests/no_raw_fs_write_audit.rs`) gained a reasoned allowlist entry
for the readiness write. No unit test: the seam reads the process environment,
which tests may not mutate, and every parked scenario exercises it end to end —
a broken readiness file fails them at the 30 s park deadline.

### Mutant builds

Nine planted regressions, three builds (the shells built six, paying a cold
dependency graph for most of them on every run):

| Family | Edits (anchored, exactly once) | Negative controls |
|---|---|---|
| `session` | `session/mod.rs`: `if let Some((var, raw)) = active_harness_var(env) {` → `… && false {`; `continuity_advisory`'s `Some(lines.join("\n"))` → `let _ = lines; None`; `session/lease.rs`: the create-path `gc(paths, procs);` removed | `mutants::session_harness_check_removed_merges_children_onto_the_ancestor_lease` (§ J), `mutants::session_advisory_silenced_drops_the_remedy` (§ K self-test 1), `mutants::session_create_sweep_removed_leaks_leases` (§ K self-test 2) |
| `journal` | `journal.rs`: `truncate` body → the pre-fix read-modify-write body with the barrier inside the read → write window; `LockMode::Shared => file.try_lock_shared(),` and `LockMode::Exclusive => file.try_lock(),` → `Ok(())`; `discard_changeset` body → barrier + bare `remove_file` (bodies as in the shell) | `mutants::journal_rmw_truncate_strands_the_parked_fanout` (§ 1b), `mutants::journal_rmw_truncate_loses_appends_made_while_parked` (§ 2b), `mutants::journal_lockless_gc_rewrites_under_a_parked_append` (§ 5b), `mutants::journal_lockless_append_lands_in_the_doomed_inode` (§ 6b), `mutants::journal_unlinking_discard_loses_a_concurrent_append` (§ 7b) |
| `lost-update` | `rdm-store-fs/src/lib.rs`: `if &current != baseline {` → `if false && &current != baseline {`; `rdm-store-git/src/commit.rs`: `self.root.join(path).exists()` → `false` | `mutants::lost_update_neutered_flush_check_loses_the_update` (§ 2c), `mutants::lost_update_short_circuited_delete_guard_destroys_the_recreated_file` (§ 6c) |

**Attribution.** Within a family each negative scenario exercises exactly one
edit; the others are inert there:

- `session_harness_check_removed_…` sets a harness variable and makes no commit
  (the advisory is irrelevant), and its children adopt the planted lease rather
  than creating one (the sweep is irrelevant).
- `session_advisory_silenced_…` checks only commit output; it sets no harness
  variable and counts no lease.
- `session_create_sweep_removed_…` sets no harness variable and makes no
  commit.
- `lost_update_neutered_flush_check_…` commits nothing, so the delete guard is
  never consulted.
- `lost_update_short_circuited_delete_guard_…`: B's recreate flushes over an
  absent/absent baseline, so the neutered flush check changes nothing.
- The journal family repeats the shell's own combination: every journal control
  was already one shared build there.

Each control also asserts its "inconclusive" guards, as the shells did — the
processes it relies on exited 0, the mutant's gc really compacted — so it can
fail as not-run but never pass without exercising its regression.

**Mechanics** (`mutant.rs`).

1. *Mirror.* The source list is `git ls-files -z --cached --others
   --exclude-standard`, run read-only in the checkout (`GIT_OPTIONAL_LOCKS=0`,
   under the sandbox), so uncommitted and untracked-unignored work builds too.
   It is mirrored into `$CARGO_TARGET_TMPDIR/rdm-mutants/<family>/src`, writing
   a file only when its bytes differ (mtimes preserved, so cargo rebuilds
   incrementally) and deleting mirror files no longer listed. The checkout is
   never written, not even transiently.
2. *Edits* are applied in memory before the write: `Replace` requires exactly
   one occurrence; `FnBody` anchors on a column-0 signature prefix occurring
   once and replaces the body up to the next line that is exactly `}`. A miss is
   `Failure::Infra` naming the family and the edit, so the control fails as
   not-run. This is fixture construction, like `MutantTree::replace_once`, not
   an assertion about source text. The inventory's phase-1 suggestion, a
   cfg-gated fault seam, is rejected: it would put test-only alternate
   implementations in production source.
3. *Build.* Under an exclusive `File::lock` on `<family>/build.lock`:
   `$CARGO build -p rdm-cli --bin rdm --frozen
   --message-format=json-render-diagnostics` in the mirror, with
   `CARGO_TARGET_DIR=<family>/target`, `RUSTFLAGS=""` (a planted edit may warn;
   the caller's `-D warnings` must not turn that into a not-run control),
   `CARGO_PROFILE_DEV_DEBUG=0` (added in implementation: debuginfo was most of
   each family's 1.1 GB target; without it a family is ~680 MB and builds ~1 s
   faster; a mutant is only executed), and every `RDM_*`, repo-redirecting git
   variable, `CARGO_ENCODED_RUSTFLAGS`, `CARGO_BUILD_RUSTFLAGS`,
   `CARGO_BUILD_TARGET_DIR` and jobserver variable removed; `HOME`/`CARGO_HOME`
   are kept for the offline registry. The binary path is read from the
   `compiler-artifact` message's `executable` field and, still under the lock,
   hard-linked (copied if linking fails) into the test's `TempDir`, so a later
   rebuild cannot swap the inode under a running test.
4. *Sharing.* Per-family target dirs, because the uplifted `rdm` path would
   collide across families. The lock serialises concurrent runs, including two
   nextest runs in one checkout and plain `cargo test` threads (verified:
   `cargo test -p rdm-cli --test concurrency` passes, all 43 tests, 2.2 s warm).

**The `rdm-mutant-builds` group** (`.config/nextest.toml`, `max-threads = 1`,
`slow-timeout = 60s × 10`; the rest of the binary gets `30s × 4`). Reason,
recorded in the TOML comment too: a family build is a full `rdm-cli` cargo
build, so several at once multiply peak CPU and memory against the rest of the
parallel suite, and group members would otherwise only block on the family
lock while holding nextest slots and burning their slow-timeout. Everything
else in the binary stays fully parallel. Measured: a cold family build is
8.5–8.6 s here (below), and the serialised group adds ~5 s warm (10 controls ×
~0.6 s) to the binary's wall time.

**The controls stay in the default `cargo nextest run`**, so CI's existing
`Test` step keeps them required with no new profile or job. Cost: one cold
build per family per fresh target dir, ~0.1 s when warm — which does not
justify a separate profile. On CI the cold cost is not measured here; the
shells' own record (~82 s of CPU for one cold build on a 2-core runner)
suggests roughly 1–1.5 min per family, 3–4 min serialised, against the ~130 s
the four shells cost on every run regardless of cache. Disk: ~2 GB for the three
target dirs.

### Case map

P = ported (named test in `concurrency` unless another binary is named),
D = demonstrated duplicate of an existing test, R = retired (reason). Totals:
**P 46, D 12, R 9** — P counts section rows, and one test can cover several.
That is 33 default-run tests and 10 negative controls, 43 in all.

**verify-session-identity.sh** (P 19 / D 2 / R 1)

| § | Disposition |
|---|---|
| A, B | P `session_identity::bare_shells_are_stable_within_and_distinct_between` (two concurrent bare drivers × 3 direct `session id`: stable within, distinct between, all rung 2, exactly two `<driver pid>.lease` files each recording its driver's id; the shell's A was already bare, so A and B are one scenario) |
| B (degenerate parent) | P `session_identity::a_direct_invocation_still_resolves` |
| C | P `session_identity::harness_variable_is_leaseless_rung3_and_rdm_session_wins` |
| D | P `session_identity::stale_lease_on_a_live_parent_is_neither_adopted_nor_kept` (a gated driver; Rust plants `<driver pid>.lease` with a bogus `start_time`, then opens the gate) |
| E | D `cli_session::journal_lists_exactly_the_mutations_paths` (whole-set equality; the property is id-agnostic) |
| F | D `cli_session::concurrent_journals_never_contain_each_others_paths`; rung-2 identity scoping itself is A/B |
| G | P `session_identity::whole_tree_commit_never_sweeps_session_state` |
| G self-test (decoy `.jsonl`) | R: a vacuity self-test of a shell `grep`; Rust asserts over a parsed tree listing |
| H | P `session_identity::resolution_cost_stays_far_under_the_hook_deadline` (the 250 000 µs bound holds under the normal parallel run) |
| H2 | P `session_identity::hook_post_commit_resolves_identity_and_lands_its_batch` |
| J (a–d) | P `session_identity::harness_id_beats_an_inherited_ancestor_lease` |
| J self-test | P `mutants::session_harness_check_removed_merges_children_onto_the_ancestor_lease` |
| K0 | P, folded into K1 as its precondition (wrapper pids pairwise distinct, every wrapper's parent the one driver) |
| K1 | P `session_identity::wrapper_calls_fragment_into_bootstrapped_rung2_changesets` |
| K2 | P `session_identity::fragmented_commit_names_cause_and_remedy_and_stays_recoverable` |
| K3, K3b | P `session_identity::dead_wrapper_leases_are_swept_and_bounded` (7 wrapped calls in its own repo; the observed lease's pid is one of that finished driver's recorded wrapper pids, so it is dead by construction) |
| K4 | P `session_identity::concurrent_wrapper_drivers_never_share_an_id` |
| K5 | P `session_identity::harness_adoption_var_gives_wrappers_one_changeset` |
| K self-test 1 | P `mutants::session_advisory_silenced_drops_the_remedy` |
| K self-test 2 | P `mutants::session_create_sweep_removed_leaks_leases` |

**verify-journal-truncation-race.sh** (P 10 / D 1 / R 1)

| § | Disposition |
|---|---|
| 1 | P `journal_race::stress_fanout_strands_nothing` (all 40 tasks at HEAD, porcelain empty immediately, empty fold, `session list` omits the changeset, clean no-op final commit) |
| 1b | P `journal_race::parked_commit_fanout_keeps_every_record` and `mutants::journal_rmw_truncate_strands_the_parked_fanout` |
| 2 | P `journal_race::records_appended_during_a_parked_truncate_survive` |
| 2b | P `mutants::journal_rmw_truncate_loses_appends_made_while_parked` |
| 3 | R: planted corruptions of the shell's own `grep`/`git status` checks; the positive-claim reader is exercised by the presence assertions of §§ 2 and 5 |
| 4 | D `cli_session::gc_sweeps_a_journal_whose_changeset_is_fully_committed` and `cli_session::gc_runs_and_reports_without_touching_journals` |
| 5 | P `journal_race::gc_is_excluded_by_a_parked_append` |
| 5b | P `mutants::journal_lockless_gc_rewrites_under_a_parked_append` |
| 6 | P `journal_race::append_waits_out_a_parked_compaction` (the wait before releasing the sweep: B's task file exists — its flush is done, so its append is next — and B is still running, which it cannot stop being while the sweep holds the lock exclusively) |
| 6b | P `mutants::journal_lockless_append_lands_in_the_doomed_inode` (same steps, but the wait before releasing the sweep is B's exit: without the lock B writes blind and exits while the sweep is parked — the old interleaving made deterministic) |
| 7 | P `journal_race::discard_keeps_a_concurrent_append` |
| 7b | P `mutants::journal_unlinking_discard_loses_a_concurrent_append` |

**verify-lost-update.sh** (P 9 / D 1 / R 3)

| § | Disposition |
|---|---|
| 1 | R: already retired 2026-09-23 (static grep); § 6 covers the behaviour |
| 2 | P `lost_update::concurrent_flush_loser_is_refused` |
| 2b | P `lost_update::concurrent_flush_loser_is_refused_without_a_session_id` (both bare from the test process, so they share one rung-2 parent exactly as the shell's `sh -c exec` did) |
| 2c (i) | P `mutants::lost_update_neutered_flush_check_loses_the_update` |
| 2c (ii) | R: a vacuity self-test of the shell's item-name `grep` |
| 3 | P `lost_update::a_sessions_sequential_writes_never_trip_the_check` |
| 3b | D `cli_hook::hook_post_commit_batches_multiple_directives_in_one_message_into_one_commit` |
| 3c | R: a self-test of the shell's exit-status predicate |
| 4 | P `lost_update::commit_refuses_a_path_another_session_overwrote` (control: the same commit on a second repo without the overwrite lands) |
| 5 | P `lost_update::hook_logs_a_refused_flush_and_exits_zero` |
| 6 | P `lost_update::delayed_delete_of_a_recreated_path_is_refused` |
| 6b | P `lost_update::delayed_delete_without_a_recreate_lands` |
| 6c | P `mutants::lost_update_short_circuited_delete_guard_destroys_the_recreated_file` |

**verify-scoped-commit.sh** (P 8 / D 8 / R 4)

| § | Disposition |
|---|---|
| A | P `scoped_commit::explicit_sessions_land_disjoint_exact_commits` |
| B | P `scoped_commit::bare_shells_land_disjoint_exact_commits` (each bare driver is gated between its create and its commit, so the first commit runs while the other shell's work is on disk — stronger than the shell's sequential run) |
| B2 | P `scoped_commit::a_bare_shells_changeset_carries_across_processes` |
| B3 | D `cli_commit::commit_reports_unattributed_dirt_instead_of_sweeping_or_going_quiet`; the `--all` recovery is D `cli_commit::commit_all_is_the_whole_tree_opt_in` |
| C | P `scoped_commit::done_hook_commits_only_its_own_changeset` |
| C self-test | R: a `commit --all` stand-in whose sweep is D `cli_commit::commit_all_is_the_whole_tree_opt_in` |
| E1 | D `cli_commit::init_remote_still_lands_its_config_commit`, `cli_init::init_remote_sets_default_remote` |
| E2 | P `scoped_commit::legacy_repo_commit_carries_only_the_authored_path` |
| E2b | D `cli_status::no_command_touches_the_merge_driver_config` |
| E3 | D `cli_commit::commit_by_changeset_id_is_the_orphan_recovery_path`, `cli_commit::status_defaults_to_the_callers_changeset_and_all_shows_everything`; the HTTP half is `rdm-server/tests/mutation_policy.rs` |
| F | P, folded into A (exact per-commit path sets, B's file byte-identical after A commits); the inherited-`INDEX.md` sub-assert is R — `rdm index` and index generation are retired (`retire-generated-index`), so no mutation can write one |
| F self-test | R: a `commit --all` stand-in, as for C |
| G | P `scoped_commit::scoped_discard_spares_another_sessions_work` |
| G `--all` arm | D `cli_commit::discard_all_is_the_whole_tree_opt_in_and_still_needs_force` |
| G3 | D `cli_commit::discard_leaves_a_path_another_changeset_overwrote_since` |
| G4 | D `cli_commit::discard_leaves_a_path_another_changeset_recreated_after_a_delete` |
| G3/G4 self-tests | R: `discard --all` stand-ins; the unconditional restore is shown by the G `--all` D |
| H | P `scoped_commit::reads_cross_the_session_boundary` |
| I | D `cli_commit::commit_lands_under_a_project_another_changeset_has_not_committed` and `cli_commit::the_owning_changeset_lands_its_manifest_and_nothing_else`, the latter extended with the convergence half (`rdm status --all` reports no changed path after both commits); the dangling-row and orphan-project-index checks are R (retired `INDEX.md`) |
| I self-tests (2) | R: vacuity self-tests of retired index checkers |

### Timings (recorded evidence, not asserted)

Host: macOS 27.0 (Darwin 27.0.0, arm64, Apple M5 Max, 18 cores), 2026-09-24,
a shared machine under concurrent load (1-minute load average 2–21 across the
runs).

Before (the shells, `sh scripts/verify-<name>.sh` with `target/debug/rdm`
built; three runs each; every run rebuilds its mutants cold in a scratch
target):

| Harness | Mutant builds per run | Wall (s) |
|---|---|---|
| verify-lost-update.sh | 2 (separate targets) | 37.17, 37.38, 37.42 |
| verify-session-identity.sh | 3 (one shared target) | 26.36, 26.45, 26.48 |
| verify-scoped-commit.sh | 0 | 2.79, 2.79, 2.79 |
| verify-journal-truncation-race.sh | 1 | 63.84, 63.57, 63.78 |
| **sum (one run each, serial)** | 6 | **≈ 130.2** |

After (`cargo nextest run -p rdm-cli --test concurrency`, nextest summary
time):

| Run | Wall (s) |
|---|---|
| cold mutant cache (`rm -rf target/tmp/rdm-mutants`), ×3 | 32.30, 33.50, 34.08 |
| warm, ×3 | 7.43, 7.30, 7.18 |
| warm, `--test-threads 1` | 14.01 |

Per-family build, as logged by `mutant::build`: cold 8.48–8.60 s (9.55–9.68 s
before debuginfo was dropped); warm no-op (cargo's fresh check plus the
mirror sync) 0.12–0.14 s; after a one-line change to
`rdm-core/src/session/journal.rs` reaches the mirror, 1.51–1.54 s per family.

Five slowest tests (warm): `lost_update::a_sessions_sequential_writes_never_trip_the_check`
1.49 s, `journal_race::gc_is_excluded_by_a_parked_append` 1.36 s,
`journal_race::stress_fanout_strands_nothing` 1.24 s,
`journal_race::parked_commit_fanout_keeps_every_record` 1.21 s,
`lost_update::commit_refuses_a_path_another_session_overwrote` 1.13 s.

Full default `cargo nextest run` (build already warm): before 31.03 s real
(3772 tests); after 32.38 s and 33.04 s with warm mutants (3815 tests), 56.89 s
with a cold mutant cache (the three family builds contending with the rest of
the suite). `session_identity::resolution_cost_stays_far_under_the_hook_deadline`
passed in every parallel run.

As in § 8, nextest intermittently reports a test here as `LEAK` (for example
`scoped_commit::a_bare_shells_changeset_carries_across_processes`); every child
a test spawns is reaped by `Proc` or `Command::output`, and the flag moves
between tests run to run.

## 10. Phase 7: nextest as the single runner, and the coverage audit

`cargo nextest run` is the contributor acceptance command. The last two
shells, `scripts/verify-{worktree-temp-hygiene,git-config-isolation}.sh`,
are now the `rdm-cli` binary `suite_hygiene` (`rdm-cli/tests/suite_hygiene/`),
which the default profile excludes (`default-filter` in `.config/nextest.toml`;
nextest reports the binary as skipped) and the required `suite-hygiene` profile
runs serially. The last standalone JavaScript test outside the Codex runtime,
`scripts/lib/review-effort.test.mjs`, is now `workflow_review::effort`. CI's
`Shell harnesses` step, `scripts/lib/mechanical-tier-check.sh` (sourced only
by harnesses deleted in phases 2–3) and `rdm-core/tests/workflow_review_effort.rs`
are deleted. Every mapping below was completed before the deletion commit.

### Suite hygiene: layout and isolation

| Module | Test | Nested run | Asserts |
|---|---|---|---|
| `temp_hygiene` | `worktree_suites_leave_no_worktree_in_tmpdir` | `-p rdm-git -p rdm-cli` (whole crates), invoking env | exit 0; no `*__worktrees` in the scratch `TMPDIR` (the message names the leaked dirs and the `dir.path().join("repo")` remedy) |
| `git_config` | `hostile_git_config_changes_no_result` | the shell's `FILTER` over `-p rdm-git -p rdm-cli`; hostile `HOME` (`.gitconfig`: identity, `diff.relative`, `diff.external` driver writing a marker, `init.defaultBranch=weird`, `advice.detachedHead`, `core.pager=false`, `commit.gpgsign`), hostile `GIT_CONFIG_SYSTEM`, empty 0700 `GNUPGHOME`, `RUSTUP_HOME`/`CARGO_HOME` pinned | exit 0; marker absent; no leak |
| `canary` | `hook_git_env_reaches_no_repository` | whole workspace, default profile, **without** the hk scrub: `GIT_DIR=<canary>/repo/.git/worktrees/wt`, `GIT_INDEX_FILE=<same>/index`, `GIT_PREFIX=`, `GIT_EDITOR=:` | exit 0; every file under `<canary>/repo/.git` and `<canary>/wt` byte-identical (differing paths named); `core.bare` reads `false` through a scrubbed git |
| `canary` | `an_unscrubbed_git_init_flips_the_canary` | none | negative control: same fixture and hook env; one unscrubbed `git init <scratch>/victim` changes `<canary>/repo/.git/config` and `core.bare` reads `true` |
| `mutants` | `tempdir_rooted_fixture_leaks_a_worktree` (M1) | in the mirror: `--frozen -p rdm-git --test worktree`, global and system git config `/dev/null` | exit 0; at least one leak (18 observed) |
| `mutants` | `stripped_git_config_isolation_fails_the_hostile_run` (M2) | in the mirror: `--frozen -p rdm-git -p rdm-cli --lib --test worktree --test cli_{worktree,gate,verify,review_change} -E FILTER`, hostile env | exit **100** (tests ran and failed: gpg signing refused on fixture commits) |

- **Nested runs** (`suite_hygiene/nested.rs`) spawn `$CARGO nextest run`
  through `rdm_devtools::process::run_bounded` in a fresh process group, with a
  25-minute inner deadline under the profile's 30-minute backstop
  (`slow-timeout = { period = "300s", terminate-after = 6 }`), and the output in
  a log file (`ProcessSpec::output_file`, added for this). The environment is
  the invoking one minus every `RDM_*` except `RDM_TEST_NODE`, every
  `NEXTEST_*`/`__NEXTEST_*`, `git_test_support::repo_redirect_removals()`, and
  the per-test cargo variables; then `CARGO_TERM_COLOR=never`,
  `TMPDIR=<test TempDir>/…/tmp`, and the scenario's overrides. Tests never
  mutate their own environment.
- **Classification is nextest's exit status**, never parsed text: 0 and 100
  are results; anything else (101 build failed, 4 no tests, 96 setup error, a
  signal, the deadline) is `Failure::Infra` with the log's tail.
- **Mutants** plant three anchored, exactly-once edits (family
  `suite-hygiene`): `FnBody` of `fn init_project_repo(` in
  `rdm-git/tests/worktree.rs` (re-rooted at its `TempDir`), and a `Replace` of
  the `GIT_CONFIG_SYSTEM`/`GIT_CONFIG_GLOBAL` isolation in both
  `git_test_support.rs` copies. The mirror is phase 6's, extracted into
  `rdm-cli/tests/common/mirror.rs` and shared with `concurrency` (whose 43
  tests stayed green). Attribution: M1 runs under a neutral git config, so the
  isolation strip is inert; M2 asserts only on test failures, which the
  re-root (it leaks, but passes) never causes.
- **Canary fixture**: `git init -b main repo`, one commit, `git worktree add
  ../wt`, all inside the test's `TempDir`, built through the `Sandbox`; only
  the one nested child (or the one `git init` in the control) carries the hook
  variables.
- **Deviation from the plan (M1)**: the plan named "a scratch `HOME`"; M1 sets
  `GIT_CONFIG_GLOBAL=/dev/null` instead, which is neutral whatever
  `XDG_CONFIG_HOME` holds and needs no Rust-home pin.

**AC3 anchor demonstration** (a scratch copy, never the worktree): in the
depth-1 clone below, the family definition was edited so the `FnBody` signature
was absent — both controls failed as `negative control not run: infrastructure
failure: mutant family suite-hygiene edit "fixture rooted at its TempDir" …
not applied: 0 lines start with "fn init_project_repo_absent_anchor("` — and,
with that restored, so the isolation anchor was absent — both failed as `… edit
"rdm-git fixture git-config isolation stripped" … not applied: its anchor
occurs 0 times`. Neither passed. The clone was then restored with `git
checkout`.

### `review-effort.test.mjs` → `workflow_review::effort`

Real `buildReviewPipeline` / `runPlanReviewDriver` / `parsePlanArgs` (via
`Lib::real`, the recording `Agent` and `Js`); expected values are per-scenario
literals (each mode's dimension keys, the effort vocabulary).

| JS case | Rust test(s) | P/D |
|---|---|---|
| 1 supplied effort reaches every finder (incl. retry) and refuter, × {code, plan} | `supplied_effort_reaches_every_finder_retry_and_refuter_{code,plan}` | P (the finder label set equals every dimension plus the first one's retry) |
| 2 no effort → no `effort` key, × {code, plan} | `absent_effort_adds_no_key_{code,plan}` | P |
| 3 invalid effort refused before any agent, × {code, plan} | `invalid_effort_refused_before_any_agent_{code,plan}` | P |
| 4 every Claude effort level accepted | `every_claude_effort_level_accepted` | P |
| 5 plan driver threads effort args to real finders/refuters | `plan_driver_effort_args_reach_real_finders_and_refuters` | P |
| 6 plan driver without effort args adds no key | `plan_driver_without_effort_args_adds_no_key` | P |
| 7 `parsePlanArgs` normalises effort like model | `parse_plan_args_normalises_effort_like_model` | P |

None is D: `driver::find_and_verify_effort_reach_every_finder_and_refuter`
covers the review *engine's* driver arm (code mode, no retry path), a
different entry point, and stays as that arm's owner.

### 1. Consolidated shell map

The phase body's "27 original verification shells" are the 24
`scripts/verify-*.sh`, the 2 `observe-*.sh` and `capture-golden.sh` present at
base `b4a3782`. Two `verify-*.mjs` files there (`verify-review-source.mjs`,
`verify-codex-coexistence.mjs`) and the four shells deleted before phase 3 are
listed after them. P = ported (named test), D = demonstrated duplicate, R =
retired with reason. Totals come from each owning section's case map; for
phases 2–4, whose tables have no totals line, they are counted over the § 1
rows (a row that ports the behaviour and retires only string sub-assertions
counts as P).

| Shell | Owner | Case map | P / D / R |
|---|---|---|---|
| verify-workflow-review.sh | phase 2 | § 1 (27 live sections) | 21 / 2 / 4 (16 in full, 5 split) |
| verify-workflow-review-outcome.sh | phase 2 | § 1 | 1 / 1 / 1 |
| verify-workflow-backlog.sh | phase 3 | § 1, § 6 | 8 / 1 / 1 |
| verify-workflow-document.sh | phase 3 | § 1, § 6 | 5 / 0 / 0 (3 sections, 2 strengthened rows) |
| verify-workflow-estimate.sh | phase 3 | § 1, § 6 | 8 / 1 / 1 |
| verify-skill-autopilot.sh | phase 3 | § 1 | 2 / 1 / 1 |
| verify-token-report.sh | phase 4 | § 1, § 7 | 6 / 0 / 1 |
| verify-refuter-agreement.sh | phase 4 | § 1, § 7 | 12 / 0 / 2 |
| verify-agent-config-distribution.sh | phase 5 | § 8 | 15 / 4 / 11 |
| verify-plugin-distribution.sh | phase 5 | § 8 | 4 / 1 / 3 |
| verify-plugin-install.sh | phase 5 | § 8 | 7 / 0 / 1 |
| verify-plugin-loop.sh | phase 5 | § 8 | 3 / 1 / 0 |
| verify-claude-code-web-loop.sh | phase 5 | § 8 | 1 / 1 / 0 |
| verify-rdm-plan-fixture.sh | phase 5 | § 8 | 0 / 2 / 3 (tested shell test support only) |
| verify-golden-json.sh, capture-golden.sh | phase 5 | § 8 | 4 / 0 / 1 |
| verify-backlog-groom-loop.sh | phase 5 | § 8 | 6 / 0 / 0 |
| verify-review-revision-loop.sh | phase 5 | § 8 | 6 / 0 / 0 |
| verify-worktree-review-loop.sh | phase 5 | § 8 | 4 / 0 / 1 |
| verify-session-identity.sh | phase 6 | § 9 | 19 / 2 / 1 |
| verify-journal-truncation-race.sh | phase 6 | § 9 | 10 / 1 / 1 |
| verify-lost-update.sh | phase 6 | § 9 | 9 / 1 / 3 |
| verify-scoped-commit.sh | phase 6 | § 9 | 8 / 8 / 4 |
| verify-worktree-temp-hygiene.sh | phase 7 | § 1, this section | 2 / 0 / 1 — §1 → P `temp_hygiene::…`; §1b mutation → P M1; §1b restore + `cmp` → R (moot: the mirror never writes the checkout) |
| verify-git-config-isolation.sh | phase 7 | § 1, this section | 3 / 2 / 3 — §1 clean arm → D (`temp_hygiene::…`, a superset of crates, and the default run); §1 hostile arm + identical results → P `git_config::…`; §1 no-results guard → R (nextest's no-tests exit is `Infra`); §1b → P M2; §1b restore + `cmp` → R (moot); §2 hostile leak count → P (inside `git_config::…`); §2 clean leak count → D (`temp_hygiene::…`); §2 "summary contains `worktree`" floor → R (not behavioral coverage: a name grep; non-vacuity comes from M1) |
| observe-plugin-install.sh | kept (live) | § 1, § 8 | exception: live observation, not run by CI |
| observe-workflow-listing.sh | kept (live) | § 1 | exception: live observation, not run by CI |
| verify-review-source.mjs | phase 2 | § 2(b) | ported to `workflow_review::{outcome,engine}` |
| verify-codex-coexistence.mjs | phase 4 | § 2(c), § 7 | ported to `rdm-smoke codex-coexistence` |
| verify-workflow-dispatch.sh, verify-workflow-do-auto.sh, verify-workflow-do-auto-task.sh, verify-skill-intent-interview.sh | deleted before phase 3 | § 6 "Shells already absent" | retired with their product, retained evidence named there |

**New, with no shell ancestor**: `canary::hook_git_env_reaches_no_repository`
and `canary::an_unscrubbed_git_init_flips_the_canary` (task
`canary-git-env-isolation-regression`).

### 2. Test inventory

`cargo nextest list` (default profile): 3825 tests across 80 binaries, plus
the `suite_hygiene` binary skipped by `profile.default.default-filter` and
three `#[ignore]`d tests. The migrated binaries list their cases
individually:

| Binary | Tests | Migrated content |
|---|---|---|
| `rdm-cli::workflow_review` | 198 | real review/plan-review helpers and drivers, mocked-driver execution, mutant controls (phases 2–3), `effort::` (10, this phase) |
| `rdm-cli::workflow_passes` | 47 | backlog/document/estimate helpers, drivers, generator drift controls (phase 3) |
| `rdm-cli::distribution` | 29 | emission, plugin tree, downstream engine execution, corruption controls (phase 5) |
| `rdm-cli::cli_loops` | 7 | plugin/web/backlog/revision/worktree-review loops (phase 5) |
| `rdm-cli::golden_json` | 5 (+1 ignored `bless`) | golden JSON contract (phase 5) |
| `rdm-cli::concurrency` | 43 | session, commit, race tests and 10 mutant controls (phase 6) |
| `rdm-devtools::*` | 152 across the lib and 9 test binaries | process lifecycle, workflow host, measurement tools, smoke (phases 1, 4) |
| `rdm-cli::suite_hygiene` | 6 (`--profile suite-hygiene` only) | this phase |
| `rdm-cli::codex_process` | 50 | Codex runtime transport, run state, whole runner and queue, 2 mutant controls (phase 8, § 11; the default-profile total is then 3880) |

`cargo nextest list --profile suite-hygiene` lists exactly the six tests
above. No test is `#[ignore]`d to leave the default run.

### 3. JavaScript ownership

From `git ls-files '*.js' '*.mjs' '*.cjs'` (47 files):

| Files | Owner / role | Disposition |
|---|---|---|
| `.claude/workflows/lib/{review,plan-review,estimate,backlog,document}.mjs` (5), `.claude/workflows/rdm-wf-*.js` (5), `rdm-core/src/templates/workflows/rdm-wf-*.js` (5), `plugins/rdm/workflows/rdm-wf-*.js` (5) | production Claude workflow code: canonical libraries, engines, embedded and plugin copies | kept |
| `rdm-devtools/src/workflow_host.mjs` | test-side binding: a narrow generic runtime host (`include_str!` into `rdm_devtools::workflow`) with no cases, expected values or tool logic; not shipped (`rdm-devtools` is `publish = false`, `dist = false`) | kept |
| `rdm-core/src/templates/codex-runtime/**` (8), `scripts/rdm-codex.mjs`, `scripts/gen-codex-runtime.mjs` (generator), `scripts/lib/codex-{process,runtime,runtime-estimate,runtime-state,spike-estimate,spike-process,spike-review}.mjs`, `scripts/run-codex-orchestration-spike.mjs` (opt-in research runner) | Codex production runtime code (`codex-agent-support`) | kept |
| `rdm-cli/tests/support/codex-bridge.mjs` | Codex test transport: a narrow generic runtime binding (`docs/codex-test-migration.md`) | kept |
| `scripts/lib/codex-{spike-process,runtime-state,runtime-review-process,runtime-queue}.test.mjs` and CI's `node --test` step | ported to the `rdm-cli` binary `codex_process` (§ 11) | **phase 8 (done)**: deleted, with the step |
| `rdm-server/assets/{edit,review-anchor,review-highlight}.js` | browser assets, product UI | recorded separately, outside this change |
| `scripts/lib/review-effort.test.mjs` | — | removed this phase |

No `package.json`, `node_modules`, npm test framework or JS tooling layer
exists. **Roadmap condition status**: with phase 8 the roadmap's "no
JavaScript test files" and "runtime provisioning only for executing production
JavaScript" conditions are **fully met**: `git ls-files '*.test.mjs'` prints
nothing, and Node is provisioned (`mise exec node --`) only for `cargo nextest
run` (and the suite-hygiene profile's nested runs of it), where it executes
the production workflow and Codex runtime modules for Rust-owned tests.

### 4. Hidden-suite audit (a review, not a test)

A read of the Rust test sources for JavaScript embedded in strings and for
programs a test writes at run time found no relocated suite:

- `include_str!("workflow_host.mjs")` in `rdm_devtools::workflow`: the
  generic host above.
- Generated `#!/bin/sh` stubs, each a fixture, never an assertion carrier: a
  pre-existing custom hook (`cli_hook.rs`), a no-network `curl` stub
  (`cli_loops/claude_code_web.rs`), the external-diff marker driver
  (`cli_review_change.rs`, `suite_hygiene/git_config.rs`), fake `rdm`/`codex`
  binaries recording or refusing calls (`codex_runtime.rs`,
  `workflow_review/driver.rs`), the fake `cargo` exec stub
  (`distribution/codex.rs`), and the concurrency `ShellDriver` scripts (the
  process topology under test, no waits or branches; § 9).
- One-line JavaScript fixture files written as data (`codex_runtime.rs`
  `a.js`, `distribution/plugin.rs` `stray.js`, `distribution/superseded.rs`
  a tampered `autopilot.js`) and the emitted engine copied to `raw.mjs` to
  prove it does not import (`distribution/downstream.rs`).
- `codex_bridge` requests name production modules and exports, not test
  logic.
- Phase 8: the `codex_process` fakes (`FAKE_CODEX`, `FAKE_RDM`,
  `FAKE_RUNNER_CODEX` in `codex_process/support.rs`) are `#!/bin/sh` stubs
  that record each call and replay a Rust-written response; the only branches
  are on the argument the runtime passes, the schema/prompt role marker and
  Rust-written behaviour files — no assertion, no expected value. The glue
  gained generic ops only (`new`, a kept `get`, `invoke`, `ping`, a raw
  `reject` reply; § 11), and still holds no case or fixture branch.

Rust smoke and measurement implementations (`rdm-smoke`, `rdm-measure`) have
no JavaScript counterpart left: every file in § 2(d) is deleted.

### 5. Exceptions to `cargo nextest run`

- `cargo nextest run --profile suite-hygiene`: nested whole-suite runs, kept
  out of the default profile for recursion and cost; CI-required.
- `cargo test --doc --workspace`: nextest does not run doctests (30 today);
  CI and the hk `cargo_doctest` step run them.
- Live observations, which report non-execution distinctly:
  `observe-plugin-install.sh`, `observe-workflow-listing.sh`,
  `codex_coexistence_live` (`--run-ignored only`), the Codex spike runner.
- Non-test gates: fmt, clippy (`--workspace --all-targets`), shellcheck,
  shfmt, the feature-matrix `cargo check`s, the release build, `cargo deny`.
- Deliberate `#[ignore]` writers, not regressions: `golden_json::bless`,
  `rdm-core`'s `regenerate_raw_skills_baseline`.
- Plain `cargo test` also runs the `suite_hygiene` binary (it is a normal
  test target so `--all-targets` clippy covers it). That is correct — its
  nested runs use nextest's default profile, so nothing recurses — just slow.

### 6. Wall times (recorded evidence, not asserted)

Host: macOS 27.0 (Darwin 27.0.0, arm64, Apple M5 Max, 18 cores), 2026-09-24,
a shared machine (1-minute load average 1.7–16 across the runs, recorded per
run from `uptime`). rustc 1.94.0, cargo-nextest 0.9.130, Node v24.18.0, git
2.55.0. All "warm" runs had a built target dir.

Before (the two shells, `bash scripts/verify-<name>.sh` in a scratch clone of
`580973e` with its own warm target, so the in-place mutations never touched
this worktree; three runs each):

| Harness | Wall (s) |
|---|---|
| verify-worktree-temp-hygiene.sh | 28.65, 29.15, 28.14 (a first run, 61.33, overlapped an unrelated compile and is discarded) |
| verify-git-config-isolation.sh | 13.23, 13.58, 13.58 |
| **sum (one run each)** | **≈ 42** |

Earlier figures for the same shells (78 s and 143 s before their repetition was
removed) are in `docs/review-evidence-closeout.md`; per-phase shell timings for
everything else are in §§ 4–9.

After:

| Run | Wall (s) |
|---|---|
| `cargo nextest run`, warm ×3 | 31.87, 33.09, 32.81 (summary 31.5, 32.7, 32.4; 3825 tests) |
| `cargo nextest run --test-threads 2` (a 2-core stand-in) | 121.92 (summary 121.5) |
| `--profile suite-hygiene`, warm ×2 | 67.81, 65.87 (summary 67.3, 65.6) |
| `--profile suite-hygiene`, cold mutant cache (`rm -rf target/tmp/rdm-mutants/suite-hygiene`) | 76.61 (summary 76.3) |
| `cargo test --doc --workspace` | 1.95 (30 doctests) |

Per test (warm): canary whole-workspace run 32.0–33.9 s, `temp_hygiene` 25.2–26.1
s, `git_config` 5.4–7.1 s, M2 0.4–1.0 s, M1 0.65–0.9 s, the canary control
0.06 s. Cold mirror: M2's nested run including the family build 9.8 s, then M1
4.1 s; mirror sync of 623 files. Nested runs do not rebuild: under the canary's
hook environment and under a hostile `HOME`, the nested build step reports
`Finished … in 0.11s` / `0.08s`.

**Default-profile backstop.** The slowest default test under
`--test-threads 2` was `rdm-devtools::process_lifecycle broken_cleanup_mutant_is_detected`
at 9.04 s (next: 2.19 s). `[profile.default] slow-timeout = { period = "60s",
terminate-after = 5 }` gives a 300 s backstop, over 30× that; the existing,
more specific overrides (mutant builds 600 s, workflow and concurrency 120 s,
rdm-devtools 60 s) keep precedence.

**Bottlenecks.** The suite-hygiene profile is three nested whole-suite runs
(canary ≈ 32 s, temp hygiene ≈ 25 s, the git-config filter ≈ 6 s): it is
slower than the two shells it replaces (≈ 66 s against ≈ 42 s) only because it
adds the canary's whole-workspace run, new coverage; without it the two ports
take ≈ 34 s. Cold mutant builds cost ≈ 10 s here and scale with CI cores. The
recurring nextest `LEAK` flag (§§ 8–9) still appears on the concurrency binary
(1 leaky of 43 in one run); it is host noise, and nextest's default leak
result is pass.

### 7. Exact commands and results

**AC2 — the checkout is unchanged by the suite-hygiene profile.** In the
worktree (with this phase's docs edits uncommitted):

```bash
git status --porcelain=v1 -uall > status-before
git ls-files -z --cached --others --exclude-standard | xargs -0 shasum -a 256 > sums-before   # 623 files
mise exec node -- cargo nextest run --profile suite-hygiene     # 6 passed, 64.2 s
git status --porcelain=v1 -uall > status-after
git ls-files -z --cached --others --exclude-standard | xargs -0 shasum -a 256 > sums-after
cmp status-before status-after && cmp sums-before sums-after     # identical
```

The primary checkout's `git status --porcelain=v1 -uall` was identical before
and after, and `git config --get core.bare` read `false` in both the
primary checkout and the worktree.

**AC6 — every `ci.yml` `run:` command, in order**, from the worktree with
`CARGO_TERM_COLOR=always`, `RUSTFLAGS="-D warnings"`, no `RDM_*`, and a fresh
`CARGO_TARGET_DIR` (a cold cache, and no explicit `cargo build` anywhere).
`ci.yml` parses with Python's PyYAML (`yaml.safe_load`) and Ruby's Psych to
the 17 steps listed in the plan's D2 (checkout, toolchain, rust-cache,
nextest, cargo-deny, mise, then the eleven below).

| Step | Command | Exit | Wall |
|---|---|---|---|
| Check formatting | `cargo fmt --check` | 0 | 0 s |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` | 0 | 8 s |
| Shell lint | `mise exec shellcheck -- shellcheck $(git ls-files '*.sh')` | 0 | 0 s |
| Shell format | `mise exec shfmt -- shfmt -d $(git ls-files '*.sh')` | 0 | 0 s |
| Feature matrix | the four `cargo check` lines | 0 | 13 s |
| Test | `mise exec node -- cargo nextest run` | 0 | 71 s (3825 passed) |
| Doctests | `cargo test --doc --workspace` | 0 | 2 s |
| Suite hygiene | `mise exec node -- cargo nextest run --profile suite-hygiene` | 0 | 89 s (6 passed; cold mutant cache) |
| Build (release) | `cargo build --release` | 0 | 20 s |
| Codex legacy | `mise exec node -- node --test --test-concurrency=1 scripts/lib/codex-{spike-process,runtime-state,runtime-review-process,runtime-queue}.test.mjs` | 0 | 63 s |
| Audit | `cargo deny check` | 0 | 1 s |

Finding 6, refined: the Codex step passes after only `cargo nextest run`,
but nextest's build does not leave a `cargo run`-fresh dev `rdm` binary —
`target/debug/rdm` was re-linked during the Codex step (its mtime falls inside
that step), so the first `scripts/rdm-dev.sh` call compiled `rdm-cli`'s
binary. A warm re-run of the step took 54 s against 63 s. Its calls allow
120 s per `rdm` invocation, so this held here; on a 2-core runner the first
call pays a single `rdm-cli` bin compile against that budget. Phase 8 deletes
the step.

**AC11 — shallow clone.** `git clone --depth 1 -b roadmap/rust-test-suite-consolidation
file://<worktree>` of `0374796` (`git rev-parse --is-shallow-repository` →
`true`), own target dir: `cargo nextest run` passed, 3825 tests, 71.4 s wall
including the cold build; `cargo nextest run --profile suite-hygiene` passed, 6
tests, 88.7 s. No test reads the checkout's history, so `ci.yml` drops
`fetch-depth: 0` and returns to the default shallow checkout.

**hk.** `hk validate` passes. The `cargo_nextest` step's glob was checked
against what tests read: `include_str!` targets (`rdm-core/src/templates/**`,
`rdm-server/assets/**`, `rdm-devtools/src/workflow_host.mjs`), paths joined to
the repo root (`.claude/{workflows,skills,agents}`, `.agents/skills`,
`.claude-plugin/plugin.json`, `plugins/rdm`, `scripts/lib`, `scripts/rdm-codex.mjs`,
`tests/fixtures`, `tests/golden`, `templates/claude-code-web`,
`docs/principles.md`, `docs/token-baseline.json`), plus `.config/nextest.toml`,
`.mise.toml`, `Cargo.lock` and manifests; every crate directory is included
whole (`rdm-*/**`). In a throwaway `git clone` (never this checkout), `hk check
--plan` with only `.claude/workflows/lib/review.mjs` staged selects
`cargo_nextest` alone; with only an `.rs` file staged it selects fmt, clippy,
`cargo_nextest` and `cargo_doctest`; with only `CLAUDE.md` staged it selects
none. The `env -u GIT_*` scrub prefix is unchanged, and `cargo_doctest` uses
the same one.

### 8. Evidence limits

- Everything here is component or real-process evidence on one macOS host.
  There is no Claude-host observation, and no pushed CI run is claimed.
- Linux-specific paths were not exercised: the `/proc` zombie probe and signal
  tests in `rdm-devtools`, the rustup-shim `HOME` pin that `nested::rust_home_pins`
  carries over from the shell's CI fix (locally `cargo` is the rustup shim and
  `RUSTUP_HOME` is set, so the pin was exercised only in its already-set form),
  GNU vs BSD tool differences, and 2-core contention on real runners. No Linux
  run was made: the Docker daemon was not running on the host.
- The default-profile backstop is sized from a `--test-threads 2` run on a fast
  host, not from a real 2-core runner.

## 11. Phase 8: the remaining Codex runtime process suites

The four `node --test` suites CI still ran in its "Remaining legacy Codex
process contracts" step — `scripts/lib/codex-spike-process.test.mjs`,
`codex-runtime-state.test.mjs`, `codex-runtime-review-process.test.mjs` and
`codex-runtime-queue.test.mjs`, 50 cases — are now the `rdm-cli` test binary
`codex_process`. The suites and the step are deleted; `git ls-files
'*.test.mjs'` prints nothing (§ 10.3). Node still executes the real production
modules (`scripts/lib/codex-process.mjs`, `codex-runtime-state.mjs`,
`codex-runtime.mjs`, `scripts/rdm-codex.mjs`); Rust owns every fixture, fake
script, response, wait, assertion and teardown. No production module or
generated template copy changed, so `scripts/gen-codex-runtime.mjs` was not
run.

### Layout

| Module | Production JS | Tests |
|---|---|---|
| `codex_process/transport.rs` | `codex-process.mjs` (and one run through `codex-spike-process.mjs`) | 25 |
| `codex_process/state.rs` | `codex-runtime-state.mjs` (`createRun`) | 15 |
| `codex_process/runner.rs` | `codex-runtime.mjs` (`runRuntime`) and the CLI `rdm-codex.mjs` | 8 |
| `codex_process/mutants.rs` | a private temp copy of the runtime's import closure | 2 |
| `codex_process/support.rs` | — (roots, host config, fakes, `Reaper`, readers) | — |

`main.rs` includes `common/workflow_support.rs`, `git_test_support.rs` and
`common/plan_fixture.rs` as `concurrency/main.rs` does. `.config/nextest.toml`
adds `binary(codex_process)` to the Node-hosted 30 s × 4 slow-timeout
override.

- **Roots.** Each test owns a canonical `TempDir` with its own `Sandbox`; every
  `git`/`rdm` child the test spawns goes through it. The Node host's child
  environment is the sandbox's (its removals — every inherited `RDM_*`,
  `REPO_REDIRECT_VARS`, `CODEX_HOME`, harness session ids — and its
  `HOME`/XDG/`/dev/null` git config/identity) plus `TMPDIR=<root>/tmp`, so the
  runtime's own `mkdtemp` stays in the root. Tests never set their own
  environment.
- **Fakes.** `FAKE_CODEX` (transport), `FAKE_RDM` (state) and
  `FAKE_RUNNER_CODEX` (runner) are POSIX `sh` written 0700 into the root. Each
  records pid, argv, stdin, `pwd -P` and environment under `calls/<pid>/` and
  replays a response Rust wrote; the runner fake also appends `start`/`end`
  lines (O_APPEND) for barrier and concurrency evidence. Their only branches are
  on the argument the code under test passes, the schema/prompt role marker, or
  Rust-written behaviour files (`hang`, `kill-self`, `spawn-and-hang`, delays).
- **Reaping.** Production spawns every fake `detached` (pid = pgid). The
  `Reaper` kills every recorded group and descendant on drop, on every path;
  `Host`/`Session` teardown kills and reaps the Node group; waits are bounded
  readiness polls. The only fixed sleeps are the state tests' 1.5 s "no late
  effect" windows (the fake's 1 s delay plus 0.5 s).
- **Runner plan repos** are seeded with `CARGO_BIN_EXE_rdm` under the sandbox
  (the legacy suites ran `scripts/rdm-dev.sh`, a `cargo run` per call), so the
  runtime's `rdmBin` is the binary under test, and the expected models and
  efforts come from its `rdm model resolve <step> --host codex --format json`.

### The binding extension (repository-only tooling)

`codex_runtime.rs` drives production JS through `tests/support/codex-bridge.mjs`,
which answers one request per process and cannot act while a call is in flight.
These cases must: abort a real `AbortController` after a fake reports
readiness, hold `boundedParallel` thunk replies, race `finish`/`fail` against an
active command, and SIGTERM a running runtime. Phase 2's
`rdm_devtools::workflow::Host` gained generic transport for that — no cases,
expected values or fixture branches — each with its own test in
`rdm-devtools/tests/workflow_host.rs`:

| Addition | Test |
|---|---|
| glue `new`, kept `get`, `invoke`; `Host::{construct, get_ref, invoke}` | `construct_get_ref_and_invoke_drive_a_real_abort_controller` |
| `Host::{start_call, is_settled, await_call, service, flush, reply}`; glue `ping` | `a_started_call_settles_only_after_its_held_reply` |
| `HostConfig::{env, env_remove}` (child only, after the built-in removals) | `host_config_env_reaches_the_child_and_env_remove_cancels_it` |
| a result for an id never started is still a protocol error | `a_result_for_an_unstarted_id_is_a_protocol_error` |
| `Outbox::reject_with` / glue `reject` reply | exercised by `codex_process::transport::bounded_parallel_falsy_rejection_is_a_failure` |

`flush` sends two pings, the second only after the first is answered, so it is
read in a fresh macrotask after every microtask the earlier lines queued.
`codex-bridge.mjs` is unchanged and still serves `codex_runtime.rs`.

### Case map

P = named Rust test, D = duplicate of an existing Rust test. No case is
retired: the plan's one retirement (the spike re-export identity check) became
a behavioural P test on plan review.

**`codex-spike-process.test.mjs` (26) → `codex_process::transport`**

| Legacy case | Rust test | |
|---|---|---|
| spike entrypoint re-exports the runtime transport | `spike_entrypoint_runs_the_runtime_transport` | P: imports `codex-spike-process.mjs` and drives `runCodex` (one fake call, read-only sandbox, stdin prompt) and `boundedParallel` through it |
| validates canonical subset, fails closed on unknown keywords | `validate_schema_accepts_the_closed_subset_and_fails_closed` | P |
| fresh subprocess: strict nullable schema, stdin, safe argv | `run_codex_uses_strict_nullable_schema_stdin_and_safe_argv` | P (every property required, compared as a set: serde_json sorts the keys the runtime receives) |
| judgment subprocess disables optional capabilities/escalation | `run_codex_disables_optional_capabilities_and_escalation` | P |
| completed turn recovers from a failed exploratory command | `run_codex_accepts_a_recovered_nonzero_shell_command` | P |
| rejects command-inconsistent, command-interrupted, tool-failed, death, malformed, error, failed, duplicate, truncated, invalid, flood (11) | `run_codex_rejects_{inconsistent_command,interrupted_command,failed_tool,killed_child,malformed_stream,error_event,failed_turn,duplicate_thread,truncated_stream,invalid_response,output_flood}_without_diagnostics` | P ×11 (message contains neither `secret` nor `sensitive`) |
| rejects auth | `run_codex_rejects_nonzero_exit_without_diagnostics` | P |
| rejects rate, unknown-model, nonzero (3) | → the auth test | D: the legacy fake ran one identical branch for all four |
| timeout and abort await shutdown | `run_codex_timeout_rejects_a_hung_child`, `run_codex_abort_rejects_a_hung_child` | P ×2 (the fake's pid is dead afterwards; abort after its pid file exists; the timeout is 1 s, not 150 ms, so the fake has recorded itself) |
| resume requires matching thread identity | `run_codex_resume_requires_the_same_thread` | P (also: `resume <id>` in argv, no `--ephemeral`) |
| canonical labels; private, redacted diagnostics | `run_codex_failure_evidence_is_private_and_redacted` | P (dir 0700, file 0600, `[REDACTED]` present, token absent) |
| bounded parallel: order, limit, waits for in-flight failure cleanup | `bounded_parallel_preserves_order_under_the_limit`, `bounded_parallel_failure_waits_for_in_flight_work` | P ×2 (held replies: never more than 2 pending, 2 reached, answered newest-first; after the failing reply and `flush` the call is unsettled until the in-flight thunk is answered) |
| falsy rejection cannot succeed | `bounded_parallel_falsy_rejection_is_a_failure` | P (`reject_with(null)`; the rejection surfaces as `null`) |
| cancellation stops descendants | `run_codex_cancellation_kills_descendants` | P |

Totals: 23 P, 3 D; 25 tests.

**`codex-runtime-state.test.mjs` (16) → `codex_process::state`**

| Legacy case | Rust test | |
|---|---|---|
| explicit identity, private evidence, owned session, direct argv | `create_run_binds_explicit_identity_and_direct_argv` | P (argv reaches the fake verbatim and no shell ran; session/cwd/root/project from the recorded environment; 0700 run dir; completed manifest; reuse and post-finish `rdm` refused) |
| rejects implicit paths/sessions, nested checkout, symlinked evidence parent | `create_run_rejects_implicit_identity_nested_checkout_and_symlinked_evidence` | P |
| write intent durable before execution; failure prevents success | → `codex_runtime::session_uncertain_write_prevents_success_and_records_recovery_evidence` | D |
| `--all` rejected; JSON decode failure poisons mutations | `json_decode_failure_poisons_later_mutations` | P for the decode half (plus a later mutation refused and never run); the `--all` half is D → `codex_runtime::session_forbids_all_session_commit_and_identity_overrides` |
| acknowledged writes capture plan HEAD and can finish | `acknowledged_write_records_plan_head_and_finishes` | P (`data.planHead` equals the plan repo's HEAD) |
| identity overrides and unjournaled commits fail before execution | `changeset_override_and_unjournaled_commit_fail_before_execution` | P for `--changeset=…` and bare `commit`, no fake invocation; the `--root` half is D → the same existing test |
| bounded mutation timeout is uncertain, forbids retry | `mutation_timeout_is_uncertain_and_forbids_retry` | P (also: the journaled `rdm-started` pid is dead) |
| inherited GIT_DIR cannot redirect source identity | `inherited_git_dir_cannot_redirect_source_identity` | P (`GIT_DIR` on the Node child only) |
| adapter readback uncertainty persists in the failed manifest | `adapter_readback_uncertainty_persists_in_failed_manifest` | P |
| caller RDM overrides and harness knobs don't enter direct commands | `caller_rdm_overrides_do_not_reach_direct_commands` | P (`RDM_CHANGESET` on the Node child; the fake's `RDM_*` keys are exactly BIN/PROJECT/ROOT/SESSION) |
| manifest records runner identity | `manifest_records_runner_identity` | P (pid = the host, ppid = the test process, executable and argv = the resolved Node and the host's glue path, ISO-8601 start) |
| cancellation stops mutation descendants, cannot report completion | `cancellation_kills_mutation_descendants_and_blocks_completion` | P (real `AbortController`, aborted after the fake records its descendant) |
| preaborted run cannot start a mutation or finish | `preaborted_run_starts_no_mutation_and_cannot_finish` | P |
| direct process exit reaps the leftover group | `exited_command_group_is_reaped_before_return` | P |
| output limit fails mutations conservatively | `output_limit_fails_mutation_conservatively` | P |
| active command cannot race finalization | `active_command_blocks_finish_and_fail` | P |

Totals: 15 P, 1 D; 15 tests.

**`codex-runtime-review-process.test.mjs` (6) and `codex-runtime-queue.test.mjs` (2) → `codex_process::runner`**

| Legacy case | Rust test(s) | |
|---|---|---|
| full runner maps models, fresh contexts, finder barrier before planted refutation (× code, plan) | `{code,plan}_review_runner_maps_models_and_keeps_the_finder_barrier` | P ×2 (every finder `end` precedes the one refuter `start`; distinct pids; `--sandbox read-only`, `--ephemeral`, no `resume`; `-m`/effort per role from real `rdm model resolve`; journal tiers medium/large; one `agent-completed` per call; completed manifest; neither repo written) |
| finder effort from a configured codex profile (× code, plan) | `{code,plan}_review_runner_takes_finder_effort_from_the_codex_profile` | P ×2 |
| malformed refuter JSON rejected (× code, plan) | `{code,plan}_review_runner_rejects_malformed_refuter_json` | P ×2 |
| preview bounds five Codex children to two and completes all | `estimate_preview_bounds_five_codex_children_to_two` | P, through the CLI under `run_bounded` (5 proposals, peak 2, 5 distinct stems, every child group dead, completed manifest, no writes) |
| cancellation kills active judgments, never launches the queued three | `estimate_cancellation_launches_no_queued_judgment` | P, `runRuntime` on a `Host`, SIGTERM to the host pid (exactly 2 starts, groups dead, 2 `agent-started`, no `run-completed`, failed manifest, no writes) |

Totals: 8 P; 8 tests. The CLI's failure-path stderr stays owned by
`codex_runtime::runtime_cli_invalid_specs_have_no_success_output_or_run_evidence`.

**All suites: 50 cases = 46 P + 4 D + 0 R**, as 48 case tests plus the 2
negative controls below — 50 tests (`cargo nextest list -p rdm-cli --test
codex_process`).

### Mutant map

The legacy opt-in `CODEX_QUEUE_TEST_MUTATION` / `CODEX_REVIEW_TEST_MUTATION`
modes are behavioural, so they are kept, always on. `MutantTree::copy` copies
`scripts/rdm-codex.mjs`, `scripts/lib/codex-{runtime,runtime-state,process,runtime-estimate}.mjs`
and `.claude/workflows/lib/{review,plan-review,estimate}.mjs` into a private
temp dir (relative imports resolve inside the copy; the checkout is never
written); `replace_once` plants one edit; a missing or ambiguous anchor panics
as "negative control not run".

| Test | Planted edit in `codex-runtime.mjs` | Path | Not-run guard | Caught by |
|---|---|---|---|---|
| `mutants::unbounded_agent_semaphore_exceeds_two` | `if(active>=concurrency)await new Promise(r=>queue.push(r));else active++;` → `active++;` | the queue-preview CLI (mutant `scripts/rdm-codex.mjs`) | the run exits 0 with 5 proposals and 5 starts | `runner::check_bounded` fails; observed peak 5 |
| `mutants::refuter_mapped_to_review_find_is_detected` | `role === 'refuter' ? 'review-verify'` → `role === 'refuter' ? 'review-find'` | a code-review `runRuntime` on a `Host` | `review-find` and `review-verify` resolve to different profiles (here `gpt-6-sol`/high vs `gpt-6-astra`/medium); exactly one refuter call | `runner::check_models` fails; the refuter ran on the `review-find` model and effort |

Anchor demonstration (the worktree file edited, run, then restored byte for
byte from a scratch copy): with both anchors replaced by absent strings, both
tests failed in 0.017 s with `negative control not run: infrastructure
failure: mutant `…` not applied: its anchor occurs 0 times in
scripts/lib/codex-runtime.mjs (expected exactly once)`. Neither passed.

### Timings (recorded evidence, not asserted)

Host: macOS 27.0 (Darwin 27.0.0, arm64, Apple M5 Max, 18 cores), 2026-09-24,
rustc 1.94.0, cargo-nextest 0.9.130, Node v24.18.0, git 2.55.0; warm target.

Before (from the plan, same host: `node --test --test-concurrency=1`, `RDM_*`
unset, load 1.1–1.7):

| Suite | Tests | Wall |
|---|---|---|
| spike-process | 26 | 2.14 s |
| runtime-state | 16 | 4.36 s |
| review-process | 6 | 26.87 s |
| queue | 2 | 20.23 s |
| **Total** | **50** | **≈ 53.6 s** |

Most of the review and queue time was `scripts/rdm-dev.sh`'s `cargo run` per
`rdm` call.

After:

| Run | Wall | Load (1/5/15 min) |
|---|---|---|
| `cargo nextest run -p rdm-cli --test codex_process`, warm ×3 | 4.97, 4.97, 5.01 s (summary 4.72, 4.72, 4.76 s; 50 tests) | 3.9 / 5.8 / 4.0 |
| `cargo nextest run` (whole workspace), warm | summary 34.3–35.1 s over four runs; 3880 tests (3830 with `codex_process` filtered out: 32.3–32.9 s) | 3.3 / 5.5 / 4.0 |

Slowest tests: `runner::estimate_preview_bounds_five_codex_children_to_two`
≈ 3.2 s alone (three 0.75 s rounds), 4.9 s under the binary's own
parallelism; the review runs and the refuter control 1.1–1.8 s alone, ≈ 2.9 s
in parallel; everything else under 2.2 s (the state tests with a 1.5 s
late-effect window).

### Evidence limits

- One macOS host; no Linux run. The fakes use only POSIX `sh`, `sleep` with
  fractional seconds, `head -c`, `tr`, `sed -n`, `grep -q` and `kill`, which
  GNU coreutils/dash provide, but that was not exercised here.
- nextest's `LEAK` flag appeared on 0–2 `codex_process` tests per run, as it
  does on the existing `codex_estimate` binary on this host (1–2 per run in the
  same session) — the host noise §§ 8–10 record; nextest's default leak result
  is pass. No test left a process behind (every fake group is killed by the
  `Reaper`, every host by `Session`).
- A pre-existing race surfaced: `codex_runtime::process_timeout_reaps_descendants_before_late_effects`
  (a 100 ms `runCodex` timeout against a `sh` fake that must first write a pid
  file) failed in 2 of ~9 whole-suite runs with this binary present (0 of 3
  with it filtered out). It is filed as task
  `codex-runtime-timeout-test-startup-race`, not fixed here.
