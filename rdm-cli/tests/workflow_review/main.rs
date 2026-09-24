//! Rust-driven component tests for the review workflow (`.claude/workflows/`
//! `lib/review.mjs`, `lib/plan-review.mjs` and the `rdm-wf-review-refute-fix`
//! engine), executed under Node through `rdm_devtools::workflow`.
//!
//! Rust owns every scenario, scripted fake-agent reply, expected value and
//! assertion; the JavaScript under test is the real source (or an isolated
//! mutant copy of it). Persistence and gates run against the real `rdm`
//! binary. See `docs/test-migration-inventory.md` § "Phase 2" for the mapping
//! from the retired shell suites.

#[macro_use]
mod support;

#[path = "../git_test_support.rs"]
mod git_test_support;

mod budget;
mod coverage;
mod engine;
mod generators;
mod outcome;
mod persist;
mod pipeline;
mod plan;
