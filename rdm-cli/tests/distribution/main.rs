//! rdm's distribution boundary, from the consumer's side: what
//! `rdm agent-config` emits for Claude (`--skills` and `--plugin`) and Codex,
//! the checked-in `plugins/rdm/` and `.agents/skills/` trees kept equal to
//! their generators, the retired-engine cleanup, and the emitted review
//! engine executed in a foreign, non-Rust repo.
//!
//! Every emission and every script runs in a sandbox (temp `HOME`/XDG, no
//! inherited `RDM_*`, git config isolated) and writes only under the test's
//! temp directory; generators run from scratch copies. Node is required by
//! [`downstream`] (resolved by `rdm_devtools::workflow`, an actionable error
//! when missing, never a skip). Ported from the retired
//! `scripts/verify-{agent-config-distribution,plugin-distribution,`
//! `plugin-install}.sh`; see `docs/test-migration-inventory.md` § 8.

#[macro_use]
#[path = "../common/workflow_support.rs"]
mod workflow_support;

#[path = "../git_test_support.rs"]
mod git_test_support;

#[path = "../common/plan_fixture.rs"]
mod plan_fixture;

mod support;

mod claude_skills;
mod codex;
mod downstream;
mod plugin;
mod superseded;
