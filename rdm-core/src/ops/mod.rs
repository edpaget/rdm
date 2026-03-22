//! Domain operations for plan repo entities.
//!
//! Each sub-module groups the CRUD operations for one entity type.
//! All functions take `&impl Store` or `&mut impl Store` and are
//! usable without a [`PlanRepo`](crate::repo::PlanRepo).

/// Phase operations: list, create, update, remove, resolve.
pub mod phase;
/// Project operations: create, list.
pub mod project;
/// Roadmap operations: create, update, delete, list, archive, split, dependencies.
pub mod roadmap;
/// Task operations: create, update, list, promote.
pub mod task;
