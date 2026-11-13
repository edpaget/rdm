//! Docker-gated: the SCBench agent image's plan root is trusted by git for a
//! uid that does not own it.
//!
//! Builds a small image mirroring `bench/scbench/agent/docker.j2`'s plan-root
//! lines (the SCBench base image, `/opt/rdm` from a local payload image, and
//! `plan-root-setup.sh` copied from this checkout), then runs `rdm init`,
//! `rdm task create` and `rdm commit` as an unprivileged uid with no passwd
//! entry and no gitconfig of its own. A negative control disables the system
//! gitconfig and expects git's "dubious ownership" refusal. It never pulls an
//! image, compiles rdm, or runs SCBench or `claude`. It skips (passes with a
//! `skipping:` line on stderr) when Docker, the base image or a payload image
//! is absent.

use std::path::Path;
use std::process::{Command, Output};

use rdm_devtools::measure::scbench::rdm_image::{PAYLOAD_REPO, payload_tag};

/// The SCBench environment image for the manifest's `docker-python3.12-uv`
/// environment: the base the real agent image is `FROM` (Debian trixie with
/// apt `git`).
const BASE_IMAGE: &str = "slop-code:python3.12";

/// The uid the container runs as: no passwd entry, owns nothing in the image.
const RUN_USER: &str = "4321:4321";

/// Runs inside the container as `RUN_USER`.
const PROBE: &str = r#"set -e
mkdir -p "$HOME"
# One changeset across the separate rdm calls (no harness session id here).
export RDM_SESSION=probe
stat -c %u /rdm-plan
/opt/rdm/bin/rdm --root /rdm-plan init --default-project bench
/opt/rdm/bin/rdm --root /rdm-plan task create probe --title Probe --no-edit --project bench
/opt/rdm/bin/rdm --root /rdm-plan commit -m probe
git -C /rdm-plan log -1 --format=%s
"#;

fn docker(args: &[&str]) -> Option<Output> {
    Command::new("docker").args(args).output().ok()
}

fn text(output: &Output) -> String {
    format!(
        "status: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// Removes the built image on drop.
struct Image(String);

impl Drop for Image {
    fn drop(&mut self) {
        let _ = docker(&["image", "rm", "-f", &self.0]);
    }
}

/// The payload tag to copy rdm from: this checkout's HEAD when present,
/// otherwise the newest local tag.
fn payload_image() -> Option<String> {
    let listed = docker(&["image", "ls", PAYLOAD_REPO, "--format", "{{.Tag}}"])?;
    if !listed.status.success() {
        return None;
    }
    let tags: Vec<String> = String::from_utf8_lossy(&listed.stdout)
        .lines()
        .map(str::trim)
        .filter(|t| !t.is_empty() && *t != "<none>")
        .map(str::to_owned)
        .collect();
    let head = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned());
    let tag = match head {
        Some(h) if tags.contains(&h) => h,
        _ => tags.into_iter().next()?,
    };
    Some(payload_tag(&tag))
}

/// Builds the image, or returns `None` after printing why the test skips.
fn prepare() -> Option<Image> {
    match docker(&["version", "--format", "{{.Server.Version}}"]) {
        Some(o) if o.status.success() => {}
        _ => {
            eprintln!("skipping: docker CLI or daemon unavailable");
            return None;
        }
    }
    if !docker(&["image", "inspect", BASE_IMAGE]).is_some_and(|o| o.status.success()) {
        eprintln!("skipping: base image {BASE_IMAGE} absent");
        return None;
    }
    let Some(payload) = payload_image() else {
        eprintln!("skipping: no {PAYLOAD_REPO} image");
        return None;
    };
    let ctx = tempfile::TempDir::new().expect("temp dir");
    let script =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../bench/scbench/agent/plan-root-setup.sh");
    std::fs::copy(&script, ctx.path().join("plan-root-setup.sh")).expect("copy setup script");
    std::fs::write(
        ctx.path().join("Dockerfile"),
        format!(
            "FROM {BASE_IMAGE}\nUSER root\nCOPY --from={payload} /opt/rdm /opt/rdm\n\
             COPY plan-root-setup.sh /opt/rdm/bin/rdm-plan-root-setup\n\
             RUN /opt/rdm/bin/rdm-plan-root-setup /rdm-plan\n"
        ),
    )
    .expect("write Dockerfile");
    let ctx_path = ctx.path().to_str().expect("utf-8 temp path");
    let built = docker(&["build", "-q", ctx_path]).expect("spawn docker build");
    assert!(
        built.status.success(),
        "docker build failed\n{}",
        text(&built)
    );
    let id = String::from_utf8_lossy(&built.stdout).trim().to_owned();
    eprintln!("built {id} from {payload}");
    Some(Image(id))
}

fn run(image: &Image, extra_env: &[&str]) -> Output {
    let mut args = vec!["run", "--rm", "--user", RUN_USER, "-e", "HOME=/tmp/h"];
    for var in [
        "GIT_AUTHOR_NAME=rdm-scbench",
        "GIT_AUTHOR_EMAIL=rdm-scbench@localhost",
        "GIT_COMMITTER_NAME=rdm-scbench",
        "GIT_COMMITTER_EMAIL=rdm-scbench@localhost",
    ]
    .iter()
    .chain(extra_env)
    {
        args.extend(["-e", var]);
    }
    args.extend([image.0.as_str(), "sh", "-c", PROBE]);
    docker(&args).expect("spawn docker run")
}

#[test]
fn plan_repo_is_trusted_for_a_non_owner_uid() {
    let Some(image) = prepare() else { return };
    let out = run(&image, &[]);
    let report = text(&out);
    eprintln!("{report}");
    assert!(out.status.success(), "probe failed\n{report}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines.first().copied(),
        Some("0"),
        "plan root owner\n{report}"
    );
    assert_eq!(
        lines.last().copied(),
        Some("probe"),
        "commit landed\n{report}"
    );
}

#[test]
fn without_the_system_entry_git_refuses_the_plan_repo() {
    let Some(image) = prepare() else { return };
    let out = run(&image, &["GIT_CONFIG_NOSYSTEM=1"]);
    let report = text(&out);
    eprintln!("{report}");
    assert!(!out.status.success(), "probe should fail\n{report}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("dubious ownership"),
        "expected git's dubious-ownership refusal\n{report}"
    );
}
