//! The backlog grooming loop the `rdm-backlog` skill proposes, driven end to
//! end: `rdm backlog report` surfaces every signal, then each proposed
//! action (`promote --into`, `task merge`, `task update --reason`,
//! `roadmap archive`) is run and read back, and a second report reflects the
//! archive. Pieces overlap `cli_task.rs`'s `promote_into_existing_roadmap`
//! and `task_merge_folds_sources` and `cli_backlog.rs`'s
//! `older_than_zero_flags_fresh`; the report → act → report loop is covered
//! only here.

use serde_json::Value;

use crate::{Plan, field, items};

const PROJECT: &str = "groom-proj";

fn report(plan: &Plan) -> Value {
    plan.json(&[
        "backlog",
        "report",
        "--older-than",
        "0",
        "--format",
        "json",
        "--project",
        PROJECT,
    ])
}

fn task(plan: &Plan, slug: &str) -> Value {
    plan.json(&[
        "task",
        "show",
        slug,
        "--format",
        "json",
        "--project",
        PROJECT,
    ])
}

fn create_task(plan: &Plan, slug: &str, title: &str, extra: &[&str]) {
    let mut args = vec!["task", "create", slug, "--title", title];
    args.extend_from_slice(extra);
    args.extend_from_slice(&["--no-edit", "--project", PROJECT]);
    plan.ok(&args);
}

fn create_roadmap(plan: &Plan, slug: &str, title: &str, body: &str, phase: (&str, &str, &str)) {
    plan.ok(&[
        "roadmap",
        "create",
        slug,
        "--title",
        title,
        "--body",
        body,
        "--no-edit",
        "--project",
        PROJECT,
    ]);
    plan.ok(&[
        "phase",
        "create",
        phase.0,
        "--title",
        phase.1,
        "--number",
        "1",
        "--body",
        phase.2,
        "--no-edit",
        "--roadmap",
        slug,
        "--project",
        PROJECT,
    ]);
}

