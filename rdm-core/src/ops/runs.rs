//! Run-record operations: record a run, start and end its units, close it,
//! and list a project's runs.
//!
//! A [`Run`] records **what ran, where, when** — the lane driver, the
//! roadmap or task it drove, the Claude Code session it ran in, and one
//! [`RunUnit`] entry per dispatched unit with its time window and outcome.
//! Every operation here is commit-free and takes the clock as a parameter:
//! core never reads the clock or the environment for run data, so the
//! caller (the CLI) supplies `now` and the session uuid.

use chrono::{DateTime, Utc};

use crate::document::Document;
use crate::error::{Error, Result};
use crate::model::{Run, RunDriver, RunStatus, RunTarget, RunUnit};
use crate::store::{DirEntryKind, Store};

/// Request describing a new run to record.
#[derive(Debug, Clone)]
pub struct CreateRun<'a> {
    /// Project the run belongs to.
    pub project: &'a str,
    /// Which lane driver is opening the run.
    pub driver: RunDriver,
    /// The roadmap or task the run drives. Must exist at record time.
    pub target: RunTarget,
    /// The raw Claude Code session uuid, when known. An empty string is
    /// treated as absent.
    pub session_uuid: Option<&'a str>,
    /// Free-form invocation arguments of the driver. An empty string is
    /// treated as absent.
    pub args: Option<&'a str>,
    /// The mint instant: becomes the run's `started` and seeds its id.
    pub now: DateTime<Utc>,
}

/// Records a new `open` run with no units.
///
/// The id is `YYYY-MM-DD-HHMM-xxxx` from `now` plus a collision-resistant
/// suffix, probed against the project's existing run files.
///
/// # Errors
///
/// Returns [`Error::ProjectNotFound`] if the project doesn't exist,
/// [`Error::RoadmapNotFound`] / [`Error::TaskNotFound`] if the target
/// doesn't exist, [`Error::RunIdExhausted`] if repeated id-generation
/// attempts all collided, [`Error::Io`] if the write fails, or
/// [`Error::FrontmatterParse`] if serialization fails.
///
/// # Examples
///
/// ```
/// use chrono::Utc;
/// use rdm_core::model::{RunDriver, RunStatus, RunTarget};
/// use rdm_core::ops::runs::{CreateRun, create_run};
/// use rdm_core::store::MemoryStore;
///
/// let mut store = MemoryStore::new();
/// rdm_core::ops::init::init(&mut store).unwrap();
/// rdm_core::ops::project::create_project(&mut store, "demo", "Demo").unwrap();
/// rdm_core::ops::task::create_task(
///     &mut store,
///     rdm_core::ops::CreateTask { project: "demo", slug: "fix-bug", title: "Fix", ..Default::default() },
/// )
/// .unwrap();
///
/// let run = create_run(
///     &mut store,
///     CreateRun {
///         project: "demo",
///         driver: RunDriver::DispatchPhase,
///         target: RunTarget::Task("fix-bug".into()),
///         session_uuid: Some("0f3c9a1e-1111-4222-8333-444455556666"),
///         args: None,
///         now: Utc::now(),
///     },
/// )
/// .unwrap();
/// assert_eq!(run.frontmatter.status, RunStatus::Open);
/// assert!(run.frontmatter.units.is_empty());
/// ```
pub fn create_run(store: &mut impl Store, req: CreateRun<'_>) -> Result<Document<Run>> {
    let CreateRun {
        project,
        driver,
        target,
        session_uuid,
        args,
        now,
    } = req;
    if !store.exists(&crate::paths::project_md_path(project)) {
        return Err(Error::ProjectNotFound(project.to_string()));
    }
    match &target {
        RunTarget::Roadmap(slug) => {
            if !store.exists(&crate::paths::roadmap_path(project, slug)) {
                return Err(Error::RoadmapNotFound(slug.clone()));
            }
        }
        RunTarget::Task(slug) => {
            if !store.exists(&crate::paths::task_path(project, slug)) {
                return Err(Error::TaskNotFound(slug.clone()));
            }
        }
    }

    let id = next_available_run_id(store, project, || {
        crate::ops::id::generate_timestamp_id(now)
    })?;
    let non_empty = |s: Option<&str>| s.filter(|v| !v.is_empty()).map(str::to_string);
    let doc = Document {
        frontmatter: Run {
            id: id.clone(),
            project: project.to_string(),
            driver,
            target,
            session_uuid: non_empty(session_uuid),
            args: non_empty(args),
            status: RunStatus::Open,
            started: now,
            ended: None,
            stop_reason: None,
            units: Vec::new(),
        },
        body: String::new(),
    };
    crate::io::write_run(store, project, &id, &doc)?;
    Ok(doc)
}

