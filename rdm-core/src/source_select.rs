//! Which source repository a command reads from, as a **pure** decision.
//!
//! Two commands ask the same question — "is the checkout I am standing in
//! the project's configured source repository?" — and used to answer it
//! with two hand-written ladders in `rdm-cli`:
//!
//! - `rdm review --on change/…` ([`SourceFallback::ConfiguredLocal`]) pins
//!   content and must name a source or refuse, so it may leave the cwd for
//!   a configured local directory.
//! - `rdm link check` ([`SourceFallback::CwdOnly`]) is a lint that fails
//!   open, so it never verifies against a repository the operator is not
//!   standing in; every other environment is a documented skip.
//!
//! Auditing the two ladders confirmed they genuinely disagree in three of
//! the seven environments below (E2, E4, E6) — and that the disagreement is
//! **intentional**, not drift. So this module owns the *classification* of
//! the environment, while the [`SourceFallback`] parameter keeps the
//! *dispositions* deliberately different. Collapsing the two into one
//! ladder would erase link check's fail-open skip.
//!
//! Nothing here does I/O: adapters supply the three booleans (am I in a
//! checkout, does it match the configured source, is the configured source
//! a local directory) and map the answer back onto their own messages.
//!
//! See the decision table on [`select_source`], and
//! `docs/change-reviews.md` § "Source discovery" for the same table in
//! prose.

/// The project's configured `source.repo`, reduced to what the decision
/// actually depends on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfiguredSource<'a> {
    /// The configured locator verbatim — a filesystem path or a clone URL.
    pub locator: &'a str,
    /// Whether [`Self::locator`] names a directory on this machine.
    ///
    /// Computed by the adapter (`Path::is_dir`); core does no I/O. A
    /// relative locator therefore resolves against the invoking process's
    /// cwd, which is the adapter's existing behavior.
    pub is_local_dir: bool,
}

/// Everything about the invoking environment the decision depends on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceEnvironment<'a> {
    /// Whether the cwd is inside a git checkout at all.
    pub in_checkout: bool,
    /// Whether that checkout is the project's configured source.
    ///
    /// Computed by the adapter from the checkout's *identity* root
    /// (canonicalized path, or its `origin` remote) — never used as a path
    /// to read through. Meaningless, and ignored, when
    /// [`Self::configured`] is `None`.
    pub checkout_matches_configured: bool,
    /// Whether the checkout the cwd sits in **is the plan repo itself**.
    ///
    /// Computed by the adapter by comparing canonicalized *identity* roots
    /// (the discovered repository root of the cwd against the discovered
    /// repository root of the plan store), never by string-matching paths.
    /// Meaningless, and ignored, when [`Self::configured`] is `Some`: an
    /// operator who explicitly configures `source.repo` at the plan repo has
    /// named that choice and keeps working.
    pub checkout_is_plan_repo: bool,
    /// The project's configured source, when it configures one.
    pub configured: Option<ConfiguredSource<'a>>,
}

/// How far a consumer is willing to go when the cwd checkout is not the
/// answer. The one axis on which the two consumers legitimately differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFallback {
    /// May leave the cwd for a configured **local** source directory —
    /// `rdm review --on change/…`, which must name a source or refuse.
    ConfiguredLocal,
    /// Never reads a repository the operator is not standing in — `rdm
    /// link check`, whose verification is a fail-open lint.
    CwdOnly,
}

