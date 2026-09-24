//! Real-binary fixtures for the workflow component tests: a per-test plan
//! repo seeded through `CARGO_BIN_EXE_rdm`, a source repo with registered
//! worktrees, a shell runner for the commands a workflow hands back, and the
//! isolated user context ([`Sandbox`]) every distribution and CLI-loop
//! process runs in.
//!
//! Included with `#[path = "../common/plan_fixture.rs"] mod plan_fixture;` by
//! the `workflow_review`, `workflow_passes`, `distribution`, `cli_loops` and
//! `golden_json` binaries, each of which also
//! includes `common/workflow_support.rs` and `git_test_support.rs` (this file
//! names both through `crate::`, which is why it is under `tests/common/`
//! and not a test target of its own).
//!
//! # Hermetic environment
//!
//! Every `rdm` and shell process runs with no inherited `RDM_*` variable,
//! global/system git config pointed at `/dev/null`, an unreachable
//! `XDG_CONFIG_HOME` (so no real rdm global config is read), a fixed review
//! author and git identity, and `RDM_ROOT` naming this repo. The commands a
//! workflow emits carry no `--root`, so `RDM_ROOT` is how they find the plan.
//!
//! # Executing returned commands (the decoy fixture)
//!
//! [`Run::bare`] runs a script with `PATH=/usr/bin:/bin`, so no ambient `rdm`
//! can resolve: a command that dropped the caller's `rdmBin` fails to run.
//! A plan repo seeded by [`PlanRepo::with_decoy`] has the default project
//! [`DECOY`], so a command that dropped `--project` resolves the decoy and
//! the test's read-back of the real project fails. The fixture is its own
//! negative control; no command text is inspected. [`PlanRepo::rdm_on_path`]
//! provides a `bin/` directory holding an `rdm` symlink to the binary under
//! test, for the arm where the caller omits `rdmBin` and a plain `rdm` must
//! resolve on `PATH`.
//!
//! # Source repos
//!
//! [`SourceRepo`] roots its git repo at `<TempDir>/src`, so the
//! `src__worktrees/` directory `rdm worktree add` creates as a sibling stays
//! inside the plan repo's `TempDir` and is removed with it.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Duration;

use rdm_devtools::process::{ProcessSpec, Session, SessionError};
use serde_json::{Value, json};
use tempfile::TempDir;

use crate::git_test_support::git;
use crate::workflow_support::{Failure, infra};

/// The default project of a [`PlanRepo::with_decoy`] repo: a command that
/// drops its `--project` flag lands here instead of the target project.
pub const DECOY: &str = "decoy";

/// The review author every process runs as.
pub const REVIEW_AUTHOR: &str = "workflow-test";

/// `PATH` for [`Run::bare`]: system directories only.
pub const BARE_PATH: &str = "/usr/bin:/bin";

/// The binary under test.
pub fn rdm_bin() -> &'static str {
    env!("CARGO_BIN_EXE_rdm")
}

/// Applies the hermetic environment every `rdm` (and emitted-command) process
/// runs under: no inherited `RDM_*`, no real global/system git or rdm config,
/// a fixed review author and git identity.
pub fn hermetic(cmd: &mut Command, root: &Path, session: &str) {
    for key in hermetic_removals() {
        cmd.env_remove(key);
    }
    for (key, value) in hermetic_vars(root, session) {
        cmd.env(key, value);
    }
}

/// The variables [`hermetic`] removes: every inherited `RDM_*`, and the git
/// variables that would redirect a repo.
pub fn hermetic_removals() -> Vec<OsString> {
    let mut keys: Vec<OsString> = std::env::vars_os()
        .map(|(k, _)| k)
        .filter(|k| k.to_string_lossy().starts_with("RDM_"))
        .collect();
    keys.extend(["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE"].map(OsString::from));
    keys
}

/// The variables [`hermetic`] sets.
pub fn hermetic_vars(root: &Path, session: &str) -> Vec<(&'static str, OsString)> {
    vec![
        ("RDM_ROOT", root.into()),
        ("RDM_SESSION", session.into()),
        ("RDM_REVIEW_AUTHOR", REVIEW_AUTHOR.into()),
        ("XDG_CONFIG_HOME", "/dev/null/nonexistent".into()),
        ("GIT_CONFIG_GLOBAL", "/dev/null".into()),
        ("GIT_CONFIG_SYSTEM", "/dev/null".into()),
        ("GIT_AUTHOR_NAME", "test".into()),
        ("GIT_AUTHOR_EMAIL", "test@test.com".into()),
        ("GIT_COMMITTER_NAME", "test".into()),
        ("GIT_COMMITTER_EMAIL", "test@test.com".into()),
    ]
}

