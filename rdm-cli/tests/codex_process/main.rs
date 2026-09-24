//! The Codex runtime's process, run-state, whole-runner and queue contracts,
//! driven through the real production JavaScript under Node:
//!
//! - [`transport`]: `scripts/lib/codex-process.mjs` (`runCodex`,
//!   `validateSchema`, `boundedParallel`), plus one run through the
//!   `scripts/lib/codex-spike-process.mjs` entrypoint that re-exports it;
//! - [`state`]: `scripts/lib/codex-runtime-state.mjs` (`createRun`: owned
//!   identity, durable evidence, direct bounded `rdm` commands);
//! - [`runner`]: `scripts/lib/codex-runtime.mjs` (`runRuntime`) and the
//!   documented CLI `scripts/rdm-codex.mjs`, over a plan repo seeded with the
//!   binary under test — code review, plan review and the estimate queue;
//! - [`mutants`]: the two negative controls the legacy suites ran opt-in, a
//!   planted edit in a private copy of the runtime that the positive checks
//!   must catch.
//!
//! Rust owns every fixture, fake-binary script, response, wait, assertion and
//! teardown ([`support`]). The fakes are POSIX `sh` that record each call and
//! replay a response Rust wrote; they decide nothing about the scenario. The
//! JavaScript is driven through `rdm_devtools::workflow::Host` (held callback
//! replies, detached calls, a real `AbortController`), not
//! `tests/support/codex-bridge.mjs`, which answers one request per process and
//! cannot act while a call is in flight. Ported from the retired
//! `scripts/lib/codex-{spike-process,runtime-state,runtime-review-process,`
//! `runtime-queue}.test.mjs`; the case map is `docs/test-migration-inventory.md`
//! § 11.

#[macro_use]
#[path = "../common/workflow_support.rs"]
mod workflow_support;

#[path = "../git_test_support.rs"]
mod git_test_support;

#[path = "../common/plan_fixture.rs"]
mod plan_fixture;

mod mutants;
mod runner;
mod state;
mod support;
mod transport;
