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
//! [`resolve_session`] walks a binding chain, highest precedence first, and
//! is **infallible by construction** — it returns a [`ResolvedSession`],
//! never a `Result`, so no degradation path can turn into an error on the
//! bounded git-hook deadline. The rung *labels* below (`Rung::Lease == 2`,
//! `Rung::Harness == 3`, matching [`Rung::number`] and every doc/CLI
//! reference to "rung 2"/"rung 3") are fixed, binding identifiers — only the
//! *order* [`resolve_id`] checks them has changed, as of phase 10:
//!
//! 1. **Explicit** — [`RDM_SESSION_ENV`]. Wins outright: no lease is read, no
//!    ancestry is walked, no harness variable is consulted.
//! 2. **Harness variable** (rung 3) — the first non-empty entry of
//!    [`HARNESS_SESSION_VARS`], hashed. Purely derived, so two unrelated
//!    processes agree with zero on-disk state. Checked before any lease is
//!    read or created.
//! 3. **Inherited lease** (rung 2) — the nearest ancestor process holding a
//!    valid lease (see [`lease`] for the shipped stopping rule). When no
//!    ancestor holds one and no harness variable applies, a lease is
//!    *created* at the immediate parent — still rung 2, and reached only
//!    once both explicit and harness have been ruled out.
//! 4. **Per-process** — a fresh id derived from this process. Always resolves.
//!
//! Phase 3 originally checked the inherited lease (rung 2) before the
//! harness variable (rung 3). Phase 10 reordered this: an explicit harness
//! statement of session membership must outrank an inherited on-disk lease.
//! Under the old order, a parent shell that ran one bare (harness-less) `rdm`
//! invocation minted a lease at that parent, and every child launched under
//! it — regardless of its own distinct harness session id — silently
//! inherited that lease and merged onto one changeset. Checking the harness
//! variable first removes that merging bug while leaving the no-harness case
//! (phases 3-4's bare-shell lease sharing) unchanged, since lease bootstrap
//! is still reached only once neither explicit nor harness applies.
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

/// The variable any harness can export to opt into session continuity.
///
/// Every entry of [`HARNESS_SESSION_VARS`] works, but the others name one
/// specific tool. This one is rdm's own, tool-agnostic adoption path: a
/// harness that publishes no session id of its own exports this once per
/// session and lands on rung 3. It is the variable [`continuity_advisory`]
/// names as the remedy, so it must stay a member of [`HARNESS_SESSION_VARS`]
/// or that advice would be wrong — asserted by a unit test.
pub const HARNESS_ADOPTION_VAR: &str = "RDM_HARNESS_SESSION_ID";

/// Harness-published session variables, in precedence order.
///
/// This list is the extension point for tools that already track a session of
/// their own: adding an entry wires that harness in without any on-disk state.
/// `CLAUDE_CODE_SESSION_ID` is the reference entry named by phase 3;
/// [`HARNESS_ADOPTION_VAR`] is the tool-agnostic entry any other harness can
/// use without a code change here.
pub const HARNESS_SESSION_VARS: &[&str] = &[
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_SESSION_ID",
    HARNESS_ADOPTION_VAR,
];

/// How long a harness barrier waits for its release file before proceeding
/// regardless.
pub const HARNESS_BARRIER_CEILING: std::time::Duration = std::time::Duration::from_secs(60);

/// The suffix [`harness_barrier`] appends to a barrier path to announce that a
/// process has parked there.
pub const HARNESS_BARRIER_PARKED_SUFFIX: &str = ".parked";

