use std::path::{Path, PathBuf};
use std::sync::Arc;

use rdm_core::config::QuickFilter;
use rdm_core::session::SessionId;
use rdm_core::store::VersionedStore;
use rdm_store_fs::FsStore;

use crate::templates::{QuickFilterView, quick_filter_views};

/// Constructs a fresh [`VersionedStore`] instance rooted at the given path,
/// attributing its writes to the given changeset.
///
/// Called once per request/lookup by [`AppState::store`]; defaults to
/// [`default_store_factory`], which prefers a real git-backed
/// [`rdm_store_git::GitStore`] when the plan root is a git repository
/// (built with the `git` feature) and falls back to [`FsStore`] otherwise.
/// Tests may override it via [`AppState::with_store_factory`] for
/// deterministic control over which backend serves the `?at=<sha>`
/// historical-read path.
///
/// The changeset argument is what keeps the id the server *advertises*
/// (`X-Rdm-Changeset`, the boot notice, the reconciliation command) and the
/// id its writes are *journaled to* the same id. Passing it down rather than
/// exporting `RDM_SESSION` is deliberate: mutating process-global
/// environment state is `unsafe` on a running server and racy besides.
pub type StoreFactory =
    Arc<dyn Fn(&Path, Option<&SessionId>) -> Box<dyn VersionedStore + Send + Sync> + Send + Sync>;

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
    ///
    /// It is not merely advertised: [`AppState::store`] passes it to the
    /// [`StoreFactory`], so the writes really do journal here.
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
    /// Loud rather than silent, on three surfaces, so a staged mutation can
    /// never pass unremarked: the changeset id and the reconciliation
    /// command are printed at boot ([`AppState::boot_notice`]), printed
    /// again on **every individual mutation** ([`AppState::post_mutate`]),
    /// and returned on every mutating response as `X-Rdm-Staged` alongside
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

/// The operator's startup decision, resolved from argv and the environment.
///
/// Split out of `main` so the precedence rules are testable without
/// spawning a process: `main` is then a two-line extractor
/// (`ServerOptions::resolve` → `AppState`), and every edge case below is
/// covered by unit tests rather than by hoping.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServerOptions {
    /// What happens to a mutation's writes once they are flushed to disk.
    pub mutation_policy: MutationPolicy,
    /// An explicitly-named changeset id, or `None` to resolve the ordinary
    /// rung chain against the plan repo at startup.
    pub changeset: Option<String>,
}

impl ServerOptions {
    /// Resolves the startup options from a full argv and an env lookup.
    ///
    /// Precedence, highest first:
    ///
    /// - autocommit: `--autocommit` anywhere in argv, else
    ///   `RDM_SERVER_AUTOCOMMIT` set to `1` or `true`. Any other value (or
    ///   an unset variable) keeps the [`MutationPolicy::StagingOnly`]
    ///   default — this is an opt-in, so an unparseable value never
    ///   silently opts in.
    /// - changeset: `--changeset <id>`, else `RDM_SESSION`.
    ///
    /// A trailing `--changeset` with no following argument, and a blank id
    /// from either source, both resolve to `None` — fall back to the rung
    /// chain rather than name a changeset `""`, which no `rdm commit
    /// --changeset` could ever address.
    #[must_use]
    pub fn resolve<S, F>(args: &[S], env: F) -> Self
    where
        S: AsRef<str>,
        F: Fn(&str) -> Option<String>,
    {
        let autocommit = args.iter().any(|a| a.as_ref() == "--autocommit")
            || matches!(env("RDM_SERVER_AUTOCOMMIT").as_deref(), Some("1" | "true"));
        let changeset = args
            .iter()
            .position(|a| a.as_ref() == "--changeset")
            .and_then(|i| args.get(i + 1))
            .map(|s| s.as_ref().to_string())
            .or_else(|| env("RDM_SESSION"))
            .filter(|s| !s.trim().is_empty());
        Self {
            mutation_policy: if autocommit {
                MutationPolicy::Autocommit
            } else {
                MutationPolicy::StagingOnly
            },
            changeset,
        }
    }
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
fn default_store_factory(
    root: &Path,
    changeset: Option<&SessionId>,
) -> Box<dyn VersionedStore + Send + Sync> {
    #[cfg(feature = "git")]
    {
        if let Ok(store) = rdm_store_git::GitStore::new(root) {
            return match changeset {
                Some(id) => Box::new(store.with_session_id(id.clone())),
                None => Box::new(store),
            };
        }
    }
    let _ = changeset;
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
        (self.store_factory)(&self.plan_root, self.session_id().as_ref())
    }

