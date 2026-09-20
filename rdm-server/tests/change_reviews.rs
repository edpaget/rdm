//! HTTP- and HTML-level coverage of `change/<sha>` review resolution.
//!
//! The REST surface used to report every change-review comment `unresolved`
//! with `unresolved_reason` and `source_verification_skipped` both absent —
//! indistinguishable from anchors that genuinely no longer resolve — while
//! `rdm review show --format json` reported real states for the same review
//! file. `docs/change-reviews.md` § "The degrade rule" promises the read path
//! degrades *with a note*, so that was a defect against the contract, not a
//! deferral.
//!
//! What this file pins:
//!
//! 1. with a configured local `source.repo`, the REST response's per-comment
//!    resolution objects equal the ones the CLI's own pass produces for the
//!    same review file, and match the states the fixture independently
//!    dictates (one resolved, one drifted);
//! 2. with no source configured, every comment is `unresolved` AND
//!    `source_verification_skipped` is non-null;
//! 3. never `unresolved` with a null note, asserted per comment in BOTH runs;
//! 4. the HTML review section on the page of the item the review's plan
//!    implements renders the same states.
//!
//! Everything is seeded through `rdm-core` against a temp plan repo, and the
//! source repository is a real temp git checkout, exactly as
//! `tests/reviewed_gate.rs` seeds its fixtures.

use std::net::SocketAddr;
use std::path::Path;
use std::process::Command;

use chrono::Utc;
use rdm_core::document::Document;
use rdm_core::link::ItemRef;
use rdm_core::model::{
    Anchor, Plan, PlanStatus, Priority, Review, ReviewComment, ReviewCommentStatus, ReviewState,
    ReviewTarget, Verdict,
};
use rdm_store_fs::FsStore;
use reqwest::Client;
use serde_json::Value;
use tempfile::TempDir;

const PROJECT: &str = "demo";
const ROADMAP: &str = "auth";
const STEM: &str = "phase-1-design";
const REVIEW_ID: &str = "2026-09-19-0900-abcd";
const PLAN: &str = "design-plan";

/// The quote that still resolves at the branch tip, and the one edited away
/// after the review was recorded. Two lines apart on purpose: a quote whose own
/// neighbouring line changed drifts too, which would make the resolved
/// expectation measure the wrong thing.
const RESOLVING_QUOTE: &str = "fn two_renamed";
const DRIFTING_QUOTE: &str = "fn four_renamed";

fn flush(store: &mut FsStore) {
    rdm_core::store::Store::commit(store).unwrap();
}

/// A `git` command in `dir`, isolated from the developer's ambient git config
/// and identity — the same construction `rdm-cli/tests/git_test_support.rs`
/// uses, so a hostile `~/.gitconfig` cannot change what these fixtures build.
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

/// A source repository whose `topic` branch carries the reviewed head AND one
/// later commit, so exactly one of the two anchored quotes survives at the tip
/// drift is measured against.
fn seed_source_repo() -> (TempDir, String, String) {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    git(p, &["init", "-b", "main"]);
    std::fs::create_dir_all(p.join("src")).unwrap();
    std::fs::write(
        p.join("src/lib.rs"),
        "fn one() {}\nfn two() {}\nfn three() {}\nfn four() {}\n",
    )
    .unwrap();
    git(p, &["add", "."]);
    git(p, &["commit", "-m", "base"]);
    let base = git_out(p, &["rev-parse", "HEAD"]);

    git(p, &["checkout", "-b", "topic"]);
    std::fs::write(
        p.join("src/lib.rs"),
        "fn one() {}\nfn two_renamed() {}\nfn three() {}\nfn four_renamed() {}\n",
    )
    .unwrap();
    git(p, &["add", "."]);
    git(p, &["commit", "-m", "the reviewed change"]);
    let head = git_out(p, &["rev-parse", "HEAD"]);

    // A later commit on the same branch edits the second quote away. The first
    // quote and both of its neighbouring lines are untouched.
    std::fs::write(
        p.join("src/lib.rs"),
        "fn one() {}\nfn two_renamed() {}\nfn three() {}\nfn four_final() {}\n",
    )
    .unwrap();
    git(p, &["add", "."]);
    git(p, &["commit", "-m", "later work"]);

    (dir, base, head)
}

