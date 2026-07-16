use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::BootstrapSubcommand;
use crate::OutputFormat;
use crate::paths;

/// Dispatches the `rdm bootstrap` command: runs the `doctor` subcommand (and
/// exits with its status code) or performs the clone/fast-forward.
///
/// `print_root` and `format` are the raw `--print-root`/`--format` flags from
/// this invocation (`format` is `None` unless a literal `--format` token was
/// given — never the ambient-resolved format every other subcommand
/// consumes). Both are ignored by the `doctor` subcommand.
///
/// # Errors
///
/// Returns an error if no `--plan-repo` is given for the clone path, or if the
/// clone/fast-forward fails. The `doctor` subcommand never returns — it exits
/// the process with its own status code.
#[allow(clippy::too_many_arguments)]
pub fn run_command(
    plan_repo: Option<String>,
    path: Option<PathBuf>,
    branch: Option<String>,
    init: bool,
    token: Option<String>,
    command: Option<BootstrapSubcommand>,
    print_root: bool,
    format: Option<OutputFormat>,
) -> Result<()> {
    match command {
        Some(BootstrapSubcommand::Doctor {
            plan_repo: doc_plan_repo,
            token: doc_token,
        }) => {
            let exit_code = doctor(
                doc_plan_repo.or(plan_repo).as_deref(),
                doc_token.or(token).as_deref(),
            );
            std::process::exit(exit_code);
        }
        None => {
            let url = plan_repo.as_deref().ok_or_else(|| {
                anyhow::anyhow!("--plan-repo is required (or pass a subcommand like `doctor`)")
            })?;
            run(
                url,
                path,
                branch,
                init,
                token.as_deref(),
                print_root,
                format,
            )
        }
    }
}

/// Runs the `rdm bootstrap` command.
///
/// Clones the plan repo at `plan_repo_url` into `target` (resolved from
/// `path_override` or the default XDG data dir), or fast-forwards it if the
/// clone already exists. Validates the result is a plan repo, or runs
/// `rdm init` on an otherwise-empty remote if `init_if_empty` is set.
///
/// When `token` is set and `plan_repo_url` is HTTPS, the token is injected
/// into the URL used for cloning but never printed. SSH URLs are cloned
/// as-is with a warning. Plain HTTP URLs are rejected when a token is
/// present to avoid cleartext leakage.
///
/// `print_root` and `format` control how the result is rendered: with
/// `print_root` set, only the resolved path is printed to stdout (all
/// narration moves to stderr) and `format` is ignored entirely; otherwise
/// `format` selects between the human banner (`None`/`Some(Human)`) and a
/// structured JSON payload (`Some(Json)`) — `Some(Table)`/`Some(Markdown)`
/// are rejected as unsupported for this command.
pub fn run(
    plan_repo_url: &str,
    path_override: Option<PathBuf>,
    branch: Option<String>,
    init_if_empty: bool,
    token: Option<&str>,
    print_root: bool,
    format: Option<OutputFormat>,
) -> Result<()> {
    let mode = resolve_render_mode(print_root, format)?;

    let target = resolve_target_path(path_override)?;
    let state = classify_target(&target)?;

    let clone_url = resolve_clone_url(plan_repo_url, token)?;

    let outcome = match state {
        TargetState::Empty => clone_fresh(
            plan_repo_url,
            &clone_url,
            &target,
            branch.as_deref(),
            init_if_empty,
            mode,
        )?,
        TargetState::PlanRepo => update_existing(&target, mode)?,
        TargetState::NonPlanGitRepo => bail!(
            "target '{}' is a git repo but is not an rdm plan repo (no rdm.toml). \
             Pick a different --path, or delete this directory and re-run.",
            target.display()
        ),
        TargetState::NonEmptyNonGit => bail!(
            "target '{}' exists and is not empty. \
             Pick a different --path, or delete this directory and re-run.",
            target.display()
        ),
    };

    render_outcome(&outcome, mode)
}

/// How `bootstrap`'s result should be rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BootstrapRenderMode {
    /// The pre-existing human banner on stdout (default).
    Human,
    /// Only the resolved path on stdout; narration moves to stderr.
    PrintRoot,
    /// A single pretty-printed JSON object on stdout; narration suppressed.
    Json,
}

