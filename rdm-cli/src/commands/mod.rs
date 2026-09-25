use std::io::{self, Read, Write};
use std::path::Path;
#[cfg(feature = "git")]
use std::sync::mpsc;
#[cfg(feature = "git")]
use std::thread;
#[cfg(feature = "git")]
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use is_terminal::IsTerminal;
use rdm_core::model::PhaseStatus;
use rdm_core::search::ItemStatus;

#[cfg(feature = "git")]
use crate::paths;
use crate::{AppStore, ItemKindArg, OutputFormat};

pub mod agent_config;
pub mod backlinks;
pub mod backlog;
pub mod config;
pub mod cost;
pub mod cost_report;
pub mod describe;
pub mod info;
pub mod init;
pub mod link;
pub mod list;
pub mod model;
pub mod next;
pub mod phase;
pub mod plan;
pub mod project;
pub mod promote;
pub mod roadmap;
pub mod run;
pub mod search;
pub mod session;
pub mod tag;
pub mod task;
pub mod tree;

#[cfg(feature = "git")]
pub mod bootstrap;
#[cfg(feature = "git")]
pub mod commit;
#[cfg(feature = "git")]
pub mod conflicts;
#[cfg(feature = "git")]
pub mod discard;
#[cfg(feature = "git")]
pub mod hook;
#[cfg(feature = "git")]
pub mod remote;
#[cfg(feature = "git")]
pub mod resolve;
#[cfg(feature = "git")]
pub mod review;
#[cfg(feature = "git")]
pub mod status;
/// `rdm verify resolve` / `rdm verify run` — the CLI surface over the
/// `dispatch.verify` key, resolved for the command's project.
#[cfg(feature = "git")]
pub mod verify;
#[cfg(feature = "git")]
pub mod worktree;

#[cfg(feature = "server")]
pub mod serve;

/// The environment variable Claude Code sets to the running session's uuid.
///
/// Read raw by `rdm cost` (its default session) and `rdm run record` (the
/// session a run is recorded against); neither hashes it or consults any
/// other harness variable.
pub const CLAUDE_SESSION_ENV: &str = "CLAUDE_CODE_SESSION_ID";

/// Parses a status string into an `ItemStatus`, using the `--type` hint if available.
pub fn parse_status(status: &str, kind: Option<ItemKindArg>) -> Result<ItemStatus> {
    use rdm_core::model::TaskStatus;

    match kind {
        Some(ItemKindArg::Phase) => {
            let s: PhaseStatus = status.parse()?;
            Ok(ItemStatus::Phase(s))
        }
        Some(ItemKindArg::Task) => {
            let s: TaskStatus = status.parse()?;
            Ok(ItemStatus::Task(s))
        }
        Some(ItemKindArg::Roadmap) => {
            bail!("roadmaps do not have a status — remove --status or change --type")
        }
        Some(ItemKindArg::Review) => {
            bail!(
                "reviews do not use --status in search — filter with `rdm review list --state <state>` (or --verdict) instead"
            )
        }
        None => {
            // Try both; a status valid for both kinds becomes kind-agnostic.
            match (
                status.parse::<PhaseStatus>().ok(),
                status.parse::<TaskStatus>().ok(),
            ) {
                (Some(p), Some(t)) => Ok(ItemStatus::Either(p, t)),
                (Some(p), None) => Ok(ItemStatus::Phase(p)),
                (None, Some(t)) => Ok(ItemStatus::Task(t)),
                (None, None) => bail!(
                    "invalid status '{status}' — use a phase status (not-started, in-progress, needs-review, reviewed, done, blocked, wont-fix) or task status (open, in-progress, needs-review, reviewed, done, wont-fix)"
                ),
            }
        }
    }
}

/// Opens a store for an existing plan repo.
pub fn make_store(root: &Path) -> Result<AppStore> {
    #[cfg(feature = "git")]
    {
        rdm_store_git::GitStore::new(root).context("failed to open git repository")
    }
    #[cfg(not(feature = "git"))]
    {
        Ok(rdm_store_fs::FsStore::new(root))
    }
}

/// Creates a store for initializing a new plan repo.
pub fn make_init_store(root: &Path) -> Result<AppStore> {
    #[cfg(feature = "git")]
    {
        rdm_store_git::GitStore::init(root).context("failed to initialize git repository")
    }
    #[cfg(not(feature = "git"))]
    {
        Ok(rdm_store_fs::FsStore::new(root))
    }
}

/// Resolve body content from `--body` flag, piped stdin, or interactive editor.
///
/// `--body` is authoritative: when `body_flag` is `Some`, stdin is not read
/// at all. This avoids fragile interactions with background and other
/// non-interactive runners that may leave bytes on stdin even when the
/// caller intended to pass the body inline.
pub fn resolve_body(body_flag: Option<String>, no_edit: bool) -> Result<Option<String>> {
    if let Some(b) = body_flag {
        return Ok(Some(b));
    }

    let is_tty = io::stdin().is_terminal();
    let stdin_body = if !is_tty {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf)?;
        let trimmed = buf.trim_end_matches('\n');
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    } else {
        None
    };

    match stdin_body {
        Some(s) => Ok(Some(s)),
        None => {
            if no_edit || !is_tty {
                Ok(None)
            } else {
                open_editor()
            }
        }
    }
}

/// Resolve body content for a `review start`/`comment`/`submit` invocation.
///
/// Unlike [`resolve_body`] (used by the `create`-family commands, where a
/// piped-heredoc body is documented ergonomics), this helper **never reads
/// stdin**. That's the fix: `resolve_body` unconditionally blocked on
/// `io::stdin().read_to_string(...)` whenever stdin was not a TTY, *before*
/// `no_edit` was even consulted — under an agent's Bash tool stdin is a
/// non-TTY pipe that is never closed, so the read never returned, and
/// `--no-edit` could not save you because the hang happened first.
///
/// `--body` remains authoritative. Otherwise, on a genuine TTY with
/// `--no-edit` absent, the interactive `$EDITOR`/`$VISUAL` flow still runs —
/// that path never blocked (it's real human input at a real terminal) and
/// dropping it would be an unrelated UX regression. In every other case
/// (non-TTY, or `--no-edit` passed) this returns `None` rather than reading
/// anything from stdin.
///
/// This also doesn't reuse `BodyUpdate`'s Keep/Set/Clear shape (the fix for
/// the same class of bug in `task`/`phase`/`roadmap update`): `start`,
/// `comment`, and `submit` have no existing value to "keep", so that type
/// doesn't fit here.
#[cfg(feature = "git")]
pub fn resolve_review_body(body_flag: Option<String>, no_edit: bool) -> Result<Option<String>> {
    if let Some(b) = body_flag {
        return Ok(Some(b));
    }

    if !no_edit && io::stdin().is_terminal() {
        open_editor()
    } else {
        Ok(None)
    }
}

/// Map a [`rdm_core::error::Error::BodyClobberRefused`] into an actionable
/// CLI error message that points at `--clear-body`.
pub fn map_body_clobber(err: anyhow::Error) -> anyhow::Error {
    if let Some(rdm_core::error::Error::BodyClobberRefused) =
        err.downcast_ref::<rdm_core::error::Error>()
    {
        return anyhow::anyhow!(
            "refusing to clear an existing body; pass `--clear-body` to confirm or `--body <text>` to replace it"
        );
    }
    err
}