/// Why no source repository is available.
///
/// One variant per distinct cause, so no consumer has to conflate two
/// causes into one message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceUnavailable {
    /// The cwd is not inside a checkout, though the project does configure
    /// a source.
    NotInCheckout,
    /// The cwd is inside a checkout, but the project configures no source
    /// to verify it against.
    NoSourceConfigured,
    /// The cwd is inside a checkout that is demonstrably a *different*
    /// repository from the configured source.
    CheckoutIsNotConfiguredSource {
        /// The configured `source.repo`, for the message.
        locator: String,
    },
    /// The configured source is not a local directory (a clone URL), and
    /// there is no checkout to read instead.
    ConfiguredSourceNotLocal {
        /// The configured `source.repo`, for the message.
        locator: String,
    },
    /// Neither a checkout nor a configured source: nothing to read at all.
    NoCheckoutAndNoSource,
    /// The cwd is inside a checkout, the project configures no source, and
    /// that checkout is the **plan repo itself** — so reading it would mint a
    /// review of the plan repo's own history instead of the project's code.
    ///
    /// Carries no payload: this module is handed only booleans and does no
    /// I/O, so it cannot name the plan root. The adapter that computed
    /// [`SourceEnvironment::checkout_is_plan_repo`] already holds the store
    /// and composes the operator-facing message.
    CheckoutIsPlanRepo,
}

/// Which repository to read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceSelection {
    /// Read the **current working directory**.
    ///
    /// The cwd specifically, and never the checkout's identity root: for a
    /// linked worktree, `git rev-parse --show-toplevel`-style discovery
    /// answers with the repository's *main* working tree, and reading
    /// through that would make `change/HEAD` pin main's HEAD and stamp
    /// `main` as the change branch instead of the worktree's own. This
    /// classifier is handed only booleans, so it is structurally incapable
    /// of naming the identity root.
    ReadCwdCheckout,
    /// Read the project's configured local source directory.
    ReadConfiguredLocal,
    /// Read nothing; the consumer errors or skips.
    Unavailable(SourceUnavailable),
}

