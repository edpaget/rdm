use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn rdm() -> Command {
    let mut cmd = Command::cargo_bin("rdm").unwrap();
    // Isolate from host global config (e.g. default_format = "json").
    cmd.env("XDG_CONFIG_HOME", "/dev/null/nonexistent");
    cmd
}

fn init_with_project(dir: &TempDir) {
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("init")
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "acme"])
        .assert()
        .success();
}

fn create_roadmap(dir: &TempDir, slug: &str, title: &str) {
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
            "acme",
            "--no-edit",
        ])
        .assert()
        .success();
}

fn create_phase(dir: &TempDir, roadmap: &str, slug: &str, title: &str, number: &str) {
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
            "acme",
            "--number",
            number,
            "--no-edit",
        ])
        .assert()
        .success();
}

fn create_task(dir: &TempDir, slug: &str, title: &str) {
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "create",
            slug,
            "--title",
            title,
            "--project",
            "acme",
            "--no-edit",
        ])
        .assert()
        .success();
}

fn setup_test_data(dir: &TempDir) {
    init_with_project(dir);
    create_roadmap(dir, "widget-launch", "Widget Launch");
    create_phase(dir, "widget-launch", "design", "Design the Widget", "1");
    create_phase(
        dir,
        "widget-launch",
        "implementation",
        "Implement the Widget",
        "2",
    );
    // Mark phase 1 as done
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
            "widget-launch",
            "--project",
            "acme",
            "--no-edit",
        ])
        .assert()
        .success();
    create_task(dir, "fix-login-bug", "Fix Login Bug");
    create_task(dir, "add-search", "Add Search Feature");
    // Mark one task as done
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "update",
            "fix-login-bug",
            "--status",
            "done",
            "--project",
            "acme",
            "--no-edit",
        ])
        .assert()
        .success();
}

#[test]
fn search_basic_title_match() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["search", "Fix Login Bug"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Fix Login Bug"));
}

#[test]
fn search_fuzzy_match() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["search", "fx logn bg"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Fix Login Bug"));
}

#[test]
fn search_filter_by_type_task() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["search", "widget", "--type", "task"])
        .assert()
        .success()
        .stdout(predicate::str::contains("roadmap").not());
}

#[test]
fn search_filter_by_status() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["search", "bug", "--type", "task", "--status", "done"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Fix Login Bug"));
}

#[test]
fn search_ambiguous_status_without_type_matches_task() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    // Move the add-search task to needs-review (a status shared by both kinds).
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "update",
            "add-search",
            "--status",
            "needs-review",
            "--project",
            "acme",
            "--no-edit",
        ])
        .assert()
        .success();

    // With no --type, the needs-review filter must still surface the task.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["search", "", "--status", "needs-review"])
        .assert()
        .success()
        .stdout(predicate::str::contains("add-search"));
}

#[test]
fn search_filter_by_project() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    // Create a second project with a task
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "other"])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "create",
            "other-task",
            "--title",
            "Other Task",
            "--project",
            "other",
            "--no-edit",
        ])
        .assert()
        .success();

    // Search only acme project
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["search", "task", "--project", "acme"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Other Task").not());
}

#[test]
fn search_limit() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    let output = rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["search", "widget", "--limit", "1"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();
    // Count data rows (lines starting with "| " followed by a digit)
    let data_rows = stdout
        .lines()
        .filter(|l| l.starts_with("| ") && l.chars().nth(2).is_some_and(|c| c.is_ascii_digit()))
        .count();
    assert_eq!(data_rows, 1, "Expected exactly 1 result row, got: {stdout}");
}

#[test]
fn search_json_output() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    let output = rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["search", "Fix Login", "--format", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("should be valid JSON");
    let arr = parsed.as_array().expect("should be an array");
    assert!(!arr.is_empty(), "Expected results in JSON output");
    let first = &arr[0];
    assert!(first.get("kind").is_some());
    assert!(first.get("title").is_some());
    assert!(first.get("identifier").is_some());
    assert!(first.get("score").is_some());
    assert!(first.get("snippet").is_some());
}

#[test]
fn search_no_results() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["search", "xyzzy-nonexistent-qqq"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No results found"));
}

#[test]
fn search_min_score_ratio_zero_keeps_all() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    // With ratio 0, even low-scoring results are kept
    let output_no_cutoff = rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["search", "w", "--min-score-ratio", "0", "--format", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let all: serde_json::Value = serde_json::from_slice(&output_no_cutoff).expect("valid JSON");
    let all_count = all.as_array().unwrap().len();

    // With strict ratio, fewer results
    let output_strict = rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "search",
            "w",
            "--min-score-ratio",
            "0.9",
            "--format",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let strict: serde_json::Value = serde_json::from_slice(&output_strict).expect("valid JSON");
    let strict_count = strict.as_array().unwrap().len();

    assert!(
        all_count >= strict_count,
        "Zero cutoff ({all_count}) should keep at least as many results as strict cutoff ({strict_count})"
    );
}