fn phase_item() -> ItemRef {
    ItemRef::Phase {
        roadmap: ROADMAP.to_string(),
        stem: STEM.to_string(),
    }
}

/// Hand-edits `project.md` to configure (or, with `None`, to clear) the
/// project's `source.repo`. There is no CLI command for it, so fixtures write
/// the file, exactly as `cli_review_change.rs` does.
fn set_project_source(plan: &Path, repo: Option<&str>) {
    let path = plan.join("projects").join(PROJECT).join("project.md");
    let content = std::fs::read_to_string(&path).unwrap();
    let kept: Vec<&str> = content
        .lines()
        .filter(|l| !l.starts_with("source:") && !l.starts_with("  repo:"))
        .collect();
    let stripped = format!("{}\n", kept.join("\n"));
    let Some(repo) = repo else {
        std::fs::write(&path, stripped).unwrap();
        return;
    };
    let rest = stripped.strip_prefix("---\n").expect("frontmatter open");
    let end = rest.find("\n---").expect("frontmatter close");
    let (frontmatter, tail) = rest.split_at(end);
    std::fs::write(
        &path,
        format!("---\n{frontmatter}\nsource:\n  repo: \"{repo}\"{tail}"),
    )
    .unwrap();
}

/// A plan repo carrying a roadmap + phase, an approved plan implementing that
/// phase, and a submitted `change/<head>` review with two anchored comments.
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
    // Present so the project is well-formed for the HTML page; nothing here
    // depends on it.
    rdm_core::ops::task::create_task(
        &mut store,
        rdm_core::ops::task::CreateTask {
            project: PROJECT,
            slug: "solo",
            title: "Solo",
            priority: Priority::Medium,
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
                implements: phase_item(),
                supersedes: None,
                status: PlanStatus::Approved,
                created: today,
                updated: today,
            },
            body: "Plan body.\n".to_string(),
        },
    )
    .unwrap();

    let anchor = |quote: &str, line: u32| {
        Some(Anchor::FileQuote {
            path: "src/lib.rs".to_string(),
            quote: quote.to_string(),
            occurrence: 1,
            start_line: line,
            end_line: line,
        })
    };
    let comment = |id: u32, quote: &str, line: u32| ReviewComment {
        id,
        doc: None,
        status: ReviewCommentStatus::Open,
        applied_commit: None,
        anchor: anchor(quote, line),
        body: format!("A finding about {quote}."),
        reply: None,
    };
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
                comments: vec![
                    comment(1, RESOLVING_QUOTE, 2),
                    comment(2, DRIFTING_QUOTE, 4),
                ],
            },
            body: "Two findings.\n".to_string(),
        },
    )
    .unwrap();
    flush(&mut store);
    dir
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

async fn get_review_json(addr: SocketAddr, client: &Client) -> Value {
    let res = client
        .get(format!(
            "http://{addr}/projects/{PROJECT}/reviews/{REVIEW_ID}"
        ))
        .header("Accept", "application/json")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    res.json().await.unwrap()
}

/// The invariant that failing used to be indistinguishable from working: a
/// comment may be `unresolved`, but never without SOME explanation — either its
/// own `unresolved_reason` or the review-wide `source_verification_skipped`.
fn assert_never_unresolved_without_a_note(review: &Value) {
    let source_note = review["source_verification_skipped"].as_str();
    for comment in review["comments"].as_array().unwrap() {
        if comment["resolution"]["state"] != "unresolved" {
            continue;
        }
        assert!(
            source_note.is_some() || comment["unresolved_reason"].as_str().is_some(),
            "an unresolved comment with no explanation is exactly the defect: {comment}"
        );
    }
}