/// Classifies `env` into the repository to read, given how far the
/// consumer is willing to fall back.
///
/// # The decision table
///
/// Eight environments, two consumers, sixteen cells. E6a, E6b and E7 are
/// distinct environments with distinct causes and are never collapsed.
///
/// | Environment | `ConfiguredLocal` (change review) | `CwdOnly` (link check) |
/// |---|---|---|
/// | E1 matching checkout | `ReadCwdCheckout` | `ReadCwdCheckout` |
/// | E2 mismatching checkout + configured LOCAL source | `ReadConfiguredLocal` | `Unavailable(CheckoutIsNotConfiguredSource)` |
/// | E3 mismatching checkout + configured REMOTE source | `Unavailable(CheckoutIsNotConfiguredSource)` | `Unavailable(CheckoutIsNotConfiguredSource)` |
/// | E4 no checkout + configured LOCAL source | `ReadConfiguredLocal` | `Unavailable(NotInCheckout)` |
/// | E5 no checkout + configured REMOTE source | `Unavailable(ConfiguredSourceNotLocal)` | `Unavailable(NotInCheckout)` |
/// | E6a in a checkout that is NOT the plan repo, NO source configured | `ReadCwdCheckout` | `Unavailable(NoSourceConfigured)` |
/// | E6b in the PLAN REPO itself, NO source configured | `Unavailable(CheckoutIsPlanRepo)` | `Unavailable(NoSourceConfigured)` |
/// | E7 no checkout, NO source configured | `Unavailable(NoCheckoutAndNoSource)` | `Unavailable(NoCheckoutAndNoSource)` |
///
/// The three cells where the consumers differ — **E2, E4 and E6a** — are the
/// audited, intentional disagreement: change review may leave the cwd for a
/// configured local directory and treats an unconfigured checkout as
/// uncontradicted, while link check refuses to verify against anything but
/// the checkout it can prove is the source. The unit test asserts those
/// three differ, so a later attempt to "unify" the ladders fails loudly
/// instead of silently erasing link check's skip.
///
/// **E6b** is the one environment where `ConfiguredLocal` refuses a checkout
/// it is standing in. Reading the plan repo as a source repository is never
/// what the operator meant: it pins the plan repo's own `HEAD`, derives a
/// merge base of `HEAD` against the plan repo's own default branch (so the
/// reviewed range is empty), and records a review of the plan data as though
/// it were the project's code. `CwdOnly` needs no split — it already refuses
/// the whole unconfigured arm.
///
/// # Examples
///
/// ```
/// use rdm_core::source_select::{
///     ConfiguredSource, SourceEnvironment, SourceFallback, SourceSelection, SourceUnavailable,
///     select_source,
/// };
///
/// // E6a: in a checkout (not the plan repo) with nothing configured to
/// // contradict it.
/// let env = SourceEnvironment {
///     in_checkout: true,
///     checkout_matches_configured: false,
///     checkout_is_plan_repo: false,
///     configured: None,
/// };
/// assert_eq!(
///     select_source(&env, SourceFallback::ConfiguredLocal),
///     SourceSelection::ReadCwdCheckout
/// );
/// assert_eq!(
///     select_source(&env, SourceFallback::CwdOnly),
///     SourceSelection::Unavailable(SourceUnavailable::NoSourceConfigured)
/// );
///
/// // E1: the checkout IS the configured source — both consumers read it.
/// let env = SourceEnvironment {
///     in_checkout: true,
///     checkout_matches_configured: true,
///     checkout_is_plan_repo: false,
///     configured: Some(ConfiguredSource { locator: "/srv/repo", is_local_dir: true }),
/// };
/// assert_eq!(
///     select_source(&env, SourceFallback::CwdOnly),
///     SourceSelection::ReadCwdCheckout
/// );
/// ```
#[must_use]
pub fn select_source(env: &SourceEnvironment<'_>, fallback: SourceFallback) -> SourceSelection {
    match (env.in_checkout, env.configured) {
        // E1 — the checkout is provably the configured source.
        (true, Some(_)) if env.checkout_matches_configured => SourceSelection::ReadCwdCheckout,
        // E2 / E3 — in a checkout that is a different repository.
        (true, Some(cfg)) => {
            if fallback == SourceFallback::ConfiguredLocal && cfg.is_local_dir {
                SourceSelection::ReadConfiguredLocal
            } else {
                SourceSelection::Unavailable(SourceUnavailable::CheckoutIsNotConfiguredSource {
                    locator: cfg.locator.to_string(),
                })
            }
        }
        // E6a / E6b — in a checkout, nothing configured to contradict it.
        (true, None) => match fallback {
            // E6b: the checkout IS the plan repo. Reading it would review the
            // plan data instead of the project's code, so refuse rather than
            // mint a meaningless review; E6a is unchanged.
            SourceFallback::ConfiguredLocal if env.checkout_is_plan_repo => {
                SourceSelection::Unavailable(SourceUnavailable::CheckoutIsPlanRepo)
            }
            SourceFallback::ConfiguredLocal => SourceSelection::ReadCwdCheckout,
            SourceFallback::CwdOnly => {
                SourceSelection::Unavailable(SourceUnavailable::NoSourceConfigured)
            }
        },
        // E4 / E5 — no checkout, but something is configured.
        (false, Some(cfg)) => match fallback {
            SourceFallback::ConfiguredLocal if cfg.is_local_dir => {
                SourceSelection::ReadConfiguredLocal
            }
            SourceFallback::ConfiguredLocal => {
                SourceSelection::Unavailable(SourceUnavailable::ConfiguredSourceNotLocal {
                    locator: cfg.locator.to_string(),
                })
            }
            SourceFallback::CwdOnly => {
                SourceSelection::Unavailable(SourceUnavailable::NotInCheckout)
            }
        },
        // E7 — nothing at all.
        (false, None) => SourceSelection::Unavailable(SourceUnavailable::NoCheckoutAndNoSource),
    }
}

/// Trims a trailing `/` and a trailing `.git` (in that order) so a source
/// URL/path can be compared for equality regardless of those two common
/// stylistic variations — e.g. `https://example.com/org/repo` and
/// `https://example.com/org/repo.git/` normalize to the same value.
///
/// Not a full URL parse: it deliberately does not reconcile scheme
/// differences (`git@host:org/repo.git` vs `https://host/org/repo`), so
/// those still compare unequal.
#[must_use]
pub fn normalize_repo_locator(locator: &str) -> String {
    locator
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .to_string()
}

