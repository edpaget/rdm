use chrono::NaiveDate;
use rdm_core::config::Config;
use rdm_core::document::Document;
use rdm_core::error::Error;
use rdm_core::model::*;
use rdm_core::store::{MemoryStore, Store, VersionedStore};

fn make_store() -> MemoryStore {
    MemoryStore::new()
}

// -- Write + Load round-trip tests --

#[test]
fn write_and_load_roadmap() {
    let mut store = make_store();
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
            priority: None,
            tags: None,
        },
        body: "Summary here.\n".to_string(),
    };
    rdm_core::io::write_roadmap(&mut store, "fbm", "two-way-players", &doc).unwrap();
    let loaded = rdm_core::io::load_roadmap(&store, "fbm", "two-way-players").unwrap();
    assert_eq!(loaded.frontmatter, doc.frontmatter);
    assert_eq!(loaded.body, doc.body);
}

#[test]
fn write_and_load_phase() {
    let mut store = make_store();
    let doc = Document {
        frontmatter: Phase {
            phase: 1,
            title: "Core valuation layer".to_string(),
            status: PhaseStatus::Done,
            tags: None,
            completed: Some(NaiveDate::from_ymd_opt(2026, 3, 13).unwrap()),
            commit: None,
            review_sha: None,
            review_branch: None,
            difficulty: None,
            model: None,
            blocked_reason: None,
        },
        body: "## Steps\n\n1. Do things.\n".to_string(),
    };
    rdm_core::io::write_phase(
        &mut store,
        "fbm",
        "two-way-players",
        "phase-1-core-valuation",
        &doc,
    )
    .unwrap();
    let loaded =
        rdm_core::io::load_phase(&store, "fbm", "two-way-players", "phase-1-core-valuation")
            .unwrap();
    assert_eq!(loaded.frontmatter, doc.frontmatter);
    assert_eq!(loaded.body, doc.body);
}

#[test]
fn write_and_load_task() {
    let mut store = make_store();
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
            review_sha: None,
            review_branch: None,
        },
        body: "Details.\n".to_string(),
    };
    rdm_core::io::write_task(&mut store, "fbm", "fix-barrel-nulls", &doc).unwrap();
    let loaded = rdm_core::io::load_task(&store, "fbm", "fix-barrel-nulls").unwrap();
    assert_eq!(loaded.frontmatter, doc.frontmatter);
    assert_eq!(loaded.body, doc.body);
}

#[test]
fn load_project_success() {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    rdm_core::ops::project::create_project(&mut store, "fbm", "Fantasy Baseball Manager").unwrap();
    let doc = rdm_core::io::load_project(&store, "fbm").unwrap();
    assert_eq!(doc.frontmatter.name, "fbm");
    assert_eq!(doc.frontmatter.title, "Fantasy Baseball Manager");
}

#[test]
fn load_project_not_found() {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    let result = rdm_core::io::load_project(&store, "nonexistent");
    assert!(matches!(result, Err(Error::ProjectNotFound(ref s)) if s == "nonexistent"));
}

#[test]
fn load_roadmap_not_found() {
    let store = make_store();
    let result = rdm_core::io::load_roadmap(&store, "fbm", "nonexistent");
    assert!(matches!(result, Err(Error::RoadmapNotFound(ref s)) if s == "nonexistent"));
}

#[test]
fn load_task_not_found() {
    let store = make_store();
    let result = rdm_core::io::load_task(&store, "fbm", "does-not-exist");
    assert!(matches!(result, Err(Error::TaskNotFound(ref s)) if s == "does-not-exist"));
}

// -- Init tests --

#[test]
fn init_creates_structure() {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();

    assert!(store.exists(&rdm_core::paths::config_path()));
    assert!(store.exists(&rdm_core::paths::index_path()));

    // Config should be parseable
    let toml_str = store.read(&rdm_core::paths::config_path()).unwrap();
    Config::from_toml(&toml_str).unwrap();
}

#[test]
fn load_config_after_init() {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    let config = rdm_core::io::load_config(&store).unwrap();
    assert_eq!(config.default_project, None);
}

#[test]
fn load_config_not_found() {
    let store = make_store();
    let result = rdm_core::io::load_config(&store);
    assert!(matches!(result, Err(Error::ConfigNotFound)));
}

// -- Project tests --

#[test]
fn create_project_success() {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    rdm_core::ops::project::create_project(&mut store, "fbm", "Fantasy Baseball Manager").unwrap();

    // Verify project file exists by loading it
    let doc = rdm_core::io::load_project(&store, "fbm").unwrap();
    assert_eq!(doc.frontmatter.name, "fbm");
}

#[test]
fn create_project_duplicate() {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    rdm_core::ops::project::create_project(&mut store, "fbm", "Fantasy Baseball Manager").unwrap();
    let result = rdm_core::ops::project::create_project(&mut store, "fbm", "Duplicate");
    assert!(matches!(result, Err(Error::DuplicateSlug(ref s)) if s == "fbm"));
}

#[test]
fn list_projects_empty() {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    assert_eq!(
        rdm_core::ops::project::list_projects(&store).unwrap(),
        Vec::<String>::new()
    );
}

#[test]
fn list_projects_sorted() {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    rdm_core::ops::project::create_project(&mut store, "zzz", "Last").unwrap();
    rdm_core::ops::project::create_project(&mut store, "aaa", "First").unwrap();
    rdm_core::ops::project::create_project(&mut store, "mmm", "Middle").unwrap();
    let projects = rdm_core::ops::project::list_projects(&store).unwrap();
    assert_eq!(projects, vec!["aaa", "mmm", "zzz"]);
}

// -- Roadmap tests --

#[test]
fn create_roadmap_success() {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    rdm_core::ops::project::create_project(&mut store, "fbm", "FBM").unwrap();
    let doc = rdm_core::ops::roadmap::create_roadmap(
        &mut store,
        "fbm",
        "two-way",
        "Two-Way Players",
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(doc.frontmatter.project, "fbm");
    assert_eq!(doc.frontmatter.roadmap, "two-way");
    assert_eq!(doc.frontmatter.title, "Two-Way Players");
    assert!(doc.frontmatter.phases.is_empty());

    // Should be loadable
    let loaded = rdm_core::io::load_roadmap(&store, "fbm", "two-way").unwrap();
    assert_eq!(loaded.frontmatter, doc.frontmatter);
}

#[test]
fn create_roadmap_with_body() {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    rdm_core::ops::project::create_project(&mut store, "fbm", "FBM").unwrap();
    let body = "# Description\n\nA roadmap for two-way players.\n";
    let doc = rdm_core::ops::roadmap::create_roadmap(
        &mut store,
        "fbm",
        "two-way",
        "Two-Way Players",
        Some(body),
        None,
        None,
    )
    .unwrap();
    assert_eq!(doc.body, body);

    let loaded = rdm_core::io::load_roadmap(&store, "fbm", "two-way").unwrap();
    assert_eq!(loaded.body, body);
}

#[test]
fn create_roadmap_project_not_found() {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    let result = rdm_core::ops::roadmap::create_roadmap(
        &mut store, "nope", "slug", "Title", None, None, None,
    );
    assert!(matches!(result, Err(Error::ProjectNotFound(_))));
}

#[test]
fn create_roadmap_duplicate() {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    rdm_core::ops::project::create_project(&mut store, "fbm", "FBM").unwrap();
    rdm_core::ops::roadmap::create_roadmap(
        &mut store,
        "fbm",
        "two-way",
        "Two-Way Players",
        None,
        None,
        None,
    )
    .unwrap();
    let result = rdm_core::ops::roadmap::create_roadmap(
        &mut store, "fbm", "two-way", "Dup", None, None, None,
    );
    assert!(matches!(result, Err(Error::DuplicateSlug(_))));
}

#[test]
fn update_roadmap_body_replaces_existing() {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    rdm_core::ops::project::create_project(&mut store, "fbm", "FBM").unwrap();
    rdm_core::ops::roadmap::create_roadmap(
        &mut store,
        "fbm",
        "two-way",
        "Two-Way",
        Some("Original.\n"),
        None,
        None,
    )
    .unwrap();
    let updated = rdm_core::ops::roadmap::update_roadmap(
        &mut store,
        "fbm",
        "two-way",
        rdm_core::ops::BodyUpdate::Set("Replaced.\n".to_string()),
        rdm_core::ops::PriorityUpdate::Keep,
        rdm_core::ops::TagsUpdate::Keep,
    )
    .unwrap();
    assert_eq!(updated.body, "Replaced.\n");

    let loaded = rdm_core::io::load_roadmap(&store, "fbm", "two-way").unwrap();
    assert_eq!(loaded.body, "Replaced.\n");
}

#[test]
fn update_roadmap_none_body_preserves_existing() {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    rdm_core::ops::project::create_project(&mut store, "fbm", "FBM").unwrap();
    rdm_core::ops::roadmap::create_roadmap(
        &mut store,
        "fbm",
        "two-way",
        "Two-Way",
        Some("Keep this.\n"),
        None,
        None,
    )
    .unwrap();
    let updated = rdm_core::ops::roadmap::update_roadmap(
        &mut store,
        "fbm",
        "two-way",
        rdm_core::ops::BodyUpdate::Keep,
        rdm_core::ops::PriorityUpdate::Keep,
        rdm_core::ops::TagsUpdate::Keep,
    )
    .unwrap();
    assert_eq!(updated.body, "Keep this.\n");
}

#[test]
fn update_roadmap_not_found() {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    rdm_core::ops::project::create_project(&mut store, "fbm", "FBM").unwrap();
    let result = rdm_core::ops::roadmap::update_roadmap(
        &mut store,
        "fbm",
        "nope",
        rdm_core::ops::BodyUpdate::Set("body".to_string()),
        rdm_core::ops::PriorityUpdate::Keep,
        rdm_core::ops::TagsUpdate::Keep,
    );
    assert!(matches!(result, Err(Error::RoadmapNotFound(_))));
}

#[test]
fn list_roadmaps_sorted() {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    rdm_core::ops::project::create_project(&mut store, "fbm", "FBM").unwrap();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "zzz-road", "Z", None, None, None)
        .unwrap();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "aaa-road", "A", None, None, None)
        .unwrap();
    let roadmaps = rdm_core::ops::roadmap::list_roadmaps(&store, "fbm", None, None).unwrap();
    assert_eq!(roadmaps.len(), 2);
    assert_eq!(roadmaps[0].frontmatter.roadmap, "aaa-road");
    assert_eq!(roadmaps[1].frontmatter.roadmap, "zzz-road");
}

#[test]
fn list_roadmaps_empty() {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    rdm_core::ops::project::create_project(&mut store, "fbm", "FBM").unwrap();
    let roadmaps = rdm_core::ops::roadmap::list_roadmaps(&store, "fbm", None, None).unwrap();
    assert!(roadmaps.is_empty());
}

#[test]
fn list_roadmaps_project_not_found() {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    let result = rdm_core::ops::roadmap::list_roadmaps(&store, "nope", None, None);
    assert!(matches!(result, Err(Error::ProjectNotFound(_))));
}

#[test]
fn create_roadmap_with_priority() {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    rdm_core::ops::project::create_project(&mut store, "fbm", "FBM").unwrap();
    let doc = rdm_core::ops::roadmap::create_roadmap(
        &mut store,
        "fbm",
        "urgent",
        "Urgent Fix",
        None,
        Some(rdm_core::model::Priority::High),
        None,
    )
    .unwrap();
    assert_eq!(
        doc.frontmatter.priority,
        Some(rdm_core::model::Priority::High)
    );
}

#[test]
fn update_roadmap_priority() {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    rdm_core::ops::project::create_project(&mut store, "fbm", "FBM").unwrap();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "alpha", "Alpha", None, None, None)
        .unwrap();
    let doc = rdm_core::ops::roadmap::update_roadmap(
        &mut store,
        "fbm",
        "alpha",
        rdm_core::ops::BodyUpdate::Keep,
        rdm_core::ops::PriorityUpdate::Set(rdm_core::model::Priority::Critical),
        rdm_core::ops::TagsUpdate::Keep,
    )
    .unwrap();
    assert_eq!(
        doc.frontmatter.priority,
        Some(rdm_core::model::Priority::Critical)
    );
}

#[test]
fn create_roadmap_with_tags() {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    rdm_core::ops::project::create_project(&mut store, "fbm", "FBM").unwrap();
    let doc = rdm_core::ops::roadmap::create_roadmap(
        &mut store,
        "fbm",
        "tagged",
        "Tagged",
        None,
        None,
        Some(vec!["api".to_string(), "mcp".to_string()]),
    )
    .unwrap();
    assert_eq!(
        doc.frontmatter.tags,
        Some(vec!["api".to_string(), "mcp".to_string()])
    );
    let loaded = rdm_core::io::load_roadmap(&store, "fbm", "tagged").unwrap();
    assert_eq!(
        loaded.frontmatter.tags,
        Some(vec!["api".to_string(), "mcp".to_string()])
    );
}

#[test]
fn update_roadmap_replace_tags() {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    rdm_core::ops::project::create_project(&mut store, "fbm", "FBM").unwrap();
    rdm_core::ops::roadmap::create_roadmap(
        &mut store,
        "fbm",
        "alpha",
        "Alpha",
        None,
        None,
        Some(vec!["old".to_string()]),
    )
    .unwrap();
    let doc = rdm_core::ops::roadmap::update_roadmap(
        &mut store,
        "fbm",
        "alpha",
        rdm_core::ops::BodyUpdate::Keep,
        rdm_core::ops::PriorityUpdate::Keep,
        rdm_core::ops::TagsUpdate::Set(vec!["new".to_string(), "fresh".to_string()]),
    )
    .unwrap();
    assert_eq!(
        doc.frontmatter.tags,
        Some(vec!["new".to_string(), "fresh".to_string()])
    );
}

#[test]
fn update_roadmap_clear_tags() {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    rdm_core::ops::project::create_project(&mut store, "fbm", "FBM").unwrap();
    rdm_core::ops::roadmap::create_roadmap(
        &mut store,
        "fbm",
        "alpha",
        "Alpha",
        None,
        None,
        Some(vec!["keep-me".to_string()]),
    )
    .unwrap();
    let doc = rdm_core::ops::roadmap::update_roadmap(
        &mut store,
        "fbm",
        "alpha",
        rdm_core::ops::BodyUpdate::Keep,
        rdm_core::ops::PriorityUpdate::Keep,
        rdm_core::ops::TagsUpdate::Set(vec![]),
    )
    .unwrap();
    assert_eq!(doc.frontmatter.tags, None);
}

#[test]
fn update_roadmap_clear_priority() {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    rdm_core::ops::project::create_project(&mut store, "fbm", "FBM").unwrap();
    rdm_core::ops::roadmap::create_roadmap(
        &mut store,
        "fbm",
        "alpha",
        "Alpha",
        None,
        Some(rdm_core::model::Priority::High),
        None,
    )
    .unwrap();
    let doc = rdm_core::ops::roadmap::update_roadmap(
        &mut store,
        "fbm",
        "alpha",
        rdm_core::ops::BodyUpdate::Keep,
        rdm_core::ops::PriorityUpdate::Clear,
        rdm_core::ops::TagsUpdate::Keep,
    )
    .unwrap();
    assert_eq!(doc.frontmatter.priority, None);
}

