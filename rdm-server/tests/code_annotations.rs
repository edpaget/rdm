//! HTTP-level coverage of the web UI as the reference consumer of the
//! code-as-linkable-target contract: the code-location annotations resource
//! (`GET /projects/:project/annotations?target=…`), the entry points to it
//! from rendered code and commit links, the degradation labels, and
//! Referenced-by for commit references.
//!
//! Everything is seeded through `rdm-core` against a temp plan repo, and the
//! source repository is a real temp git checkout, like
//! `tests/change_reviews.rs` (whose `git()` / `set_project_source` helpers
//! this copies in reduced form).
//!
//! The fixture: a source repo whose `topic` head `H` edits line 2 of
//! `src/lib.rs`; a roadmap/phase with an approved plan implementing it; a
//! submitted `change/H` review implementing that plan with four comments —
//!
//! 1. a file-quote on `src/lib.rs` line 2 (range), `applied_commit = H`;
//! 2. anchorless, `outside-hunk`, meant `src/lib.rs` (file);
//! 3. anchorless, `whole-document` (commit, deliberate);
//! 4. anchorless, `path-missing`, meant `src/gone.rs` (commit, degraded);
//!
//! — and a task whose body links `rdm:src/src/lib.rs@H#L2`, `rdm:src@H`, and
//! a malformed `rdm:src@`.

use std::net::SocketAddr;
use std::path::Path;
use std::process::Command;

use chrono::Utc;
use rdm_core::document::Document;
use rdm_core::link::ItemRef;
use rdm_core::model::{
    Anchor, AnchorState, Plan, PlanStatus, Priority, Review, ReviewComment, ReviewCommentStatus,
    ReviewState, ReviewTarget, Verdict,
};
use rdm_store_fs::FsStore;
use reqwest::Client;
use serde_json::Value;
use tempfile::TempDir;

const PROJECT: &str = "demo";
const ROADMAP: &str = "auth";
const STEM: &str = "phase-1-design";
const REVIEW_ID: &str = "2026-09-26-0900-abcd";
const PLAN: &str = "design-plan";
const TASK: &str = "linker";

/// A line that exists only in the source repository — never in any plan
/// document — so its absence from every response proves no file content is
/// served.
const SOURCE_ONLY_LINE: &str = "const NEVER_SERVED_SENTINEL: u32 = 42;";