/// Whether two repository locators name the same repository, up to the
/// normalization [`normalize_repo_locator`] performs.
///
/// The pure half of repository identity. Adapters compare local paths by
/// canonicalized form *first* — so a real directory literally named
/// `…/repo.git` still matches itself — and fall back to this for URL-shaped
/// locators.
#[must_use]
pub fn locators_match(a: &str, b: &str) -> bool {
    normalize_repo_locator(a) == normalize_repo_locator(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The full decision table: EIGHT environments x two consumers = 16
    /// cells, every one asserted. The length assertion pins the row count
    /// so a dropped or merged environment fails loudly rather than leaving
    /// a cell untested.
    #[test]
    fn select_source_decision_table() {
        let local = Some(ConfiguredSource {
            locator: "/srv/source",
            is_local_dir: true,
        });
        let remote = Some(ConfiguredSource {
            locator: "https://example.com/org/repo.git",
            is_local_dir: false,
        });
        let not_source = |locator: &str| {
            SourceSelection::Unavailable(SourceUnavailable::CheckoutIsNotConfiguredSource {
                locator: locator.to_string(),
            })
        };

        let cases: Vec<(
            &str,
            SourceEnvironment<'_>,
            SourceSelection,
            SourceSelection,
        )> = vec![
            (
                "E1 matching checkout",
                SourceEnvironment {
                    in_checkout: true,
                    checkout_matches_configured: true,
                    checkout_is_plan_repo: false,
                    configured: local,
                },
                SourceSelection::ReadCwdCheckout,
                SourceSelection::ReadCwdCheckout,
            ),
            (
                "E2 mismatching checkout + configured LOCAL source",
                SourceEnvironment {
                    in_checkout: true,
                    checkout_matches_configured: false,
                    checkout_is_plan_repo: false,
                    configured: local,
                },
                SourceSelection::ReadConfiguredLocal,
                not_source("/srv/source"),
            ),
            (
                "E3 mismatching checkout + configured REMOTE source",
                SourceEnvironment {
                    in_checkout: true,
                    checkout_matches_configured: false,
                    checkout_is_plan_repo: false,
                    configured: remote,
                },
                not_source("https://example.com/org/repo.git"),
                not_source("https://example.com/org/repo.git"),
            ),
            (
                "E4 no checkout + configured LOCAL source",
                SourceEnvironment {
                    in_checkout: false,
                    checkout_matches_configured: false,
                    checkout_is_plan_repo: false,
                    configured: local,
                },
                SourceSelection::ReadConfiguredLocal,
                SourceSelection::Unavailable(SourceUnavailable::NotInCheckout),
            ),
            (
                "E5 no checkout + configured REMOTE source",
                SourceEnvironment {
                    in_checkout: false,
                    checkout_matches_configured: false,
                    checkout_is_plan_repo: false,
                    configured: remote,
                },
                SourceSelection::Unavailable(SourceUnavailable::ConfiguredSourceNotLocal {
                    locator: "https://example.com/org/repo.git".to_string(),
                }),
                SourceSelection::Unavailable(SourceUnavailable::NotInCheckout),
            ),
            (
                "E6a in a checkout that is NOT the plan repo, NO source configured",
                SourceEnvironment {
                    in_checkout: true,
                    checkout_matches_configured: false,
                    checkout_is_plan_repo: false,
                    configured: None,
                },
                SourceSelection::ReadCwdCheckout,
                SourceSelection::Unavailable(SourceUnavailable::NoSourceConfigured),
            ),
            (
                "E6b in the PLAN REPO itself, NO source configured",
                SourceEnvironment {
                    in_checkout: true,
                    checkout_matches_configured: false,
                    checkout_is_plan_repo: true,
                    configured: None,
                },
                SourceSelection::Unavailable(SourceUnavailable::CheckoutIsPlanRepo),
                // link check already refuses the whole unconfigured arm, so it
                // needs no plan-repo-specific cell.
                SourceSelection::Unavailable(SourceUnavailable::NoSourceConfigured),
            ),
            (
                "E7 no checkout, NO source configured",
                SourceEnvironment {
                    in_checkout: false,
                    checkout_matches_configured: false,
                    checkout_is_plan_repo: false,
                    configured: None,
                },
                SourceSelection::Unavailable(SourceUnavailable::NoCheckoutAndNoSource),
                SourceSelection::Unavailable(SourceUnavailable::NoCheckoutAndNoSource),
            ),
        ];

        assert_eq!(
            cases.len() * 2,
            16,
            "the decision table is EIGHT environments x two consumers"
        );

        for (label, env, change_review, link_check) in &cases {
            assert_eq!(
                &select_source(env, SourceFallback::ConfiguredLocal),
                change_review,
                "{label}: ConfiguredLocal (change review)"
            );
            assert_eq!(
                &select_source(env, SourceFallback::CwdOnly),
                link_check,
                "{label}: CwdOnly (link check)"
            );
        }

        // The three audited, intentional disagreements. Asserting them
        // explicitly means a later "unify the ladders" change fails here
        // rather than silently erasing link check's fail-open skip.
        for label in [
            "E2 mismatching checkout + configured LOCAL source",
            "E4 no checkout + configured LOCAL source",
            "E6a in a checkout that is NOT the plan repo, NO source configured",
        ] {
            let (_, _env, change_review, link_check) =
                cases.iter().find(|(l, ..)| *l == label).unwrap();
            assert_ne!(
                change_review, link_check,
                "{label} is one of the three cells the consumers deliberately differ on"
            );
        }

        // E6b is the one cell where `ConfiguredLocal` refuses a checkout it is
        // standing in, and it must NOT be reachable when a source IS
        // configured: an operator who points `source.repo` at the plan repo
        // has named that choice (E1/E2), and the flag is ignored there.
        let configured_at_the_plan_repo = SourceEnvironment {
            in_checkout: true,
            checkout_matches_configured: true,
            checkout_is_plan_repo: true,
            configured: local,
        };
        assert_eq!(
            select_source(
                &configured_at_the_plan_repo,
                SourceFallback::ConfiguredLocal
            ),
            SourceSelection::ReadCwdCheckout,
            "an explicitly configured plan-repo source stays E1 — the plan-repo guard is scoped to configured: None"
        );
    }

    /// A matching checkout never depends on the fallback, and a checkout
    /// whose configured source is absent is E6, not E1 — the
    /// `checkout_matches_configured` flag is ignored without a configured
    /// source.
    #[test]
    fn matches_flag_is_ignored_without_a_configured_source() {
        let env = SourceEnvironment {
            in_checkout: true,
            checkout_matches_configured: true,
            checkout_is_plan_repo: false,
            configured: None,
        };
        assert_eq!(
            select_source(&env, SourceFallback::CwdOnly),
            SourceSelection::Unavailable(SourceUnavailable::NoSourceConfigured)
        );
    }

    #[test]
    fn normalize_repo_locator_trims_trailing_slash_then_dot_git() {
        assert_eq!(
            normalize_repo_locator("https://example.com/org/repo.git/"),
            "https://example.com/org/repo"
        );
        assert_eq!(
            normalize_repo_locator("https://example.com/org/repo.git"),
            "https://example.com/org/repo"
        );
        assert_eq!(
            normalize_repo_locator("https://example.com/org/repo/"),
            "https://example.com/org/repo"
        );
        assert_eq!(
            normalize_repo_locator("https://example.com/org/repo"),
            "https://example.com/org/repo"
        );
    }

    #[test]
    fn normalize_repo_locator_does_not_reconcile_scheme_differences() {
        assert_ne!(
            normalize_repo_locator("git@example.com:org/repo.git"),
            normalize_repo_locator("https://example.com/org/repo")
        );
        assert!(!locators_match(
            "git@example.com:org/repo.git",
            "https://example.com/org/repo"
        ));
    }

    #[test]
    fn locators_match_up_to_normalization() {
        assert!(locators_match(
            "https://example.com/org/repo.git/",
            "https://example.com/org/repo"
        ));
        assert!(!locators_match(
            "https://example.com/org/repo",
            "https://example.com/org/other"
        ));
    }
}