#[test]
fn list_roadmaps_sort_by_priority() {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    rdm_core::ops::project::create_project(&mut store, "fbm", "FBM").unwrap();
    rdm_core::ops::roadmap::create_roadmap(
        &mut store,
        "fbm",
        "low-pri",
        "Low",
        None,
        Some(rdm_core::model::Priority::Low),
        None,
    )
    .unwrap();
    rdm_core::ops::roadmap::create_roadmap(
        &mut store,
        "fbm",
        "high-pri",
        "High",
        None,
        Some(rdm_core::model::Priority::High),
        None,
    )
    .unwrap();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "no-pri", "None", None, None, None)
        .unwrap();
    let roadmaps = rdm_core::ops::roadmap::list_roadmaps(
        &store,
        "fbm",
        Some(rdm_core::model::RoadmapSort::Priority),
        None,
    )
    .unwrap();
    assert_eq!(roadmaps.len(), 3);
    assert_eq!(roadmaps[0].frontmatter.roadmap, "high-pri");
    assert_eq!(roadmaps[1].frontmatter.roadmap, "low-pri");
    assert_eq!(roadmaps[2].frontmatter.roadmap, "no-pri");
}

#[test]
fn list_roadmaps_filter_by_priority() {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    rdm_core::ops::project::create_project(&mut store, "fbm", "FBM").unwrap();
    rdm_core::ops::roadmap::create_roadmap(
        &mut store,
        "fbm",
        "alpha",
        "Alpha",
        None,
        Some(rdm_core::model::Priority::High),
        None,
    )
    .unwrap();
    rdm_core::ops::roadmap::create_roadmap(
        &mut store,
        "fbm",
        "beta",
        "Beta",
        None,
        Some(rdm_core::model::Priority::Low),
        None,
    )
    .unwrap();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "gamma", "Gamma", None, None, None)
        .unwrap();
    let roadmaps = rdm_core::ops::roadmap::list_roadmaps(
        &store,
        "fbm",
        None,
        Some(rdm_core::model::Priority::High),
    )
    .unwrap();
    assert_eq!(roadmaps.len(), 1);
    assert_eq!(roadmaps[0].frontmatter.roadmap, "alpha");
}

// -- Phase tests --

fn setup_with_roadmap() -> MemoryStore {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    rdm_core::ops::project::create_project(&mut store, "fbm", "FBM").unwrap();
    rdm_core::ops::roadmap::create_roadmap(
        &mut store,
        "fbm",
        "two-way",
        "Two-Way Players",
        None,
        None,
        None,
    )
    .unwrap();
    store
}

#[test]
fn create_phase_auto_number() {
    let mut store = setup_with_roadmap();
    let doc = rdm_core::ops::phase::create_phase(
        &mut store,
        "fbm",
        "two-way",
        "core",
        "Core Valuation",
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(doc.frontmatter.phase, 1);
    assert_eq!(doc.frontmatter.status, PhaseStatus::NotStarted);

    let doc2 = rdm_core::ops::phase::create_phase(
        &mut store,
        "fbm",
        "two-way",
        "service",
        "Keeper Service",
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(doc2.frontmatter.phase, 2);

    // Verify roadmap phases list was updated
    let roadmap = rdm_core::io::load_roadmap(&store, "fbm", "two-way").unwrap();
    assert_eq!(
        roadmap.frontmatter.phases,
        vec!["phase-1-core", "phase-2-service"]
    );
}

#[test]
fn create_phase_explicit_number() {
    let mut store = setup_with_roadmap();
    let doc = rdm_core::ops::phase::create_phase(
        &mut store,
        "fbm",
        "two-way",
        "core",
        "Core",
        Some(5),
        None,
        None,
    )
    .unwrap();
    assert_eq!(doc.frontmatter.phase, 5);

    // Stem should be phase-5-core
    let loaded = rdm_core::io::load_phase(&store, "fbm", "two-way", "phase-5-core").unwrap();
    assert_eq!(loaded.frontmatter, doc.frontmatter);
}

#[test]
fn create_phase_with_body() {
    let mut store = setup_with_roadmap();
    let body = "## Acceptance Criteria\n\n- [ ] Criterion one\n- [ ] Criterion two\n";
    let doc = rdm_core::ops::phase::create_phase(
        &mut store,
        "fbm",
        "two-way",
        "core",
        "Core",
        None,
        Some(body),
        None,
    )
    .unwrap();
    assert_eq!(doc.body, body);

    let loaded = rdm_core::io::load_phase(&store, "fbm", "two-way", "phase-1-core").unwrap();
    assert_eq!(loaded.body, body);
}

#[test]
fn create_phase_with_tags() {
    let mut store = setup_with_roadmap();
    let doc = rdm_core::ops::phase::create_phase(
        &mut store,
        "fbm",
        "two-way",
        "core",
        "Core",
        None,
        None,
        Some(vec!["infra".to_string(), "search".to_string()]),
    )
    .unwrap();
    assert_eq!(
        doc.frontmatter.tags,
        Some(vec!["infra".to_string(), "search".to_string()])
    );
    let loaded = rdm_core::io::load_phase(&store, "fbm", "two-way", "phase-1-core").unwrap();
    assert_eq!(
        loaded.frontmatter.tags,
        Some(vec!["infra".to_string(), "search".to_string()])
    );
}

#[test]
fn update_phase_replace_tags() {
    let mut store = setup_with_roadmap();
    rdm_core::ops::phase::create_phase(
        &mut store,
        "fbm",
        "two-way",
        "core",
        "Core",
        None,
        None,
        Some(vec!["old".to_string()]),
    )
    .unwrap();
    let doc = rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        None,
        rdm_core::ops::TagsUpdate::Set(vec!["new".to_string(), "fresh".to_string()]),
        rdm_core::ops::BodyUpdate::Keep,
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(
        doc.frontmatter.tags,
        Some(vec!["new".to_string(), "fresh".to_string()])
    );
}

#[test]
fn update_phase_clear_tags() {
    let mut store = setup_with_roadmap();
    rdm_core::ops::phase::create_phase(
        &mut store,
        "fbm",
        "two-way",
        "core",
        "Core",
        None,
        None,
        Some(vec!["drop-me".to_string()]),
    )
    .unwrap();
    let doc = rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        None,
        rdm_core::ops::TagsUpdate::Set(vec![]),
        rdm_core::ops::BodyUpdate::Keep,
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(doc.frontmatter.tags, None);
}

#[test]
fn create_phase_roadmap_not_found() {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    rdm_core::ops::project::create_project(&mut store, "fbm", "FBM").unwrap();
    let result =
        rdm_core::ops::phase::create_phase(&mut store, "fbm", "nope", "s", "T", None, None, None);
    assert!(matches!(result, Err(Error::RoadmapNotFound(_))));
}

#[test]
fn list_phases_sorted() {
    let mut store = setup_with_roadmap();
    rdm_core::ops::phase::create_phase(
        &mut store,
        "fbm",
        "two-way",
        "core",
        "Core",
        Some(2),
        None,
        None,
    )
    .unwrap();
    rdm_core::ops::phase::create_phase(
        &mut store,
        "fbm",
        "two-way",
        "service",
        "Service",
        Some(1),
        None,
        None,
    )
    .unwrap();
    let phases = rdm_core::ops::phase::list_phases(&store, "fbm", "two-way").unwrap();
    assert_eq!(phases.len(), 2);
    assert_eq!(phases[0].1.frontmatter.phase, 1);
    assert_eq!(phases[1].1.frontmatter.phase, 2);
}

#[test]
fn update_phase_to_done_sets_completed() {
    let mut store = setup_with_roadmap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "two-way", "core", "Core", None, None, None,
    )
    .unwrap();
    let updated = rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        Some(PhaseStatus::Done),
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
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
    let mut store = setup_with_roadmap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "two-way", "core", "Core", None, None, None,
    )
    .unwrap();
    let updated = rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        Some(PhaseStatus::Done),
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        Some("abc123".to_string()),
        None,
        None,
    )
    .unwrap();
    assert_eq!(updated.frontmatter.status, PhaseStatus::Done);
    assert!(updated.frontmatter.completed.is_some());
    assert_eq!(updated.frontmatter.commit, Some("abc123".to_string()));

    // Verify persistence
    let loaded = rdm_core::io::load_phase(&store, "fbm", "two-way", "phase-1-core").unwrap();
    assert_eq!(loaded.frontmatter.commit, Some("abc123".to_string()));
}

#[test]
fn update_phase_to_needs_review_stamps_review_sha() {
    let mut store = setup_with_roadmap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "two-way", "core", "Core", None, None, None,
    )
    .unwrap();
    let updated = rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        Some(PhaseStatus::NeedsReview),
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        Some("deadbeef".to_string()),
        None,
    )
    .unwrap();
    assert_eq!(updated.frontmatter.status, PhaseStatus::NeedsReview);
    assert_eq!(updated.frontmatter.review_sha, Some("deadbeef".to_string()));

    // Verify persistence
    let loaded = rdm_core::io::load_phase(&store, "fbm", "two-way", "phase-1-core").unwrap();
    assert_eq!(loaded.frontmatter.review_sha, Some("deadbeef".to_string()));
}

#[test]
fn update_phase_leaving_needs_review_clears_review_sha() {
    let mut store = setup_with_roadmap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "two-way", "core", "Core", None, None, None,
    )
    .unwrap();
    rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        Some(PhaseStatus::NeedsReview),
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        Some("deadbeef".to_string()),
        None,
    )
    .unwrap();
    // Transition to a different status clears the stamp.
    let updated = rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        Some(PhaseStatus::Done),
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(updated.frontmatter.review_sha, None);
}

#[test]
fn update_phase_already_needs_review_restamps_on_reapply() {
    let mut store = setup_with_roadmap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "two-way", "core", "Core", None, None, None,
    )
    .unwrap();
    // Initial finalize stamps sha1 / branch-a.
    rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        Some(PhaseStatus::NeedsReview),
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        Some("sha1".to_string()),
        Some("branch-a".to_string()),
    )
    .unwrap();
    // Re-applying NeedsReview while already in that status OVERWRITES the
    // stamp — this is the refresh path rdm review restamp depends on.
    let updated = rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        Some(PhaseStatus::NeedsReview),
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        Some("sha2".to_string()),
        Some("branch-b".to_string()),
    )
    .unwrap();
    assert_eq!(updated.frontmatter.review_sha, Some("sha2".to_string()));
    assert_eq!(
        updated.frontmatter.review_branch,
        Some("branch-b".to_string())
    );
    let loaded = rdm_core::io::load_phase(&store, "fbm", "two-way", "phase-1-core").unwrap();
    assert_eq!(loaded.frontmatter.review_sha, Some("sha2".to_string()));
    assert_eq!(
        loaded.frontmatter.review_branch,
        Some("branch-b".to_string())
    );
}

#[test]
fn update_phase_status_none_preserves_review_sha() {
    let mut store = setup_with_roadmap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "two-way", "core", "Core", None, None, None,
    )
    .unwrap();
    rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        Some(PhaseStatus::NeedsReview),
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        Some("deadbeef".to_string()),
        Some("feature/x".to_string()),
    )
    .unwrap();
    // A metadata-only update (status None) leaves both stamps intact.
    let updated = rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        None,
        rdm_core::ops::TagsUpdate::Set(vec!["x".to_string()]),
        rdm_core::ops::BodyUpdate::Keep,
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(updated.frontmatter.review_sha, Some("deadbeef".to_string()));
    assert_eq!(
        updated.frontmatter.review_branch,
        Some("feature/x".to_string())
    );
}

#[test]
fn update_phase_to_needs_review_stamps_review_branch() {
    let mut store = setup_with_roadmap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "two-way", "core", "Core", None, None, None,
    )
    .unwrap();
    let updated = rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        Some(PhaseStatus::NeedsReview),
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        Some("deadbeef".to_string()),
        Some("feature/x".to_string()),
    )
    .unwrap();
    assert_eq!(
        updated.frontmatter.review_branch,
        Some("feature/x".to_string())
    );

    // Verify persistence
    let loaded = rdm_core::io::load_phase(&store, "fbm", "two-way", "phase-1-core").unwrap();
    assert_eq!(
        loaded.frontmatter.review_branch,
        Some("feature/x".to_string())
    );
}

#[test]
fn update_phase_leaving_needs_review_clears_review_branch() {
    let mut store = setup_with_roadmap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "two-way", "core", "Core", None, None, None,
    )
    .unwrap();
    rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        Some(PhaseStatus::NeedsReview),
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        Some("deadbeef".to_string()),
        Some("feature/x".to_string()),
    )
    .unwrap();
    // Transition to a different status clears the branch stamp in lockstep.
    let updated = rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        Some(PhaseStatus::Done),
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(updated.frontmatter.review_branch, None);
}

#[test]
fn update_phase_from_done_clears_completed() {
    let mut store = setup_with_roadmap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "two-way", "core", "Core", None, None, None,
    )
    .unwrap();
    rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        Some(PhaseStatus::Done),
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        Some("abc123".to_string()),
        None,
        None,
    )
    .unwrap();
    let updated = rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        Some(PhaseStatus::InProgress),
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(updated.frontmatter.status, PhaseStatus::InProgress);
    assert_eq!(updated.frontmatter.completed, None);
    assert_eq!(updated.frontmatter.commit, None);
}

#[test]
fn blocked_reason_is_recorded_then_preserved_across_resume_then_clearable() {
    let mut store = setup_with_roadmap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "two-way", "core", "Core", None, None, None,
    )
    .unwrap();

    // Park the phase as blocked and record the escalation reason.
    rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        Some(PhaseStatus::Blocked),
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        None,
        None,
    )
    .unwrap();
    let parked = rdm_core::ops::phase::set_phase_blocked_reason(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        rdm_core::ops::ReasonUpdate::Set("AC 2 is ambiguous: which crate owns this?".to_string()),
    )
    .unwrap();
    assert_eq!(parked.frontmatter.status, PhaseStatus::Blocked);
    assert_eq!(
        parked.frontmatter.blocked_reason.as_deref(),
        Some("AC 2 is ambiguous: which crate owns this?")
    );

    // Resuming the phase must NOT lose the recorded reason — a plain status
    // change leaves blocked_reason untouched.
    let resumed = rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        Some(PhaseStatus::InProgress),
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(resumed.frontmatter.status, PhaseStatus::InProgress);
    assert_eq!(
        resumed.frontmatter.blocked_reason.as_deref(),
        Some("AC 2 is ambiguous: which crate owns this?"),
        "resuming a phase must preserve the recorded blocked reason"
    );

    // The reason can be explicitly cleared.
    let cleared = rdm_core::ops::phase::set_phase_blocked_reason(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        rdm_core::ops::ReasonUpdate::Clear,
    )
    .unwrap();
    assert_eq!(cleared.frontmatter.blocked_reason, None);
}

