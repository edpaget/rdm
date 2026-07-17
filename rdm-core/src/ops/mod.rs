//! Domain operations for plan repo entities.
//!
//! Each sub-module groups the CRUD operations for one entity type.
//! All functions take `&impl Store` or `&mut impl Store` and are
//! usable with any [`Store`](crate::store::Store) implementation.

/// Read-only backlog grooming report: stale tasks, duplicate clusters, tag
/// clusters, archivable roadmaps.
pub mod backlog;
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
/// regeneration sees the mutation `f` just staged. The single trailing
/// [`Store::commit`] flushes the staged writes to disk without creating a git
/// commit; landing a real commit is the separate, explicit responsibility of
/// the store's commit-now pathway.
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
    /// succeeded) followed by flushing the batch to the store. `Ok(())` when
    /// both succeeded or when there was nothing to do.
    ///
    /// For a git-backed store this flush does **not** create a git commit —
    /// [`Store::commit`] only ever flushes to disk. Callers that need the
    /// batch to land as a single real commit inspect [`Self::commit_message`]
    /// and pass it to the store's blessed always-commit pathway themselves.
    pub finalize_result: Result<()>,
    /// The commit message for the batch, produced by the `message` closure
    /// passed to [`mutate_batch`]. `Some` iff at least one step succeeded (and
    /// there is therefore something to commit); `None` when every step failed
    /// or the batch was empty, so callers land no empty commit.
    pub commit_message: Option<String>,
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
/// `INDEX.md` exactly once (only if at least one step succeeded), then flushes
/// the batch to the store exactly once. The single commit message is computed
/// (only if at least one step succeeded) by calling `message` with the full,
/// in-order slice of per-step results, so the message can name only the steps
/// that actually succeeded; it is returned in [`BatchOutcome::commit_message`]
/// for the caller to land as one real commit via the store's blessed
/// always-commit pathway. Flushing here never creates a git commit — a
/// git-backed [`Store::commit`] only writes to disk.
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
/// batch's successful steps are flushed — they remain uncommitted (see
/// [`Store::commit`]'s flush-only semantics) rather than
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
    let any_ok = step_results.iter().any(Result::is_ok);
    // Compute the batch commit message iff there is something to commit,
    // calling the caller's closure exactly once so it can name only the
    // successful steps. The caller lands the real commit via the store's
    // blessed always-commit pathway.
    let commit_message = any_ok.then(|| message(&step_results));
    let finalize_result = (|| {
        if any_ok {
            index::generate_index_for_project(store, project)?;
        }
        // Flush the batch to disk only. For a git-backed store this creates no
        // git commit; the caller commits via `commit_message`.
        store.commit()
    })();
    BatchOutcome {
        step_results,
        finalize_result,
        commit_message,
    }
}
