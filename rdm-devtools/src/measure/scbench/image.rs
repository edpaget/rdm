//! `rdm-measure scbench image build|check`: builds the rdm-equipped agent
//! image an `image = "rdm"` arm runs in, and smoke-checks it.
//!
//! `build` (slow: a release build of rdm plus SCBench's base image):
//!
//! 1. loads the manifest and requires its `[rdm]` table;
//! 2. resolves the rdm checkout under test (`git rev-parse --show-toplevel`
//!    and `HEAD` from the manifest's directory) and refuses a dirty one, since
//!    the build context is the working tree;
//! 3. provisions the pinned SCBench checkout under `--cache-dir` and refuses
//!    a plan root inside the environment's graded directory;
//! 4. runs `docker build` of [`rdm_image::DOCKERFILE`] with the checkout as
//!    context, `--build-arg RDM_SHA=<HEAD>`, the [`rdm_image::SHA_LABEL`]
//!    label and the [`rdm_image::payload_tag`];
//! 5. writes the rendered agent template and the rdm agent config under
//!    `<cache>/rdm-image/<HEAD>/`;
//! 6. runs `uv run slop-code docker build-agent ... --force-build`, which
//!    builds exactly the agent image a run uses; then, whatever its exit
//!    status, scrubs the provider's credential from `<cache>/scbench-home`
//!    and `<cache>/rdm-image/<HEAD>/` ([`redact`]).
//!
//! `check` runs three probes in that agent image with `docker run --rm`: the
//! recorded [`rdm_image::SHA_FILE`] equals HEAD, `rdm --version` exits 0, and
//! `claude --plugin-dir` lists every skill under the checkout's
//! `plugins/rdm/skills`. Every probe runs; each failure is printed, and the
//! exit code is 1 when any failed.
//!
//! Neither runs an agent or makes a paid call.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use super::environment::{self, Environment};
use super::exec::{Io, provision, spawn_command, write_file, write_json};
use super::manifest::{Manifest, Rdm};
use super::plan::{self, CommandSpec};
use super::rdm_image;
use super::redact;

/// Options for [`build`] and [`check`].
#[derive(Debug, Clone)]
pub struct ImageOptions {
    /// The manifest file.
    pub manifest: PathBuf,
    /// Where the pinned SCBench checkout and the generated image files live.
    pub cache_dir: PathBuf,
    /// The `docker` executable.
    pub docker: String,
    /// The `uv` executable (`build` only).
    pub uv: String,
}

/// The loaded inputs both subcommands share.
struct Target {
    manifest: Manifest,
    rdm: Rdm,
    toplevel: PathBuf,
    sha: String,
    cache_dir: PathBuf,
}

fn spec(program: &str, args: &[&str]) -> CommandSpec {
    CommandSpec {
        program: program.to_owned(),
        args: args.iter().map(|&a| a.to_owned()).collect(),
        cwd: None,
        env: BTreeMap::new(),
    }
}

/// Runs `spec` with captured stdout and returns its trimmed stdout.
fn capture(what: &str, spec: &CommandSpec) -> Result<String, String> {
    let (ran, _) = spawn_command(spec, Io::Capture);
    if !ran.ok() {
        return Err(format!(
            "{what} (`{}`) {}",
            spec.argv().join(" "),
            ran.describe()
        ));
    }
    Ok(ran.stdout.trim().to_owned())
}

/// Runs `spec` with inherited stdio and fails naming `what` unless it exits 0.
fn inherit(what: &str, spec: &CommandSpec) -> Result<(), String> {
    let (ran, _) = spawn_command(spec, Io::Inherit);
    if !ran.ok() {
        return Err(format!(
            "{what} (`{}`) {}",
            spec.argv().join(" "),
            ran.describe()
        ));
    }
    Ok(())
}

fn load(opts: &ImageOptions) -> Result<Target, String> {
    let manifest_path = std::path::absolute(&opts.manifest).unwrap_or(opts.manifest.clone());
    let manifest = Manifest::load(&manifest_path)?;
    let rdm = manifest.rdm_table()?.clone();
    let dir = manifest.dir.display().to_string();
    let toplevel = PathBuf::from(capture(
        "cannot resolve the rdm checkout",
        &spec("git", &["-C", &dir, "rev-parse", "--show-toplevel"]),
    )?);
    let sha = capture(
        "cannot resolve the rdm SHA",
        &spec("git", &["-C", &dir, "rev-parse", "HEAD"]),
    )?;
    Ok(Target {
        manifest,
        rdm,
        toplevel,
        sha,
        cache_dir: std::path::absolute(&opts.cache_dir).unwrap_or(opts.cache_dir.clone()),
    })
}