#[test]
fn blocked_phases_lists_only_parked_phases_with_reasons() {
    let mut store = setup_with_roadmap();
    rdm_core::ops::phase::create_phase(
        &mut store,
        "fbm",
        "two-way",
        "core",
        "Core",
        Some(1),
        None,
        None,
    )
    .unwrap();
    rdm_core::ops::phase::create_phase(
        &mut store,
        "fbm",
        "two-way",
        "service",
        "Service",
        Some(2),
        None,
        None,
    )
    .unwrap();

    // Block phase 1 with a reason; leave phase 2 not-started.
    rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        Some(PhaseStatus::Blocked),
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        None,
        None,
    )
    .unwrap();
    rdm_core::ops::phase::set_phase_blocked_reason(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        rdm_core::ops::ReasonUpdate::Set("needs an external credential".to_string()),
    )
    .unwrap();

    let blocked = rdm_core::ops::review::blocked_phases(&store, "fbm").unwrap();
    assert_eq!(blocked.len(), 1, "only the blocked phase should be listed");
    assert_eq!(blocked[0].identifier, "two-way/phase-1-core");
    assert_eq!(
        blocked[0].reason.as_deref(),
        Some("needs an external credential")
    );
}

#[test]
fn update_phase_body_replaces_existing() {
    let mut store = setup_with_roadmap();
    rdm_core::ops::phase::create_phase(
        &mut store,
        "fbm",
        "two-way",
        "core",
        "Core",
        None,
        Some("Original body.\n"),
        None,
    )
    .unwrap();
    let updated = rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        Some(PhaseStatus::InProgress),
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Set("Replaced body.\n".to_string()),
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(updated.body, "Replaced body.\n");

    let loaded = rdm_core::io::load_phase(&store, "fbm", "two-way", "phase-1-core").unwrap();
    assert_eq!(loaded.body, "Replaced body.\n");
}

#[test]
fn update_phase_none_body_preserves_existing() {
    let mut store = setup_with_roadmap();
    rdm_core::ops::phase::create_phase(
        &mut store,
        "fbm",
        "two-way",
        "core",
        "Core",
        None,
        Some("Keep this body.\n"),
        None,
    )
    .unwrap();
    let updated = rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        Some(PhaseStatus::InProgress),
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(updated.body, "Keep this body.\n");
}

#[test]
fn update_phase_not_found() {
    let mut store = setup_with_roadmap();
    let result = rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "two-way",
        "phase-99-nope",
        Some(PhaseStatus::Done),
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        None,
        None,
    );
    assert!(matches!(result, Err(Error::PhaseNotFound(_))));
}

#[test]
fn set_phase_estimate_sets_and_persists_both() {
    let mut store = setup_with_roadmap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "two-way", "core", "Core", None, None, None,
    )
    .unwrap();

    let doc = rdm_core::ops::phase::set_phase_estimate(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        rdm_core::ops::DifficultyUpdate::Set(Difficulty::Hard),
        rdm_core::ops::ModelTierUpdate::Set(ModelTier::Large),
    )
    .unwrap();
    assert_eq!(doc.frontmatter.difficulty, Some(Difficulty::Hard));
    assert_eq!(doc.frontmatter.model, Some(ModelTier::Large));

    // Persisted to disk, not just returned.
    let loaded = rdm_core::io::load_phase(&store, "fbm", "two-way", "phase-1-core").unwrap();
    assert_eq!(loaded.frontmatter.difficulty, Some(Difficulty::Hard));
    assert_eq!(loaded.frontmatter.model, Some(ModelTier::Large));
}

#[test]
fn set_phase_estimate_clears_one_keeps_other() {
    let mut store = setup_with_roadmap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "two-way", "core", "Core", None, None, None,
    )
    .unwrap();
    rdm_core::ops::phase::set_phase_estimate(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        rdm_core::ops::DifficultyUpdate::Set(Difficulty::Easy),
        rdm_core::ops::ModelTierUpdate::Set(ModelTier::Small),
    )
    .unwrap();

    // Clear difficulty, keep model — fields are independent.
    let doc = rdm_core::ops::phase::set_phase_estimate(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        rdm_core::ops::DifficultyUpdate::Clear,
        rdm_core::ops::ModelTierUpdate::Keep,
    )
    .unwrap();
    assert_eq!(doc.frontmatter.difficulty, None);
    assert_eq!(doc.frontmatter.model, Some(ModelTier::Small));

    // Inverse: keep difficulty (now None), clear model.
    let doc = rdm_core::ops::phase::set_phase_estimate(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        rdm_core::ops::DifficultyUpdate::Keep,
        rdm_core::ops::ModelTierUpdate::Clear,
    )
    .unwrap();
    assert_eq!(doc.frontmatter.difficulty, None);
    assert_eq!(doc.frontmatter.model, None);
}

#[test]
fn set_phase_estimate_not_found() {
    let mut store = setup_with_roadmap();
    let result = rdm_core::ops::phase::set_phase_estimate(
        &mut store,
        "fbm",
        "two-way",
        "phase-99-nope",
        rdm_core::ops::DifficultyUpdate::Set(Difficulty::Hard),
        rdm_core::ops::ModelTierUpdate::Keep,
    );
    assert!(matches!(result, Err(Error::PhaseNotFound(_))));
}

#[test]
fn set_phase_estimate_difficulty_only_derives_model_tier() {
    let mut store = setup_with_roadmap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "two-way", "core", "Core", None, None, None,
    )
    .unwrap();

    // Difficulty set, model left untouched, none recorded yet → tier derived.
    let doc = rdm_core::ops::phase::set_phase_estimate(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        rdm_core::ops::DifficultyUpdate::Set(Difficulty::Moderate),
        rdm_core::ops::ModelTierUpdate::Keep,
    )
    .unwrap();
    assert_eq!(doc.frontmatter.difficulty, Some(Difficulty::Moderate));
    assert_eq!(doc.frontmatter.model, Some(ModelTier::Medium));

    // Persisted, not just returned.
    let loaded = rdm_core::io::load_phase(&store, "fbm", "two-way", "phase-1-core").unwrap();
    assert_eq!(loaded.frontmatter.model, Some(ModelTier::Medium));
}

#[test]
fn set_phase_estimate_explicit_model_overrides_derive() {
    let mut store = setup_with_roadmap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "two-way", "core", "Core", None, None, None,
    )
    .unwrap();

    // Hard would derive Large, but an explicit Small wins.
    let doc = rdm_core::ops::phase::set_phase_estimate(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        rdm_core::ops::DifficultyUpdate::Set(Difficulty::Hard),
        rdm_core::ops::ModelTierUpdate::Set(ModelTier::Small),
    )
    .unwrap();
    assert_eq!(doc.frontmatter.difficulty, Some(Difficulty::Hard));
    assert_eq!(doc.frontmatter.model, Some(ModelTier::Small));
}

#[test]
fn set_phase_estimate_preserves_existing_model_on_difficulty_change() {
    let mut store = setup_with_roadmap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "two-way", "core", "Core", None, None, None,
    )
    .unwrap();

    // Seed a human-set model tier.
    rdm_core::ops::phase::set_phase_estimate(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        rdm_core::ops::DifficultyUpdate::Keep,
        rdm_core::ops::ModelTierUpdate::Set(ModelTier::Small),
    )
    .unwrap();

    // Setting difficulty alone must not clobber the existing model.
    let doc = rdm_core::ops::phase::set_phase_estimate(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        rdm_core::ops::DifficultyUpdate::Set(Difficulty::Hard),
        rdm_core::ops::ModelTierUpdate::Keep,
    )
    .unwrap();
    assert_eq!(doc.frontmatter.difficulty, Some(Difficulty::Hard));
    assert_eq!(doc.frontmatter.model, Some(ModelTier::Small));
}

#[test]
fn set_phase_estimate_clear_model_prevents_derive() {
    let mut store = setup_with_roadmap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "two-way", "core", "Core", None, None, None,
    )
    .unwrap();

    // Explicitly clearing the model wins over the difficulty derive: setting a
    // difficulty in the same call must NOT re-populate the just-cleared model.
    let doc = rdm_core::ops::phase::set_phase_estimate(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        rdm_core::ops::DifficultyUpdate::Set(Difficulty::Hard),
        rdm_core::ops::ModelTierUpdate::Clear,
    )
    .unwrap();
    assert_eq!(doc.frontmatter.difficulty, Some(Difficulty::Hard));
    assert_eq!(doc.frontmatter.model, None);
}

#[test]
fn update_phase_done_to_done_with_new_commit_updates_sha() {
    let mut store = setup_with_roadmap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "two-way", "core", "Core", None, None, None,
    )
    .unwrap();
    let first = rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        Some(PhaseStatus::Done),
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        Some("abc123".to_string()),
        None,
        None,
    )
    .unwrap();
    let first_completed = first.frontmatter.completed;

    let updated = rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        Some(PhaseStatus::Done),
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        Some("def456".to_string()),
        None,
        None,
    )
    .unwrap();
    assert_eq!(updated.frontmatter.status, PhaseStatus::Done);
    assert_eq!(updated.frontmatter.commit, Some("def456".to_string()));
    assert_eq!(updated.frontmatter.completed, first_completed);
}

#[test]
fn update_phase_done_to_done_without_commit_is_noop() {
    let mut store = setup_with_roadmap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "two-way", "core", "Core", None, None, None,
    )
    .unwrap();
    let first = rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        Some(PhaseStatus::Done),
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        Some("abc123".to_string()),
        None,
        None,
    )
    .unwrap();
    let first_completed = first.frontmatter.completed;

    let updated = rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        Some(PhaseStatus::Done),
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(updated.frontmatter.status, PhaseStatus::Done);
    assert_eq!(updated.frontmatter.commit, Some("abc123".to_string()));
    assert_eq!(updated.frontmatter.completed, first_completed);
}

#[test]
fn update_phase_to_wont_fix_sets_completed() {
    let mut store = setup_with_roadmap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "two-way", "core", "Core", None, None, None,
    )
    .unwrap();
    let updated = rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        Some(PhaseStatus::WontFix),
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(updated.frontmatter.status, PhaseStatus::WontFix);
    assert!(updated.frontmatter.completed.is_some());
    assert_eq!(updated.frontmatter.commit, None);

    // Verify persistence
    let loaded = rdm_core::io::load_phase(&store, "fbm", "two-way", "phase-1-core").unwrap();
    assert_eq!(loaded.frontmatter.status, PhaseStatus::WontFix);
    assert!(loaded.frontmatter.completed.is_some());
}

#[test]
fn update_phase_wont_fix_to_not_started_clears_completed() {
    let mut store = setup_with_roadmap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "two-way", "core", "Core", None, None, None,
    )
    .unwrap();
    rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        Some(PhaseStatus::WontFix),
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        Some("abc123".to_string()),
        None,
        None,
    )
    .unwrap();
    let updated = rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        Some(PhaseStatus::NotStarted),
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(updated.frontmatter.status, PhaseStatus::NotStarted);
    assert_eq!(updated.frontmatter.completed, None);
    assert_eq!(updated.frontmatter.commit, None);
}

#[test]
fn resolve_by_number() {
    let mut store = setup_with_roadmap();
    rdm_core::ops::phase::create_phase(
        &mut store,
        "fbm",
        "two-way",
        "core",
        "Core",
        Some(1),
        None,
        None,
    )
    .unwrap();
    rdm_core::ops::phase::create_phase(
        &mut store,
        "fbm",
        "two-way",
        "service",
        "Service",
        Some(2),
        None,
        None,
    )
    .unwrap();
    let stem = rdm_core::ops::phase::resolve_phase_stem(&store, "fbm", "two-way", "2").unwrap();
    assert_eq!(stem, "phase-2-service");
}

#[test]
fn resolve_by_stem_passthrough() {
    let store = setup_with_roadmap();
    let stem =
        rdm_core::ops::phase::resolve_phase_stem(&store, "fbm", "two-way", "phase-1-core").unwrap();
    assert_eq!(stem, "phase-1-core");
}

#[test]
fn resolve_number_not_found() {
    let mut store = setup_with_roadmap();
    rdm_core::ops::phase::create_phase(
        &mut store,
        "fbm",
        "two-way",
        "core",
        "Core",
        Some(1),
        None,
        None,
    )
    .unwrap();
    let result = rdm_core::ops::phase::resolve_phase_stem(&store, "fbm", "two-way", "99");
    assert!(matches!(result, Err(Error::PhaseNotFound(ref s)) if s == "99"));
}

// -- Remove phase tests --

#[test]
fn remove_phase_deletes_file() {
    let mut store = setup_with_roadmap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "two-way", "core", "Core", None, None, None,
    )
    .unwrap();
    let path = rdm_core::paths::phase_path("fbm", "two-way", "phase-1-core");
    assert!(store.exists(&path));

    rdm_core::ops::phase::remove_phase(&mut store, "fbm", "two-way", "phase-1-core").unwrap();
    assert!(!store.exists(&path));
}

#[test]
fn remove_phase_updates_roadmap() {
    let mut store = setup_with_roadmap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "two-way", "core", "Core", None, None, None,
    )
    .unwrap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "two-way", "service", "Service", None, None, None,
    )
    .unwrap();

    rdm_core::ops::phase::remove_phase(&mut store, "fbm", "two-way", "phase-1-core").unwrap();

    let roadmap = rdm_core::io::load_roadmap(&store, "fbm", "two-way").unwrap();
    assert_eq!(roadmap.frontmatter.phases, vec!["phase-2-service"]);
}

#[test]
fn remove_phase_not_found() {
    let mut store = setup_with_roadmap();
    let result = rdm_core::ops::phase::remove_phase(&mut store, "fbm", "two-way", "phase-99-nope");
    assert!(matches!(result, Err(Error::PhaseNotFound(ref s)) if s == "phase-99-nope"));
}

// -- Review tests --

#[test]
fn pending_review_items_lists_phases_and_tasks_in_needs_review() {
    let mut store = setup_with_roadmap();
    // Two phases: one needs-review (stamped), one in-progress (excluded).
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "two-way", "core", "Core", None, None, None,
    )
    .unwrap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "two-way", "svc", "Service", None, None, None,
    )
    .unwrap();
    rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        Some(PhaseStatus::NeedsReview),
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        Some("sha-phase".to_string()),
        Some("branch-phase".to_string()),
    )
    .unwrap();

    // Two tasks: one needs-review (unstamped), one open (excluded).
    rdm_core::ops::task::create_task(&mut store, "fbm", "t1", "T1", Priority::Low, None, None)
        .unwrap();
    rdm_core::ops::task::create_task(&mut store, "fbm", "t2", "T2", Priority::Low, None, None)
        .unwrap();
    rdm_core::ops::task::update_task(
        &mut store,
        "fbm",
        "t1",
        Some(TaskStatus::NeedsReview),
        None,
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        None,
        None,
    )
    .unwrap();

    let items = rdm_core::ops::review::pending_review_items(&store, "fbm").unwrap();
    assert_eq!(items.len(), 2);

    let phase = items
        .iter()
        .find(|i| i.kind == rdm_core::ops::review::PendingReviewKind::Phase)
        .unwrap();
    assert_eq!(phase.identifier, "two-way/phase-1-core");
    assert_eq!(phase.review_sha, Some("sha-phase".to_string()));
    assert_eq!(phase.review_branch, Some("branch-phase".to_string()));

    let task = items
        .iter()
        .find(|i| i.kind == rdm_core::ops::review::PendingReviewKind::Task)
        .unwrap();
    assert_eq!(task.identifier, "t1");
    assert_eq!(task.review_sha, None);
    assert_eq!(task.review_branch, None);
}

