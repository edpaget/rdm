//! Rust-driven component tests for the review workflow (`.claude/workflows/`
//! `lib/review.mjs`, `lib/plan-review.mjs`, and the `rdm-wf-review-refute-fix`
//! and `rdm-wf-plan-review` engines), executed under Node through
//! `rdm_devtools::workflow`.
//!
//! Rust owns every scenario, scripted fake-agent reply, expected value and
//! assertion; the JavaScript under test is the real source (or an isolated
//! mutant copy of it). Persistence and gates run against the real `rdm`
//! binary. See `docs/test-migration-inventory.md` § "Phase 2" and § "Phase 3"
//! for the mapping from the retired shell suites and JavaScript tests
//! (`driver` and `plan_driver` are phase 3's; `effort` is phase 7's port of
//! `scripts/lib/review-effort.test.mjs`).

#[macro_use]
#[path = "../common/workflow_support.rs"]
mod workflow_support;

mod support;

#[path = "../git_test_support.rs"]
mod git_test_support;

#[path = "../common/plan_fixture.rs"]
mod plan_fixture;

mod budget;
mod consolidate;
mod coverage;
mod driver;
mod effort;
mod engine;
mod generators;
mod outcome;
mod persist;
mod pipeline;
mod plan;
mod plan_driver;
