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
use rdm_core::store::Store;

#[cfg(feature = "git")]
use crate::paths;
use crate::{AppStore, ItemKindArg, OutputFormat};

pub mod agent_config;
pub mod backlog;
pub mod config;
pub mod describe;
pub mod index;
pub mod init;
pub mod list;
pub mod model;
pub mod next;
pub mod phase;
pub mod project;
pub mod promote;
pub mod roadmap;
pub mod search;
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
#[cfg(feature = "git")]
pub mod worktree;

#[cfg(feature = "mcp")]
pub mod mcp;

#[cfg(feature = "server")]
pub mod serve;

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

/// Runs a mutating op as a single transaction and prints the staging hint.
///
/// Wraps `f` in [`rdm_core::ops::mutate`], so the entity write, `INDEX.md`
/// regeneration, and the single staged flush happen together — the CLI never
/// has to remember to regenerate the index. `context` labels any failure.
///
/// `--no-index` is honored as an escape hatch: the mutation is still applied
/// and staged, but the index is left stale (the user can rebuild it later
/// with `rdm index`). The trailing `rdm commit` hint is always printed —
/// staging is now the only workflow.
pub fn commit_mutation<T>(
    store: &mut AppStore,
    project: &str,
    no_index: bool,
    context: &str,
    f: impl FnOnce(&mut AppStore) -> rdm_core::error::Result<T>,
) -> Result<T> {
    let out = if no_index {
        let out = f(store).with_context(|| context.to_string())?;
        store.commit().with_context(|| context.to_string())?;
        out
    } else {
        rdm_core::ops::mutate(store, project, f).with_context(|| context.to_string())?
    };
    #[cfg(feature = "git")]
    eprintln!("  (staged — run `rdm commit` to persist)");
    Ok(out)
}

/// Prints a hint about uncommitted changes.
///
/// Called after read-only commands (list, show, search) so the user is aware
/// that the data they see includes uncommitted staged mutations.
#[cfg(feature = "git")]
pub fn maybe_print_uncommitted_hint(store: &AppStore) {
    if let Ok(statuses) = store.git().git_status()
        && !statuses.is_empty()
    {
        eprintln!(
            "\n  ({} uncommitted change(s) — run `rdm status` for details)",
            statuses.len()
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
/// transaction: one `INDEX.md` regeneration and one plan-repo commit cover
/// every directive in `directives_with_sha`, rather than one commit per
/// directive. The resulting commit's message enumerates each successfully
/// applied directive as a `Done: <target> (<sha>)` line, so per-directive
/// provenance survives the collapse into a single commit. That commit is
/// produced via [`rdm_store_git::GitStore`]'s low-level git primitive, which
/// bypasses git porcelain/hooks entirely, so it can never recursively
/// re-trigger this same hook.
///
/// Silently skips directives whose phase or task cannot be found. A single
/// directive's mutation failing does not abort the rest of the batch — every
/// other directive is still applied and committed.
///
/// # Errors
///
/// Returns an error if the store cannot be opened, the project cannot be
/// resolved, or the batch's shared finalize stage (index regeneration or the
/// single commit) fails. In the finalize-failure case, every per-directive
/// outcome has already been logged before the error is returned.
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
    // pathway (`commit_now`).
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
    let mut outcome = rdm_core::ops::mutate_batch(&mut store, &project, steps, move |results| {
        build_batch_commit_message(&message_metas, results)
    });

    // Log every per-directive outcome unconditionally, before inspecting the
    // shared finalize result — this preserves per-directive log fidelity even
    // when the index regen / commit step below fails.
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

    // Surface any flush / index-regen failure before attempting the commit —
    // if the store never flushed cleanly there is nothing safe to commit.
    if let Err(e) = outcome.finalize_result {
        let msg = format!("{e}");
        logger.log(hook, "batch-commit-error", &[("error", msg.as_str())]);
        return Err(e.into());
    }

    // Land exactly one real git commit for the whole batch via the blessed
    // always-commit pathway. `mutate_batch` only flushed to disk (the store's
    // `commit` never touches git); `commit_message` is `Some` iff at least one
    // directive applied, so an all-skipped batch produces no empty commit.
    if let Some(message) = outcome.commit_message.take()
        && let Err(e) = store.commit_now(&message)
    {
        let msg = format!("{e}");
        logger.log(hook, "batch-commit-error", &[("error", msg.as_str())]);
        return Err(e.into());
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
