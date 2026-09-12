//! Integration tests for `rdm link check`/`list`/`resolve` and `rdm
//! backlinks` against a temp plan repo (and, for path-verification cases, a
//! separate temp source-repo checkout) — following `cli_review.rs`'s
//! `init_plan_repo`/`rdm`/`git` fixture pattern.

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use std::path::Path;
use tempfile::TempDir;

fn rdm() -> Command {
    let mut cmd = Command::cargo_bin("rdm").unwrap();
    cmd.env("XDG_CONFIG_HOME", "/dev/null/nonexistent");
    cmd
}

fn git(dir: &Path, args: &[&str]) -> std::process::Output {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@test.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@test.com")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

/// A plan repo with a `demo` project and no items yet.
fn init_plan_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("init")
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "demo"])
        .assert()
        .success();
    dir
}

fn create_task(plan: &Path, slug: &str, title: &str, body: &str) {
    rdm()
        .arg("--root")
        .arg(plan)
        .args([
            "task",
            "create",
            slug,
            "--title",
            title,
            "--body",
            body,
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .success();
}

/// Hand-edits `projects/<project>/project.md`'s frontmatter to add a
/// `source: { repo: <repo> }` block — there is no CLI command to set a
/// project's `source` yet, so path-verification fixtures write the file
/// directly, the same way other fixtures in this crate reach past the CLI
/// for setup the product surface doesn't yet expose.
fn set_project_source(plan: &Path, project: &str, repo: &str) {
    let path = plan.join("projects").join(project).join("project.md");
    let content = std::fs::read_to_string(&path).unwrap();
    let rest = content.strip_prefix("---\n").expect("frontmatter open");
    let end = rest.find("\n---").expect("frontmatter close");
    let (frontmatter, tail) = rest.split_at(end);
    let new_content = format!("---\n{frontmatter}\nsource:\n  repo: \"{repo}\"{tail}");
    std::fs::write(&path, new_content).unwrap();
}

fn json_stdout(cmd: &mut Command) -> Value {
    let output = cmd.assert().get_output().stdout.clone();
    serde_json::from_slice(&output).unwrap_or_else(|e| {
        panic!(
            "failed to parse JSON stdout: {e}\nstdout: {}",
            String::from_utf8_lossy(&output)
        )
    })
}

// --- AC1: `link check` names the source document, link text, and missing target ---

#[test]
fn check_reports_dangling_link_with_source_document_link_text_and_target() {
    let plan = init_plan_repo();
    create_task(
        plan.path(),
        "referrer",
        "Referrer",
        "See [ghost](rdm:task/does-not-exist) for background.",
    );

    rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["link", "check", "--project", "demo"])
        .assert()
        .failure()
        .code(1)
        .stdout(predicate::str::contains("task/referrer"))
        .stdout(predicate::str::contains("rdm:task/does-not-exist"))
        .stdout(predicate::str::contains("task/does-not-exist"));
}

// --- `link check` reports malformed `rdm:` destinations as diagnostics,
// distinct from dangling item links and missing-at-rev findings, and the
// diagnostic alone drives a nonzero exit in both text and JSON output ---

#[test]
fn check_reports_parse_diagnostic_distinct_from_dangling_and_missing_at_rev() {
    let plan = init_plan_repo();
    create_task(
        plan.path(),
        "malformed",
        "Malformed",
        "See [bad](rdm:foo/bar) for background.",
    );

    // Run outside any git checkout so path verification is skipped and the
    // exit code depends purely on the parse diagnostic.
    let nongit = TempDir::new().unwrap();

    rdm()
        .arg("--root")
        .arg(plan.path())
        .current_dir(nongit.path())
        .args(["link", "check", "--project", "demo"])
        .assert()
        .failure()
        .code(1)
        .stdout(predicate::str::contains("task/malformed"))
        .stdout(predicate::str::contains("rdm:foo/bar"))
        .stdout(predicate::str::contains("parse error"));

    let json = json_stdout(
        rdm()
            .arg("--root")
            .arg(plan.path())
            .current_dir(nongit.path())
            .args(["link", "check", "--project", "demo", "--format", "json"]),
    );
    let diagnostics = json["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1, "expected one diagnostic: {json}");
    assert_eq!(diagnostics[0]["uri"], "rdm:foo/bar");
    assert!(
        diagnostics[0]["error"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "expected a non-empty parse error message: {json}"
    );
    // No dangling item link and nothing missing at rev — the diagnostic is
    // the sole broken finding.
    assert_eq!(json["dangling"], serde_json::json!([]));
    assert_eq!(json["missing_at_rev"], serde_json::json!([]));
}

// --- AC2: `--on` scopes to one document; clean documents exit 0 ---

#[test]
fn check_on_scopes_to_one_document_and_clean_document_exits_zero() {
    let plan = init_plan_repo();
    create_task(plan.path(), "clean", "Clean", "No links here.");
    create_task(
        plan.path(),
        "dirty",
        "Dirty",
        "See [ghost](rdm:task/does-not-exist).",
    );

    // Scoped to the clean document: exits 0 with a summary line, even
    // though the project has an unrelated dangling link elsewhere.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["link", "check", "--on", "task/clean", "--project", "demo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("task/clean"))
        .stdout(predicate::str::contains("resolve cleanly"));

    // JSON mode: dangling/diagnostics empty for the scoped document.
    let json = json_stdout(rdm().arg("--root").arg(plan.path()).args([
        "link",
        "check",
        "--on",
        "task/clean",
        "--project",
        "demo",
        "--format",
        "json",
    ]));
    assert_eq!(json["on"], "task/clean");
    assert_eq!(json["dangling"], serde_json::json!([]));
    assert_eq!(json["diagnostics"], serde_json::json!([]));

    // Scoped to the dirty document: exits nonzero.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["link", "check", "--on", "task/dirty", "--project", "demo"])
        .assert()
        .failure()
        .code(1);

    // Unscoped, with nothing broken: exits 0 project-wide too.
    let plan2 = init_plan_repo();
    create_task(plan2.path(), "clean-a", "Clean A", "No links.");
    create_task(plan2.path(), "clean-b", "Clean B", "Also no links.");
    rdm()
        .arg("--root")
        .arg(plan2.path())
        .args(["link", "check", "--project", "demo"])
        .assert()
        .success();
}

// --- AC3: `link list` lists outgoing links resolved; empty case ---

#[test]
fn list_resolves_outgoing_links_and_handles_empty_document() {
    let plan = init_plan_repo();
    create_task(plan.path(), "fix-login", "Fix login", "The fix.");
    create_task(
        plan.path(),
        "referrer",
        "Referrer",
        "See [fix](rdm:task/fix-login) and [ghost](rdm:task/also-missing).",
    );
    create_task(plan.path(), "no-links", "No links", "Nothing here.");

    let json = json_stdout(rdm().arg("--root").arg(plan.path()).args([
        "link",
        "list",
        "--on",
        "task/referrer",
        "--project",
        "demo",
        "--format",
        "json",
    ]));
    let arr = json.as_array().expect("array");
    assert_eq!(arr.len(), 2);
    assert!(
        arr.iter()
            .any(|e| e["uri"] == "rdm:task/fix-login" && e["exists"] == true)
    );
    assert!(
        arr.iter()
            .any(|e| e["uri"] == "rdm:task/also-missing" && e["exists"] == false)
    );

    // Same populated document, default (text) format: a summary line plus
    // one rendered `format_resolved` line per link, showing the resolved
    // path and exists/missing state — not just the empty-list branch.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["link", "list", "--on", "task/referrer", "--project", "demo"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "task/referrer: 2 outgoing link(s)",
        ))
        .stdout(predicate::str::contains(
            "rdm:task/fix-login -> projects/demo/tasks/fix-login.md (exists)",
        ))
        .stdout(predicate::str::contains(
            "rdm:task/also-missing -> projects/demo/tasks/also-missing.md (missing)",
        ));

    // Empty document: JSON `[]`, text summary line, both exit 0.
    let empty_json = json_stdout(rdm().arg("--root").arg(plan.path()).args([
        "link",
        "list",
        "--on",
        "task/no-links",
        "--project",
        "demo",
        "--format",
        "json",
    ]));
    assert_eq!(empty_json, serde_json::json!([]));

    rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["link", "list", "--on", "task/no-links", "--project", "demo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("no outgoing links"));
}

// --- AC4: `link resolve` emits the documented JSON shape; item exists flag ---

#[test]
fn resolve_emits_documented_json_shape() {
    let plan = init_plan_repo();

    let json = json_stdout(rdm().arg("--root").arg(plan.path()).args([
        "link",
        "resolve",
        "rdm:src/a/b.rs@abc#L18",
        "--project",
        "demo",
        "--format",
        "json",
    ]));
    assert_eq!(
        json,
        serde_json::json!({
            "kind": "code",
            "path": "a/b.rs",
            "rev": "abc",
            "line": 18
        })
    );
}

// --- `resolve`'s JSON also populates `end_line`/`web_url` for a line-range
// code link when the project has a `source` configured ---

#[test]
fn resolve_reports_end_line_and_web_url_for_a_line_range_when_source_configured() {
    let plan = init_plan_repo();
    set_project_source(plan.path(), "demo", "https://github.com/acme/widgets");

    let json = json_stdout(rdm().arg("--root").arg(plan.path()).args([
        "link",
        "resolve",
        "rdm:src/a/b.rs@abc#L5-L12",
        "--project",
        "demo",
        "--format",
        "json",
    ]));
    assert_eq!(
        json,
        serde_json::json!({
            "kind": "code",
            "path": "a/b.rs",
            "rev": "abc",
            "line": 5,
            "end_line": 12,
            "web_url": "https://github.com/acme/widgets/blob/abc/a/b.rs#L5-L12"
        })
    );
}

// --- `resolve`/`list`'s default (text) format renders both an item and a
// code link via `format_resolved` — not just their JSON shapes ---

#[test]
fn resolve_default_format_renders_item_and_code_links_as_text() {
    let plan = init_plan_repo();
    create_task(plan.path(), "present", "Present", "Body.");
    set_project_source(plan.path(), "demo", "https://github.com/acme/widgets");

    rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["link", "resolve", "rdm:task/present", "--project", "demo"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "rdm:task/present -> projects/demo/tasks/present.md (exists)",
        ));

    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "link",
            "resolve",
            "rdm:src/a/b.rs@abc#L5-L12",
            "--project",
            "demo",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "rdm:src/a/b.rs@abc#L5-L12 -> a/b.rs@abc#L5-L12 (https://github.com/acme/widgets/blob/abc/a/b.rs#L5-L12)",
        ));
}