/// Resolves the render mode from the raw `--print-root`/`--format` flags.
///
/// `format` is the raw `--format` flag (`cli.format`), not main.rs's
/// ambient-resolved value — `None` means "no `--format` on this invocation"
/// and always falls back to `Human`, regardless of what `RDM_FORMAT` or a
/// repo/global `default_format` say (AC3b). Only an explicit
/// `Some(Table)`/`Some(Markdown)` is rejected (AC3a). `print_root` always
/// wins over `format`, even for `Table`/`Markdown` (AC4).
///
/// # Errors
///
/// Returns an error if `format` is `Some(Table)` or `Some(Markdown)` and
/// `print_root` is `false`.
fn resolve_render_mode(
    print_root: bool,
    format: Option<OutputFormat>,
) -> Result<BootstrapRenderMode> {
    if print_root {
        return Ok(BootstrapRenderMode::PrintRoot);
    }
    match format {
        None | Some(OutputFormat::Human) => Ok(BootstrapRenderMode::Human),
        Some(OutputFormat::Json) => Ok(BootstrapRenderMode::Json),
        Some(fmt @ (OutputFormat::Table | OutputFormat::Markdown)) => bail!(
            "--format {fmt} is not supported for 'bootstrap'; use --format json, or omit --format"
        ),
    }
}

/// Emits a narration line to the sink appropriate for `mode`: stdout in
/// `Human` mode, stderr in `PrintRoot` mode, suppressed entirely in `Json`
/// mode.
fn note(mode: BootstrapRenderMode, msg: &str) {
    match mode {
        BootstrapRenderMode::Json => {}
        BootstrapRenderMode::PrintRoot => eprintln!("{msg}"),
        BootstrapRenderMode::Human => println!("{msg}"),
    }
}

/// The result of a `clone_fresh`/`update_existing` call: the resolved plan
/// repo path plus what happened to get there.
struct BootstrapOutcome {
    path: PathBuf,
    status: BootstrapStatus,
}

/// What kind of operation `clone_fresh`/`update_existing` performed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BootstrapStatus {
    /// A fresh clone of an existing plan repo.
    Cloned,
    /// A fresh clone of an empty remote, initialized via `--init`.
    Initialized,
    /// An existing plan repo with no configured remote — nothing to fetch.
    NoRemote,
    /// An existing plan repo already up to date with its remote.
    UpToDate,
    /// An existing plan repo fast-forwarded by `commits_merged` commit(s).
    FastForwarded {
        /// Number of commits merged by the fast-forward.
        commits_merged: usize,
    },
}

impl BootstrapStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Cloned => "cloned",
            Self::Initialized => "initialized",
            Self::NoRemote => "no-remote",
            Self::UpToDate => "up-to-date",
            Self::FastForwarded { .. } => "fast-forwarded",
        }
    }
}

/// Renders a [`BootstrapOutcome`] according to `mode`.
///
/// # Errors
///
/// Returns an error if JSON serialization fails (should not happen in
/// practice — the payload is a simple struct of strings/numbers).
fn render_outcome(outcome: &BootstrapOutcome, mode: BootstrapRenderMode) -> Result<()> {
    match mode {
        BootstrapRenderMode::Human => print_success(&outcome.path),
        BootstrapRenderMode::PrintRoot => {
            print_success_stderr(&outcome.path);
            println!("{}", outcome.path.display());
        }
        BootstrapRenderMode::Json => {
            let commits_merged = match outcome.status {
                BootstrapStatus::FastForwarded { commits_merged } => Some(commits_merged),
                _ => None,
            };
            let json = serde_json::json!({
                "path": outcome.path.display().to_string(),
                "status": outcome.status.as_str(),
                "commits_merged": commits_merged,
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&json)
                    .context("failed to serialize bootstrap result")?
            );
        }
    }
    Ok(())
}

/// What we found at the target path before acting.
enum TargetState {
    /// Missing or empty — safe to clone.
    Empty,
    /// Existing git working tree with an `rdm.toml`.
    PlanRepo,
    /// Existing git working tree with no `rdm.toml`.
    NonPlanGitRepo,
    /// Exists, non-empty, no `.git` — unsafe to touch.
    NonEmptyNonGit,
}

