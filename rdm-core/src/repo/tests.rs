use super::*;
use crate::model::*;
use crate::store::MemoryStore;
use chrono::NaiveDate;

fn make_repo() -> PlanRepo<MemoryStore> {
    PlanRepo::new(MemoryStore::new())
}

// -- Write + Load round-trip tests --

#[test]
fn write_and_load_roadmap() {
    let mut repo = make_repo();
    let doc = Document {
        frontmatter: Roadmap {
            project: "fbm".to_string(),
            roadmap: "two-way-players".to_string(),
            title: "Two-Way Player Identity".to_string(),
            phases: vec![
                "phase-1-core-valuation".to_string(),
                "phase-2-keeper-service".to_string(),
            ],
            dependencies: Some(vec!["keeper-surplus-value".to_string()]),
        },
        body: "Summary here.\n".to_string(),
    };
    repo.write_roadmap("fbm", "two-way-players", &doc).unwrap();
    let loaded = repo.load_roadmap("fbm", "two-way-players").unwrap();
    assert_eq!(loaded.frontmatter, doc.frontmatter);
    assert_eq!(loaded.body, doc.body);
}

#[test]
fn write_and_load_phase() {
    let mut repo = make_repo();
    let doc = Document {
        frontmatter: Phase {
            phase: 1,
            title: "Core valuation layer".to_string(),
            status: PhaseStatus::Done,
            completed: Some(NaiveDate::from_ymd_opt(2026, 3, 13).unwrap()),
            commit: None,
        },
        body: "## Steps\n\n1. Do things.\n".to_string(),
    };
    repo.write_phase("fbm", "two-way-players", "phase-1-core-valuation", &doc)
        .unwrap();
    let loaded = repo
        .load_phase("fbm", "two-way-players", "phase-1-core-valuation")
        .unwrap();
    assert_eq!(loaded.frontmatter, doc.frontmatter);
    assert_eq!(loaded.body, doc.body);
}

#[test]
fn write_and_load_task() {
    let mut repo = make_repo();
    let doc = Document {
        frontmatter: Task {
            project: "fbm".to_string(),
            title: "Fix barrel column".to_string(),
            status: TaskStatus::Open,
            priority: Priority::High,
            created: NaiveDate::from_ymd_opt(2026, 3, 14).unwrap(),
            tags: Some(vec!["data".to_string()]),
            completed: None,
            commit: None,
        },
        body: "Details.\n".to_string(),
    };
    repo.write_task("fbm", "fix-barrel-nulls", &doc).unwrap();
    let loaded = repo.load_task("fbm", "fix-barrel-nulls").unwrap();
    assert_eq!(loaded.frontmatter, doc.frontmatter);
    assert_eq!(loaded.body, doc.body);
}

#[test]
fn load_project_success() {
    let mut repo = PlanRepo::init(MemoryStore::new()).unwrap();
    repo.create_project("fbm", "Fantasy Baseball Manager")
        .unwrap();
    let doc = repo.load_project("fbm").unwrap();
    assert_eq!(doc.frontmatter.name, "fbm");
    assert_eq!(doc.frontmatter.title, "Fantasy Baseball Manager");
}

#[test]
fn load_project_not_found() {
    let repo = PlanRepo::init(MemoryStore::new()).unwrap();
    let result = repo.load_project("nonexistent");
    assert!(matches!(result, Err(Error::ProjectNotFound(ref s)) if s == "nonexistent"));
}

#[test]
fn load_roadmap_not_found() {
    let repo = make_repo();
    let result = repo.load_roadmap("fbm", "nonexistent");
    assert!(matches!(result, Err(Error::RoadmapNotFound(ref s)) if s == "nonexistent"));
}

#[test]
fn load_task_not_found() {
    let repo = make_repo();
    let result = repo.load_task("fbm", "does-not-exist");
    assert!(matches!(result, Err(Error::TaskNotFound(ref s)) if s == "does-not-exist"));
}

// -- Init tests --

#[test]
fn init_creates_structure() {
    let repo = PlanRepo::init(MemoryStore::new()).unwrap();

    assert!(repo.store().exists(&crate::paths::config_path()));
    assert!(repo.store().exists(&crate::paths::index_path()));

    // Config should be parseable
    let toml_str = repo.store().read(&crate::paths::config_path()).unwrap();
    Config::from_toml(&toml_str).unwrap();
}

#[test]
fn load_config_after_init() {
    let repo = PlanRepo::init(MemoryStore::new()).unwrap();
    let config = repo.load_config().unwrap();
    assert_eq!(config.default_project, None);
}

#[test]
fn load_config_not_found() {
    let repo = make_repo();
    let result = repo.load_config();
    assert!(matches!(result, Err(Error::ConfigNotFound)));
}

// -- Project tests --

#[test]
fn create_project_success() {
    let mut repo = PlanRepo::init(MemoryStore::new()).unwrap();
    repo.create_project("fbm", "Fantasy Baseball Manager")
        .unwrap();

    assert!(repo.store().exists(&crate::paths::project_md_path("fbm")));
}

#[test]
fn create_project_duplicate() {
    let mut repo = PlanRepo::init(MemoryStore::new()).unwrap();
    repo.create_project("fbm", "Fantasy Baseball Manager")
        .unwrap();
    let result = repo.create_project("fbm", "Duplicate");
    assert!(matches!(result, Err(Error::DuplicateSlug(ref s)) if s == "fbm"));
}

#[test]
fn list_projects_empty() {
    let repo = PlanRepo::init(MemoryStore::new()).unwrap();
    assert_eq!(repo.list_projects().unwrap(), Vec::<String>::new());
}

#[test]
fn list_projects_sorted() {
    let mut repo = PlanRepo::init(MemoryStore::new()).unwrap();
    repo.create_project("zzz", "Last").unwrap();
    repo.create_project("aaa", "First").unwrap();
    repo.create_project("mmm", "Middle").unwrap();
    let projects = repo.list_projects().unwrap();
    assert_eq!(projects, vec!["aaa", "mmm", "zzz"]);
}

// -- Roadmap tests --

#[test]
fn create_roadmap_success() {
    let mut repo = PlanRepo::init(MemoryStore::new()).unwrap();
    repo.create_project("fbm", "FBM").unwrap();
    let doc = repo
        .create_roadmap("fbm", "two-way", "Two-Way Players", None)
        .unwrap();
    assert_eq!(doc.frontmatter.project, "fbm");
    assert_eq!(doc.frontmatter.roadmap, "two-way");
    assert_eq!(doc.frontmatter.title, "Two-Way Players");
    assert!(doc.frontmatter.phases.is_empty());

    // Should be loadable
    let loaded = repo.load_roadmap("fbm", "two-way").unwrap();
    assert_eq!(loaded.frontmatter, doc.frontmatter);
}

#[test]
fn create_roadmap_with_body() {
    let mut repo = PlanRepo::init(MemoryStore::new()).unwrap();
    repo.create_project("fbm", "FBM").unwrap();
    let body = "# Description\n\nA roadmap for two-way players.\n";
    let doc = repo
        .create_roadmap("fbm", "two-way", "Two-Way Players", Some(body))
        .unwrap();
    assert_eq!(doc.body, body);

    let loaded = repo.load_roadmap("fbm", "two-way").unwrap();
    assert_eq!(loaded.body, body);
}

#[test]
fn create_roadmap_project_not_found() {
    let mut repo = PlanRepo::init(MemoryStore::new()).unwrap();
    let result = repo.create_roadmap("nope", "slug", "Title", None);
    assert!(matches!(result, Err(Error::ProjectNotFound(_))));
}