/// Returns the first candidate id that does not collide with an existing
/// run file, retrying up to
/// [`MAX_ID_ATTEMPTS`](crate::ops::id::MAX_ID_ATTEMPTS) times.
fn next_available_run_id(
    store: &impl Store,
    project: &str,
    candidate: impl FnMut() -> String,
) -> Result<String> {
    crate::ops::id::next_available_id(
        |id| store.exists(&crate::paths::run_path(project, id)),
        candidate,
        Error::RunIdExhausted,
    )
}

/// Loads a single run by id.
///
/// # Errors
///
/// Returns [`Error::RunNotFound`] if the run doesn't exist — including an
/// id that can never name a run because it is not a single path component
/// (such as `../x`) — [`Error::Io`] on read failure, or
/// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] on a malformed
/// run file.
pub fn get_run(store: &impl Store, project: &str, run_id: &str) -> Result<Document<Run>> {
    crate::io::load_run(store, project, run_id)
}

/// Lists all runs for a project, sorted by id — which is chronological,
/// since ids start with the mint timestamp.
///
/// Returns an empty vec if the runs directory doesn't exist. Compose with
/// [`filter_runs`] to narrow by roadmap or task.
///
/// # Errors
///
/// Returns [`Error::ProjectNotFound`] if the project does not exist,
/// [`Error::Io`] if the directory cannot be read, or
/// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] if a run file
/// has invalid frontmatter.
pub fn list_runs(store: &impl Store, project: &str) -> Result<Vec<(String, Document<Run>)>> {
    if !store.exists(&crate::paths::project_md_path(project)) {
        return Err(Error::ProjectNotFound(project.to_string()));
    }
    let entries = store.list(&crate::paths::runs_dir(project))?;
    let mut runs = Vec::new();
    for entry in entries {
        if entry.kind != DirEntryKind::File {
            continue;
        }
        let Some(id) = entry.name.strip_suffix(".md") else {
            continue;
        };
        let doc = crate::io::load_run(store, project, id)?;
        runs.push((id.to_string(), doc));
    }
    runs.sort_by(|(a, _), (b, _)| a.cmp(b));
    Ok(runs)
}

/// Filter criteria for [`run_matches`].
///
/// Each populated field narrows the result independently; the default (all
/// `None`) keeps every run. Matching is by recorded slug only — a run whose
/// roadmap has since been deleted still matches its slug.
#[derive(Debug, Clone, Default)]
pub struct RunFilter {
    /// Keep only runs driving exactly this roadmap.
    pub roadmap: Option<String>,
    /// Keep only runs driving exactly this task.
    pub task: Option<String>,
}

/// Returns whether `run` satisfies every populated criterion in `filter`.
#[must_use]
pub fn run_matches(run: &Run, filter: &RunFilter) -> bool {
    filter
        .roadmap
        .as_deref()
        .is_none_or(|r| run.target.roadmap() == Some(r))
        && filter
            .task
            .as_deref()
            .is_none_or(|t| run.target.task() == Some(t))
}