/// Launch `$VISUAL` / `$EDITOR` / `vi` to interactively edit body content.
/// Returns `None` if the user saves an empty file.
pub fn open_editor() -> Result<Option<String>> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());

    let mut tmp = tempfile::Builder::new()
        .suffix(".md")
        .tempfile()
        .context("failed to create temp file for editor")?;

    writeln!(
        tmp,
        "<!-- Enter body content below. This comment will be removed. -->"
    )?;
    tmp.flush()?;

    let path = tmp.path().to_owned();

    let status = std::process::Command::new(&editor)
        .arg(&path)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()
        .with_context(|| format!("failed to launch editor '{editor}'"))?;

    if !status.success() {
        bail!("editor exited with non-zero status");
    }

    let content = std::fs::read_to_string(&path).context("failed to read editor temp file")?;

    let body: String = content
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !(trimmed.starts_with("<!--") && trimmed.ends_with("-->"))
        })
        .collect::<Vec<_>>()
        .join("\n");
    let body = body.trim().to_string();

    if body.is_empty() {
        Ok(None)
    } else {
        Ok(Some(body))
    }
}

pub fn reject_non_human(format: OutputFormat, command_name: &str) -> Result<()> {
    if format != OutputFormat::Human {
        bail!(
            "--format {format} is not supported for '{command_name}'; use --format human or omit --format"
        );
    }
    Ok(())
}

/// The concrete [`WorktreeProbe`](rdm_core::worktree::WorktreeProbe) the CLI's
/// `reviewed` gate reads through.
///
/// Aliased rather than named inline so the phase and task `update` arms stay
/// feature-agnostic: without the `git` feature there is no worktree to probe,
/// and the always-empty in-memory double stands in so the call sites need no
/// second code path.
#[cfg(feature = "git")]
pub type GateProbe = rdm_git::worktree::GitWorktreeProbe;
/// See the `git`-enabled alias above.
#[cfg(not(feature = "git"))]
pub type GateProbe = rdm_core::worktree::MemoryWorktreeProbe;

/// Builds the [`GateProbe`] the `reviewed` gate's worktree precondition (c)
/// reads through, or `None` when there is nothing to probe.
///
/// The single place the `git`/non-`git` split is spelled out, so the `phase
/// update` and `task update` arms stay feature-agnostic — the contract
/// [`GateProbe`] itself documents. Without the `git` feature rdm cannot
/// resolve a worktree at all, so precondition (c) is never applicable and the
/// probe is always `None`; preconditions (a) and (b) are unaffected and still
/// enforce.
///
/// With `git`, the probe degrades to `None` when the cwd is not a distinct
/// project repo: rule (c) is "if `rdm worktree` knows one", and from outside a
/// project checkout it knows nothing. A deliberate fail-open on (c) alone.
///
/// # Errors
///
/// Never returns an error: an undiscoverable project repo is reported as
/// "no worktree to check", not as a failure.
#[cfg(feature = "git")]
#[must_use]
pub fn build_gate_probe(enabled: bool, root: &Path) -> Option<GateProbe> {
    if !enabled {
        return None;
    }
    std::env::current_dir()
        .ok()
        .and_then(|cwd| rdm_git::worktree::discover_distinct_project_repo(&cwd, root).ok())
        .map(GateProbe::new)
}

/// See the `git`-enabled builder above: without `git` there is no worktree to
/// resolve, so precondition (c) is never applicable.
#[cfg(not(feature = "git"))]
#[must_use]
pub fn build_gate_probe(_enabled: bool, _root: &Path) -> Option<GateProbe> {
    None
}

/// Assembles the [`ReviewedGate`](rdm_core::ops::ReviewedGate) one `phase
/// update` / `task update` invocation is evaluated against.
///
/// Shared by both arms so the two can never drift in *when* the gate enforces,
/// which probe it reads through, or how an operator override is attached.
///
/// An operator override is attached even when `enabled` is `false`, so core
/// can *refuse* it rather than silently drop it: an override honored as a
/// no-op would discard the reason and actor the operator supplied, and the
/// override exists precisely so that a bypass is an audited act. See
/// [`Error::GateOverrideGateDisabled`](rdm_core::error::Error::GateOverrideGateDisabled).
pub fn build_reviewed_gate<'a>(
    enabled: bool,
    probe: Option<&'a GateProbe>,
    reason: Option<&'a str>,
    actor: Option<&'a str>,
) -> rdm_core::ops::ReviewedGate<'a> {
    let gate = if enabled {
        rdm_core::ops::ReviewedGate::enforcing(
            probe.map(|p| p as &dyn rdm_core::worktree::WorktreeProbe),
        )
    } else {
        rdm_core::ops::ReviewedGate::disabled()
    };
    match (reason, actor) {
        (Some(r), Some(a)) => gate.with_override(r, a),
        _ => gate,
    }
}

/// Runs a mutating op as a single transaction and prints the staging hint.
///
/// Wraps `f` in [`rdm_core::ops::mutate`], so the entity write and the single
/// staged flush happen together. `context` labels any failure.
///
/// The trailing `rdm commit` hint is always printed — staging is the only
/// workflow, and the hint names the caller's *changeset* rather than the
/// working tree because that is what the eventual `rdm commit` will land:
/// this is the first place an agent meets the concept, so it must not imply
/// the whole plan repo is about to be swept up.
pub fn commit_mutation<T>(
    store: &mut AppStore,
    context: &str,
    f: impl FnOnce(&mut AppStore) -> rdm_core::error::Result<T>,
) -> Result<T> {
    let out = rdm_core::ops::mutate(store, f).with_context(|| context.to_string())?;
    #[cfg(feature = "git")]
    eprintln!("  (staged in this session's changeset — run `rdm commit` to persist)");
    Ok(out)
}

/// Prints a hint about uncommitted changes.
///
/// Called after read-only commands (list, show, search) so the user is aware
/// that the data they see includes uncommitted staged mutations.
///
/// Counts `report.user`. rdm has no generated-path class, so in the
/// whole-tree view that is every dirty path — a stale tracked `INDEX.md` left
/// over from a plan repo that predates the removal of `rdm index` now counts
/// toward the hint like any other uncommitted file, and `rdm status` lists it
/// by name.
#[cfg(feature = "git")]
pub fn maybe_print_uncommitted_hint(store: &AppStore) {
    if let Ok(report) = store.git().git_status_report()
        && !report.user.is_empty()
    {
        eprintln!(
            "\n  ({} uncommitted change(s) — run `rdm status` for details)",
            report.user.len()
        );
    }
}

#[cfg(not(feature = "git"))]
pub fn maybe_print_uncommitted_hint(_store: &AppStore) {}

/// Built-in hook execution deadline (seconds) used when `hook_timeout_secs`
/// is unset, or configured to `0`. `0` is deliberately normalized to this
/// default rather than treated as "unbounded" — an unbounded timeout would
/// defeat the purpose of the guard entirely.
#[cfg(feature = "git")]
pub const DEFAULT_HOOK_TIMEOUT_SECS: u64 = 30;

/// Resolves the hook execution deadline (in seconds) for the plan repo at
/// `root`, following the same repo-config-then-global-then-built-in-default
/// precedence as `default_branch`.
#[cfg(feature = "git")]
pub fn resolve_hook_timeout_secs(root: &Path) -> u64 {
    let global = paths::load_global_config();
    let repo = paths::load_repo_config(root);
    resolve_hook_timeout_secs_inner(&repo, &global)
}