/// The fixed git identity sandboxed processes commit as.
pub const SANDBOX_GIT_NAME: &str = "fixture-bot";
/// The fixed git email sandboxed processes commit as.
pub const SANDBOX_GIT_EMAIL: &str = "fixture@example.invalid";

/// An isolated user context for a process a test spawns: its own `HOME` and
/// `XDG_{CONFIG,DATA,STATE}_HOME` under the test's temp directory, no
/// inherited `RDM_*`, `CODEX_HOME`, `CLAUDE_CONFIG_DIR` or
/// `CLAUDE_CODE_SESSION_ID`, global/system git config pointed at
/// `/dev/null`, and a fixed git identity. Everything goes on the child
/// [`Command`]; the test process's own environment is never touched.
#[derive(Clone, Debug)]
pub struct Sandbox {
    /// `HOME`.
    pub home: PathBuf,
    /// `XDG_CONFIG_HOME`.
    pub xdg_config: PathBuf,
    /// `XDG_DATA_HOME`.
    pub xdg_data: PathBuf,
    /// `XDG_STATE_HOME`.
    pub xdg_state: PathBuf,
}

impl Sandbox {
    /// Creates `home`, `xdg-config`, `xdg-data` and `xdg-state` under
    /// `parent` (a directory the test owns).
    pub fn new(parent: &Path) -> Result<Self, Failure> {
        let sandbox = Self {
            home: parent.join("home"),
            xdg_config: parent.join("xdg-config"),
            xdg_data: parent.join("xdg-data"),
            xdg_state: parent.join("xdg-state"),
        };
        for dir in [
            &sandbox.home,
            &sandbox.xdg_config,
            &sandbox.xdg_data,
            &sandbox.xdg_state,
        ] {
            std::fs::create_dir_all(dir).map_err(infra)?;
        }
        Ok(sandbox)
    }

    /// The variables [`Sandbox::apply`] removes.
    pub fn removals() -> Vec<OsString> {
        let mut keys = hermetic_removals();
        keys.extend(
            ["CODEX_HOME", "CLAUDE_CONFIG_DIR", "CLAUDE_CODE_SESSION_ID"].map(OsString::from),
        );
        keys
    }

    /// The variables [`Sandbox::apply`] sets.
    pub fn vars(&self) -> Vec<(&'static str, OsString)> {
        vec![
            ("HOME", self.home.clone().into()),
            ("XDG_CONFIG_HOME", self.xdg_config.clone().into()),
            ("XDG_DATA_HOME", self.xdg_data.clone().into()),
            ("XDG_STATE_HOME", self.xdg_state.clone().into()),
            ("GIT_CONFIG_GLOBAL", "/dev/null".into()),
            ("GIT_CONFIG_SYSTEM", "/dev/null".into()),
            ("GIT_AUTHOR_NAME", SANDBOX_GIT_NAME.into()),
            ("GIT_AUTHOR_EMAIL", SANDBOX_GIT_EMAIL.into()),
            ("GIT_COMMITTER_NAME", SANDBOX_GIT_NAME.into()),
            ("GIT_COMMITTER_EMAIL", SANDBOX_GIT_EMAIL.into()),
        ]
    }