// -- Task tests --

fn setup_with_project() -> MemoryStore {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    rdm_core::ops::project::create_project(&mut store, "fbm", "FBM").unwrap();
    store
}

#[test]
fn create_task_success() {
    let mut store = setup_with_project();
    let doc = rdm_core::ops::task::create_task(
        &mut store,
        "fbm",
        "fix-bug",
        "Fix the bug",
        Priority::High,
        None,
        None,
    )
    .unwrap();
    assert_eq!(doc.frontmatter.title, "Fix the bug");
    assert_eq!(doc.frontmatter.status, TaskStatus::Open);
    assert_eq!(doc.frontmatter.priority, Priority::High);
    assert!(doc.frontmatter.tags.is_none());

    // Should be loadable
    let loaded = rdm_core::io::load_task(&store, "fbm", "fix-bug").unwrap();
    assert_eq!(loaded.frontmatter, doc.frontmatter);
}

#[test]
fn create_task_with_tags() {
    let mut store = setup_with_project();
    let doc = rdm_core::ops::task::create_task(
        &mut store,
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
    let mut store = setup_with_project();
    let body = "## Notes\n\nSome detailed task notes.\n";
    let doc = rdm_core::ops::task::create_task(
        &mut store,
        "fbm",
        "fix-bug",
        "Fix",
        Priority::High,
        None,
        Some(body),
    )
    .unwrap();
    assert_eq!(doc.body, body);

    let loaded = rdm_core::io::load_task(&store, "fbm", "fix-bug").unwrap();
    assert_eq!(loaded.body, body);
}

#[test]
fn create_task_project_not_found() {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    let result = rdm_core::ops::task::create_task(
        &mut store,
        "nope",
        "slug",
        "Title",
        Priority::Low,
        None,
        None,
    );
    assert!(matches!(result, Err(Error::ProjectNotFound(_))));
}

#[test]
fn create_task_duplicate() {
    let mut store = setup_with_project();
    rdm_core::ops::task::create_task(
        &mut store,
        "fbm",
        "fix-bug",
        "Fix",
        Priority::Low,
        None,
        None,
    )
    .unwrap();
    let result = rdm_core::ops::task::create_task(
        &mut store,
        "fbm",
        "fix-bug",
        "Dup",
        Priority::Low,
        None,
        None,
    );
    assert!(matches!(result, Err(Error::DuplicateSlug(_))));
}

#[test]
fn list_tasks_sorted() {
    let mut store = setup_with_project();
    rdm_core::ops::task::create_task(
        &mut store,
        "fbm",
        "zzz-task",
        "Z",
        Priority::Low,
        None,
        None,
    )
    .unwrap();
    rdm_core::ops::task::create_task(
        &mut store,
        "fbm",
        "aaa-task",
        "A",
        Priority::High,
        None,
        None,
    )
    .unwrap();
    let tasks = rdm_core::ops::task::list_tasks(&store, "fbm").unwrap();
    assert_eq!(tasks.len(), 2);
    assert_eq!(tasks[0].0, "aaa-task");
    assert_eq!(tasks[1].0, "zzz-task");
}

#[test]
fn list_tasks_empty() {
    let store = setup_with_project();
    let tasks = rdm_core::ops::task::list_tasks(&store, "fbm").unwrap();
    assert!(tasks.is_empty());
}

#[test]
fn list_tasks_project_not_found() {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    let result = rdm_core::ops::task::list_tasks(&store, "nonexistent");
    assert!(matches!(result, Err(Error::ProjectNotFound(_))));
}

#[test]
fn update_task_status() {
    let mut store = setup_with_project();
    rdm_core::ops::task::create_task(
        &mut store,
        "fbm",
        "fix-bug",
        "Fix",
        Priority::Low,
        None,
        None,
    )
    .unwrap();
    let updated = rdm_core::ops::task::update_task(
        &mut store,
        "fbm",
        "fix-bug",
        Some(TaskStatus::Done),
        None,
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(updated.frontmatter.status, TaskStatus::Done);

    let loaded = rdm_core::io::load_task(&store, "fbm", "fix-bug").unwrap();
    assert_eq!(loaded.frontmatter.status, TaskStatus::Done);
}

#[test]
fn update_task_priority() {
    let mut store = setup_with_project();
    rdm_core::ops::task::create_task(
        &mut store,
        "fbm",
        "fix-bug",
        "Fix",
        Priority::Low,
        None,
        None,
    )
    .unwrap();
    let updated = rdm_core::ops::task::update_task(
        &mut store,
        "fbm",
        "fix-bug",
        None,
        Some(Priority::Critical),
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(updated.frontmatter.priority, Priority::Critical);
}

#[test]
fn update_task_tags() {
    let mut store = setup_with_project();
    rdm_core::ops::task::create_task(
        &mut store,
        "fbm",
        "fix-bug",
        "Fix",
        Priority::Low,
        None,
        None,
    )
    .unwrap();
    let updated = rdm_core::ops::task::update_task(
        &mut store,
        "fbm",
        "fix-bug",
        None,
        None,
        rdm_core::ops::TagsUpdate::Set(vec!["new-tag".to_string()]),
        rdm_core::ops::BodyUpdate::Keep,
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(updated.frontmatter.tags, Some(vec!["new-tag".to_string()]));
}

#[test]
fn update_task_body_replaces_existing() {
    let mut store = setup_with_project();
    rdm_core::ops::task::create_task(
        &mut store,
        "fbm",
        "fix-bug",
        "Fix",
        Priority::Low,
        None,
        Some("Original.\n"),
    )
    .unwrap();
    let updated = rdm_core::ops::task::update_task(
        &mut store,
        "fbm",
        "fix-bug",
        None,
        None,
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Set("Replaced.\n".to_string()),
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(updated.body, "Replaced.\n");

    let loaded = rdm_core::io::load_task(&store, "fbm", "fix-bug").unwrap();
    assert_eq!(loaded.body, "Replaced.\n");
}

#[test]
fn update_task_none_body_preserves_existing() {
    let mut store = setup_with_project();
    rdm_core::ops::task::create_task(
        &mut store,
        "fbm",
        "fix-bug",
        "Fix",
        Priority::Low,
        None,
        Some("Keep this.\n"),
    )
    .unwrap();
    let updated = rdm_core::ops::task::update_task(
        &mut store,
        "fbm",
        "fix-bug",
        Some(TaskStatus::Done),
        None,
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(updated.body, "Keep this.\n");
}

#[test]
fn update_task_not_found() {
    let mut store = setup_with_project();
    let result = rdm_core::ops::task::update_task(
        &mut store,
        "fbm",
        "nope",
        Some(TaskStatus::Done),
        None,
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        None,
        None,
    );
    assert!(matches!(result, Err(Error::TaskNotFound(_))));
}

#[test]
fn update_task_done_sets_completed_and_commit() {
    let mut store = setup_with_project();
    rdm_core::ops::task::create_task(
        &mut store,
        "fbm",
        "fix-bug",
        "Fix",
        Priority::Low,
        None,
        None,
    )
    .unwrap();
    let updated = rdm_core::ops::task::update_task(
        &mut store,
        "fbm",
        "fix-bug",
        Some(TaskStatus::Done),
        None,
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        Some("abc123".to_string()),
        None,
        None,
    )
    .unwrap();
    assert_eq!(updated.frontmatter.status, TaskStatus::Done);
    assert!(updated.frontmatter.completed.is_some());
    assert_eq!(updated.frontmatter.commit, Some("abc123".to_string()));

    // Verify persisted
    let loaded = rdm_core::io::load_task(&store, "fbm", "fix-bug").unwrap();
    assert_eq!(loaded.frontmatter.commit, Some("abc123".to_string()));
    assert!(loaded.frontmatter.completed.is_some());
}

#[test]
fn update_task_to_needs_review_stamps_review_sha() {
    let mut store = setup_with_project();
    rdm_core::ops::task::create_task(
        &mut store,
        "fbm",
        "fix-bug",
        "Fix",
        Priority::Low,
        None,
        None,
    )
    .unwrap();
    let updated = rdm_core::ops::task::update_task(
        &mut store,
        "fbm",
        "fix-bug",
        Some(TaskStatus::NeedsReview),
        None,
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        Some("deadbeef".to_string()),
        None,
    )
    .unwrap();
    assert_eq!(updated.frontmatter.status, TaskStatus::NeedsReview);
    assert_eq!(updated.frontmatter.review_sha, Some("deadbeef".to_string()));

    let loaded = rdm_core::io::load_task(&store, "fbm", "fix-bug").unwrap();
    assert_eq!(loaded.frontmatter.review_sha, Some("deadbeef".to_string()));
}

#[test]
fn update_task_leaving_needs_review_clears_review_sha() {
    let mut store = setup_with_project();
    rdm_core::ops::task::create_task(
        &mut store,
        "fbm",
        "fix-bug",
        "Fix",
        Priority::Low,
        None,
        None,
    )
    .unwrap();
    rdm_core::ops::task::update_task(
        &mut store,
        "fbm",
        "fix-bug",
        Some(TaskStatus::NeedsReview),
        None,
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        Some("deadbeef".to_string()),
        None,
    )
    .unwrap();
    let updated = rdm_core::ops::task::update_task(
        &mut store,
        "fbm",
        "fix-bug",
        Some(TaskStatus::Done),
        None,
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(updated.frontmatter.review_sha, None);
}

#[test]
fn update_task_already_needs_review_restamps_on_reapply() {
    let mut store = setup_with_project();
    rdm_core::ops::task::create_task(
        &mut store,
        "fbm",
        "fix-bug",
        "Fix",
        Priority::Low,
        None,
        None,
    )
    .unwrap();
    rdm_core::ops::task::update_task(
        &mut store,
        "fbm",
        "fix-bug",
        Some(TaskStatus::NeedsReview),
        None,
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        Some("sha1".to_string()),
        Some("branch-a".to_string()),
    )
    .unwrap();
    // Re-applying NeedsReview overwrites the stamp (the restamp refresh path).
    let updated = rdm_core::ops::task::update_task(
        &mut store,
        "fbm",
        "fix-bug",
        Some(TaskStatus::NeedsReview),
        None,
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        Some("sha2".to_string()),
        Some("branch-b".to_string()),
    )
    .unwrap();
    assert_eq!(updated.frontmatter.review_sha, Some("sha2".to_string()));
    assert_eq!(
        updated.frontmatter.review_branch,
        Some("branch-b".to_string())
    );
    let loaded = rdm_core::io::load_task(&store, "fbm", "fix-bug").unwrap();
    assert_eq!(loaded.frontmatter.review_sha, Some("sha2".to_string()));
    assert_eq!(
        loaded.frontmatter.review_branch,
        Some("branch-b".to_string())
    );
}

#[test]
fn update_task_status_none_preserves_review_sha() {
    let mut store = setup_with_project();
    rdm_core::ops::task::create_task(
        &mut store,
        "fbm",
        "fix-bug",
        "Fix",
        Priority::Low,
        None,
        None,
    )
    .unwrap();
    rdm_core::ops::task::update_task(
        &mut store,
        "fbm",
        "fix-bug",
        Some(TaskStatus::NeedsReview),
        None,
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        Some("deadbeef".to_string()),
        Some("feature/x".to_string()),
    )
    .unwrap();
    let updated = rdm_core::ops::task::update_task(
        &mut store,
        "fbm",
        "fix-bug",
        None,
        Some(Priority::High),
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(updated.frontmatter.review_sha, Some("deadbeef".to_string()));
    assert_eq!(
        updated.frontmatter.review_branch,
        Some("feature/x".to_string())
    );
}

#[test]
fn update_task_to_needs_review_stamps_review_branch() {
    let mut store = setup_with_project();
    rdm_core::ops::task::create_task(
        &mut store,
        "fbm",
        "fix-bug",
        "Fix",
        Priority::Low,
        None,
        None,
    )
    .unwrap();
    let updated = rdm_core::ops::task::update_task(
        &mut store,
        "fbm",
        "fix-bug",
        Some(TaskStatus::NeedsReview),
        None,
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        Some("deadbeef".to_string()),
        Some("feature/x".to_string()),
    )
    .unwrap();
    assert_eq!(
        updated.frontmatter.review_branch,
        Some("feature/x".to_string())
    );

    // Verify persistence
    let loaded = rdm_core::io::load_task(&store, "fbm", "fix-bug").unwrap();
    assert_eq!(
        loaded.frontmatter.review_branch,
        Some("feature/x".to_string())
    );
}

#[test]
fn update_task_leaving_needs_review_clears_review_branch() {
    let mut store = setup_with_project();
    rdm_core::ops::task::create_task(
        &mut store,
        "fbm",
        "fix-bug",
        "Fix",
        Priority::Low,
        None,
        None,
    )
    .unwrap();
    rdm_core::ops::task::update_task(
        &mut store,
        "fbm",
        "fix-bug",
        Some(TaskStatus::NeedsReview),
        None,
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        Some("deadbeef".to_string()),
        Some("feature/x".to_string()),
    )
    .unwrap();
    // Transition to a different status clears the branch stamp in lockstep.
    let updated = rdm_core::ops::task::update_task(
        &mut store,
        "fbm",
        "fix-bug",
        Some(TaskStatus::Done),
        None,
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(updated.frontmatter.review_branch, None);
}

#[test]
fn update_task_done_sets_completed_without_commit() {
    let mut store = setup_with_project();
    rdm_core::ops::task::create_task(
        &mut store,
        "fbm",
        "fix-bug",
        "Fix",
        Priority::Low,
        None,
        None,
    )
    .unwrap();
    let updated = rdm_core::ops::task::update_task(
        &mut store,
        "fbm",
        "fix-bug",
        Some(TaskStatus::Done),
        None,
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
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
    let mut store = setup_with_project();
    rdm_core::ops::task::create_task(
        &mut store,
        "fbm",
        "fix-bug",
        "Fix",
        Priority::Low,
        None,
        None,
    )
    .unwrap();
    let first = rdm_core::ops::task::update_task(
        &mut store,
        "fbm",
        "fix-bug",
        Some(TaskStatus::Done),
        None,
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        Some("sha1".to_string()),
        None,
        None,
    )
    .unwrap();
    let first_completed = first.frontmatter.completed;

    // Re-mark as done with a new commit
    let second = rdm_core::ops::task::update_task(
        &mut store,
        "fbm",
        "fix-bug",
        Some(TaskStatus::Done),
        None,
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        Some("sha2".to_string()),
        None,
        None,
    )
    .unwrap();
    assert_eq!(second.frontmatter.status, TaskStatus::Done);
    assert_eq!(second.frontmatter.commit, Some("sha2".to_string()));
    // completed date preserved
    assert_eq!(second.frontmatter.completed, first_completed);
}

#[test]
fn update_task_reopen_clears_completed_and_commit() {
    let mut store = setup_with_project();
    rdm_core::ops::task::create_task(
        &mut store,
        "fbm",
        "fix-bug",
        "Fix",
        Priority::Low,
        None,
        None,
    )
    .unwrap();
    rdm_core::ops::task::update_task(
        &mut store,
        "fbm",
        "fix-bug",
        Some(TaskStatus::Done),
        None,
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        Some("abc123".to_string()),
        None,
        None,
    )
    .unwrap();

    // Reopen the task
    let reopened = rdm_core::ops::task::update_task(
        &mut store,
        "fbm",
        "fix-bug",
        Some(TaskStatus::InProgress),
        None,
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
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
fn update_task_wont_fix_sets_completed() {
    let mut store = setup_with_project();
    rdm_core::ops::task::create_task(
        &mut store,
        "fbm",
        "fix-bug",
        "Fix",
        Priority::Low,
        None,
        None,
    )
    .unwrap();
    let updated = rdm_core::ops::task::update_task(
        &mut store,
        "fbm",
        "fix-bug",
        Some(TaskStatus::WontFix),
        None,
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        Some("sha-wf".to_string()),
        None,
        None,
    )
    .unwrap();
    assert_eq!(updated.frontmatter.status, TaskStatus::WontFix);
    // WontFix is terminal, so it must stamp a completed date and the commit.
    assert!(updated.frontmatter.completed.is_some());
    assert_eq!(updated.frontmatter.commit, Some("sha-wf".to_string()));
}

#[test]
fn promote_task_to_roadmap() {
    let mut store = setup_with_project();
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
            review_sha: None,
            review_branch: None,
        },
        body: "Task body content.\n".to_string(),
    };
    rdm_core::io::write_task(&mut store, "fbm", "big-feature", &task).unwrap();

    let roadmap_doc =
        rdm_core::ops::task::promote_task(&mut store, "fbm", "big-feature", "big-feature-rm")
            .unwrap();
    assert_eq!(roadmap_doc.frontmatter.title, "Big Feature");
    assert_eq!(roadmap_doc.frontmatter.roadmap, "big-feature-rm");
    assert_eq!(roadmap_doc.frontmatter.phases, vec!["phase-1-big-feature"]);

    // Task file should be removed
    assert!(!store.exists(&rdm_core::paths::task_path("fbm", "big-feature")));

    // Roadmap should preserve task metadata in body
    let loaded_rm = rdm_core::io::load_roadmap(&store, "fbm", "big-feature-rm").unwrap();
    assert_eq!(loaded_rm.frontmatter.title, "Big Feature");
    assert!(loaded_rm.body.contains("priority: high"));
    assert!(loaded_rm.body.contains("created: 2026-03-15"));
    assert!(loaded_rm.body.contains("tags: infra"));

    let loaded_phase =
        rdm_core::io::load_phase(&store, "fbm", "big-feature-rm", "phase-1-big-feature").unwrap();
    assert_eq!(loaded_phase.frontmatter.title, "Big Feature");
    assert_eq!(loaded_phase.body, "Task body content.\n");
    // Promotion should also carry task tags onto the seed phase frontmatter.
    assert_eq!(
        loaded_phase.frontmatter.tags,
        Some(vec!["infra".to_string()])
    );
}

#[test]
fn promote_task_not_found() {
    let mut store = setup_with_project();
    let result = rdm_core::ops::task::promote_task(&mut store, "fbm", "nope", "rm-slug");
    assert!(matches!(result, Err(Error::TaskNotFound(_))));
}

#[test]
fn promote_task_duplicate_roadmap() {
    let mut store = setup_with_project();
    rdm_core::ops::task::create_task(
        &mut store,
        "fbm",
        "my-task",
        "Task",
        Priority::Low,
        None,
        None,
    )
    .unwrap();
    rdm_core::ops::roadmap::create_roadmap(
        &mut store,
        "fbm",
        "existing-rm",
        "Existing",
        None,
        None,
        None,
    )
    .unwrap();
    let result = rdm_core::ops::task::promote_task(&mut store, "fbm", "my-task", "existing-rm");
    assert!(matches!(result, Err(Error::DuplicateSlug(_))));
}

// -- Dependency tests --

#[test]
fn add_dependency_success() {
    let mut store = setup_with_project();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "alpha", "Alpha", None, None, None)
        .unwrap();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "beta", "Beta", None, None, None)
        .unwrap();

    let doc = rdm_core::ops::roadmap::add_dependency(&mut store, "fbm", "beta", "alpha").unwrap();
    assert_eq!(
        doc.frontmatter.dependencies,
        Some(vec!["alpha".to_string()])
    );

    // Verify persisted
    let loaded = rdm_core::io::load_roadmap(&store, "fbm", "beta").unwrap();
    assert_eq!(
        loaded.frontmatter.dependencies,
        Some(vec!["alpha".to_string()])
    );
}

#[test]
fn add_dependency_multiple() {
    let mut store = setup_with_project();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "alpha", "Alpha", None, None, None)
        .unwrap();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "beta", "Beta", None, None, None)
        .unwrap();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "gamma", "Gamma", None, None, None)
        .unwrap();

    rdm_core::ops::roadmap::add_dependency(&mut store, "fbm", "gamma", "alpha").unwrap();
    let doc = rdm_core::ops::roadmap::add_dependency(&mut store, "fbm", "gamma", "beta").unwrap();
    assert_eq!(
        doc.frontmatter.dependencies,
        Some(vec!["alpha".to_string(), "beta".to_string()])
    );
}

#[test]
fn add_dependency_duplicate_is_noop() {
    let mut store = setup_with_project();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "alpha", "Alpha", None, None, None)
        .unwrap();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "beta", "Beta", None, None, None)
        .unwrap();

    rdm_core::ops::roadmap::add_dependency(&mut store, "fbm", "beta", "alpha").unwrap();
    let doc = rdm_core::ops::roadmap::add_dependency(&mut store, "fbm", "beta", "alpha").unwrap();
    assert_eq!(
        doc.frontmatter.dependencies,
        Some(vec!["alpha".to_string()])
    );
}

#[test]
fn add_dependency_self_cycle() {
    let mut store = setup_with_project();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "alpha", "Alpha", None, None, None)
        .unwrap();

    let result = rdm_core::ops::roadmap::add_dependency(&mut store, "fbm", "alpha", "alpha");
    assert!(matches!(result, Err(Error::CyclicDependency(_))));
}

#[test]
fn add_dependency_direct_cycle() {
    let mut store = setup_with_project();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "alpha", "Alpha", None, None, None)
        .unwrap();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "beta", "Beta", None, None, None)
        .unwrap();

    rdm_core::ops::roadmap::add_dependency(&mut store, "fbm", "beta", "alpha").unwrap();
    let result = rdm_core::ops::roadmap::add_dependency(&mut store, "fbm", "alpha", "beta");
    assert!(matches!(result, Err(Error::CyclicDependency(_))));
}