fn git(dir: &Path, args: &[&str]) -> std::process::Output {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.com")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

fn git_out(dir: &Path, args: &[&str]) -> String {
    String::from_utf8_lossy(&git(dir, args).stdout)
        .trim()
        .to_string()
}

/// A source repository: `main` has `src/lib.rs`, `topic` edits its line 2
/// at head `H`. Returns `(dir, base, head)`.
fn seed_source_repo() -> (TempDir, String, String) {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    git(p, &["init", "-b", "main"]);
    std::fs::create_dir_all(p.join("src")).unwrap();
    std::fs::write(
        p.join("src/lib.rs"),
        format!("fn one() {{}}\nfn two() {{}}\n{SOURCE_ONLY_LINE}\n"),
    )
    .unwrap();
    git(p, &["add", "."]);
    git(p, &["commit", "-m", "base"]);
    let base = git_out(p, &["rev-parse", "HEAD"]);
    git(p, &["checkout", "-b", "topic"]);
    std::fs::write(
        p.join("src/lib.rs"),
        format!("fn one() {{}}\nfn two_renamed() {{}}\n{SOURCE_ONLY_LINE}\n"),
    )
    .unwrap();
    git(p, &["add", "."]);
    git(p, &["commit", "-m", "the reviewed change"]);
    let head = git_out(p, &["rev-parse", "HEAD"]);
    (dir, base, head)
}

/// Hand-edits `project.md` to configure (or, with `None`, to leave unset) the
/// project's `source.repo` — there is no CLI command for it.
fn set_project_source(plan: &Path, repo: Option<&str>) {
    let Some(repo) = repo else { return };
    let path = plan.join("projects").join(PROJECT).join("project.md");
    let content = std::fs::read_to_string(&path).unwrap();
    let rest = content.strip_prefix("---\n").expect("frontmatter open");
    let end = rest.find("\n---").expect("frontmatter close");
    let (frontmatter, tail) = rest.split_at(end);
    std::fs::write(
        &path,
        format!("---\n{frontmatter}\nsource:\n  repo: \"{repo}\"{tail}"),
    )
    .unwrap();
}

fn comment(id: u32, anchor: Option<Anchor>, state: Option<AnchorState>) -> ReviewComment {
    ReviewComment {
        id,
        doc: None,
        status: ReviewCommentStatus::Open,
        applied_commit: None,
        anchor,
        anchor_state: state,
        intended_path: None,
        body: format!("Finding {id}."),
        reply: None,
    }
}

fn seed_plan_repo(base: &str, head: &str) -> TempDir {
    let dir = TempDir::new().unwrap();
    let mut store = FsStore::new(dir.path());
    rdm_core::ops::init::init(&mut store).unwrap();
    rdm_core::ops::project::create_project(&mut store, PROJECT, "Demo").unwrap();
    rdm_core::ops::roadmap::create_roadmap(
        &mut store,
        rdm_core::ops::roadmap::CreateRoadmap {
            project: PROJECT,
            slug: ROADMAP,
            title: "Auth",
            ..Default::default()
        },
    )
    .unwrap();
    rdm_core::ops::phase::create_phase(
        &mut store,
        rdm_core::ops::phase::CreatePhase {
            project: PROJECT,
            roadmap: ROADMAP,
            slug: "design",
            title: "Design",
            number: Some(1),
            body: Some("Phase body.\n"),
            ..Default::default()
        },
    )
    .unwrap();
    let task_body = format!(
        "See [a](rdm:src/src/lib.rs@{head}#L2) and [c](rdm:src@{head}) and [bad](rdm:src@).\n"
    );
    rdm_core::ops::task::create_task(
        &mut store,
        rdm_core::ops::task::CreateTask {
            project: PROJECT,
            slug: TASK,
            title: "Linker",
            priority: Priority::Medium,
            body: Some(&task_body),
            ..Default::default()
        },
    )
    .unwrap();
    let today = Utc::now().date_naive();
    rdm_core::io::write_plan(
        &mut store,
        PROJECT,
        PLAN,
        &Document {
            frontmatter: Plan {
                project: PROJECT.to_string(),
                plan: PLAN.to_string(),
                title: "Design plan".to_string(),
                implements: ItemRef::Phase {
                    roadmap: ROADMAP.to_string(),
                    stem: STEM.to_string(),
                },
                supersedes: None,
                status: PlanStatus::Approved,
                created: today,
                updated: today,
            },
            body: "Plan body.\n".to_string(),
        },
    )
    .unwrap();

    let mut c1 = comment(
        1,
        Some(Anchor::FileQuote {
            path: "src/lib.rs".to_string(),
            quote: "fn two_renamed() {}".to_string(),
            occurrence: 1,
            start_line: 2,
            end_line: 2,
        }),
        None,
    );
    c1.applied_commit = Some(head.to_string());
    let mut c2 = comment(2, None, Some(AnchorState::OutsideHunk));
    c2.intended_path = Some("src/lib.rs".to_string());
    let c3 = comment(3, None, Some(AnchorState::WholeDocument));
    let mut c4 = comment(4, None, Some(AnchorState::PathMissing));
    c4.intended_path = Some("src/gone.rs".to_string());
    rdm_core::io::write_review(
        &mut store,
        PROJECT,
        REVIEW_ID,
        &Document {
            frontmatter: Review {
                id: REVIEW_ID.to_string(),
                author: "reviewer".to_string(),
                target: ReviewTarget::Change {
                    head: head.to_string(),
                    base: Some(base.to_string()),
                },
                state: ReviewState::Submitted,
                verdict: Some(Verdict::RequestChanges),
                created: Utc::now(),
                submitted: Some(Utc::now()),
                created_commit: None,
                implements: Some(ReviewTarget::Plan {
                    slug: PLAN.to_string(),
                }),
                change_branch: Some("topic".to_string()),
                comments: vec![c1, c2, c3, c4],
            },
            body: "Four findings.\n".to_string(),
        },
    )
    .unwrap();
    rdm_core::store::Store::commit(&mut store).unwrap();
    dir
}

/// The seeded source and plan repos, with the plan's `source.repo` pointing
/// at the source (or left unset with `with_source: false`).
#[cfg_attr(not(feature = "git"), allow(dead_code))]
struct Fixture {
    src: TempDir,
    plan: TempDir,
    head: String,
}

fn fixture(with_source: bool) -> Fixture {
    let (src, base, head) = seed_source_repo();
    let plan = seed_plan_repo(&base, &head);
    if with_source {
        set_project_source(plan.path(), Some(&src.path().to_string_lossy()));
    }
    Fixture { src, plan, head }
}

async fn spawn(dir: &Path) -> (SocketAddr, Client) {
    let state = rdm_server::state::AppState {
        plan_root: dir.to_path_buf(),
        quick_filters: Vec::new(),
        ..Default::default()
    };
    let app = rdm_server::router::build_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, Client::new())
}

