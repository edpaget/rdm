//! The `--format json` stdout contract, checked against a seeded fixture:
//! every command in [`COVERED`] prints exactly one JSON document, and each
//! `create` prints a subset of what the matching `show` prints for the same
//! item.

use std::collections::BTreeSet;

use serde_json::Value;

use crate::seeded_plan::{PROJECT, SeededPlan};

/// A row takes `--project <PROJECT>` after `--format json`.
const PROJECT_SCOPED: bool = true;
/// A row acts on the whole plan repo and rejects `--project`.
const REPO_WIDE: bool = false;

/// Commands whose `--format json` stdout must be exactly one JSON document,
/// by name, arguments and whether `--project <PROJECT>` is appended after
/// `--format json`. Rows run in order against one fixture, so `status`,
/// `commit` and `discard` see the creates above them staged.
const COVERED: &[(&str, &[&str], bool)] = &[
    (
        "roadmap create",
        &[
            "roadmap",
            "create",
            "contract-roadmap",
            "--title",
            "Contract Roadmap",
            "--no-edit",
        ],
        PROJECT_SCOPED,
    ),
    (
        "phase create",
        &[
            "phase",
            "create",
            "contract-phase",
            "--roadmap",
            "contract-roadmap",
            "--title",
            "Contract Phase",
            "--no-edit",
        ],
        PROJECT_SCOPED,
    ),
    (
        "task create",
        &[
            "task",
            "create",
            "contract-task",
            "--title",
            "Contract Task",
            "--no-edit",
        ],
        PROJECT_SCOPED,
    ),
    ("status", &["status"], REPO_WIDE),
    ("status --all", &["status", "--all"], REPO_WIDE),
    ("commit", &["commit", "-m", "contract commit"], REPO_WIDE),
    ("discard --force", &["discard", "--force"], REPO_WIDE),
];

fn fixture() -> SeededPlan {
    SeededPlan::new(PROJECT).unwrap_or_else(|f| panic!("{f}"))
}

fn with_json(args: &[&str], project: bool) -> Vec<String> {
    let scope: &[&str] = if project {
        &["--project", PROJECT]
    } else {
        &[]
    };
    args.iter()
        .copied()
        .chain(["--format", "json"])
        .chain(scope.iter().copied())
        .map(str::to_owned)
        .collect()
}

fn run_json(fixture: &SeededPlan, name: &str, args: &[&str], project: bool) -> String {
    let args = with_json(args, project);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    fixture
        .ok(&args)
        .unwrap_or_else(|f| panic!("`{name}` failed: {f}"))
}

/// Parses `stdout` as exactly one JSON value followed only by whitespace.
fn single_document(name: &str, stdout: &str) -> Value {
    let mut values = serde_json::Deserializer::from_str(stdout).into_iter::<Value>();
    let first = match values.next() {
        Some(Ok(v)) => v,
        Some(Err(e)) => panic!("`{name}` stdout is not JSON: {e}: {stdout:?}"),
        None => panic!("`{name}` printed no JSON document: {stdout:?}"),
    };
    if let Some(extra) = values.next() {
        panic!("`{name}` printed more than one JSON document (next: {extra:?}): {stdout:?}");
    }
    first
}

fn keys(name: &str, v: &Value) -> BTreeSet<String> {
    v.as_object()
        .unwrap_or_else(|| panic!("`{name}` did not print a JSON object: {v}"))
        .keys()
        .cloned()
        .collect()
}

#[test]
fn covered_commands_print_exactly_one_json_document() {
    let fixture = fixture();
    for (name, args, project) in COVERED {
        let stdout = run_json(&fixture, name, args, *project);
        single_document(name, &stdout);
    }
}

/// Runs `create`, then `show`, and checks the create's keys are a subset of
/// the show's and that every `identity` field is present and equal in both.
fn assert_create_matches_show(
    fixture: &SeededPlan,
    name: &str,
    create: &[&str],
    show: &[&str],
    identity: &[(&str, Value)],
) {
    let created = single_document(name, &run_json(fixture, name, create, PROJECT_SCOPED));
    let shown = single_document(name, &run_json(fixture, name, show, PROJECT_SCOPED));
    let (created_keys, shown_keys) = (keys(name, &created), keys(name, &shown));
    let extra: Vec<_> = created_keys.difference(&shown_keys).collect();
    assert!(
        extra.is_empty(),
        "`{name}` json has keys `show` lacks: {extra:?}"
    );
    for (field, expected) in identity {
        assert_eq!(&created[*field], expected, "`{name}` json `{field}`");
        assert_eq!(
            created[*field], shown[*field],
            "`{name}` json `{field}` differs from show"
        );
    }
}

#[test]
fn create_json_is_a_subset_of_show_json() {
    let fixture = fixture();
    assert_create_matches_show(
        &fixture,
        "roadmap create",
        &[
            "roadmap",
            "create",
            "subset-roadmap",
            "--title",
            "Subset Roadmap",
            "--no-edit",
        ],
        &["roadmap", "show", "subset-roadmap"],
        &[
            ("project", Value::from(PROJECT)),
            ("slug", Value::from("subset-roadmap")),
            ("title", Value::from("Subset Roadmap")),
        ],
    );
    assert_create_matches_show(
        &fixture,
        "phase create",
        &[
            "phase",
            "create",
            "subset-phase",
            "--number",
            "2",
            "--roadmap",
            "subset-roadmap",
            "--title",
            "Subset Phase",
            "--no-edit",
        ],
        &[
            "phase",
            "show",
            "phase-2-subset-phase",
            "--roadmap",
            "subset-roadmap",
        ],
        &[
            ("roadmap", Value::from("subset-roadmap")),
            ("stem", Value::from("phase-2-subset-phase")),
            ("phase", Value::from(2)),
            ("title", Value::from("Subset Phase")),
        ],
    );
    assert_create_matches_show(
        &fixture,
        "task create",
        &[
            "task",
            "create",
            "subset-task",
            "--title",
            "Subset Task",
            "--no-edit",
        ],
        &["task", "show", "subset-task"],
        &[
            ("project", Value::from(PROJECT)),
            ("slug", Value::from("subset-task")),
            ("title", Value::from("Subset Task")),
        ],
    );
}