#[test]
fn create_roadmap_duplicate() {
    let mut repo = PlanRepo::init(MemoryStore::new()).unwrap();
    repo.create_project("fbm", "FBM").unwrap();
    repo.create_roadmap("fbm", "two-way", "Two-Way Players", None)
        .unwrap();
    let result = repo.create_roadmap("fbm", "two-way", "Dup", None);
    assert!(matches!(result, Err(Error::DuplicateSlug(_))));
}

#[test]
fn update_roadmap_body_replaces_existing() {
    let mut repo = PlanRepo::init(MemoryStore::new()).unwrap();
    repo.create_project("fbm", "FBM").unwrap();
    repo.create_roadmap("fbm", "two-way", "Two-Way", Some("Original.\n"))
        .unwrap();
    let updated = repo
        .update_roadmap("fbm", "two-way", Some("Replaced.\n"))
        .unwrap();
    assert_eq!(updated.body, "Replaced.\n");

    let loaded = repo.load_roadmap("fbm", "two-way").unwrap();
    assert_eq!(loaded.body, "Replaced.\n");
}

#[test]
fn update_roadmap_none_body_preserves_existing() {
    let mut repo = PlanRepo::init(MemoryStore::new()).unwrap();
    repo.create_project("fbm", "FBM").unwrap();
    repo.create_roadmap("fbm", "two-way", "Two-Way", Some("Keep this.\n"))
        .unwrap();
    let updated = repo.update_roadmap("fbm", "two-way", None).unwrap();
    assert_eq!(updated.body, "Keep this.\n");
}

#[test]
fn update_roadmap_not_found() {
    let mut repo = PlanRepo::init(MemoryStore::new()).unwrap();
    repo.create_project("fbm", "FBM").unwrap();
    let result = repo.update_roadmap("fbm", "nope", Some("body"));
    assert!(matches!(result, Err(Error::RoadmapNotFound(_))));
}

#[test]
fn list_roadmaps_sorted() {
    let mut repo = PlanRepo::init(MemoryStore::new()).unwrap();
    repo.create_project("fbm", "FBM").unwrap();
    repo.create_roadmap("fbm", "zzz-road", "Z", None).unwrap();
    repo.create_roadmap("fbm", "aaa-road", "A", None).unwrap();
    let roadmaps = repo.list_roadmaps("fbm").unwrap();
    assert_eq!(roadmaps.len(), 2);
    assert_eq!(roadmaps[0].frontmatter.roadmap, "aaa-road");
    assert_eq!(roadmaps[1].frontmatter.roadmap, "zzz-road");
}

#[test]
fn list_roadmaps_empty() {
    let mut repo = PlanRepo::init(MemoryStore::new()).unwrap();
    repo.create_project("fbm", "FBM").unwrap();
    let roadmaps = repo.list_roadmaps("fbm").unwrap();
    assert!(roadmaps.is_empty());
}

#[test]
fn list_roadmaps_project_not_found() {
    let repo = PlanRepo::init(MemoryStore::new()).unwrap();
    let result = repo.list_roadmaps("nope");
    assert!(matches!(result, Err(Error::ProjectNotFound(_))));
}

// -- Phase tests --

fn setup_with_roadmap() -> PlanRepo<MemoryStore> {
    let mut repo = PlanRepo::init(MemoryStore::new()).unwrap();
    repo.create_project("fbm", "FBM").unwrap();
    repo.create_roadmap("fbm", "two-way", "Two-Way Players", None)
        .unwrap();
    repo
}

#[test]
fn create_phase_auto_number() {
    let mut repo = setup_with_roadmap();
    let doc = repo
        .create_phase("fbm", "two-way", "core", "Core Valuation", None, None)
        .unwrap();
    assert_eq!(doc.frontmatter.phase, 1);
    assert_eq!(doc.frontmatter.status, PhaseStatus::NotStarted);

    let doc2 = repo
        .create_phase("fbm", "two-way", "service", "Keeper Service", None, None)
        .unwrap();
    assert_eq!(doc2.frontmatter.phase, 2);

    // Verify roadmap phases list was updated
    let roadmap = repo.load_roadmap("fbm", "two-way").unwrap();
    assert_eq!(
        roadmap.frontmatter.phases,
        vec!["phase-1-core", "phase-2-service"]
    );
}

#[test]
fn create_phase_explicit_number() {
    let mut repo = setup_with_roadmap();
    let doc = repo
        .create_phase("fbm", "two-way", "core", "Core", Some(5), None)
        .unwrap();
    assert_eq!(doc.frontmatter.phase, 5);

    // Stem should be phase-5-core
    let loaded = repo.load_phase("fbm", "two-way", "phase-5-core").unwrap();
    assert_eq!(loaded.frontmatter, doc.frontmatter);
}

#[test]
fn create_phase_with_body() {
    let mut repo = setup_with_roadmap();
    let body = "## Acceptance Criteria\n\n- [ ] Criterion one\n- [ ] Criterion two\n";
    let doc = repo
        .create_phase("fbm", "two-way", "core", "Core", None, Some(body))
        .unwrap();
    assert_eq!(doc.body, body);

    let loaded = repo.load_phase("fbm", "two-way", "phase-1-core").unwrap();
    assert_eq!(loaded.body, body);
}

#[test]
fn create_phase_roadmap_not_found() {
    let mut repo = PlanRepo::init(MemoryStore::new()).unwrap();
    repo.create_project("fbm", "FBM").unwrap();
    let result = repo.create_phase("fbm", "nope", "s", "T", None, None);
    assert!(matches!(result, Err(Error::RoadmapNotFound(_))));
}

#[test]
fn list_phases_sorted() {
    let mut repo = setup_with_roadmap();
    repo.create_phase("fbm", "two-way", "core", "Core", Some(2), None)
        .unwrap();
    repo.create_phase("fbm", "two-way", "service", "Service", Some(1), None)
        .unwrap();
    let phases = repo.list_phases("fbm", "two-way").unwrap();
    assert_eq!(phases.len(), 2);
    assert_eq!(phases[0].1.frontmatter.phase, 1);
    assert_eq!(phases[1].1.frontmatter.phase, 2);
}

#[test]
fn update_phase_to_done_sets_completed() {
    let mut repo = setup_with_roadmap();
    repo.create_phase("fbm", "two-way", "core", "Core", None, None)
        .unwrap();
    let updated = repo
        .update_phase(
            "fbm",
            "two-way",
            "phase-1-core",
            Some(PhaseStatus::Done),
            None,
            None,
        )
        .unwrap();
    assert_eq!(updated.frontmatter.status, PhaseStatus::Done);
    assert!(updated.frontmatter.completed.is_some());
    assert_eq!(updated.frontmatter.commit, None);
}

#[test]
fn update_phase_to_done_with_commit_stores_sha() {
    let mut repo = setup_with_roadmap();
    repo.create_phase("fbm", "two-way", "core", "Core", None, None)
        .unwrap();
    let updated = repo
        .update_phase(
            "fbm",
            "two-way",
            "phase-1-core",
            Some(PhaseStatus::Done),
            None,
            Some("abc123".to_string()),
        )
        .unwrap();
    assert_eq!(updated.frontmatter.status, PhaseStatus::Done);
    assert!(updated.frontmatter.completed.is_some());
    assert_eq!(updated.frontmatter.commit, Some("abc123".to_string()));

    // Verify persistence
    let loaded = repo.load_phase("fbm", "two-way", "phase-1-core").unwrap();
    assert_eq!(loaded.frontmatter.commit, Some("abc123".to_string()));
}