#[test]
fn add_dependency_transitive_cycle() {
    let mut store = setup_with_project();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "alpha", "Alpha", None, None, None)
        .unwrap();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "beta", "Beta", None, None, None)
        .unwrap();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "gamma", "Gamma", None, None, None)
        .unwrap();

    rdm_core::ops::roadmap::add_dependency(&mut store, "fbm", "beta", "alpha").unwrap();
    rdm_core::ops::roadmap::add_dependency(&mut store, "fbm", "gamma", "beta").unwrap();
    // gamma -> beta -> alpha, now alpha -> gamma would create a cycle
    let result = rdm_core::ops::roadmap::add_dependency(&mut store, "fbm", "alpha", "gamma");
    assert!(matches!(result, Err(Error::CyclicDependency(_))));
}

#[test]
fn add_dependency_target_not_found() {
    let mut store = setup_with_project();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "alpha", "Alpha", None, None, None)
        .unwrap();

    let result = rdm_core::ops::roadmap::add_dependency(&mut store, "fbm", "alpha", "nonexistent");
    assert!(matches!(result, Err(Error::RoadmapNotFound(_))));
}

#[test]
fn add_dependency_source_not_found() {
    let mut store = setup_with_project();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "alpha", "Alpha", None, None, None)
        .unwrap();

    let result = rdm_core::ops::roadmap::add_dependency(&mut store, "fbm", "nonexistent", "alpha");
    assert!(matches!(result, Err(Error::RoadmapNotFound(_))));
}

#[test]
fn remove_dependency_success() {
    let mut store = setup_with_project();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "alpha", "Alpha", None, None, None)
        .unwrap();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "beta", "Beta", None, None, None)
        .unwrap();

    rdm_core::ops::roadmap::add_dependency(&mut store, "fbm", "beta", "alpha").unwrap();
    let doc =
        rdm_core::ops::roadmap::remove_dependency(&mut store, "fbm", "beta", "alpha").unwrap();
    assert_eq!(doc.frontmatter.dependencies, None);

    let loaded = rdm_core::io::load_roadmap(&store, "fbm", "beta").unwrap();
    assert_eq!(loaded.frontmatter.dependencies, None);
}

#[test]
fn remove_dependency_not_present_is_noop() {
    let mut store = setup_with_project();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "alpha", "Alpha", None, None, None)
        .unwrap();

    let doc = rdm_core::ops::roadmap::remove_dependency(&mut store, "fbm", "alpha", "nonexistent")
        .unwrap();
    assert_eq!(doc.frontmatter.dependencies, None);
}

#[test]
fn remove_dependency_preserves_others() {
    let mut store = setup_with_project();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "alpha", "Alpha", None, None, None)
        .unwrap();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "beta", "Beta", None, None, None)
        .unwrap();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "gamma", "Gamma", None, None, None)
        .unwrap();

    rdm_core::ops::roadmap::add_dependency(&mut store, "fbm", "gamma", "alpha").unwrap();
    rdm_core::ops::roadmap::add_dependency(&mut store, "fbm", "gamma", "beta").unwrap();
    let doc =
        rdm_core::ops::roadmap::remove_dependency(&mut store, "fbm", "gamma", "alpha").unwrap();
    assert_eq!(doc.frontmatter.dependencies, Some(vec!["beta".to_string()]));
}

#[test]
fn dependency_graph_returns_entries() {
    let mut store = setup_with_project();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "alpha", "Alpha", None, None, None)
        .unwrap();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "beta", "Beta", None, None, None)
        .unwrap();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "gamma", "Gamma", None, None, None)
        .unwrap();

    rdm_core::ops::roadmap::add_dependency(&mut store, "fbm", "beta", "alpha").unwrap();
    rdm_core::ops::roadmap::add_dependency(&mut store, "fbm", "gamma", "alpha").unwrap();
    rdm_core::ops::roadmap::add_dependency(&mut store, "fbm", "gamma", "beta").unwrap();

    let graph = rdm_core::ops::roadmap::dependency_graph(&store, "fbm").unwrap();
    assert_eq!(graph.len(), 2);
    // sorted by slug
    assert_eq!(graph[0].0, "beta");
    assert_eq!(graph[0].1, vec!["alpha"]);
    assert_eq!(graph[1].0, "gamma");
    assert_eq!(graph[1].1, vec!["alpha", "beta"]);
}

#[test]
fn dependency_graph_empty() {
    let mut store = setup_with_project();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "alpha", "Alpha", None, None, None)
        .unwrap();
    let graph = rdm_core::ops::roadmap::dependency_graph(&store, "fbm").unwrap();
    assert!(graph.is_empty());
}

// -- Delete roadmap tests --

#[test]
fn delete_roadmap_removes_files() {
    let mut store = setup_with_project();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "alpha", "Alpha", None, None, None)
        .unwrap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "alpha", "core", "Core", None, None, None,
    )
    .unwrap();

    let roadmap_file = rdm_core::paths::roadmap_path("fbm", "alpha");
    assert!(store.exists(&roadmap_file));

    rdm_core::ops::roadmap::delete_roadmap(&mut store, "fbm", "alpha").unwrap();
    assert!(!store.exists(&roadmap_file));
}

#[test]
fn delete_roadmap_not_found() {
    let mut store = setup_with_project();
    let result = rdm_core::ops::roadmap::delete_roadmap(&mut store, "fbm", "nonexistent");
    assert!(matches!(result, Err(Error::RoadmapNotFound(ref s)) if s == "nonexistent"));
}

#[test]
fn delete_roadmap_cleans_up_dependencies() {
    let mut store = setup_with_project();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "alpha", "Alpha", None, None, None)
        .unwrap();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "beta", "Beta", None, None, None)
        .unwrap();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "gamma", "Gamma", None, None, None)
        .unwrap();

    rdm_core::ops::roadmap::add_dependency(&mut store, "fbm", "beta", "alpha").unwrap();
    rdm_core::ops::roadmap::add_dependency(&mut store, "fbm", "gamma", "alpha").unwrap();
    rdm_core::ops::roadmap::add_dependency(&mut store, "fbm", "gamma", "beta").unwrap();

    rdm_core::ops::roadmap::delete_roadmap(&mut store, "fbm", "alpha").unwrap();

    // beta should have no dependencies left
    let beta = rdm_core::io::load_roadmap(&store, "fbm", "beta").unwrap();
    assert_eq!(beta.frontmatter.dependencies, None);

    // gamma should still depend on beta but not alpha
    let gamma = rdm_core::io::load_roadmap(&store, "fbm", "gamma").unwrap();
    assert_eq!(
        gamma.frontmatter.dependencies,
        Some(vec!["beta".to_string()])
    );
}

#[test]
fn delete_roadmap_not_in_list() {
    let mut store = setup_with_project();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "alpha", "Alpha", None, None, None)
        .unwrap();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "beta", "Beta", None, None, None)
        .unwrap();

    rdm_core::ops::roadmap::delete_roadmap(&mut store, "fbm", "alpha").unwrap();

    let roadmaps = rdm_core::ops::roadmap::list_roadmaps(&store, "fbm", None, None).unwrap();
    let slugs: Vec<_> = roadmaps
        .iter()
        .map(|r| r.frontmatter.roadmap.as_str())
        .collect();
    assert_eq!(slugs, vec!["beta"]);
}

// -- Split roadmap tests --

fn setup_with_four_phases() -> MemoryStore {
    let mut store = setup_with_project();
    rdm_core::ops::roadmap::create_roadmap(
        &mut store,
        "fbm",
        "big-rm",
        "Big Roadmap",
        None,
        None,
        None,
    )
    .unwrap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "big-rm", "design", "Design", None, None, None,
    )
    .unwrap();
    rdm_core::ops::phase::create_phase(
        &mut store,
        "fbm",
        "big-rm",
        "impl",
        "Implementation",
        None,
        None,
        None,
    )
    .unwrap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "big-rm", "test", "Testing", None, None, None,
    )
    .unwrap();
    rdm_core::ops::phase::create_phase(
        &mut store,
        "fbm",
        "big-rm",
        "deploy",
        "Deployment",
        None,
        None,
        None,
    )
    .unwrap();
    store
}