/// Classifies the current state of the target directory.
fn classify_target(target: &Path) -> Result<TargetState> {
    if !target.exists() {
        return Ok(TargetState::Empty);
    }
    let has_entries = std::fs::read_dir(target)
        .with_context(|| format!("failed to read {}", target.display()))?
        .next()
        .is_some();
    if !has_entries {
        return Ok(TargetState::Empty);
    }
    if target.join(".git").exists() {
        if target.join("rdm.toml").exists() {
            Ok(TargetState::PlanRepo)
        } else {
            Ok(TargetState::NonPlanGitRepo)
        }
    } else {
        Ok(TargetState::NonEmptyNonGit)
    }
}

/// Resolves `--path` or falls back to `$XDG_DATA_HOME/rdm/plan-repo`.
fn resolve_target_path(path_override: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(p) = path_override {
        return Ok(p);
    }
    let data_dir = paths::default_data_dir()
        .context("cannot determine default plan-repo path — set --path or ensure $HOME is set")?;
    Ok(data_dir.join("plan-repo"))
}

/// Kind of URL, used to decide whether/how to inject the token.
enum UrlKind {
    Https,
    Ssh,
    Http,
    Other,
}

fn classify_url(url: &str) -> UrlKind {
    if url.starts_with("https://") {
        UrlKind::Https
    } else if url.starts_with("http://") {
        UrlKind::Http
    } else if url.starts_with("git@") || url.starts_with("ssh://") {
        UrlKind::Ssh
    } else {
        UrlKind::Other
    }
}

/// Injects `token` into an HTTPS URL. Returns `Err` if the URL already has
/// basic-auth embedded (we don't want to clobber user-supplied creds).
///
/// The basic-auth detection relies on RFC 3986 structure: if an `@` appears
/// before the first `/`, it's part of the userinfo component. `@` characters
/// that appear later (in the path or query) are not userinfo and don't count.
fn inject_token(url: &str, token: &str) -> Result<String> {
    let rest = url
        .strip_prefix("https://")
        .expect("inject_token called on non-HTTPS URL — caller must classify first");
    let authority = rest.split('/').next().unwrap_or(rest);
    if authority.contains('@') {
        bail!(
            "refusing to inject --token: URL already contains basic-auth. \
             Remove the embedded credential from the URL or clear --token."
        );
    }
    Ok(format!("https://{token}@{rest}"))
}

/// Resolves the URL that will be passed to `git clone`. Prints a warning
/// for SSH (token not applied) and errors for plain HTTP (cleartext risk).
fn resolve_clone_url(original: &str, token: Option<&str>) -> Result<String> {
    match (classify_url(original), token) {
        (UrlKind::Https, Some(t)) => inject_token(original, t),
        (UrlKind::Ssh, Some(_)) => {
            eprintln!(
                "note: --token/RDM_PLAN_REPO_TOKEN is ignored for SSH URLs; \
                 SSH uses key-based auth."
            );
            Ok(original.to_string())
        }
        (UrlKind::Http, Some(_)) => bail!(
            "refusing to inject --token into a plain http:// URL — tokens must only be \
             sent over HTTPS. Use an https:// URL instead."
        ),
        (UrlKind::Other, Some(_)) => {
            eprintln!("note: --token ignored for non-HTTP(S), non-SSH URL.");
            Ok(original.to_string())
        }
        (_, None) => Ok(original.to_string()),
    }
}

/// Clones into an empty target, then validates or initializes.
///
/// `display_url` is shown to the user; `clone_url` is the one passed to git
/// (may contain an embedded token). Only `display_url` ever touches stdout.
fn clone_fresh(
    display_url: &str,
    clone_url: &str,
    target: &Path,
    branch: Option<&str>,
    init_if_empty: bool,
    mode: BootstrapRenderMode,
) -> Result<BootstrapOutcome> {
    note(
        mode,
        &format!("Cloning {display_url} → {}", target.display()),
    );
    match rdm_store_git::GitStore::clone_remote(clone_url, target, branch) {
        Ok(s) => drop(s),
        Err(e) => {
            // Scrub any occurrence of the (possibly token-bearing) clone URL
            // from the error before surfacing it. Git sometimes echoes the
            // URL back in its messages.
            let msg = e.to_string().replace(clone_url, display_url);
            bail!("failed to clone plan repo: {msg}");
        }
    }

    if target.join("rdm.toml").exists() {
        return Ok(BootstrapOutcome {
            path: target.to_path_buf(),
            status: BootstrapStatus::Cloned,
        });
    }

    if !init_if_empty {
        bail!(
            "cloned repo has no rdm.toml — '{display_url}' is not a plan repo. \
             Re-run with --init to initialize an empty remote as a plan repo."
        );
    }

    let mut store = rdm_store_git::GitStore::new(target).context("failed to open cloned repo")?;
    rdm_core::ops::init::init_with_config(&mut store, rdm_core::config::Config::default())
        .context("failed to initialize cloned repo as a plan repo")?;
    store
        .commit_now("rdm: initialize plan repo via bootstrap --init")
        .context("failed to commit initial plan repo state")?;
    Ok(BootstrapOutcome {
        path: target.to_path_buf(),
        status: BootstrapStatus::Initialized,
    })
}