/// Pure core of [`resolve_hook_timeout_secs`], split out for direct unit
/// testing: merges repo over global via `with_global_defaults`, then
/// normalizes unset **and `0`** to [`DEFAULT_HOOK_TIMEOUT_SECS`] — a `0`
/// deadline would mean "unbounded", which defeats the guard. Note that a
/// repo-level `hook_timeout_secs = 0` does NOT fall back to a global value:
/// `with_global_defaults` merges `Some(0)` as a present value, so `0` in the
/// repo config always yields the built-in default even when the global
/// config carries a nonzero setting.
#[cfg(feature = "git")]
fn resolve_hook_timeout_secs_inner(
    repo: &rdm_core::config::Config,
    global: &rdm_core::config::GlobalConfig,
) -> u64 {
    repo.with_global_defaults(global)
        .hook_timeout_secs
        .filter(|&secs| secs > 0)
        .unwrap_or(DEFAULT_HOOK_TIMEOUT_SECS)
}

/// Bounds the execution of `f` — the body of a `post-merge`/`post-commit`
/// hook invocation — to `timeout`, run on a fresh thread.
///
/// If `f` completes before `timeout` elapses, its result is returned
/// directly. If `f` is still running when the deadline passes, a
/// `"timeout"` event (with the elapsed wall-clock time) is logged via
/// `logger` and `Ok(())` is returned immediately, **without** joining or
/// killing the still-running worker thread — it (and anything it is
/// blocked on, such as a hung `git` child process or an interactive editor)
/// is deliberately abandoned. This is safe under the hooks' existing
/// contract: `hook.rs`'s `PostMerge`/`PostCommit` arms always let the
/// process exit immediately after this call returns (they must always exit
/// 0 to avoid blocking git), which tears down every thread in the process
/// regardless of what it's doing. If `f` panics instead of completing, a
/// distinct `"panicked"` event is logged (the worker's channel sender is
/// dropped without a send) and `Ok(())` is likewise returned.
///
/// # Errors
///
/// Returns whatever `f` returns if it completes before `timeout` elapses.
/// Never returns an error on timeout or panic — the hook still exits 0, by
/// design (see above).
#[cfg(feature = "git")]
pub fn run_hook_with_timeout<F>(
    timeout: Duration,
    logger: &crate::hook_log::HookLogger,
    hook: &str,
    f: F,
) -> Result<()>
where
    F: FnOnce() -> Result<()> + Send + 'static,
{
    let start = Instant::now();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let result = f();
        // The receiver may already be gone (timed out and returned) — a
        // failed send just means nobody is listening anymore, which is fine.
        let _ = tx.send(result);
    });
    match rx.recv_timeout(timeout) {
        Ok(result) => result,
        Err(err) => {
            let elapsed = format!("{:.3}", start.elapsed().as_secs_f64());
            let timeout_str = format!("{:.3}", timeout.as_secs_f64());
            // Distinguish the deadline passing from the worker thread
            // panicking (which drops `tx` without sending): mislabeling a
            // panic as "timeout" would report a nonsensical near-zero
            // elapsed time and send debugging down the wrong path.
            let event = match err {
                mpsc::RecvTimeoutError::Timeout => "timeout",
                mpsc::RecvTimeoutError::Disconnected => "panicked",
            };
            logger.log(
                hook,
                event,
                &[
                    ("elapsed_secs", elapsed.as_str()),
                    ("timeout_secs", timeout_str.as_str()),
                ],
            );
            Ok(())
        }
    }
}

/// Per-directive bookkeeping kept alongside a batched mutation step, so the
/// commit message and per-directive log lines can be built once
/// [`rdm_core::ops::mutate_batch`] returns its per-step results.
#[cfg(feature = "git")]
#[derive(Clone)]
enum DirectiveMeta {
    /// A `Done: <roadmap>/<phase>` directive.
    Phase {
        /// The roadmap slug named in the directive.
        roadmap: String,
        /// The resolved phase stem (not the raw directive text, which may be
        /// a bare number or partial slug).
        stem: String,
        /// The source commit SHA the directive was parsed from.
        sha: String,
    },
    /// A `Done: task/<slug>` directive.
    Task {
        /// The task slug named in the directive.
        slug: String,
        /// The source commit SHA the directive was parsed from.
        sha: String,
    },
}

#[cfg(feature = "git")]
impl DirectiveMeta {
    /// Formats the directive back into `<roadmap>/<phase>` or `task/<slug>`
    /// form, for use in the batch commit message.
    fn target(&self) -> String {
        match self {
            DirectiveMeta::Phase { roadmap, stem, .. } => format!("{roadmap}/{stem}"),
            DirectiveMeta::Task { slug, .. } => format!("task/{slug}"),
        }
    }

    /// The source commit SHA carried by this directive.
    fn sha(&self) -> &str {
        match self {
            DirectiveMeta::Phase { sha, .. } | DirectiveMeta::Task { sha, .. } => sha,
        }
    }
}

/// Builds the single plan-repo commit message for a batch of applied `Done:`
/// directives: one summary line, then one `Done: <target> (<sha>)` line per
/// directive whose step succeeded (failed/skipped steps are omitted).
#[cfg(feature = "git")]
fn build_batch_commit_message(
    metas: &[DirectiveMeta],
    results: &[rdm_core::error::Result<()>],
) -> String {
    let applied: Vec<&DirectiveMeta> = metas
        .iter()
        .zip(results.iter())
        .filter_map(|(meta, result)| result.is_ok().then_some(meta))
        .collect();
    let mut message = format!("rdm: apply {} Done: directive(s)\n", applied.len());
    for meta in applied {
        let short_sha = meta.sha().get(..7).unwrap_or(meta.sha());
        message.push_str(&format!("\nDone: {} ({short_sha})", meta.target()));
    }
    message
}

