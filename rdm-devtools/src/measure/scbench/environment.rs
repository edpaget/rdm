//! The SCBench environment an arm runs in
//! (`<scbench>/configs/environments/<name>.yaml`), read for the one value rdm
//! needs: the graded directory, `docker.workdir`.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The fields of an SCBench environment config rdm reads; others are ignored.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Environment {
    /// The environment's name (the suffix of SCBench's agent image name).
    pub name: String,
    /// Its Docker settings.
    #[serde(default)]
    pub docker: Docker,
}

/// An environment's `docker` table.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Docker {
    /// The container directory SCBench snapshots and grades
    /// (`DockerConfig.workdir`, default `/workspace`).
    #[serde(default = "default_workdir")]
    pub workdir: String,
}

impl Default for Docker {
    fn default() -> Self {
        Self {
            workdir: default_workdir(),
        }
    }
}

fn default_workdir() -> String {
    "/workspace".to_owned()
}

/// The config file for environment `name` in the SCBench checkout `scbench`.
pub fn path(scbench: &Path, name: &str) -> PathBuf {
    scbench
        .join("configs/environments")
        .join(format!("{name}.yaml"))
}

/// Reads and parses the environment config at `path`.
///
/// # Errors
///
/// When the file cannot be read or does not parse.
pub fn load(path: &Path) -> Result<Environment, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read SCBench environment {}: {e}", path.display()))?;
    serde_saphyr::from_str(&text)
        .map_err(|e| format!("invalid SCBench environment {}: {e}", path.display()))
}

/// Refuses a plan root inside the graded directory, so plan-repo files never
/// count toward SCBench's metrics. `who` prefixes the message.
///
/// # Errors
///
/// When `plan_root` is `graded_dir` or lies under it.
pub fn refuse_graded_plan_root(who: &str, plan_root: &str, graded_dir: &str) -> Result<(), String> {
    if Path::new(plan_root).starts_with(graded_dir) {
        return Err(format!(
            "{who}: rdm.plan_root {plan_root} is inside SCBench's graded directory {graded_dir}"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workdir_defaults_and_unknown_fields_are_ignored() {
        let e: Environment =
            serde_saphyr::from_str("type: docker\nname: python3.12\nsetup:\n  x: [1]\n")
                .expect("parses");
        assert_eq!(e.name, "python3.12");
        assert_eq!(e.docker.workdir, "/workspace");
    }

    #[test]
    fn plan_root_under_workdir_is_refused() {
        assert!(refuse_graded_plan_root("arm a", "/rdm-plan", "/workspace").is_ok());
        assert!(refuse_graded_plan_root("arm a", "/workspace-x", "/workspace").is_ok());
        let e =
            refuse_graded_plan_root("arm a", "/workspace/.rdm", "/workspace").expect_err("inside");
        assert!(e.contains("arm a") && e.contains("/workspace"), "{e}");
    }
}