#[test]
fn split_roadmap_basic() {
    let mut store = setup_with_four_phases();

    // Extract phases 3 and 4 (test + deploy) into a new roadmap
    let target = rdm_core::ops::roadmap::split_roadmap(
        &mut store,
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
    let source = rdm_core::io::load_roadmap(&store, "fbm", "big-rm").unwrap();
    assert_eq!(
        source.frontmatter.phases,
        vec!["phase-1-design", "phase-2-impl"]
    );
}

#[test]
fn split_roadmap_renumbers_source() {
    let mut store = setup_with_four_phases();

    // Extract phase 1 (design), leaving phases 2,3,4 which should renumber to 1,2,3
    rdm_core::ops::roadmap::split_roadmap(
        &mut store,
        "fbm",
        "big-rm",
        "design-rm",
        "Design Roadmap",
        &["phase-1-design".to_string()],
        None,
    )
    .unwrap();

    let source = rdm_core::io::load_roadmap(&store, "fbm", "big-rm").unwrap();
    assert_eq!(
        source.frontmatter.phases,
        vec!["phase-1-impl", "phase-2-test", "phase-3-deploy"]
    );

    // Verify phase files have correct numbers
    let p1 = rdm_core::io::load_phase(&store, "fbm", "big-rm", "phase-1-impl").unwrap();
    assert_eq!(p1.frontmatter.phase, 1);
    assert_eq!(p1.frontmatter.title, "Implementation");

    let p2 = rdm_core::io::load_phase(&store, "fbm", "big-rm", "phase-2-test").unwrap();
    assert_eq!(p2.frontmatter.phase, 2);

    let p3 = rdm_core::io::load_phase(&store, "fbm", "big-rm", "phase-3-deploy").unwrap();
    assert_eq!(p3.frontmatter.phase, 3);
}

#[test]
fn split_roadmap_renumbers_target() {
    let mut store = setup_with_four_phases();

    // Extract phases 2 and 4 -- they should renumber to 1, 2
    let target = rdm_core::ops::roadmap::split_roadmap(
        &mut store,
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

    let p1 = rdm_core::io::load_phase(&store, "fbm", "new-rm", "phase-1-impl").unwrap();
    assert_eq!(p1.frontmatter.phase, 1);
    assert_eq!(p1.frontmatter.title, "Implementation");

    let p2 = rdm_core::io::load_phase(&store, "fbm", "new-rm", "phase-2-deploy").unwrap();
    assert_eq!(p2.frontmatter.phase, 2);
    assert_eq!(p2.frontmatter.title, "Deployment");
}

#[test]
fn split_roadmap_with_dependency() {
    let mut store = setup_with_four_phases();

    let target = rdm_core::ops::roadmap::split_roadmap(
        &mut store,
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
    let mut store = setup_with_four_phases();
    rdm_core::ops::roadmap::create_roadmap(
        &mut store, "fbm", "existing", "Existing", None, None, None,
    )
    .unwrap();

    let result = rdm_core::ops::roadmap::split_roadmap(
        &mut store,
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
    let mut store = setup_with_project();

    let result = rdm_core::ops::roadmap::split_roadmap(
        &mut store,
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
    let mut store = setup_with_four_phases();

    let result = rdm_core::ops::roadmap::split_roadmap(
        &mut store,
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
    let mut store = setup_with_four_phases();

    let result = rdm_core::ops::roadmap::split_roadmap(
        &mut store,
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
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    let result = rdm_core::ops::init::init(&mut store);
    assert!(matches!(result, Err(Error::AlreadyInitialized)));
}

#[test]
fn init_with_config_writes_custom_config() {
    let config = Config {
        default_project: Some("myproj".to_string()),
        stage: Some(true),
        ..Default::default()
    };
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init_with_config(&mut store, config).unwrap();
    let loaded = rdm_core::io::load_config(&store).unwrap();
    assert_eq!(loaded.default_project, Some("myproj".to_string()));
    assert_eq!(loaded.stage, Some(true));
}

#[test]
fn init_with_config_validates_format() {
    let config = Config {
        default_format: Some("xml".to_string()),
        ..Default::default()
    };
    let mut store = MemoryStore::new();
    let result = rdm_core::ops::init::init_with_config(&mut store, config);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("xml"));
}

#[test]
fn init_delegates_to_init_with_config() {
    let mut store_plain = MemoryStore::new();
    rdm_core::ops::init::init(&mut store_plain).unwrap();
    let mut store_config = MemoryStore::new();
    rdm_core::ops::init::init_with_config(&mut store_config, Config::default()).unwrap();

    let config_plain = rdm_core::io::load_config(&store_plain).unwrap();
    let config_via = rdm_core::io::load_config(&store_config).unwrap();
    assert_eq!(config_plain, config_via);

    // Both create INDEX.md
    assert!(store_plain.exists(&rdm_core::paths::index_path()));
    assert!(store_config.exists(&rdm_core::paths::index_path()));
}

// -- Index generation tests --

#[test]
fn generate_index_creates_file() {
    let mut store = setup_with_project();
    rdm_core::ops::roadmap::create_roadmap(
        &mut store,
        "fbm",
        "alpha",
        "Alpha Roadmap",
        None,
        None,
        None,
    )
    .unwrap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "alpha", "core", "Core", None, None, None,
    )
    .unwrap();
    rdm_core::ops::index::generate_index(&mut store).unwrap();

    let content = store.read(&rdm_core::paths::index_path()).unwrap();
    assert!(content.contains("# Plan Index"));
    // Top-level index links to project INDEX.md
    assert!(content.contains("[fbm](projects/fbm/INDEX.md)"));
    assert!(content.contains("not started"));
    // Details are NOT inlined -- no project heading or task tables
    assert!(!content.contains("## Project: fbm"));
}

#[test]
fn generate_index_idempotent() {
    let mut store = setup_with_project();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "alpha", "Alpha", None, None, None)
        .unwrap();
    rdm_core::ops::index::generate_index(&mut store).unwrap();
    let first = store.read(&rdm_core::paths::index_path()).unwrap();
    rdm_core::ops::index::generate_index(&mut store).unwrap();
    let second = store.read(&rdm_core::paths::index_path()).unwrap();
    assert_eq!(first, second);
}

#[test]
fn generate_index_empty_repo() {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    rdm_core::ops::index::generate_index(&mut store).unwrap();
    let content = store.read(&rdm_core::paths::index_path()).unwrap();
    assert!(content.contains("# Plan Index"));
}

#[test]
fn generate_index_task_priority_ordering_in_project_index() {
    let mut store = setup_with_project();
    rdm_core::ops::task::create_task(
        &mut store,
        "fbm",
        "low-task",
        "Low",
        Priority::Low,
        None,
        None,
    )
    .unwrap();
    rdm_core::ops::task::create_task(
        &mut store,
        "fbm",
        "crit-task",
        "Critical",
        Priority::Critical,
        None,
        None,
    )
    .unwrap();
    rdm_core::ops::task::create_task(
        &mut store,
        "fbm",
        "high-task",
        "High",
        Priority::High,
        None,
        None,
    )
    .unwrap();
    rdm_core::ops::index::generate_index(&mut store).unwrap();

    // Task ordering is in the per-project index, not the root index
    let content = store
        .read(&rdm_core::paths::project_index_path("fbm"))
        .unwrap();
    let crit_pos = content.find("crit-task").unwrap();
    let high_pos = content.find("high-task").unwrap();
    let low_pos = content.find("low-task").unwrap();
    assert!(crit_pos < high_pos);
    assert!(high_pos < low_pos);

    // Root index just shows task count
    let root = store.read(&rdm_core::paths::index_path()).unwrap();
    assert!(root.contains("| 3 |")); // 3 tasks
}

#[test]
fn generate_index_counts_wont_fix_in_done_count() {
    let mut store = setup_with_project();
    rdm_core::ops::roadmap::create_roadmap(
        &mut store,
        "fbm",
        "alpha",
        "Alpha Roadmap",
        None,
        None,
        None,
    )
    .unwrap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "alpha", "core", "Core", None, None, None,
    )
    .unwrap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "alpha", "extra", "Extra", None, None, None,
    )
    .unwrap();
    rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "alpha",
        "phase-1-core",
        Some(PhaseStatus::Done),
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        None,
        None,
    )
    .unwrap();
    rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "alpha",
        "phase-2-extra",
        Some(PhaseStatus::WontFix),
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        None,
        None,
    )
    .unwrap();
    rdm_core::ops::index::generate_index(&mut store).unwrap();

    let content = store
        .read(&rdm_core::paths::project_index_path("fbm"))
        .unwrap();
    assert!(content.contains("complete"));
}

#[test]
fn needs_review_and_reviewed_round_trip_and_render_in_index() {
    let mut store = setup_with_project();
    rdm_core::ops::roadmap::create_roadmap(
        &mut store,
        "fbm",
        "alpha",
        "Alpha Roadmap",
        None,
        None,
        None,
    )
    .unwrap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "alpha", "core", "Core", None, None, None,
    )
    .unwrap();
    rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "alpha",
        "phase-1-core",
        Some(PhaseStatus::NeedsReview),
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        None,
        None,
    )
    .unwrap();
    rdm_core::ops::task::create_task(
        &mut store,
        "fbm",
        "ship-it",
        "Ship It",
        Priority::Medium,
        None,
        None,
    )
    .unwrap();
    rdm_core::ops::task::update_task(
        &mut store,
        "fbm",
        "ship-it",
        Some(TaskStatus::Reviewed),
        None,
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        None,
        None,
    )
    .unwrap();

    // Statuses round-trip losslessly through write→read, and neither
    // non-terminal status stamps a `completed` date.
    let phase = rdm_core::io::load_phase(&store, "fbm", "alpha", "phase-1-core").unwrap();
    assert_eq!(phase.frontmatter.status, PhaseStatus::NeedsReview);
    assert_eq!(phase.frontmatter.completed, None);
    let task = rdm_core::io::load_task(&store, "fbm", "ship-it").unwrap();
    assert_eq!(task.frontmatter.status, TaskStatus::Reviewed);
    assert_eq!(task.frontmatter.completed, None);

    // The task status renders in the per-project INDEX.md (the roadmap row
    // shows progress counts rather than per-phase statuses).
    rdm_core::ops::index::generate_index(&mut store).unwrap();
    let content = store
        .read(&rdm_core::paths::project_index_path("fbm"))
        .unwrap();
    assert!(content.contains("reviewed"));

    // The phase status renders in the roadmap's phase list (Display path).
    let phases = rdm_core::ops::phase::list_phases(&store, "fbm", "alpha").unwrap();
    let rendered = rdm_core::display::format_phase_list(&phases);
    assert!(rendered.contains("needs-review"));
}

// -- Per-project index tests --

#[test]
fn generate_project_index_creates_file() {
    let mut store = setup_with_project();
    rdm_core::ops::roadmap::create_roadmap(
        &mut store,
        "fbm",
        "alpha",
        "Alpha Roadmap",
        None,
        None,
        None,
    )
    .unwrap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "alpha", "core", "Core", None, None, None,
    )
    .unwrap();
    rdm_core::ops::index::generate_project_index(&mut store, "fbm").unwrap();

    let content = store
        .read(&rdm_core::paths::project_index_path("fbm"))
        .unwrap();
    assert!(content.contains("# Project: fbm"));
    assert!(content.contains("auto-generated by rdm"));
    assert!(content.contains("roadmaps/alpha/roadmap.md"));
    assert!(!content.contains("projects/fbm/"));
}

#[test]
fn generate_index_for_project_only_writes_targeted_project() {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    rdm_core::ops::project::create_project(&mut store, "fbm", "FBM").unwrap();
    rdm_core::ops::project::create_project(&mut store, "acme", "ACME").unwrap();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "alpha", "Alpha", None, None, None)
        .unwrap();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "acme", "beta", "Beta", None, None, None)
        .unwrap();

    rdm_core::ops::index::generate_index_for_project(&mut store, "fbm").unwrap();

    // fbm's per-project INDEX.md should be written
    let fbm_index = store
        .read(&rdm_core::paths::project_index_path("fbm"))
        .unwrap();
    assert!(fbm_index.contains("# Project: fbm"));
    assert!(fbm_index.contains("roadmaps/alpha/roadmap.md"));

    // acme's per-project INDEX.md should NOT be written
    assert!(
        !store.exists(&rdm_core::paths::project_index_path("acme")),
        "acme INDEX.md should not be written by generate_index_for_project(\"fbm\")"
    );

    // Top-level INDEX.md should contain both projects
    let root = store.read(&rdm_core::paths::index_path()).unwrap();
    assert!(root.contains("[fbm]"));
    assert!(root.contains("[acme]"));
}

#[test]
fn mutate_regenerates_index_and_commits_once() {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    rdm_core::ops::mutate(&mut store, "fbm", |s| {
        rdm_core::ops::project::create_project(s, "fbm", "FBM")
    })
    .unwrap();

    // Snapshot the commit identifier before the mutation under test. The
    // MemoryStore exposes a monotonic `mem-N` token that advances once per
    // commit, so it doubles as a commit counter.
    let before = store.head_sha().unwrap();

    let doc = rdm_core::ops::mutate(&mut store, "fbm", |s| {
        rdm_core::ops::roadmap::create_roadmap(s, "fbm", "alpha", "Alpha", None, None, None)
    })
    .unwrap();
    assert_eq!(doc.frontmatter.roadmap, "alpha");

    // The entity write + index regeneration collapse into exactly ONE commit.
    let after = store.head_sha().unwrap();
    let counter = |sha: &str| sha.strip_prefix("mem-").unwrap().parse::<u32>().unwrap();
    assert_eq!(
        counter(&after),
        counter(&before) + 1,
        "ops::mutate must produce exactly one commit (before={before}, after={after})"
    );

    // The index was regenerated inside the same transaction: the roadmap
    // created by `f` is already referenced in the freshly written INDEX.md.
    let project_index = store
        .read(&rdm_core::paths::project_index_path("fbm"))
        .unwrap();
    assert!(
        project_index.contains("roadmaps/alpha/roadmap.md"),
        "INDEX.md should reference the roadmap created in the same mutate(): {project_index}"
    );
}

#[test]
fn generate_index_writes_project_index() {
    let mut store = setup_with_project();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "alpha", "Alpha", None, None, None)
        .unwrap();
    rdm_core::ops::index::generate_index(&mut store).unwrap();

    // Root index should exist
    let root = store.read(&rdm_core::paths::index_path()).unwrap();
    assert!(root.contains("# Plan Index"));

    // Per-project index should also exist
    let project = store
        .read(&rdm_core::paths::project_index_path("fbm"))
        .unwrap();
    assert!(project.contains("# Project: fbm"));
    assert!(project.contains("roadmaps/alpha/roadmap.md"));
}

// -- Archive roadmap tests --

#[test]
fn archive_roadmap_moves_files() {
    let mut store = setup_with_project();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "alpha", "Alpha", None, None, None)
        .unwrap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "alpha", "core", "Core", None, None, None,
    )
    .unwrap();
    rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "alpha",
        "phase-1-core",
        Some(PhaseStatus::Done),
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        None,
        None,
    )
    .unwrap();

    rdm_core::ops::roadmap::archive_roadmap(&mut store, "fbm", "alpha", false).unwrap();

    // Gone from active
    assert!(!store.exists(&rdm_core::paths::roadmap_path("fbm", "alpha")));
    // Present in archive
    assert!(store.exists(&rdm_core::paths::archived_roadmap_path("fbm", "alpha")));
}

