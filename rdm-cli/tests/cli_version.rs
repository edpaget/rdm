//! `rdm --version` / `rdm -V` print the crate version and need no plan repo.

use assert_cmd::Command;
use tempfile::TempDir;

fn rdm_in_bare_dir(dir: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("rdm").unwrap();
    cmd.current_dir(dir.path());
    cmd.env("XDG_CONFIG_HOME", "/dev/null/nonexistent");
    cmd.env_remove("RDM_ROOT");
    cmd.env_remove("RDM_PROJECT");
    cmd.env_remove("RDM_FORMAT");
    cmd
}

#[test]
fn version_flags_print_crate_version_without_a_plan_repo() {
    let dir = TempDir::new().unwrap();
    let expected = format!("rdm {}\n", env!("CARGO_PKG_VERSION"));

    for flag in ["--version", "-V"] {
        rdm_in_bare_dir(&dir)
            .arg(flag)
            .assert()
            .success()
            .stdout(expected.clone());
    }
}
