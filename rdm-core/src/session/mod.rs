//! Session identity and the per-session changeset journal.
//!
//! Two concurrent `rdm` sessions sharing one `$RDM_ROOT` must be able to tell
//! their work apart. This module supplies the *identity* half of that: a
//! stable, distinct, automatically-resolved changeset id, plus a journal
//! recording exactly which paths each session flushed. It changes no commit
//! behavior — routing committers through the journal is a later phase.
//!
//! # The four-rung chain
//!
//! [`resolve_session`] walks phase 3's binding chain, highest precedence
//! first, and is **infallible by construction** — it returns a
//! [`ResolvedSession`], never a `Result`, so no degradation path can turn into
//! an error on the bounded git-hook deadline:
//!
//! 1. **Explicit** — [`RDM_SESSION_ENV`]. Wins outright: no lease is read, no
//!    ancestry is walked, no harness variable is consulted.
//! 2. **Inherited lease** — the nearest ancestor process holding a valid lease
//!    (see [`lease`] for the shipped stopping rule). When no ancestor holds
//!    one and no harness variable applies, a lease is *created* at the
//!    immediate parent, which is still this rung.
//! 3. **Harness variable** — the first non-empty entry of
//!    [`HARNESS_SESSION_VARS`], hashed. Purely derived, so two unrelated
//!    processes agree with zero on-disk state.
//! 4. **Per-process** — a fresh id derived from this process. Always resolves.
//!
//! Rung 2 is tried before rung 3 (phase 3's ordering), but lease *creation* is
//! deferred until after rung 3 has been checked: when a harness already
//! publishes a stable id there is nothing for a lease to bootstrap.
//!
//! # Where the state lives
//!
//! Under `<git-dir>/rdm/`. This is not a preference: rdm's two whole-tree
//! walks (`build_tree_from_dir` and `collect_working_tree` in
//! `rdm-store-git`) skip exactly the name `.git` and **neither consults
//! `.gitignore`**, so a gitignore entry would exclude nothing. See
//! [`SessionPaths`] and `docs/session-identity.md`.

pub mod journal;
pub mod lease;
pub mod process;

use std::path::{Path, PathBuf};
use std::time::Instant;

use sha2::{Digest, Sha256};

use process::{ProcessTable, SystemProcessTable};

/// The explicit session-identity escape hatch. Set it to pin a deterministic
/// id (CI, scripts, harnesses); it outranks every other rung.
pub const RDM_SESSION_ENV: &str = "RDM_SESSION";

/// Harness-published session variables, in precedence order.
///
/// This list is the extension point for tools that already track a session of
/// their own: adding an entry wires that harness in without any on-disk state.
/// `CLAUDE_CODE_SESSION_ID` is the reference entry named by phase 3.
pub const HARNESS_SESSION_VARS: &[&str] = &[
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_SESSION_ID",
    "RDM_HARNESS_SESSION_ID",
];

/// Maximum length, in bytes, of a session id.
pub const MAX_SESSION_ID_LEN: usize = 64;

/// A validated session (changeset) id.
///
/// An id becomes a file name, so this is a path-safety type, not a cosmetic
/// one: the value is restricted to `[A-Za-z0-9._-]`, capped at
/// [`MAX_SESSION_ID_LEN`] bytes, and may not be empty or consist solely of
/// dots.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionId(String);

impl SessionId {
    /// Sanitizes `raw` into a `SessionId`, or returns `None` when nothing
    /// usable survives.
    ///
    /// The input is trimmed, disallowed characters are dropped (not rejected,
    /// so a harness value with punctuation still yields a usable id), and the
    /// result is truncated to [`MAX_SESSION_ID_LEN`]. `None` means "fall
    /// through to the next rung" — never an error.
    pub fn new(raw: &str) -> Option<Self> {
        let mut cleaned: String = raw
            .trim()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
            .collect();
        cleaned.truncate(MAX_SESSION_ID_LEN);
        if cleaned.is_empty() || cleaned.chars().all(|c| c == '.') {
            return None;
        }
        Some(Self(cleaned))
    }

    /// Returns the id as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Which rung of the identity chain produced an id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rung {
    /// Rung 1: the explicit [`RDM_SESSION_ENV`] variable.
    Explicit,
    /// Rung 2: an inherited (or newly created) ancestor lease.
    Lease,
    /// Rung 3: a harness-published variable from [`HARNESS_SESSION_VARS`].
    Harness,
    /// Rung 4: a fresh per-process id. Always resolves.
    Process,
}