#[test]
fn archive_roadmap_not_found() {
    let mut store = setup_with_project();
    let result = rdm_core::ops::roadmap::archive_roadmap(&mut store, "fbm", "nonexistent", false);
    assert!(matches!(result, Err(Error::RoadmapNotFound(ref s)) if s == "nonexistent"));
}

#[test]
fn archive_roadmap_rejects_incomplete_phases() {
    let mut store = setup_with_project();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "alpha", "Alpha", None, None, None)
        .unwrap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "alpha", "core", "Core", None, None, None,
    )
    .unwrap();

    let result = rdm_core::ops::roadmap::archive_roadmap(&mut store, "fbm", "alpha", false);
    assert!(matches!(
        result,
        Err(Error::RoadmapHasIncompletePhases(ref s)) if s == "alpha"
    ));
}

#[test]
fn archive_roadmap_force_overrides_check() {
    let mut store = setup_with_project();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "alpha", "Alpha", None, None, None)
        .unwrap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "alpha", "core", "Core", None, None, None,
    )
    .unwrap();

    // force=true succeeds even with incomplete phases
    rdm_core::ops::roadmap::archive_roadmap(&mut store, "fbm", "alpha", true).unwrap();
    assert!(store.exists(&rdm_core::paths::archived_roadmap_path("fbm", "alpha")));
}

#[test]
fn archive_roadmap_all_done_no_force_needed() {
    let mut store = setup_with_project();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "alpha", "Alpha", None, None, None)
        .unwrap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "alpha", "core", "Core", None, None, None,
    )
    .unwrap();
    rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "alpha",
        "phase-1-core",
        Some(PhaseStatus::Done),
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        None,
        None,
    )
    .unwrap();

    // All phases done, force=false should succeed
    rdm_core::ops::roadmap::archive_roadmap(&mut store, "fbm", "alpha", false).unwrap();
    assert!(store.exists(&rdm_core::paths::archived_roadmap_path("fbm", "alpha")));
}

#[test]
fn archive_succeeds_with_mixed_done_and_wont_fix() {
    let mut store = setup_with_project();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "alpha", "Alpha", None, None, None)
        .unwrap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "alpha", "core", "Core", None, None, None,
    )
    .unwrap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "alpha", "skip", "Skip", None, None, None,
    )
    .unwrap();
    rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "alpha",
        "phase-1-core",
        Some(PhaseStatus::Done),
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        None,
        None,
    )
    .unwrap();
    rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "alpha",
        "phase-2-skip",
        Some(PhaseStatus::WontFix),
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        None,
        None,
    )
    .unwrap();

    // Mixed done + wont-fix is all-terminal; force=false should succeed
    rdm_core::ops::roadmap::archive_roadmap(&mut store, "fbm", "alpha", false).unwrap();
    assert!(store.exists(&rdm_core::paths::archived_roadmap_path("fbm", "alpha")));
}

#[test]
fn archive_roadmap_cleans_up_dependencies() {
    let mut store = setup_with_project();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "alpha", "Alpha", None, None, None)
        .unwrap();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "beta", "Beta", None, None, None)
        .unwrap();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "gamma", "Gamma", None, None, None)
        .unwrap();

    rdm_core::ops::roadmap::add_dependency(&mut store, "fbm", "beta", "alpha").unwrap();
    rdm_core::ops::roadmap::add_dependency(&mut store, "fbm", "gamma", "alpha").unwrap();
    rdm_core::ops::roadmap::add_dependency(&mut store, "fbm", "gamma", "beta").unwrap();

    rdm_core::ops::roadmap::archive_roadmap(&mut store, "fbm", "alpha", true).unwrap();

    // beta should have no dependencies left
    let beta = rdm_core::io::load_roadmap(&store, "fbm", "beta").unwrap();
    assert_eq!(beta.frontmatter.dependencies, None);

    // gamma should still depend on beta but not alpha
    let gamma = rdm_core::io::load_roadmap(&store, "fbm", "gamma").unwrap();
    assert_eq!(
        gamma.frontmatter.dependencies,
        Some(vec!["beta".to_string()])
    );
}

#[test]
fn archive_roadmap_not_in_active_list() {
    let mut store = setup_with_project();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "alpha", "Alpha", None, None, None)
        .unwrap();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "beta", "Beta", None, None, None)
        .unwrap();

    rdm_core::ops::roadmap::archive_roadmap(&mut store, "fbm", "alpha", true).unwrap();

    let roadmaps = rdm_core::ops::roadmap::list_roadmaps(&store, "fbm", None, None).unwrap();
    let slugs: Vec<_> = roadmaps
        .iter()
        .map(|r| r.frontmatter.roadmap.as_str())
        .collect();
    assert_eq!(slugs, vec!["beta"]);
}

#[test]
fn list_archived_roadmaps_returns_archived() {
    let mut store = setup_with_project();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "alpha", "Alpha", None, None, None)
        .unwrap();

    rdm_core::ops::roadmap::archive_roadmap(&mut store, "fbm", "alpha", true).unwrap();

    let archived = rdm_core::ops::roadmap::list_archived_roadmaps(&store, "fbm").unwrap();
    assert_eq!(archived.len(), 1);
    assert_eq!(archived[0].frontmatter.roadmap, "alpha");
}

#[test]
fn list_archived_roadmaps_empty() {
    let store = setup_with_project();
    let archived = rdm_core::ops::roadmap::list_archived_roadmaps(&store, "fbm").unwrap();
    assert!(archived.is_empty());
}

#[test]
fn unarchive_roadmap_restores_files() {
    let mut store = setup_with_project();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "alpha", "Alpha", None, None, None)
        .unwrap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "alpha", "core", "Core", None, None, None,
    )
    .unwrap();

    rdm_core::ops::roadmap::archive_roadmap(&mut store, "fbm", "alpha", true).unwrap();
    assert!(!store.exists(&rdm_core::paths::roadmap_path("fbm", "alpha")));

    rdm_core::ops::roadmap::unarchive_roadmap(&mut store, "fbm", "alpha").unwrap();
    assert!(store.exists(&rdm_core::paths::roadmap_path("fbm", "alpha")));
    assert!(!store.exists(&rdm_core::paths::archived_roadmap_path("fbm", "alpha")));
}

#[test]
fn unarchive_roadmap_not_found() {
    let mut store = setup_with_project();
    let result = rdm_core::ops::roadmap::unarchive_roadmap(&mut store, "fbm", "nonexistent");
    assert!(matches!(result, Err(Error::RoadmapNotFound(ref s)) if s == "nonexistent"));
}

#[test]
fn unarchive_roadmap_duplicate_slug() {
    let mut store = setup_with_project();
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "alpha", "Alpha", None, None, None)
        .unwrap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "alpha", "core", "Core", None, None, None,
    )
    .unwrap();

    rdm_core::ops::roadmap::archive_roadmap(&mut store, "fbm", "alpha", true).unwrap();

    // Create a new active roadmap with the same slug
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "alpha", "Alpha 2", None, None, None)
        .unwrap();

    let result = rdm_core::ops::roadmap::unarchive_roadmap(&mut store, "fbm", "alpha");
    assert!(matches!(result, Err(Error::DuplicateSlug(ref s)) if s == "alpha"));
}

// -- Body-clobber guard tests for update_phase / update_task / update_roadmap --

#[test]
fn update_phase_empty_body_refused_when_existing_nonempty() {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    rdm_core::ops::project::create_project(&mut store, "fbm", "FBM").unwrap();
    rdm_core::ops::roadmap::create_roadmap(
        &mut store, "fbm", "two-way", "Two-Way", None, None, None,
    )
    .unwrap();
    rdm_core::ops::phase::create_phase(
        &mut store,
        "fbm",
        "two-way",
        "core",
        "Core",
        Some(1),
        Some("Existing body."),
        None,
    )
    .unwrap();
    let result = rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        None,
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Set(String::new()),
        None,
        None,
        None,
    );
    assert!(matches!(result, Err(Error::BodyClobberRefused)));
    let loaded = rdm_core::io::load_phase(&store, "fbm", "two-way", "phase-1-core").unwrap();
    assert_eq!(loaded.body, "Existing body.\n");
}

#[test]
fn update_phase_empty_body_allowed_when_existing_empty() {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    rdm_core::ops::project::create_project(&mut store, "fbm", "FBM").unwrap();
    rdm_core::ops::roadmap::create_roadmap(
        &mut store, "fbm", "two-way", "Two-Way", None, None, None,
    )
    .unwrap();
    rdm_core::ops::phase::create_phase(
        &mut store,
        "fbm",
        "two-way",
        "core",
        "Core",
        Some(1),
        None,
        None,
    )
    .unwrap();
    let updated = rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        None,
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Set(String::new()),
        None,
        None,
        None,
    )
    .unwrap();
    assert!(updated.body.is_empty());
}

#[test]
fn update_phase_empty_body_allowed_with_flag() {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    rdm_core::ops::project::create_project(&mut store, "fbm", "FBM").unwrap();
    rdm_core::ops::roadmap::create_roadmap(
        &mut store, "fbm", "two-way", "Two-Way", None, None, None,
    )
    .unwrap();
    rdm_core::ops::phase::create_phase(
        &mut store,
        "fbm",
        "two-way",
        "core",
        "Core",
        Some(1),
        Some("Existing body."),
        None,
    )
    .unwrap();
    let updated = rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        None,
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Clear,
        None,
        None,
        None,
    )
    .unwrap();
    assert!(updated.body.is_empty());
}

#[test]
fn update_task_empty_body_refused_when_existing_nonempty() {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    rdm_core::ops::project::create_project(&mut store, "fbm", "FBM").unwrap();
    rdm_core::ops::task::create_task(
        &mut store,
        "fbm",
        "fix-bug",
        "Fix Bug",
        rdm_core::model::Priority::Medium,
        None,
        Some("Existing body."),
    )
    .unwrap();
    let result = rdm_core::ops::task::update_task(
        &mut store,
        "fbm",
        "fix-bug",
        None,
        None,
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Set(String::new()),
        None,
        None,
        None,
    );
    assert!(matches!(result, Err(Error::BodyClobberRefused)));
    let loaded = rdm_core::io::load_task(&store, "fbm", "fix-bug").unwrap();
    assert_eq!(loaded.body, "Existing body.\n");
}

#[test]
fn update_task_empty_body_allowed_with_flag() {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    rdm_core::ops::project::create_project(&mut store, "fbm", "FBM").unwrap();
    rdm_core::ops::task::create_task(
        &mut store,
        "fbm",
        "fix-bug",
        "Fix Bug",
        rdm_core::model::Priority::Medium,
        None,
        Some("Existing body."),
    )
    .unwrap();
    let updated = rdm_core::ops::task::update_task(
        &mut store,
        "fbm",
        "fix-bug",
        None,
        None,
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Clear,
        None,
        None,
        None,
    )
    .unwrap();
    assert!(updated.body.is_empty());
}

#[test]
fn update_roadmap_empty_body_refused_when_existing_nonempty() {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    rdm_core::ops::project::create_project(&mut store, "fbm", "FBM").unwrap();
    rdm_core::ops::roadmap::create_roadmap(
        &mut store,
        "fbm",
        "two-way",
        "Two-Way",
        Some("Existing body."),
        None,
        None,
    )
    .unwrap();
    let result = rdm_core::ops::roadmap::update_roadmap(
        &mut store,
        "fbm",
        "two-way",
        rdm_core::ops::BodyUpdate::Set(String::new()),
        rdm_core::ops::PriorityUpdate::Keep,
        rdm_core::ops::TagsUpdate::Keep,
    );
    assert!(matches!(result, Err(Error::BodyClobberRefused)));
    let loaded = rdm_core::io::load_roadmap(&store, "fbm", "two-way").unwrap();
    assert_eq!(loaded.body, "Existing body.\n");
}

#[test]
fn update_roadmap_empty_body_allowed_with_flag() {
    let mut store = MemoryStore::new();
    rdm_core::ops::init::init(&mut store).unwrap();
    rdm_core::ops::project::create_project(&mut store, "fbm", "FBM").unwrap();
    rdm_core::ops::roadmap::create_roadmap(
        &mut store,
        "fbm",
        "two-way",
        "Two-Way",
        Some("Existing body."),
        None,
        None,
    )
    .unwrap();
    let updated = rdm_core::ops::roadmap::update_roadmap(
        &mut store,
        "fbm",
        "two-way",
        rdm_core::ops::BodyUpdate::Clear,
        rdm_core::ops::PriorityUpdate::Keep,
        rdm_core::ops::TagsUpdate::Keep,
    )
    .unwrap();
    assert!(updated.body.is_empty());
}

// -- next_actionable tests --

use rdm_core::ops::next::{NextActionable, next_actionable};

/// Creates `count` phases in a roadmap, returning nothing. Phases are numbered
/// 1..=count with stems `phase-N-pN`.
fn make_phases(store: &mut MemoryStore, roadmap: &str, count: u32) {
    for n in 1..=count {
        rdm_core::ops::phase::create_phase(
            store,
            "fbm",
            roadmap,
            &format!("p{n}"),
            &format!("Phase {n}"),
            Some(n),
            None,
            None,
        )
        .unwrap();
    }
}

fn set_status(store: &mut MemoryStore, roadmap: &str, stem: &str, status: PhaseStatus) {
    rdm_core::ops::phase::update_phase(
        store,
        "fbm",
        roadmap,
        stem,
        Some(status),
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        None,
        None,
    )
    .unwrap();
}

#[test]
fn next_actionable_fresh_roadmap_returns_phase_one() {
    let mut store = setup_with_roadmap();
    make_phases(&mut store, "two-way", 3);
    let result = next_actionable(&store, "fbm", "two-way").unwrap();
    match result {
        NextActionable::Phase(p) => {
            assert_eq!(p.number, 1);
            assert_eq!(p.stem, "phase-1-p1");
            assert_eq!(p.status, PhaseStatus::NotStarted);
            assert_eq!(p.roadmap, "two-way");
        }
        other => panic!("expected Phase, got {other:?}"),
    }
}

#[test]
fn next_actionable_in_progress_outranks_later_not_started() {
    let mut store = setup_with_roadmap();
    make_phases(&mut store, "two-way", 3);
    set_status(&mut store, "two-way", "phase-1-p1", PhaseStatus::Done);
    set_status(&mut store, "two-way", "phase-2-p2", PhaseStatus::InProgress);
    // phase 3 left not-started
    let result = next_actionable(&store, "fbm", "two-way").unwrap();
    match result {
        NextActionable::Phase(p) => {
            assert_eq!(p.number, 2);
            assert_eq!(p.stem, "phase-2-p2");
            assert_eq!(p.status, PhaseStatus::InProgress);
        }
        other => panic!("expected Phase 2, got {other:?}"),
    }
}