/// Filters `runs` to those satisfying `filter`, preserving order.
#[must_use]
pub fn filter_runs(
    runs: Vec<(String, Document<Run>)>,
    filter: &RunFilter,
) -> Vec<(String, Document<Run>)> {
    runs.into_iter()
        .filter(|(_, doc)| run_matches(&doc.frontmatter, filter))
        .collect()
}

/// Loads a run and refuses when it is already closed or abandoned.
fn load_open_run(store: &impl Store, project: &str, run_id: &str) -> Result<Document<Run>> {
    let doc = crate::io::load_run(store, project, run_id)?;
    if doc.frontmatter.status.is_terminal() {
        return Err(Error::RunNotOpen {
            run_id: run_id.to_string(),
            status: doc.frontmatter.status,
        });
    }
    Ok(doc)
}

/// Starts a new unit entry on an open run.
///
/// On a roadmap run, `unit` is a phase stem or number, resolved against the
/// run's roadmap (so `3` and `phase-3-run-artifact` name the same unit); on
/// a task run it must be the run's task slug. The new entry's attempt
/// ordinal is one more than the number of entries this run already has for
/// the resolved unit — a rework re-dispatch appends a new entry, never
/// mutates an old one.
///
/// # Errors
///
/// Returns [`Error::RunNotFound`] if the run doesn't exist,
/// [`Error::RunNotOpen`] if it is closed or abandoned,
/// [`Error::RunUnitAlreadyOpen`] while another unit entry is still open,
/// [`Error::PhaseNotFound`] / [`Error::RoadmapNotFound`] if a roadmap run's
/// unit does not resolve to an existing phase — including a unit that is not
/// a single path component (such as `../x`) — [`Error::RunUnitMismatch`] if
/// a task run's unit is not its task, or [`Error::Io`] /
/// [`Error::FrontmatterParse`] on read or write failure.
pub fn start_unit(
    store: &mut impl Store,
    project: &str,
    run_id: &str,
    unit: &str,
    now: DateTime<Utc>,
) -> Result<Document<Run>> {
    let mut doc = load_open_run(store, project, run_id)?;
    if let Some(open) = doc.frontmatter.units.iter().find(|u| !u.is_complete()) {
        return Err(Error::RunUnitAlreadyOpen {
            run_id: run_id.to_string(),
            unit: open.unit.clone(),
            attempt: open.attempt,
        });
    }
    let resolved = match &doc.frontmatter.target {
        RunTarget::Roadmap(roadmap) => {
            let stem = crate::ops::phase::resolve_phase_stem(store, project, roadmap, unit)?;
            // A stem that is not a single path component (`../x`) can never
            // name a phase, and would panic in `phase_path`.
            if !crate::paths::is_single_component(&stem)
                || !store.exists(&crate::paths::phase_path(project, roadmap, &stem))
            {
                return Err(Error::PhaseNotFound(unit.to_string()));
            }
            stem
        }
        RunTarget::Task(task) => {
            if unit != task {
                return Err(Error::RunUnitMismatch {
                    run_id: run_id.to_string(),
                    unit: unit.to_string(),
                    expected: task.clone(),
                });
            }
            task.clone()
        }
    };
    let prior = doc
        .frontmatter
        .units
        .iter()
        .filter(|u| u.unit == resolved)
        .count();
    let attempt = u32::try_from(prior).unwrap_or(u32::MAX).saturating_add(1);
    doc.frontmatter.units.push(RunUnit {
        unit: resolved,
        attempt,
        started: now,
        ended: None,
        outcome: None,
    });
    crate::io::write_run(store, project, run_id, &doc)?;
    Ok(doc)
}