impl Rung {
    /// Returns the rung's 1-based number, as used in JSON output and docs.
    pub fn number(self) -> u8 {
        match self {
            Self::Explicit => 1,
            Self::Lease => 2,
            Self::Harness => 3,
            Self::Process => 4,
        }
    }
}

impl std::fmt::Display for Rung {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.number())
    }
}

/// A resolved session identity plus how it was reached and what it cost.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedSession {
    /// The changeset id.
    pub id: SessionId,
    /// Which rung produced [`id`](Self::id).
    pub rung: Rung,
    /// Wall time spent resolving, in microseconds.
    ///
    /// Surfaced so the cost on the bounded `hook_timeout_secs` path is
    /// measurable rather than asserted.
    pub resolve_micros: u128,
}

/// A source of environment variables.
///
/// Injected so tests never mutate process-global environment state, which is
/// inherently racy under a parallel test runner.
pub trait EnvSource {
    /// Returns the value of `key`, or `None` when it is unset.
    fn get(&self, key: &str) -> Option<String>;
}

/// The real process environment.
#[derive(Clone, Copy, Debug, Default)]
pub struct RealEnv;

impl EnvSource for RealEnv {
    fn get(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }
}

/// An [`EnvSource`] backed by an explicit list of key/value pairs.
///
/// Anything not listed reads as unset.
#[derive(Clone, Debug, Default)]
pub struct MapEnv(Vec<(String, String)>);

impl MapEnv {
    /// Creates an empty environment.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets `key` to `value`, returning `self` for chaining.
    #[must_use]
    pub fn with(mut self, key: &str, value: &str) -> Self {
        self.0.push((key.to_string(), value.to_string()));
        self
    }
}

impl EnvSource for MapEnv {
    fn get(&self, key: &str) -> Option<String> {
        self.0
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    }
}

/// Where a plan repo's session state lives on disk.
///
/// # Why the base is `<git-dir>/rdm/`
///
/// `rdm-store-git` builds git trees by walking the working directory
/// (`build_tree_from_dir`) and computes status the same way
/// (`collect_working_tree`). Both skip exactly one name — `.git` — at every
/// recursion level, and **neither consults `.gitignore`**, so a gitignore entry
/// would exclude nothing from rdm's commit path. Siting the state inside the
/// git directory is therefore what makes it invisible to a whole-tree commit
/// and to `rdm status`, with no change to either walk.
///
/// Any future refactor of those walks must preserve that skip, or session
/// state starts landing in commits.
///
/// A store with no git directory (a plain filesystem backend) falls back to an
/// XDG state directory via [`SessionPaths::fallback_for_root`]. Session state
/// is never written under `$RDM_ROOT` outside `.git`.
///
/// Directories are created lazily on first write, never at store-open time, so
/// a read-only plan repo still opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionPaths {
    base: PathBuf,
}

impl SessionPaths {
    /// Creates paths rooted at an explicit base directory.
    pub fn new(base: impl Into<PathBuf>) -> Self {
        Self { base: base.into() }
    }

    /// Creates paths under a repository's git directory (`<git-dir>/rdm`).
    pub fn for_git_dir(git_dir: &Path) -> Self {
        Self::new(git_dir.join("rdm"))
    }

    /// Creates paths in an XDG state directory keyed by the plan repo root.
    ///
    /// Used when there is no git directory to hide state inside. Returns
    /// `None` when neither `XDG_STATE_HOME` nor `HOME` is set — in which case
    /// there is no state directory at all and resolution silently lands on a
    /// lower rung.
    pub fn fallback_for_root(root: &Path) -> Option<Self> {
        let state = std::env::var("XDG_STATE_HOME")
            .ok()
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|home| PathBuf::from(home).join(".local").join("state"))
            })?;
        let key = hex16(&[&root.to_string_lossy()]);
        Some(Self::new(state.join("rdm").join("sessions").join(key)))
    }

    /// Returns the base directory.
    pub fn base(&self) -> &Path {
        &self.base
    }

    /// Returns the directory holding per-ancestor lease files.
    pub fn leases_dir(&self) -> PathBuf {
        self.base.join("leases")
    }

    /// Returns the directory holding per-changeset journals.
    pub fn changesets_dir(&self) -> PathBuf {
        self.base.join("changesets")
    }
}