/// The annotations href for `reference`, encoded like the server encodes it.
fn annotations_path(reference: &str) -> String {
    let encoded: String =
        percent_encoding::utf8_percent_encode(reference, percent_encoding::NON_ALPHANUMERIC)
            .to_string();
    format!("/projects/{PROJECT}/annotations?target={encoded}")
}

async fn get(addr: SocketAddr, client: &Client, path: &str, accept: &str) -> (u16, String, String) {
    let res = client
        .get(format!("http://{addr}{path}"))
        .header("Accept", accept)
        .send()
        .await
        .unwrap();
    let status = res.status().as_u16();
    let content_type = res
        .headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default();
    (status, content_type, res.text().await.unwrap())
}

async fn get_hal(addr: SocketAddr, client: &Client, path: &str) -> Value {
    let (status, _, body) = get(addr, client, path, "application/hal+json").await;
    assert_eq!(status, 200, "{path}: {body}");
    serde_json::from_str(&body).unwrap()
}

async fn get_html(addr: SocketAddr, client: &Client, path: &str) -> String {
    let (status, _, body) = get(addr, client, path, "text/html").await;
    assert_eq!(status, 200, "{path}: {body}");
    body
}

#[cfg(feature = "git")]
/// A HAL `_links` relation as a list of hrefs, whether it serialized as one
/// link object or an array.
fn hrefs(rel: &Value) -> Vec<String> {
    match rel {
        Value::Array(links) => links
            .iter()
            .map(|l| l["href"].as_str().unwrap().to_string())
            .collect(),
        Value::Object(_) => vec![rel["href"].as_str().unwrap().to_string()],
        _ => Vec::new(),
    }
}

#[cfg(feature = "git")]
/// Every `href` of an `rdm-link-annotations` anchor in `html`, in order.
fn annotation_entry_hrefs(html: &str) -> Vec<String> {
    let marker = r#"class="rdm-link-annotations" href=""#;
    html.match_indices(marker)
        .map(|(i, _)| {
            let rest = &html[i + marker.len()..];
            rest[..rest.find('"').unwrap()].to_string()
        })
        .collect()
}

#[cfg(feature = "git")]
/// The `target` of an annotations href, decoded.
fn decoded_target(href: &str) -> String {
    let (_, query) = href.split_once("?target=").unwrap();
    percent_encoding::percent_decode_str(query)
        .decode_utf8()
        .unwrap()
        .into_owned()
}

/// Core's own answer for `reference`, as `rdm backlinks --format json`
/// prints it.
#[cfg(feature = "git")]
fn core_annotations_json(f: &Fixture, reference: &str) -> Vec<Value> {
    let store = FsStore::new(f.plan.path());
    let source = rdm_git::GitSourceRepo::new(f.src.path());
    let target = rdm_core::ops::links::parse_backlink_target(&store, PROJECT, reference).unwrap();
    rdm_core::ops::links::code_annotations(&store, &source, PROJECT, &target)
        .unwrap()
        .iter()
        .map(|a| serde_json::to_value(rdm_core::json::code_annotation_to_json(a)).unwrap())
        .collect()
}

