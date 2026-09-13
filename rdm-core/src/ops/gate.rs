//! The write-time **`reviewed` transition gate**.
//!
//! `docs/subagent-dispatch-enforcement.md` records why this lives in core
//! rather than in prose: a weaker model collapses the planner, reviewer and
//! implementer roles inline, and prose cannot prevent it. If rdm refuses the
//! terminal write unless the review records exist, a model that skipped a gate
//! cannot complete the phase without forging a review — and the trail shows
//! exactly what happened.
//!
//! [`check_reviewed_gate`] evaluates three preconditions in a **fixed order**,
//! so the first failure reported is always the most specific one an operator
//! can act on:
//!
//! 1. **(a)** an `approved` implementation plan implements the item;
//! 2. **(b)** an approving `change/` review's `implements` names that same
//!    plan;
//! 3. **(c)** the item's worktree, if [`rdm worktree`](crate::worktree) knows
//!    one, is clean.
//!
//! `--override-gate` waives **(a) and (b) only** — cleanliness always applies,
//! because a dirty worktree means the reviewed code is not the committed code
//! and no operator intent can make that untrue.
//!
//! The gate is opt-in behind the repo-only `gates.reviewed` config key and
//! defaults to `false`. See `docs/core-enforced-gates.md`.

use crate::error::{Error, Result};
use crate::link::ItemRef;
use crate::model::GateOverride;
use crate::store::Store;
use crate::worktree::WorktreeProbe;

/// An operator's request to bypass the gate's record preconditions.
#[derive(Debug, Clone, Copy)]
pub struct GateOverrideRequest<'a> {
    /// Why the gate is being bypassed. Empty or whitespace-only is rejected.
    pub reason: &'a str,
    /// Who is bypassing it.
    pub actor: &'a str,
}

/// The gate configuration one status write is evaluated against.
///
/// Construct it with [`ReviewedGate::disabled`] (the default when
/// `gates.reviewed` is unset) or [`ReviewedGate::enforcing`], optionally
/// adding an operator bypass with [`ReviewedGate::with_override`].
pub struct ReviewedGate<'a> {
    enabled: bool,
    probe: Option<&'a dyn WorktreeProbe>,
    over: Option<GateOverrideRequest<'a>>,
}

impl std::fmt::Debug for ReviewedGate<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReviewedGate")
            .field("enabled", &self.enabled)
            .field("has_probe", &self.probe.is_some())
            .field("override", &self.over)
            .finish()
    }
}

impl<'a> ReviewedGate<'a> {
    /// A gate that never fires — every `reviewed` write is
    /// [`GateDecision::NotApplicable`].
    ///
    /// This is what every caller gets when `gates.reviewed` is unset, which is
    /// the default, so enabling core-enforced gates is always a deliberate
    /// act.
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            probe: None,
            over: None,
        }
    }

    /// A gate that enforces all three preconditions.
    ///
    /// `probe` is the worktree port precondition (c) reads through. `None`
    /// means the caller has no worktree context at all (the HTTP server, the
    /// MCP server, a unit test), which skips (c) — the same documented
    /// fail-open the "if `rdm worktree` knows one" clause already carries.
    #[must_use]
    pub fn enforcing(probe: Option<&'a dyn WorktreeProbe>) -> Self {
        Self {
            enabled: true,
            probe,
            over: None,
        }
    }

    /// Attaches an operator bypass of preconditions (a) and (b).
    #[must_use]
    pub fn with_override(mut self, reason: &'a str, actor: &'a str) -> Self {
        self.over = Some(GateOverrideRequest { reason, actor });
        self
    }

    /// Whether an operator bypass was requested.
    #[must_use]
    pub fn has_override(&self) -> bool {
        self.over.is_some()
    }

    /// Whether the gate is enforcing at all.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// What [`check_reviewed_gate`] concluded about one `reviewed` write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateDecision {
    /// The gate did not apply — it is disabled, or the write is not a
    /// transition to `reviewed`.
    NotApplicable,
    /// Every precondition held. Carries the records that satisfied it, so a
    /// caller can report *why* the write was allowed.
    Satisfied {
        /// Slug of the approved plan that satisfied (a).
        plan: String,
        /// Id of the approving change review that satisfied (b).
        review_id: String,
    },
    /// An operator bypassed (a) and (b); (c) still held.
    Overridden(GateOverride),
}

