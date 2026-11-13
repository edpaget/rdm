//! The rdm-equipped agent image: its names, the rendered SCBench agent
//! template and the agent config an rdm arm runs with. Pure; no I/O.
//!
//! The image is two layers. The payload image (`bench/scbench/agent/Dockerfile`,
//! context = the rdm checkout under test) holds `/opt/rdm`: the Linux `rdm`
//! binary, `plugins/rdm`, the [`WRAPPER`], the [`PLAN_ROOT_SETUP`] script and the
//! [`SHA_FILE`]. SCBench's
//! agent image is then built from `bench/scbench/agent/docker.j2`, which
//! copies `/opt/rdm` out of the payload with `COPY --from`. Everything here is
//! a `ClaudeCodeConfig` field (`docker_template`, `version`, `binary`,
//! `extra_args`, `env`), so SCBench runs it unmodified.

use std::path::Path;

use serde_json::{Value, json};

use super::lane;
use super::manifest::{Arm, Limits, Manifest, Rdm};
use super::plan::stock_agent_config;

/// The payload image's repository; its tag is the rdm SHA.
pub const PAYLOAD_REPO: &str = "rdm-scbench-payload";
/// The label the payload image records the rdm SHA under.
pub const SHA_LABEL: &str = "org.rdm.scbench.rdm-sha";
/// The file in the agent image holding the rdm SHA (labels do not survive
/// `COPY --from`).
pub const SHA_FILE: &str = "/opt/rdm/RDM_SHA";
/// The `rdm` binary in the agent image.
pub const RDM_BIN: &str = "/opt/rdm/bin/rdm";
/// The checkout's `plugins/rdm`, as installed in the agent image and loaded
/// with `--plugin-dir` (an install under `~/.claude` is hidden at run time).
pub const PLUGIN_DIR: &str = "/opt/rdm/plugin";
/// The agent binary: initializes the plan repo, then runs `claude`.
pub const WRAPPER: &str = "/opt/rdm/bin/claude-rdm";
/// Run as root at agent-image build time: makes the plan root writable by any
/// uid and trusted by git for any uid (a system `safe.directory`).
pub const PLAN_ROOT_SETUP: &str = "/opt/rdm/bin/rdm-plan-root-setup";
/// The payload Dockerfile, relative to the manifest's directory.
pub const DOCKERFILE: &str = "agent/Dockerfile";
/// The unrendered agent template, relative to the manifest's directory.
pub const TEMPLATE: &str = "agent/docker.j2";
/// Where the wrapper exports the plan repo, relative to the container's
/// `$HOME/.claude`. It lies under SCBench's `_WORKSPACE_PROJECT`
/// (`projects/-workspace`), whose new files `traces.copy_new_trace_files`
/// copies into each checkpoint's `agent/workspace/` after the checkpoint; the
/// runner reads the exports back from there (`lane::collect`).
pub const PLAN_EXPORT_SUBDIR: &str = "projects/-workspace/rdm-plan";
/// Stands in for the rdm SHA where it is not yet resolved (`--dry-run`).
pub const SHA_PLACEHOLDER: &str = "<rdm-sha>";

/// SCBench's image-name prefix (`docker_runtime.IMAGE_NAME_PREFIX`).
const IMAGE_NAME_PREFIX: &str = "slop-code";

/// The rdm agent config's `version`: `<claude_code_version>-rdm.<sha>`. It
/// names SCBench's image, so a stock arm and each rdm commit get their own.
pub fn agent_version(claude_code_version: &str, sha: &str) -> String {
    format!("{claude_code_version}-rdm.{sha}")
}

/// The payload image's tag, `rdm-scbench-payload:<sha>`.
pub fn payload_tag(sha: &str) -> String {
    format!("{PAYLOAD_REPO}:{sha}")
}

/// The agent image SCBench builds and runs for an rdm arm, mirroring
/// `AgentConfigBase.get_image`: `slop-code:claude_code-<version>-<env name>`.
pub fn agent_image_name(claude_code_version: &str, sha: &str, env_name: &str) -> String {
    format!(
        "{IMAGE_NAME_PREFIX}:claude_code-{}-{env_name}",
        agent_version(claude_code_version, sha)
    )
}