/// AC6, case (a). With a configured local source, the REST response reports the
/// states the fixture dictates — and the very same resolution objects the CLI's
/// pass produces for this review file, because both now go through
/// `rdm_core::change::resolve_change_review`.
#[cfg(feature = "git")]
#[tokio::test]
async fn rest_change_review_matches_cli_resolution_when_a_source_resolves() {
    let (src, base, head) = seed_source_repo();
    let plan = seed_plan_repo(&base, &head);
    set_project_source(plan.path(), Some(&src.path().to_string_lossy()));

    let (addr, client) = spawn(plan.path()).await;
    let served = get_review_json(addr, &client).await;

    // The fixture's own, independent expectation: the first quote survives at
    // the branch tip, the second was edited away.
    assert!(
        served["source_verification_skipped"].is_null(),
        "verification really ran, so nothing was skipped: {served}"
    );
    let comments = served["comments"].as_array().unwrap();
    assert_eq!(comments.len(), 2);
    assert_eq!(comments[0]["resolution"]["state"], "resolved");
    assert_eq!(comments[0]["resolution"]["quote"], RESOLVING_QUOTE);
    assert_eq!(comments[1]["resolution"]["state"], "drifted");
    for comment in comments {
        assert!(
            comment["unresolved_reason"].is_null(),
            "a resolvable blob anchor has nothing more specific to say: {comment}"
        );
    }
    assert_never_unresolved_without_a_note(&served);

    // …and field-for-field against the CLI's own pass over the same file. This
    // is what would fail if the server stopped routing through core, or routed
    // through it with the wrong notes slice.
    let store = FsStore::new(plan.path());
    let doc = rdm_core::ops::reviews::get_review(&store, PROJECT, REVIEW_ID).unwrap();
    let source = rdm_git::GitSourceRepo::new(src.path());
    let (resolutions, notes, source_note) =
        rdm_core::change::resolve_change_review(&source, &doc.frontmatter);
    let expected = serde_json::to_value(
        rdm_core::json::review_to_json(REVIEW_ID, &doc, &resolutions, &notes)
            .with_source_note(source_note),
    )
    .unwrap();
    for (i, comment) in comments.iter().enumerate() {
        assert_eq!(
            comment["resolution"], expected["comments"][i]["resolution"],
            "comment {i}'s resolution must match the CLI's pass"
        );
        assert_eq!(
            comment["unresolved_reason"], expected["comments"][i]["unresolved_reason"],
            "comment {i}'s reason must match the CLI's pass"
        );
    }
    assert_eq!(
        served["source_verification_skipped"], expected["source_verification_skipped"],
        "the source note must match the CLI's pass"
    );
}

/// AC6, case (b). With no `source.repo`, the server cannot resolve anything —
/// and says so. `unresolved` with a null note is the shape this replaces, and it
/// holds in every build, with or without the `git` feature.
#[tokio::test]
async fn rest_change_review_sets_an_explicit_skip_note_without_a_source() {
    let (_src, base, head) = seed_source_repo();
    let plan = seed_plan_repo(&base, &head);
    set_project_source(plan.path(), None);

    let (addr, client) = spawn(plan.path()).await;
    let served = get_review_json(addr, &client).await;

    let note = served["source_verification_skipped"]
        .as_str()
        .unwrap_or_else(|| panic!("the skip must be reported: {served}"));
    assert!(
        note.contains("this server does not resolve change-review anchors"),
        "the note must name the surface: {note}"
    );
    for comment in served["comments"].as_array().unwrap() {
        assert_eq!(comment["resolution"]["state"], "unresolved", "{comment}");
    }
    assert_never_unresolved_without_a_note(&served);
}

