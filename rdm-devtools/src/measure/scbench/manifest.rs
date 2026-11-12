//! The pinned SCBench manifest (`bench/scbench/manifest.toml`): every value a
//! result depends on, parsed strictly and validated before anything runs.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// A parsed, validated manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// The pinned SlopCodeBench checkout.
    pub scbench: Pin,
    /// The pinned problem repository and the subset to run.
    pub problems: Problems,
    /// The agent, model and evaluation settings every arm shares.
    pub agent: Agent,
    /// The arms, run in this order.
    #[serde(rename = "arm", default)]
    pub arms: Vec<Arm>,
    /// The rdm-equipped image's plan repo settings; required when an arm
    /// selects `image = "rdm"` and by `scbench image`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rdm: Option<Rdm>,
    /// The directory holding the manifest (relative paths resolve against it).
    #[serde(skip)]
    pub dir: PathBuf,
}

/// A git repository pinned to one commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pin {
    /// The clone URL (or a local path).
    pub repo: String,
    /// The full 40-character commit SHA.
    pub sha: String,
}

/// The pinned problem repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Problems {
    /// The clone URL (or a local path).
    pub repo: String,
    /// The full 40-character commit SHA.
    pub sha: String,
    /// The problem names to run, in order.
    pub subset: Vec<String>,
}

/// Agent, model and evaluation settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Agent {
    /// SCBench provider name (`claude_code_oauth` or `anthropic`).
    pub provider: String,
    /// SCBench model name.
    pub model: String,
    /// The model definition rdm ships, relative to the manifest's directory
    /// unless absolute.
    pub model_file: String,
    /// SCBench thinking preset.
    pub thinking: String,
    /// The Claude Code version installed in the agent image.
    pub claude_code_version: String,
    /// The evaluation pass policy.
    pub pass_policy: String,
    /// The SCBench environment name.
    pub environment: String,
    /// The agent's step limit.
    pub step_limit: u64,
    /// The agent's cost limit (0 = unlimited).
    pub cost_limit: f64,
    /// The agent's net cost limit (0 = unlimited).
    pub net_cost_limit: f64,
}

/// One arm: a named prompt, run in the stock or the rdm-equipped image.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Arm {
    /// The arm name (used as a directory name).
    pub name: String,
    /// The prompt template, relative to the SCBench checkout or, with
    /// [`PromptFrom::Manifest`], to the manifest's directory.
    pub prompt: String,
    /// Where [`Arm::prompt`] lives (default [`PromptFrom::Scbench`]).
    #[serde(default)]
    pub prompt_from: PromptFrom,
    /// The agent image the arm runs in (default [`ArmImage::Stock`]).
    #[serde(default)]
    pub image: ArmImage,
    /// The treatment group the arm belongs to; when absent it follows
    /// [`Arm::image`] (see [`Arm::group`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<ArmGroup>,
    /// Overrides `[agent] step_limit` for this arm.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_limit: Option<u64>,
    /// Overrides `[agent] cost_limit` for this arm (0 = unlimited).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_limit: Option<f64>,
    /// Overrides `[agent] net_cost_limit` for this arm (0 = unlimited).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub net_cost_limit: Option<f64>,
}

/// Where an arm's prompt template lives.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PromptFrom {
    /// In the SCBench checkout, used in place and unrendered.
    #[default]
    Scbench,
    /// rdm's own template beside the manifest: the runner fills its
    /// `@RDM_ROADMAP@`/`@RDM_PROJECT@` placeholders and writes it to
    /// `<out>/<arm>/prompt.jinja` (see [`super::lane::render_prompt`]).
    Manifest,
}

/// The agent limits one arm runs with (SCBench's `cost_limits`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Limits {
    /// The agent's step limit (`--max-turns`).
    pub step_limit: u64,
    /// The agent's cost limit (0 = unlimited).
    pub cost_limit: f64,
    /// The agent's net cost limit (0 = unlimited).
    pub net_cost_limit: f64,
}

impl Agent {
    /// The agent-wide limits every arm inherits.
    pub fn limits(&self) -> Limits {
        Limits {
            step_limit: self.step_limit,
            cost_limit: self.cost_limit,
            net_cost_limit: self.net_cost_limit,
        }
    }
}