/// Applies a list of `Done:` directives, marking matching phases/tasks as done
/// with the associated commit SHA.
///
/// All directives are applied as a single [`rdm_core::ops::mutate_batch`]
/// transaction: one plan-repo commit covers every directive in
/// `directives_with_sha`, rather than one commit per directive. The resulting
/// commit's message enumerates each successfully
/// applied directive as a `Done: <target> (<sha>)` line, so per-directive
/// provenance survives the collapse into a single commit. That commit is
/// produced via [`rdm_store_git::GitStore::commit_changeset`], which bypasses
/// git porcelain/hooks entirely, so it can never recursively re-trigger this
/// same hook.
///
/// # The hook commits its own changeset, and only its own
///
/// This is the one committer that fires with no human or agent deciding to
/// commit — on every merge and every default-branch commit, including
/// `rdm-land`'s fast-forward. It therefore commits **scoped**, exactly like
/// every other committer: the tree is HEAD plus this hook run's journaled
/// paths, so a session that happens to be holding uncommitted work on the
/// same plan repo does not get it swept into a `Done:` commit it never asked
/// for.
///
/// Whose changeset that is follows the ordinary identity chain (as of phase
/// 10, checked harness variable before inherited lease — see
/// `docs/session-identity.md`), and every case is correct:
///
/// - **Rung 3 (harness variable) or Rung 2 (inherited lease)** — the hook was
///   spawned by a shell already carrying a harness session id, or (absent
///   one) already holding a lease, so `mutate_batch`'s `Store::commit`
///   appends to that session's changeset and the commit lands that session's
///   work together with the status flips. This is the usual `rdm-land`
///   shape, whichever rung supplied the identity.
/// - **Rung 4 (per-process)** — neither a harness variable nor a reachable
///   lease (a bare `git merge` in a fresh process tree), so the hook resolves
///   a fresh id and its changeset contains *only* what the hook itself just
///   wrote: the status flips, and nothing else can ride along.
///
/// Bounded exactly as before: the `RDM_GIT_SUBPROCESS` short-circuit still
/// returns before any of this, and the added work (one journal read plus a
/// tree build over HEAD's entries) sits well inside `hook_timeout_secs`. A
/// journal that cannot be read degrades to an empty changeset — reported,
/// never a whole-tree sweep and never a hang.
///
/// Silently skips directives whose phase or task cannot be found. A single
/// directive's mutation failing does not abort the rest of the batch — every
/// other directive is still applied and committed.
///
/// # Errors
///
/// Returns an error if the store cannot be opened, the project cannot be
/// resolved, or the batch's shared finalize stage (the single flush) fails.
/// In the finalize-failure case, every per-directive outcome has already been
/// logged before the error is returned.
#[cfg(feature = "git")]
pub fn apply_done_directives(
    root: &Path,
    directives_with_sha: &[(rdm_core::hook::DoneDirective, String)],
    logger: &crate::hook_log::HookLogger,
    hook: &str,
) -> Result<()> {
    if directives_with_sha.is_empty() {
        logger.log(hook, "skip-empty", &[]);
        return Ok(());
    }

    // Each per-directive `ops::mutate` → `Store::commit` only flushes to
    // disk — that's the only thing `Store::commit` ever does now. After the
    // loop we land exactly one real commit via the blessed always-commit
    // pathway (`commit_changeset`).
    let mut store = match make_store(root) {
        Ok(s) => s,
        Err(e) => {
            let msg = format!("{e:#}");
            logger.log(hook, "store-open-error", &[("error", msg.as_str())]);
            return Err(e);
        }
    };
    let hook_global_config = paths::load_global_config();
    let hook_repo_config = paths::load_repo_config(root).with_global_defaults(&hook_global_config);
    let project = match paths::resolve_project(None, &hook_repo_config) {
        Ok(p) => p,
        Err(e) => {
            let msg = format!("{e:#}");
            logger.log(
                hook,
                "project-resolution-failed",
                &[("error", msg.as_str())],
            );
            return Err(e);
        }
    };

    let mut steps: Vec<rdm_core::ops::BatchStep<'_, AppStore>> = Vec::new();
    let mut metas: Vec<DirectiveMeta> = Vec::new();

    for (directive, sha) in directives_with_sha {
        match directive {
            rdm_core::hook::DoneDirective::Phase { roadmap, phase } => {
                let stem = match rdm_core::ops::phase::resolve_phase_stem(
                    &store, &project, roadmap, phase,
                ) {
                    Ok(s) => s,
                    Err(e) => {
                        let msg = format!("{e}");
                        logger.log(
                            hook,
                            "skip-unknown-phase",
                            &[
                                ("roadmap", roadmap.as_str()),
                                ("phase", phase.as_str()),
                                ("error", msg.as_str()),
                            ],
                        );
                        continue;
                    }
                };
                let project_owned = project.clone();
                let roadmap_owned = roadmap.clone();
                let sha_owned = sha.clone();
                let stem_for_step = stem.clone();
                steps.push(Box::new(move |s| {
                    // Deliberately UNGATED: this is the `Done:` post-merge /
                    // post-commit hook path, which is contractually exit-0 and
                    // must never acquire a failure mode (CLAUDE.md's hook
                    // reliability guarantee). It writes `Done`, which the
                    // `reviewed` gate never guards anyway.
                    rdm_core::ops::phase::update_phase(
                        s,
                        &project_owned,
                        &roadmap_owned,
                        &stem_for_step,
                        Some(rdm_core::model::PhaseStatus::Done),
                        rdm_core::ops::TagsUpdate::Keep,
                        rdm_core::ops::BodyUpdate::Keep,
                        Some(sha_owned),
                        None,
                        None,
                        None,
                        rdm_core::ops::TitleUpdate::Keep,
                    )
                    .map(|_| ())
                }));
                metas.push(DirectiveMeta::Phase {
                    roadmap: roadmap.clone(),
                    stem,
                    sha: sha.clone(),
                });
            }
            rdm_core::hook::DoneDirective::Task { slug } => {
                let project_owned = project.clone();
                let slug_owned = slug.clone();
                let sha_owned = sha.clone();
                steps.push(Box::new(move |s| {
                    // Deliberately UNGATED: same `Done:` hook path as above,
                    // writing `Done`.
                    rdm_core::ops::task::update_task(
                        s,
                        &project_owned,
                        &slug_owned,
                        Some(rdm_core::model::TaskStatus::Done),
                        None,
                        rdm_core::ops::TagsUpdate::Keep,
                        rdm_core::ops::BodyUpdate::Keep,
                        Some(sha_owned),
                        None,
                        None,
                        None,
                        rdm_core::ops::TitleUpdate::Keep,
                    )
                    .map(|_| ())
                }));
                metas.push(DirectiveMeta::Task {
                    slug: slug.clone(),
                    sha: sha.clone(),
                });
            }
        }
    }

    if steps.is_empty() {
        // Every directive was skipped pre-mutate (e.g. all unknown phases) —
        // nothing to batch, mirror the empty-directives no-op path.
        return Ok(());
    }

    let message_metas = metas.clone();
    let mut outcome = rdm_core::ops::mutate_batch(&mut store, steps, move |results| {
        build_batch_commit_message(&message_metas, results)
    });

    // Log every per-directive outcome unconditionally, before inspecting the
    // shared finalize result — this preserves per-directive log fidelity even
    // when the commit step below fails.
    for (meta, result) in metas.iter().zip(outcome.step_results.iter()) {
        match meta {
            DirectiveMeta::Phase { roadmap, stem, sha } => match result {
                Ok(()) => logger.log(
                    hook,
                    "apply-phase",
                    &[
                        ("status", "ok"),
                        ("roadmap", roadmap.as_str()),
                        ("phase", stem.as_str()),
                        ("sha", sha.as_str()),
                    ],
                ),
                Err(e) => {
                    let msg = format!("{e}");
                    logger.log(
                        hook,
                        "apply-phase",
                        &[
                            ("status", "error"),
                            ("roadmap", roadmap.as_str()),
                            ("phase", stem.as_str()),
                            ("sha", sha.as_str()),
                            ("error", msg.as_str()),
                        ],
                    );
                }
            },
            DirectiveMeta::Task { slug, sha } => match result {
                Ok(()) => logger.log(
                    hook,
                    "apply-task",
                    &[
                        ("status", "ok"),
                        ("slug", slug.as_str()),
                        ("sha", sha.as_str()),
                    ],
                ),
                Err(e) => {
                    let msg = format!("{e}");
                    logger.log(
                        hook,
                        "apply-task",
                        &[
                            ("status", "error"),
                            ("slug", slug.as_str()),
                            ("sha", sha.as_str()),
                            ("error", msg.as_str()),
                        ],
                    );
                }
            },
        }
    }

    // Surface any flush failure before attempting the commit — if the store
    // never flushed cleanly there is nothing safe to commit.
    if let Err(e) = outcome.finalize_result {
        let msg = format!("{e}");
        logger.log(hook, "batch-commit-error", &[("error", msg.as_str())]);
        return Err(e.into());
    }

    // Land exactly one real git commit for the whole batch through the
    // blessed *scoped* pathway. `mutate_batch` only flushed to disk (the
    // store's `commit` stages and journals, never touching git);
    // `commit_message` is `Some` iff at least one directive applied, so an
    // all-skipped batch produces no empty commit.
    if let Some(message) = outcome.commit_message.take() {
        match store.commit_changeset(Some(&message), &[]) {
            Ok(scoped) => {
                // Attributable after the fact: which changeset the hook
                // committed, and how many paths it actually landed.
                let count = scoped.committed.len().to_string();
                let changeset = scoped.changeset.clone().unwrap_or_default();
                let sha = scoped.sha.clone().unwrap_or_default();
                logger.log(
                    hook,
                    "batch-commit",
                    &[
                        ("changeset", changeset.as_str()),
                        ("paths", count.as_str()),
                        ("sha", sha.as_str()),
                    ],
                );
            }
            Err(e) => {
                let msg = format!("{e}");
                logger.log(hook, "batch-commit-error", &[("error", msg.as_str())]);
                return Err(e.into());
            }
        }
    }

    Ok(())
}

