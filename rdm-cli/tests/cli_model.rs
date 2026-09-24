use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn rdm() -> Command {
    let mut cmd = Command::cargo_bin("rdm").unwrap();
    // Isolate from host global config (e.g. default_format = "json").
    cmd.env("XDG_CONFIG_HOME", "/dev/null/nonexistent");
    cmd
}

fn write_review_floor_override(dir: &TempDir) {
    std::fs::write(
        dir.path().join("rdm.toml"),
        "[models]\nreview_floor = \"large\"\n",
    )
    .unwrap();
}

fn resolve_json(dir: &TempDir, args: &[&str]) -> serde_json::Value {
    let assert = rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["model", "resolve"])
        .args(args)
        .args(["--format", "json"])
        .assert()
        .success();
    serde_json::from_slice(&assert.get_output().stdout).unwrap()
}

#[test]
fn resolve_json_exposes_default_step_tier_and_model() {
    let dir = TempDir::new().unwrap();
    for (step, tier, model) in [
        ("plan", "medium", "sonnet"),
        ("implement", "medium", "sonnet"),
        ("review-find", "medium", "sonnet"),
        ("review-verify", "large", "opus"),
        ("mechanical", "small", "opus"),
    ] {
        assert_eq!(
            resolve_json(&dir, &[step]),
            serde_json::json!({"step": step, "tier": tier, "model": model})
        );
    }
}

#[test]
fn resolve_json_reports_effective_review_floor() {
    let dir = TempDir::new().unwrap();
    assert_eq!(
        resolve_json(&dir, &["review-find", "--tier", "small"]),
        serde_json::json!({"step": "review-find", "tier": "medium", "model": "sonnet"})
    );
    write_review_floor_override(&dir);
    assert_eq!(
        resolve_json(&dir, &["review-find", "--tier", "small"]),
        serde_json::json!({"step": "review-find", "tier": "large", "model": "opus"})
    );
}

#[test]
fn resolve_json_honors_configured_step_and_model_with_hint_precedence() {
    let dir = TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("rdm.toml"),
        "[models]\nsmall = \"custom-small\"\nlarge = \"custom-large\"\n[models.steps]\nimplement = \"large\"\n",
    )
    .unwrap();
    assert_eq!(
        resolve_json(&dir, &["implement"]),
        serde_json::json!({"step": "implement", "tier": "large", "model": "custom-large"})
    );
    assert_eq!(
        resolve_json(&dir, &["implement", "--tier", "small"]),
        serde_json::json!({"step": "implement", "tier": "small", "model": "custom-small"})
    );
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["model", "resolve", "implement"])
        .assert()
        .success()
        .stdout("custom-large\n");
}

#[test]
fn resolve_non_json_formats_preserve_plain_model_output() {
    let dir = TempDir::new().unwrap();
    for format in ["human", "markdown", "table"] {
        rdm()
            .arg("--root")
            .arg(dir.path())
            .args(["model", "resolve", "review-find", "--format", format])
            .assert()
            .success()
            .stdout("sonnet\n");
    }
}

#[test]
fn resolve_review_find_hint_small_prints_sonnet() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["model", "resolve", "review-find", "--tier", "small"])
        .assert()
        .success()
        .stdout("sonnet\n");
}

#[test]
fn resolve_review_find_hint_large_prints_opus() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["model", "resolve", "review-find", "--tier", "large"])
        .assert()
        .success()
        .stdout("opus\n");
}

#[test]
fn resolve_mechanical_no_hint_prints_opus() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["model", "resolve", "mechanical"])
        .assert()
        .success()
        .stdout("opus\n");
}

#[test]
fn resolve_review_verify_no_hint_prints_opus() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["model", "resolve", "review-verify"])
        .assert()
        .success()
        .stdout("opus\n");
}

#[test]
fn resolve_honors_repo_review_floor_override() {
    let dir = TempDir::new().unwrap();
    write_review_floor_override(&dir);
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["model", "resolve", "review-find"])
        .assert()
        .success()
        .stdout("opus\n");
}

#[test]
fn resolve_invalid_step_errors_once() {
    let dir = TempDir::new().unwrap();
    let assert = rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["model", "resolve", "bogus-step"])
        .assert()
        .failure();
    let output = assert.get_output();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("invalid dispatch step: 'bogus-step'"),
        "stderr should mention the invalid step, got: {stderr}"
    );
    let occurrences = stderr.matches("invalid dispatch step").count();
    assert_eq!(
        occurrences, 1,
        "error message should not be doubled, got: {stderr}"
    );
}

#[test]
fn resolve_invalid_tier_errors_once() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["model", "resolve", "plan", "--tier", "bogus-tier"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid model tier: 'bogus-tier'"));
}

#[test]
fn resolve_both_invalid_reports_step_error_only() {
    // `step` is parsed before `tier`, so when both are invalid the step error
    // is the one surfaced (documented precedence in commands/model.rs).
    let dir = TempDir::new().unwrap();
    let assert = rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["model", "resolve", "bogus-step", "--tier", "bogus-tier"])
        .assert()
        .failure();
    let output = assert.get_output();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("invalid dispatch step: 'bogus-step'"),
        "step error should win when both step and tier are invalid, got: {stderr}"
    );
    assert!(
        !stderr.contains("invalid model tier"),
        "tier is never parsed once step fails, got: {stderr}"
    );
}

#[test]
fn resolve_step_is_case_sensitive() {
    // Dispatch-step tokens are matched exactly; a capitalized variant is rejected.
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["model", "resolve", "Review-Find"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "invalid dispatch step: 'Review-Find'",
        ));
}

#[test]
fn show_human_lists_bindings_floor_and_steps() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["model", "show"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("small: opus")
                .and(predicate::str::contains("medium: sonnet"))
                .and(predicate::str::contains("large: opus"))
                .and(predicate::str::contains("review_floor: medium"))
                .and(predicate::str::contains("review-verify: opus"))
                .and(predicate::str::contains("mechanical: opus")),
        );
}

#[test]
fn show_json_is_structured() {
    let dir = TempDir::new().unwrap();
    let assert = rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["model", "show", "--format", "json"])
        .assert()
        .success();
    let output = assert.get_output();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["small"], "opus");
    assert_eq!(value["review_floor"], "medium");
    let steps = value["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 5);
    let review_verify = steps.iter().find(|s| s["step"] == "review-verify").unwrap();
    assert_eq!(review_verify["model"], "opus");
}

#[test]
fn show_table_rejected() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["model", "show", "--format", "table"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--format table is not supported for 'model show'",
        ));
}

#[test]
fn show_json_reflects_repo_override() {
    let dir = TempDir::new().unwrap();
    write_review_floor_override(&dir);
    let assert = rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["model", "show", "--format", "json"])
        .assert()
        .success();
    let output = assert.get_output();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["review_floor"], "large");
    let steps = value["steps"].as_array().unwrap();
    let review_find = steps.iter().find(|s| s["step"] == "review-find").unwrap();
    assert_eq!(review_find["model"], "opus");
}
