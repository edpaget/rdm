//! `bench/scbench/agent/claude-rdm.sh`'s plan-repo setup against the real
//! `rdm` binary: the first call leaves `dispatch.verify` resolving to the
//! committed manifest's `[rdm] verify` and every `RDM_SCBENCH_CONFIG`
//! override applied, committed, and a later call never overwrites them. `claude` is a fake that prints one stream-json line; no
//! Docker, network or SCBench is involved.
//!
//! Every child runs with a cleared environment: a temp `HOME`, an
//! unreachable `XDG_CONFIG_HOME`, `GIT_CONFIG_GLOBAL=/dev/null`,
//! `GIT_CONFIG_NOSYSTEM=1` and a fixed identity, so neither the invoking
//! user's rdm or git config nor an inherited `GIT_*` variable can change a
//! result.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use rdm_devtools::measure::scbench::manifest::Manifest;

/// The repository root (the workspace holding `bench/`).
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

/// The committed manifest's `[rdm] verify`.
fn committed_verify() -> String {
    Manifest::load(&repo_root().join("bench/scbench/manifest.toml"))
        .expect("committed manifest")
        .rdm_table()
        .expect("[rdm]")
        .verify
        .clone()
}

struct Sandbox {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    verify: String,
}

impl Sandbox {
    fn new() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        // One level down, so nothing rdm or git writes beside a directory
        // escapes the TempDir.
        let root = std::fs::canonicalize(tmp.path())
            .expect("canonical tmp")
            .join("s");
        for d in ["bin", "home", "tmp"] {
            std::fs::create_dir_all(root.join(d)).expect("mkdir");
        }
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_rdm"), root.join("bin/rdm"))
            .expect("symlink rdm");
        let claude = root.join("bin/claude");
        std::fs::write(
            &claude,
            "#!/bin/sh\necho '{\"type\":\"result\",\"subtype\":\"success\"}'\nexit 0\n",
        )
        .expect("fake claude");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755))
            .expect("chmod claude");
        Self {
            _tmp: tmp,
            root,
            verify: committed_verify(),
        }
    }

    fn plan(&self) -> PathBuf {
        self.root.join("plan")
    }

    /// A command with the sandbox's cleared environment.
    fn command(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
        let mut cmd = Command::new(program);
        cmd.env_clear()
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", self.root.join("bin").display()),
            )
            .env("HOME", self.root.join("home"))
            .env("TMPDIR", self.root.join("tmp"))
            .env("XDG_CONFIG_HOME", "/dev/null/nonexistent")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_NAME", "rdm-scbench")
            .env("GIT_AUTHOR_EMAIL", "rdm-scbench@localhost")
            .env("GIT_COMMITTER_NAME", "rdm-scbench")
            .env("GIT_COMMITTER_EMAIL", "rdm-scbench@localhost");
        cmd
    }

    /// Runs the wrapper as SlopCodeBench would, once, with no resume.
    fn wrapper(&self) -> Output {
        self.wrapper_with(&[])
    }

    /// [`Sandbox::wrapper`] with `env` added.
    fn wrapper_with(&self, env: &[(&str, &str)]) -> Output {
        self.command("sh")
            .arg(repo_root().join("bench/scbench/agent/claude-rdm.sh"))
            .args(["--print", "--", "PROMPT"])
            .env("RDM_ROOT", self.plan())
            .env("RDM_PROJECT", "bench")
            .env("RDM_ROADMAP", "solution")
            .env("RDM_PLAN_EXPORT_SUBDIR", "projects/-workspace/rdm-plan")
            .env("RDM_RESUME_MAX", "0")
            .env("RDM_SCBENCH_VERIFY", &self.verify)
            .envs(env.iter().copied())
            .output()
            .expect("run claude-rdm.sh")
    }

    /// Runs the real `rdm --root <plan> <args>` and returns its stdout.
    fn rdm(&self, args: &[&str]) -> String {
        let out = self
            .command(env!("CARGO_BIN_EXE_rdm"))
            .arg("--root")
            .arg(self.plan())
            .args(args)
            .output()
            .expect("run rdm");
        assert!(out.status.success(), "rdm {args:?}: {}", text(&out.stderr));
        text(&out.stdout)
    }

    /// `dispatch.verify` as the repo and the `bench` project resolve it.
    fn resolved_verify(&self) -> [String; 2] {
        [
            self.rdm(&["config", "get", "dispatch.verify", "--raw"]),
            self.rdm(&[
                "config",
                "get",
                "dispatch.verify",
                "--raw",
                "--project",
                "bench",
            ]),
        ]
        .map(|s| s.trim_end_matches('\n').to_owned())
    }
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

fn assert_ok(out: &Output) {
    assert!(
        out.status.success(),
        "wrapper failed: {}\nstdout: {}",
        text(&out.stderr),
        text(&out.stdout)
    );
}

#[test]
fn wrapper_setup_resolves_dispatch_verify_to_the_bench_command() {
    let s = Sandbox::new();
    assert!(!s.verify.is_empty());
    let out = s.wrapper();
    assert_ok(&out);
    assert_eq!(
        text(&out.stdout),
        "{\"type\":\"result\",\"subtype\":\"success\"}\n",
        "stdout carries only claude's stream"
    );
    assert_eq!(s.resolved_verify(), [s.verify.clone(), s.verify.clone()]);
    let status = s.rdm(&["status"]);
    assert!(
        status.contains("No uncommitted changes"),
        "the gate is committed: {status}"
    );
}

#[test]
fn a_later_wrapper_call_keeps_the_plan_repos_gate() {
    let s = Sandbox::new();
    assert_ok(&s.wrapper());
    s.rdm(&["config", "set", "dispatch.verify", "other"]);
    assert_ok(&s.wrapper());
    assert_eq!(
        s.resolved_verify(),
        ["other".to_owned(), "other".to_owned()]
    );
}

#[test]
fn wrapper_setup_applies_the_arms_config_overrides_beside_the_gate() {
    let s = Sandbox::new();
    assert_ok(&s.wrapper_with(&[("RDM_SCBENCH_CONFIG", "max_refutations=0")]));
    assert_eq!(
        s.rdm(&[
            "config",
            "get",
            "max_refutations",
            "--raw",
            "--project",
            "bench"
        ])
        .trim_end_matches('\n'),
        "0"
    );
    let shown = s.rdm(&["config", "get", "max_refutations", "--project", "bench"]);
    assert!(shown.contains("repo config"), "{shown}");
    assert_eq!(s.resolved_verify(), [s.verify.clone(), s.verify.clone()]);
    let status = s.rdm(&["status"]);
    assert!(
        status.contains("No uncommitted changes"),
        "the overrides are committed: {status}"
    );
}