#[test]
fn groom_loop_end_to_end() {
    let plan = Plan::empty();
    plan.ok(&["init", "--default-project", PROJECT]);
    // A duplicate pair with distinct tags, so the merge's tag union shows.
    create_task(
        &plan,
        "dup-a",
        "Fix login bug on mobile",
        &["--tags", "bug"],
    );
    create_task(
        &plan,
        "dup-b",
        "Fix login bug on mobile devices",
        &["--tags", "mobile"],
    );
    // A tag cluster of unrelated titles.
    create_task(
        &plan,
        "tag-a",
        "Refactor the settings loader",
        &["--tags", "cluster-tag"],
    );
    create_task(
        &plan,
        "tag-b",
        "Document the export pipeline",
        &["--tags", "cluster-tag"],
    );
    create_task(&plan, "stale-one", "Stale One", &["--body", "Retire me."]);
    create_roadmap(
        &plan,
        "existing-rm",
        "Existing Roadmap",
        "An existing roadmap.",
        ("seed", "Seed", "seed phase"),
    );
    create_task(
        &plan,
        "consolidate-me",
        "Consolidate Me",
        &["--body", "Body of consolidate-me task."],
    );
    create_roadmap(
        &plan,
        "terminal-rm",
        "Terminal Roadmap",
        "A finished roadmap.",
        ("only", "Only", "the only phase"),
    );
    plan.ok(&[
        "phase",
        "update",
        "phase-1-only",
        "--status",
        "done",
        "--no-edit",
        "--roadmap",
        "terminal-rm",
        "--project",
        PROJECT,
    ]);
    plan.ok(&["commit", "-m", "seed: groom fixtures"]);

    // 1. The report surfaces every signal, each under its own key.
    let before = report(&plan);
    assert!(field(&before["stale_tasks"], "slug").contains(&"dup-a".to_owned()));
    let dup_members: Vec<Vec<String>> = items(&before["duplicate_clusters"])
        .iter()
        .map(|c| field(&c["members"], "slug"))
        .collect();
    assert!(
        dup_members
            .iter()
            .any(|m| m.contains(&"dup-a".to_owned()) && m.contains(&"dup-b".to_owned())),
        "one duplicate cluster holds dup-a and dup-b: {before}"
    );
    let cluster = items(&before["tag_clusters"])
        .into_iter()
        .find(|c| c["tag"] == "cluster-tag")
        .unwrap_or_else(|| panic!("a cluster-tag tag cluster: {before}"));
    let clustered = field(&cluster["tasks"], "slug");
    assert!(clustered.contains(&"tag-a".to_owned()) && clustered.contains(&"tag-b".to_owned()));
    assert!(field(&before["archivable_roadmaps"], "roadmap").contains(&"terminal-rm".to_owned()));

    // 2. Consolidate a task into an existing roadmap as a new phase.
    let promoted = plan.ok(&[
        "promote",
        "consolidate-me",
        "--into",
        "existing-rm",
        "--no-edit",
        "--project",
        PROJECT,
    ]);
    assert!(
        promoted.lines().any(|l| l
            == "Consolidated task 'consolidate-me' → roadmap 'existing-rm' as phase 'phase-2-consolidate-me' (task status: done)"),
        "promote --into confirms the consolidation: {promoted}"
    );
    let existing = plan.json(&[
        "roadmap",
        "show",
        "existing-rm",
        "--format",
        "json",
        "--project",
        PROJECT,
    ]);
    assert!(
        field(&existing["phases"], "stem").contains(&"phase-2-consolidate-me".to_owned()),
        "{existing}"
    );
    let consolidated = task(&plan, "consolidate-me");
    assert_eq!(consolidated["status"], "done");
    assert!(
        consolidated["body"]
            .as_str()
            .is_some_and(|b| b.contains("Consolidated into roadmap ")),
        "the closed task points at its roadmap: {consolidated}"
    );

    // 3. Merge the duplicate into its twin.
    let merged = plan.ok(&[
        "task",
        "merge",
        "dup-a",
        "--from",
        "dup-b",
        "--no-edit",
        "--project",
        PROJECT,
    ]);
    assert!(
        merged
            .lines()
            .any(|l| l.starts_with("Merged 1 task(s) into 'dup-a'")),
        "task merge confirms the merge: {merged}"
    );
    let dup_a = task(&plan, "dup-a");
    let tags: Vec<String> = dup_a["tags"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| t.as_str().map(str::to_owned))
        .collect();
    assert!(tags.contains(&"bug".to_owned()) && tags.contains(&"mobile".to_owned()));
    assert!(
        dup_a["body"]
            .as_str()
            .is_some_and(|b| b.contains("## Merged from task `dup-b`")),
        "the survivor records the merge: {dup_a}"
    );
    let dup_b = task(&plan, "dup-b");
    assert_eq!(dup_b["status"], "wont-fix");
    assert!(
        dup_b["close_reason"]
            .as_str()
            .is_some_and(|r| r.contains("superseded by task/dup-a")),
        "{dup_b}"
    );

    // 4. Retire a stale task with a reason.
    plan.ok(&[
        "task",
        "update",
        "stale-one",
        "--status",
        "wont-fix",
        "--reason",
        "no longer relevant",
        "--no-edit",
        "--project",
        PROJECT,
    ]);
    let stale = task(&plan, "stale-one");
    assert_eq!(stale["status"], "wont-fix");
    assert_eq!(stale["close_reason"], "no longer relevant");

    // 5. Archive the terminal roadmap; the next report no longer offers it.
    let archived = plan.ok(&["roadmap", "archive", "terminal-rm", "--project", PROJECT]);
    assert!(
        archived
            .lines()
            .any(|l| l.starts_with("Archived roadmap 'terminal-rm' from project 'groom-proj'")),
        "roadmap archive confirms: {archived}"
    );
    let archived_list = plan.json(&[
        "roadmap",
        "list",
        "--archived",
        "--format",
        "json",
        "--project",
        PROJECT,
    ]);
    assert!(field(&archived_list, "slug").contains(&"terminal-rm".to_owned()));
    let active = plan.json(&["roadmap", "list", "--format", "json", "--project", PROJECT]);
    assert!(!field(&active, "slug").contains(&"terminal-rm".to_owned()));
    let after = report(&plan);
    assert!(
        !field(&after["archivable_roadmaps"], "roadmap").contains(&"terminal-rm".to_owned()),
        "{after}"
    );
}