/// Test-only stall injector used to exercise the hook-timeout wrapper
/// ([`run_hook_with_timeout`]) end-to-end through the real compiled `rdm`
/// binary. When the `RDM_TEST_STALL_HOOK_MS` environment variable is set to
/// a valid millisecond count, sleeps for that long before the hook does any
/// further work; a complete no-op otherwise (the variable is never set
/// outside of tests). This exists because the hook body itself
/// (`apply_done_directives`) has no genuine, deterministic way to hang from
/// the outside without a real stuck `git` subprocess or editor — which AC2
/// and AC3 fix — so the integration test for AC1 needs a controlled way to
/// simulate "the hook body is still running past its deadline".
#[cfg(feature = "git")]
fn maybe_test_stall_hook() {
    if let Ok(ms) = std::env::var("RDM_TEST_STALL_HOOK_MS")
        && let Ok(ms) = ms.parse::<u64>()
    {
        thread::sleep(Duration::from_millis(ms));
    }
}

/// Runs the post-merge hook logic: parse `Done:` directives from commits
/// and mark matching phases done.
///
/// When `since` is `None`, scans commits introduced by the most recent merge
/// (using the reflog anchor `HEAD@{1}`). When `since` is `Some(ref)`, scans
/// all commits reachable from HEAD but not from the given ref.
///
/// All errors are intentionally swallowed by the caller — this must never
/// block a git merge. Execution is additionally bounded by
/// [`run_hook_with_timeout`] at the `hook.rs` call site, so even a body that
/// hangs past its configured deadline cannot block the invoking `git merge`
/// indefinitely.
///
/// If this process was itself spawned as a git subprocess by rdm (tagged
/// with [`rdm_git::RDM_GIT_SUBPROCESS_ENV`]), it returns immediately without
/// touching the store — see that constant's doc comment, and
/// [`GitRepo::create_git_commit`](rdm_store_git::GitRepo::create_git_commit)'s,
/// for the full re-entrancy rationale.
#[cfg(feature = "git")]
pub fn run_post_merge_hook(root: &Path, since: Option<&str>) -> Result<()> {
    let cwd = std::env::current_dir().context("cannot determine current directory")?;
    let logger = crate::hook_log::HookLogger::new(&cwd);
    let hook = "post-merge";
    let cwd_str = cwd.display().to_string();
    let timeout_secs = resolve_hook_timeout_secs(root).to_string();
    logger.log(
        hook,
        "entry",
        &[
            ("cwd", cwd_str.as_str()),
            ("since", since.unwrap_or("")),
            ("timeout_secs", timeout_secs.as_str()),
        ],
    );

    if std::env::var(rdm_git::RDM_GIT_SUBPROCESS_ENV).is_ok() {
        logger.log(hook, "skip-reentrant", &[]);
        logger.log(hook, "exit", &[("ok", "true")]);
        return Ok(());
    }

    maybe_test_stall_hook();

    let commits = match rdm_git::commit_messages_since_at(&cwd, since) {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("{e}");
            logger.log(hook, "git-error", &[("error", msg.as_str())]);
            logger.log(hook, "exit", &[("ok", "false")]);
            return Err(e.into());
        }
    };
    if commits.is_empty() {
        logger.log(hook, "skip-no-commits", &[]);
        logger.log(hook, "exit", &[("ok", "true")]);
        return Ok(());
    }

    // Collect directives from all commits. Commits are newest-first, so the
    // first occurrence of a directive wins (latest SHA).
    let mut seen = std::collections::HashSet::new();
    let mut directives_with_sha = Vec::new();
    for commit in &commits {
        for directive in rdm_core::hook::parse_done_directives(&commit.message) {
            if seen.insert(directive.clone()) {
                directives_with_sha.push((directive, commit.sha.clone()));
            }
        }
    }

    let count = directives_with_sha.len().to_string();
    logger.log(hook, "parsed-directives", &[("count", count.as_str())]);

    let result = apply_done_directives(root, &directives_with_sha, &logger, hook);
    logger.log(
        hook,
        "exit",
        &[("ok", if result.is_ok() { "true" } else { "false" })],
    );
    result
}

/// Runs the post-commit hook logic: on the default branch, parse `Done:`
/// directives from HEAD and mark matching phases/tasks done.
///
/// Skips processing if the current branch is not the default branch
/// (configured via `default_branch` in config, falling back to `"main"`).
///
/// All errors are intentionally swallowed by the caller — this must never
/// block a git commit. Execution is additionally bounded by
/// [`run_hook_with_timeout`] at the `hook.rs` call site, so even a body that
/// hangs past its configured deadline cannot block the invoking `git commit`
/// indefinitely.
///
/// If this process was itself spawned as a git subprocess by rdm (tagged
/// with [`rdm_git::RDM_GIT_SUBPROCESS_ENV`]), it returns immediately without
/// touching the store, instead of re-running the full `Done:`-directive
/// pipeline. This matters because
/// [`GitRepo::create_git_commit`](rdm_store_git::GitRepo::create_git_commit)
/// — the commit path behind every *ordinary* plan-repo mutation — never
/// invokes git hooks at all (it bypasses the git porcelain entirely via
/// gix's low-level `commit_as`), so an ordinary `rdm phase update`/`rdm task
/// update` can never reach this guard in the first place. The one path that
/// *can* is a genuine subprocess `git commit`/`git merge` against a repo
/// that itself has rdm's hooks installed (see
/// [`GitRepo::git_resolve_conflict`](rdm_store_git::GitRepo::git_resolve_conflict),
/// which completes a merge with a real `git commit --no-edit`) — the guard
/// short-circuits that one-level re-entrancy instead of leaving it
/// completely untested and unbounded.
#[cfg(feature = "git")]
pub fn run_post_commit_hook(root: &Path) -> Result<()> {
    let cwd = std::env::current_dir().context("cannot determine current directory")?;
    let logger = crate::hook_log::HookLogger::new(&cwd);
    let hook = "post-commit";
    let cwd_str = cwd.display().to_string();
    let timeout_secs = resolve_hook_timeout_secs(root).to_string();
    logger.log(
        hook,
        "entry",
        &[
            ("cwd", cwd_str.as_str()),
            ("timeout_secs", timeout_secs.as_str()),
        ],
    );

    if std::env::var(rdm_git::RDM_GIT_SUBPROCESS_ENV).is_ok() {
        logger.log(hook, "skip-reentrant", &[]);
        logger.log(hook, "exit", &[("ok", "true")]);
        return Ok(());
    }

    maybe_test_stall_hook();

    // Only run on the default branch.
    let current_branch = match rdm_git::current_branch_at(&cwd) {
        Ok(b) => b,
        Err(e) => {
            let msg = format!("{e}");
            logger.log(hook, "git-error", &[("error", msg.as_str())]);
            logger.log(hook, "exit", &[("ok", "false")]);
            return Err(e.into());
        }
    };
    let hook_global_config = paths::load_global_config();
    let hook_repo_config = paths::load_repo_config(root).with_global_defaults(&hook_global_config);
    let default_branch = hook_repo_config.default_branch.as_deref().unwrap_or("main");
    match current_branch.as_deref() {
        Some(branch) if branch == default_branch => {}
        other => {
            logger.log(
                hook,
                "skip-branch",
                &[("branch", other.unwrap_or("")), ("default", default_branch)],
            );
            logger.log(hook, "exit", &[("ok", "true")]);
            return Ok(());
        }
    }

    let commit = match rdm_git::head_commit_info_at(&cwd) {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("{e}");
            logger.log(hook, "git-error", &[("error", msg.as_str())]);
            logger.log(hook, "exit", &[("ok", "false")]);
            return Err(e.into());
        }
    };
    let commit = match commit {
        Some(c) => c,
        None => {
            logger.log(hook, "skip-no-head", &[]);
            logger.log(hook, "exit", &[("ok", "true")]);
            return Ok(());
        }
    };

    let directives: Vec<_> = rdm_core::hook::parse_done_directives(&commit.message)
        .into_iter()
        .map(|d| (d, commit.sha.clone()))
        .collect();

    let count = directives.len().to_string();
    logger.log(
        hook,
        "parsed-directives",
        &[("count", count.as_str()), ("sha", commit.sha.as_str())],
    );

    let result = apply_done_directives(root, &directives, &logger, hook);
    logger.log(
        hook,
        "exit",
        &[("ok", if result.is_ok() { "true" } else { "false" })],
    );
    result
}