/// Evaluates the `reviewed` transition gate for `item`.
///
/// The caller is responsible for only invoking this on a write whose status is
/// `Some(Reviewed)`; this function does not inspect the status itself, because
/// the phase and task status enums are distinct types.
///
/// Reads go through the same `store` the write does, so an approved plan or an
/// approving review created earlier in the **same uncommitted session** counts
/// — an agent never has to `rdm commit` mid-dispatch just to satisfy its own
/// gate.
///
/// # Errors
///
/// In the fixed order (a) → (b) → (c):
///
/// - [`Error::GateOverrideEmptyReason`] if an override carries no reason
///   (checked first — a malformed bypass is never silently honored);
/// - [`Error::GateNoApprovedPlan`] when no `approved` plan implements `item`;
/// - [`Error::GateNoApprovedChangeReview`] when no approving `change/` review
///   names any of those plans, listing every plan checked;
/// - [`Error::GateWorktreeDirty`] when the item's worktree has uncommitted
///   changes;
/// - [`Error::GateWorktreeUnobservable`] when the worktree could not be
///   inspected at all (fail-closed);
/// - anything [`crate::ops::plan::approved_plans_for`] or
///   [`crate::ops::plan::approving_change_reviews_for_plan`] returns.
pub fn check_reviewed_gate(
    store: &impl Store,
    project: &str,
    item: &ItemRef,
    gate: &ReviewedGate<'_>,
) -> Result<GateDecision> {
    if !gate.enabled {
        return Ok(GateDecision::NotApplicable);
    }

    // A malformed bypass is rejected before anything else: recording an empty
    // reason would leave an audit trail that explains nothing.
    let over = match gate.over {
        Some(req) if req.reason.trim().is_empty() => {
            return Err(Error::GateOverrideEmptyReason);
        }
        other => other,
    };

    let satisfied = if over.is_some() {
        // (a) and (b) waived. (c) below still runs.
        None
    } else {
        // (a) an approved plan implements the item.
        let approved = crate::ops::plan::approved_plans_for(store, project, item)?;
        if approved.is_empty() {
            return Err(Error::GateNoApprovedPlan(item.label()));
        }
        // (b) an approving `change/` review's `implements` names one of them.
        let mut hit: Option<(String, String)> = None;
        for (slug, _) in &approved {
            let reviews =
                crate::ops::plan::approving_change_reviews_for_plan(store, project, slug)?;
            if let Some((review_id, _)) = reviews.into_iter().next() {
                hit = Some((slug.clone(), review_id));
                break;
            }
        }
        let Some((plan, review_id)) = hit else {
            return Err(Error::GateNoApprovedChangeReview {
                item: item.label(),
                plans: approved.into_iter().map(|(slug, _)| slug).collect(),
            });
        };
        Some((plan, review_id))
    };

    // (c) the worktree, if rdm knows one, is clean. Never waived by an
    // override: a dirty worktree means the reviewed code is not the committed
    // code, which no operator intent can make untrue.
    if let Some(probe) = gate.probe {
        match probe.worktree_for(item) {
            // rdm manages no worktree for this item — the "if `rdm worktree`
            // knows one" escape clause. Skip (c).
            Ok(None) => {}
            Ok(Some(check)) if check.is_clean() => {}
            Ok(Some(check)) if !check.observable => {
                return Err(Error::GateWorktreeUnobservable {
                    item: item.label(),
                    cause: format!(
                        "unreadable `git status --porcelain` output for {}",
                        check.path
                    ),
                });
            }
            Ok(Some(check)) => {
                return Err(Error::GateWorktreeDirty {
                    path: check.path,
                    paths: check.dirty,
                    truncated: check.truncated,
                });
            }
            // Fail-closed: an unobservable worktree is never a clean one.
            Err(e) => {
                return Err(Error::GateWorktreeUnobservable {
                    item: item.label(),
                    cause: e.to_string(),
                });
            }
        }
    }

    Ok(match (over, satisfied) {
        (Some(req), _) => GateDecision::Overridden(GateOverride {
            reason: req.reason.to_string(),
            actor: req.actor.to_string(),
            at: chrono::Local::now().date_naive(),
        }),
        (None, Some((plan, review_id))) => GateDecision::Satisfied { plan, review_id },
        // Unreachable: `over` is None exactly when `satisfied` is Some.
        (None, None) => GateDecision::NotApplicable,
    })
}
