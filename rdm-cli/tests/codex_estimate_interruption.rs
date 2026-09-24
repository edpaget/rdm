use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
#[path = "support/command.rs"]
mod command;

struct Fixture {
    root: tempfile::TempDir,
    child: Option<Child>,
}
impl Fixture {
    fn source(&self) -> PathBuf {
        self.root.path().join("source")
    }
    fn plans(&self) -> PathBuf {
        self.root.path().join("plans")
    }
    fn command(&self, bin: impl AsRef<std::ffi::OsStr>, args: &[&str], session: &str) -> Command {
        let mut c = Command::new(bin);
        c.args(args)
            .current_dir(self.source())
            .env_clear()
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    self.root.path().join("bin").display(),
                    std::env::var("PATH").unwrap()
                ),
            )
            .env("HOME", self.root.path())
            .env("XDG_CONFIG_HOME", self.root.path())
            .env("RDM_ROOT", self.plans())
            .env("RDM_PROJECT", "fixture")
            .env("RDM_SESSION", session)
            .env("RDM_BIN", assert_cmd::cargo::cargo_bin!("rdm"))
            .env("FIXTURE_ROOT", self.root.path())
            .env("FIXTURE_RDM", assert_cmd::cargo::cargo_bin!("rdm"))
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
            .env("GIT_COMMITTER_NAME", "Fixture")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid");
        c
    }
    fn exec(&self, bin: impl AsRef<std::ffi::OsStr>, args: &[&str], session: &str) -> String {
        let output = command::bounded_output(&mut self.command(bin, args, session)).unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
    fn rdm(&self, args: &[&str], session: &str) -> String {
        self.exec(assert_cmd::cargo::cargo_bin!("rdm"), args, session)
    }
    fn show(&self, stem: &str, session: &str) -> Value {
        serde_json::from_str(&self.rdm(
            &[
                "phase",
                "show",
                stem,
                "--roadmap",
                "example",
                "--format",
                "json",
            ],
            session,
        ))
        .unwrap()
    }
    fn head(&self) -> String {
        self.exec(
            "git",
            &["-C", self.plans().to_str().unwrap(), "rev-parse", "HEAD"],
            "seed",
        )
    }
    fn committed(&self, stem: &str) -> String {
        self.exec(
            "git",
            &[
                "-C",
                self.plans().to_str().unwrap(),
                "show",
                &format!("HEAD:projects/fixture/roadmaps/example/{stem}.md"),
            ],
            "seed",
        )
    }
    fn read(&self, path: &str) -> String {
        std::fs::read_to_string(self.root.path().join(path)).unwrap()
    }
    fn json(&self, path: &str) -> Value {
        serde_json::from_str(&self.read(path)).unwrap()
    }
}
fn signal(pid: u32, name: &str) {
    let _ = Command::new("kill")
        .args([name, &pid.to_string()])
        .stderr(Stdio::null())
        .status();
}
fn alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success()
}
fn until(mut check: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(20);
    while !check() {
        assert!(Instant::now() < end, "fixture timed out");
        std::thread::sleep(Duration::from_millis(20));
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Ok(text) = std::fs::read_to_string(self.root.path().join("boundary")) {
            for pid in text.lines().take(2).filter_map(|p| p.parse().ok()) {
                signal(pid, "-KILL");
            }
        }
        if let Ok(text) = std::fs::read_to_string(self.root.path().join("calls")) {
            for pid in text
                .lines()
                .filter_map(|p| p.split('\t').next()?.parse().ok())
            {
                signal(pid, "-KILL");
            }
        }
    }
}
fn interruption(boundary: &str) {
    let mut f = Fixture {
        root: tempfile::tempdir().unwrap(),
        child: None,
    };
    for d in ["source", "plans", "bin"] {
        std::fs::create_dir(f.root.path().join(d)).unwrap();
    }
    let checkout = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let fixture_binary = f.root.path().join("bin/rdm-wrapper");
    let compile = command::bounded_output(
        Command::new("rustc")
            .args(["--edition=2024", "-o"])
            .arg(&fixture_binary)
            .arg(checkout.join("rdm-cli/tests/support/codex-estimate-host.rs")),
    )
    .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    std::fs::copy(&fixture_binary, f.root.path().join("bin/codex")).unwrap();
    for stem in ["phase-1-first", "phase-2-second"] {
        std::fs::write(
            f.root.path().join(stem),
            json!({"stem":stem,"difficulty":"moderate","justification":"Bounded fixture work."})
                .to_string(),
        )
        .unwrap();
    }
    std::fs::write(
        f.root.path().join("events"),
        format!(
            "{}\n{}\n",
            json!({"type":"thread.started","thread_id":"fixture"}),
            json!({"type":"turn.completed","usage":{"input_tokens":1,"output_tokens":1}})
        ),
    )
    .unwrap();
    f.exec("git", &["init", "-q", "-b", "main"], "seed");
    std::fs::write(f.source().join("README"), "fixture").unwrap();
    f.exec("git", &["add", "."], "seed");
    f.exec("git", &["commit", "-qm", "seed"], "seed");
    f.rdm(&["init", "--default-project", "fixture"], "seed");
    f.rdm(
        &[
            "roadmap",
            "create",
            "example",
            "--title",
            "Example",
            "--body",
            "Fixture roadmap",
            "--no-edit",
        ],
        "seed",
    );
    for (number, slug) in [("1", "first"), ("2", "second"), ("3", "other")] {
        f.rdm(
            &[
                "phase",
                "create",
                slug,
                "--number",
                number,
                "--title",
                slug,
                "--body",
                "Original body.",
                "--no-edit",
                "--roadmap",
                "example",
            ],
            "seed",
        );
    }
    f.rdm(
        &[
            "phase",
            "update",
            "phase-3-other",
            "--difficulty",
            "hard",
            "--no-edit",
            "--roadmap",
            "example",
        ],
        "seed",
    );
    f.rdm(&["commit", "-m", "test: seed"], "seed");
    f.rdm(
        &[
            "phase",
            "update",
            "phase-3-other",
            "--body",
            "Unrelated pending edit.",
            "--no-edit",
            "--roadmap",
            "example",
        ],
        "unrelated",
    );
    let initial_head = f.head();
    let mut spec = json!({"operation":"estimate","sourceDir":f.source().canonicalize().unwrap(),"planRoot":f.plans().canonicalize().unwrap(),
        "rdmBin":fixture_binary.canonicalize().unwrap(),"project":"fixture","session":"parent","roadmap":"example","apply":true,"runDir":f.root.path().canonicalize().unwrap().join("run"),"rdmTimeoutMs":60000});
    let spec_file = f.root.path().join("spec.json");
    std::fs::write(&spec_file, spec.to_string()).unwrap();
    let runner = checkout.join("scripts/rdm-codex.mjs");
    let args = [runner.to_str().unwrap(), spec_file.to_str().unwrap()];
    let stdout = std::fs::File::create(f.root.path().join("stdout")).unwrap();
    let stderr = std::fs::File::create(f.root.path().join("stderr")).unwrap();
    f.child = Some(
        f.command("node", &args, "parent")
            .env("FIXTURE_BOUNDARY", boundary)
            .stdin(Stdio::null())
            .stdout(stdout)
            .stderr(stderr)
            .spawn()
            .unwrap(),
    );
    if boundary != "missing" {
        until(|| {
            f.root.path().join("boundary").exists()
                || f.child.as_mut().unwrap().try_wait().unwrap().is_some()
        });
        assert!(
            f.root.path().join("boundary").exists(),
            "{}",
            f.read("stderr")
        );
        signal(f.child.as_ref().unwrap().id(), "-TERM");
    }
    until(|| f.child.as_mut().unwrap().try_wait().unwrap().is_some());
    assert!(!f.child.as_mut().unwrap().wait().unwrap().success());
    assert_eq!(f.read("stdout"), "");
    assert!(
        f.read("stderr").contains("uncertain"),
        "{}",
        f.read("stderr")
    );
    let manifest = f.json("run/manifest.json");
    assert_eq!(manifest["status"], "failed");
    assert_eq!(manifest["uncertainWrites"], true);
    let session = manifest["identity"]["session"].as_str().unwrap();
    let journal: Vec<Value> = f
        .read("run/journal.jsonl")
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert!(journal.iter().all(|e| e["type"] != "run-completed"));
    assert_eq!(
        f.show("phase-3-other", "unrelated")["body"]
            .as_str()
            .unwrap()
            .trim(),
        "Unrelated pending edit."
    );
    assert!(
        !f.committed("phase-3-other")
            .contains("Unrelated pending edit.")
    );
    if boundary == "missing" {
        let intent = journal
            .iter()
            .find(|e| e["type"] == "write-intent" && e["data"]["args"][0] == "commit")
            .unwrap();
        let ack = journal
            .iter()
            .find(|e| {
                e["type"] == "write-acknowledged" && e["data"]["callId"] == intent["data"]["callId"]
            })
            .unwrap();
        assert!(
            ack["data"]["stdout"]
                .as_str()
                .unwrap()
                .to_lowercase()
                .contains("skip")
        );
        let owned: Value =
            serde_json::from_str(&f.rdm(&["session", "journal", "--format", "json"], session))
                .unwrap();
        assert!(!owned["paths"].as_array().unwrap().is_empty());
        return;
    }
    let marker = f.read("boundary");
    let ids: Vec<_> = marker.lines().collect();
    assert_eq!(ids[2], session);
    for pid in ids.iter().take(2).map(|p| p.parse().unwrap()) {
        until(|| !alive(pid));
    }
    assert!(journal.iter().any(|e| e["type"] == "write-uncertain"));
    let calls = f.read("calls");
    assert_eq!(
        calls.lines().filter(|l| l.contains("\tcommit\t")).count(),
        usize::from(boundary == "commit")
    );
    assert_eq!(
        calls
            .lines()
            .filter(|l| l.contains("\tphase\tupdate\t"))
            .count(),
        if boundary == "commit" { 2 } else { 1 }
    );
    for line in f.read("agents").lines() {
        assert!(!alive(line.split('\t').next().unwrap().parse().unwrap()));
    }
    if boundary == "update" {
        assert_eq!(f.head(), initial_head);
        assert_eq!(f.show("phase-1-first", session)["difficulty"], "moderate");
        assert!(f.show("phase-2-second", "seed")["difficulty"].is_null());
        f.rdm(&["status"], session);
        f.rdm(
            &["commit", "-m", "test: reconcile interrupted estimate"],
            session,
        );
    } else {
        assert_ne!(f.head(), initial_head);
        f.rdm(&["status"], session);
        assert!(f.committed("phase-1-first").contains("Original body."));
    }
    assert!(
        !command::bounded_output(&mut f.command("node", &args, "parent"))
            .unwrap()
            .status
            .success()
    );
    spec["runDir"] = json!(f.root.path().canonicalize().unwrap().join("fresh-run"));
    std::fs::write(&spec_file, spec.to_string()).unwrap();
    let retry: Value = serde_json::from_str(&f.exec("node", &args, "parent")).unwrap();
    assert_eq!(
        retry["result"]["estimated"].as_array().unwrap().len(),
        usize::from(boundary == "update")
    );
    for stem in ["phase-1-first", "phase-2-second"] {
        let phase = f.show(stem, "seed");
        assert_eq!(phase["difficulty"], "moderate");
        assert_eq!(phase["body"].as_str().unwrap().trim(), "Original body.");
    }
    assert_eq!(
        f.show("phase-3-other", "unrelated")["body"]
            .as_str()
            .unwrap()
            .trim(),
        "Unrelated pending edit."
    );
    assert!(
        !f.committed("phase-3-other")
            .contains("Unrelated pending edit.")
    );
}
#[test]
fn estimate_sigterm_after_actual_update_requires_reconciliation() {
    interruption("update");
}
#[test]
fn estimate_sigterm_after_actual_commit_requires_reconciliation() {
    interruption("commit");
}
#[test]
fn estimate_zero_exit_commit_skipping_removed_phase_is_uncertain() {
    interruption("missing");
}