    /// This server's advertised changeset id, parsed.
    ///
    /// `None` when nothing was resolved, or when the configured id is not a
    /// legal changeset name — in which case the store falls back to the rung
    /// chain rather than journaling to a name no `rdm commit --changeset`
    /// could address.
    #[must_use]
    pub fn session_id(&self) -> Option<SessionId> {
        self.changeset.as_deref().and_then(SessionId::new)
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

    /// The changeset id this server attributes its mutations to, for display.
    ///
    /// Falls back to `<unresolved>` so a notice never renders an empty id.
    #[must_use]
    pub fn changeset_label(&self) -> &str {
        self.changeset.as_deref().unwrap_or("<unresolved>")
    }

    /// The exact command that lands what this server has staged.
    ///
    /// One source for the boot notice, the per-mutation notice, and the
    /// `X-Rdm-Staged` response header, so an operator is never told three
    /// slightly different things.
    #[must_use]
    pub fn reconcile_command(&self) -> String {
        format!("rdm commit --changeset {}", self.changeset_label())
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
        Some(format!(
            "WARN: mutations are staged, not committed. They are attributed to \
             changeset '{}' and are surfaced on every mutating response as \
             X-Rdm-Changeset / X-Rdm-Staged. Reconcile with: {}",
            self.changeset_label(),
            self.reconcile_command()
        ))
    }

    /// The notice a `StagingOnly` server emits for **each** mutation, and the
    /// value of the `X-Rdm-Staged` response header.
    ///
    /// Boot-time-only reporting is not enough: a server that has been up for
    /// a week has scrolled its boot line away, and the acceptance criterion
    /// this policy answers to is that a server mutation either reaches a
    /// commit *or fails loudly* — per mutation, not per process. Returns
    /// `None` under [`MutationPolicy::Autocommit`], where the mutation does
    /// reach a commit.
    #[must_use]
    pub fn staged_notice(&self) -> Option<String> {
        if self.mutation_policy == MutationPolicy::Autocommit {
            return None;
        }
        // Deliberately plain ASCII: this exact string is also an HTTP header
        // value (`X-Rdm-Staged`), and a header value a client cannot read
        // back as a `str` reports nothing at all.
        Some(format!(
            "WARN: mutation staged, NOT committed. Changeset '{}'. Reconcile with: {}",
            self.changeset_label(),
            self.reconcile_command()
        ))
    }

    /// **The one post-mutate helper every handler calls.**
    ///
    /// Deliberately the *only* place in `rdm-server` that can reach a commit
    /// primitive: the 22 `ops::mutate` sites call this, never
    /// `commit_changeset`/`commit_whole_tree` directly, so the commit-call-site
    /// gate stays satisfied and the policy lives in exactly one place.
    ///
    /// Call it **after** `ops::mutate` returns, never inside the closure: the
    /// store holds staged writes in memory until `ops::mutate`'s own
    /// `Store::commit` flushes them, so a commit attempted from inside the
    /// closure would see neither the write nor its journal entry. Call it
    /// **unconditionally** for a mutation, never behind a condition that
    /// governs some *other* part of the request.
    ///
    /// Under [`MutationPolicy::StagingOnly`] this commits nothing — the write
    /// is already flushed and journaled — but it is not silent: it emits
    /// [`AppState::staged_notice`] for *this* mutation, and the router
    /// returns the same text as `X-Rdm-Staged`. Under
    /// [`MutationPolicy::Autocommit`] it lands this session's changeset,
    /// scoped.
    ///
    /// Never fails a request: a commit failure warns on stderr. The write is
    /// on disk and attributed either way, so the mutation is not lost.
    pub fn post_mutate(&self) {
        if self.mutation_policy != MutationPolicy::Autocommit {
            if let Some(notice) = self.staged_notice() {
                eprintln!("{notice}");
            }
            return;
        }
        #[cfg(feature = "git")]
        {
            match rdm_store_git::GitStore::new(&self.plan_root) {
                Ok(store) => {
                    // Commit the changeset this server resolved at startup and
                    // advertises on every response — not whatever the ambient
                    // process environment happens to resolve to now. Under
                    // `--changeset <id>` those differ, and committing the
                    // wrong one would land nothing while reporting success.
                    let id = self
                        .changeset
                        .as_deref()
                        .and_then(rdm_core::session::SessionId::new);
                    match store.commit_changeset_id(id.as_ref(), None, &[]) {
                        // A changeset that landed nothing is the "fails
                        // loudly" half of the acceptance criterion: the write
                        // is on disk, attributed to nobody the commit could
                        // see, and would otherwise vanish without a word.
                        Ok(commit) if commit.sha.is_none() => eprintln!(
                            "ERROR: autocommit landed nothing — changeset '{}' claims no \
                             paths. The write is on disk; land it with {}",
                            self.changeset_label(),
                            self.reconcile_command()
                        ),
                        Ok(_) => {}
                        Err(e) => eprintln!(
                            "ERROR: autocommit failed: {e}. The write is on disk and \
                             journaled — land it with {}",
                            self.reconcile_command()
                        ),
                    }
                }
                Err(e) => eprintln!(
                    "ERROR: autocommit could not open the plan repo: {e}. The write is on \
                     disk and journaled — land it with {}",
                    self.reconcile_command()
                ),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Resolves options from a literal argv and a fixed environment map.
    fn resolve(args: &[&str], env: &[(&str, &str)]) -> ServerOptions {
        let env: Vec<(String, String)> = env
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        ServerOptions::resolve(args, |key| {
            env.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
        })
    }

    fn state_with(policy: MutationPolicy, changeset: Option<&str>) -> AppState {
        AppState {
            mutation_policy: policy,
            changeset: changeset.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn default_policy_is_staging_only() {
        let opts = resolve(&["rdm-server"], &[]);
        assert_eq!(opts.mutation_policy, MutationPolicy::StagingOnly);
        assert_eq!(opts.changeset, None);
    }

    #[test]
    fn autocommit_flag_opts_in() {
        assert_eq!(
            resolve(&["rdm-server", "--autocommit"], &[]).mutation_policy,
            MutationPolicy::Autocommit
        );
    }

    #[test]
    fn autocommit_env_opts_in_only_for_1_or_true() {
        for value in ["1", "true"] {
            assert_eq!(
                resolve(&["rdm-server"], &[("RDM_SERVER_AUTOCOMMIT", value)]).mutation_policy,
                MutationPolicy::Autocommit,
                "{value} should opt in"
            );
        }
        // An unparseable value must never silently opt in to committing.
        for value in ["0", "false", "yes", "", "TRUE"] {
            assert_eq!(
                resolve(&["rdm-server"], &[("RDM_SERVER_AUTOCOMMIT", value)]).mutation_policy,
                MutationPolicy::StagingOnly,
                "{value} must not opt in"
            );
        }
    }

    #[test]
    fn changeset_flag_beats_env() {
        let opts = resolve(
            &["rdm-server", "--changeset", "from-flag"],
            &[("RDM_SESSION", "from-env")],
        );
        assert_eq!(opts.changeset.as_deref(), Some("from-flag"));
    }

    #[test]
    fn changeset_falls_back_to_rdm_session() {
        let opts = resolve(&["rdm-server"], &[("RDM_SESSION", "from-env")]);
        assert_eq!(opts.changeset.as_deref(), Some("from-env"));
    }

    #[test]
    fn trailing_changeset_flag_with_no_value_resolves_to_none() {
        // Must not panic, and must not name a changeset "" — fall back to the
        // rung chain instead.
        let opts = resolve(&["rdm-server", "--changeset"], &[]);
        assert_eq!(opts.changeset, None);
    }

    #[test]
    fn blank_changeset_from_either_source_resolves_to_none() {
        assert_eq!(
            resolve(&["rdm-server"], &[("RDM_SESSION", "")]).changeset,
            None
        );
        assert_eq!(
            resolve(&["rdm-server"], &[("RDM_SESSION", "   ")]).changeset,
            None
        );
        assert_eq!(
            resolve(&["rdm-server", "--changeset", "  "], &[]).changeset,
            None
        );
    }

    #[test]
    fn boot_notice_warns_and_names_the_reconcile_command_when_staging_only() {
        let notice = state_with(MutationPolicy::StagingOnly, Some("cs-1"))
            .boot_notice()
            .expect("staging-only must warn at boot");
        assert!(notice.starts_with("WARN:"), "{notice}");
        assert!(notice.contains("cs-1"), "{notice}");
        assert!(notice.contains("rdm commit --changeset cs-1"), "{notice}");
    }

    #[test]
    fn boot_notice_is_silent_under_autocommit() {
        assert_eq!(
            state_with(MutationPolicy::Autocommit, Some("cs-1")).boot_notice(),
            None
        );
    }

    #[test]
    fn staged_notice_is_per_mutation_and_silent_under_autocommit() {
        let notice = state_with(MutationPolicy::StagingOnly, Some("cs-1"))
            .staged_notice()
            .expect("staging-only must warn per mutation");
        assert!(notice.contains("NOT committed"), "{notice}");
        assert!(notice.contains("rdm commit --changeset cs-1"), "{notice}");
        assert_eq!(
            state_with(MutationPolicy::Autocommit, Some("cs-1")).staged_notice(),
            None
        );
    }

    #[test]
    fn unresolved_changeset_renders_a_placeholder_not_an_empty_id() {
        let state = state_with(MutationPolicy::StagingOnly, None);
        assert_eq!(state.changeset_label(), "<unresolved>");
        assert!(state.boot_notice().unwrap().contains("<unresolved>"));
    }
}