#[test]
fn update_phase_from_done_clears_completed() {
    let mut repo = setup_with_roadmap();
    repo.create_phase("fbm", "two-way", "core", "Core", None, None)
        .unwrap();
    repo.update_phase(
        "fbm",
        "two-way",
        "phase-1-core",
        Some(PhaseStatus::Done),
        None,
        Some("abc123".to_string()),
    )
    .unwrap();
    let updated = repo
        .update_phase(
            "fbm",
            "two-way",
            "phase-1-core",
            Some(PhaseStatus::InProgress),
            None,
            None,
        )
        .unwrap();
    assert_eq!(updated.frontmatter.status, PhaseStatus::InProgress);
    assert_eq!(updated.frontmatter.completed, None);
    assert_eq!(updated.frontmatter.commit, None);
}

#[test]
fn update_phase_body_replaces_existing() {
    let mut repo = setup_with_roadmap();
    repo.create_phase(
        "fbm",
        "two-way",
        "core",
        "Core",
        None,
        Some("Original body.\n"),
    )
    .unwrap();
    let updated = repo
        .update_phase(
            "fbm",
            "two-way",
            "phase-1-core",
            Some(PhaseStatus::InProgress),
            Some("Replaced body.\n"),
            None,
        )
        .unwrap();
    assert_eq!(updated.body, "Replaced body.\n");

    let loaded = repo.load_phase("fbm", "two-way", "phase-1-core").unwrap();
    assert_eq!(loaded.body, "Replaced body.\n");
}

#[test]
fn update_phase_none_body_preserves_existing() {
    let mut repo = setup_with_roadmap();
    repo.create_phase(
        "fbm",
        "two-way",
        "core",
        "Core",
        None,
        Some("Keep this body.\n"),
    )
    .unwrap();
    let updated = repo
        .update_phase(
            "fbm",
            "two-way",
            "phase-1-core",
            Some(PhaseStatus::InProgress),
            None,
            None,
        )
        .unwrap();
    assert_eq!(updated.body, "Keep this body.\n");
}

#[test]
fn update_phase_not_found() {
    let mut repo = setup_with_roadmap();
    let result = repo.update_phase(
        "fbm",
        "two-way",
        "phase-99-nope",
        Some(PhaseStatus::Done),
        None,
        None,
    );
    assert!(matches!(result, Err(Error::PhaseNotFound(_))));
}

#[test]
fn update_phase_done_to_done_with_new_commit_updates_sha() {
    let mut repo = setup_with_roadmap();
    repo.create_phase("fbm", "two-way", "core", "Core", None, None)
        .unwrap();
    let first = repo
        .update_phase(
            "fbm",
            "two-way",
            "phase-1-core",
            Some(PhaseStatus::Done),
            None,
            Some("abc123".to_string()),
        )
        .unwrap();
    let first_completed = first.frontmatter.completed;

    let updated = repo
        .update_phase(
            "fbm",
            "two-way",
            "phase-1-core",
            Some(PhaseStatus::Done),
            None,
            Some("def456".to_string()),
        )
        .unwrap();
    assert_eq!(updated.frontmatter.status, PhaseStatus::Done);
    assert_eq!(updated.frontmatter.commit, Some("def456".to_string()));
    assert_eq!(updated.frontmatter.completed, first_completed);
}

#[test]
fn update_phase_done_to_done_without_commit_is_noop() {
    let mut repo = setup_with_roadmap();
    repo.create_phase("fbm", "two-way", "core", "Core", None, None)
        .unwrap();
    let first = repo
        .update_phase(
            "fbm",
            "two-way",
            "phase-1-core",
            Some(PhaseStatus::Done),
            None,
            Some("abc123".to_string()),
        )
        .unwrap();
    let first_completed = first.frontmatter.completed;

    let updated = repo
        .update_phase(
            "fbm",
            "two-way",
            "phase-1-core",
            Some(PhaseStatus::Done),
            None,
            None,
        )
        .unwrap();
    assert_eq!(updated.frontmatter.status, PhaseStatus::Done);
    assert_eq!(updated.frontmatter.commit, Some("abc123".to_string()));
    assert_eq!(updated.frontmatter.completed, first_completed);
}

#[test]
fn resolve_by_number() {
    let mut repo = setup_with_roadmap();
    repo.create_phase("fbm", "two-way", "core", "Core", Some(1), None)
        .unwrap();
    repo.create_phase("fbm", "two-way", "service", "Service", Some(2), None)
        .unwrap();
    let stem = repo.resolve_phase_stem("fbm", "two-way", "2").unwrap();
    assert_eq!(stem, "phase-2-service");
}

#[test]
fn resolve_by_stem_passthrough() {
    let repo = setup_with_roadmap();
    let stem = repo
        .resolve_phase_stem("fbm", "two-way", "phase-1-core")
        .unwrap();
    assert_eq!(stem, "phase-1-core");
}

#[test]
fn resolve_number_not_found() {
    let mut repo = setup_with_roadmap();
    repo.create_phase("fbm", "two-way", "core", "Core", Some(1), None)
        .unwrap();
    let result = repo.resolve_phase_stem("fbm", "two-way", "99");
    assert!(matches!(result, Err(Error::PhaseNotFound(ref s)) if s == "99"));
}

// -- Remove phase tests --

#[test]
fn remove_phase_deletes_file() {
    let mut repo = setup_with_roadmap();
    repo.create_phase("fbm", "two-way", "core", "Core", None, None)
        .unwrap();
    let path = crate::paths::phase_path("fbm", "two-way", "phase-1-core");
    assert!(repo.store().exists(&path));

    repo.remove_phase("fbm", "two-way", "phase-1-core").unwrap();
    assert!(!repo.store().exists(&path));
}

#[test]
fn remove_phase_updates_roadmap() {
    let mut repo = setup_with_roadmap();
    repo.create_phase("fbm", "two-way", "core", "Core", None, None)
        .unwrap();
    repo.create_phase("fbm", "two-way", "service", "Service", None, None)
        .unwrap();

    repo.remove_phase("fbm", "two-way", "phase-1-core").unwrap();

    let roadmap = repo.load_roadmap("fbm", "two-way").unwrap();
    assert_eq!(roadmap.frontmatter.phases, vec!["phase-2-service"]);
}

#[test]
fn remove_phase_not_found() {
    let mut repo = setup_with_roadmap();
    let result = repo.remove_phase("fbm", "two-way", "phase-99-nope");
    assert!(matches!(result, Err(Error::PhaseNotFound(ref s)) if s == "phase-99-nope"));
}

// -- Task tests --

fn setup_with_project() -> PlanRepo<MemoryStore> {
    let mut repo = PlanRepo::init(MemoryStore::new()).unwrap();
    repo.create_project("fbm", "FBM").unwrap();
    repo
}

#[test]
fn create_task_success() {
    let mut repo = setup_with_project();
    let doc = repo
        .create_task("fbm", "fix-bug", "Fix the bug", Priority::High, None, None)
        .unwrap();
    assert_eq!(doc.frontmatter.title, "Fix the bug");
    assert_eq!(doc.frontmatter.status, TaskStatus::Open);
    assert_eq!(doc.frontmatter.priority, Priority::High);
    assert!(doc.frontmatter.tags.is_none());

    // Should be loadable
    let loaded = repo.load_task("fbm", "fix-bug").unwrap();
    assert_eq!(loaded.frontmatter, doc.frontmatter);
}