fn environment_path(t: &Target) -> PathBuf {
    environment::path(
        &plan::scbench_dir(&t.cache_dir),
        &t.manifest.agent.environment,
    )
}

/// The directory `build` writes the rendered template and agent config to.
fn image_dir(t: &Target) -> PathBuf {
    t.cache_dir.join("rdm-image").join(&t.sha)
}

/// Runs `scbench image build`, returning the process exit code.
///
/// # Errors
///
/// When the manifest cannot be loaded or has no `[rdm]` table, the checkout
/// cannot be resolved or is dirty, the SCBench environment config cannot be
/// read or parsed, the plan root lies in the graded directory, or a step fails
/// (named in the message); later steps are not spawned.
pub fn build(opts: &ImageOptions, stderr: &mut dyn Write) -> Result<u8, String> {
    let t = load(opts)?;
    let top = t.toplevel.display().to_string();
    let status = capture(
        "cannot read the rdm checkout's status",
        &spec("git", &["-C", &top, "status", "--porcelain"]),
    )?;
    if !status.is_empty() {
        return Err(format!(
            "the rdm checkout {top} has uncommitted changes; the image is built from the \
             working tree, so commit or remove them first"
        ));
    }

    let checkout = plan::scbench_checkout(&t.manifest, &t.cache_dir);
    std::fs::create_dir_all(&t.cache_dir)
        .map_err(|e| format!("cannot create --cache-dir {}: {e}", t.cache_dir.display()))?;
    provision("slop-code-bench", &checkout, &mut |s| {
        Ok(spawn_command(s, Io::Quiet).0)
    })?;
    let env_path = environment_path(&t);
    let env = environment::load(&env_path)?;
    environment::refuse_graded_plan_root(
        "scbench image build",
        &t.rdm.plan_root,
        &env.docker.workdir,
    )?;

    let dockerfile = t.manifest.dir.join(rdm_image::DOCKERFILE);
    let mut docker_build = spec(
        &opts.docker,
        &[
            "build",
            "-f",
            &dockerfile.display().to_string(),
            "--build-arg",
            &format!("RDM_SHA={}", t.sha),
            "--label",
            &format!("{}={}", rdm_image::SHA_LABEL, t.sha),
            "-t",
            &rdm_image::payload_tag(&t.sha),
            &top,
        ],
    );
    docker_build
        .env
        .insert("DOCKER_BUILDKIT".to_owned(), "1".to_owned());
    inherit("docker build", &docker_build)?;

    let source = t.manifest.dir.join(rdm_image::TEMPLATE);
    let template = std::fs::read_to_string(&source)
        .map_err(|e| format!("cannot read agent template {}: {e}", source.display()))?;
    let dir = image_dir(&t);
    let template_path = dir.join("docker.j2");
    let agent_path = dir.join("agent.yaml");
    write_file(
        &template_path,
        rdm_image::render_template(
            &template,
            &t.manifest.agent.claude_code_version,
            &t.sha,
            &t.rdm.plan_root,
        )
        .as_bytes(),
    )?;
    write_json(
        &agent_path,
        &rdm_image::agent_config(
            &t.manifest,
            &t.rdm,
            &t.sha,
            &template_path,
            &t.manifest.agent.limits(),
        ),
    )?;

    let mut build_agent = spec(
        &opts.uv,
        &[
            "run",
            "slop-code",
            "docker",
            "build-agent",
            &agent_path.display().to_string(),
            &env_path.display().to_string(),
            "--force-build",
        ],
    );
    build_agent.cwd = Some(checkout.dir.clone());
    let scbench_home = t.cache_dir.join("scbench-home");
    build_agent.env.insert(
        "SCBENCH_HOME".to_owned(),
        scbench_home.display().to_string(),
    );
    let (ran, _) = spawn_command(&build_agent, Io::Inherit);
    let secret = redact::Secret::for_provider(&t.manifest.agent.provider);
    for root in [&scbench_home, &dir] {
        redact::scrub_tree(root, secret.as_ref(), "scbench image build", stderr);
    }
    if !ran.ok() {
        return Err(format!(
            "slop-code docker build-agent (`{}`) {}",
            build_agent.argv().join(" "),
            ran.describe()
        ));
    }

    let _ = writeln!(stderr, "scbench image build: {}", agent_image(&t, &env));
    Ok(0)
}