#[test]
fn search_ambiguous_status_both_phase_and_task_without_type() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    // Move phase 2 (implementation) to needs-review
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "2",
            "--status",
            "needs-review",
            "--roadmap",
            "widget-launch",
            "--project",
            "acme",
            "--no-edit",
        ])
        .assert()
        .success();

    // Move add-search task to needs-review
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "update",
            "add-search",
            "--status",
            "needs-review",
            "--project",
            "acme",
            "--no-edit",
        ])
        .assert()
        .success();

    // Search with no --type flag; must return both phase and task
    let output = rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["search", "", "--status", "needs-review"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();
    assert!(
        stdout.contains("widget-launch/phase-2-implementation"),
        "Expected phase identifier in output: {stdout}"
    );
    assert!(
        stdout.contains("add-search"),
        "Expected task slug in output: {stdout}"
    );

    // Verify --type task narrows to only the task
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["search", "", "--status", "needs-review", "--type", "task"])
        .assert()
        .success()
        .stdout(predicate::str::contains("add-search"))
        .stdout(predicate::str::contains("widget-launch/phase-2-implementation").not());

    // Verify --type phase narrows to only the phase
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["search", "", "--status", "needs-review", "--type", "phase"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "widget-launch/phase-2-implementation",
        ))
        .stdout(predicate::str::contains("add-search").not());
}

#[test]
fn search_help() {
    rdm()
        .args(["search", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Search across roadmaps"));
}

#[test]
fn search_type_review_matches_summary_and_comment_bodies() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);
    create_task(&dir, "reviewed-task", "Reviewed Task");

    // Start a review whose summary and comment carry distinctive tokens.
    let out = rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "review",
            "start",
            "--on",
            "task/reviewed-task",
            "--author",
            "tester",
            "--no-edit",
            "--project",
            "acme",
            "--body",
            "Summary mentions zanzibar.",
            "--format",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let id = serde_json::from_slice::<serde_json::Value>(&out).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "review",
            "comment",
            &id,
            "--body",
            "Comment mentions quokka.",
            "--no-edit",
            "--project",
            "acme",
        ])
        .assert()
        .success();

    // Summary text matches.
    let out = rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "search",
            "zanzibar",
            "--type",
            "review",
            "--project",
            "acme",
            "--format",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let results: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let arr = results.as_array().unwrap();
    assert_eq!(arr.len(), 1, "summary text must match: {results}");
    assert_eq!(arr[0]["kind"], "review");
    assert_eq!(arr[0]["identifier"], id.as_str());
    assert_eq!(arr[0]["title"], "review of task/reviewed-task");

    // Comment body matches too.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["search", "quokka", "--type", "review", "--project", "acme"])
        .assert()
        .success()
        .stdout(predicate::str::contains(id.as_str()));

    // --type review with --status is rejected with a pointer to review list.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "search",
            "zanzibar",
            "--type",
            "review",
            "--status",
            "open",
            "--project",
            "acme",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("rdm review list --state"));
}

fn create_tagged_task(dir: &TempDir, slug: &str, title: &str, tags: &str) {
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "create",
            slug,
            "--title",
            title,
            "--project",
            "acme",
            "--tags",
            tags,
            "--no-edit",
        ])
        .assert()
        .success();
}

#[test]
fn search_filters_by_tag() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);
    create_tagged_task(&dir, "login-bug", "Login is broken", "bug");
    create_task(&dir, "login-polish", "Login polish");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["search", "login", "--project", "acme", "--tag", "bug"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("login-bug")
                .and(predicate::str::contains("login-polish").not()),
        );
}

#[test]
fn search_empty_query_with_tag_lists_tagged_items() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);
    create_tagged_task(&dir, "login-bug", "Login is broken", "bug");
    create_task(&dir, "login-polish", "Login polish");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["search", "", "--project", "acme", "--tag", "bug"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("login-bug")
                .and(predicate::str::contains("login-polish").not()),
        );
}

#[test]
fn search_multi_tag_is_and() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);
    create_tagged_task(&dir, "login-both", "Login both", "bug,ui");
    create_tagged_task(&dir, "login-bug-only", "Login bug only", "bug");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "search",
            "login",
            "--project",
            "acme",
            "--tag",
            "bug",
            "--tag",
            "ui",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("login-both")
                .and(predicate::str::contains("login-bug-only").not()),
        );
}