impl Arm {
    /// This arm's limits: each one the arm sets, else `agent`'s.
    pub fn limits(&self, agent: &Agent) -> Limits {
        Limits {
            step_limit: self.step_limit.unwrap_or(agent.step_limit),
            cost_limit: self.cost_limit.unwrap_or(agent.cost_limit),
            net_cost_limit: self.net_cost_limit.unwrap_or(agent.net_cost_limit),
        }
    }

    /// This arm's treatment group: the explicit [`Arm::group`], else
    /// [`ArmGroup::Rdm`] for the rdm image and [`ArmGroup::Control`] for the
    /// stock one.
    pub fn group(&self) -> ArmGroup {
        self.group.unwrap_or(match self.image {
            ArmImage::Rdm => ArmGroup::Rdm,
            ArmImage::Stock => ArmGroup::Control,
        })
    }
}

/// The treatment group an arm belongs to, which the ablation comparison keys
/// on. It is distinct from [`ArmImage`]: an rdm-image arm whose prompt never
/// uses rdm is a control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArmGroup {
    /// An rdm treatment: the arm drives its work through rdm.
    Rdm,
    /// A control: the arm does not use rdm.
    Control,
}

/// Which agent image an arm runs in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArmImage {
    /// SCBench's stock `claude_code` image.
    #[default]
    Stock,
    /// The rdm-equipped image (see [`super::rdm_image`]): the rdm binary and
    /// `plugins/rdm` from the checkout under test, and a plan repo.
    Rdm,
}

/// The `[rdm]` table: where the rdm-equipped image keeps its plan repo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rdm {
    /// The plan repo's absolute path inside the agent container; it must lie
    /// outside SCBench's graded directory.
    pub plan_root: String,
    /// The plan repo's default project (`[a-z0-9-]+`).
    pub project: String,
}

