//! Repository-only measurement tools behind the `rdm-measure` binary.
//!
//! These replace the JavaScript instruments that measured the autonomous
//! lane's token spend and graded refuter behaviour:
//!
//! | Subcommand | Replaces | Module |
//! |---|---|---|
//! | `lane-tokens` | `scripts/measure-lane-tokens.mjs` + `scripts/lib/token-report.mjs` | [`lane_tokens`], [`sidecar`] |
//! | `refuter-severity` | `scripts/measure-refuter-severity.mjs` | [`refuter_severity`] |
//! | `mine-refuter-corpus` | `scripts/mine-refuter-corpus.mjs` | `refuter_agreement::miner` |
//! | `refuter-agreement` | `scripts/run-refuter-agreement.mjs` + `scripts/lib/refuter-agreement.mjs` | `refuter_agreement` |
//!
//! Measurement orchestration, parsing, scoring and reporting are Rust. The
//! few canonical workflow decisions a measurement replays (ranking, gating,
//! the refuter prompt, the dimension table, the non-gating severity set) are
//! called in the real `.claude/workflows/lib/review.mjs` through
//! [`review_rules`], never copied; only those paths need Node.
//!
//! [`jsnum`], [`jsjson`] and [`jsdate`] reproduce the JavaScript number
//! formatting, JSON key order and `Date.parse` the committed figures were
//! produced with, so the golden outputs captured from the JS tools are the
//! regression evidence.

pub mod args;
pub mod jsdate;
pub mod jsjson;
pub mod jsnum;
pub mod lane_tokens;
pub mod refuter_severity;
pub mod review_rules;
pub mod sidecar;