/// Ends the run's single open unit entry with an outcome.
///
/// `outcome` is free-form (the lane's own vocabulary — `reviewed`,
/// `rework`, `escalated`, …) but must be non-empty.
///
/// # Errors
///
/// Returns [`Error::RunNotFound`] if the run doesn't exist,
/// [`Error::RunNotOpen`] if it is closed or abandoned,
/// [`Error::RunOutcomeEmpty`] if `outcome` is empty or whitespace,
/// [`Error::RunNoOpenUnit`] if no unit entry is open, or [`Error::Io`] /
/// [`Error::FrontmatterParse`] on read or write failure.
pub fn end_unit(
    store: &mut impl Store,
    project: &str,
    run_id: &str,
    outcome: &str,
    now: DateTime<Utc>,
) -> Result<Document<Run>> {
    let mut doc = load_open_run(store, project, run_id)?;
    if outcome.trim().is_empty() {
        return Err(Error::RunOutcomeEmpty);
    }
    let open = doc
        .frontmatter
        .units
        .iter_mut()
        .find(|u| !u.is_complete())
        .ok_or_else(|| Error::RunNoOpenUnit(run_id.to_string()))?;
    open.ended = Some(now);
    open.outcome = Some(outcome.to_string());
    crate::io::write_run(store, project, run_id, &doc)?;
    Ok(doc)
}

/// How a run ends: the two terminal [`RunStatus`] values, so `open` is
/// unrepresentable as a close status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunEnd {
    /// The driver finished and recorded why.
    Closed,
    /// The driver was interrupted.
    Abandoned,
}

impl From<RunEnd> for RunStatus {
    fn from(end: RunEnd) -> Self {
        match end {
            RunEnd::Closed => RunStatus::Closed,
            RunEnd::Abandoned => RunStatus::Abandoned,
        }
    }
}

