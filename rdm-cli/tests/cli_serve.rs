use assert_cmd::Command;
use predicates::prelude::*;

fn rdm() -> Command {
    Command::cargo_bin("rdm").unwrap()
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