/// Hashes the given parts into a 16-character lowercase hex digest.
///
/// Parts are NUL-separated so `("ab", "c")` and `("a", "bc")` cannot collide.
pub(crate) fn hex16(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0u8]);
    }
    hasher
        .finalize()
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Resolves this session's identity by walking the four-rung chain.
///
/// Infallible by construction: every fallible sub-step (an unreadable or
/// corrupt lease, a missing `/proc`, an absent `ps`, an unwritable state
/// directory, a blank `RDM_SESSION`) falls through to the next rung, and rung 4
/// always produces an id. The guarantee is therefore structural, not
/// disciplinary — there is no `Result` in the signature to ignore.
///
/// Nothing here special-cases `RDM_GIT_SUBPROCESS` or any other short-circuit
/// flag: a git hook resolves through the same chain whether or not it was
/// spawned by rdm.
pub fn resolve_session(
    paths: &SessionPaths,
    procs: &dyn ProcessTable,
    env: &dyn EnvSource,
) -> ResolvedSession {
    let started = Instant::now();
    let (id, rung) = resolve_id(paths, procs, env);
    ResolvedSession {
        id,
        rung,
        resolve_micros: started.elapsed().as_micros(),
    }
}

fn resolve_id(
    paths: &SessionPaths,
    procs: &dyn ProcessTable,
    env: &dyn EnvSource,
) -> (SessionId, Rung) {
    // Rung 1 — explicit. Returns immediately: no lease read, no ancestry walk,
    // no harness-variable read.
    if let Some(raw) = env.get(RDM_SESSION_ENV)
        && let Some(id) = SessionId::new(&raw)
    {
        return (id, Rung::Explicit);
    }

    // Rung 2 — an already-inherited ancestor lease.
    if let Some(id) = lease::adopt_inherited(paths, procs) {
        return (id, Rung::Lease);
    }

    // Rung 3 — a harness-published id. Checked before creating a lease: a
    // harness that already publishes a stable id needs no on-disk state.
    for var in HARNESS_SESSION_VARS {
        if let Some(raw) = env.get(var)
            && !raw.trim().is_empty()
        {
            let digest = hex16(&[var, raw.trim()]);
            if let Some(id) = SessionId::new(&format!("h-{digest}")) {
                return (id, Rung::Harness);
            }
        }
    }

    // Rung 2 (bootstrap) — create the lease this session's later invocations
    // will inherit. Creation happens only at the immediate parent.
    if let Some(id) = lease::create_at_parent(paths, procs) {
        return (id, Rung::Lease);
    }

    // Rung 4 — always resolves.
    (per_process_id(paths, procs), Rung::Process)
}

/// Derives the always-available per-process id.
fn per_process_id(paths: &SessionPaths, procs: &dyn ProcessTable) -> SessionId {
    let pid = procs.self_pid();
    let start_time = procs.get(pid).map(|i| i.start_time).unwrap_or_default();
    let digest = hex16(&[
        &pid.to_string(),
        &start_time,
        &paths.base().to_string_lossy(),
    ]);
    SessionId::new(&format!("p-{digest}")).expect("derived per-process id is always well-formed")
}

/// Returns the process-wide memoized OS process table.
///
/// The snapshot is expensive relative to everything else in resolution (one
/// `ps` spawn on macOS, one `/proc` scan on Linux), so it is taken at most once
/// per `rdm` invocation.
pub fn system_process_table() -> &'static SystemProcessTable {
    static TABLE: std::sync::OnceLock<SystemProcessTable> = std::sync::OnceLock::new();
    TABLE.get_or_init(SystemProcessTable::snapshot)
}

/// Resolves this session against the real OS process table and environment.
///
/// The reported [`ResolvedSession::resolve_micros`] covers the whole
/// resolution, including building the process table on the first call.
pub fn resolve_system_session(paths: &SessionPaths) -> ResolvedSession {
    let started = Instant::now();
    let table = system_process_table();
    let mut resolved = resolve_session(paths, table, &RealEnv);
    resolved.resolve_micros = started.elapsed().as_micros();
    resolved
}

#[cfg(test)]
mod tests {
    use super::process::MapProcessTable;
    use super::*;
    use tempfile::TempDir;

    fn paths(dir: &TempDir) -> SessionPaths {
        SessionPaths::new(dir.path().join("rdm"))
    }