#[test]
fn create_task_with_tags() {
    let mut repo = setup_with_project();
    let doc = repo
        .create_task(
            "fbm",
            "fix-bug",
            "Fix the bug",
            Priority::High,
            Some(vec!["bug".to_string(), "urgent".to_string()]),
            None,
        )
        .unwrap();
    assert_eq!(
        doc.frontmatter.tags,
        Some(vec!["bug".to_string(), "urgent".to_string()])
    );
}

#[test]
fn create_task_with_body() {
    let mut repo = setup_with_project();
    let body = "## Notes\n\nSome detailed task notes.\n";
    let doc = repo
        .create_task("fbm", "fix-bug", "Fix", Priority::High, None, Some(body))
        .unwrap();
    assert_eq!(doc.body, body);

    let loaded = repo.load_task("fbm", "fix-bug").unwrap();
    assert_eq!(loaded.body, body);
}

#[test]
fn create_task_project_not_found() {
    let mut repo = PlanRepo::init(MemoryStore::new()).unwrap();
    let result = repo.create_task("nope", "slug", "Title", Priority::Low, None, None);
    assert!(matches!(result, Err(Error::ProjectNotFound(_))));
}

#[test]
fn create_task_duplicate() {
    let mut repo = setup_with_project();
    repo.create_task("fbm", "fix-bug", "Fix", Priority::Low, None, None)
        .unwrap();
    let result = repo.create_task("fbm", "fix-bug", "Dup", Priority::Low, None, None);
    assert!(matches!(result, Err(Error::DuplicateSlug(_))));
}

#[test]
fn list_tasks_sorted() {
    let mut repo = setup_with_project();
    repo.create_task("fbm", "zzz-task", "Z", Priority::Low, None, None)
        .unwrap();
    repo.create_task("fbm", "aaa-task", "A", Priority::High, None, None)
        .unwrap();
    let tasks = repo.list_tasks("fbm").unwrap();
    assert_eq!(tasks.len(), 2);
    assert_eq!(tasks[0].0, "aaa-task");
    assert_eq!(tasks[1].0, "zzz-task");
}

#[test]
fn list_tasks_empty() {
    let repo = setup_with_project();
    let tasks = repo.list_tasks("fbm").unwrap();
    assert!(tasks.is_empty());
}

#[test]
fn list_tasks_project_not_found() {
    let repo = PlanRepo::init(MemoryStore::new()).unwrap();
    let result = repo.list_tasks("nonexistent");
    assert!(matches!(result, Err(Error::ProjectNotFound(_))));
}

#[test]
fn update_task_status() {
    let mut repo = setup_with_project();
    repo.create_task("fbm", "fix-bug", "Fix", Priority::Low, None, None)
        .unwrap();
    let updated = repo
        .update_task(
            "fbm",
            "fix-bug",
            Some(TaskStatus::Done),
            None,
            None,
            None,
            None,
        )
        .unwrap();
    assert_eq!(updated.frontmatter.status, TaskStatus::Done);

    let loaded = repo.load_task("fbm", "fix-bug").unwrap();
    assert_eq!(loaded.frontmatter.status, TaskStatus::Done);
}

#[test]
fn update_task_priority() {
    let mut repo = setup_with_project();
    repo.create_task("fbm", "fix-bug", "Fix", Priority::Low, None, None)
        .unwrap();
    let updated = repo
        .update_task(
            "fbm",
            "fix-bug",
            None,
            Some(Priority::Critical),
            None,
            None,
            None,
        )
        .unwrap();
    assert_eq!(updated.frontmatter.priority, Priority::Critical);
}

#[test]
fn update_task_tags() {
    let mut repo = setup_with_project();
    repo.create_task("fbm", "fix-bug", "Fix", Priority::Low, None, None)
        .unwrap();
    let updated = repo
        .update_task(
            "fbm",
            "fix-bug",
            None,
            None,
            Some(vec!["new-tag".to_string()]),
            None,
            None,
        )
        .unwrap();
    assert_eq!(updated.frontmatter.tags, Some(vec!["new-tag".to_string()]));
}

#[test]
fn update_task_body_replaces_existing() {
    let mut repo = setup_with_project();
    repo.create_task(
        "fbm",
        "fix-bug",
        "Fix",
        Priority::Low,
        None,
        Some("Original.\n"),
    )
    .unwrap();
    let updated = repo
        .update_task(
            "fbm",
            "fix-bug",
            None,
            None,
            None,
            Some("Replaced.\n"),
            None,
        )
        .unwrap();
    assert_eq!(updated.body, "Replaced.\n");

    let loaded = repo.load_task("fbm", "fix-bug").unwrap();
    assert_eq!(loaded.body, "Replaced.\n");
}

#[test]
fn update_task_none_body_preserves_existing() {
    let mut repo = setup_with_project();
    repo.create_task(
        "fbm",
        "fix-bug",
        "Fix",
        Priority::Low,
        None,
        Some("Keep this.\n"),
    )
    .unwrap();
    let updated = repo
        .update_task(
            "fbm",
            "fix-bug",
            Some(TaskStatus::Done),
            None,
            None,
            None,
            None,
        )
        .unwrap();
    assert_eq!(updated.body, "Keep this.\n");
}

#[test]
fn update_task_not_found() {
    let mut repo = setup_with_project();
    let result = repo.update_task(
        "fbm",
        "nope",
        Some(TaskStatus::Done),
        None,
        None,
        None,
        None,
    );
    assert!(matches!(result, Err(Error::TaskNotFound(_))));
}

#[test]
fn update_task_done_sets_completed_and_commit() {
    let mut repo = setup_with_project();
    repo.create_task("fbm", "fix-bug", "Fix", Priority::Low, None, None)
        .unwrap();
    let updated = repo
        .update_task(
            "fbm",
            "fix-bug",
            Some(TaskStatus::Done),
            None,
            None,
            None,
            Some("abc123".to_string()),
        )
        .unwrap();
    assert_eq!(updated.frontmatter.status, TaskStatus::Done);
    assert!(updated.frontmatter.completed.is_some());
    assert_eq!(updated.frontmatter.commit, Some("abc123".to_string()));

    // Verify persisted
    let loaded = repo.load_task("fbm", "fix-bug").unwrap();
    assert_eq!(loaded.frontmatter.commit, Some("abc123".to_string()));
    assert!(loaded.frontmatter.completed.is_some());
}

#[test]
fn update_task_done_sets_completed_without_commit() {
    let mut repo = setup_with_project();
    repo.create_task("fbm", "fix-bug", "Fix", Priority::Low, None, None)
        .unwrap();
    let updated = repo
        .update_task(
            "fbm",
            "fix-bug",
            Some(TaskStatus::Done),
            None,
            None,
            None,
            None,
        )
        .unwrap();
    assert_eq!(updated.frontmatter.status, TaskStatus::Done);
    assert!(updated.frontmatter.completed.is_some());
    assert_eq!(updated.frontmatter.commit, None);
}

#[test]
fn update_task_idempotent_done_updates_commit() {
    let mut repo = setup_with_project();
    repo.create_task("fbm", "fix-bug", "Fix", Priority::Low, None, None)
        .unwrap();
    let first = repo
        .update_task(
            "fbm",
            "fix-bug",
            Some(TaskStatus::Done),
            None,
            None,
            None,
            Some("sha1".to_string()),
        )
        .unwrap();
    let first_completed = first.frontmatter.completed;

    // Re-mark as done with a new commit
    let second = repo
        .update_task(
            "fbm",
            "fix-bug",
            Some(TaskStatus::Done),
            None,
            None,
            None,
            Some("sha2".to_string()),
        )
        .unwrap();
    assert_eq!(second.frontmatter.status, TaskStatus::Done);
    assert_eq!(second.frontmatter.commit, Some("sha2".to_string()));
    // completed date preserved
    assert_eq!(second.frontmatter.completed, first_completed);
}