#[test]
fn search_tag_filter_matches_roadmaps_and_phases() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "roadmap",
            "create",
            "login-rm",
            "--title",
            "Login roadmap",
            "--project",
            "acme",
            "--tags",
            "bug",
            "--no-edit",
        ])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            "login-phase",
            "--title",
            "Login phase",
            "--roadmap",
            "login-rm",
            "--project",
            "acme",
            "--number",
            "1",
            "--tags",
            "bug",
            "--no-edit",
        ])
        .assert()
        .success();
    create_roadmap(&dir, "login-plain-rm", "Login plain roadmap");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["search", "login", "--project", "acme", "--tag", "bug"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("login-rm")
                .and(predicate::str::contains("login-phase"))
                .and(predicate::str::contains("login-plain-rm").not()),
        );
}

#[test]
fn search_tag_filter_with_no_match_prints_empty_state() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);
    create_tagged_task(&dir, "login-bug", "Login is broken", "bug");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["search", "login", "--project", "acme", "--tag", "nope"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No results found"));
}

fn search_json(dir: &TempDir, args: &[&str]) -> Vec<serde_json::Value> {
    let out = rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("search")
        .args(args)
        .args(["--project", "acme", "--format", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice::<serde_json::Value>(&out)
        .unwrap()
        .as_array()
        .unwrap()
        .clone()
}

fn kinds_of(hits: &[serde_json::Value]) -> Vec<String> {
    hits.iter()
        .map(|h| h["kind"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn search_type_accepts_a_comma_list_and_repeats() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    let hits = search_json(
        &dir,
        &[
            "widget",
            "--type",
            "roadmap,phase",
            "--min-score-ratio",
            "0",
        ],
    );
    let kinds = kinds_of(&hits);
    assert!(kinds.contains(&"roadmap".to_string()), "{kinds:?}");
    assert!(kinds.contains(&"phase".to_string()), "{kinds:?}");
    assert!(
        kinds.iter().all(|k| k == "roadmap" || k == "phase"),
        "{kinds:?}"
    );

    let repeated = search_json(
        &dir,
        &[
            "widget",
            "--type",
            "roadmap",
            "--type",
            "phase",
            "--min-score-ratio",
            "0",
        ],
    );
    assert_eq!(kinds_of(&repeated), kinds);

    let items = search_json(
        &dir,
        &[
            "i",
            "--type",
            "roadmap,phase,task",
            "--min-score-ratio",
            "0",
        ],
    );
    let kinds = kinds_of(&items);
    assert!(kinds.contains(&"task".to_string()), "{kinds:?}");
    assert!(kinds.iter().all(|k| k != "review"), "{kinds:?}");
}

#[test]
fn search_json_hits_carry_status_structured_ids_and_refs() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    let hits = search_json(&dir, &["widget", "--min-score-ratio", "0"]);
    let phase = hits
        .iter()
        .find(|h| h["identifier"] == "widget-launch/phase-1-design")
        .expect("phase hit");
    assert_eq!(phase["status"], "done");
    assert_eq!(phase["roadmap"], "widget-launch");
    assert_eq!(phase["stem"], "phase-1-design");
    assert!(phase.get("slug").is_none());
    assert_eq!(phase["ref"], "widget-launch/phase-1-design");
    assert_eq!(phase["review_ref"], "phase/widget-launch/phase-1-design");

    let roadmap = hits
        .iter()
        .find(|h| h["kind"] == "roadmap")
        .expect("roadmap hit");
    assert_eq!(roadmap["roadmap"], "widget-launch");
    assert!(roadmap.get("status").is_none());
    assert_eq!(roadmap["ref"], "widget-launch");

    let hits = search_json(&dir, &["Fix Login", "--type", "task"]);
    assert_eq!(hits[0]["slug"], "fix-login-bug");
    assert_eq!(hits[0]["status"], "done");
    assert_eq!(hits[0]["ref"], "task/fix-login-bug");
    assert_eq!(hits[0]["review_ref"], "task/fix-login-bug");
    // The identifier stays for existing consumers.
    assert_eq!(hits[0]["identifier"], "fix-login-bug");
}

#[test]
fn search_multi_type_is_applied_before_the_limit() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);
    // A task that matches "quokka" only as a scattered fuzzy subsequence…
    create_task(&dir, "outpost-audit", "Quiet outpost kitchen kettle audit");
    // …and a review whose summary is the query itself, so it outranks it.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "review",
            "start",
            "--on",
            "task/outpost-audit",
            "--author",
            "tester",
            "--no-edit",
            "--project",
            "acme",
            "--body",
            "quokka",
        ])
        .assert()
        .success();

    let top = search_json(&dir, &["quokka", "--limit", "1"]);
    assert_eq!(
        kinds_of(&top),
        ["review"],
        "precondition: the review ranks first"
    );

    let top = search_json(
        &dir,
        &["quokka", "--type", "roadmap,phase,task", "--limit", "1"],
    );
    assert_eq!(top.len(), 1, "{top:?}");
    assert_eq!(top[0]["kind"], "task");
    assert_eq!(top[0]["slug"], "outpost-audit");
}
