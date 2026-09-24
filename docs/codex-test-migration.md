# Codex regression test ownership

Phase 4 moves the affected runtime/spike scenarios into individually discoverable
Rust integration tests. `cargo nextest run` is the acceptance entrypoint. Node.js
24 (pinned by `.mise.toml`) executes the actual production JavaScript, not a second
test suite. Missing Node fails with an installation instruction; it never skips.

```sh
cargo nextest run -p rdm-cli --test codex_runtime --test codex_estimate --test codex_estimate_interruption --test codex_distribution --test cli_agent_config
cargo nextest run -p rdm-core -E 'test(codex_)'
```

Rust owns temporary source/plan repositories, Cargo-resolved binaries, expected
values, scripted judgments, assertions, isolated configuration, deadlines and
process cleanup. `rdm-cli/tests/support/codex-bridge.mjs` only loads/invokes modules,
hydrates callback/host primitives, and transports JSON replies/errors. The
interruption tests compile a test-only Rust host fixture; no live model,
authentication, globally installed RDM, or user configuration is involved.

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

## Deliberately retained legacy coverage

CI explicitly lists `codex-spike-process.test.mjs`, `codex-runtime-state.test.mjs`,
`codex-runtime-review-process.test.mjs`, and `codex-runtime-queue.test.mjs` in a
**remaining legacy** step. Those complete case sets have not yet moved to Rust;
the new minimal process/session slice is not a claim of full equivalence. Remove
that step only when the owning migration accounts for every remaining case.
The retained review-process fixture supplies base/head only for code reviews;
plan source pins now require an explicit phase item, covered by
`plan_review_rejects_source_revisions_without_item`.
The older smoke/dev-wrapper checks and unrelated repository shell harnesses
also remain outside this phase's migration. No new Codex acceptance depends on
running or wrapping `verify-*.sh`, and none is run to validate this phase.