#[test]
fn resolve_item_uris_report_exists_flag_and_never_error_on_dangling() {
    let plan = init_plan_repo();
    create_task(plan.path(), "present", "Present", "Body.");

    let existing = json_stdout(rdm().arg("--root").arg(plan.path()).args([
        "link",
        "resolve",
        "rdm:task/present",
        "--project",
        "demo",
        "--format",
        "json",
    ]));
    assert_eq!(existing["kind"], "item");
    assert_eq!(existing["exists"], true);
    assert_eq!(existing["path"], "projects/demo/tasks/present.md");

    // A dangling item resolves successfully with exists: false, never an
    // error — asserted via a successful (exit 0) process.
    let missing = json_stdout(rdm().arg("--root").arg(plan.path()).args([
        "link",
        "resolve",
        "rdm:task/missing",
        "--project",
        "demo",
        "--format",
        "json",
    ]));
    assert_eq!(missing["kind"], "item");
    assert_eq!(missing["exists"], false);
}

// --- AC5: `rdm backlinks` lists referencing documents in text and json ---

#[test]
fn backlinks_lists_referencing_documents_and_handles_empty_result() {
    let plan = init_plan_repo();
    create_task(plan.path(), "target", "Target", "Body.");
    create_task(
        plan.path(),
        "referrer",
        "Referrer",
        "See [target](rdm:task/target).",
    );
    create_task(plan.path(), "unrelated", "Unrelated", "No links.");

    rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["backlinks", "task/target", "--project", "demo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("task/referrer"));

    let json = json_stdout(rdm().arg("--root").arg(plan.path()).args([
        "backlinks",
        "task/target",
        "--project",
        "demo",
        "--format",
        "json",
    ]));
    let arr = json.as_array().expect("array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["kind"], "task");
    assert_eq!(arr[0]["slug"], "referrer");

    // No referencing documents: exit 0, empty/summary output either format.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["backlinks", "task/unrelated", "--project", "demo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("no backlinks found"));

    let empty_json = json_stdout(rdm().arg("--root").arg(plan.path()).args([
        "backlinks",
        "task/unrelated",
        "--project",
        "demo",
        "--format",
        "json",
    ]));
    assert_eq!(empty_json, serde_json::json!([]));
}

// --- Path verification: distinct from dangling item links, checkout-gated ---

#[test]
fn check_inside_checkout_reports_missing_at_rev_distinct_from_dangling() {
    let plan = init_plan_repo();

    // A real source-repo checkout with one commit, so `main` resolves but
    // the referenced path does not exist at it.
    let src = TempDir::new().unwrap();
    git(src.path(), &["init", "-b", "main"]);
    std::fs::write(src.path().join("README.md"), "# project").unwrap();
    git(src.path(), &["add", "."]);
    git(src.path(), &["commit", "-m", "initial"]);
    // The project must be configured with this checkout as its source, or
    // path verification correctly refuses to trust an unrelated checkout —
    // see `check_inside_a_non_matching_checkout_skips_verification`.
    set_project_source(plan.path(), "demo", &src.path().to_string_lossy());

    create_task(
        plan.path(),
        "mixed",
        "Mixed",
        "Dangling: [ghost](rdm:task/does-not-exist). Code: [src](rdm:src/does-not-exist.rs).",
    );

    let json = json_stdout(
        rdm()
            .arg("--root")
            .arg(plan.path())
            .current_dir(src.path())
            .args([
                "link",
                "check",
                "--on",
                "task/mixed",
                "--project",
                "demo",
                "--format",
                "json",
            ]),
    );

    let dangling = json["dangling"].as_array().unwrap();
    let missing_at_rev = json["missing_at_rev"].as_array().unwrap();
    assert_eq!(dangling.len(), 1, "expected one dangling item link: {json}");
    assert_eq!(
        missing_at_rev.len(),
        1,
        "expected one missing-at-rev code link: {json}"
    );
    assert_eq!(dangling[0]["target"], "task/does-not-exist");
    assert_eq!(missing_at_rev[0]["path"], "does-not-exist.rs");
    // Path verification actually ran — no skip note.
    assert!(json.get("path_verification_skipped").is_none());

    // Both findings count toward the nonzero exit.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .current_dir(src.path())
        .args(["link", "check", "--on", "task/mixed", "--project", "demo"])
        .assert()
        .failure()
        .code(1);
}

#[test]
fn check_inside_a_non_matching_checkout_skips_verification() {
    let plan = init_plan_repo();

    // The project's *actual* configured source, elsewhere on disk.
    let configured_src = TempDir::new().unwrap();
    git(configured_src.path(), &["init", "-b", "main"]);
    std::fs::write(configured_src.path().join("README.md"), "# project").unwrap();
    git(configured_src.path(), &["add", "."]);
    git(configured_src.path(), &["commit", "-m", "initial"]);
    set_project_source(
        plan.path(),
        "demo",
        &configured_src.path().to_string_lossy(),
    );

    // An unrelated git checkout that happens to contain `cwd` — e.g. the
    // plan repo itself is git-managed, or any other repo on the machine.
    let unrelated = TempDir::new().unwrap();
    git(unrelated.path(), &["init", "-b", "main"]);
    std::fs::write(unrelated.path().join("README.md"), "# unrelated").unwrap();
    git(unrelated.path(), &["add", "."]);
    git(unrelated.path(), &["commit", "-m", "initial"]);

    create_task(
        plan.path(),
        "code-link",
        "Code link",
        "Code: [src](rdm:src/does-not-exist.rs).",
    );

    let json = json_stdout(
        rdm()
            .arg("--root")
            .arg(plan.path())
            .current_dir(unrelated.path())
            .args([
                "link",
                "check",
                "--on",
                "task/code-link",
                "--project",
                "demo",
                "--format",
                "json",
            ]),
    );

    // Never verified against the unrelated repo — no false "missing at
    // rev" from a coincidentally-present-or-absent path in the wrong repo.
    assert_eq!(json["missing_at_rev"], serde_json::json!([]));
    assert!(
        json["path_verification_skipped"]
            .as_str()
            .is_some_and(|s| s.contains("not the project's configured source")),
        "expected a mismatch skip note: {json}"
    );

    // No dangling item link, no diagnostics, and the one code link's real
    // status was never verified — exit 0.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .current_dir(unrelated.path())
        .args([
            "link",
            "check",
            "--on",
            "task/code-link",
            "--project",
            "demo",
        ])
        .assert()
        .success();
}

#[test]
fn check_outside_checkout_skips_path_verification_and_exit_depends_on_dangling_only() {
    let plan = init_plan_repo();
    create_task(
        plan.path(),
        "code-only",
        "Code only",
        "Just a code link: [src](rdm:src/does-not-exist.rs).",
    );

    // A plain non-git directory: no checkout to verify paths against.
    let nongit = TempDir::new().unwrap();

    let json = json_stdout(
        rdm()
            .arg("--root")
            .arg(plan.path())
            .current_dir(nongit.path())
            .args([
                "link",
                "check",
                "--on",
                "task/code-only",
                "--project",
                "demo",
                "--format",
                "json",
            ]),
    );
    assert_eq!(json["missing_at_rev"], serde_json::json!([]));
    assert!(
        json["path_verification_skipped"]
            .as_str()
            .is_some_and(|s| s.contains("checkout")),
        "expected a skip note: {json}"
    );

    // No dangling item link and no parse diagnostics in this document, and
    // path verification didn't run — exit 0 (the code link's real
    // missing-at-rev status is simply unverified, not reported broken).
    rdm()
        .arg("--root")
        .arg(plan.path())
        .current_dir(nongit.path())
        .args([
            "link",
            "check",
            "--on",
            "task/code-only",
            "--project",
            "demo",
        ])
        .assert()
        .success();
}

// --- URL-form `source.repo`: `repo_matches_source` compares the checkout's
// `origin` remote, normalized, rather than a canonicalized filesystem path
// (see `check_inside_checkout_reports_missing_at_rev_distinct_from_dangling`
// for the filesystem-path branch) ---

#[test]
fn check_inside_checkout_matches_url_source_via_origin_remote() {
    let plan = init_plan_repo();

    let src = TempDir::new().unwrap();
    git(src.path(), &["init", "-b", "main"]);
    std::fs::write(src.path().join("README.md"), "# project").unwrap();
    git(src.path(), &["add", "."]);
    git(src.path(), &["commit", "-m", "initial"]);
    // Trailing `/` and `.git` on the remote, vs. the configured source
    // carrying neither — `repo_matches_source` must normalize both sides
    // before comparing rather than requiring a byte-for-byte match.
    git(
        src.path(),
        &[
            "remote",
            "add",
            "origin",
            "https://example.com/org/repo.git/",
        ],
    );
    set_project_source(plan.path(), "demo", "https://example.com/org/repo");

    create_task(
        plan.path(),
        "url-source",
        "URL source",
        "Code: [src](rdm:src/does-not-exist.rs).",
    );

    let json = json_stdout(
        rdm()
            .arg("--root")
            .arg(plan.path())
            .current_dir(src.path())
            .args([
                "link",
                "check",
                "--on",
                "task/url-source",
                "--project",
                "demo",
                "--format",
                "json",
            ]),
    );

    // Path verification actually ran against this checkout — no skip note,
    // and the missing path was found.
    assert!(json.get("path_verification_skipped").is_none(), "{json}");
    let missing_at_rev = json["missing_at_rev"].as_array().unwrap();
    assert_eq!(missing_at_rev.len(), 1, "expected one finding: {json}");
    assert_eq!(missing_at_rev[0]["path"], "does-not-exist.rs");
}

#[test]
fn check_inside_checkout_mismatched_origin_remote_skips_verification() {
    let plan = init_plan_repo();

    let src = TempDir::new().unwrap();
    git(src.path(), &["init", "-b", "main"]);
    std::fs::write(src.path().join("README.md"), "# project").unwrap();
    git(src.path(), &["add", "."]);
    git(src.path(), &["commit", "-m", "initial"]);
    git(
        src.path(),
        &[
            "remote",
            "add",
            "origin",
            "https://example.com/org/other-repo.git",
        ],
    );
    set_project_source(plan.path(), "demo", "https://example.com/org/repo");

    create_task(
        plan.path(),
        "url-mismatch",
        "URL mismatch",
        "Code: [src](rdm:src/does-not-exist.rs).",
    );

    let json = json_stdout(
        rdm()
            .arg("--root")
            .arg(plan.path())
            .current_dir(src.path())
            .args([
                "link",
                "check",
                "--on",
                "task/url-mismatch",
                "--project",
                "demo",
                "--format",
                "json",
            ]),
    );

    // Never verified against this checkout — its origin doesn't match the
    // configured URL source, so no false "missing at rev".
    assert_eq!(json["missing_at_rev"], serde_json::json!([]));
    assert!(
        json["path_verification_skipped"]
            .as_str()
            .is_some_and(|s| s.contains("not the project's configured source")),
        "expected a mismatch skip note: {json}"
    );
}
