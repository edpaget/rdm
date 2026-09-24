//! The document-review revision loop the `rdm-revise` skill drives, end to
//! end: a submitted `request-changes` review with four comments is worked
//! comment by comment — a resolved anchor, a whole-document comment, a
//! drifted anchor that gets a clarification reply and blocks closing, a
//! wont-fix — and then closed, leaving the change-request queue.
//!
//! Every body edit is committed before its comment is marked addressed, so
//! each `--applied-commit` is that edit's own commit, and the read-back
//! asserts it. (The retired shell captured the plan HEAD before committing,
//! so every applied commit it recorded was the seed commit.)

use serde_json::Value;

use crate::{Plan, items, stderr};

const PROJECT: &str = "revise-proj";

const SEED_BODY: &str = "The login flow rejects valid tokens when the clock skews.

Repro: post a token minted five seconds in the future.

Cleanup: the retry helper duplicates backoff logic.
";

fn show(plan: &Plan, id: &str) -> Value {
    plan.json(&[
        "review",
        "show",
        id,
        "--format",
        "json",
        "--project",
        PROJECT,
    ])
}

fn comments(review: &Value) -> Vec<Value> {
    items(&review["comments"])
}

fn comment(review: &Value, n: u64) -> Value {
    comments(review)
        .into_iter()
        .find(|c| c["id"] == n)
        .unwrap_or_else(|| panic!("comment {n}: {review}"))
}

fn count(review: &Value, key: &str, value: &str) -> usize {
    comments(review).iter().filter(|c| c[key] == value).count()
}

fn resolutions(review: &Value, state: &str) -> usize {
    comments(review)
        .iter()
        .filter(|c| c["resolution"]["state"] == state)
        .count()
}

fn request_ids(plan: &Plan) -> Vec<String> {
    let v = plan.json(&[
        "review",
        "requests",
        "--format",
        "json",
        "--project",
        PROJECT,
    ]);
    items(&v)
        .iter()
        .filter_map(|r| r["id"].as_str().map(str::to_owned))
        .collect()
}

/// Replaces the task body and commits it; returns the edit's commit.
fn edit(plan: &Plan, body: &str, msg: &str) -> String {
    plan.ok(&[
        "task",
        "update",
        "fix-login",
        "--body",
        body,
        "--no-edit",
        "--project",
        PROJECT,
    ]);
    plan.ok(&["commit", "-m", msg]);
    plan.head()
}

fn address(plan: &Plan, id: &str, n: &str, sha: &str, reply: &str) {
    plan.ok(&[
        "review",
        "update",
        id,
        "--comment",
        n,
        "--status",
        "addressed",
        "--applied-commit",
        sha,
        "--reply",
        reply,
        "--project",
        PROJECT,
    ]);
}

