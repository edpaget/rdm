use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn rdm() -> Command {
    Command::cargo_bin("rdm").unwrap()
}

fn create_phase(dir: &TempDir, slug: &str, title: &str) {
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            slug,
            "--title",
            title,
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success();
}

fn init_with_roadmap(dir: &TempDir) {
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("init")
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "fbm"])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "roadmap",
            "create",
            "two-way",
            "--title",
            "Two-Way Players",
            "--project",
            "fbm",
        ])
        .assert()
        .success();
}

#[test]
fn phase_create_auto_number() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            "core",
            "--title",
            "Core Valuation",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Created phase 'phase-1-core'"));

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            "service",
            "--title",
            "Keeper Service",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Created phase 'phase-2-service'"));
}

#[test]
fn phase_create_explicit_number() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            "core",
            "--title",
            "Core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--number",
            "5",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("phase-5-core"));
}

#[test]
fn phase_show() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            "core",
            "--title",
            "Core Valuation",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Phase 1: Core Valuation")
                .and(predicate::str::contains("Status: not-started")),
        );
}

#[test]
fn phase_update_to_done() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            "core",
            "--title",
            "Core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--status",
            "done",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Updated 'phase-1-core' → done"));

    // Verify completed date is set
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Status: done").and(predicate::str::contains("Completed:")),
        );
}

#[test]
fn phase_update_done_then_back() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            "core",
            "--title",
            "Core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--status",
            "done",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--status",
            "in-progress",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("in-progress"));

    // Verify completed date is cleared
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Status: in-progress")
                .and(predicate::str::contains("Completed:").not()),
        );
}

#[test]
fn phase_list() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");
    create_phase(&dir, "service", "Keeper Service");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["phase", "list", "--roadmap", "two-way", "--project", "fbm"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("| # | Phase | Status | Stem |")
                .and(predicate::str::contains(
                    "| 1 | Core Valuation | not-started | phase-1-core |",
                ))
                .and(predicate::str::contains(
                    "| 2 | Keeper Service | not-started | phase-2-service |",
                )),
        );
}

#[test]
fn phase_list_empty() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["phase", "list", "--roadmap", "two-way", "--project", "fbm"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No phases yet."));
}

#[test]
fn phase_show_by_number() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "1",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Phase 1: Core Valuation")
                .and(predicate::str::contains("Stem: phase-1-core")),
        );
}

#[test]
fn phase_show_by_number_not_found() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "99",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("phase not found: 99"));
}

#[test]
fn phase_reorder_basic() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "alpha", "Alpha");
    create_phase(&dir, "beta", "Beta");
    create_phase(&dir, "gamma", "Gamma");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "reorder",
            "phase-3-gamma",
            "--number",
            "1",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Reordered 'phase-3-gamma' to position 1",
        ));

    // Verify with phase list
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["phase", "list", "--roadmap", "two-way", "--project", "fbm"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("| 1 | Gamma")
                .and(predicate::str::contains("| 2 | Alpha"))
                .and(predicate::str::contains("| 3 | Beta")),
        );
}

#[test]
fn phase_reorder_by_number() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "alpha", "Alpha");
    create_phase(&dir, "beta", "Beta");
    create_phase(&dir, "gamma", "Gamma");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "reorder",
            "3",
            "--number",
            "1",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Reordered 'phase-3-gamma' to position 1",
        ));
}

#[test]
fn phase_update_by_number() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "1",
            "--status",
            "done",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Updated 'phase-1-core' → done"));
}

// -- phase move tests --

fn init_with_two_roadmaps(dir: &TempDir) {
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("init")
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "fbm"])
        .assert()
        .success();
    for (slug, title) in [("src", "Source"), ("dst", "Destination")] {
        rdm()
            .arg("--root")
            .arg(dir.path())
            .args([
                "roadmap",
                "create",
                slug,
                "--title",
                title,
                "--project",
                "fbm",
            ])
            .assert()
            .success();
    }
    for (slug, title, roadmap) in [
        ("alpha", "Alpha", "src"),
        ("beta", "Beta", "src"),
        ("gamma", "Gamma", "src"),
        ("delta", "Delta", "dst"),
        ("epsilon", "Epsilon", "dst"),
    ] {
        rdm()
            .arg("--root")
            .arg(dir.path())
            .args([
                "phase",
                "create",
                slug,
                "--title",
                title,
                "--roadmap",
                roadmap,
                "--project",
                "fbm",
            ])
            .assert()
            .success();
    }
}

#[test]
fn phase_move_basic() {
    let dir = TempDir::new().unwrap();
    init_with_two_roadmaps(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "move",
            "phase-2-beta",
            "--from",
            "src",
            "--to",
            "dst",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Moved 'phase-2-beta' from 'src' to 'dst'",
        ));

    // Verify source
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["phase", "list", "--roadmap", "src", "--project", "fbm"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("| 1 | Alpha")
                .and(predicate::str::contains("| 2 | Gamma"))
                .and(predicate::str::contains("Beta").not()),
        );

    // Verify destination
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["phase", "list", "--roadmap", "dst", "--project", "fbm"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("| 1 | Delta")
                .and(predicate::str::contains("| 2 | Epsilon"))
                .and(predicate::str::contains("| 3 | Beta")),
        );
}

#[test]
fn phase_move_with_position() {
    let dir = TempDir::new().unwrap();
    init_with_two_roadmaps(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "move",
            "phase-1-alpha",
            "--from",
            "src",
            "--to",
            "dst",
            "--number",
            "1",
            "--project",
            "fbm",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["phase", "list", "--roadmap", "dst", "--project", "fbm"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("| 1 | Alpha")
                .and(predicate::str::contains("| 2 | Delta"))
                .and(predicate::str::contains("| 3 | Epsilon")),
        );
}

#[test]
fn phase_move_by_number() {
    let dir = TempDir::new().unwrap();
    init_with_two_roadmaps(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "move",
            "2",
            "--from",
            "src",
            "--to",
            "dst",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Moved 'phase-2-beta' from 'src' to 'dst'",
        ));
}

#[test]
fn phase_move_same_roadmap_error() {
    let dir = TempDir::new().unwrap();
    init_with_two_roadmaps(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "move",
            "phase-1-alpha",
            "--from",
            "src",
            "--to",
            "src",
            "--project",
            "fbm",
        ])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("source and destination roadmaps are the same")
                .and(predicate::str::contains("rdm phase reorder")),
        );
}
