# Codex regression test ownership

Phase 4 moves the affected runtime/spike scenarios into individually discoverable
Rust integration tests. `cargo nextest run` is the acceptance entrypoint. Node.js
24 (pinned by `.mise.toml`) executes the actual production JavaScript, not a second
test suite. Missing Node fails with an installation instruction; it never skips.

```sh
cargo nextest run -p rdm-cli --test codex_runtime --test codex_estimate --test codex_estimate_interruption --test codex_distribution --test cli_agent_config
cargo nextest run -p rdm-cli --test codex_process
cargo nextest run -p rdm-core -E 'test(codex_)'
```

Rust owns temporary source/plan repositories, Cargo-resolved binaries, expected
values, scripted judgments, assertions, isolated configuration, deadlines and
process cleanup. `rdm-cli/tests/support/codex-bridge.mjs` only loads/invokes modules,
hydrates callback/host primitives, and transports JSON replies/errors. The
interruption tests compile a test-only Rust host fixture; no live model,
authentication, globally installed RDM, or user configuration is involved.

The `codex_process` binary (`rust-test-suite-consolidation` phase 8) drives the
production modules through the repository-only `rdm_devtools::workflow` host
instead of `codex-bridge.mjs`. The bridge answers one request per process, so a
test cannot act while a call is in flight; the process and state cases need
exactly that — abort a real `AbortController` after a fake reports readiness,
hold `boundedParallel` thunk replies, race `finish`/`fail` against an active
command, send SIGTERM to a running runtime. The host's detached calls, held
replies and constructed globals are generic transport with no cases of their
own. The fake `codex`/`rdm` binaries are POSIX `sh` that record each call and
replay a response Rust wrote.

## Retired JavaScript cases

Names below omit the common test prefix where indicated. Combined JavaScript
cases are split into individual Rust cases where useful. Historical live spike
evidence stays dated and is not refreshed by these deterministic tests.

### `codex-runtime.test.mjs` → `codex_runtime.rs`

| Previous case | Rust test(s) |
| --- | --- |
| Core effective tiers; host floor/capability validation | `models_take_model_and_effort_from_the_core_codex_profile`, `models_need_no_host_configuration`, `models_refuse_host_tiers_and_steps_with_the_core_remedy`, `models_reject_unusable_core_profiles_and_undeclared_capabilities` |
| Mechanical/unknown role rejection | `judgment_rejects_mechanical_and_unknown_roles_without_processes` |
| Arbitrary plan and content drift | `plan_review_pins_content_and_detects_drift` |
| Clean range, wrong HEAD, option-like revision, AC and source drift | `code_review_clean_range_has_ac_coverage`, `code_review_rejects_wrong_head_before_judgment`, `code_review_rejects_revision_options`, `code_review_selected_ac_requires_evidence`, `code_review_rejects_dirty_source_after_judgment` |
| Phase checkout identity and scoped reads | `phase_review_scopes_reads_and_checks_checkout_identity` (wrong path and branch included) |
| Phase body drift | `phase_review_rejects_body_drift` |
| Clean HEAD drift | `code_review_rejects_clean_head_drift` |
| Refuter error and budget overflow | `code_review_refuter_failure_is_incomplete`, `code_review_refuter_budget_overflow_is_incomplete` |
| Missing finder | `code_review_missing_finder_is_incomplete` |
| Pre-aborted judgment | `judgment_preaborted_signal_creates_no_child_evidence` |
| Invalid CLI specs | `runtime_cli_invalid_specs_have_no_success_output_or_run_evidence` |
| Policy drift during model resolution | `phase_review_rejects_policy_drift_before_judgment`, `runtime_model_resolution_process_cannot_change_phase_policy_silently` |

### `codex-spike-review.test.mjs` → `codex_runtime.rs`

| Previous case | Rust test(s) |
| --- | --- |
| Barrier, independent refutation, plan driver; blocking finding cannot imply rework | `spike_uses_plan_identifier_and_keeps_finder_refuter_barrier` |
| Actual clean plan round | `spike_clean_review_requires_actual_plan_round` |
| Incomplete coverage | `spike_missing_ac_cannot_report_acceptance` |
| Ungraded budget/error and empty AC | `spike_rejects_budget_errors_and_empty_ac` |

### `codex-runtime-estimate.test.mjs` → `codex_estimate.rs`

Every destination below has the prefix `estimate_`.

| Previous case | Rust test(s), without prefix |
| --- | --- |
| Preview | `preview_validates_proposals_without_writes` |
| Apply/body/tags/scoped commit/no-op | `apply_preserves_body_tags_and_is_idempotent` |
| Invalid batch | `invalid_batch_is_rejected` |
| Concurrent body/tag drift | `drift_is_detected_before_writes` |
| Wrong target/rejected judgment | `wrong_target_is_rejected`, `agent_failure_rejects_batch` |
| Post-write readback mismatch | `readback_failure_stops_writes` |
| Atomic precondition | `atomic_precondition_preserves_concurrent_edit` |
| Missing conditional snapshot | `requires_conditional_snapshot` |
| Zero-exit skipped commit | `skipped_commit_is_not_success` |
| Unsettled journal | `owned_journal_must_settle` |
| Update/commit interruption | `interrupted_update_is_not_retried`, `interrupted_commit_is_not_retried` |
| Real repository preview/apply beside another session/idempotence | `real_repository_preview_scoped_apply_and_repeat_noop` |