/// Fast-forwards an existing plan-repo clone.
fn update_existing(target: &Path, mode: BootstrapRenderMode) -> Result<BootstrapOutcome> {
    let mut store =
        rdm_store_git::GitStore::new(target).context("failed to open existing plan repo")?;

    let remotes = store
        .git()
        .git_remote_list()
        .context("failed to list remotes")?;
    let remote_name = remotes
        .iter()
        .find(|r| r.name == "origin")
        .map(|r| r.name.clone())
        .or_else(|| remotes.first().map(|r| r.name.clone()));
    let Some(remote_name) = remote_name else {
        note(
            mode,
            &format!(
                "Plan repo already present at {} (no remote configured, skipping fetch).",
                target.display()
            ),
        );
        return Ok(BootstrapOutcome {
            path: target.to_path_buf(),
            status: BootstrapStatus::NoRemote,
        });
    };

    note(
        mode,
        &format!(
            "Plan repo already at {}; fetching {remote_name}…",
            target.display()
        ),
    );
    let status = match store
        .git_mut()
        .git_pull(&remote_name)
        .context("failed to fast-forward plan repo")?
    {
        rdm_store_git::PullOutcome::Success(result) => {
            if result.changed {
                note(
                    mode,
                    &format!(
                        "Fast-forwarded {} commit(s) from {}/{}.",
                        result.commits_merged, result.remote, result.branch
                    ),
                );
                BootstrapStatus::FastForwarded {
                    commits_merged: result.commits_merged,
                }
            } else {
                note(mode, "Already up to date.");
                BootstrapStatus::UpToDate
            }
        }
        rdm_store_git::PullOutcome::Conflict(conflict) => {
            bail!(
                "merge conflict while updating plan repo ({} file(s) need resolution). \
                 Resolve conflicts in {} manually before re-running.",
                conflict.conflicted_files.len(),
                target.display()
            );
        }
    };
    Ok(BootstrapOutcome {
        path: target.to_path_buf(),
        status,
    })
}

/// Prints the post-bootstrap success banner to stdout (`Human` mode only).
fn print_success(path: &Path) {
    println!();
    println!("Plan repo ready at {}", path.display());
    println!("  export RDM_ROOT=\"{}\"", path.display());
}

/// Prints the post-bootstrap success banner to stderr (`PrintRoot` mode).
fn print_success_stderr(path: &Path) {
    eprintln!();
    eprintln!("Plan repo ready at {}", path.display());
    eprintln!("  export RDM_ROOT=\"{}\"", path.display());
}

// ============================================================================
// Doctor
// ============================================================================