/// Parks at the harness barrier named by the environment variable `var`.
///
/// The single implementation behind every `RDM_HARNESS_*_BARRIER` seam: the
/// flush barrier in `rdm-store-fs` and the journal, append and compaction
/// barriers in [`journal`]. A barrier lets a harness hold one real `rdm`
/// process inside a window that opens and closes within one invocation, and
/// drive a second process to completion before releasing it.
///
/// Inert unless `var` is set to a non-empty path `<m>`. When it is, the
/// process first creates the sibling file `<m>.parked` (see
/// [`HARNESS_BARRIER_PARKED_SUFFIX`]) — the readiness signal a harness waits
/// for instead of guessing from elapsed time — and then polls until `<m>`
/// exists. The append and compaction seams park with the journal lock
/// already held (shared and exclusive respectively), so there the readiness
/// file also means "the lock is held". Creating it is best effort: a failed
/// write is ignored and never changes what the process does next.
///
/// Bounded unconditionally: a harness that never creates `<m>` delays the
/// process by at most [`HARNESS_BARRIER_CEILING`], it never wedges it.
pub fn harness_barrier(var: &str) {
    let Ok(marker) = std::env::var(var) else {
        return;
    };
    if marker.is_empty() {
        return;
    }
    let marker = PathBuf::from(marker);
    let mut parked = marker.clone().into_os_string();
    parked.push(HARNESS_BARRIER_PARKED_SUFFIX);
    let _ = std::fs::write(PathBuf::from(parked), b"");
    let deadline = Instant::now() + HARNESS_BARRIER_CEILING;
    while !marker.exists() && Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

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
    /// Whether this invocation reached rung 2 by *creating* a lease rather
    /// than inheriting one.
    ///
    /// True only on a rung-2 bootstrap; false on every other rung and on an
    /// adopted lease. It is the signal that distinguishes "this shell owns a
    /// changeset its later invocations will join" from "every invocation is
    /// minting its own", which is what [`continuity_advisory`] keys on.
    pub lease_bootstrapped: bool,
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
    let (id, rung, lease_bootstrapped) = resolve_id(paths, procs, env);
    ResolvedSession {
        id,
        rung,
        resolve_micros: started.elapsed().as_micros(),
        lease_bootstrapped,
    }
}

/// Resolves the id, the rung that produced it, and whether a rung-2 lease was
/// *created* (rather than inherited) along the way.
fn resolve_id(
    paths: &SessionPaths,
    procs: &dyn ProcessTable,
    env: &dyn EnvSource,
) -> (SessionId, Rung, bool) {
    // Rung 1 — explicit. Returns immediately: no lease read, no ancestry walk,
    // no harness-variable read.
    if let Some(raw) = env.get(RDM_SESSION_ENV)
        && let Some(id) = SessionId::new(&raw)
    {
        return (id, Rung::Explicit, false);
    }

    // Rung 3 — a harness-published id, checked before any lease is
    // consulted. An explicit harness statement of session membership must
    // outrank an inherited on-disk lease: without this, two sessions
    // launched under one already-leased ancestor shell (e.g. a parent
    // terminal that ran a bare, harness-less `rdm` once) would both adopt
    // that ancestor's lease and silently share a changeset. See phase 10 of
    // the plan-repo-concurrency roadmap.
    if let Some((var, raw)) = active_harness_var(env) {
        let digest = hex16(&[var, &raw]);
        if let Some(id) = SessionId::new(&format!("h-{digest}")) {
            return (id, Rung::Harness, false);
        }
    }

    // Rung 2 — an already-inherited ancestor lease. Reached only once no
    // harness variable applies.
    if let Some(id) = lease::adopt_inherited(paths, procs) {
        return (id, Rung::Lease, false);
    }

    // Rung 2 (bootstrap) — create the lease this session's later invocations
    // will inherit. Creation happens only at the immediate parent, and only
    // once both explicit and harness rungs have been ruled out.
    if let Some(id) = lease::create_at_parent(paths, procs) {
        return (id, Rung::Lease, true);
    }

    // Rung 4 — always resolves.
    (per_process_id(paths, procs), Rung::Process, false)
}

/// Returns the first entry of [`HARNESS_SESSION_VARS`] set to a non-empty
/// value, paired with its trimmed value — the same test [`resolve_id`] uses
/// to decide whether rung 3 applies.
///
/// Exposed beyond `resolve_id` so [`journal::adopt_changeset`] can detect,
/// before repointing an ancestor lease (rung 2 state), that a harness
/// variable is present and would make the repoint unreachable: since phase
/// 10, rung 3 is checked before rung 2, so a caller with a harness variable
/// set never falls through to an inherited or repointed lease.
pub(crate) fn active_harness_var(env: &dyn EnvSource) -> Option<(&'static str, String)> {
    for var in HARNESS_SESSION_VARS {
        if let Some(raw) = env.get(var) {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                return Some((var, trimmed.to_string()));
            }
        }
    }
    None
}