    #[test]
    fn explicit_env_beats_lease_and_harness_var() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let table = MapProcessTable::chain(10, &[(20, "s20")]);
        // A perfectly adoptable lease AND a harness variable are both present.
        lease::create_at_parent(&p, &table).unwrap();
        let env = MapEnv::new()
            .with(RDM_SESSION_ENV, "explicit-1")
            .with("CLAUDE_CODE_SESSION_ID", "abc123");

        let resolved = resolve_session(&p, &table, &env);
        assert_eq!(resolved.id.as_str(), "explicit-1");
        assert_eq!(resolved.rung, Rung::Explicit);
        assert_eq!(resolved.rung.number(), 1);
    }

    #[test]
    fn inherited_lease_beats_harness_var() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let table = MapProcessTable::chain(10, &[(20, "s20")]);
        let leased = lease::create_at_parent(&p, &table).unwrap();
        let env = MapEnv::new().with("CLAUDE_CODE_SESSION_ID", "abc123");

        let resolved = resolve_session(&p, &table, &env);
        assert_eq!(resolved.id, leased);
        assert_eq!(resolved.rung, Rung::Lease);
    }

    #[test]
    fn harness_var_is_derived_so_unrelated_processes_agree() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let env = MapEnv::new().with("CLAUDE_CODE_SESSION_ID", "abc123");
        // Two unrelated ancestries, no shared on-disk state, same id.
        let a = resolve_session(&p, &MapProcessTable::empty(10), &env);
        let b = resolve_session(&p, &MapProcessTable::empty(11), &env);
        assert_eq!(a.id, b.id);
        assert_eq!(a.rung, Rung::Harness);
        assert_eq!(a.rung.number(), 3);

        let other = resolve_session(
            &p,
            &MapProcessTable::empty(10),
            &MapEnv::new().with("CLAUDE_CODE_SESSION_ID", "different"),
        );
        assert_ne!(a.id, other.id);
    }

    #[test]
    fn harness_vars_are_tried_in_declared_order() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let first = resolve_session(
            &p,
            &MapProcessTable::empty(10),
            &MapEnv::new()
                .with("CLAUDE_CODE_SESSION_ID", "x")
                .with("RDM_HARNESS_SESSION_ID", "y"),
        );
        let only_first = resolve_session(
            &p,
            &MapProcessTable::empty(10),
            &MapEnv::new().with("CLAUDE_CODE_SESSION_ID", "x"),
        );
        assert_eq!(first.id, only_first.id);
        assert_eq!(HARNESS_SESSION_VARS[0], "CLAUDE_CODE_SESSION_ID");
    }

    #[test]
    fn lease_is_created_when_nothing_is_set() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let table = MapProcessTable::chain(10, &[(20, "s20")]);
        let first = resolve_session(&p, &table, &MapEnv::new());
        assert_eq!(first.rung, Rung::Lease);
        // A sibling process under the same ancestor inherits, not re-creates.
        let sibling = MapProcessTable::chain(11, &[(20, "s20")]);
        let second = resolve_session(&p, &sibling, &MapEnv::new());
        assert_eq!(second.id, first.id);
        assert_eq!(second.rung, Rung::Lease);
        // A different ancestor is a different session.
        let other = resolve_session(
            &p,
            &MapProcessTable::chain(12, &[(21, "s21")]),
            &MapEnv::new(),
        );
        assert_ne!(other.id, first.id);
    }

    #[test]
    fn degrades_to_rung_four_on_empty_process_table() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let resolved = resolve_session(&p, &MapProcessTable::empty(4242), &MapEnv::new());
        assert_eq!(resolved.rung, Rung::Process);
        assert_eq!(resolved.rung.number(), 4);
        assert!(resolved.id.as_str().starts_with("p-"));
        // Distinct per process by construction.
        let other = resolve_session(&p, &MapProcessTable::empty(4243), &MapEnv::new());
        assert_ne!(other.id, resolved.id);
    }

    #[test]
    fn degrades_gracefully_on_corrupt_lease() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        std::fs::create_dir_all(p.leases_dir()).unwrap();
        std::fs::write(lease::lease_path(&p, 20), b"\x00garbage").unwrap();
        let table = MapProcessTable::chain(10, &[(20, "s20")]);

        let resolved = resolve_session(&p, &table, &MapEnv::new());
        // No error, no panic: the garbage is dropped and a fresh lease lands.
        assert_eq!(resolved.rung, Rung::Lease);
        assert!(resolved.id.as_str().starts_with("s-"));
        assert!(lease::read_lease(&p, 20).is_some());
    }

    #[test]
    fn degrades_to_rung_four_on_unwritable_state_dir() {
        let dir = TempDir::new().unwrap();
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, b"i am a file").unwrap();
        // `create_dir_all` cannot succeed under a regular file.
        let p = SessionPaths::new(blocker.join("rdm"));
        let resolved = resolve_session(
            &p,
            &MapProcessTable::chain(10, &[(20, "s20")]),
            &MapEnv::new(),
        );
        assert_eq!(resolved.rung, Rung::Process);
    }

    #[test]
    fn degrades_on_blank_explicit_var() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let resolved = resolve_session(
            &p,
            &MapProcessTable::empty(10),
            &MapEnv::new().with(RDM_SESSION_ENV, "   "),
        );
        assert_eq!(
            resolved.rung,
            Rung::Process,
            "blank falls through, never errors"
        );
    }

    #[test]
    fn resolves_with_rdm_git_subprocess_set() {
        // Phase 3 requires resolution not to assume the git-subprocess
        // short-circuit fired. Nothing special-cases the variable.
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let with_flag = resolve_session(
            &p,
            &MapProcessTable::empty(10),
            &MapEnv::new().with("RDM_GIT_SUBPROCESS", "1"),
        );
        let without = resolve_session(&p, &MapProcessTable::empty(10), &MapEnv::new());
        assert_eq!(with_flag.id, without.id);
        assert_eq!(with_flag.rung, Rung::Process);
    }

    #[test]
    fn session_id_sanitizes_path_traversal_and_oversize_input() {
        assert_eq!(
            SessionId::new("../../etc/passwd").unwrap().as_str(),
            "....etcpasswd"
        );
        assert_eq!(
            SessionId::new("  spaced out  ").unwrap().as_str(),
            "spacedout"
        );
        assert_eq!(
            SessionId::new(&"a".repeat(4096)).unwrap().as_str().len(),
            MAX_SESSION_ID_LEN
        );
        assert!(SessionId::new("").is_none());
        assert!(SessionId::new("   ").is_none());
        assert!(SessionId::new("///").is_none());
        assert!(
            SessionId::new("..").is_none(),
            "a dots-only id is not a usable file name"
        );
        assert_eq!(SessionId::new("ok.id_1-2").unwrap().as_str(), "ok.id_1-2");
    }

    #[test]
    fn hex16_is_nul_separated_so_parts_cannot_collide() {
        assert_ne!(hex16(&["ab", "c"]), hex16(&["a", "bc"]));
        assert_eq!(hex16(&["ab", "c"]).len(), 16);
    }

    #[test]
    fn resolution_is_deterministic_across_repeated_calls() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let table = MapProcessTable::chain(10, &[(20, "s20")]);
        let a = resolve_session(&p, &table, &MapEnv::new());
        let b = resolve_session(&p, &table, &MapEnv::new());
        assert_eq!(a.id, b.id);
        assert_eq!(a.rung, b.rung);
    }

    #[test]
    fn real_env_reads_the_process_environment() {
        // Read-only: assert on a variable that is certainly unset rather than
        // mutating global state under a parallel runner.
        assert!(RealEnv.get("RDM_DEFINITELY_UNSET_VARIABLE_XYZ").is_none());
    }

    #[test]
    fn resolve_system_session_never_errors_and_reports_a_cost() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let resolved = resolve_system_session(&p);
        assert!(!resolved.id.as_str().is_empty());
        assert!(resolved.rung.number() >= 1 && resolved.rung.number() <= 4);
    }

    #[test]
    fn fallback_paths_are_keyed_by_root() {
        // `fallback_for_root` reads HOME/XDG_STATE_HOME; whichever it finds,
        // two different roots must not share a base.
        if let (Some(a), Some(b)) = (
            SessionPaths::fallback_for_root(Path::new("/tmp/repo-a")),
            SessionPaths::fallback_for_root(Path::new("/tmp/repo-b")),
        ) {
            assert_ne!(a.base(), b.base());
        }
    }

    #[test]
    fn session_paths_place_state_under_the_git_dir() {
        let p = SessionPaths::for_git_dir(Path::new("/plan/.git"));
        assert_eq!(p.base(), Path::new("/plan/.git/rdm"));
        assert!(p.leases_dir().starts_with("/plan/.git"));
        assert!(p.changesets_dir().starts_with("/plan/.git"));
    }
}
