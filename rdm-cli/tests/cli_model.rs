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
    for (step, tier, model, effort) in [
        ("plan", "medium", "opus", "medium"),
        ("implement", "medium", "opus", "medium"),
        ("review-find", "medium", "opus", "medium"),
        ("review-verify", "large", "opus", "high"),
        ("mechanical", "small", "opus", "low"),
        ("review-consolidate", "large", "opus", "high"),
    ] {
        assert_eq!(
            resolve_json(&dir, &[step]),
            serde_json::json!({
                "step": step, "host": "claude", "tier": tier, "model": model, "effort": effort
            })
        );
    }
}

#[test]
fn resolve_json_reports_effective_review_floor() {
    let dir = TempDir::new().unwrap();
    assert_eq!(
        resolve_json(&dir, &["review-find", "--tier", "small"]),
        serde_json::json!({
            "step": "review-find", "host": "claude", "tier": "medium", "model": "opus", "effort": "medium"
        })
    );
    write_review_floor_override(&dir);
    assert_eq!(
        resolve_json(&dir, &["review-find", "--tier", "small"]),
        serde_json::json!({
            "step": "review-find", "host": "claude", "tier": "large", "model": "opus", "effort": "high"
        })
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
        serde_json::json!({
            "step": "implement", "host": "claude", "tier": "large", "model": "custom-large", "effort": "high"
        })
    );
    assert_eq!(
        resolve_json(&dir, &["implement", "--tier", "small"]),
        serde_json::json!({
            "step": "implement", "host": "claude", "tier": "small", "model": "custom-small", "effort": "low"
        })
    );
    // Legacy keys never touch the codex host.
    assert_eq!(
        resolve_json(&dir, &["implement", "--tier", "small", "--host", "codex"]),
        serde_json::json!({
            "step": "implement", "host": "codex", "tier": "small", "model": "gpt-6-sol", "effort": "medium"
        })
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
            .stdout("opus\n");
    }
}

#[test]
fn resolve_review_find_hint_small_prints_opus() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["model", "resolve", "review-find", "--tier", "small"])
        .assert()
        .success()
        .stdout("opus\n");
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
            predicate::str::contains("host: claude")
                .and(predicate::str::contains("small: opus @ low"))
                .and(predicate::str::contains("medium: opus @ medium"))
                .and(predicate::str::contains("large: opus @ high"))
                .and(predicate::str::contains("frontier: opus @ xhigh"))
                .and(predicate::str::contains("review_floor: medium"))
                .and(predicate::str::contains("plan: opus @ medium"))
                .and(predicate::str::contains("review-verify: opus @ high"))
                .and(predicate::str::contains("mechanical: opus @ low"))
                .and(predicate::str::contains("review-consolidate: opus @ high")),
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
    assert_eq!(value["host"], "claude");
    assert_eq!(value["small"], "opus");
    assert_eq!(value["medium"], "opus");
    assert_eq!(value["large"], "opus");
    assert_eq!(value["frontier"], "opus");
    assert_eq!(value["review_floor"], "medium");
    assert_eq!(
        value["profiles"],
        serde_json::json!({
            "small": {"model": "opus", "effort": "low"},
            "medium": {"model": "opus", "effort": "medium"},
            "large": {"model": "opus", "effort": "high"},
            "frontier": {"model": "opus", "effort": "xhigh"},
        })
    );
    let steps = value["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 6);
    let review_verify = steps.iter().find(|s| s["step"] == "review-verify").unwrap();
    assert_eq!(review_verify["model"], "opus");
    assert_eq!(review_verify["tier"], "large");
    assert_eq!(review_verify["effort"], "high");
    let review_consolidate = steps
        .iter()
        .find(|s| s["step"] == "review-consolidate")
        .unwrap();
    assert_eq!(review_consolidate["model"], "opus");
    assert_eq!(review_consolidate["tier"], "large");
    assert_eq!(review_consolidate["effort"], "high");
}

#[test]
fn show_json_codex_host() {
    let dir = TempDir::new().unwrap();
    let assert = rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["model", "show", "--host", "codex", "--format", "json"])
        .assert()
        .success();
    let value: serde_json::Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(value["host"], "codex");
    assert_eq!(value["small"], "gpt-6-sol");
    assert_eq!(value["large"], "gpt-6-astra");
    assert_eq!(
        value["profiles"]["frontier"],
        serde_json::json!({"model": "gpt-6-astra", "effort": "xhigh"})
    );
}

fn valid_efforts(host: &str) -> &'static [&'static str] {
    match host {
        "claude" => &["low", "medium", "high", "xhigh", "max"],
        "codex" => &["low", "medium", "high", "xhigh"],
        other => panic!("unknown host {other}"),
    }
}

