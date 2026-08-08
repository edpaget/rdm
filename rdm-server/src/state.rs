use std::path::{Path, PathBuf};
use std::sync::Arc;

use rdm_core::config::QuickFilter;
use rdm_core::store::VersionedStore;
use rdm_store_fs::FsStore;

use crate::templates::{QuickFilterView, quick_filter_views};

/// Constructs a fresh [`VersionedStore`] instance rooted at the given path.
///
/// Called once per request/lookup by [`AppState::store`]; defaults to
/// [`default_store_factory`], which prefers a real git-backed
/// [`rdm_store_git::GitStore`] when the plan root is a git repository
/// (built with the `git` feature) and falls back to [`FsStore`] otherwise.
/// Tests may override it via [`AppState::with_store_factory`] for
/// deterministic control over which backend serves the `?at=<sha>`
/// historical-read path.
pub type StoreFactory = Arc<dyn Fn(&Path) -> Box<dyn VersionedStore + Send + Sync> + Send + Sync>;

/// Shared application state for the rdm server.
#[derive(Clone)]
pub struct AppState {
    /// Root path of the plan repository.
    pub plan_root: PathBuf,
    /// Quick-filter chips configured for HTML list views.
    ///
    /// Resolved by the CLI from `[server.quick_filters]` in `rdm.toml`,
    /// `RDM_SERVER_QUICK_FILTERS` env var, and `--quick-filter` CLI flags.
    pub quick_filters: Vec<QuickFilter>,
    /// Constructs the [`VersionedStore`] backend used by [`AppState::store`].
    ///
    /// Defaults to [`default_store_factory`]: a real git-backed
    /// [`rdm_store_git::GitStore`] when the plan root is a git repository
    /// (built with the `git` feature), falling back to [`FsStore`]
    /// otherwise. This field is `pub` (rather than crate-private) so that
    /// `AppState { .. , ..Default::default() }` struct-update syntax — used
    /// pervasively by production call sites and test fixtures across crate
    /// boundaries (`rdm-cli`, `rdm-server`'s own `tests/` integration
    /// binaries) — type-checks regardless of module or crate visibility.
    /// Prefer [`AppState::with_store_factory`] over setting this directly
    /// for deterministic test control (e.g. forcing `FsStore` or a
    /// non-default backend); it reads as intent ("inject a store backend")
    /// rather than a raw field assignment.
    pub store_factory: StoreFactory,
    /// What happens to a mutation's writes after they are flushed to disk.
    ///
    /// See [`MutationPolicy`]. The shipped default is
    /// [`MutationPolicy::StagingOnly`].
    pub mutation_policy: MutationPolicy,
    /// The changeset every mutation this process makes is attributed to.
    ///
    /// Resolved once at startup, because a long-lived server is **one
    /// session**: every request shares this id. That is the correct model
    /// (one server = one session), and it is surfaced — at boot, and on every
    /// response as `X-Rdm-Changeset` — so an operator can always name the
    /// changeset to reconcile.
    pub changeset: Option<String>,
}

/// What the server does with a mutation's writes once they are on disk.
///
/// **The operator decision this phase records.** A server mutation must
/// either reach a commit or fail loudly; what it must never do is sit on disk
/// forever, readable by everyone and landed by no one, with nothing said
/// about it. See `docs/scoping-model-decision.md` §
/// "Which Interaction Layers Are Covered".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MutationPolicy {
    /// **The default.** Writes are flushed and journaled to the server's
    /// startup changeset, never committed by the server itself.
    ///
    /// Loud rather than silent: the changeset id is printed at boot with the
    /// reconciliation command, and returned on every response as
    /// `X-Rdm-Changeset`. Reconcile with:
    ///
    /// ```text
    /// rdm commit --changeset <id>
    /// ```
    #[default]
    StagingOnly,
    /// Opt-in: commit this session's changeset (scoped) after every mutation.
    ///
    /// Chosen with `--autocommit` / `RDM_SERVER_AUTOCOMMIT=1`. Still scoped —
    /// a server commit can no more sweep a CLI session's dirty paths than a
    /// CLI commit can sweep the server's.
    Autocommit,
}

/// Constructs the server's default [`VersionedStore`] backend for `root`.
///
/// Tries a real, git-backed [`rdm_store_git::GitStore`] first (the normal
/// case, since every rdm plan repo is git-managed) so `?at=<sha>` and
/// review-anchor drift resolution see real committed history. Falls back
/// to the non-versioned [`FsStore`] — whose [`VersionedStore`] impl always
/// reports [`rdm_core::error::Error::HistoryUnavailable`] — when `root` is
/// not (yet) a git repository, or when built without the `git` feature. Any
/// `GitStore::new` failure (not just "not a git repo") deliberately falls
/// back silently — the server is a best-effort viewer and treats missing
/// history as a degraded capability, not a fatal error.
fn default_store_factory(root: &Path) -> Box<dyn VersionedStore + Send + Sync> {
    #[cfg(feature = "git")]
    {
        if let Ok(store) = rdm_store_git::GitStore::new(root) {
            return Box::new(store);
        }
    }
    Box::new(FsStore::new(root))
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("plan_root", &self.plan_root)
            .field("quick_filters", &self.quick_filters)
            .finish_non_exhaustive()
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            plan_root: PathBuf::new(),
            quick_filters: Vec::new(),
            store_factory: Arc::new(default_store_factory),
            mutation_policy: MutationPolicy::default(),
            changeset: None,
        }
    }
}