#[test]
fn update_task_reopen_clears_completed_and_commit() {
    let mut repo = setup_with_project();
    repo.create_task("fbm", "fix-bug", "Fix", Priority::Low, None, None)
        .unwrap();
    repo.update_task(
        "fbm",
        "fix-bug",
        Some(TaskStatus::Done),
        None,
        None,
        None,
        Some("abc123".to_string()),
    )
    .unwrap();

    // Reopen the task
    let reopened = repo
        .update_task(
            "fbm",
            "fix-bug",
            Some(TaskStatus::InProgress),
            None,
            None,
            None,
            None,
        )
        .unwrap();
    assert_eq!(reopened.frontmatter.status, TaskStatus::InProgress);
    assert_eq!(reopened.frontmatter.completed, None);
    assert_eq!(reopened.frontmatter.commit, None);
}

#[test]
fn promote_task_to_roadmap() {
    let mut repo = setup_with_project();
    let task = Document {
        frontmatter: Task {
            project: "fbm".to_string(),
            title: "Big Feature".to_string(),
            status: TaskStatus::Open,
            priority: Priority::High,
            created: NaiveDate::from_ymd_opt(2026, 3, 15).unwrap(),
            tags: Some(vec!["infra".to_string()]),
            completed: None,
            commit: None,
        },
        body: "Task body content.\n".to_string(),
    };
    repo.write_task("fbm", "big-feature", &task).unwrap();

    let roadmap_doc = repo
        .promote_task("fbm", "big-feature", "big-feature-rm")
        .unwrap();
    assert_eq!(roadmap_doc.frontmatter.title, "Big Feature");
    assert_eq!(roadmap_doc.frontmatter.roadmap, "big-feature-rm");
    assert_eq!(roadmap_doc.frontmatter.phases, vec!["phase-1-big-feature"]);

    // Task file should be removed
    assert!(
        !repo
            .store()
            .exists(&crate::paths::task_path("fbm", "big-feature"))
    );

    // Roadmap should preserve task metadata in body
    let loaded_rm = repo.load_roadmap("fbm", "big-feature-rm").unwrap();
    assert_eq!(loaded_rm.frontmatter.title, "Big Feature");
    assert!(loaded_rm.body.contains("priority: high"));
    assert!(loaded_rm.body.contains("created: 2026-03-15"));
    assert!(loaded_rm.body.contains("tags: infra"));

    let loaded_phase = repo
        .load_phase("fbm", "big-feature-rm", "phase-1-big-feature")
        .unwrap();
    assert_eq!(loaded_phase.frontmatter.title, "Big Feature");
    assert_eq!(loaded_phase.body, "Task body content.\n");
}

#[test]
fn promote_task_not_found() {
    let mut repo = setup_with_project();
    let result = repo.promote_task("fbm", "nope", "rm-slug");
    assert!(matches!(result, Err(Error::TaskNotFound(_))));
}

#[test]
fn promote_task_duplicate_roadmap() {
    let mut repo = setup_with_project();
    repo.create_task("fbm", "my-task", "Task", Priority::Low, None, None)
        .unwrap();
    repo.create_roadmap("fbm", "existing-rm", "Existing", None)
        .unwrap();
    let result = repo.promote_task("fbm", "my-task", "existing-rm");
    assert!(matches!(result, Err(Error::DuplicateSlug(_))));
}

// -- Dependency tests --

#[test]
fn add_dependency_success() {
    let mut repo = setup_with_project();
    repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();
    repo.create_roadmap("fbm", "beta", "Beta", None).unwrap();

    let doc = repo.add_dependency("fbm", "beta", "alpha").unwrap();
    assert_eq!(
        doc.frontmatter.dependencies,
        Some(vec!["alpha".to_string()])
    );

    // Verify persisted
    let loaded = repo.load_roadmap("fbm", "beta").unwrap();
    assert_eq!(
        loaded.frontmatter.dependencies,
        Some(vec!["alpha".to_string()])
    );
}

#[test]
fn add_dependency_multiple() {
    let mut repo = setup_with_project();
    repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();
    repo.create_roadmap("fbm", "beta", "Beta", None).unwrap();
    repo.create_roadmap("fbm", "gamma", "Gamma", None).unwrap();

    repo.add_dependency("fbm", "gamma", "alpha").unwrap();
    let doc = repo.add_dependency("fbm", "gamma", "beta").unwrap();
    assert_eq!(
        doc.frontmatter.dependencies,
        Some(vec!["alpha".to_string(), "beta".to_string()])
    );
}

#[test]
fn add_dependency_duplicate_is_noop() {
    let mut repo = setup_with_project();
    repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();
    repo.create_roadmap("fbm", "beta", "Beta", None).unwrap();

    repo.add_dependency("fbm", "beta", "alpha").unwrap();
    let doc = repo.add_dependency("fbm", "beta", "alpha").unwrap();
    assert_eq!(
        doc.frontmatter.dependencies,
        Some(vec!["alpha".to_string()])
    );
}

#[test]
fn add_dependency_self_cycle() {
    let mut repo = setup_with_project();
    repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();

    let result = repo.add_dependency("fbm", "alpha", "alpha");
    assert!(matches!(result, Err(Error::CyclicDependency(_))));
}

#[test]
fn add_dependency_direct_cycle() {
    let mut repo = setup_with_project();
    repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();
    repo.create_roadmap("fbm", "beta", "Beta", None).unwrap();

    repo.add_dependency("fbm", "beta", "alpha").unwrap();
    let result = repo.add_dependency("fbm", "alpha", "beta");
    assert!(matches!(result, Err(Error::CyclicDependency(_))));
}

#[test]
fn add_dependency_transitive_cycle() {
    let mut repo = setup_with_project();
    repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();
    repo.create_roadmap("fbm", "beta", "Beta", None).unwrap();
    repo.create_roadmap("fbm", "gamma", "Gamma", None).unwrap();

    repo.add_dependency("fbm", "beta", "alpha").unwrap();
    repo.add_dependency("fbm", "gamma", "beta").unwrap();
    // gamma → beta → alpha, now alpha → gamma would create a cycle
    let result = repo.add_dependency("fbm", "alpha", "gamma");
    assert!(matches!(result, Err(Error::CyclicDependency(_))));
}

#[test]
fn add_dependency_target_not_found() {
    let mut repo = setup_with_project();
    repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();

    let result = repo.add_dependency("fbm", "alpha", "nonexistent");
    assert!(matches!(result, Err(Error::RoadmapNotFound(_))));
}

#[test]
fn add_dependency_source_not_found() {
    let mut repo = setup_with_project();
    repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();

    let result = repo.add_dependency("fbm", "nonexistent", "alpha");
    assert!(matches!(result, Err(Error::RoadmapNotFound(_))));
}

#[test]
fn remove_dependency_success() {
    let mut repo = setup_with_project();
    repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();
    repo.create_roadmap("fbm", "beta", "Beta", None).unwrap();

    repo.add_dependency("fbm", "beta", "alpha").unwrap();
    let doc = repo.remove_dependency("fbm", "beta", "alpha").unwrap();
    assert_eq!(doc.frontmatter.dependencies, None);

    let loaded = repo.load_roadmap("fbm", "beta").unwrap();
    assert_eq!(loaded.frontmatter.dependencies, None);
}