#[test]
fn resolve_json_covers_every_step_and_tier() {
    let dir = TempDir::new().unwrap();
    for host in ["claude", "codex"] {
        for step in [
            "plan",
            "implement",
            "review-find",
            "review-verify",
            "review-consolidate",
            "mechanical",
        ] {
            for tier in [
                None,
                Some("small"),
                Some("medium"),
                Some("large"),
                Some("frontier"),
            ] {
                let mut args = vec![step, "--host", host];
                if let Some(t) = tier {
                    args.extend(["--tier", t]);
                }
                let v = resolve_json(&dir, &args);
                assert_eq!(v["step"], step);
                assert_eq!(v["host"], host);
                assert!(v["model"].as_str().is_some_and(|m| !m.is_empty()), "{v}");
                assert!(v["tier"].is_string(), "{v}");
                let effort = v["effort"].as_str().expect("effort present");
                assert!(
                    valid_efforts(host).contains(&effort),
                    "{host} {step} {tier:?}: effort {effort} not valid"
                );
                if tier.is_none() {
                    assert_ne!(v["tier"], "frontier", "no default reaches frontier: {v}");
                }
            }
        }
    }
}

#[test]
fn resolve_json_frontier_hint_resolves_frontier_profile() {
    let dir = TempDir::new().unwrap();
    assert_eq!(
        resolve_json(&dir, &["implement", "--tier", "frontier"]),
        serde_json::json!({
            "step": "implement", "host": "claude", "tier": "frontier", "model": "opus", "effort": "xhigh"
        })
    );
    assert_eq!(
        resolve_json(
            &dir,
            &["implement", "--tier", "frontier", "--host", "codex"]
        ),
        serde_json::json!({
            "step": "implement", "host": "codex", "tier": "frontier", "model": "gpt-6-astra", "effort": "xhigh"
        })
    );
}

#[test]
fn resolve_text_output_is_bare_model_id() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["model", "resolve", "plan"])
        .assert()
        .success()
        .stdout("opus\n");
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["model", "resolve", "review-verify", "--host", "codex"])
        .assert()
        .success()
        .stdout("gpt-6-astra\n");
}

#[test]
fn resolve_invalid_host_errors() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["model", "resolve", "plan", "--host", "gemini"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "invalid host: 'gemini' (expected claude or codex)",
        ));
}

#[test]
fn resolve_rejects_invalid_effort_config() {
    let dir = TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("rdm.toml"),
        "[models.profiles.claude.small]\neffort = \"ultra\"\n",
    )
    .unwrap();
    let assert = rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["model", "resolve", "plan"])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_string();
    assert!(stderr.contains("ultra"), "{stderr}");
    for valid in ["low", "medium", "high", "xhigh", "max"] {
        assert!(stderr.contains(&format!("`{valid}`")), "{stderr}");
    }
}

#[test]
fn resolve_rejects_invalid_effort_in_global_config() {
    let dir = TempDir::new().unwrap();
    let xdg = TempDir::new().unwrap();
    std::fs::create_dir_all(xdg.path().join("rdm")).unwrap();
    std::fs::write(
        xdg.path().join("rdm").join("config.toml"),
        "[models.profiles.codex.large]\neffort = \"max\"\n",
    )
    .unwrap();
    rdm()
        .env("XDG_CONFIG_HOME", xdg.path())
        .arg("--root")
        .arg(dir.path())
        .args(["model", "resolve", "plan", "--host", "codex"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("gpt").not())
        .stderr(
            predicate::str::contains("models.profiles.codex.large.effort")
                .and(predicate::str::contains("low, medium, high, xhigh")),
        );
}

#[test]
fn resolve_rejects_codex_max_effort_config() {
    let dir = TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("rdm.toml"),
        "[models.profiles.codex.large]\neffort = \"max\"\n",
    )
    .unwrap();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["model", "resolve", "plan", "--host", "codex"])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("models.profiles.codex.large.effort")
                .and(predicate::str::contains("low, medium, high, xhigh")),
        );
}

#[test]
fn resolve_honors_configured_profile() {
    let dir = TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("rdm.toml"),
        "[models.profiles.codex.medium]\nmodel = \"gpt-6-astra\"\neffort = \"low\"\n[models.profiles.claude.large]\neffort = \"max\"\n",
    )
    .unwrap();
    assert_eq!(
        resolve_json(&dir, &["implement", "--host", "codex"]),
        serde_json::json!({
            "step": "implement", "host": "codex", "tier": "medium", "model": "gpt-6-astra", "effort": "low"
        })
    );
    assert_eq!(
        resolve_json(&dir, &["review-verify"]),
        serde_json::json!({
            "step": "review-verify", "host": "claude", "tier": "large", "model": "opus", "effort": "max"
        })
    );
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
