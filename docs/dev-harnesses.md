# Dev harnesses

Regression harnesses for rdm's dogfooding lane, moved verbatim out of `CLAUDE.md` by
`authoring-grammar-and-context-diet` phase 4 (a reference an agent consults occasionally,
not a directive it follows every turn). Cited from `CLAUDE.md`'s `## Dogfooding` section.

- Suite hygiene: `cargo nextest run --profile suite-hygiene` — the six `rdm-cli/tests/suite_hygiene/` tests, each a whole-suite nested `cargo nextest run` under a controlled environment, so the default profile skips them (listed as skipped) and CI runs them in a required step. `temp_hygiene::worktree_suites_leave_no_worktree_in_tmpdir` runs all of `rdm-git` + `rdm-cli` with `TMPDIR` at a scratch dir and requires no `*__worktrees` leak (`worktree::add` places a worktree beside its repo root, so a fixture rooted at its own `TempDir` leaks one — ~44k had accumulated before `c647ab0`). `git_config::hostile_git_config_changes_no_result` runs the change-review/worktree suites under a hostile `HOME`/`GIT_CONFIG_SYSTEM` (`diff.relative`, `diff.external`, `init.defaultBranch`, `core.pager`, `commit.gpgsign`, …) and requires every test to pass, the external diff driver never to run, and no leak. `canary::hook_git_env_reaches_no_repository` runs the whole workspace with a git hook's `GIT_DIR`/`GIT_INDEX_FILE` pointed at a throwaway canary repo and requires every byte of it unchanged (the 2026-09-24 `core.bare` incident); `canary::an_unscrubbed_git_init_flips_the_canary` is its negative control. `mutants::tempdir_rooted_fixture_leaks_a_worktree` and `mutants::stripped_git_config_isolation_fails_the_hostile_run` plant the two regressions in a working-tree mirror under `target/tmp/rdm-mutants/suite-hygiene/` (never the checkout) and require the checks to see them. Run it after touching `worktree_path`/`add` in `rdm-git/src/worktree.rs`, any worktree fixture, either `git_test_support.rs`, `rdm-git/src/source.rs`'s `unified_diff_argv` guards, or how any test spawns git. Cost: about 70 s warm on a developer machine (three nested suite runs); the mirror's first build adds a cold `rdm-cli` build. Measurements: `docs/test-migration-inventory.md` § 10.
- Rendered-listing observer: `bash scripts/observe-workflow-listing.sh` — NON-hermetic (needs the `claude` CLI): captures the real skill/slash-command listing from a fresh `claude -p` rooted here and asserts the `rdm-wf-` engine-prefix contract against it. Run it deliberately when engine names change; it is not CI-run. Its prior hermetic `--self-test-only` half was retired by the operator amendment to `retire-static-grep-harnesses` (2026-09-23) — it exercised no `claude` CLI, nothing real. See `docs/workflow-schemas.md` § "Observing the rendered listing".

## Retired shell harnesses and their nextest owners

These harnesses were ported to Rust by the `rust-test-suite-consolidation` roadmap and deleted; `cargo nextest run` runs their replacements. The section-by-section map is `docs/test-migration-inventory.md`.

| Retired harness | Now |
|---|---|
| `verify-workflow-review.sh`, `verify-workflow-review-outcome.sh` | `cargo nextest run -p rdm-cli --test workflow_review` (Node binding: `rdm-devtools/src/workflow.rs`) |
| `verify-workflow-backlog.sh`, `verify-workflow-document.sh`, `verify-workflow-estimate.sh` | `cargo nextest run -p rdm-cli --test workflow_passes` |
| `verify-skill-autopilot.sh` | `cli_phase::reviewed_and_blocked_reason_read_back_as_json`, `cli_hook::done_line_amended_onto_branch_tip_completes_after_ff_merge` |
| `verify-token-report.sh`, `verify-refuter-agreement.sh` | `cargo nextest run -p rdm-devtools` (`rdm-measure`) |
| `verify-agent-config-distribution.sh`, `verify-plugin-distribution.sh`, `verify-plugin-install.sh` | `cargo nextest run -p rdm-cli --test distribution` |
| `verify-plugin-loop.sh`, `verify-claude-code-web-loop.sh`, `verify-backlog-groom-loop.sh`, `verify-review-revision-loop.sh`, `verify-worktree-review-loop.sh` | `cargo nextest run -p rdm-cli --test cli_loops` |
| `verify-golden-json.sh` (+ `capture-golden.sh`) | `cargo nextest run -p rdm-cli --test golden_json`; re-bless with `--run-ignored only -E 'test(=bless)'` |
| `verify-rdm-plan-fixture.sh` | retired with the shell fixture library it tested |
| `verify-session-identity.sh`, `verify-scoped-commit.sh`, `verify-lost-update.sh`, `verify-journal-truncation-race.sh` | `cargo nextest run -p rdm-cli --test concurrency` (negative controls in `mutants::`, serialised by the nextest group `rdm-mutant-builds`) |
| `verify-worktree-temp-hygiene.sh`, `verify-git-config-isolation.sh` | `cargo nextest run --profile suite-hygiene` (the `rdm-cli/tests/suite_hygiene/` binary) |

## Exceptions to `cargo nextest run`

`cargo nextest run` is the contributor acceptance command. The small set of checks it does not run — each justified in `docs/test-migration-inventory.md` § 10 — is:

- `cargo nextest run --profile suite-hygiene`: whole-suite nested runs (recursion and cost keep them out of the default profile); CI-required.
- `cargo test --doc --workspace`: nextest does not run doctests; CI and the hk pre-commit hook run it.
- Live observations that need a real client and report non-execution distinctly: `scripts/observe-plugin-install.sh`, `scripts/observe-workflow-listing.sh`, `rdm-smoke codex-coexistence` (the ignored `codex_coexistence_live` test), and the opt-in Codex spike runner.
- Non-test gates: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `shellcheck`, `shfmt`, the feature-matrix `cargo check`s, the release build and `cargo deny check`.
- Deliberate `#[ignore]` writers, not regressions: `golden_json::bless` and `rdm-core`'s `regenerate_raw_skills_baseline`.