#[test]
fn next_actionable_all_terminal_returns_nothing() {
    let mut store = setup_with_roadmap();
    make_phases(&mut store, "two-way", 2);
    set_status(&mut store, "two-way", "phase-1-p1", PhaseStatus::Done);
    set_status(&mut store, "two-way", "phase-2-p2", PhaseStatus::WontFix);
    let result = next_actionable(&store, "fbm", "two-way").unwrap();
    assert_eq!(result, NextActionable::Nothing);
}

#[test]
fn next_actionable_skips_non_actionable_non_terminal() {
    let mut store = setup_with_roadmap();
    make_phases(&mut store, "two-way", 3);
    set_status(
        &mut store,
        "two-way",
        "phase-1-p1",
        PhaseStatus::NeedsReview,
    );
    set_status(&mut store, "two-way", "phase-2-p2", PhaseStatus::Reviewed);
    set_status(&mut store, "two-way", "phase-3-p3", PhaseStatus::Blocked);
    let result = next_actionable(&store, "fbm", "two-way").unwrap();
    assert_eq!(result, NextActionable::Nothing);
}

#[test]
fn next_actionable_reviewed_phase_is_skipped_for_later_not_started() {
    let mut store = setup_with_roadmap();
    make_phases(&mut store, "two-way", 2);
    set_status(&mut store, "two-way", "phase-1-p1", PhaseStatus::Reviewed);
    // phase 2 left not-started: a reviewed phase must not outrank it.
    let result = next_actionable(&store, "fbm", "two-way").unwrap();
    match result {
        NextActionable::Phase(p) => {
            assert_eq!(p.number, 2);
            assert_eq!(p.status, PhaseStatus::NotStarted);
        }
        other => panic!("expected Phase 2, got {other:?}"),
    }
}

#[test]
fn next_actionable_unmet_dependency_blocks() {
    let mut store = setup_with_roadmap();
    make_phases(&mut store, "two-way", 1);
    // A dependency roadmap with an open (not-started) phase.
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "dep", "Dep", None, None, None)
        .unwrap();
    make_phases(&mut store, "dep", 1);
    rdm_core::ops::roadmap::add_dependency(&mut store, "fbm", "two-way", "dep").unwrap();

    let result = next_actionable(&store, "fbm", "two-way").unwrap();
    assert_eq!(
        result,
        NextActionable::BlockedOnDependencies {
            unmet: vec!["dep".to_string()]
        }
    );
}

#[test]
fn next_actionable_missing_dependency_roadmap_is_unmet() {
    let mut store = setup_with_roadmap();
    make_phases(&mut store, "two-way", 1);
    // Point at a dependency roadmap that does not exist, by writing the
    // dependency directly (add_dependency would reject a missing target).
    let mut doc = rdm_core::io::load_roadmap(&store, "fbm", "two-way").unwrap();
    doc.frontmatter.dependencies = Some(vec!["ghost".to_string()]);
    rdm_core::io::write_roadmap(&mut store, "fbm", "two-way", &doc).unwrap();

    let result = next_actionable(&store, "fbm", "two-way").unwrap();
    assert_eq!(
        result,
        NextActionable::BlockedOnDependencies {
            unmet: vec!["ghost".to_string()]
        }
    );
}

#[test]
fn next_actionable_met_dependency_proceeds() {
    let mut store = setup_with_roadmap();
    make_phases(&mut store, "two-way", 1);
    rdm_core::ops::roadmap::create_roadmap(&mut store, "fbm", "dep", "Dep", None, None, None)
        .unwrap();
    make_phases(&mut store, "dep", 1);
    set_status(&mut store, "dep", "phase-1-p1", PhaseStatus::Done);
    rdm_core::ops::roadmap::add_dependency(&mut store, "fbm", "two-way", "dep").unwrap();

    let result = next_actionable(&store, "fbm", "two-way").unwrap();
    match result {
        NextActionable::Phase(p) => assert_eq!(p.number, 1),
        other => panic!("expected Phase, got {other:?}"),
    }
}

#[test]
fn next_actionable_unknown_roadmap_errors() {
    let store = setup_with_roadmap();
    let result = next_actionable(&store, "fbm", "does-not-exist");
    assert!(matches!(result, Err(Error::RoadmapNotFound(_))));
}

#[test]
fn next_actionable_carries_difficulty_and_model() {
    let mut store = setup_with_roadmap();
    make_phases(&mut store, "two-way", 1);
    rdm_core::ops::phase::set_phase_estimate(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-p1",
        rdm_core::ops::DifficultyUpdate::Set(Difficulty::Hard),
        rdm_core::ops::ModelTierUpdate::Set(ModelTier::Large),
    )
    .unwrap();
    let result = next_actionable(&store, "fbm", "two-way").unwrap();
    match result {
        NextActionable::Phase(p) => {
            assert_eq!(p.difficulty, Some(Difficulty::Hard));
            assert_eq!(p.model, Some(ModelTier::Large));
        }
        other => panic!("expected Phase, got {other:?}"),
    }
}

// -- Consolidated single-write create/update-with-estimate entry points --

#[test]
fn create_phase_with_estimate_applies_estimate_in_one_op() {
    let mut store = setup_with_roadmap();
    let doc = rdm_core::ops::phase::create_phase_with_estimate(
        &mut store,
        "fbm",
        "two-way",
        "core",
        "Core Valuation",
        None,
        Some("Body text."),
        Some(vec!["audit".to_string()]),
        rdm_core::ops::DifficultyUpdate::Set(Difficulty::Hard),
        rdm_core::ops::ModelTierUpdate::Keep,
    )
    .unwrap();

    // Estimate applied (Hard → Large via auto-derive) alongside the base fields.
    assert_eq!(doc.frontmatter.difficulty, Some(Difficulty::Hard));
    assert_eq!(doc.frontmatter.model, Some(ModelTier::Large));
    assert_eq!(doc.frontmatter.status, PhaseStatus::NotStarted);
    assert_eq!(doc.frontmatter.title, "Core Valuation");
    assert_eq!(doc.frontmatter.tags, Some(vec!["audit".to_string()]));
    assert_eq!(doc.body, "Body text.");

    // Persisted to disk in the same op, not just returned.
    let loaded = rdm_core::io::load_phase(&store, "fbm", "two-way", "phase-1-core").unwrap();
    assert_eq!(loaded.frontmatter.difficulty, Some(Difficulty::Hard));
    assert_eq!(loaded.frontmatter.model, Some(ModelTier::Large));
    assert_eq!(loaded.frontmatter.status, PhaseStatus::NotStarted);
}

#[test]
fn create_phase_with_estimate_explicit_model_wins_over_derive() {
    let mut store = setup_with_roadmap();
    // Hard would derive Large, but an explicit Small wins.
    let doc = rdm_core::ops::phase::create_phase_with_estimate(
        &mut store,
        "fbm",
        "two-way",
        "core",
        "Core",
        None,
        None,
        None,
        rdm_core::ops::DifficultyUpdate::Set(Difficulty::Hard),
        rdm_core::ops::ModelTierUpdate::Set(ModelTier::Small),
    )
    .unwrap();
    assert_eq!(doc.frontmatter.difficulty, Some(Difficulty::Hard));
    assert_eq!(doc.frontmatter.model, Some(ModelTier::Small));
}

#[test]
fn create_phase_with_estimate_keep_keep_matches_plain_create() {
    let mut store = setup_with_roadmap();
    let doc = rdm_core::ops::phase::create_phase_with_estimate(
        &mut store,
        "fbm",
        "two-way",
        "core",
        "Core Valuation",
        None,
        Some("Body text."),
        Some(vec!["audit".to_string()]),
        rdm_core::ops::DifficultyUpdate::Keep,
        rdm_core::ops::ModelTierUpdate::Keep,
    )
    .unwrap();

    // Behaviorally identical to plain create_phase: no estimate recorded.
    assert_eq!(doc.frontmatter.difficulty, None);
    assert_eq!(doc.frontmatter.model, None);
    assert_eq!(doc.frontmatter.status, PhaseStatus::NotStarted);
    assert_eq!(doc.frontmatter.title, "Core Valuation");
    assert_eq!(doc.frontmatter.tags, Some(vec!["audit".to_string()]));
    assert_eq!(doc.body, "Body text.");
}

#[test]
fn update_phase_with_estimate_applies_status_review_and_estimate_in_one_op() {
    let mut store = setup_with_roadmap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "two-way", "core", "Core", None, None, None,
    )
    .unwrap();

    let doc = rdm_core::ops::phase::update_phase_with_estimate(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        Some(PhaseStatus::NeedsReview),
        rdm_core::ops::TagsUpdate::Set(vec!["reviewed".to_string()]),
        rdm_core::ops::BodyUpdate::Set("Updated body.".to_string()),
        None,
        Some("abc123".to_string()),
        Some("roadmap/two-way".to_string()),
        rdm_core::ops::DifficultyUpdate::Set(Difficulty::Hard),
        rdm_core::ops::ModelTierUpdate::Keep,
    )
    .unwrap();

    // Status + review stamp (from update_phase logic).
    assert_eq!(doc.frontmatter.status, PhaseStatus::NeedsReview);
    assert_eq!(doc.frontmatter.review_sha, Some("abc123".to_string()));
    assert_eq!(
        doc.frontmatter.review_branch,
        Some("roadmap/two-way".to_string())
    );
    // NeedsReview is non-terminal: completed/commit cleared, matching update_phase.
    assert_eq!(doc.frontmatter.completed, None);
    assert_eq!(doc.frontmatter.commit, None);
    // Tags + body applied.
    assert_eq!(doc.frontmatter.tags, Some(vec!["reviewed".to_string()]));
    assert_eq!(doc.body, "Updated body.");
    // Estimate applied in the same op (Hard → Large via auto-derive).
    assert_eq!(doc.frontmatter.difficulty, Some(Difficulty::Hard));
    assert_eq!(doc.frontmatter.model, Some(ModelTier::Large));

    // Persisted, not just returned.
    let loaded = rdm_core::io::load_phase(&store, "fbm", "two-way", "phase-1-core").unwrap();
    assert_eq!(loaded.frontmatter.status, PhaseStatus::NeedsReview);
    assert_eq!(loaded.frontmatter.difficulty, Some(Difficulty::Hard));
    assert_eq!(loaded.frontmatter.model, Some(ModelTier::Large));
    assert_eq!(loaded.frontmatter.review_sha, Some("abc123".to_string()));
}

#[test]
fn update_phase_with_estimate_keep_keep_matches_plain_update() {
    // Two identical phases; apply plain update_phase to one and
    // update_phase_with_estimate(Keep, Keep) to the other with the same args.
    let mut store = setup_with_roadmap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "two-way", "core", "Core", None, None, None,
    )
    .unwrap();
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "two-way", "svc", "Service", None, None, None,
    )
    .unwrap();

    let plain = rdm_core::ops::phase::update_phase(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        Some(PhaseStatus::InProgress),
        rdm_core::ops::TagsUpdate::Set(vec!["x".to_string()]),
        rdm_core::ops::BodyUpdate::Set("Body.".to_string()),
        None,
        None,
        None,
    )
    .unwrap();

    let consolidated = rdm_core::ops::phase::update_phase_with_estimate(
        &mut store,
        "fbm",
        "two-way",
        "phase-2-svc",
        Some(PhaseStatus::InProgress),
        rdm_core::ops::TagsUpdate::Set(vec!["x".to_string()]),
        rdm_core::ops::BodyUpdate::Set("Body.".to_string()),
        None,
        None,
        None,
        rdm_core::ops::DifficultyUpdate::Keep,
        rdm_core::ops::ModelTierUpdate::Keep,
    )
    .unwrap();

    // Same status/tags/body/estimate outcome (modulo phase number/title/stem).
    assert_eq!(consolidated.frontmatter.status, plain.frontmatter.status);
    assert_eq!(consolidated.frontmatter.tags, plain.frontmatter.tags);
    assert_eq!(consolidated.body, plain.body);
    assert_eq!(
        consolidated.frontmatter.difficulty,
        plain.frontmatter.difficulty
    );
    assert_eq!(consolidated.frontmatter.model, plain.frontmatter.model);
    assert_eq!(
        consolidated.frontmatter.completed,
        plain.frontmatter.completed
    );
    assert_eq!(consolidated.frontmatter.commit, plain.frontmatter.commit);
}

/// A [`Store`] wrapper that records every `write` path, so tests can assert the
/// number of writes a single op performs. Delegates all other behavior to an
/// inner [`MemoryStore`].
struct CountingStore {
    inner: MemoryStore,
    writes: Vec<String>,
}

impl CountingStore {
    fn new(inner: MemoryStore) -> Self {
        Self {
            inner,
            writes: Vec::new(),
        }
    }

    /// Number of `write` calls recorded for paths whose string contains
    /// `needle`.
    fn writes_matching(&self, needle: &str) -> usize {
        self.writes.iter().filter(|p| p.contains(needle)).count()
    }
}

impl Store for CountingStore {
    fn read(&self, path: &rdm_core::store::RelPath) -> rdm_core::error::Result<String> {
        self.inner.read(path)
    }

    fn exists(&self, path: &rdm_core::store::RelPath) -> bool {
        self.inner.exists(path)
    }

    fn list(
        &self,
        path: &rdm_core::store::RelPath,
    ) -> rdm_core::error::Result<Vec<rdm_core::store::DirEntry>> {
        self.inner.list(path)
    }

    fn write(
        &mut self,
        path: &rdm_core::store::RelPath,
        content: String,
    ) -> rdm_core::error::Result<()> {
        self.writes.push(path.as_str().to_string());
        self.inner.write(path, content)
    }

    fn delete(&mut self, path: &rdm_core::store::RelPath) -> rdm_core::error::Result<()> {
        self.inner.delete(path)
    }

    fn commit(&mut self) -> rdm_core::error::Result<()> {
        self.inner.commit()
    }

    fn discard(&mut self) {
        self.inner.discard();
    }
}

#[test]
fn update_phase_with_estimate_writes_phase_file_exactly_once() {
    let mut store = CountingStore::new(setup_with_roadmap());
    rdm_core::ops::phase::create_phase(
        &mut store, "fbm", "two-way", "core", "Core", None, None, None,
    )
    .unwrap();

    // Reset the write log so we only measure the update-with-estimate op.
    store.writes.clear();

    rdm_core::ops::phase::update_phase_with_estimate(
        &mut store,
        "fbm",
        "two-way",
        "phase-1-core",
        Some(PhaseStatus::InProgress),
        rdm_core::ops::TagsUpdate::Keep,
        rdm_core::ops::BodyUpdate::Keep,
        None,
        None,
        None,
        rdm_core::ops::DifficultyUpdate::Set(Difficulty::Hard),
        rdm_core::ops::ModelTierUpdate::Keep,
    )
    .unwrap();

    // Exactly ONE write to the phase file — the whole point of the consolidation
    // (the old two-op composition wrote it twice: update_phase + set_phase_estimate).
    assert_eq!(
        store.writes_matching("phase-1-core"),
        1,
        "update_phase_with_estimate must write the phase file exactly once; writes: {:?}",
        store.writes
    );
}
