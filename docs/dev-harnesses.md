# Dev harnesses

Regression harnesses for rdm's dogfooding lane, moved verbatim out of `CLAUDE.md` by
`authoring-grammar-and-context-diet` phase 4 (a reference an agent consults occasionally,
not a directive it follows every turn). Cited from `CLAUDE.md`'s `## Dogfooding` section.

- Worktree temp-hygiene harness: `bash scripts/verify-worktree-temp-hygiene.sh` — hermetic regression (~86s) proving no worktree-creating test leaks a worktree into the system temp dir. `worktree::add` places a worktree at `repo_root.parent()/<name>__worktrees/`, so a fixture whose git root IS its `TempDir` puts that sibling where `TempDir::drop` never reaches it — this had accumulated ~44k populated worktrees. Redirects `TMPDIR` at a scratch dir so the count is its own, and carries a planted-mutation self-test (re-root a fixture at its `TempDir`, assert the leak reappears) so the check can never pass vacuously. Run it after touching `worktree_path`/`add` in `rdm-git/src/worktree.rs` or any worktree fixture (`rdm-cli/tests/{cli_worktree,cli_gate,cli_verify}.rs`, `rdm-git/tests/worktree.rs`).
- Git-config isolation harness: `bash scripts/verify-git-config-isolation.sh` — hermetic regression proving the change-review/worktree test suites (`rdm-git/src/{source,worktree}.rs`'s unit tests, `rdm-git/tests/worktree.rs`, and `rdm-cli/tests/{cli_worktree,cli_gate,cli_verify,cli_review_change}.rs`) produce identical results under a clean environment and one seeded with several real settings a developer's `~/.gitconfig` carries (`diff.relative`, `init.defaultBranch`, `advice.detachedHead`, `core.pager`, `commit.gpgsign`) — the shape of the real report behind `2c55784`'s `--no-relative`/`:(top)` fix. Every fixture git call routes through `rdm-git/src/git_test_support.rs` / `rdm-cli/tests/git_test_support.rs` (`git`/`git_with_global`/`write_global_config`), which isolate `GIT_CONFIG_GLOBAL`/`GIT_CONFIG_SYSTEM` to `/dev/null` by default with an explicit opt-in for a scenario that wants to inject a specific hostile setting; a planted-mutation self-test strips that isolation and asserts the hostile run diverges. Its final step re-runs `verify-worktree-temp-hygiene.sh` under a milder hostile environment, confirming `c647ab0`'s leak-free guarantee holds there too. Run it after touching either `git_test_support.rs`, any file that includes one of them, or `rdm-git/src/source.rs`'s `unified_diff_argv` guards.
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
