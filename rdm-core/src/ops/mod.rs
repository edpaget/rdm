//! Domain operations for plan repo entities.
//!
//! Each sub-module groups the CRUD operations for one entity type.
//! All functions take `&impl Store` or `&mut impl Store` and are
//! usable with any [`Store`](crate::store::Store) implementation.

/// Index generation operations.
pub mod index;
/// Plan repo initialization.
pub mod init;
/// Next-actionable-phase selection for a single roadmap.
pub mod next;
/// Phase operations: list, create, update, remove, resolve.
pub mod phase;
/// Project operations: create, list.
pub mod project;
/// Review operations: enumerate items awaiting review.
pub mod review;
/// Review-document operations: create, comment, submit, transition, list,
/// delete.
pub mod reviews;
/// Roadmap operations: create, update, delete, list, archive, split, dependencies.
pub mod roadmap;
/// Task operations: create, update, list, promote.
pub mod task;
/// Request types for the `update` operations (`BodyUpdate` / `TitleUpdate` /
/// `TagsUpdate` / `PriorityUpdate` / `DifficultyUpdate` / `ModelTierUpdate` /
/// `ReasonUpdate`).
pub mod update;

pub use phase::CreatePhase;
pub use reviews::CreateReview;
pub use roadmap::CreateRoadmap;
pub use task::CreateTask;
pub use update::{
    BodyUpdate, DifficultyUpdate, ModelTierUpdate, PriorityUpdate, ReasonUpdate, TagsUpdate,
    TitleUpdate,
};

use crate::error::Result;
use crate::store::Store;

/// Runs a mutating operation as a single transaction: applies `f`, regenerates
/// the project's `INDEX.md`, then commits exactly once.
///
/// This is the one seam every frontend (CLI, server, MCP) should route
/// mutations through. The mutating ops in this module are *commit-free* — they
/// stage their writes but never commit — so `mutate` owns both the derived
/// `INDEX.md` invariant and the single commit. Callers never name
/// [`index::generate_index_for_project`] themselves; forgetting it (and thus
/// shipping a stale index) is no longer possible.
///
/// `f` receives the store and performs the entity write (e.g.
/// [`roadmap::create_roadmap`]). Because reads observe staged writes, the index
/// regeneration sees the mutation `f` just staged. The single trailing commit
/// is staging-aware: under staging mode the backend defers it like any other.
///
/// # Errors
///
/// Propagates any error from `f`, from index regeneration, or from the commit.
/// On error nothing is committed; the caller may discard the staged changes.
pub fn mutate<S: Store, T>(
    store: &mut S,
    project: &str,
    f: impl FnOnce(&mut S) -> Result<T>,
) -> Result<T> {
    let out = f(store)?;
    index::generate_index_for_project(store, project)?;
    store.commit()?;
    Ok(out)
}

/// The outcome of a [`mutate_batch`] call: every step's individual result,
/// plus the result of the shared finalize stage (index regeneration + the
/// single commit).
///
/// `mutate_batch` is deliberately infallible at the top level and returns
/// this struct unconditionally: a failure in the *shared* finalize stage
/// must not swallow the per-step outcomes, because callers (the `Done:`
/// hook path) log each step's result individually and that observability
/// must survive a finalize failure. Callers are expected to consume
/// `step_results` first (e.g. for logging), then inspect and propagate
/// `finalize_result`.
pub struct BatchOutcome {
    /// Per-step results, in the same order as the `steps` passed in.
    pub step_results: Vec<Result<()>>,
    /// Result of the finalize stage: index regeneration (if any step
    /// succeeded) followed by the single commit. `Ok(())` when both
    /// succeeded or when there was nothing to do.
    pub finalize_result: Result<()>,
}

/// A single step in a [`mutate_batch`] call: a boxed, one-shot mutation
/// closure over the store, normalized to return `Result<()>` regardless of
/// what the underlying operation (e.g. [`phase::update_phase`],
/// [`task::update_task`]) actually returns.
pub type BatchStep<'a, S> = Box<dyn FnOnce(&mut S) -> Result<()> + 'a>;

/// Applies a batch of independent mutating operations as a single
/// transaction: runs every step in `steps`, in order, **without**
/// short-circuiting on an individual step's error (unlike [`mutate`]) — one
/// bad step must not block the rest, matching the hook directive path's
/// long-standing skip-and-continue contract. Regenerates the project's
/// `INDEX.md` exactly once (only if at least one step succeeded), then
/// commits exactly once, using the message returned by `message` — which is
/// called with the full, in-order slice of per-step results, so the message
/// can name only the steps that actually succeeded.
///
/// Each step's return type is normalized to `Result<()>`: batched steps are
/// typically heterogeneous (e.g. a phase update and a task update in the same
/// batch, which return different `Document<_>` types), and the caller of
/// `mutate_batch` never needs the returned document — only pass/fail plus
/// whatever data it already captured in its own closure/metadata to build the
/// commit message and its own logging.
///
/// This function never returns `Err` itself — all failure reporting flows
/// through the returned [`BatchOutcome`]. Individual step failures land in
/// `step_results`; an index-regeneration or commit failure lands in
/// `finalize_result`. When `finalize_result` is `Err`, **none** of the
/// batch's successful steps are committed — they remain staged (see
/// [`Store::commit_with_message`]/[`Store::commit`]'s semantics) rather than
/// partially landing, but `step_results` still faithfully reports which
/// steps' mutations were applied (staged). This is an intentional
/// consequence of collapsing the batch into a single transaction — under
/// the old per-directive-commit model, an unrelated later failure could not
/// undo an earlier directive's already-committed success; under batching it
/// can, in this one specific failure mode (e.g. index regen erroring due to
/// unrelated corrupt data elsewhere in the project).
pub fn mutate_batch<'a, S: Store>(
    store: &mut S,
    project: &str,
    steps: Vec<BatchStep<'a, S>>,
    message: impl FnOnce(&[Result<()>]) -> String,
) -> BatchOutcome {
    let mut step_results = Vec::with_capacity(steps.len());
    for step in steps {
        step_results.push(step(store));
    }
    let finalize_result = (|| {
        if step_results.iter().any(Result::is_ok) {
            index::generate_index_for_project(store, project)?;
        }
        let msg = message(&step_results);
        store.commit_with_message(&msg)
    })();
    BatchOutcome {
        step_results,
        finalize_result,
    }
}