impl Manifest {
    /// Reads, parses and validates the manifest at `path`.
    ///
    /// # Errors
    ///
    /// When the file cannot be read, does not parse, or fails validation; the
    /// message names the offending field.
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read manifest {}: {e}", path.display()))?;
        let dir = path
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        let dir = std::path::absolute(&dir).unwrap_or(dir);
        Self::parse(&text, &dir).map_err(|e| format!("invalid manifest {}: {e}", path.display()))
    }

    /// Parses and validates manifest text whose relative paths resolve
    /// against `dir`.
    ///
    /// # Errors
    ///
    /// When the text does not parse or fails validation; the message names
    /// the offending field.
    pub fn parse(text: &str, dir: &Path) -> Result<Self, String> {
        let mut manifest: Self = toml::from_str(text).map_err(|e| e.to_string())?;
        manifest.dir = dir.to_path_buf();
        manifest.validate()?;
        Ok(manifest)
    }

    /// The `[rdm]` table, which `scbench image` requires.
    ///
    /// # Errors
    ///
    /// When the manifest has no `[rdm]` table.
    pub fn rdm_table(&self) -> Result<&Rdm, String> {
        self.rdm
            .as_ref()
            .ok_or_else(|| "rdm: scbench image needs an [rdm] table".to_owned())
    }

    /// The absolute path of `arm`'s prompt when it is a
    /// [`PromptFrom::Manifest`] prompt; `None` for an SCBench prompt.
    pub fn manifest_prompt_source(&self, arm: &Arm) -> Option<PathBuf> {
        (arm.prompt_from == PromptFrom::Manifest).then(|| self.dir.join(&arm.prompt))
    }

    /// The model file's absolute source path.
    pub fn model_file_source(&self) -> PathBuf {
        let p = Path::new(&self.agent.model_file);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.dir.join(p)
        }
    }

    fn validate(&self) -> Result<(), String> {
        let strings = [
            ("scbench.repo", &self.scbench.repo),
            ("scbench.sha", &self.scbench.sha),
            ("problems.repo", &self.problems.repo),
            ("problems.sha", &self.problems.sha),
            ("agent.provider", &self.agent.provider),
            ("agent.model", &self.agent.model),
            ("agent.model_file", &self.agent.model_file),
            ("agent.thinking", &self.agent.thinking),
            ("agent.claude_code_version", &self.agent.claude_code_version),
            ("agent.pass_policy", &self.agent.pass_policy),
            ("agent.environment", &self.agent.environment),
        ];
        for (field, value) in strings {
            if value.trim().is_empty() {
                return Err(format!("{field} must not be empty"));
            }
        }
        for (field, sha) in [
            ("scbench.sha", &self.scbench.sha),
            ("problems.sha", &self.problems.sha),
        ] {
            if !is_full_sha(sha) {
                return Err(format!(
                    "{field} must be 40 lowercase hex characters, got \"{sha}\""
                ));
            }
        }
        if self.problems.subset.is_empty() {
            return Err("problems.subset must name at least one problem".to_owned());
        }
        if self.problems.subset.iter().any(|p| p.trim().is_empty()) {
            return Err("problems.subset must not contain an empty name".to_owned());
        }
        if self.arms.is_empty() {
            return Err("arm: the manifest must define at least one [[arm]]".to_owned());
        }
        let mut seen = BTreeSet::new();
        for arm in &self.arms {
            if arm.name.is_empty()
                || !arm
                    .name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                return Err(format!(
                    "arm.name must match [A-Za-z0-9_-]+, got \"{}\"",
                    arm.name
                ));
            }
            if arm.prompt.trim().is_empty() {
                return Err(format!(
                    "arm.prompt of arm \"{}\" must not be empty",
                    arm.name
                ));
            }
            if !seen.insert(arm.name.as_str()) {
                return Err(format!("arm.name \"{}\" is duplicated", arm.name));
            }
            if arm.image == ArmImage::Rdm && self.rdm.is_none() {
                return Err(format!(
                    "rdm: arm \"{}\" selects image \"rdm\" but the manifest has no [rdm] table",
                    arm.name
                ));
            }
            if arm.group == Some(ArmGroup::Rdm) && arm.image != ArmImage::Rdm {
                return Err(format!(
                    "arm.group of arm \"{}\" is \"rdm\" but the arm runs in the stock image, which has no rdm",
                    arm.name
                ));
            }
            if arm.prompt_from == PromptFrom::Manifest && self.rdm.is_none() {
                return Err(format!(
                    "arm.prompt_from of arm \"{}\" is \"manifest\" but the manifest has no [rdm] table to render it with",
                    arm.name
                ));
            }
        }
        if let Some(rdm) = &self.rdm {
            if !Path::new(&rdm.plan_root).is_absolute() {
                return Err(format!(
                    "rdm.plan_root must be an absolute path, got \"{}\"",
                    rdm.plan_root
                ));
            }
            if rdm.project.is_empty()
                || !rdm
                    .project
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            {
                return Err(format!(
                    "rdm.project must match [a-z0-9-]+, got \"{}\"",
                    rdm.project
                ));
            }
        }
        Ok(())
    }
}