/// Explains, in plain language, why the caller's harness gives `rdm` no
/// continuity across invocations — and how to fix it.
///
/// # What it detects
///
/// A harness that spawns a *fresh wrapper shell per tool call* and publishes
/// none of [`HARNESS_SESSION_VARS`] leaves rdm nothing stable to key a session
/// on: the only ancestor it may mint a lease at (see [`lease`] for why
/// creation never ascends) is that wrapper, which dies moments later. Every
/// invocation then resolves its own changeset, so a mutation made in one tool
/// call is "another changeset" to the `rdm commit` in the next and nothing
/// lands.
///
/// The condition is exactly that shape:
///
/// - no harness variable is set — a caller on rung 3 has continuity by
///   construction and must never be nagged; and
/// - this invocation either *created* its rung-2 lease
///   ([`ResolvedSession::lease_bootstrapped`]) or fell all the way through to
///   rung 4, i.e. it inherited nothing from any earlier invocation.
///
/// A caller that *adopted* an existing lease has continuity and gets `None`,
/// which is what keeps a plain interactive shell quiet.
///
/// # Why it is advisory and not an error
///
/// Fragmentation is the safe direction (phase 3's binding asymmetry: stopping
/// too high merges concurrent sessions, stopping too low merely splits one).
/// The remedy is one exported variable, so the right response is to say so —
/// not to refuse the command.
///
/// Returns `None` whenever there is nothing useful to say. Callers decide
/// *where* to print it; see `rdm-cli`'s `commit`, which prints it only on the
/// branches that are actually symptoms.
pub fn continuity_advisory(resolved: &ResolvedSession, env: &dyn EnvSource) -> Option<String> {
    if active_harness_var(env).is_some() {
        return None;
    }
    let inherited_nothing = resolved.rung == Rung::Process
        || (resolved.rung == Rung::Lease && resolved.lease_bootstrapped);
    if !inherited_nothing {
        return None;
    }
    let vars = HARNESS_SESSION_VARS.join(", ");
    // Built line by line rather than as one continued literal: a trailing `\`
    // in a Rust string strips the next line's leading whitespace, which would
    // silently flatten the indentation this block relies on to read as a
    // labelled advisory rather than a wall of prose.
    //
    // The opening line states only what is certainly true of THIS invocation.
    // The diagnosis is deliberately conditioned on "if that repeats": the same
    // rung-2 bootstrap happens on the first `rdm` command from an ordinary
    // long-lived shell, which does have continuity from its second command on,
    // and telling that user their harness is broken would be simply wrong.
    let lines = [
        format!(
            "Note: this invocation started a new changeset ({}, rung {}) rather than \
             joining one an earlier invocation began.",
            resolved.id,
            resolved.rung.number()
        ),
        format!(
            "  Cause:  if that happens on every `rdm` call, the harness running them starts \
             a fresh shell per command and publishes none of {vars} — so rdm has nothing \
             outliving a single command to key a session on, each call becomes its own \
             changeset, and a commit finds the previous call's work attributed elsewhere. \
             (From a normal long-lived shell this line is expected once, on the first \
             command, and continuity works from the next one on.)"
        ),
        "  Remedy: if it is the former, export a stable per-session id once in the harness, \
         before it runs any rdm command:"
            .to_string(),
        format!("            export {HARNESS_ADOPTION_VAR}=<stable per-session id>"),
        "          or pin one explicitly for a single script or CI job:".to_string(),
        format!("            export {RDM_SESSION_ENV}=<id>"),
    ];
    Some(lines.join("\n"))
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
    fn harness_var_beats_inherited_lease() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let table = MapProcessTable::chain(10, &[(20, "s20")]);
        let leased = lease::create_at_parent(&p, &table).unwrap();
        let env = MapEnv::new().with("CLAUDE_CODE_SESSION_ID", "abc123");

        let resolved = resolve_session(&p, &table, &env);
        assert_ne!(resolved.id, leased);
        assert_eq!(resolved.rung, Rung::Harness);
    }

    #[test]
    fn harness_var_beats_an_already_inherited_lease() {
        // Reproduces the phase-10 merging bug: a parent shell runs a bare
        // (harness-less) rdm invocation first, minting an ancestor lease.
        // Two children of that same ancestor, each carrying its own harness
        // session id, must NOT be forced onto the inherited lease — an
        // explicit harness statement of session membership outranks an
        // on-disk artifact created before that harness var was ever set.
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let ancestor = MapProcessTable::chain(10, &[(20, "s20")]);
        let leased = lease::create_at_parent(&p, &ancestor).unwrap();

        // Two "children" of the same ancestor (pid 20), distinct harness ids.
        let child_one_table = MapProcessTable::chain(30, &[(20, "s20")]);
        let child_two_table = MapProcessTable::chain(31, &[(20, "s20")]);
        let env_one = MapEnv::new().with("CLAUDE_CODE_SESSION_ID", "child-one");
        let env_two = MapEnv::new().with("CLAUDE_CODE_SESSION_ID", "child-two");

        let child_one = resolve_session(&p, &child_one_table, &env_one);
        let child_two = resolve_session(&p, &child_two_table, &env_two);

        assert_eq!(child_one.rung, Rung::Harness);
        assert_eq!(child_two.rung, Rung::Harness);
        assert_ne!(child_one.id, child_two.id, "distinct harness ids diverge");
        assert_ne!(
            child_one.id, leased,
            "must not adopt the ancestor's inherited lease"
        );
        assert_ne!(
            child_two.id, leased,
            "must not adopt the ancestor's inherited lease"
        );

        // The same child invoked twice resolves the same id.
        let child_one_again = resolve_session(&p, &child_one_table, &env_one);
        assert_eq!(child_one.id, child_one_again.id);
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
        // The advisory tells operators to export HARNESS_ADOPTION_VAR. That
        // advice is only true while the variable is actually consulted.
        assert!(
            HARNESS_SESSION_VARS.contains(&HARNESS_ADOPTION_VAR),
            "continuity_advisory names {HARNESS_ADOPTION_VAR} as the remedy, but resolve_id \
             does not read it"
        );
    }

    #[test]
    fn advisory_fires_only_when_this_invocation_inherited_nothing() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let bare = MapEnv::new();

        // Rung 4: no ancestry at all, so nothing can ever be inherited.
        let orphan = resolve_session(&p, &MapProcessTable::empty(10), &bare);
        assert_eq!(orphan.rung, Rung::Process);
        let text = continuity_advisory(&orphan, &bare).expect("rung 4 must advise");
        assert!(
            text.contains("RDM_HARNESS_SESSION_ID"),
            "the remedy is missing: {text}"
        );
        assert!(
            text.contains("CLAUDE_CODE_SESSION_ID"),
            "the cause is missing: {text}"
        );
        assert!(text.contains("Cause:") && text.contains("Remedy:"));

        // Rung 2 bootstrap: this invocation minted the lease itself, so no
        // earlier invocation shares its changeset.
        let table = MapProcessTable::chain(10, &[(20, "s20")]);
        let bootstrap = resolve_session(&p, &table, &bare);
        assert_eq!(bootstrap.rung, Rung::Lease);
        assert!(bootstrap.lease_bootstrapped);
        assert!(continuity_advisory(&bootstrap, &bare).is_some());

        // Rung 2 adoption: continuity is working, so there is nothing to say.
        // This is the case that keeps a plain interactive shell quiet.
        let later = resolve_session(&p, &MapProcessTable::chain(11, &[(20, "s20")]), &bare);
        assert_eq!(later.rung, Rung::Lease);
        assert!(!later.lease_bootstrapped);
        assert!(
            continuity_advisory(&later, &bare).is_none(),
            "a shell whose second invocation adopted its own lease has continuity"
        );
    }

    #[test]
    fn advisory_is_silent_for_every_caller_that_already_has_continuity() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);

        // Rung 3 — the remedy is already applied. Advising here would nag the
        // callers who did the right thing, and would contradict phase 10's
        // rule that a harness variable outranks any lease.
        for var in HARNESS_SESSION_VARS {
            let env = MapEnv::new().with(var, "session-value");
            let resolved = resolve_session(&p, &MapProcessTable::empty(10), &env);
            assert_eq!(resolved.rung, Rung::Harness);
            assert!(
                continuity_advisory(&resolved, &env).is_none(),
                "{var} is set, so rung 3 already supplies continuity"
            );
        }

        // Rung 1 — an explicitly pinned id is continuity by definition.
        let env = MapEnv::new().with(RDM_SESSION_ENV, "ci-run-7");
        let resolved = resolve_session(&p, &MapProcessTable::empty(10), &env);
        assert_eq!(resolved.rung, Rung::Explicit);
        assert!(continuity_advisory(&resolved, &env).is_none());

        // A harness variable also silences a rung-4 resolution that somehow
        // reached it: `active_harness_var` is checked first, unconditionally.
        let harnessed = ResolvedSession {
            id: SessionId::new("p-whatever").unwrap(),
            rung: Rung::Process,
            resolve_micros: 0,
            lease_bootstrapped: false,
        };
        let env = MapEnv::new().with("CLAUDE_CODE_SESSION_ID", "abc");
        assert!(continuity_advisory(&harnessed, &env).is_none());
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