/// Fills `docker.j2`'s `@CLAUDE_CODE_VERSION@`, `@RDM_PAYLOAD_IMAGE@` and
/// `@RDM_PLAN_ROOT@`, leaving `{{ base_image }}` for SCBench's Jinja.
pub fn render_template(
    template: &str,
    claude_code_version: &str,
    sha: &str,
    plan_root: &str,
) -> String {
    template
        .replace("@CLAUDE_CODE_VERSION@", claude_code_version)
        .replace("@RDM_PAYLOAD_IMAGE@", &payload_tag(sha))
        .replace("@RDM_PLAN_ROOT@", plan_root)
}

/// The agent config for an rdm arm: the stock config (with `limits`) with
/// the wrapper as `binary`, the per-SHA `version`, the rendered template at
/// `template_path`, `--plugin-dir` and the plan repo's environment, including
/// the [`PLAN_EXPORT_SUBDIR`] the wrapper exports the plan repo to and
/// `RDM_ROADMAP` ([`lane::ROADMAP`], the roadmap the prompts' `@RDM_ROADMAP@`
/// name), whose highest-numbered phase the wrapper reads to decide whether a
/// checkpoint has settled or must be resumed. rdm itself reads no
/// `RDM_ROADMAP`.
///
/// `RDM_SCBENCH_VERIFY` carries `arm`'s effective verify ([`Arm::verify`]:
/// its `config.dispatch.verify`, else `[rdm] verify`), which the wrapper sets
/// as the plan repo's `dispatch.verify` on its first call. `RDM_SCBENCH_CONFIG`
/// carries the arm's other config overrides ([`Arm::config_lines`]), which
/// the wrapper applies with `rdm config set` on that same first call; it is
/// left out when there are none. Neither is rdm's own `RDM_<KEY>` variable,
/// which would override the plan repo's config invisibly and never land in
/// its `rdm.toml`. `arm` is `None` for the image smoke check, which runs no
/// arm.
pub fn agent_config(
    manifest: &Manifest,
    rdm: &Rdm,
    sha: &str,
    template_path: &Path,
    limits: &Limits,
    arm: Option<&Arm>,
) -> Value {
    let mut config = stock_agent_config(&manifest.agent, limits);
    config["binary"] = json!(WRAPPER);
    config["version"] = json!(agent_version(&manifest.agent.claude_code_version, sha));
    config["docker_template"] = json!(template_path.display().to_string());
    config["extra_args"] = json!(["--plugin-dir", PLUGIN_DIR]);
    let verify = arm.and_then(|a| a.verify(Some(rdm))).unwrap_or(&rdm.verify);
    config["env"] = json!({
        "RDM_ROOT": rdm.plan_root,
        "RDM_PROJECT": rdm.project,
        "RDM_ROADMAP": lane::ROADMAP,
        "RDM_SCBENCH_VERIFY": verify,
        "RDM_PLAN_EXPORT_SUBDIR": PLAN_EXPORT_SUBDIR,
        "GIT_AUTHOR_NAME": "rdm-scbench",
        "GIT_AUTHOR_EMAIL": "rdm-scbench@localhost",
        "GIT_COMMITTER_NAME": "rdm-scbench",
        "GIT_COMMITTER_EMAIL": "rdm-scbench@localhost",
    });
    if let Some(lines) = arm.and_then(Arm::config_lines) {
        config["env"]["RDM_SCBENCH_CONFIG"] = json!(lines);
    }
    config
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_image_name_carries_version_sha_and_env() {
        assert_eq!(
            agent_image_name("2.1.322", "S", "python3.12"),
            "slop-code:claude_code-2.1.322-rdm.S-python3.12"
        );
    }

    #[test]
    fn agent_config_env_names_the_lane_roadmap() {
        let text = r#"
[scbench]
repo = "R1"
sha = "1111111111111111111111111111111111111111"
[problems]
repo = "R2"
sha = "2222222222222222222222222222222222222222"
subset = ["p1"]
checkpoints = { p1 = 2 }
[agent]
provider = "prov"
model = "mod"
model_file = "models/m.yaml"
thinking = "low"
claude_code_version = "9.9.9"
pass_policy = "core-cases"
environment = "env"
step_limit = 7
cost_limit = 0
net_cost_limit = 0
[rdm]
plan_root = "/rdm-plan"
project = "bench"
verify = "pytest -q && ruff check"
[[arm]]
name = "a1"
prompt = "configs/prompts/a1.jinja"
"#;
        let manifest = Manifest::parse(text, Path::new("/rdm/bench")).expect("valid");
        let rdm = manifest.rdm_table().expect("rdm table");
        let config = agent_config(
            &manifest,
            rdm,
            "S",
            Path::new("/t/docker.j2"),
            &manifest.agent.limits(),
            Some(&manifest.arms[0]),
        );
        assert_eq!(config["env"]["RDM_ROADMAP"], json!(lane::ROADMAP));
        assert_eq!(config["env"]["RDM_PROJECT"], json!("bench"));
        assert_eq!(
            config["env"]["RDM_SCBENCH_VERIFY"],
            json!("pytest -q && ruff check")
        );
    }

    #[test]
    fn agent_config_env_carries_each_arms_config_overrides() {
        let text = r#"
[scbench]
repo = "R1"
sha = "1111111111111111111111111111111111111111"
[problems]
repo = "R2"
sha = "2222222222222222222222222222222222222222"
subset = ["p1"]
checkpoints = { p1 = 2 }
[agent]
provider = "prov"
model = "mod"
model_file = "models/m.yaml"
thinking = "low"
claude_code_version = "9.9.9"
pass_policy = "core-cases"
environment = "env"
step_limit = 7
cost_limit = 0
net_cost_limit = 0
[rdm]
plan_root = "/rdm-plan"
project = "bench"
verify = "pytest -q"
[[arm]]
name = "full-lane"
prompt = "prompts/full-lane.jinja"
prompt_from = "manifest"
image = "rdm"
[[arm]]
base = "full-lane"
sweep = { setting = "config.max_refutations", values = [0] }
[[arm]]
name = "gated"
base = "full-lane"
config.dispatch.verify = "pytest -x"
"#;
        let manifest = Manifest::parse(text, Path::new("/rdm/bench")).expect("valid");
        let rdm = manifest.rdm_table().expect("rdm table");
        let env = |name: &str| {
            let arm = manifest.arms.iter().find(|a| a.name == name).expect("arm");
            agent_config(
                &manifest,
                rdm,
                "S",
                Path::new("/t/docker.j2"),
                &arm.limits(&manifest.agent),
                Some(arm),
            )["env"]
                .clone()
        };
        let base = env("full-lane");
        assert!(base.get("RDM_SCBENCH_CONFIG").is_none(), "{base}");
        assert_eq!(base["RDM_SCBENCH_VERIFY"], json!("pytest -q"));
        let swept = env("full-lane.max_refutations-0");
        assert_eq!(swept["RDM_SCBENCH_CONFIG"], json!("max_refutations=0"));
        assert_eq!(swept["RDM_SCBENCH_VERIFY"], json!("pytest -q"));
        let gated = env("gated");
        assert_eq!(gated["RDM_SCBENCH_VERIFY"], json!("pytest -x"));
        assert!(gated.get("RDM_SCBENCH_CONFIG").is_none(), "{gated}");
    }

    #[test]
    fn render_template_fills_placeholders_and_keeps_base_image() {
        let t = "FROM {{ base_image }}\nRUN npm i x@@CLAUDE_CODE_VERSION@\n\
                 COPY --from=@RDM_PAYLOAD_IMAGE@ /opt/rdm /opt/rdm\n\
                 RUN /opt/rdm/bin/rdm-plan-root-setup @RDM_PLAN_ROOT@\n";
        assert_eq!(
            render_template(t, "2.1.322", "S", "/rdm-plan"),
            "FROM {{ base_image }}\nRUN npm i x@2.1.322\n\
             COPY --from=rdm-scbench-payload:S /opt/rdm /opt/rdm\n\
             RUN /opt/rdm/bin/rdm-plan-root-setup /rdm-plan\n"
        );
    }
}