/// Closes or abandons an open run, stamping its end time and stop reason.
///
/// A unit entry still open at close time is left as is — without `ended` —
/// and reported incomplete: recording is observe-only, so a close never
/// fails because the lane skipped a `unit-end`.
///
/// # Errors
///
/// Returns [`Error::RunNotFound`] if the run doesn't exist,
/// [`Error::RunNotOpen`] if it is already closed or abandoned,
/// [`Error::RunStopReasonEmpty`] if `stop_reason` is empty or whitespace,
/// or [`Error::Io`] / [`Error::FrontmatterParse`] on read or write failure.
pub fn close_run(
    store: &mut impl Store,
    project: &str,
    run_id: &str,
    end: RunEnd,
    stop_reason: &str,
    now: DateTime<Utc>,
) -> Result<Document<Run>> {
    let mut doc = load_open_run(store, project, run_id)?;
    if stop_reason.trim().is_empty() {
        return Err(Error::RunStopReasonEmpty);
    }
    doc.frontmatter.status = end.into();
    doc.frontmatter.ended = Some(now);
    doc.frontmatter.stop_reason = Some(stop_reason.to_string());
    crate::io::write_run(store, project, run_id, &doc)?;
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::id::MAX_ID_ATTEMPTS;
    use crate::store::MemoryStore;
    use chrono::TimeZone;

    fn t(min: u32, sec: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 24, 15, min, sec).unwrap()
    }

    fn setup_store() -> MemoryStore {
        let mut store = MemoryStore::new();
        crate::ops::project::create_project(&mut store, "test", "Test Project").unwrap();
        for slug in ["alpha", "beta"] {
            crate::ops::roadmap::create_roadmap(
                &mut store,
                crate::ops::roadmap::CreateRoadmap {
                    project: "test",
                    slug,
                    title: slug,
                    ..Default::default()
                },
            )
            .unwrap();
        }
        for (n, slug) in [(1, "one"), (2, "two")] {
            crate::ops::phase::create_phase(
                &mut store,
                crate::ops::phase::CreatePhase {
                    project: "test",
                    roadmap: "alpha",
                    slug,
                    title: slug,
                    number: Some(n),
                    ..Default::default()
                },
            )
            .unwrap();
        }
        crate::ops::task::create_task(
            &mut store,
            crate::ops::task::CreateTask {
                project: "test",
                slug: "fix-bug",
                title: "Fix bug",
                ..Default::default()
            },
        )
        .unwrap();
        store
    }

    fn record(store: &mut MemoryStore, target: RunTarget, now: DateTime<Utc>) -> String {
        create_run(
            store,
            CreateRun {
                project: "test",
                driver: RunDriver::Autopilot,
                target,
                session_uuid: None,
                args: None,
                now,
            },
        )
        .unwrap()
        .frontmatter
        .id
    }

    fn alpha() -> RunTarget {
        RunTarget::Roadmap("alpha".to_string())
    }

    // -- ids --

    #[test]
    fn next_available_run_id_returns_first_free_candidate() {
        let mut store = setup_store();
        let taken = record(&mut store, alpha(), t(30, 0));
        let mut calls = 0;
        let id = next_available_run_id(&store, "test", || {
            calls += 1;
            if calls == 1 {
                taken.clone()
            } else {
                "free".to_string()
            }
        })
        .unwrap();
        assert_eq!(id, "free");
        assert_eq!(calls, 2);
    }

    #[test]
    fn next_available_run_id_exhausted_after_repeated_collisions() {
        let mut store = setup_store();
        let stuck = record(&mut store, alpha(), t(30, 0));
        let mut calls = 0;
        let result = next_available_run_id(&store, "test", || {
            calls += 1;
            stuck.clone()
        });
        assert!(matches!(result, Err(Error::RunIdExhausted)));
        assert_eq!(calls, MAX_ID_ATTEMPTS);
    }

    #[test]
    fn create_run_twice_at_the_same_instant_yields_distinct_ids() {
        let mut store = setup_store();
        let a = record(&mut store, alpha(), t(30, 0));
        let b = record(&mut store, alpha(), t(30, 0));
        assert_ne!(a, b);
        assert!(a.starts_with("2026-09-24-1530-"), "{a}");
    }

    // -- create / get / list --

    #[test]
    fn create_run_records_an_open_run() {
        let mut store = setup_store();
        let doc = create_run(
            &mut store,
            CreateRun {
                project: "test",
                driver: RunDriver::DispatchPhase,
                target: RunTarget::Task("fix-bug".to_string()),
                session_uuid: Some("sess-1"),
                args: Some("task/fix-bug"),
                now: t(30, 0),
            },
        )
        .unwrap();
        let fm = &doc.frontmatter;
        assert_eq!(fm.status, RunStatus::Open);
        assert_eq!(fm.started, t(30, 0));
        assert_eq!(fm.session_uuid.as_deref(), Some("sess-1"));
        assert_eq!(fm.args.as_deref(), Some("task/fix-bug"));
        assert!(fm.units.is_empty());
        assert_eq!(get_run(&store, "test", &fm.id).unwrap(), doc);
    }

    #[test]
    fn create_run_treats_empty_session_and_args_as_absent() {
        let mut store = setup_store();
        let doc = create_run(
            &mut store,
            CreateRun {
                project: "test",
                driver: RunDriver::Autopilot,
                target: alpha(),
                session_uuid: Some(""),
                args: Some(""),
                now: t(30, 0),
            },
        )
        .unwrap();
        assert_eq!(doc.frontmatter.session_uuid, None);
        assert_eq!(doc.frontmatter.args, None);
    }

    #[test]
    fn create_run_rejects_missing_project_and_targets() {
        let mut store = setup_store();
        let mut req = |project: &'static str, target: RunTarget| {
            create_run(
                &mut store,
                CreateRun {
                    project,
                    driver: RunDriver::Autopilot,
                    target,
                    session_uuid: None,
                    args: None,
                    now: t(30, 0),
                },
            )
        };
        assert!(matches!(
            req("nope", alpha()),
            Err(Error::ProjectNotFound(_))
        ));
        assert!(matches!(
            req("test", RunTarget::Roadmap("ghost".to_string())),
            Err(Error::RoadmapNotFound(_))
        ));
        assert!(matches!(
            req("test", RunTarget::Task("ghost".to_string())),
            Err(Error::TaskNotFound(_))
        ));
    }

    #[test]
    fn list_runs_is_sorted_and_filterable_without_the_target_existing() {
        let mut store = setup_store();
        let late = record(&mut store, alpha(), t(45, 0));
        let early = record(&mut store, RunTarget::Roadmap("beta".to_string()), t(10, 0));
        let task = record(&mut store, RunTarget::Task("fix-bug".to_string()), t(20, 0));

        let all = list_runs(&store, "test").unwrap();
        let ids: Vec<&str> = all.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, vec![early.as_str(), task.as_str(), late.as_str()]);

        crate::ops::roadmap::delete_roadmap(&mut store, "test", "alpha").unwrap();
        let alpha_runs = filter_runs(
            list_runs(&store, "test").unwrap(),
            &RunFilter {
                roadmap: Some("alpha".to_string()),
                ..Default::default()
            },
        );
        assert_eq!(alpha_runs.len(), 1);
        assert_eq!(alpha_runs[0].0, late);
        let task_runs = filter_runs(
            list_runs(&store, "test").unwrap(),
            &RunFilter {
                task: Some("fix-bug".to_string()),
                ..Default::default()
            },
        );
        assert_eq!(task_runs.len(), 1);
        assert_eq!(task_runs[0].0, task);
    }

    #[test]
    fn list_runs_missing_dir_is_empty_and_unknown_project_errors() {
        let store = setup_store();
        assert!(list_runs(&store, "test").unwrap().is_empty());
        assert!(matches!(
            list_runs(&store, "nope"),
            Err(Error::ProjectNotFound(_))
        ));
    }

    #[test]
    fn get_run_survives_its_roadmap_being_deleted() {
        let mut store = setup_store();
        let id = record(&mut store, alpha(), t(30, 0));
        crate::ops::roadmap::delete_roadmap(&mut store, "test", "alpha").unwrap();
        let doc = get_run(&store, "test", &id).unwrap();
        assert_eq!(doc.frontmatter.target, alpha());
        assert!(matches!(
            get_run(&store, "test", "missing"),
            Err(Error::RunNotFound(_))
        ));
    }

    #[test]
    fn a_path_escaping_run_id_is_not_found_rather_than_a_panic() {
        let mut store = setup_store();
        for bad in ["../x", "./x", "a/../b", "..", ".", "", "a\\b", "x/y"] {
            assert!(
                matches!(get_run(&store, "test", bad), Err(Error::RunNotFound(ref id)) if id == bad),
                "get_run {bad:?}"
            );
            assert!(
                matches!(
                    start_unit(&mut store, "test", bad, "1", t(31, 0)),
                    Err(Error::RunNotFound(_))
                ),
                "start_unit {bad:?}"
            );
            assert!(
                matches!(
                    end_unit(&mut store, "test", bad, "reviewed", t(31, 0)),
                    Err(Error::RunNotFound(_))
                ),
                "end_unit {bad:?}"
            );
            assert!(
                matches!(
                    close_run(&mut store, "test", bad, RunEnd::Closed, "done", t(31, 0)),
                    Err(Error::RunNotFound(_))
                ),
                "close_run {bad:?}"
            );
        }
        let id = record(&mut store, alpha(), t(30, 0));
        let doc = get_run(&store, "test", &id).unwrap();
        assert!(matches!(
            crate::io::write_run(&mut store, "test", "../x", &doc),
            Err(Error::RunNotFound(_))
        ));
    }

    #[test]
    fn a_path_escaping_unit_is_not_found_rather_than_a_panic() {
        let mut store = setup_store();
        let roadmap_run = record(&mut store, alpha(), t(30, 0));
        let task_run = record(&mut store, RunTarget::Task("fix-bug".to_string()), t(30, 0));
        for bad in ["../x", "./x", "a/../b", "..", ".", "", "a\\b", "x/y"] {
            assert!(
                matches!(
                    start_unit(&mut store, "test", &roadmap_run, bad, t(31, 0)),
                    Err(Error::PhaseNotFound(ref u)) if u == bad
                ),
                "roadmap run {bad:?}"
            );
            assert!(
                matches!(
                    start_unit(&mut store, "test", &task_run, bad, t(31, 0)),
                    Err(Error::RunUnitMismatch { ref unit, .. }) if unit == bad
                ),
                "task run {bad:?}"
            );
        }
        for id in [&roadmap_run, &task_run] {
            assert!(
                get_run(&store, "test", id)
                    .unwrap()
                    .frontmatter
                    .units
                    .is_empty()
            );
        }
    }

    // -- units --

    #[test]
    fn start_and_end_unit_record_one_complete_entry() {
        let mut store = setup_store();
        let id = record(&mut store, alpha(), t(30, 0));
        let started = start_unit(&mut store, "test", &id, "1", t(31, 0)).unwrap();
        let unit = &started.frontmatter.units[0];
        assert_eq!(unit.unit, "phase-1-one");
        assert_eq!(unit.attempt, 1);
        assert!(!unit.is_complete());

        let ended = end_unit(&mut store, "test", &id, "reviewed", t(40, 0)).unwrap();
        let unit = &ended.frontmatter.units[0];
        assert_eq!(unit.started, t(31, 0));
        assert_eq!(unit.ended, Some(t(40, 0)));
        assert_eq!(unit.outcome.as_deref(), Some("reviewed"));
        assert_eq!(get_run(&store, "test", &id).unwrap(), ended);
    }

    #[test]
    fn attempt_ordinal_counts_per_unit() {
        let mut store = setup_store();
        let id = record(&mut store, alpha(), t(30, 0));
        for (unit, min) in [("1", 31), ("phase-2-two", 33), ("phase-1-one", 35)] {
            start_unit(&mut store, "test", &id, unit, t(min, 0)).unwrap();
            end_unit(&mut store, "test", &id, "rework", t(min + 1, 0)).unwrap();
        }
        let units = get_run(&store, "test", &id).unwrap().frontmatter.units;
        let got: Vec<(&str, u32)> = units.iter().map(|u| (u.unit.as_str(), u.attempt)).collect();
        assert_eq!(
            got,
            vec![("phase-1-one", 1), ("phase-2-two", 1), ("phase-1-one", 2)]
        );
    }

    #[test]
    fn start_unit_refuses_while_another_unit_is_open() {
        let mut store = setup_store();
        let id = record(&mut store, alpha(), t(30, 0));
        start_unit(&mut store, "test", &id, "1", t(31, 0)).unwrap();
        let err = start_unit(&mut store, "test", &id, "2", t(32, 0)).unwrap_err();
        assert!(
            matches!(&err, Error::RunUnitAlreadyOpen { unit, attempt: 1, .. } if unit == "phase-1-one"),
            "{err:?}"
        );
    }

    #[test]
    fn start_unit_rejects_an_unknown_phase() {
        let mut store = setup_store();
        let id = record(&mut store, alpha(), t(30, 0));
        for bad in ["9", "phase-9-typo"] {
            assert!(matches!(
                start_unit(&mut store, "test", &id, bad, t(31, 0)),
                Err(Error::PhaseNotFound(_))
            ));
        }
    }

    #[test]
    fn task_run_units_must_name_the_task() {
        let mut store = setup_store();
        let id = record(&mut store, RunTarget::Task("fix-bug".to_string()), t(30, 0));
        let err = start_unit(&mut store, "test", &id, "other", t(31, 0)).unwrap_err();
        assert!(
            matches!(&err, Error::RunUnitMismatch { expected, .. } if expected == "fix-bug"),
            "{err:?}"
        );
        let doc = start_unit(&mut store, "test", &id, "fix-bug", t(31, 0)).unwrap();
        assert_eq!(doc.frontmatter.units[0].unit, "fix-bug");
    }

    #[test]
    fn end_unit_needs_an_open_unit_and_a_non_empty_outcome() {
        let mut store = setup_store();
        let id = record(&mut store, alpha(), t(30, 0));
        assert!(matches!(
            end_unit(&mut store, "test", &id, "reviewed", t(31, 0)),
            Err(Error::RunNoOpenUnit(_))
        ));
        start_unit(&mut store, "test", &id, "1", t(31, 0)).unwrap();
        assert!(matches!(
            end_unit(&mut store, "test", &id, "  ", t(32, 0)),
            Err(Error::RunOutcomeEmpty)
        ));
    }

    // -- close --

    #[test]
    fn close_run_stamps_end_reason_and_status() {
        let mut store = setup_store();
        let id = record(&mut store, alpha(), t(30, 0));
        let doc = close_run(&mut store, "test", &id, RunEnd::Closed, "done", t(50, 0)).unwrap();
        assert_eq!(doc.frontmatter.status, RunStatus::Closed);
        assert_eq!(doc.frontmatter.ended, Some(t(50, 0)));
        assert_eq!(doc.frontmatter.stop_reason.as_deref(), Some("done"));
        assert!(doc.frontmatter.is_complete());

        let other = record(&mut store, alpha(), t(30, 0));
        let doc = close_run(
            &mut store,
            "test",
            &other,
            RunEnd::Abandoned,
            "interrupted",
            t(51, 0),
        )
        .unwrap();
        assert_eq!(doc.frontmatter.status, RunStatus::Abandoned);
    }

    #[test]
    fn close_run_rejects_an_empty_stop_reason() {
        let mut store = setup_store();
        let id = record(&mut store, alpha(), t(30, 0));
        for end in [RunEnd::Closed, RunEnd::Abandoned] {
            for bad in ["", "   ", "\t\n"] {
                assert!(
                    matches!(
                        close_run(&mut store, "test", &id, end, bad, t(50, 0)),
                        Err(Error::RunStopReasonEmpty)
                    ),
                    "{end:?} {bad:?}"
                );
            }
        }
        let doc = get_run(&store, "test", &id).unwrap();
        assert_eq!(doc.frontmatter.status, RunStatus::Open);
        assert_eq!(doc.frontmatter.stop_reason, None);
        assert_eq!(doc.frontmatter.ended, None);
    }

    #[test]
    fn a_terminal_run_refuses_further_writes() {
        let mut store = setup_store();
        let id = record(&mut store, alpha(), t(30, 0));
        close_run(&mut store, "test", &id, RunEnd::Closed, "done", t(50, 0)).unwrap();
        let not_open = |r: Result<Document<Run>>| {
            matches!(
                r,
                Err(Error::RunNotOpen {
                    status: RunStatus::Closed,
                    ..
                })
            )
        };
        assert!(not_open(close_run(
            &mut store,
            "test",
            &id,
            RunEnd::Abandoned,
            "again",
            t(51, 0)
        )));
        assert!(not_open(start_unit(&mut store, "test", &id, "1", t(52, 0))));
        assert!(not_open(end_unit(&mut store, "test", &id, "x", t(53, 0))));
    }

    #[test]
    fn close_with_an_open_unit_leaves_that_unit_incomplete() {
        let mut store = setup_store();
        let id = record(&mut store, alpha(), t(30, 0));
        start_unit(&mut store, "test", &id, "1", t(31, 0)).unwrap();
        let doc = close_run(
            &mut store,
            "test",
            &id,
            RunEnd::Abandoned,
            "killed",
            t(40, 0),
        )
        .unwrap();
        assert_eq!(doc.frontmatter.status, RunStatus::Abandoned);
        assert!(!doc.frontmatter.units[0].is_complete());
        assert_eq!(doc.frontmatter.units[0].outcome, None);
    }
}