#[cfg(test)]
mod resolve_body_tests {
    use super::*;

    /// `--body` is authoritative: when it is `Some`, `resolve_body` must
    /// return the content verbatim without touching stdin, regardless of
    /// backticks, em-dashes, or other special characters it contains. This
    /// is not gated behind the `git` feature — `resolve_body` has no git
    /// dependency, so this must hold in every build (including
    /// `--no-default-features`).
    #[test]
    fn resolve_body_returns_special_character_body_verbatim() {
        let special =
            "backtick `code` em-dash — curly “quotes” ellipsis … shell $!\\;|<>*~&& --no-edit";

        let result = resolve_body(Some(special.to_string()), true).unwrap();

        assert_eq!(result, Some(special.to_string()));
    }

    /// A body made up entirely of special characters (no alphanumerics)
    /// should also round-trip verbatim.
    #[test]
    fn resolve_body_returns_only_special_characters_verbatim() {
        let special = "`—“”‘’…$!\\;|<>*~&&";

        let result = resolve_body(Some(special.to_string()), true).unwrap();

        assert_eq!(result, Some(special.to_string()));
    }
}

#[cfg(all(test, feature = "git"))]
mod resolve_review_body_tests {
    use super::*;

    /// `--body` is authoritative for review write commands too, verbatim
    /// through special characters.
    #[test]
    fn resolve_review_body_returns_special_character_body_verbatim() {
        let special =
            "backtick `code` em-dash — curly “quotes” ellipsis … shell $!\\;|<>*~&& --no-edit";

        let result = resolve_review_body(Some(special.to_string()), true).unwrap();

        assert_eq!(result, Some(special.to_string()));
    }

    /// With `--no-edit` and no `--body`, the result is `None` regardless of
    /// whether the test process's own stdin happens to be a TTY — `no_edit`
    /// short-circuits before the TTY check.
    #[test]
    fn resolve_review_body_no_edit_without_body_is_none() {
        let result = resolve_review_body(None, true).unwrap();

        assert_eq!(result, None);
    }
}

#[cfg(all(test, feature = "git"))]
mod hook_timeout_tests {
    use super::*;
    use crate::hook_log::HookLogger;
    use tempfile::TempDir;

    /// Builds a `HookLogger` backed by a fresh temp git repo, so `.log()`
    /// calls actually write somewhere and can be read back.
    fn logger_in_temp_repo() -> (TempDir, HookLogger) {
        let dir = TempDir::new().unwrap();
        gix::init(dir.path()).unwrap();
        let logger = HookLogger::new(dir.path());
        (dir, logger)
    }

    #[test]
    fn run_hook_with_timeout_returns_ok_when_inner_completes_promptly() {
        let (_dir, logger) = logger_in_temp_repo();
        let start = Instant::now();

        let result =
            run_hook_with_timeout(Duration::from_secs(5), &logger, "post-commit", || Ok(()));

        assert!(result.is_ok());
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "should return almost immediately when the inner closure completes promptly"
        );
    }

    #[test]
    fn run_hook_with_timeout_logs_timeout_event_and_returns_when_inner_hangs() {
        let (dir, logger) = logger_in_temp_repo();
        let start = Instant::now();

        let result =
            run_hook_with_timeout(Duration::from_millis(200), &logger, "post-commit", || {
                // Blocks forever: nobody ever sends on this channel.
                let (_tx, rx) = mpsc::channel::<()>();
                let _ = rx.recv();
                Ok(())
            });

        let elapsed = start.elapsed();
        assert!(result.is_ok(), "a timed-out hook must still report Ok");
        assert!(
            elapsed < Duration::from_secs(2),
            "should return shortly after the configured timeout, took {elapsed:?}"
        );

        let log_path = dir.path().join(".git/rdm-hook.log");
        let log = std::fs::read_to_string(&log_path).unwrap();
        assert!(
            log.contains("post-commit timeout"),
            "log missing timeout event: {log}"
        );
    }

    #[test]
    fn run_hook_with_timeout_logs_panicked_event_when_inner_panics() {
        let (dir, logger) = logger_in_temp_repo();
        let start = Instant::now();

        let result = run_hook_with_timeout(Duration::from_secs(5), &logger, "post-commit", || {
            panic!("hook body blew up");
        });

        let elapsed = start.elapsed();
        assert!(result.is_ok(), "a panicked hook must still report Ok");
        assert!(
            elapsed < Duration::from_secs(2),
            "should return as soon as the worker disconnects, took {elapsed:?}"
        );

        let log_path = dir.path().join(".git/rdm-hook.log");
        let log = std::fs::read_to_string(&log_path).unwrap();
        assert!(
            log.contains("post-commit panicked"),
            "log missing panicked event: {log}"
        );
        assert!(
            !log.contains("post-commit timeout"),
            "a panic must not be mislabeled as a timeout: {log}"
        );
    }

    // -- resolve_hook_timeout_secs resolution semantics --

    fn repo_config(toml: &str) -> rdm_core::config::Config {
        rdm_core::config::Config::from_toml(toml).unwrap()
    }

    fn global_config(toml: &str) -> rdm_core::config::GlobalConfig {
        rdm_core::config::GlobalConfig::from_toml(toml).unwrap()
    }

    #[test]
    fn hook_timeout_defaults_when_unset() {
        assert_eq!(
            resolve_hook_timeout_secs_inner(&repo_config(""), &global_config("")),
            DEFAULT_HOOK_TIMEOUT_SECS
        );
    }

    #[test]
    fn hook_timeout_zero_normalizes_to_default_not_unbounded() {
        assert_eq!(
            resolve_hook_timeout_secs_inner(
                &repo_config("hook_timeout_secs = 0"),
                &global_config("")
            ),
            DEFAULT_HOOK_TIMEOUT_SECS
        );
    }

    #[test]
    fn hook_timeout_nonzero_repo_value_wins_over_global() {
        assert_eq!(
            resolve_hook_timeout_secs_inner(
                &repo_config("hook_timeout_secs = 10"),
                &global_config("hook_timeout_secs = 60")
            ),
            10
        );
    }

    #[test]
    fn hook_timeout_global_fills_in_when_repo_unset() {
        assert_eq!(
            resolve_hook_timeout_secs_inner(
                &repo_config(""),
                &global_config("hook_timeout_secs = 60")
            ),
            60
        );
    }

    #[test]
    fn hook_timeout_repo_zero_does_not_defer_to_global() {
        // Pins current behavior: with_global_defaults treats Some(0) as a
        // present repo value, so repo=0 yields the built-in default even
        // when the global config carries a nonzero setting — it does NOT
        // fall through to the global value.
        assert_eq!(
            resolve_hook_timeout_secs_inner(
                &repo_config("hook_timeout_secs = 0"),
                &global_config("hook_timeout_secs = 60")
            ),
            DEFAULT_HOOK_TIMEOUT_SECS
        );
    }

    #[test]
    fn hook_timeout_public_fn_reads_repo_rdm_toml() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("rdm.toml"), "hook_timeout_secs = 7\n").unwrap();
        // Isolate from the host's real global config. SAFETY (env mutation):
        // cargo-nextest runs each test in its own OS process, so this cannot
        // race with other tests.
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", "/dev/null/nonexistent");
        }
        assert_eq!(resolve_hook_timeout_secs(dir.path()), 7);
    }
}

