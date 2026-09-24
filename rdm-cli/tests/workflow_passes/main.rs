//! Rust-driven component tests for the batched workflow passes: backlog
//! grooming (`lib/backlog.mjs`, `rdm-wf-backlog.js`), roadmap documentation
//! (`lib/document.mjs`, `rdm-wf-document.js`) and phase estimation
//! (`lib/estimate.mjs`, `rdm-wf-estimate.js`), executed under Node through
//! `rdm_devtools::workflow`.
//!
//! Rust owns every scenario, scripted fake-agent reply, expected value and
//! assertion; the JavaScript under test is the real canonical module or the
//! real `.claude/workflows` engine. Every command a pass returns or names is
//! executed against `CARGO_BIN_EXE_rdm` in a per-test plan repo (see
//! `plan_fixture.rs`). No live agent, no `claude`, no network. See
//! `docs/test-migration-inventory.md` § "Phase 3" for the mapping from the
//! retired shell suites and JavaScript tests.

#[macro_use]
#[path = "../common/workflow_support.rs"]
mod workflow_support;

#[path = "../git_test_support.rs"]
mod git_test_support;

#[path = "../common/plan_fixture.rs"]
mod plan_fixture;

mod generators;

mod backlog;
mod document;
mod estimate;