impl AppState {
    /// Opens the configured [`VersionedStore`] backend at the plan root.
    ///
    /// Defaults to [`default_store_factory`]: a real git-backed store when
    /// the plan root is a git repository (with the `git` feature enabled),
    /// falling back to [`FsStore`] (no history) otherwise. Overridden by
    /// [`AppState::with_store_factory`] for deterministic test control
    /// (e.g. forcing a specific backend) rather than as the only way to
    /// get real history.
    pub fn store(&self) -> Box<dyn VersionedStore + Send + Sync> {
        (self.store_factory)(&self.plan_root)
    }

    /// Overrides the store backend used by [`AppState::store`].
    ///
    /// Used by integration tests to back the server with a real
    /// git-backed [`rdm_store_git::GitStore`] so the `?at=<sha>`
    /// historical-read path can be exercised end-to-end.
    #[must_use]
    pub fn with_store_factory(mut self, factory: StoreFactory) -> Self {
        self.store_factory = factory;
        self
    }

    /// Resolves this server's changeset id and returns `self`.
    ///
    /// Prefers an explicit id (`--changeset` / `RDM_SESSION`); otherwise
    /// resolves the ordinary rung chain against the plan repo, so the id
    /// matches what a `rdm` CLI run inside the same process tree would use.
    #[must_use]
    pub fn with_resolved_changeset(mut self, explicit: Option<String>) -> Self {
        self.changeset = explicit.or_else(|| self.resolve_changeset());
        self
    }

    #[cfg(feature = "git")]
    fn resolve_changeset(&self) -> Option<String> {
        let store = rdm_store_git::GitStore::new(&self.plan_root).ok()?;
        store.session().map(|s| s.id.to_string())
    }

    #[cfg(not(feature = "git"))]
    fn resolve_changeset(&self) -> Option<String> {
        None
    }

    /// The one-line notice a `StagingOnly` server prints at boot.
    ///
    /// Returns `None` under [`MutationPolicy::Autocommit`], where nothing is
    /// left pending.
    #[must_use]
    pub fn boot_notice(&self) -> Option<String> {
        if self.mutation_policy == MutationPolicy::Autocommit {
            return None;
        }
        let id = self.changeset.as_deref().unwrap_or("<unresolved>");
        Some(format!(
            "WARN: mutations are staged, not committed. They are attributed to \
             changeset '{id}' and are surfaced on every response as X-Rdm-Changeset. \
             Reconcile with: rdm commit --changeset {id}"
        ))
    }

    /// **The one post-mutate helper every handler calls.**
    ///
    /// Deliberately the *only* place in `rdm-server` that can reach a commit
    /// primitive: the 22 `ops::mutate` sites call this, never
    /// `commit_changeset`/`commit_whole_tree` directly, so the commit-call-site
    /// gate stays satisfied and the policy lives in exactly one place.
    ///
    /// Under [`MutationPolicy::StagingOnly`] this is a no-op — the write is
    /// already flushed and journaled, and the pending changeset is reported
    /// at boot and on every response. Under [`MutationPolicy::Autocommit`] it
    /// lands this session's changeset, scoped.
    ///
    /// Never fails a request: a commit failure warns on stderr. The write is
    /// on disk and attributed either way, so the mutation is not lost.
    pub fn post_mutate(&self) {
        if self.mutation_policy != MutationPolicy::Autocommit {
            return;
        }
        #[cfg(feature = "git")]
        {
            match rdm_store_git::GitStore::new(&self.plan_root) {
                Ok(store) => {
                    if let Err(e) = store.commit_changeset(None, &[]) {
                        eprintln!("warning: autocommit failed: {e}");
                    }
                }
                Err(e) => eprintln!("warning: autocommit could not open the plan repo: {e}"),
            }
        }
    }

    /// Build the [`QuickFilterView`] list for a given page path.
    ///
    /// `page_path` should be the page's path without query string (e.g.
    /// `/projects/demo/roadmaps`); each chip's href is built by appending
    /// `?tag=<encoded-tag>`. `active_tag` highlights the matching chip.
    pub fn quick_filter_views_for_path(
        &self,
        page_path: &str,
        active_tag: Option<&str>,
    ) -> Vec<QuickFilterView> {
        quick_filter_views(&self.quick_filters, page_path, active_tag)
    }
}