/// Diagnoses a sandbox's readiness to bootstrap a plan repo.
///
/// Runs five independent checks (rdm on PATH, plan-repo root configured,
/// URL present, token present if needed, token scopes via the GitHub API)
/// and prints each result. Returns a process exit code: `0` when every
/// critical check passes, `1` otherwise. Does not return `Result`; all
/// failures are surfaced via stdout + the exit code so the caller can
/// shell-pipe the output.
pub fn doctor(plan_repo: Option<&str>, token: Option<&str>) -> i32 {
    let mut critical_failures = 0u32;

    println!("rdm bootstrap doctor");
    println!();

    // 1. rdm binary on PATH.
    match which_on_path("rdm") {
        Some(path) => {
            println!("  ✓ rdm on PATH: {}", path.display());
            if let Ok(exe) = std::env::current_exe()
                && exe != path
            {
                println!(
                    "    warn: the rdm on PATH differs from this invocation ({}).",
                    exe.display()
                );
            }
        }
        None => {
            println!("  ✗ rdm not found on PATH");
            println!(
                "    fix: install via install.sh or ensure $HOME/.local/bin (or \
                 $CARGO_HOME/bin) is on PATH."
            );
            critical_failures += 1;
        }
    }

    // 2. Plan-repo root reachable.
    match detect_plan_repo_root() {
        Ok(Some(root)) => println!("  ✓ plan repo configured at {}", root.display()),
        Ok(None) => {
            println!("  ✗ no plan repo configured");
            println!(
                "    fix: run `rdm bootstrap --plan-repo <url>` or set `root` in ~/.config/rdm/config.toml."
            );
            critical_failures += 1;
        }
        Err(e) => {
            println!("  ! could not check plan repo root: {e}");
        }
    }

    // 3. Plan repo URL present.
    let Some(url) = plan_repo else {
        println!("  ✗ no plan repo URL");
        println!("    fix: set RDM_PLAN_REPO or pass --plan-repo to `rdm bootstrap doctor`.");
        critical_failures += 1;
        return exit_code(critical_failures);
    };
    println!("  ✓ plan repo URL: {url}");

    // 4. Token presence (conditional on URL kind).
    let url_kind = classify_url(url);
    let token_expected = matches!(url_kind, UrlKind::Https)
        && parse_owner_repo(url).is_some_and(|(host, _, _)| host == "github.com");
    match (token, &url_kind) {
        (Some(_), UrlKind::Https) => println!("  ✓ token present"),
        (Some(_), UrlKind::Ssh) => {
            println!("  ! token set but URL uses SSH; token will be ignored.");
        }
        (Some(_), _) => {
            println!("  ! token set but URL is neither HTTPS nor SSH; token will be ignored.");
        }
        (None, UrlKind::Ssh) => println!("  ✓ SSH URL — no token needed"),
        (None, _) => {
            if token_expected {
                println!("  ✗ no token");
                println!(
                    "    fix: set RDM_PLAN_REPO_TOKEN (fine-grained GitHub PAT with Contents: \
                     read/write on the plan repo)."
                );
                critical_failures += 1;
            } else {
                println!("  - no token (URL doesn't look like it needs one)");
            }
        }
    }

    // 5. Token scopes (best-effort).
    if let (Some(t), UrlKind::Https) = (token, &url_kind)
        && let Some((host, owner, repo)) = parse_owner_repo(url)
        && host == "github.com"
    {
        match check_github_repo_access(&owner, &repo, t) {
            GitHubCheck::Ok => println!("  ✓ token can access {owner}/{repo}"),
            GitHubCheck::Unauthorized => {
                println!("  ✗ token rejected (401)");
                println!("    fix: regenerate the token — it may be revoked or expired.");
                critical_failures += 1;
            }
            GitHubCheck::Forbidden => {
                println!("  ✗ token lacks permission (403)");
                println!("    fix: regenerate with Contents: read and write on this repo.");
                critical_failures += 1;
            }
            GitHubCheck::NotFound => {
                println!("  ✗ repo not visible to this token (404)");
                println!("    fix: add this repo to the token's Repository Access list.");
                critical_failures += 1;
            }
            GitHubCheck::Inconclusive(reason) => {
                println!("  ! could not verify token scopes: {reason}");
            }
        }
    }

    println!();
    if critical_failures == 0 {
        println!("All checks passed.");
    } else {
        println!("{critical_failures} critical check(s) failed.");
    }

    exit_code(critical_failures)
}

fn exit_code(critical_failures: u32) -> i32 {
    if critical_failures == 0 { 0 } else { 1 }
}

