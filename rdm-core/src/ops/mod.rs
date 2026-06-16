//! Domain operations for plan repo entities.
//!
//! Each sub-module groups the CRUD operations for one entity type.
//! All functions take `&impl Store` or `&mut impl Store` and are
//! usable with any [`Store`](crate::store::Store) implementation.

/// Index generation operations.
pub mod index;
/// Plan repo initialization.
pub mod init;
/// Phase operations: list, create, update, remove, resolve.
pub mod phase;
/// Project operations: create, list.
pub mod project;
/// Roadmap operations: create, update, delete, list, archive, split, dependencies.
pub mod roadmap;
/// Task operations: create, update, list, promote.
pub mod task;

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