/// Resolve explicit source arguments for thin phase/task/review adapters.
///
/// Loads `item`'s recorded `started_head` from `store` (best-effort: a load
/// failure — e.g. the item vanished between resolution and this call — maps
/// to `None` rather than aborting the source resolution) and threads it into
/// the [`rdm_core::ReviewSourceRequest`], so [`rdm_core::resolve_review_source`]
/// can default an unset `--base` to it instead of the merge-base with
/// `default_branch`. An explicit `--base` still overrides.
#[cfg(feature = "git")]
pub fn resolve_source_args(
    store: &AppStore,
    project: &str,
    args: &crate::cli::ReviewSourceArgs,
    item: &rdm_core::link::ItemRef,
    default_branch: &str,
    root: Option<&Path>,
) -> Result<(GateProbe, rdm_core::ReviewSource)> {
    let cwd = std::env::current_dir()?;
    let repo = match root {
        Some(root) => rdm_git::worktree::discover_distinct_project_repo(&cwd, root)?,
        None => cwd,
    };
    let started_head = match item {
        rdm_core::link::ItemRef::Phase { roadmap, stem } => {
            rdm_core::io::load_phase(store, project, roadmap, stem)
                .ok()
                .and_then(|doc| doc.frontmatter.started_head)
        }
        rdm_core::link::ItemRef::Task { slug } => rdm_core::io::load_task(store, project, slug)
            .ok()
            .and_then(|doc| doc.frontmatter.started_head),
        _ => None,
    };
    let request = rdm_core::ReviewSourceRequest {
        path: args.source.clone(),
        base: args.base.clone(),
        expected_head: args.expected_head.clone(),
        expected_branch: args.expected_branch.clone(),
        default_branch: default_branch.to_string(),
        no_code: args.no_code,
        started_head,
    };
    let probe = GateProbe::new(repo);
    let identity = rdm_core::resolve_review_source(&probe, item, &request)?;
    Ok((probe.with_source(item.clone(), request), identity))
}

/// Resolves and validates a `--start-commit <sha>` value for `phase update`/
/// `task update`, before it is threaded into the write-once `started_head`
/// apply in `rdm-core`.
///
/// Format is checked first, reusing the same shape check already applied to
/// stored change identities
/// ([`rdm_core::model::ReviewTarget::validate_change_identity`]) rather than
/// writing a second regex — `sha` must be a full 40-lowercase-hex-character
/// commit SHA. Existence is then checked against a repository: `source_path`
/// when an explicit `--source <path>` was also given on the same update
/// (matching how other explicit-source flags override auto-resolution in
/// `phase update`/`task update`), otherwise the item's registered roadmap/
/// task worktree — the same "which worktree serves this item" composition
/// (`discover_distinct_project_repo` → `registered_worktree_for`)
/// `rdm-cli/src/commands/verify.rs`'s `item_worktree` uses for `rdm verify
/// run --item`. This checks only that the object exists and is a commit
/// (`git cat-file -e <sha>^{commit}`), not that it is reachable from any
/// particular branch — ancestry is `review source`'s concern at read time,
/// not this write's.
///
/// Unlike the old automatic `started_head` resolution this replaces, there is
/// no best-effort path: `--start-commit` is an explicit instruction, so a
/// resolution failure is always a hard error naming the rejected value (and,
/// for the existence check, the repository path it was checked against)
/// rather than a silent skip.
///
/// # Errors
///
/// Returns an error naming `sha` when it is not a full 40-lowercase-hex-character
/// commit SHA, when no worktree is registered for `item` and no explicit
/// `--source` was given, or when `sha` does not resolve to a commit in the
/// resolved repository.
#[cfg(feature = "git")]
pub fn resolve_start_commit(
    root: &Path,
    item: &rdm_git::worktree::ItemRef,
    sha: &str,
    source_path: Option<&str>,
) -> Result<String> {
    rdm_core::model::ReviewTarget::validate_change_identity(sha, None)
        .map_err(|e| anyhow::anyhow!("invalid --start-commit '{sha}': {e}"))?;
    let repo_path: std::path::PathBuf = match source_path {
        Some(path) => std::path::PathBuf::from(path),
        None => {
            let cwd = std::env::current_dir()?;
            let repo = rdm_git::worktree::discover_distinct_project_repo(&cwd, root)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            rdm_git::worktree::registered_worktree_for(&repo, item)
                .map_err(|e| anyhow::anyhow!("{e}"))?
                .map(|w| w.path)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "cannot validate --start-commit '{sha}': no rdm worktree is registered \
                         for this item — create one with `rdm worktree add`, or pass --source to \
                         validate against an explicit checkout"
                    )
                })?
        }
    };
    let exists = rdm_git::commit_exists_at(&repo_path, sha).map_err(|e| anyhow::anyhow!("{e}"))?;
    if !exists {
        anyhow::bail!(
            "--start-commit '{sha}' does not resolve to a commit in '{}'",
            repo_path.display()
        );
    }
    Ok(sha.to_string())
}