#[test]
fn revision_loop_end_to_end() {
    let plan = Plan::empty();
    plan.ok(&["init"]);
    plan.ok(&["project", "create", PROJECT, "--title", "Revise Project"]);
    plan.ok(&[
        "task",
        "create",
        "fix-login",
        "--title",
        "Fix login",
        "--body",
        SEED_BODY,
        "--no-edit",
        "--project",
        PROJECT,
    ]);
    // The review pins created_commit to a revision holding this body.
    plan.ok(&["commit", "-m", "seed: add fix-login task"]);

    let started = plan.json(&[
        "review",
        "start",
        "--on",
        "task/fix-login",
        "--author",
        "reviewer",
        "--no-edit",
        "--project",
        PROJECT,
        "--format",
        "json",
    ]);
    let id = started["id"].as_str().expect("review id").to_owned();
    let id = id.as_str();
    let add = |quote: Option<&str>, body: &str| {
        let mut args = vec!["review", "comment", id];
        if let Some(q) = quote {
            args.extend(["--quote", q]);
        }
        args.extend(["--body", body, "--no-edit", "--project", PROJECT]);
        plan.ok(&args);
    };
    add(
        Some("rejects valid tokens"),
        "Name the error code the client sees.",
    );
    add(
        None,
        "Overall: the repro section needs the exact curl invocation.",
    );
    add(
        Some("minted five seconds in the future"),
        "Five seconds is too tight — justify the window.",
    );
    add(None, "Also fold the retry-helper cleanup into this task.");
    plan.ok(&[
        "review",
        "submit",
        id,
        "--verdict",
        "request-changes",
        "--body",
        "Tighten the task description before implementation.",
        "--no-edit",
        "--project",
        PROJECT,
    ]);
    assert!(request_ids(&plan).contains(&id.to_owned()));

    // Reword only comment 3's span (its context survives): the anchor drifts.
    edit(
        &plan,
        "The login flow rejects valid tokens when the clock skews.\n\nRepro: post a token minted with generous clock skew.\n\nCleanup: the retry helper duplicates backoff logic.\n",
        "docs(plan): reword the repro",
    );
    let s0 = show(&plan, id);
    assert_eq!(resolutions(&s0, "resolved"), 1, "{s0}");
    assert_eq!(resolutions(&s0, "drifted"), 1, "{s0}");
    assert_eq!(resolutions(&s0, "unresolved"), 2, "{s0}");
    assert_eq!(
        comment(&s0, 3)["resolution"]["quote"],
        "minted five seconds in the future",
        "the drifted resolution carries the quote the reviewer saw"
    );

    // A. The resolved anchor.
    let sha_a = edit(
        &plan,
        "The login flow rejects valid tokens with error AUTH-401 when the clock skews.\n\nRepro: post a token minted with generous clock skew.\n\nCleanup: the retry helper duplicates backoff logic.\n",
        "docs(plan): name the error code",
    );
    address(
        &plan,
        id,
        "1",
        &sha_a,
        "Named the AUTH-401 error code; anchor resolved cleanly.",
    );
    let sa = show(&plan, id);
    assert_eq!(comment(&sa, 1)["applied_commit"], sha_a.as_str());
    assert_eq!(count(&sa, "status", "addressed"), 1);

    // B. The whole-document comment.
    let sha_b = edit(
        &plan,
        "The login flow rejects valid tokens with error AUTH-401 when the clock skews.\n\nRepro: post a token minted with generous clock skew.\n\n    curl -X POST /login -H \"Authorization: Bearer $SKEWED_TOKEN\"\n\nCleanup: the retry helper duplicates backoff logic.\n",
        "docs(plan): add the curl repro",
    );
    assert_ne!(sha_a, sha_b, "each edit has its own commit");
    address(
        &plan,
        id,
        "2",
        &sha_b,
        "Added the exact curl invocation. No anchor was resolved (whole-document comment); applied against the current body.",
    );
    let sb = show(&plan, id);
    assert_eq!(comment(&sb, 2)["applied_commit"], sha_b.as_str());
    assert_eq!(count(&sb, "status", "addressed"), 2);

    // C. The drifted anchor: a clarification reply leaves it open, and
    // closing the review is refused without changing anything.
    let question = "The quoted span changed since the review — should the window be justified in the repro, or relaxed in the fix itself?";
    plan.ok(&[
        "review",
        "update",
        id,
        "--comment",
        "3",
        "--reply",
        question,
        "--project",
        PROJECT,
    ]);
    let sc = show(&plan, id);
    assert_eq!(count(&sc, "status", "open"), 2, "{sc}");
    assert_eq!(comment(&sc, 3)["reply"], question);
    let head = plan.head();
    let refused = plan.run_in(
        plan.dir.path(),
        &[
            "review",
            "update",
            id,
            "--state",
            "addressed",
            "--project",
            PROJECT,
        ],
    );
    assert!(
        !refused.status.success(),
        "closing with open comments is refused"
    );
    assert!(
        stderr(&refused).contains("open comment"),
        "the refusal says comments are still open: {}",
        stderr(&refused)
    );
    let after_refusal = show(&plan, id);
    assert_eq!(after_refusal["state"], "submitted");
    assert_eq!(count(&after_refusal, "status", "open"), 2);
    assert_eq!(plan.head(), head, "the refusal made no plan-repo commit");

    // D. Wont-fix with a reason and no applied commit.
    plan.ok(&[
        "review",
        "update",
        id,
        "--comment",
        "4",
        "--status",
        "wont-fix",
        "--reply",
        "The retry-helper cleanup is unrelated; tracked as its own task.",
        "--project",
        PROJECT,
    ]);
    let sd = show(&plan, id);
    assert_eq!(count(&sd, "status", "wont-fix"), 1);
    assert!(
        comment(&sd, 4)
            .get("applied_commit")
            .is_none_or(Value::is_null),
        "wont-fix records no applied commit: {sd}"
    );
    assert_eq!(
        comments(&sd)
            .iter()
            .filter(|c| c.get("applied_commit").is_some_and(|v| !v.is_null()))
            .count(),
        2
    );

    // E. Resolve the clarified comment, close, and the queue drains.
    let sha_e = edit(
        &plan,
        "The login flow rejects valid tokens with error AUTH-401 when the clock skews.\n\nRepro: post a token minted with generous clock skew (the five-second window\nin the original report was an observation, not a requirement).\n\n    curl -X POST /login -H \"Authorization: Bearer $SKEWED_TOKEN\"\n\nCleanup: the retry helper duplicates backoff logic.\n",
        "docs(plan): justify the window",
    );
    address(
        &plan,
        id,
        "3",
        &sha_e,
        "Reviewer confirmed: justified the window in the repro. Anchor had drifted; edit applied against the current body.",
    );
    plan.ok(&[
        "review",
        "update",
        id,
        "--state",
        "addressed",
        "--project",
        PROJECT,
    ]);
    let se = show(&plan, id);
    assert_eq!(se["state"], "addressed");
    assert_eq!(count(&se, "status", "open"), 0);
    assert_eq!(comment(&se, 3)["applied_commit"], sha_e.as_str());
    assert!(!request_ids(&plan).contains(&id.to_owned()));
}
