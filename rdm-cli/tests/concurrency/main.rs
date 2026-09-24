//! Session identity, scoped commits and the concurrency races, driven through
//! real, separate `rdm` processes — the binary under test, or an isolated
//! mutant build of it — against per-test temp plan repos: rung-2 leases and
//! rung-3 derivation ([`session_identity`]), the changeset journal under
//! concurrent writers ([`journal_race`]), content-checked flushes, commits and
//! deletes ([`lost_update`]), session-scoped commit/status/discard
//! ([`scoped_commit`]), and one negative control per planted regression
//! ([`mutants`], built by [`mutant`] in a working-tree [`mirror`]).
//!
//! Rust owns every fixture, spawn, barrier release, wait, assertion and
//! teardown ([`support`]); `sh` appears only as the modelled process topology.
//! Ported from the retired `scripts/verify-{session-identity,scoped-commit,`
//! `lost-update,journal-truncation-race}.sh`; see
//! `docs/test-migration-inventory.md` § 9.

#[macro_use]
#[path = "../common/workflow_support.rs"]
mod workflow_support;

#[path = "../git_test_support.rs"]
mod git_test_support;

#[path = "../common/plan_fixture.rs"]
mod plan_fixture;

#[path = "../common/mirror.rs"]
mod mirror;

mod journal_race;
mod lost_update;
mod mutant;
mod mutants;
mod scoped_commit;
mod session_identity;
mod support;