#[test]
fn remove_dependency_not_present_is_noop() {
    let mut repo = setup_with_project();
    repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();

    let doc = repo
        .remove_dependency("fbm", "alpha", "nonexistent")
        .unwrap();
    assert_eq!(doc.frontmatter.dependencies, None);
}

#[test]
fn remove_dependency_preserves_others() {
    let mut repo = setup_with_project();
    repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();
    repo.create_roadmap("fbm", "beta", "Beta", None).unwrap();
    repo.create_roadmap("fbm", "gamma", "Gamma", None).unwrap();

    repo.add_dependency("fbm", "gamma", "alpha").unwrap();
    repo.add_dependency("fbm", "gamma", "beta").unwrap();
    let doc = repo.remove_dependency("fbm", "gamma", "alpha").unwrap();
    assert_eq!(doc.frontmatter.dependencies, Some(vec!["beta".to_string()]));
}

#[test]
fn dependency_graph_returns_entries() {
    let mut repo = setup_with_project();
    repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();
    repo.create_roadmap("fbm", "beta", "Beta", None).unwrap();
    repo.create_roadmap("fbm", "gamma", "Gamma", None).unwrap();

    repo.add_dependency("fbm", "beta", "alpha").unwrap();
    repo.add_dependency("fbm", "gamma", "alpha").unwrap();
    repo.add_dependency("fbm", "gamma", "beta").unwrap();

    let graph = repo.dependency_graph("fbm").unwrap();
    assert_eq!(graph.len(), 2);
    // sorted by slug
    assert_eq!(graph[0].0, "beta");
    assert_eq!(graph[0].1, vec!["alpha"]);
    assert_eq!(graph[1].0, "gamma");
    assert_eq!(graph[1].1, vec!["alpha", "beta"]);
}

#[test]
fn dependency_graph_empty() {
    let mut repo = setup_with_project();
    repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();
    let graph = repo.dependency_graph("fbm").unwrap();
    assert!(graph.is_empty());
}

// -- Delete roadmap tests --

#[test]
fn delete_roadmap_removes_files() {
    let mut repo = setup_with_project();
    repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();
    repo.create_phase("fbm", "alpha", "core", "Core", None, None)
        .unwrap();

    let roadmap_file = crate::paths::roadmap_path("fbm", "alpha");
    assert!(repo.store().exists(&roadmap_file));

    repo.delete_roadmap("fbm", "alpha").unwrap();
    assert!(!repo.store().exists(&roadmap_file));
}

#[test]
fn delete_roadmap_not_found() {
    let mut repo = setup_with_project();
    let result = repo.delete_roadmap("fbm", "nonexistent");
    assert!(matches!(result, Err(Error::RoadmapNotFound(ref s)) if s == "nonexistent"));
}

#[test]
fn delete_roadmap_cleans_up_dependencies() {
    let mut repo = setup_with_project();
    repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();
    repo.create_roadmap("fbm", "beta", "Beta", None).unwrap();
    repo.create_roadmap("fbm", "gamma", "Gamma", None).unwrap();

    repo.add_dependency("fbm", "beta", "alpha").unwrap();
    repo.add_dependency("fbm", "gamma", "alpha").unwrap();
    repo.add_dependency("fbm", "gamma", "beta").unwrap();

    repo.delete_roadmap("fbm", "alpha").unwrap();

    // beta should have no dependencies left
    let beta = repo.load_roadmap("fbm", "beta").unwrap();
    assert_eq!(beta.frontmatter.dependencies, None);

    // gamma should still depend on beta but not alpha
    let gamma = repo.load_roadmap("fbm", "gamma").unwrap();
    assert_eq!(
        gamma.frontmatter.dependencies,
        Some(vec!["beta".to_string()])
    );
}

#[test]
fn delete_roadmap_not_in_list() {
    let mut repo = setup_with_project();
    repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();
    repo.create_roadmap("fbm", "beta", "Beta", None).unwrap();

    repo.delete_roadmap("fbm", "alpha").unwrap();

    let roadmaps = repo.list_roadmaps("fbm").unwrap();
    let slugs: Vec<_> = roadmaps
        .iter()
        .map(|r| r.frontmatter.roadmap.as_str())
        .collect();
    assert_eq!(slugs, vec!["beta"]);
}

// -- Split roadmap tests --

fn setup_with_four_phases() -> PlanRepo<MemoryStore> {
    let mut repo = setup_with_project();
    repo.create_roadmap("fbm", "big-rm", "Big Roadmap", None)
        .unwrap();
    repo.create_phase("fbm", "big-rm", "design", "Design", None, None)
        .unwrap();
    repo.create_phase("fbm", "big-rm", "impl", "Implementation", None, None)
        .unwrap();
    repo.create_phase("fbm", "big-rm", "test", "Testing", None, None)
        .unwrap();
    repo.create_phase("fbm", "big-rm", "deploy", "Deployment", None, None)
        .unwrap();
    repo
}

#[test]
fn split_roadmap_basic() {
    let mut repo = setup_with_four_phases();

    // Extract phases 3 and 4 (test + deploy) into a new roadmap
    let target = repo
        .split_roadmap(
            "fbm",
            "big-rm",
            "big-rm-v2",
            "Big Roadmap V2",
            &["phase-3-test".to_string(), "phase-4-deploy".to_string()],
            None,
        )
        .unwrap();

    assert_eq!(target.frontmatter.roadmap, "big-rm-v2");
    assert_eq!(target.frontmatter.title, "Big Roadmap V2");
    assert_eq!(
        target.frontmatter.phases,
        vec!["phase-1-test", "phase-2-deploy"]
    );

    // Source should have remaining 2 phases
    let source = repo.load_roadmap("fbm", "big-rm").unwrap();
    assert_eq!(
        source.frontmatter.phases,
        vec!["phase-1-design", "phase-2-impl"]
    );
}

#[test]
fn split_roadmap_renumbers_source() {
    let mut repo = setup_with_four_phases();

    // Extract phase 1 (design), leaving phases 2,3,4 which should renumber to 1,2,3
    repo.split_roadmap(
        "fbm",
        "big-rm",
        "design-rm",
        "Design Roadmap",
        &["phase-1-design".to_string()],
        None,
    )
    .unwrap();

    let source = repo.load_roadmap("fbm", "big-rm").unwrap();
    assert_eq!(
        source.frontmatter.phases,
        vec!["phase-1-impl", "phase-2-test", "phase-3-deploy"]
    );

    // Verify phase files have correct numbers
    let p1 = repo.load_phase("fbm", "big-rm", "phase-1-impl").unwrap();
    assert_eq!(p1.frontmatter.phase, 1);
    assert_eq!(p1.frontmatter.title, "Implementation");

    let p2 = repo.load_phase("fbm", "big-rm", "phase-2-test").unwrap();
    assert_eq!(p2.frontmatter.phase, 2);

    let p3 = repo.load_phase("fbm", "big-rm", "phase-3-deploy").unwrap();
    assert_eq!(p3.frontmatter.phase, 3);
}

#[test]
fn split_roadmap_renumbers_target() {
    let mut repo = setup_with_four_phases();

    // Extract phases 2 and 4 — they should renumber to 1, 2
    let target = repo
        .split_roadmap(
            "fbm",
            "big-rm",
            "new-rm",
            "New Roadmap",
            &["phase-2-impl".to_string(), "phase-4-deploy".to_string()],
            None,
        )
        .unwrap();

    assert_eq!(
        target.frontmatter.phases,
        vec!["phase-1-impl", "phase-2-deploy"]
    );

    let p1 = repo.load_phase("fbm", "new-rm", "phase-1-impl").unwrap();
    assert_eq!(p1.frontmatter.phase, 1);
    assert_eq!(p1.frontmatter.title, "Implementation");

    let p2 = repo.load_phase("fbm", "new-rm", "phase-2-deploy").unwrap();
    assert_eq!(p2.frontmatter.phase, 2);
    assert_eq!(p2.frontmatter.title, "Deployment");
}