fn is_full_sha(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA_A: &str = "af5a9cbc311605b5cceb80200ca9832bdff2d7b1";
    const SHA_B: &str = "9cd9ca3a51c3d3e2a99d2488a25baf73a2204451";

    fn text() -> String {
        format!(
            r#"
[scbench]
repo = "https://example.invalid/scbench"
sha = "{SHA_A}"

[problems]
repo = "https://example.invalid/problems"
sha = "{SHA_B}"
subset = ["mvvault", "xjq"]

[agent]
provider = "claude_code_oauth"
model = "opus-5.5"
model_file = "models/opus-5.5.yaml"
thinking = "xhigh"
claude_code_version = "2.1.322"
pass_policy = "all-cases"
environment = "docker-python3.12-uv"
step_limit = 100
cost_limit = 0
net_cost_limit = 0

[[arm]]
name = "just-solve"
prompt = "configs/prompts/just-solve.jinja"

[[arm]]
name = "anti_slop"
prompt = "configs/prompts/anti_slop.jinja"
"#
        )
    }

    fn parse(t: &str) -> Result<Manifest, String> {
        Manifest::parse(t, Path::new("/m"))
    }

    fn fails_naming(t: &str, field: &str) {
        let e = parse(t).expect_err("must fail");
        assert!(e.contains(field), "error {e:?} should name {field:?}");
    }

    #[test]
    fn valid_manifest_parses_and_resolves_model_file() {
        let m = parse(&text()).expect("valid");
        assert_eq!(m.arms.len(), 2);
        assert_eq!(m.arms[1].name, "anti_slop");
        assert_eq!(m.agent.step_limit, 100);
        assert_eq!(
            m.model_file_source(),
            PathBuf::from("/m/models/opus-5.5.yaml")
        );
    }

    #[test]
    fn absolute_model_file_is_kept() {
        let t = text().replace("\"models/opus-5.5.yaml\"", "\"/abs/x.yaml\"");
        let m = parse(&t).expect("valid");
        assert_eq!(m.model_file_source(), PathBuf::from("/abs/x.yaml"));
    }

    #[test]
    fn non_hex_sha_fails() {
        let bad = SHA_A.replace('a', "g");
        fails_naming(&text().replace(SHA_A, &bad), "scbench.sha");
    }

    #[test]
    fn short_sha_fails() {
        fails_naming(&text().replace(SHA_B, &SHA_B[..39]), "problems.sha");
    }

    #[test]
    fn empty_subset_fails() {
        fails_naming(
            &text().replace(r#"["mvvault", "xjq"]"#, "[]"),
            "problems.subset",
        );
    }

    #[test]
    fn no_arm_fails() {
        let t = text();
        let cut = t.find("[[arm]]").expect("has arms");
        fails_naming(&t[..cut], "arm");
    }

    #[test]
    fn duplicate_arm_fails() {
        fails_naming(
            &text().replace("name = \"anti_slop\"", "name = \"just-solve\""),
            "arm.name",
        );
    }

    #[test]
    fn arm_name_with_slash_fails() {
        fails_naming(
            &text().replace("name = \"anti_slop\"", "name = \"a/b\""),
            "arm.name",
        );
    }

    #[test]
    fn unknown_key_fails() {
        fails_naming(
            &text().replace("thinking = \"xhigh\"", "thinking = \"xhigh\"\nseed = 1"),
            "seed",
        );
    }

    const RDM: &str = "\n[rdm]\nplan_root = \"/rdm-plan\"\nproject = \"bench\"\n";

    fn with_rdm_arm(t: &str) -> String {
        format!("{t}\n[[arm]]\nname = \"r\"\nprompt = \"p.jinja\"\nimage = \"rdm\"\n")
    }

    #[test]
    fn arm_without_image_is_stock() {
        let m = parse(&text()).expect("valid");
        assert!(m.arms.iter().all(|a| a.image == ArmImage::Stock));
        assert_eq!(m.rdm, None);
    }

    #[test]
    fn rdm_arm_with_rdm_table_parses() {
        let m = parse(&with_rdm_arm(&format!("{RDM}{}", text()))).expect("valid");
        assert_eq!(m.arms[2].image, ArmImage::Rdm);
        let rdm = m.rdm_table().expect("rdm table");
        assert_eq!(rdm.plan_root, "/rdm-plan");
        assert_eq!(rdm.project, "bench");
    }

    #[test]
    fn unknown_image_fails() {
        fails_naming(
            &text().replace(
                "name = \"anti_slop\"",
                "name = \"anti_slop\"\nimage = \"bogus\"",
            ),
            "image",
        );
    }

    #[test]
    fn rdm_arm_without_rdm_table_fails() {
        fails_naming(&with_rdm_arm(&text()), "rdm");
        let m = parse(&text()).expect("valid");
        assert!(m.rdm_table().expect_err("no table").contains("[rdm]"));
    }

    #[test]
    fn relative_plan_root_fails() {
        fails_naming(
            &format!("{}{}", RDM.replace("/rdm-plan", "rdm-plan"), text()),
            "rdm.plan_root",
        );
    }

    #[test]
    fn empty_rdm_project_fails() {
        fails_naming(
            &format!("{}{}", RDM.replace("\"bench\"", "\"\""), text()),
            "rdm.project",
        );
    }

    #[test]
    fn arm_without_prompt_from_is_scbench() {
        let m = parse(&text()).expect("valid");
        assert!(m.arms.iter().all(|a| a.prompt_from == PromptFrom::Scbench));
        assert_eq!(m.manifest_prompt_source(&m.arms[0]), None);
    }

    fn with_manifest_prompt_arm(t: &str, extra: &str) -> String {
        format!(
            "{t}\n[[arm]]\nname = \"lane\"\nprompt = \"prompts/l.jinja\"\nprompt_from = \"manifest\"\n{extra}"
        )
    }

    #[test]
    fn manifest_prompt_resolves_against_manifest_dir() {
        let m = parse(&with_manifest_prompt_arm(&format!("{RDM}{}", text()), "")).expect("valid");
        assert_eq!(m.arms[2].prompt_from, PromptFrom::Manifest);
        assert_eq!(
            m.manifest_prompt_source(&m.arms[2]),
            Some(PathBuf::from("/m/prompts/l.jinja"))
        );
    }

    #[test]
    fn manifest_prompt_without_rdm_table_fails() {
        fails_naming(&with_manifest_prompt_arm(&text(), ""), "prompt_from");
    }

    #[test]
    fn arm_without_limits_inherits_agent_limits() {
        let m = parse(&text()).expect("valid");
        assert_eq!(m.arms[0].limits(&m.agent), m.agent.limits());
        assert_eq!(m.agent.limits().step_limit, 100);
    }

    #[test]
    fn arm_setting_only_step_limit_keeps_agent_cost_limits() {
        let t = text().replace("cost_limit = 0\nnet", "cost_limit = 3.5\nnet");
        let t = with_manifest_prompt_arm(&format!("{RDM}{t}"), "step_limit = 1000\n");
        let m = parse(&t).expect("valid");
        assert_eq!(
            m.arms[2].limits(&m.agent),
            Limits {
                step_limit: 1000,
                cost_limit: 3.5,
                net_cost_limit: 0.0,
            }
        );
    }

    #[test]
    fn arm_overriding_every_limit_uses_its_own() {
        let t = with_manifest_prompt_arm(
            &format!("{RDM}{}", text()),
            "step_limit = 4321\ncost_limit = 12.5\nnet_cost_limit = 25\n",
        );
        let m = parse(&t).expect("valid");
        assert_eq!(
            m.arms[2].limits(&m.agent),
            Limits {
                step_limit: 4321,
                cost_limit: 12.5,
                net_cost_limit: 25.0,
            }
        );
    }

    #[test]
    fn arm_without_group_derives_it_from_image() {
        let m = parse(&with_rdm_arm(&format!("{RDM}{}", text()))).expect("valid");
        assert!(m.arms.iter().all(|a| a.group.is_none()));
        assert_eq!(m.arms[0].group(), ArmGroup::Control);
        assert_eq!(m.arms[1].group(), ArmGroup::Control);
        assert_eq!(m.arms[2].group(), ArmGroup::Rdm);
    }

    #[test]
    fn explicit_control_group_on_rdm_image_is_kept() {
        let t = format!(
            "{}group = \"control\"\n",
            with_rdm_arm(&format!("{RDM}{}", text()))
        );
        let m = parse(&t).expect("valid");
        assert_eq!(m.arms[2].image, ArmImage::Rdm);
        assert_eq!(m.arms[2].group, Some(ArmGroup::Control));
        assert_eq!(m.arms[2].group(), ArmGroup::Control);
    }

    #[test]
    fn rdm_group_on_stock_image_fails() {
        fails_naming(
            &text().replace(
                "name = \"anti_slop\"",
                "name = \"anti_slop\"\ngroup = \"rdm\"",
            ),
            "arm.group",
        );
    }

    #[test]
    fn unknown_group_fails() {
        fails_naming(
            &text().replace(
                "name = \"anti_slop\"",
                "name = \"anti_slop\"\ngroup = \"bogus\"",
            ),
            "group",
        );
    }

    #[test]
    fn empty_string_field_fails() {
        fails_naming(
            &text().replace("provider = \"claude_code_oauth\"", "provider = \"\""),
            "agent.provider",
        );
    }
}