### Other estimate suites

`codex-spike-estimate.test.mjs` maps to
`estimate_spike_current_contract_and_body_preservation` and seven
`estimate_spike_rejects_*` tests: `missing_target`, `unknown_target`,
`wrong_known_target`, `missing_result`, `invalid_difficulty`,
`empty_justification`, and `multiline_justification`. The missing-target case
also proves that the retained failed fixture is inspected through its session
overlay, not merely committed files.

`codex-runtime-estimate-interruption.test.mjs` maps to three tests in
`codex_estimate_interruption.rs`: `estimate_sigterm_after_actual_update_requires_reconciliation`,
`estimate_sigterm_after_actual_commit_requires_reconciliation`, and
`estimate_zero_exit_commit_skipping_removed_phase_is_uncertain`. They execute
real updates/commits, inspect uncertainty and child shutdown, reconcile the owned
session, and retry with fresh evidence without consuming unrelated edits.

The former Estimate-body-note expectations are intentionally retired: the landed
contract now writes only difficulty. Their replacements require original body
and tags to remain unchanged.

## Added coverage and distribution

The review suite additionally covers explicit reviewer selection, intentionally
omitted versus failed selected AC, empty selection, parent-intent commands,
pinned phase source, approved-plan identity/association/status/drift, and the
selected child-process plan/session environment. Its minimal process/session
slice checks descendant timeout, private owned evidence, uncertain-write
finalization, and rejected identity/all-session overrides.

`codex_distribution.rs`, `cli_agent_config.rs` and core `codex_*` tests cover
foreign runtime installation/execution, generated-byte drift, project/user
paths, refresh preservation, the four-skill inventory and withheld automation,
and emitted manual read/finalize commands. These replace the Codex distribution
checks formerly required through `verify-agent-config-distribution.sh`; shell
generators remain products, not test runners.

The red-first runs reproduced the stale estimate `phaseList` interface and
removed spike `planText` input, then caught omitted-AC handling, lost reviewer/
parent context, missing approved-plan scope and leaked judgment identity before
the corresponding production repairs.

## Process, run-state, runner and queue suites → `codex_process`

`rust-test-suite-consolidation` phase 8 moved the last four suites, deleted them
and deleted CI's separate `node --test` step. Every case is a named Rust test (P)
or a documented duplicate of an existing one (D); none was retired. The full
per-case table, the negative controls and timings are in
`docs/test-migration-inventory.md` § 11.

| Previous suite | Cases | Rust module | P / D |
| --- | --- | --- | --- |
| `codex-spike-process.test.mjs` | 26 | `codex_process::transport` (25 tests) | 23 / 3 |
| `codex-runtime-state.test.mjs` | 16 | `codex_process::state` (15 tests) | 15 / 1 |
| `codex-runtime-review-process.test.mjs` | 6 | `codex_process::runner` | 6 / 0 |
| `codex-runtime-queue.test.mjs` | 2 | `codex_process::runner` | 2 / 0 |

The duplicates: the legacy `rate`, `unknown-model` and `nonzero` rejections ran
the fake's single provider-failure branch (same stderr, exit 1) as `auth`, so
`run_codex_rejects_nonzero_exit_without_diagnostics` owns all four; the
state suite's "write intent is durable; failure prevents success" case is
`codex_runtime::session_uncertain_write_prevents_success_and_records_recovery_evidence`.
Two combined cases keep one half as a new test and the other half as the existing
owner: `--all` and `--root` rejection are
`codex_runtime::session_forbids_all_session_commit_and_identity_overrides`.
The spike entrypoint's re-export identity check became a behavioural test,
`spike_entrypoint_runs_the_runtime_transport`, which imports
`codex-spike-process.mjs` and drives `runCodex` and `boundedParallel` through it.

The two opt-in mutation modes (`CODEX_QUEUE_TEST_MUTATION`,
`CODEX_REVIEW_TEST_MUTATION`) are always-on negative controls in
`codex_process::mutants`, each planting its edit in a private temp copy of the
runtime's import closure.

The review fixture supplies base/head only for code reviews; plan source pins
require an explicit phase item, covered by
`plan_review_rejects_source_revisions_without_item`. The runner tests seed their
plan repo with the Cargo-built binary under test rather than the legacy suites'
`scripts/rdm-dev.sh`, which rebuilt on every call. No Codex acceptance depends on
running or wrapping `verify-*.sh`.