    /// Applies the sandbox to `cmd`.
    pub fn apply<'c>(&self, cmd: &'c mut Command) -> &'c mut Command {
        for key in Self::removals() {
            cmd.env_remove(key);
        }
        for (key, value) in self.vars() {
            cmd.env(key, value);
        }
        cmd
    }

    /// `program` under the sandbox.
    pub fn command(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
        let mut cmd = Command::new(program);
        self.apply(&mut cmd);
        cmd
    }

    /// The binary under test under the sandbox.
    pub fn rdm(&self) -> Command {
        self.command(rdm_bin())
    }

    /// `git` under the sandbox in `dir`, requiring success; returns stdout.
    pub fn git(&self, dir: &Path, args: &[&str]) -> Result<String, Failure> {
        let out = self
            .command("git")
            .args(args)
            .current_dir(dir)
            .output()
            .map_err(infra)?;
        if !out.status.success() {
            return Err(Failure::Infra(format!(
                "git {args:?} in {} failed: {}",
                dir.display(),
                String::from_utf8_lossy(&out.stderr)
            )));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

/// Which shell runs a script.
#[derive(Clone, Copy, Debug)]
pub enum Shell {
    /// `sh -c`.
    Posix,
    /// `/bin/bash -c`: a plain shell with no `-e`, the way a caller pasting a
    /// returned ladder into an existing session runs it. On macOS this is the
    /// system bash 3.2.
    Bash,
    /// `/bin/bash -euo pipefail -c`: the way an orchestrator runs a ladder.
    BashStrict,
}

/// How to run a script with [`PlanRepo::run`].
#[derive(Clone, Debug)]
pub struct Run {
    /// The shell.
    pub shell: Shell,
    /// The `PATH` the script sees.
    pub path: String,
    /// The working directory (the plan repo's temp directory when `None`).
    pub cwd: Option<PathBuf>,
    /// Extra environment, applied last.
    pub env: Vec<(String, String)>,
    /// The `RDM_SESSION` the script runs in.
    pub session: String,
}

impl Run {
    /// `sh -c` with [`BARE_PATH`].
    pub fn bare() -> Self {
        Self {
            shell: Shell::Posix,
            path: BARE_PATH.to_owned(),
            cwd: None,
            env: Vec::new(),
            session: "ladder".to_owned(),
        }
    }

    /// The same run under `shell`.
    pub fn shell(mut self, shell: Shell) -> Self {
        self.shell = shell;
        self
    }

    /// The same run with `dir` prepended to `PATH`.
    pub fn path_prepend(mut self, dir: &Path) -> Self {
        self.path = format!("{}:{}", dir.display(), self.path);
        self
    }

    /// The same run in `dir`.
    pub fn cwd(mut self, dir: &Path) -> Self {
        self.cwd = Some(dir.to_owned());
        self
    }

    /// The same run with one more environment variable.
    pub fn env(mut self, key: &str, value: &str) -> Self {
        self.env.push((key.to_owned(), value.to_owned()));
        self
    }

    /// The same run in session `session`.
    pub fn session(mut self, session: &str) -> Self {
        self.session = session.to_owned();
        self
    }
}

/// How a [`PlanRepo::run_open_stdin`] run ended.
#[derive(Debug)]
pub enum OpenStdin {
    /// The script exited within the deadline.
    Exited {
        /// Its exit status.
        code: i32,
        /// Everything it printed on stdout.
        stdout: String,
    },
    /// The deadline passed; the process group was killed and reaped.
    TimedOut {
        /// Stdout received before the deadline.
        stdout: String,
        /// The tail of stderr.
        stderr: String,
    },
}

/// Everything under a plan repo that a mutation could change: HEAD, the
/// working-tree status, and every non-`.git` file's bytes.
#[derive(Debug, PartialEq, Eq)]
pub struct Snapshot {
    /// `git rev-parse HEAD`.
    pub head: String,
    /// `git status --porcelain --untracked-files=all`.
    pub status: String,
    /// Relative path to contents, for every file outside `.git`.
    pub files: BTreeMap<String, Vec<u8>>,
}

/// A committed plan repo in its own temp directory (`<TempDir>/plan`).
pub struct PlanRepo {
    /// The owning temp directory (also holds scratch dirs and any source repo).
    pub dir: TempDir,
    /// The plan repo root.
    pub root: PathBuf,
}

impl PlanRepo {
    /// `rdm init --default-project <default_project>` in a fresh temp dir.
    pub fn init(default_project: &str) -> Result<Self, Failure> {
        let dir = TempDir::new().map_err(infra)?;
        let root = dir.path().join("plan");
        std::fs::create_dir_all(&root).map_err(infra)?;
        let repo = Self { dir, root };
        repo.ok("seed", &["init", "--default-project", default_project])?;
        Ok(repo)
    }

    /// A repo whose default project is [`DECOY`], plus each of `projects`.
    pub fn with_decoy(projects: &[&str]) -> Result<Self, Failure> {
        let repo = Self::init(DECOY)?;
        for p in projects {
            repo.ok("seed", &["project", "create", p, "--title", p])?;
        }
        Ok(repo)
    }

    /// Runs `rdm` in `session`, returning its output whatever the status.
    pub fn rdm(&self, session: &str, args: &[&str]) -> Result<Output, Failure> {
        self.rdm_in(session, args, self.dir.path())
    }

    /// Like [`PlanRepo::rdm`], in `cwd`.
    pub fn rdm_in(&self, session: &str, args: &[&str], cwd: &Path) -> Result<Output, Failure> {
        let mut cmd = Command::new(rdm_bin());
        hermetic(&mut cmd, &self.root, session);
        cmd.args(args)
            .current_dir(cwd)
            .output()
            .map_err(|e| Failure::Infra(format!("running rdm: {e}")))
    }

    /// Runs `rdm` in `session`, requiring success; returns stdout.
    pub fn ok(&self, session: &str, args: &[&str]) -> Result<String, Failure> {
        let out = self.rdm(session, args)?;
        if !out.status.success() {
            return Err(Failure::Infra(format!(
                "rdm {args:?} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            )));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Runs each argument list through [`PlanRepo::ok`] in session `seed`,
    /// then commits the seed.
    pub fn seed(&self, cmds: &[&[&str]]) -> Result<(), Failure> {
        for args in cmds {
            self.ok("seed", args)?;
        }
        self.ok("seed", &["commit", "-m", "chore(plan): seed"])?;
        Ok(())
    }

    /// Runs a read-only `rdm` command and parses its stdout as JSON.
    pub fn json(&self, args: &[&str]) -> Result<Value, Failure> {
        let text = self.ok("reader", args)?;
        serde_json::from_str(&text)
            .map_err(|e| Failure::Infra(format!("rdm {args:?} json: {e}: {text}")))
    }

    /// `phase show <stem> --roadmap <roadmap> --project <project> --format json`.
    pub fn phase(&self, project: &str, roadmap: &str, stem: &str) -> Result<Value, Failure> {
        self.json(&[
            "phase",
            "show",
            stem,
            "--roadmap",
            roadmap,
            "--project",
            project,
            "--format",
            "json",
        ])
    }

    /// `task show <slug> --project <project> --format json`.
    pub fn task(&self, project: &str, slug: &str) -> Result<Value, Failure> {
        self.json(&[
            "task",
            "show",
            slug,
            "--project",
            project,
            "--format",
            "json",
        ])
    }

    /// `review show <id> --project <project> --format json`.
    pub fn review(&self, project: &str, id: &str) -> Result<Value, Failure> {
        self.json(&[
            "review",
            "show",
            id,
            "--project",
            project,
            "--format",
            "json",
        ])
    }

    /// Every review on `target` (fully shown).
    pub fn reviews_on(&self, project: &str, target: &str) -> Result<Vec<Value>, Failure> {
        let v = self.json(&[
            "review",
            "list",
            "--on",
            target,
            "--format",
            "json",
            "--project",
            project,
        ])?;
        let list = v
            .as_array()
            .cloned()
            .or_else(|| v["reviews"].as_array().cloned())
            .unwrap_or_default();
        list.iter()
            .filter_map(|r| r["id"].as_str())
            .map(|id| self.review(project, id))
            .collect()
    }

    /// A fresh, empty directory under the temp dir.
    pub fn tmpdir(&self, name: &str) -> Result<PathBuf, Failure> {
        let p = self.dir.path().join(name);
        std::fs::create_dir_all(&p).map_err(infra)?;
        Ok(p)
    }

    /// Runs `script` under `sh -c` with the hermetic environment, the
    /// inherited `PATH`, and `tmpdir` as `TMPDIR`.
    pub fn sh(&self, script: &str, session: &str, tmpdir: &Path) -> Result<Output, Failure> {
        let mut cmd = Command::new("sh");
        hermetic(&mut cmd, &self.root, session);
        cmd.arg("-c")
            .arg(script)
            .env("TMPDIR", tmpdir)
            .current_dir(self.dir.path())
            .output()
            .map_err(|e| Failure::Infra(format!("running sh: {e}")))
    }

    /// The command [`PlanRepo::run`] would spawn.
    pub fn command(&self, run: &Run, script: &str) -> Result<Command, Failure> {
        let tmp = self.tmpdir("tmp")?;
        let mut cmd = match run.shell {
            Shell::Posix => {
                let mut c = Command::new("sh");
                c.arg("-c");
                c
            }
            Shell::Bash => {
                let mut c = Command::new("/bin/bash");
                c.arg("-c");
                c
            }
            Shell::BashStrict => {
                let mut c = Command::new("/bin/bash");
                c.args(["-euo", "pipefail", "-c"]);
                c
            }
        };
        hermetic(&mut cmd, &self.root, &run.session);
        cmd.arg(script)
            .env("PATH", &run.path)
            .env("HOME", self.dir.path())
            .env("TMPDIR", &tmp)
            .current_dir(run.cwd.as_deref().unwrap_or(self.dir.path()));
        for (k, v) in &run.env {
            cmd.env(k, v);
        }
        Ok(cmd)
    }

    /// Runs `script` as `run` describes, returning its output whatever the
    /// status.
    pub fn run(&self, run: &Run, script: &str) -> Result<Output, Failure> {
        self.command(run, script)?
            .output()
            .map_err(|e| Failure::Infra(format!("running a script: {e}")))
    }

    /// Runs `script` and requires success; returns stdout.
    pub fn run_ok(&self, run: &Run, script: &str) -> Result<String, Failure> {
        let out = self.run(run, script)?;
        check!(
            out.status.success(),
            "the returned script exited {:?}:\n{script}\n--- stdout ---\n{}\n--- stderr ---\n{}",
            out.status.code(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Runs `script` under a plain `/bin/bash -c` the way an agent's Bash
    /// tool does: stdin is a pipe that is never written to and never closed.
    /// The run is an `rdm_devtools::process::Session`, so one overall
    /// `deadline` bounds it and the whole process group is killed and reaped
    /// on every exit path; no timeout is hand-rolled here.
    pub fn run_open_stdin(
        &self,
        run: &Run,
        script: &str,
        deadline: Duration,
    ) -> Result<OpenStdin, Failure> {
        const EXIT_MARK: &str = "rdm-test-exit-status=";
        let tmp = self.tmpdir("tmp")?;
        // The script runs in a subshell so its own `exit` still reaches the
        // status line; the status line is the only framing added.
        let wrapped = format!("(\n{script}\n)\necho \"{EXIT_MARK}$?\"");
        let mut spec = ProcessSpec::new("/bin/bash")
            .arg("-c")
            .arg(&wrapped)
            .cwd(run.cwd.as_deref().unwrap_or(self.dir.path()))
            .timeout(deadline);
        for key in hermetic_removals() {
            spec = spec.env_remove(key);
        }
        for (key, value) in hermetic_vars(&self.root, &run.session) {
            spec = spec.env(key, value);
        }
        spec = spec
            .env("PATH", &run.path)
            .env("HOME", self.dir.path())
            .env("TMPDIR", &tmp);
        for (k, v) in &run.env {
            spec = spec.env(k, v);
        }
        let mut session = Session::spawn(&spec).map_err(infra)?;
        let mut stdout = String::new();
        loop {
            match session.recv_line() {
                Ok(line) => {
                    stdout.push_str(&line);
                    stdout.push('\n');
                }
                Err(SessionError::Exited { .. }) => break,
                Err(SessionError::TimedOut { stderr, .. }) => {
                    return Ok(OpenStdin::TimedOut { stdout, stderr });
                }
                Err(e) => return Err(infra(e)),
            }
        }
        let code = stdout
            .lines()
            .filter_map(|l| l.strip_prefix(EXIT_MARK))
            .next_back()
            .and_then(|c| c.trim().parse::<i32>().ok())
            .ok_or_else(|| Failure::Infra(format!("no exit status line in: {stdout}")))?;
        Ok(OpenStdin::Exited { code, stdout })
    }

    /// A `bin/` directory holding an `rdm` symlink to the binary under test,
    /// for prepending to [`Run::bare`]'s `PATH`.
    pub fn rdm_on_path(&self) -> Result<PathBuf, Failure> {
        let bin = self.dir.path().join("bin");
        let link = bin.join("rdm");
        if !link.exists() {
            std::fs::create_dir_all(&bin).map_err(infra)?;
            std::os::unix::fs::symlink(rdm_bin(), &link).map_err(infra)?;
        }
        Ok(bin)
    }

    /// The plan repo's HEAD commit.
    pub fn head(&self) -> String {
        let out = git(&self.root, &["rev-parse", "HEAD"]);
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    /// See [`Snapshot`].
    pub fn snapshot(&self) -> Result<Snapshot, Failure> {
        let status = git(
            &self.root,
            &["status", "--porcelain", "--untracked-files=all"],
        );
        let mut files = BTreeMap::new();
        let mut stack = vec![self.root.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).map_err(infra)? {
                let entry = entry.map_err(infra)?;
                let path = entry.path();
                if path.file_name().is_some_and(|n| n == ".git") {
                    continue;
                }
                if entry.file_type().map_err(infra)?.is_dir() {
                    stack.push(path);
                } else {
                    let rel = path
                        .strip_prefix(&self.root)
                        .map_err(infra)?
                        .to_string_lossy()
                        .into_owned();
                    files.insert(rel, std::fs::read(&path).map_err(infra)?);
                }
            }
        }
        Ok(Snapshot {
            head: self.head(),
            status: String::from_utf8_lossy(&status.stdout).into_owned(),
            files,
        })
    }
}

/// A source checkout's pinned identity: the four arguments a source-bound
/// review takes.
#[derive(Clone, Debug)]
pub struct Pin {
    /// The checkout path.
    pub source: String,
    /// The reviewed range's base commit.
    pub base: String,
    /// The reviewed head commit.
    pub expected_head: String,
    /// The branch the head is on.
    pub expected_branch: String,
}

impl Pin {
    /// The pin as the engines' flat argument keys.
    pub fn args(&self) -> Value {
        json!({
            "source": self.source,
            "base": self.base,
            "expectedHead": self.expected_head,
            "expectedBranch": self.expected_branch,
        })
    }
}

/// A git source repo at `<plan TempDir>/src` with one empty initial commit on
/// `main`.
pub struct SourceRepo {
    /// The repo root.
    pub root: PathBuf,
}

impl SourceRepo {
    /// Creates the repo next to `plan`'s plan root.
    pub fn init(plan: &PlanRepo) -> Result<Self, Failure> {
        let root = plan.dir.path().join("src");
        std::fs::create_dir_all(&root).map_err(infra)?;
        git(&root, &["init", "--quiet", "-b", "main", "."]);
        git(
            &root,
            &["commit", "--quiet", "--allow-empty", "-m", "chore: initial"],
        );
        Ok(Self { root })
    }

    /// `rdm worktree add <item> --project <project>`, run from inside the
    /// repo; returns the worktree path.
    pub fn add_worktree(
        &self,
        plan: &PlanRepo,
        item: &str,
        project: &str,
    ) -> Result<PathBuf, Failure> {
        let out = plan.rdm_in(
            "seed",
            &["worktree", "add", item, "--project", project],
            &self.root,
        )?;
        check!(
            out.status.success(),
            "rdm worktree add {item} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        Ok(PathBuf::from(
            String::from_utf8_lossy(&out.stdout).trim().to_owned(),
        ))
    }
}

/// Commits one file per entry of `files` (each containing `shipped`) on
/// `worktree`'s branch and pins the range `main..HEAD`.
pub fn pin(worktree: &Path, files: &[&str]) -> Result<Pin, Failure> {
    for f in files {
        let full = worktree.join(f);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).map_err(infra)?;
        }
        std::fs::write(&full, "shipped\n").map_err(infra)?;
        git(worktree, &["add", f]);
    }
    git(
        worktree,
        &[
            "commit",
            "--quiet",
            "-m",
            &format!("feat: {}", files.join(", ")),
        ],
    );
    Ok(Pin {
        source: worktree.to_string_lossy().into_owned(),
        base: rev(worktree, "main"),
        expected_head: rev(worktree, "HEAD"),
        expected_branch: branch(worktree),
    })
}

/// Writes `contents` to `file` in `worktree` and commits it; returns the new
/// HEAD.
pub fn commit_file(worktree: &Path, file: &str, contents: &str, msg: &str) -> String {
    std::fs::write(worktree.join(file), contents).expect("write source file");
    git(worktree, &["add", file]);
    git(worktree, &["commit", "--quiet", "-m", msg]);
    rev(worktree, "HEAD")
}

/// `git rev-parse <what>` in `dir`.
pub fn rev(dir: &Path, what: &str) -> String {
    let out = git(dir, &["rev-parse", what]);
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// The current branch of `dir`.
pub fn branch(dir: &Path) -> String {
    let out = git(dir, &["rev-parse", "--abbrev-ref", "HEAD"]);
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// Joins a returned command list into one script.
pub fn script(commands: &Value) -> String {
    commands
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join("\n")
}

/// The last `reviewId=<id>` line a ladder printed.
pub fn review_id(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .filter_map(|l| l.strip_prefix("reviewId="))
        .next_back()
        .map(str::to_owned)
}