/// Walk `$PATH` looking for an executable named `name`.
fn which_on_path(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Resolves the configured plan-repo root, if any, and confirms it has an
/// `rdm.toml`. Returns `Ok(None)` if nothing's configured.
fn detect_plan_repo_root() -> Result<Option<PathBuf>> {
    let global = paths::load_global_config();
    let root = match paths::resolve_root(None, &global) {
        Ok(r) => paths::expand_root(r)?,
        Err(_) => return Ok(None),
    };
    if root.join("rdm.toml").exists() {
        Ok(Some(root))
    } else {
        Ok(None)
    }
}

/// Parses `(host, owner, repo)` from a GitHub-style URL. Supports:
/// - `https://github.com/owner/repo(.git)?`
/// - `https://<token>@github.com/owner/repo(.git)?`
/// - `git@github.com:owner/repo(.git)?`
/// - `ssh://git@github.com/owner/repo(.git)?`
fn parse_owner_repo(url: &str) -> Option<(String, String, String)> {
    let (host, path) = if let Some(rest) = url.strip_prefix("https://") {
        // Strip optional user@
        let rest = rest.split_once('@').map(|(_, r)| r).unwrap_or(rest);
        rest.split_once('/')?
    } else if let Some(rest) = url.strip_prefix("git@") {
        // git@host:owner/repo
        let (host, path) = rest.split_once(':')?;
        (host, path)
    } else if let Some(rest) = url.strip_prefix("ssh://") {
        let rest = rest.split_once('@').map(|(_, r)| r).unwrap_or(rest);
        rest.split_once('/')?
    } else {
        return None;
    };
    let path = path.trim_end_matches(".git").trim_end_matches('/');
    let (owner, repo) = path.split_once('/')?;
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((host.to_string(), owner.to_string(), repo.to_string()))
}

/// Result of querying GitHub for repo access.
enum GitHubCheck {
    Ok,
    Unauthorized,
    Forbidden,
    NotFound,
    Inconclusive(String),
}

/// Shells out to `curl` to call `GET /repos/{owner}/{repo}`. Returns
/// `Inconclusive` when curl is unavailable or the call fails in a way that
/// doesn't correspond to a clear HTTP status (network errors, etc.).
fn check_github_repo_access(owner: &str, repo: &str, token: &str) -> GitHubCheck {
    let url = format!("https://api.github.com/repos/{owner}/{repo}");
    let output = match std::process::Command::new("curl")
        .args([
            "-sS",
            "-o",
            "/dev/null",
            "-w",
            "%{http_code}",
            "--proto",
            "=https",
            "--tlsv1.2",
            "-H",
            "Accept: application/vnd.github+json",
            "-H",
            "User-Agent: rdm-bootstrap-doctor",
        ])
        .arg("-H")
        .arg(format!("Authorization: Bearer {token}"))
        .arg(url)
        .output()
    {
        Ok(o) => o,
        Err(e) => return GitHubCheck::Inconclusive(format!("curl not runnable: {e}")),
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return GitHubCheck::Inconclusive(format!("curl failed: {}", stderr.trim()));
    }
    let status_str = String::from_utf8_lossy(&output.stdout);
    let status_str = status_str.trim();
    interpret_github_status(status_str)
}

fn interpret_github_status(code: &str) -> GitHubCheck {
    match code {
        "200" => GitHubCheck::Ok,
        "401" => GitHubCheck::Unauthorized,
        "403" => GitHubCheck::Forbidden,
        "404" => GitHubCheck::NotFound,
        "" => GitHubCheck::Inconclusive("no HTTP status from curl".to_string()),
        other => GitHubCheck::Inconclusive(format!("unexpected HTTP status {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inject_token_basic_https() {
        let got = inject_token("https://github.com/acme/plan.git", "SECRET").unwrap();
        assert_eq!(got, "https://SECRET@github.com/acme/plan.git");
    }

    #[test]
    fn inject_token_https_with_port() {
        let got = inject_token("https://git.example.com:8443/a/b.git", "T").unwrap();
        assert_eq!(got, "https://T@git.example.com:8443/a/b.git");
    }

    #[test]
    fn inject_token_refuses_existing_basic_auth() {
        let err = inject_token("https://user:pw@github.com/acme/plan.git", "T")
            .expect_err("should refuse existing creds");
        assert!(err.to_string().contains("basic-auth"), "got: {err}");
    }

    #[test]
    fn inject_token_refuses_basic_auth_with_port() {
        let err = inject_token("https://user:pw@git.example.com:8443/a/b.git", "T")
            .expect_err("should refuse existing creds even with port");
        assert!(err.to_string().contains("basic-auth"), "got: {err}");
    }

    #[test]
    fn inject_token_ignores_at_in_path() {
        // '@' in the path is not userinfo, so injection should succeed.
        let got = inject_token("https://github.com/acme/plan.git?ref=v@1", "T").unwrap();
        assert_eq!(got, "https://T@github.com/acme/plan.git?ref=v@1");
    }

    #[test]
    fn resolve_clone_url_http_with_token_errors() {
        let err = resolve_clone_url("http://example.com/foo.git", Some("T"))
            .expect_err("plain http should be rejected");
        assert!(err.to_string().contains("http://"), "got: {err}");
    }

    #[test]
    fn resolve_clone_url_ssh_ignores_token() {
        let got = resolve_clone_url("git@github.com:acme/plan.git", Some("T")).unwrap();
        assert_eq!(got, "git@github.com:acme/plan.git");
    }

    #[test]
    fn parse_owner_repo_https() {
        assert_eq!(
            parse_owner_repo("https://github.com/foo/bar.git"),
            Some(("github.com".into(), "foo".into(), "bar".into()))
        );
    }

    #[test]
    fn parse_owner_repo_https_with_token() {
        assert_eq!(
            parse_owner_repo("https://TOKEN@github.com/foo/bar"),
            Some(("github.com".into(), "foo".into(), "bar".into()))
        );
    }

    #[test]
    fn parse_owner_repo_ssh_scp_style() {
        assert_eq!(
            parse_owner_repo("git@github.com:foo/bar.git"),
            Some(("github.com".into(), "foo".into(), "bar".into()))
        );
    }

    #[test]
    fn parse_owner_repo_ssh_url_style() {
        assert_eq!(
            parse_owner_repo("ssh://git@github.com/foo/bar"),
            Some(("github.com".into(), "foo".into(), "bar".into()))
        );
    }

    #[test]
    fn parse_owner_repo_missing_repo_returns_none() {
        assert_eq!(parse_owner_repo("https://github.com/foo"), None);
    }

    #[test]
    fn interpret_github_status_known_codes() {
        assert!(matches!(interpret_github_status("200"), GitHubCheck::Ok));
        assert!(matches!(
            interpret_github_status("401"),
            GitHubCheck::Unauthorized
        ));
        assert!(matches!(
            interpret_github_status("403"),
            GitHubCheck::Forbidden
        ));
        assert!(matches!(
            interpret_github_status("404"),
            GitHubCheck::NotFound
        ));
        assert!(matches!(
            interpret_github_status("500"),
            GitHubCheck::Inconclusive(_)
        ));
    }

    // ------------------------------------------------------------------
    // resolve_render_mode / BootstrapStatus
    // ------------------------------------------------------------------

    #[test]
    fn resolve_render_mode_print_root_wins_over_any_format() {
        for format in [
            None,
            Some(crate::OutputFormat::Human),
            Some(crate::OutputFormat::Json),
            Some(crate::OutputFormat::Table),
            Some(crate::OutputFormat::Markdown),
        ] {
            let mode = resolve_render_mode(true, format).unwrap();
            assert_eq!(
                mode,
                BootstrapRenderMode::PrintRoot,
                "expected PrintRoot for format {format:?}"
            );
        }
    }

    #[test]
    fn resolve_render_mode_none_defaults_to_human() {
        let mode = resolve_render_mode(false, None).unwrap();
        assert_eq!(mode, BootstrapRenderMode::Human);
    }

    #[test]
    fn resolve_render_mode_human_default() {
        let mode = resolve_render_mode(false, Some(crate::OutputFormat::Human)).unwrap();
        assert_eq!(mode, BootstrapRenderMode::Human);
    }

    #[test]
    fn resolve_render_mode_json() {
        let mode = resolve_render_mode(false, Some(crate::OutputFormat::Json)).unwrap();
        assert_eq!(mode, BootstrapRenderMode::Json);
    }

    #[test]
    fn resolve_render_mode_table_and_markdown_rejected() {
        let err = resolve_render_mode(false, Some(crate::OutputFormat::Table))
            .expect_err("table should be rejected");
        assert!(
            err.to_string().contains("not supported for 'bootstrap'"),
            "got: {err}"
        );

        let err = resolve_render_mode(false, Some(crate::OutputFormat::Markdown))
            .expect_err("markdown should be rejected");
        assert!(
            err.to_string().contains("not supported for 'bootstrap'"),
            "got: {err}"
        );
    }

    #[test]
    fn bootstrap_status_as_str_variants() {
        assert_eq!(BootstrapStatus::Cloned.as_str(), "cloned");
        assert_eq!(BootstrapStatus::Initialized.as_str(), "initialized");
        assert_eq!(BootstrapStatus::NoRemote.as_str(), "no-remote");
        assert_eq!(BootstrapStatus::UpToDate.as_str(), "up-to-date");
        assert_eq!(
            BootstrapStatus::FastForwarded { commits_merged: 3 }.as_str(),
            "fast-forwarded"
        );
    }
}