fn agent_image(t: &Target, env: &Environment) -> String {
    rdm_image::agent_image_name(&t.manifest.agent.claude_code_version, &t.sha, &env.name)
}

/// The skill names listed on the `Skills (N)  a, b, ...` line of
/// `claude plugin details`, or `None` when there is no such line.
fn listed_skills(listing: &str) -> Option<BTreeSet<String>> {
    let line = listing
        .lines()
        .map(str::trim_start)
        .find(|l| l.starts_with("Skills ("))?;
    let names = &line[line.find(')')? + 1..];
    Some(
        names
            .split(',')
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(str::to_owned)
            .collect(),
    )
}

/// Every `<toplevel>/plugins/rdm/skills/<name>/SKILL.md`'s `<name>`.
fn expected_skills(toplevel: &Path) -> Result<BTreeSet<String>, String> {
    let dir = toplevel.join("plugins/rdm/skills");
    let entries = std::fs::read_dir(&dir)
        .map_err(|e| format!("cannot read the checkout's {}: {e}", dir.display()))?;
    let mut names = BTreeSet::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
        if entry.path().join("SKILL.md").is_file() {
            names.insert(entry.file_name().to_string_lossy().into_owned());
        }
    }
    Ok(names)
}

/// Runs `scbench image check`, returning the process exit code: 0 when every
/// probe passed, 1 (after printing each failure on `stderr`) otherwise.
///
/// # Errors
///
/// When the manifest cannot be loaded or has no `[rdm]` table, the checkout
/// cannot be resolved, `scbench image build` has not provisioned the SCBench
/// checkout yet, the SCBench environment config cannot be read or parsed, or
/// the checkout's `plugins/rdm/skills` directory cannot be read. A failed
/// probe is not an error: it is printed and yields exit code 1.
pub fn check(opts: &ImageOptions, stderr: &mut dyn Write) -> Result<u8, String> {
    let t = load(opts)?;
    let env_path = environment_path(&t);
    if !env_path.is_file() {
        return Err(format!(
            "no SCBench environment at {}; run `rdm-measure scbench image build` first",
            env_path.display()
        ));
    }
    let env = environment::load(&env_path)?;
    let expected = expected_skills(&t.toplevel)?;
    let image = agent_image(&t, &env);
    let probe = |entrypoint: &str, args: &[&str]| {
        let mut argv = vec!["run", "--rm", "--entrypoint", entrypoint, &image];
        argv.extend_from_slice(args);
        spawn_command(&spec(&opts.docker, &argv), Io::Capture).0
    };

    let mut failures = Vec::new();
    let ran = probe("cat", &[rdm_image::SHA_FILE]);
    if !ran.ok() {
        failures.push(format!(
            "could not read {} in {image}: {}",
            rdm_image::SHA_FILE,
            ran.describe()
        ));
    } else if ran.stdout.trim() != t.sha {
        failures.push(format!(
            "image {image} records rdm SHA {}, but the checkout under test is {}",
            ran.stdout.trim(),
            t.sha
        ));
    }

    let ran = probe(rdm_image::RDM_BIN, &["--version"]);
    if !ran.ok() {
        failures.push(format!(
            "rdm --version failed in {image}: {}",
            ran.describe()
        ));
    }

    let ran = probe(
        "claude",
        &[
            "--plugin-dir",
            rdm_image::PLUGIN_DIR,
            "plugin",
            "details",
            "rdm",
        ],
    );
    match listed_skills(&ran.stdout).filter(|_| ran.ok()) {
        None => failures.push(format!("plugin listing shows no rdm skills in {image}")),
        Some(listed) => {
            let missing: Vec<&str> = expected
                .iter()
                .filter(|n| !listed.contains(*n))
                .map(String::as_str)
                .collect();
            if !missing.is_empty() {
                failures.push(format!(
                    "plugin listing lacks rdm skills: {}",
                    missing.join(", ")
                ));
            }
        }
    }

    if failures.is_empty() {
        let _ = writeln!(stderr, "scbench image check: ok ({image}, rdm {})", t.sha);
        return Ok(0);
    }
    for f in &failures {
        let _ = writeln!(stderr, "scbench image check: {f}");
    }
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skills_line_is_parsed() {
        let listing = "rdm@inline\n  Source: rdm@inline\n  Skills (2)  do, land\n";
        let want: BTreeSet<String> = ["do", "land"].map(str::to_owned).into();
        assert_eq!(listed_skills(listing), Some(want));
        assert_eq!(listed_skills("Source: rdm@inline\n"), None);
    }
}
