//! Single git-subprocess spawner shared by every git call site in the crate.
//!
//! All process spawning routes through [`git_command`], which builds the
//! `git` command, scrubs the ambient `GIT_DIR`/`GIT_WORK_TREE`/
//! `GIT_INDEX_FILE` environment variables once, and hardens every child to be
//! strictly non-interactive. The two error domains in this crate
//! ([`rdm_core::error::Error`](rdm_core::error::Error) for the store and
//! [`WorktreeError`](crate::worktree::WorktreeError) for worktrees) each map
//! the raw [`std::io::Result`] themselves, so behavior is identical to the
//! previously duplicated spawners.

use std::path::Path;
use std::process::Output;

use crate::RDM_GIT_SUBPROCESS_ENV;

/// Spawns `git` with `args`, optionally in `cwd`, returning the raw output.
///
/// The ambient `GIT_DIR`, `GIT_WORK_TREE`, and `GIT_INDEX_FILE` environment
/// variables are removed so the command operates on the intended repository
/// rather than one inherited from the caller's environment.
///
/// `GIT_EXTERNAL_DIFF` and `GIT_CONFIG_PARAMETERS` are removed for a related
/// but distinct reason: they are the two configuration layers a command-line
/// flag cannot reach. `rdm-git`'s diff reads pass `--no-ext-diff`
/// `--no-textconv`, which override `diff.external` and a `textconv` filter
/// wherever they were *configured* — but `GIT_EXTERNAL_DIFF` is an
/// environment variable git consults directly, and
/// `GIT_CONFIG_PARAMETERS` is the serialized `-c key=value` list an outer
/// `git` (a hook, an alias, a wrapper script) smuggles into every child,
/// which can reintroduce `diff.external` underneath the flag. Removing both
/// makes rdm's git reads independent of the invoking process's environment,
/// not merely of the on-disk config files. Nothing in this workspace passes
/// `git -c`, so nothing relies on `GIT_CONFIG_PARAMETERS` being inherited.
///
/// Every rdm-invoked git subprocess is non-interactive by construction; this
/// must never depend on the caller's terminal state, `core.editor`,
/// `GIT_EDITOR`/`VISUAL`/`EDITOR`, or credential-helper configuration. Without
/// this, a `git merge` that needs a real merge commit message can invoke an
/// interactive editor (a terminal editor opening `/dev/tty` directly, or a
/// GUI editor that ignores stdin entirely), and a `git fetch`/`push` against
/// an authentication-required remote can block on a credential or host-key
/// prompt — both indefinite hangs with no output, requiring a manual
/// `SIGTERM`. Accordingly, every child spawned here has:
///
/// - `GIT_EDITOR=true` / `GIT_SEQUENCE_EDITOR=true` — `true` is a no-op binary
///   present on every POSIX system, so any git operation that would otherwise
///   launch an editor accepts the default (usually auto-generated) message
///   instead of blocking.
/// - `GIT_TERMINAL_PROMPT=0` — disables git's own terminal credential/host-key
///   prompts, causing them to fail fast with an error instead of blocking.
/// - `GIT_ASKPASS=true` — belt-and-suspenders: if some code path still invokes
///   an askpass helper, it returns empty input immediately rather than
///   blocking. `PATH` is left untouched so `true` resolves normally.
///
/// It is also tagged with [`RDM_GIT_SUBPROCESS_ENV`] `=1` so that anything
/// downstream — notably git's own hook runner, should this subprocess be a
/// real `git commit`/`git merge` against a repo with rdm's hooks installed —
/// can tell it was spawned by rdm itself rather than by a human or agent. See
/// the constant's doc comment for the full re-entrancy rationale.
pub(crate) fn git_command(cwd: Option<&Path>, args: &[&str]) -> std::io::Result<Output> {
    let mut cmd = std::process::Command::new("git");
    cmd.args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_EXTERNAL_DIFF")
        .env_remove("GIT_CONFIG_PARAMETERS")
        .env("GIT_EDITOR", "true")
        .env("GIT_SEQUENCE_EDITOR", "true")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "true")
        .env(RDM_GIT_SUBPROCESS_ENV, "1");
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    cmd.output()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tempfile::TempDir;

    /// Writes a fake `git` executable that ignores its arguments and dumps
    /// its own environment (one `KEY=VALUE` line per variable) to stdout,
    /// then exits 0. Returns the directory containing it, which callers
    /// prepend to `PATH` so `Command::new("git")` resolves to this shim
    /// instead of the real `git` binary.
    fn fake_git_env_echo_dir() -> TempDir {
        let dir = TempDir::new().unwrap();
        let script = dir.path().join("git");
        std::fs::write(&script, "#!/bin/sh\nenv\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        dir
    }

    /// Runs `git_command` with `PATH` overridden to the fake env-echoing
    /// `git` shim and parses its stdout into a `KEY -> VALUE` map. Restores
    /// `PATH` before returning.
    fn spawn_and_parse_env() -> HashMap<String, String> {
        let fake_bin = fake_git_env_echo_dir();
        let original_path = std::env::var("PATH").unwrap_or_default();
        let new_path = format!("{}:{original_path}", fake_bin.path().display());
        // SAFETY: this test runs in its own process (cargo-nextest isolates
        // each test into a separate OS process), so mutating the
        // process-wide PATH here cannot race with other tests.
        unsafe {
            std::env::set_var("PATH", &new_path);
        }
        let output = git_command(None, &["status"]).expect("fake git should spawn");
        // SAFETY: same invariant as above — nextest process isolation means
        // restoring the process-wide PATH cannot race with other tests.
        unsafe {
            std::env::set_var("PATH", original_path);
        }
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout
            .lines()
            .filter_map(|line| line.split_once('='))
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn git_command_sets_non_interactive_editor_env() {
        let env = spawn_and_parse_env();
        assert_eq!(env.get("GIT_EDITOR").map(String::as_str), Some("true"));
        assert_eq!(
            env.get("GIT_SEQUENCE_EDITOR").map(String::as_str),
            Some("true")
        );
        // Same invocation also proves the re-entrancy marker (AC4) is tagged
        // on every spawned child.
        assert_eq!(
            env.get(RDM_GIT_SUBPROCESS_ENV).map(String::as_str),
            Some("1")
        );
    }

    /// `--no-ext-diff` / `--no-textconv` override the two config *files*.
    /// `GIT_EXTERNAL_DIFF` is consulted straight from the environment and
    /// `GIT_CONFIG_PARAMETERS` re-injects arbitrary `-c` settings into every
    /// child, so neither is reachable by a flag — only by scrubbing the
    /// child's environment, which is what this pins.
    #[test]
    fn git_command_scrubs_the_config_layers_a_flag_cannot_reach() {
        let previous_driver = std::env::var_os("GIT_EXTERNAL_DIFF");
        let previous_params = std::env::var_os("GIT_CONFIG_PARAMETERS");
        // SAFETY: cargo-nextest isolates each test into its own OS process
        // (the same invariant `spawn_and_parse_env` relies on for `PATH`), so
        // mutating these process-wide variables cannot race with any other
        // test.
        unsafe {
            std::env::set_var("GIT_EXTERNAL_DIFF", "/bin/false");
            std::env::set_var("GIT_CONFIG_PARAMETERS", "'diff.external=/bin/false'");
        }
        let env = spawn_and_parse_env();
        unsafe {
            match &previous_driver {
                Some(v) => std::env::set_var("GIT_EXTERNAL_DIFF", v),
                None => std::env::remove_var("GIT_EXTERNAL_DIFF"),
            }
            match &previous_params {
                Some(v) => std::env::set_var("GIT_CONFIG_PARAMETERS", v),
                None => std::env::remove_var("GIT_CONFIG_PARAMETERS"),
            }
        }
        assert!(
            !env.contains_key("GIT_EXTERNAL_DIFF"),
            "GIT_EXTERNAL_DIFF reached the child — an ambient external diff driver would run"
        );
        assert!(
            !env.contains_key("GIT_CONFIG_PARAMETERS"),
            "GIT_CONFIG_PARAMETERS reached the child — an outer `git -c` could reintroduce diff.external"
        );
    }

    #[test]
    fn git_command_sets_terminal_prompt_and_askpass_env() {
        let env = spawn_and_parse_env();
        assert_eq!(
            env.get("GIT_TERMINAL_PROMPT").map(String::as_str),
            Some("0")
        );
        assert_eq!(env.get("GIT_ASKPASS").map(String::as_str), Some("true"));
    }
}