/// AC2: the annotation list is served from phase 4's contract. The whole
/// `_embedded.annotations` array equals `code_annotations` mapped through
/// `code_annotation_to_json` — and is rich enough (anchor and body entries,
/// range and file levels, commit-level field entries) that equality is not
/// vacuous: `level`, `reason` and the resolved `rev` are computed only
/// through the source port, so a store-scanning handler would diverge.
#[cfg(feature = "git")]
#[tokio::test]
async fn annotations_hal_equals_core_json_for_path_and_commit() {
    let f = fixture(true);
    let (addr, client) = spawn(f.plan.path()).await;

    let path_ref = "rdm:src/src/lib.rs";
    let served = get_hal(addr, &client, &annotations_path(path_ref)).await;
    let expected = core_annotations_json(&f, path_ref);
    assert_eq!(
        served["_embedded"]["annotations"],
        Value::Array(expected.clone())
    );
    assert_eq!(served["target"], "src/src/lib.rs");
    assert!(
        served.get("source_verification_skipped").is_none(),
        "{served}"
    );
    let has = |key: &str, value: &str| expected.iter().any(|e| e[key] == value);
    assert!(
        has("found_in", "anchor") && has("found_in", "body"),
        "{expected:?}"
    );
    assert!(
        has("level", "range") && has("level", "file"),
        "{expected:?}"
    );

    let commit_ref = format!("rdm:src@{}", f.head);
    let served = get_hal(addr, &client, &annotations_path(&commit_ref)).await;
    let expected = core_annotations_json(&f, &commit_ref);
    assert_eq!(
        served["_embedded"]["annotations"],
        Value::Array(expected.clone())
    );
    let has = |key: &str, value: &str| expected.iter().any(|e| e[key] == value);
    assert!(
        has("level", "commit") && has("found_in", "field"),
        "{expected:?}"
    );
    assert!(
        has("found_in", "anchor") && has("found_in", "body"),
        "{expected:?}"
    );
}

/// AC1: code and commit links render (the existing `rdm-link-code` markup)
/// and resolve: each is followed by an annotations entry point pinned to the
/// location it resolves to, and following it lists the location's
/// annotations. A malformed `rdm:src@` still renders broken.
#[cfg(feature = "git")]
#[tokio::test]
async fn code_and_commit_links_render_with_annotations_entry_points() {
    let f = fixture(true);
    let (addr, client) = spawn(f.plan.path()).await;
    let head = &f.head;

    let html = get_html(addr, &client, &format!("/projects/{PROJECT}/tasks/{TASK}")).await;
    assert_eq!(html.matches("rdm-link-code").count(), 2, "{html}");
    assert!(html.contains("rdm-link-broken"), "{html}");
    let entries = annotation_entry_hrefs(&html);
    let targets: Vec<String> = entries.iter().map(|h| decoded_target(h)).collect();
    assert_eq!(
        targets,
        vec![
            format!("rdm:src/src/lib.rs@{head}#L2"),
            format!("rdm:src@{head}")
        ]
    );

    // Each entry point resolves to a page listing the annotations there.
    let range_page = get_html(addr, &client, &entries[0]).await;
    assert!(
        range_page.contains(&format!("#comment-{REVIEW_ID}-c1")),
        "comment 1 is anchored at line 2: {range_page}"
    );
    let commit_page = get_html(addr, &client, &entries[1]).await;
    assert!(
        commit_page.contains(&format!("#review-{REVIEW_ID}")),
        "the review names the commit as its head: {commit_page}"
    );

    // The task's HAL advertises the same entry points.
    let hal = get_hal(addr, &client, &format!("/projects/{PROJECT}/tasks/{TASK}")).await;
    assert_eq!(hrefs(&hal["_links"]["rdm:annotations"]), entries);
}