/// A plan-repo review on the same server is unaffected: it still resolves
/// against the plan store and carries no source note, so the dispatch really is
/// a dispatch rather than a change-review takeover.
#[tokio::test]
async fn rest_plan_repo_review_still_resolves_against_the_plan_store() {
    let (_src, base, head) = seed_source_repo();
    let plan = seed_plan_repo(&base, &head);
    let mut store = FsStore::new(plan.path());
    let id = "2026-09-19-0901-beef";
    rdm_core::io::write_review(
        &mut store,
        PROJECT,
        id,
        &Document {
            frontmatter: Review {
                id: id.to_string(),
                author: "reviewer".to_string(),
                target: ReviewTarget::Phase {
                    roadmap: ROADMAP.to_string(),
                    stem: STEM.to_string(),
                },
                state: ReviewState::Submitted,
                verdict: Some(Verdict::Comment),
                created: Utc::now(),
                submitted: Some(Utc::now()),
                created_commit: None,
                implements: None,
                change_branch: None,
                comments: vec![ReviewComment {
                    id: 1,
                    doc: None,
                    status: ReviewCommentStatus::Open,
                    applied_commit: None,
                    anchor: Some(Anchor::TextQuote {
                        quote: "Phase body.".to_string(),
                        prefix: String::new(),
                        suffix: String::new(),
                    }),
                    body: "On the phase body.".to_string(),
                    reply: None,
                }],
            },
            body: "A plan-repo review.\n".to_string(),
        },
    )
    .unwrap();
    flush(&mut store);

    let (addr, client) = spawn(plan.path()).await;
    let res = client
        .get(format!("http://{addr}/projects/{PROJECT}/reviews/{id}"))
        .header("Accept", "application/json")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let served: Value = res.json().await.unwrap();
    assert!(
        served["source_verification_skipped"].is_null(),
        "a plan-repo review has no source to skip: {served}"
    );
    assert_eq!(served["comments"][0]["resolution"]["state"], "resolved");
}

/// AC6's HTML clause. A change review reaches the page of the item its
/// `implements` plan implements, and the badges there are computed from the same
/// pass the JSON reports: the resolved comment carries no `outdated` badge, the
/// drifted one does.
#[cfg(feature = "git")]
#[tokio::test]
async fn html_change_review_renders_the_same_states() {
    let (src, base, head) = seed_source_repo();
    let plan = seed_plan_repo(&base, &head);
    set_project_source(plan.path(), Some(&src.path().to_string_lossy()));

    let (addr, client) = spawn(plan.path()).await;
    let served = get_review_json(addr, &client).await;
    let comments = served["comments"].as_array().unwrap();
    assert_eq!(comments[0]["resolution"]["state"], "resolved");
    assert_eq!(comments[1]["resolution"]["state"], "drifted");

    let res = client
        .get(format!(
            "http://{addr}/projects/{PROJECT}/roadmaps/{ROADMAP}/phases/{STEM}"
        ))
        .header("Accept", "text/html")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let html = res.text().await.unwrap();

    assert!(
        html.contains(&format!("review-{REVIEW_ID}")),
        "the change review must reach the page of the item its plan implements"
    );
    // Both comments render; only the drifted one is badged outdated. Anything
    // else — two badges, or none — means the page resolved against the wrong
    // repository, which is what an unconditional plan-store pass did.
    assert!(html.contains(&format!("comment-{REVIEW_ID}-c1")), "{html}");
    assert!(html.contains(&format!("comment-{REVIEW_ID}-c2")), "{html}");
    assert_eq!(
        html.matches("badge-outdated").count(),
        1,
        "exactly one of the two comments is drifted, so exactly one badge: {html}"
    );
    // The resolved comment's quote renders from the SOURCE repository's content,
    // which is only reachable through the change-review pass.
    assert!(
        html.contains(RESOLVING_QUOTE),
        "the resolved anchor's quote must render: {html}"
    );
}