/// Resolves and validates a `--applied-commit <sha>` value for `review
/// update --comment <n>`, before it is threaded into
/// [`rdm_core::ops::reviews::UpdateComment`].
///
/// Unlike `--start-commit` (which requires a full 40-hex SHA already
/// resolved by the caller), `sha` may be an abbreviated SHA, `HEAD`, a
/// branch, or a tag — [`rdm_core::source::SourceRepo::rev_parse`] resolves
/// it, and the **resolved full SHA** is returned, not the operator's literal
/// input. This is a deliberate departure from `--start-commit`'s stricter
/// contract: `applied_commit` is provenance documentation resolved fresh at
/// write time, not a stored identity used for range comparisons, so letting
/// git do the expansion is strictly safer than requiring pre-expansion by
/// hand (the concrete bug this exists to close was exactly that: a
/// hand-expanded short SHA, typed wrong).
///
/// The repository checked depends on the review's target kind:
/// [`rdm_core::model::ReviewTarget::Change`] resolves against the project's
/// configured **source repo** (via [`crate::source_repo::discover_source_repo`],
/// the same discovery `review show`/`review source` already use); every
/// other target kind (`roadmap`, `phase`, `task`, `plan`) resolves against
/// the **plan repo** rooted at `root`.
///
/// This is a hard refusal, not a best-effort default: `--applied-commit` is
/// an explicit instruction minting a public `rdm:src/…@<sha>` permalink, so
/// a SHA that cannot be verified against the correct repository is refused
/// rather than accepted silently or degraded with a warning.
///
/// Two distinct refusal causes are told apart, rather than both collapsing
/// into "does not resolve":
/// [`rdm_git::discover_git_dir`] checks the checked path is actually a git
/// checkout *before* `rev_parse` ever runs, since
/// [`SourceRepo::rev_parse`](rdm_core::source::SourceRepo::rev_parse) maps
/// every non-zero git exit — including "not a git repository" and an
/// ambiguous abbreviated SHA — to the same `Ok(None)`, and reporting that as
/// "no such commit" would send an operator with a correct SHA to go recheck
/// it instead of fixing the repository. A SHA that genuinely fails to
/// resolve in a real git checkout is still reported as "does not resolve",
/// now with a hint that an abbreviated SHA may be ambiguous.
///
/// # Errors
///
/// Returns an error naming the checked path as not a git checkout when it
/// isn't one; an error naming the checked repository (with a hint that an
/// abbreviated SHA may be ambiguous) when `sha` does not resolve to a commit
/// there; or, when the repository itself cannot be discovered at all (no
/// source repo configured/reachable for a `change/<sha>` review), that
/// discovery failure's own actionable text.
#[cfg(feature = "git")]
pub fn resolve_applied_commit(
    store: &AppStore,
    project: &str,
    root: &Path,
    target: &rdm_core::model::ReviewTarget,
    sha: &str,
) -> Result<String> {
    use rdm_core::source::SourceRepo;

    let (repo_path, what) = match target {
        rdm_core::model::ReviewTarget::Change { .. } => {
            let repo = crate::source_repo::discover_source_repo(store, project)?;
            (
                repo.root().to_path_buf(),
                "the project's configured source repo",
            )
        }
        _ => (root.to_path_buf(), "the plan repo"),
    };
    if rdm_git::discover_git_dir(&repo_path).is_err() {
        bail!(
            "--applied-commit cannot be checked: {what} at '{}' is not a git checkout",
            repo_path.display()
        );
    }
    let repo = rdm_git::GitSourceRepo::new(&repo_path);
    let resolved = repo.rev_parse(sha).map_err(|e| anyhow::anyhow!("{e}"))?;
    resolved.ok_or_else(|| {
        anyhow::anyhow!(
            "--applied-commit '{sha}' does not resolve to a commit in {what} at '{}' — if this \
             is an abbreviated SHA, it may be ambiguous; try a longer prefix or the full SHA",
            repo_path.display()
        )
    })
}

// No `#[cfg(not(feature = "git"))]` counterpart: unlike `--start-commit`
// (validated by call sites in `phase.rs`/`task.rs` that exist in every
// build), `--applied-commit` is only reachable through the `review update`
// subcommand, which is itself `#[cfg(feature = "git")]`-gated in its
// entirety (see `Command::Review` in `cli.rs`) — there is no non-git call
// site for a stub to serve.

/// review 2026-09-23-1257-1982, finding
/// `tests-start-commit-explicit-source-branch-untested`: the two CLI
/// integration tests that pass `--source` together with `--start-commit`
/// both point it at the item's own registered worktree — the only path the
/// full `phase update`/`task update` `--source` binding (`explicit_source` /
/// `resolve_source_args`) ever accepts, since it independently requires the
/// path to match a registered checkout. That leaves `resolve_start_commit`'s
/// own `source_path.is_some()` branch — "validate against that path's repo
/// instead of the registry lookup" — proven only by a fixture that cannot
/// tell it apart from the registry lookup it's supposed to override.
///
/// `resolve_start_commit` itself has no such constraint: when `source_path`
/// is `Some`, it never touches the registry (`root`/`item` go unused on that
/// branch), so it can be exercised directly, as a plain function call,
/// against two independent repositories that share no objects at all —
/// something no `phase update`/`task update` invocation can construct.
#[cfg(all(test, feature = "git"))]
mod resolve_start_commit_explicit_source_tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    /// A `git` command in `dir`, isolated from the developer's real
    /// global/system config and with a fixed author/committer identity, so a
    /// bare `git commit` succeeds unattended. Mirrors
    /// `rdm-cli/tests/git_test_support.rs`'s helper — that copy lives under
    /// `tests/`, a separate compilation unit this `src/`-rooted unit test
    /// cannot import.
    fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "test@test.com")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "test@test.com")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?} failed: {out:?}");
    }

    fn rev_parse_head(dir: &Path) -> String {
        let out = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .output()
            .unwrap();
        assert!(out.status.success());
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// Two independent repositories (separate `git init`, no shared
    /// history), each with one commit unique to it.
    fn two_independent_repos() -> (TempDir, String, TempDir, String) {
        let a = TempDir::new().unwrap();
        git(a.path(), &["init", "-b", "main"]);
        std::fs::write(a.path().join("a.txt"), "a\n").unwrap();
        git(a.path(), &["add", "."]);
        git(a.path(), &["commit", "-m", "only in a"]);
        let sha_a = rev_parse_head(a.path());

        let b = TempDir::new().unwrap();
        git(b.path(), &["init", "-b", "main"]);
        std::fs::write(b.path().join("b.txt"), "b\n").unwrap();
        git(b.path(), &["add", "."]);
        git(b.path(), &["commit", "-m", "only in b"]);
        let sha_b = rev_parse_head(b.path());

        (a, sha_a, b, sha_b)
    }

    /// `root`/`item` are irrelevant on the `source_path.is_some()` branch;
    /// any dummy value proves the point, since the branch never reads them.
    fn dummy_item() -> rdm_git::worktree::ItemRef {
        rdm_git::worktree::ItemRef::Task {
            slug: "unused".to_string(),
        }
    }

    #[test]
    fn explicit_source_succeeds_for_a_sha_that_exists_only_in_that_checkout() {
        let (repo_a, sha_a, _repo_b, sha_b) = two_independent_repos();

        // A SHA that exists only in `repo_a` succeeds when `--source` points
        // at `repo_a` — proving resolution used that path's repo, not some
        // other source (a registry that was never consulted, or a shared
        // object database, since none exists here).
        let resolved = resolve_start_commit(
            Path::new("/nonexistent-root"),
            &dummy_item(),
            &sha_a,
            Some(repo_a.path().to_str().unwrap()),
        )
        .unwrap();
        assert_eq!(resolved, sha_a);

        // The reciprocal: a SHA that exists only in `repo_b` does NOT exist
        // in `repo_a`, so the same call with that SHA fails.
        let err = resolve_start_commit(
            Path::new("/nonexistent-root"),
            &dummy_item(),
            &sha_b,
            Some(repo_a.path().to_str().unwrap()),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains(&sha_b),
            "error must name the rejected SHA: {err}"
        );
    }

    #[test]
    fn explicit_source_refuses_a_sha_only_in_a_different_checkout_naming_the_source_path() {
        let (_repo_a, sha_a, repo_b, _sha_b) = two_independent_repos();

        // `sha_a` exists only in `repo_a`. Validating it against `repo_b`
        // (an entirely different, unrelated repository) must fail, and the
        // error must name the `--source` path that was actually checked —
        // `repo_b`, not `repo_a` and not some registered worktree.
        let err = resolve_start_commit(
            Path::new("/nonexistent-root"),
            &dummy_item(),
            &sha_a,
            Some(repo_b.path().to_str().unwrap()),
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains(&sha_a) && msg.contains(repo_b.path().to_str().unwrap()),
            "error must name both the rejected SHA and the --source path it was checked against: {msg}"
        );
    }
}
