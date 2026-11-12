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

/// One arm: a named prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Arm {
    /// The arm name (used as a directory name).
    pub name: String,
    /// The prompt template, relative to the SCBench checkout.
    pub prompt: String,
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

    #[test]
    fn empty_string_field_fails() {
        fails_naming(
            &text().replace("provider = \"claude_code_oauth\"", "provider = \"\""),
            "agent.provider",
        );
    }
}
