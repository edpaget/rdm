use assert_cmd::Command;
use predicates::prelude::*;

fn rdm() -> Command {
    let mut cmd = Command::cargo_bin("rdm").unwrap();
    // Isolate from host global config (e.g. default_format = "json").
    cmd.env("XDG_CONFIG_HOME", "/dev/null/nonexistent");
    cmd
}

#[test]
fn serve_help_mentions_port_and_bind() {
    rdm()
        .arg("serve")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("--port"))
        .stdout(predicate::str::contains("--bind"));
}

#[test]
fn help_includes_serve_command() {
    rdm()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("serve"));
}
