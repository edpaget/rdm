//! Suite hygiene: properties of the *whole* test suite, each proved by
//! running it — nested `cargo nextest run` invocations ([`nested`]) under a
//! controlled environment.
//!
//! - [`temp_hygiene`]: no worktree-creating test leaks a worktree into the
//!   temp directory.
//! - [`git_config`]: a hostile global/system git config changes no result.
//! - [`canary`]: a run under a git hook's repository-locating environment
//!   reaches no repository but the throwaway canary it names — the 2026-09-24
//!   `core.bare` incident — plus its negative control.
//! - [`mutants`]: the negative controls for the first two, planted in a
//!   working-tree [`mirror`] and run there; the checkout is never written.
//!
//! Every test here runs a whole-suite nested run, so this binary is excluded
//! from nextest's default profile and runs, serially, in the `suite-hygiene`
//! profile: `cargo nextest run --profile suite-hygiene` (CI runs it as a
//! required step). A nested run uses the default profile, so it can never
//! recurse into this binary. Under plain `cargo test` it still runs
//! correctly, just slowly. Ported from the retired
//! `scripts/verify-{worktree-temp-hygiene,git-config-isolation}.sh`; see
//! `docs/test-migration-inventory.md` § 10.

#[macro_use]
#[path = "../common/workflow_support.rs"]
mod workflow_support;

#[path = "../git_test_support.rs"]
mod git_test_support;

#[path = "../common/plan_fixture.rs"]
mod plan_fixture;

#[path = "../common/mirror.rs"]
mod mirror;

mod canary;
mod git_config;
mod mutants;
mod nested;
mod temp_hygiene;