/// The permalink AC over HTTP: every listed annotation carries its pinned
/// link. With a local-directory `source.repo` (the only kind this server can
/// read) that renders as the non-navigable span, per the forge-permalink
/// honesty rule; the live forge href is asserted at the view layer
/// (`annotation_views` unit tests).
#[cfg(feature = "git")]
#[tokio::test]
async fn every_listed_annotation_carries_its_pinned_link() {
    let f = fixture(true);
    let (addr, client) = spawn(f.plan.path()).await;
    let path_ref = "rdm:src/src/lib.rs";
    let html = get_html(addr, &client, &annotations_path(path_ref)).await;
    let expected = core_annotations_json(&f, path_ref);

    let items: Vec<&str> = html.split(r#"<li class="annotation">"#).skip(1).collect();
    assert_eq!(items.len(), expected.len(), "{html}");
    for (item, entry) in items.iter().zip(&expected) {
        let link = entry["link"].as_str().unwrap();
        assert!(
            link.contains(&format!("@{}", f.head)),
            "every entry is pinned at H: {link}"
        );
        assert!(item.contains("rdm-link-code"), "{item}");
        assert!(item.contains(&format!(r#"title="{link}""#)), "{item}");
        assert!(item.contains("rdm-link-nolink"), "{item}");
    }
}

/// AC3: a degraded anchor renders distinguishably from a deliberate
/// whole-document comment — on the annotations pages and in the phase
/// page's Reviews section alike.
#[cfg(feature = "git")]
#[tokio::test]
async fn degraded_anchor_renders_distinct_from_whole_document() {
    let f = fixture(true);
    let (addr, client) = spawn(f.plan.path()).await;

    let commit_page = get_html(
        addr,
        &client,
        &annotations_path(&format!("rdm:src@{}", f.head)),
    )
    .await;
    let item_of = |html: &str, comment: u32| -> String {
        html.split(r#"<li class="annotation">"#)
            .find(|item| item.contains(&format!("#comment-{REVIEW_ID}-c{comment}\"")))
            .unwrap_or_else(|| panic!("comment {comment} listed: {html}"))
            .to_string()
    };
    let c3 = item_of(&commit_page, 3);
    assert!(c3.contains("anchor-whole-document"), "{c3}");
    assert!(!c3.contains("anchor-degraded"), "{c3}");
    let c4 = item_of(&commit_page, 4);
    assert!(c4.contains("anchor-degraded"), "{c4}");
    assert!(
        c4.contains("Anchor lost: path-missing — meant src/gone.rs"),
        "{c4}"
    );

    let path_page = get_html(addr, &client, &annotations_path("rdm:src/src/lib.rs")).await;
    let c2 = item_of(&path_page, 2);
    assert!(c2.contains("badge-level-file"), "{c2}");
    assert!(c2.contains("anchor-degraded"), "{c2}");
    assert!(c2.contains("Anchor lost: outside-hunk"), "{c2}");

    let phase = get_html(
        addr,
        &client,
        &format!("/projects/{PROJECT}/roadmaps/{ROADMAP}/phases/{STEM}"),
    )
    .await;
    let comment_of = |n: u32| -> String {
        let start = phase
            .find(&format!(r#"id="comment-{REVIEW_ID}-c{n}""#))
            .unwrap_or_else(|| panic!("comment {n} on the phase page: {phase}"));
        let rest = &phase[start..];
        rest[..rest.find("</li>").unwrap()].to_string()
    };
    assert!(
        comment_of(3).contains("anchor-whole-document"),
        "{}",
        comment_of(3)
    );
    assert!(
        comment_of(4).contains("anchor-degraded"),
        "{}",
        comment_of(4)
    );
}

/// Step 4: the commit page's Referenced-by lists what references the commit
/// — the change review (linking to the implemented phase's page) and the
/// task (body link) — and so does its HAL `rdm:backlink`. On the phase page
/// comment 1's `applied_commit` links back to that commit page.
#[cfg(feature = "git")]
#[tokio::test]
async fn commit_page_lists_reviews_referencing_the_commit() {
    let f = fixture(true);
    let (addr, client) = spawn(f.plan.path()).await;
    let commit_ref = format!("rdm:src@{}", f.head);
    let page = get_html(addr, &client, &annotations_path(&commit_ref)).await;

    let section = &page[page
        .find(r#"id="referenced-by""#)
        .expect("a Referenced-by section")..];
    let review_href =
        format!("/projects/{PROJECT}/roadmaps/{ROADMAP}/phases/{STEM}#review-{REVIEW_ID}");
    let task_href = format!("/projects/{PROJECT}/tasks/{TASK}");
    assert!(
        section.contains(&format!(r#"href="{task_href}""#)),
        "{section}"
    );
    assert!(
        section.contains(&format!(r#"href="{review_href}""#)),
        "{section}"
    );

    let hal = get_hal(addr, &client, &annotations_path(&commit_ref)).await;
    let backlinks = hrefs(&hal["_links"]["rdm:backlink"]);
    assert!(backlinks.contains(&task_href), "{backlinks:?}");
    assert!(backlinks.contains(&review_href), "{backlinks:?}");

    let phase = get_html(
        addr,
        &client,
        &format!("/projects/{PROJECT}/roadmaps/{ROADMAP}/phases/{STEM}"),
    )
    .await;
    let applied = &phase[phase
        .find("applied in")
        .expect("comment 1's applied commit")..];
    let applied = &applied[..applied.find("</span>\n").unwrap_or(applied.len())];
    let entries = annotation_entry_hrefs(applied);
    assert_eq!(
        entries.first().map(|h| decoded_target(h)),
        Some(commit_ref),
        "{applied}"
    );
}

/// No source repository fails open, not 500: both representations are 200,
/// say why, and list nothing — "could not look" never reads as "nothing
/// annotates this". Holds with or without the `git` feature.
#[tokio::test]
async fn annotations_fail_open_without_a_source_repo() {
    let f = fixture(false);
    let (addr, client) = spawn(f.plan.path()).await;
    let path = annotations_path("rdm:src/src/lib.rs");

    let html = get_html(addr, &client, &path).await;
    assert!(html.contains(r#"class="source-note""#), "{html}");
    assert!(
        html.contains("configures no local source repo") || html.contains("no git support"),
        "{html}"
    );
    assert!(!html.contains(r#"<li class="annotation">"#), "{html}");

    let hal = get_hal(addr, &client, &path).await;
    assert!(hal["source_verification_skipped"].is_string(), "{hal}");
    assert!(
        hal.get("_embedded")
            .is_none_or(|e| e.get("annotations").is_none()),
        "{hal}"
    );
}

/// Core's errors reach the resource through the existing mappings: Problem
/// Details under the HAL Accept, the error page with the same status under
/// the HTML one.
#[cfg(feature = "git")]
#[tokio::test]
async fn annotations_errors_map_to_problem_details() {
    let f = fixture(true);
    let (addr, client) = spawn(f.plan.path()).await;
    let cases = [
        (annotations_path("rdm:src/src/nope.rs"), 404),
        (annotations_path("rdm:src/src"), 422),
        (
            annotations_path("rdm:src@0123456789abcdef0123456789abcdef01234567"),
            404,
        ),
        (format!("/projects/{PROJECT}/annotations"), 400),
        (format!("/projects/{PROJECT}/annotations?target="), 400),
        (annotations_path("task/x"), 400),
    ];
    for (path, status) in cases {
        let (got, content_type, body) = get(addr, &client, &path, "application/hal+json").await;
        assert_eq!(got, status, "{path}: {body}");
        assert_eq!(content_type, "application/problem+json", "{path}");
        let (got, content_type, _) = get(addr, &client, &path, "text/html").await;
        assert_eq!(got, status, "{path}");
        assert!(content_type.starts_with("text/html"), "{path}");
    }
}

/// AC4, the roadmap's principal non-goal: rdm never serves source file
/// contents. The annotations resource — the only route that reads the source
/// repository — returns none of the source file's text in either
/// representation, and plausible blob-at-revision paths do not exist.
#[cfg(feature = "git")]
#[tokio::test]
async fn no_route_serves_source_file_contents() {
    let f = fixture(true);
    let (addr, client) = spawn(f.plan.path()).await;
    let head = &f.head;

    for reference in [
        "rdm:src/src/lib.rs".to_string(),
        format!("rdm:src/src/lib.rs@{head}#L1-L3"),
        format!("rdm:src@{head}"),
    ] {
        let path = annotations_path(&reference);
        for accept in ["text/html", "application/hal+json"] {
            let (status, _, body) = get(addr, &client, &path, accept).await;
            assert_eq!(status, 200, "{reference} {accept}: {body}");
            for line in [SOURCE_ONLY_LINE, "NEVER_SERVED_SENTINEL", "fn one() {}"] {
                assert!(
                    !body.contains(line),
                    "{reference} ({accept}) must not carry file content {line:?}"
                );
            }
        }
    }

    for path in [
        format!("/projects/{PROJECT}/src/src/lib.rs@{head}"),
        format!("/projects/{PROJECT}/blob/{head}/src/lib.rs"),
        format!("/projects/{PROJECT}/src/src/lib.rs"),
        format!("/projects/{PROJECT}/raw/{head}/src/lib.rs"),
    ] {
        for accept in ["text/html", "application/hal+json"] {
            let (status, _, body) = get(addr, &client, &path, accept).await;
            assert_eq!(status, 404, "{path} ({accept}) must not exist: {body}");
            assert!(!body.contains("NEVER_SERVED_SENTINEL"), "{path}");
        }
    }
}