#[test]
fn split_roadmap_with_dependency() {
    let mut repo = setup_with_four_phases();

    let target = repo
        .split_roadmap(
            "fbm",
            "big-rm",
            "new-rm",
            "New Roadmap",
            &["phase-3-test".to_string()],
            Some("big-rm"),
        )
        .unwrap();

    assert_eq!(
        target.frontmatter.dependencies,
        Some(vec!["big-rm".to_string()])
    );
}

#[test]
fn split_roadmap_target_exists() {
    let mut repo = setup_with_four_phases();
    repo.create_roadmap("fbm", "existing", "Existing", None)
        .unwrap();

    let result = repo.split_roadmap(
        "fbm",
        "big-rm",
        "existing",
        "Existing",
        &["phase-1-design".to_string()],
        None,
    );
    assert!(matches!(result, Err(Error::DuplicateSlug(ref s)) if s == "existing"));
}

#[test]
fn split_roadmap_source_not_found() {
    let mut repo = setup_with_project();

    let result = repo.split_roadmap(
        "fbm",
        "nonexistent",
        "new-rm",
        "New",
        &["phase-1-foo".to_string()],
        None,
    );
    assert!(matches!(result, Err(Error::RoadmapNotFound(ref s)) if s == "nonexistent"));
}

#[test]
fn split_roadmap_invalid_phase() {
    let mut repo = setup_with_four_phases();

    let result = repo.split_roadmap(
        "fbm",
        "big-rm",
        "new-rm",
        "New",
        &["phase-99-nope".to_string()],
        None,
    );
    assert!(matches!(result, Err(Error::InvalidPhaseSelection(_))));
}

#[test]
fn split_roadmap_all_phases() {
    let mut repo = setup_with_four_phases();

    let result = repo.split_roadmap(
        "fbm",
        "big-rm",
        "new-rm",
        "New",
        &[
            "phase-1-design".to_string(),
            "phase-2-impl".to_string(),
            "phase-3-test".to_string(),
            "phase-4-deploy".to_string(),
        ],
        None,
    );
    assert!(matches!(result, Err(Error::InvalidPhaseSelection(_))));
}

#[test]
fn init_already_initialized() {
    let repo = PlanRepo::init(MemoryStore::new()).unwrap();
    let result = PlanRepo::init(repo.store.clone());
    assert!(matches!(result, Err(Error::AlreadyInitialized)));
}

#[test]
fn init_with_config_writes_custom_config() {
    let config = Config {
        default_project: Some("myproj".to_string()),
        stage: Some(true),
        ..Default::default()
    };
    let repo = PlanRepo::init_with_config(MemoryStore::new(), config).unwrap();
    let loaded = repo.load_config().unwrap();
    assert_eq!(loaded.default_project, Some("myproj".to_string()));
    assert_eq!(loaded.stage, Some(true));
}

#[test]
fn init_with_config_validates_format() {
    let config = Config {
        default_format: Some("xml".to_string()),
        ..Default::default()
    };
    let result = PlanRepo::init_with_config(MemoryStore::new(), config);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("xml"));
}

#[test]
fn init_delegates_to_init_with_config() {
    let repo_plain = PlanRepo::init(MemoryStore::new()).unwrap();
    let repo_config = PlanRepo::init_with_config(MemoryStore::new(), Config::default()).unwrap();

    let config_plain = repo_plain.load_config().unwrap();
    let config_via = repo_config.load_config().unwrap();
    assert_eq!(config_plain, config_via);

    // Both create INDEX.md
    assert!(repo_plain.store().exists(&crate::paths::index_path()));
    assert!(repo_config.store().exists(&crate::paths::index_path()));
}

// -- Index generation tests --

#[test]
fn generate_index_creates_file() {
    let mut repo = setup_with_project();
    repo.create_roadmap("fbm", "alpha", "Alpha Roadmap", None)
        .unwrap();
    repo.create_phase("fbm", "alpha", "core", "Core", None, None)
        .unwrap();
    repo.generate_index().unwrap();

    let content = repo.store().read(&crate::paths::index_path()).unwrap();
    assert!(content.contains("# Plan Index"));
    // Top-level index links to project INDEX.md
    assert!(content.contains("[fbm](projects/fbm/INDEX.md)"));
    assert!(content.contains("not started"));
    // Details are NOT inlined — no project heading or task tables
    assert!(!content.contains("## Project: fbm"));
}

#[test]
fn generate_index_idempotent() {
    let mut repo = setup_with_project();
    repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();
    repo.generate_index().unwrap();
    let first = repo.store().read(&crate::paths::index_path()).unwrap();
    repo.generate_index().unwrap();
    let second = repo.store().read(&crate::paths::index_path()).unwrap();
    assert_eq!(first, second);
}

#[test]
fn generate_index_empty_repo() {
    let mut repo = PlanRepo::init(MemoryStore::new()).unwrap();
    repo.generate_index().unwrap();
    let content = repo.store().read(&crate::paths::index_path()).unwrap();
    assert!(content.contains("# Plan Index"));
}

#[test]
fn generate_index_task_priority_ordering_in_project_index() {
    let mut repo = setup_with_project();
    repo.create_task("fbm", "low-task", "Low", Priority::Low, None, None)
        .unwrap();
    repo.create_task(
        "fbm",
        "crit-task",
        "Critical",
        Priority::Critical,
        None,
        None,
    )
    .unwrap();
    repo.create_task("fbm", "high-task", "High", Priority::High, None, None)
        .unwrap();
    repo.generate_index().unwrap();

    // Task ordering is in the per-project index, not the root index
    let content = repo
        .store()
        .read(&crate::paths::project_index_path("fbm"))
        .unwrap();
    let crit_pos = content.find("crit-task").unwrap();
    let high_pos = content.find("high-task").unwrap();
    let low_pos = content.find("low-task").unwrap();
    assert!(crit_pos < high_pos);
    assert!(high_pos < low_pos);

    // Root index just shows task count
    let root = repo.store().read(&crate::paths::index_path()).unwrap();
    assert!(root.contains("| 3 |")); // 3 tasks
}

// -- Per-project index tests --

#[test]
fn generate_project_index_creates_file() {
    let mut repo = setup_with_project();
    repo.create_roadmap("fbm", "alpha", "Alpha Roadmap", None)
        .unwrap();
    repo.create_phase("fbm", "alpha", "core", "Core", None, None)
        .unwrap();
    repo.generate_project_index("fbm").unwrap();

    let content = repo
        .store()
        .read(&crate::paths::project_index_path("fbm"))
        .unwrap();
    assert!(content.contains("# Project: fbm"));
    assert!(content.contains("auto-generated by rdm"));
    assert!(content.contains("roadmaps/alpha/roadmap.md"));
    assert!(!content.contains("projects/fbm/"));
}

#[test]
fn generate_index_for_project_only_writes_targeted_project() {
    let mut repo = PlanRepo::init(MemoryStore::new()).unwrap();
    repo.create_project("fbm", "FBM").unwrap();
    repo.create_project("acme", "ACME").unwrap();
    repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();
    repo.create_roadmap("acme", "beta", "Beta", None).unwrap();

    repo.generate_index_for_project("fbm").unwrap();

    // fbm's per-project INDEX.md should be written
    let fbm_index = repo
        .store()
        .read(&crate::paths::project_index_path("fbm"))
        .unwrap();
    assert!(fbm_index.contains("# Project: fbm"));
    assert!(fbm_index.contains("roadmaps/alpha/roadmap.md"));

    // acme's per-project INDEX.md should NOT be written
    assert!(
        !repo
            .store()
            .exists(&crate::paths::project_index_path("acme")),
        "acme INDEX.md should not be written by generate_index_for_project(\"fbm\")"
    );

    // Top-level INDEX.md should contain both projects
    let root = repo.store().read(&crate::paths::index_path()).unwrap();
    assert!(root.contains("[fbm]"));
    assert!(root.contains("[acme]"));
}

#[test]
fn generate_index_writes_project_index() {
    let mut repo = setup_with_project();
    repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();
    repo.generate_index().unwrap();

    // Root index should exist
    let root = repo.store().read(&crate::paths::index_path()).unwrap();
    assert!(root.contains("# Plan Index"));

    // Per-project index should also exist
    let project = repo
        .store()
        .read(&crate::paths::project_index_path("fbm"))
        .unwrap();
    assert!(project.contains("# Project: fbm"));
    assert!(project.contains("roadmaps/alpha/roadmap.md"));
}

// -- Archive roadmap tests --

#[test]
fn archive_roadmap_moves_files() {
    let mut repo = setup_with_project();
    repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();
    repo.create_phase("fbm", "alpha", "core", "Core", None, None)
        .unwrap();
    repo.update_phase(
        "fbm",
        "alpha",
        "phase-1-core",
        Some(PhaseStatus::Done),
        None,
        None,
    )
    .unwrap();

    repo.archive_roadmap("fbm", "alpha", false).unwrap();

    // Gone from active
    assert!(
        !repo
            .store()
            .exists(&crate::paths::roadmap_path("fbm", "alpha"))
    );
    // Present in archive
    assert!(
        repo.store()
            .exists(&crate::paths::archived_roadmap_path("fbm", "alpha"))
    );
}

#[test]
fn archive_roadmap_not_found() {
    let mut repo = setup_with_project();
    let result = repo.archive_roadmap("fbm", "nonexistent", false);
    assert!(matches!(result, Err(Error::RoadmapNotFound(ref s)) if s == "nonexistent"));
}

#[test]
fn archive_roadmap_rejects_incomplete_phases() {
    let mut repo = setup_with_project();
    repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();
    repo.create_phase("fbm", "alpha", "core", "Core", None, None)
        .unwrap();

    let result = repo.archive_roadmap("fbm", "alpha", false);
    assert!(matches!(
        result,
        Err(Error::RoadmapHasIncompletePhases(ref s)) if s == "alpha"
    ));
}

#[test]
fn archive_roadmap_force_overrides_check() {
    let mut repo = setup_with_project();
    repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();
    repo.create_phase("fbm", "alpha", "core", "Core", None, None)
        .unwrap();

    // force=true succeeds even with incomplete phases
    repo.archive_roadmap("fbm", "alpha", true).unwrap();
    assert!(
        repo.store()
            .exists(&crate::paths::archived_roadmap_path("fbm", "alpha"))
    );
}

#[test]
fn archive_roadmap_all_done_no_force_needed() {
    let mut repo = setup_with_project();
    repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();
    repo.create_phase("fbm", "alpha", "core", "Core", None, None)
        .unwrap();
    repo.update_phase(
        "fbm",
        "alpha",
        "phase-1-core",
        Some(PhaseStatus::Done),
        None,
        None,
    )
    .unwrap();

    // All phases done, force=false should succeed
    repo.archive_roadmap("fbm", "alpha", false).unwrap();
    assert!(
        repo.store()
            .exists(&crate::paths::archived_roadmap_path("fbm", "alpha"))
    );
}

#[test]
fn archive_roadmap_cleans_up_dependencies() {
    let mut repo = setup_with_project();
    repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();
    repo.create_roadmap("fbm", "beta", "Beta", None).unwrap();
    repo.create_roadmap("fbm", "gamma", "Gamma", None).unwrap();

    repo.add_dependency("fbm", "beta", "alpha").unwrap();
    repo.add_dependency("fbm", "gamma", "alpha").unwrap();
    repo.add_dependency("fbm", "gamma", "beta").unwrap();

    repo.archive_roadmap("fbm", "alpha", true).unwrap();

    // beta should have no dependencies left
    let beta = repo.load_roadmap("fbm", "beta").unwrap();
    assert_eq!(beta.frontmatter.dependencies, None);

    // gamma should still depend on beta but not alpha
    let gamma = repo.load_roadmap("fbm", "gamma").unwrap();
    assert_eq!(
        gamma.frontmatter.dependencies,
        Some(vec!["beta".to_string()])
    );
}

#[test]
fn archive_roadmap_not_in_active_list() {
    let mut repo = setup_with_project();
    repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();
    repo.create_roadmap("fbm", "beta", "Beta", None).unwrap();

    repo.archive_roadmap("fbm", "alpha", true).unwrap();

    let roadmaps = repo.list_roadmaps("fbm").unwrap();
    let slugs: Vec<_> = roadmaps
        .iter()
        .map(|r| r.frontmatter.roadmap.as_str())
        .collect();
    assert_eq!(slugs, vec!["beta"]);
}

#[test]
fn list_archived_roadmaps_returns_archived() {
    let mut repo = setup_with_project();
    repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();

    repo.archive_roadmap("fbm", "alpha", true).unwrap();

    let archived = repo.list_archived_roadmaps("fbm").unwrap();
    assert_eq!(archived.len(), 1);
    assert_eq!(archived[0].frontmatter.roadmap, "alpha");
}

#[test]
fn list_archived_roadmaps_empty() {
    let repo = setup_with_project();
    let archived = repo.list_archived_roadmaps("fbm").unwrap();
    assert!(archived.is_empty());
}

#[test]
fn unarchive_roadmap_restores_files() {
    let mut repo = setup_with_project();
    repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();
    repo.create_phase("fbm", "alpha", "core", "Core", None, None)
        .unwrap();

    repo.archive_roadmap("fbm", "alpha", true).unwrap();
    assert!(
        !repo
            .store()
            .exists(&crate::paths::roadmap_path("fbm", "alpha"))
    );

    repo.unarchive_roadmap("fbm", "alpha").unwrap();
    assert!(
        repo.store()
            .exists(&crate::paths::roadmap_path("fbm", "alpha"))
    );
    assert!(
        !repo
            .store()
            .exists(&crate::paths::archived_roadmap_path("fbm", "alpha"))
    );
}

#[test]
fn unarchive_roadmap_not_found() {
    let mut repo = setup_with_project();
    let result = repo.unarchive_roadmap("fbm", "nonexistent");
    assert!(matches!(result, Err(Error::RoadmapNotFound(ref s)) if s == "nonexistent"));
}

#[test]
fn unarchive_roadmap_duplicate_slug() {
    let mut repo = setup_with_project();
    repo.create_roadmap("fbm", "alpha", "Alpha", None).unwrap();
    repo.create_phase("fbm", "alpha", "core", "Core", None, None)
        .unwrap();

    repo.archive_roadmap("fbm", "alpha", true).unwrap();

    // Create a new active roadmap with the same slug
    repo.create_roadmap("fbm", "alpha", "Alpha 2", None)
        .unwrap();

    let result = repo.unarchive_roadmap("fbm", "alpha");
    assert!(matches!(result, Err(Error::DuplicateSlug(ref s)) if s == "alpha"));
}
