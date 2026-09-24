//! Joining run records to session spend by time window: what a roadmap cost,
//! and which of its phases dominated.
//!
//! [`cost_report`] takes every run record of a project, the roadmap to report
//! and a [`TranscriptSource`]. It keeps the runs that target the roadmap,
//! locates and reaps each distinct session they name exactly once, and
//! attributes every spend source of those sessions by time. Tokens only: no
//! dollars.
//!
//! # What is attributed, and by which timestamp
//!
//! - An `agent` or `workflow_agent` source is attributed **whole**, by its
//!   anchor: the main-transcript line that launched or returned it.
//! - The `main` source spans the whole session, so it is split **per
//!   request**, each request by its own timestamp.
//!
//! # Windows
//!
//! Every window is half-open, `start <= ts < end`.
//!
//! - A run's window is `[started, ended)`. A run with no `ended` (an `open`
//!   run) ends at the `started` of the next run, of any target, recorded in
//!   the same session, and is unbounded when there is none.
//! - A unit entry's window is `[started, ended)`. An entry with no `ended` is
//!   incomplete: it ends at the next entry's `started` in the same run, or
//!   else at the run window's end.
//! - Within a session, an item belongs to the run with the latest
//!   `started <= ts` among the session's runs of **every** target, and counts
//!   only if `ts` is also inside that run's window. The same rule picks among
//!   a run's unit entries. Each item therefore has at most one owner.
//!
//! # Buckets
//!
//! | bucket | contents |
//! |---|---|
//! | per-unit | inside a complete unit entry's window, not errored; summed per phase stem |
//! | overhead | inside the run window but outside every unit window, not errored |
//! | run unattributed | inside the run window and an errored `workflow_agent`, or inside an incomplete unit entry's window |
//! | session unattributed | an unanchored `agent` or `workflow_agent`, or an untimestamped main request |
//! | excluded | owned by no run of this roadmap; reported, never totalled |
//!
//! For each joined run, the per-unit, overhead and unattributed figures sum
//! to the run's total. For each joined session, its runs' totals, its
//! session-unattributed figure and its excluded figure sum to the session's
//! total. The roadmap total is the sum of the joined runs' totals and every
//! session's unattributed figure.

use std::collections::BTreeMap;
use std::fmt;

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::model::{Run, RunDriver, RunStatus, RunUnit};
use crate::transcript::{
    ModelRow, SessionReport, SourceKind, SourceReport, TranscriptError, TranscriptSource,
    UsageSummary, locate_session, reap_session,
};
use crate::usage::{RequestUsage, UsageLedger};

/// What a roadmap's recorded runs spent, joined by time window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RoadmapCostReport {
    /// The roadmap slug reported.
    pub roadmap: String,
    /// How many runs target the roadmap, by join state.
    pub counts: RunCounts,
    /// Every attributed item summed: the joined runs' totals plus every
    /// session-unattributed item. Excluded items are not part of it.
    pub totals: UsageSummary,
    /// [`totals`](Self::totals) per model, in model-string order.
    pub models: Vec<ModelRow>,
    /// The four buckets that sum to [`totals`](Self::totals).
    pub buckets: Buckets,
    /// One row per phase stem with a unit entry in a joined run, by total
    /// tokens descending, then stem.
    pub phases: Vec<PhaseCost>,
    /// Run-level unattributed items, by run, then timestamp, then id.
    pub unattributed: Vec<UnattributedItem>,
    /// Session-level unattributed items, by session, then kind, then id.
    pub session_unattributed: Vec<SessionUnattributedItem>,
    /// Items outside every run of this roadmap, over all joined sessions.
    pub excluded: ExcludedCost,
    /// Every run targeting the roadmap, by `started`, then id.
    pub runs: Vec<RunCost>,
    /// Every joined session, in the order its first run appears.
    pub sessions: Vec<SessionCost>,
    /// The joined sessions' reap warnings, each prefixed with its session
    /// uuid.
    pub warnings: Vec<String>,
}

/// The runs targeting a roadmap, by join state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct RunCounts {
    /// Every run targeting the roadmap.
    pub runs: u64,
    /// Runs whose session was located and reaped.
    pub joined: u64,
    /// Runs whose session could not be located or read.
    pub missing: u64,
    /// Runs with no session uuid.
    pub unjoinable: u64,
    /// Runs still `open`, whatever their join state.
    pub incomplete: u64,
}

/// The four buckets a roadmap's attributed spend falls into.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Buckets {
    /// Every per-unit item, summed over all phases.
    pub phases: UsageSummary,
    /// Items in a run window but outside every unit window.
    pub overhead: UsageSummary,
    /// Run-level unattributed items.
    pub unattributed: UsageSummary,
    /// Session-level unattributed items.
    pub session_unattributed: UsageSummary,
}

/// One phase stem's per-unit spend across every joined run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PhaseCost {
    /// The phase stem.
    pub stem: String,
    /// Unit entries for the stem across joined runs, incomplete ones included.
    pub attempts: u64,
    /// `ended - started` summed over the complete entries, in milliseconds.
    pub wall_clock_ms: u64,
    /// `true` when an entry never ended, so the wall clock is partial and
    /// that entry's spend is unattributed.
    pub has_incomplete_attempt: bool,
    /// The per-unit spend of the complete entries.
    pub usage: UsageSummary,
    /// [`usage`](Self::usage) per model.
    pub models: Vec<ModelRow>,
}

/// Why an item could not be attributed to a unit or to run overhead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum UnattributedCause {
    /// A Workflow agent whose progress `state` was `"error"`.
    Errored,
    /// Inside a unit entry that never recorded an end time.
    UnitIncomplete {
        /// The unit's phase stem.
        unit: String,
        /// The entry's attempt ordinal.
        attempt: u32,
    },
    /// An `agent` or `workflow_agent` source with no anchor.
    Unanchored,
    /// A main-transcript request with no timestamp.
    Untimestamped,
}

impl fmt::Display for UnattributedCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UnattributedCause::Errored => f.write_str("errored"),
            UnattributedCause::UnitIncomplete { unit, attempt } => {
                write!(f, "unit-incomplete ({unit} attempt {attempt})")
            }
            UnattributedCause::Unanchored => f.write_str("unanchored"),
            UnattributedCause::Untimestamped => f.write_str("untimestamped"),
        }
    }
}

/// A source (or a run of main requests) inside a run window that belongs to
/// no complete unit entry and is not overhead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnattributedItem {
    /// The run whose window holds the item.
    pub run_id: String,
    /// The session the item was spent in.
    pub session_uuid: String,
    /// The source kind.
    pub kind: SourceKind,
    /// The agent id, or the session uuid for main requests.
    pub id: String,
    /// The source's label.
    pub label: String,
    /// The Workflow run of a Workflow agent.
    pub workflow_run_id: Option<String>,
    /// The anchor, or the earliest of the main requests aggregated here.
    pub timestamp: DateTime<Utc>,
    /// Why it is unattributed.
    pub cause: UnattributedCause,
    /// What it spent.
    pub usage: UsageSummary,
}

/// A source (or the main requests) of a session that cannot be placed in any
/// window. Listed once per session, never inside a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionUnattributedItem {
    /// The session the item was spent in.
    pub session_uuid: String,
    /// The source kind.
    pub kind: SourceKind,
    /// The agent id, or the session uuid for main requests.
    pub id: String,
    /// The source's label.
    pub label: String,
    /// The Workflow run of a Workflow agent.
    pub workflow_run_id: Option<String>,
    /// `unanchored` or `untimestamped`.
    pub cause: UnattributedCause,
    /// What it spent.
    pub usage: UsageSummary,
}

/// Items owned by no run of this roadmap: before or after every run, between
/// runs, or inside another target's run. Never part of any total.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct ExcludedCost {
    /// Whole `agent` and `workflow_agent` sources excluded.
    pub sources: u64,
    /// Main-transcript requests excluded.
    pub main_requests: u64,
    /// What they spent.
    pub usage: UsageSummary,
}

/// How a run joined to spend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RunJoin {
    /// Its session was located and reaped.
    Joined,
    /// Its session could not be located or read; the run contributes nothing.
    Missing {
        /// The locate or reap error.
        reason: String,
    },
    /// The run has no session uuid; it contributes nothing.
    Unjoinable,
}

/// One run targeting the roadmap and what it spent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunCost {
    /// The run id.
    pub id: String,
    /// The lane driver.
    pub driver: RunDriver,
    /// The recorded status.
    pub status: RunStatus,
    /// The status as rendered, e.g. `open (incomplete)`.
    pub status_label: String,
    /// `false` while the run is still `open`.
    pub complete: bool,
    /// The session the run executed in.
    pub session_uuid: Option<String>,
    /// How the run joined to spend.
    pub join: RunJoin,
    /// When the run was recorded.
    pub started: DateTime<Utc>,
    /// When the run was closed or abandoned.
    pub ended: Option<DateTime<Utc>>,
    /// Where the run's window ends; `None` is unbounded.
    pub window_end: Option<DateTime<Utc>>,
    /// `ended - started` in milliseconds; `None` while the run has no end.
    pub wall_clock_ms: Option<u64>,
    /// Per-unit, overhead and unattributed spend summed.
    pub totals: UsageSummary,
    /// [`totals`](Self::totals) per model.
    pub models: Vec<ModelRow>,
    /// Spend inside the run window but outside every unit window.
    pub overhead: UsageSummary,
    /// The run's unattributed items summed.
    pub unattributed: UsageSummary,
    /// The run's unit entries, in recorded order.
    pub units: Vec<UnitCost>,
}

/// One unit entry of a run and its per-unit spend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnitCost {
    /// The phase stem.
    pub unit: String,
    /// The attempt ordinal.
    pub attempt: u32,
    /// When the unit started.
    pub started: DateTime<Utc>,
    /// When the unit ended.
    pub ended: Option<DateTime<Utc>>,
    /// Where the entry's window ends; `None` is unbounded.
    pub window_end: Option<DateTime<Utc>>,
    /// `false` when the entry never ended.
    pub complete: bool,
    /// The recorded outcome.
    pub outcome: Option<String>,
    /// `ended - started` in milliseconds, for a complete entry.
    pub wall_clock_ms: Option<u64>,
    /// Per-unit spend. Always zero for an incomplete entry, whose items are
    /// run-unattributed.
    pub usage: UsageSummary,
}

/// One joined session: how its spend divides between this roadmap's runs,
/// the session-unattributed items and the excluded items.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionCost {
    /// The session uuid.
    pub session_uuid: String,
    /// The project-slug directory holding it.
    pub project_slug: String,
    /// This roadmap's runs in the session, in run order.
    pub run_ids: Vec<String>,
    /// Everything the session spent.
    pub totals: UsageSummary,
    /// The totals of this roadmap's runs in the session.
    pub attributed_to_runs: UsageSummary,
    /// The session's unattributed items summed.
    pub unattributed: UsageSummary,
    /// The session's excluded items.
    pub excluded: ExcludedCost,
}

/// A half-open window `[start, end)`; `end: None` is unbounded.
#[derive(Debug, Clone, Copy)]
struct Window {
    start: DateTime<Utc>,
    end: Option<DateTime<Utc>>,
}

impl Window {
    fn contains(self, ts: DateTime<Utc>) -> bool {
        self.start <= ts && self.end.is_none_or(|end| ts < end)
    }
}

/// The position of the window owning `ts` in `windows` (sorted by start):
/// the last one starting at or before `ts`, if `ts` is also inside it.
fn owner(windows: &[Window], ts: DateTime<Utc>) -> Option<usize> {
    let i = windows.iter().rposition(|w| w.start <= ts)?;
    windows[i].contains(ts).then_some(i)
}

fn millis(start: DateTime<Utc>, end: DateTime<Utc>) -> u64 {
    u64::try_from((end - start).num_milliseconds()).unwrap_or(0)
}

fn summary(ledger: &UsageLedger) -> UsageSummary {
    ledger.total().into()
}

fn model_rows(ledger: &UsageLedger) -> Vec<ModelRow> {
    ledger
        .iter()
        .map(|(model, m)| ModelRow {
            model: model.to_owned(),
            usage: (*m).into(),
        })
        .collect()
}

fn ledger_of(requests: &[RequestUsage]) -> UsageLedger {
    UsageLedger::from_requests(requests)
}

fn kind_rank(kind: SourceKind) -> u8 {
    match kind {
        SourceKind::Main => 0,
        SourceKind::Agent => 1,
        SourceKind::WorkflowAgent => 2,
    }
}

/// A run's unit entries in start order, with their windows.
struct UnitWindows {
    /// Indices into the run's `units`, sorted by `started` then position.
    order: Vec<usize>,
    /// The window of `order[k]`.
    windows: Vec<Window>,
}

fn unit_windows(units: &[RunUnit], run_end: Option<DateTime<Utc>>) -> UnitWindows {
    let mut order: Vec<usize> = (0..units.len()).collect();
    order.sort_by_key(|&i| (units[i].started, i));
    let windows = order
        .iter()
        .enumerate()
        .map(|(k, &i)| Window {
            start: units[i].started,
            end: units[i]
                .ended
                .or_else(|| order.get(k + 1).map(|&j| units[j].started))
                .or(run_end),
        })
        .collect();
    UnitWindows { order, windows }
}

/// Where an item with a timestamp lands.
#[derive(Debug, Clone, Copy)]
enum Place {
    Excluded,
    /// Targeted-run position and unit index.
    Unit(usize, usize),
    Overhead(usize),
    Errored(usize),
    /// Targeted-run position and the incomplete unit's index.
    Incomplete(usize, usize),
}

#[derive(Default)]
struct RunAcc {
    total: UsageLedger,
    overhead: UsageLedger,
    unattributed: UsageLedger,
    units: Vec<UsageLedger>,
    items: Vec<UnattributedItem>,
    /// Main requests in an incomplete unit, by unit index: earliest
    /// timestamp and ledger.
    main_items: BTreeMap<usize, (DateTime<Utc>, UsageLedger)>,
}

#[derive(Default)]
struct SessionAcc {
    attributed: UsageLedger,
    unattributed: UsageLedger,
    excluded_sources: u64,
    excluded_main: u64,
    excluded: UsageLedger,
    items: Vec<SessionUnattributedItem>,
    untimestamped_main: Option<(String, UsageLedger)>,
}

struct Join<'a> {
    runs: &'a [Run],
    /// Indices into `runs` of the runs targeting the roadmap, in run order.
    targeted: Vec<usize>,
    /// For each run, its position in `targeted`.
    target_pos: Vec<Option<usize>>,
    run_windows: Vec<Window>,
    unit_windows: Vec<UnitWindows>,
    accs: Vec<RunAcc>,
    totals: UsageLedger,
    phases: BTreeMap<String, UsageLedger>,
    phases_total: UsageLedger,
    overhead: UsageLedger,
    run_unattributed: UsageLedger,
    session_unattributed: UsageLedger,
}

impl Join<'_> {
    fn place(&self, session_runs: &[usize], ts: DateTime<Utc>, errored: bool) -> Place {
        let windows: Vec<Window> = session_runs.iter().map(|&i| self.run_windows[i]).collect();
        let Some(k) = owner(&windows, ts) else {
            return Place::Excluded;
        };
        let ri = session_runs[k];
        let Some(pos) = self.target_pos[ri] else {
            return Place::Excluded;
        };
        if errored {
            return Place::Errored(pos);
        }
        let uw = &self.unit_windows[ri];
        match owner(&uw.windows, ts) {
            None => Place::Overhead(pos),
            Some(k) => {
                let ui = uw.order[k];
                if self.runs[ri].units[ui].is_complete() {
                    Place::Unit(pos, ui)
                } else {
                    Place::Incomplete(pos, ui)
                }
            }
        }
    }

    /// Books one request into every ledger `place` implies.
    fn book(&mut self, place: Place, session: &mut SessionAcc, r: &RequestUsage) {
        if let Place::Excluded = place {
            session.excluded.record(r);
            return;
        }
        self.totals.record(r);
        session.attributed.record(r);
        match place {
            Place::Excluded => {}
            Place::Unit(pos, ui) => {
                let acc = &mut self.accs[pos];
                acc.total.record(r);
                acc.units[ui].record(r);
                let stem = self.runs[self.targeted[pos]].units[ui].unit.clone();
                self.phases.entry(stem).or_default().record(r);
                self.phases_total.record(r);
            }
            Place::Overhead(pos) => {
                let acc = &mut self.accs[pos];
                acc.total.record(r);
                acc.overhead.record(r);
                self.overhead.record(r);
            }
            Place::Errored(pos) | Place::Incomplete(pos, _) => {
                let acc = &mut self.accs[pos];
                acc.total.record(r);
                acc.unattributed.record(r);
                self.run_unattributed.record(r);
            }
        }
    }

    fn book_session_unattributed(&mut self, session: &mut SessionAcc, r: &RequestUsage) {
        self.totals.record(r);
        self.session_unattributed.record(r);
        session.unattributed.record(r);
    }

    fn cause(&self, place: Place) -> Option<UnattributedCause> {
        match place {
            Place::Errored(_) => Some(UnattributedCause::Errored),
            Place::Incomplete(pos, ui) => {
                let unit = &self.runs[self.targeted[pos]].units[ui];
                Some(UnattributedCause::UnitIncomplete {
                    unit: unit.unit.clone(),
                    attempt: unit.attempt,
                })
            }
            _ => None,
        }
    }

    fn join_session(
        &mut self,
        report: &SessionReport,
        session_runs: &[usize],
        session: &mut SessionAcc,
    ) {
        for source in &report.sources {
            if source.kind == SourceKind::Main {
                self.join_main(report, source, session_runs, session);
            } else {
                self.join_whole(report, source, session_runs, session);
            }
        }
    }

    fn join_whole(
        &mut self,
        report: &SessionReport,
        source: &SourceReport,
        session_runs: &[usize],
        session: &mut SessionAcc,
    ) {
        let Some(anchor) = source.anchor.filter(|_| source.anchored) else {
            for r in &source.requests {
                self.book_session_unattributed(session, r);
            }
            session.items.push(SessionUnattributedItem {
                session_uuid: report.session_id.clone(),
                kind: source.kind,
                id: source.id.clone(),
                label: source.label.clone(),
                workflow_run_id: source.run_id.clone(),
                cause: UnattributedCause::Unanchored,
                usage: summary(&ledger_of(&source.requests)),
            });
            return;
        };
        let errored = source.kind == SourceKind::WorkflowAgent && source.errored;
        let place = self.place(session_runs, anchor, errored);
        if let Place::Excluded = place {
            session.excluded_sources += 1;
        }
        for r in &source.requests {
            self.book(place, session, r);
        }
        if let (Some(cause), Place::Errored(pos) | Place::Incomplete(pos, _)) =
            (self.cause(place), place)
        {
            let run_id = self.runs[self.targeted[pos]].id.clone();
            self.accs[pos].items.push(UnattributedItem {
                run_id,
                session_uuid: report.session_id.clone(),
                kind: source.kind,
                id: source.id.clone(),
                label: source.label.clone(),
                workflow_run_id: source.run_id.clone(),
                timestamp: anchor,
                cause,
                usage: summary(&ledger_of(&source.requests)),
            });
        }
    }

    fn join_main(
        &mut self,
        report: &SessionReport,
        source: &SourceReport,
        session_runs: &[usize],
        session: &mut SessionAcc,
    ) {
        for r in &source.requests {
            let Some(ts) = r.timestamp.filter(|_| source.anchored) else {
                self.book_session_unattributed(session, r);
                session
                    .untimestamped_main
                    .get_or_insert_with(|| (source.label.clone(), UsageLedger::new()))
                    .1
                    .record(r);
                continue;
            };
            let place = self.place(session_runs, ts, false);
            match place {
                Place::Excluded => {
                    session.excluded_main += 1;
                    self.book(place, session, r);
                }
                Place::Incomplete(pos, ui) => {
                    self.book(place, session, r);
                    let slot = self.accs[pos]
                        .main_items
                        .entry(ui)
                        .or_insert_with(|| (ts, UsageLedger::new()));
                    slot.0 = slot.0.min(ts);
                    slot.1.record(r);
                }
                _ => self.book(place, session, r),
            }
        }
        let main_items = self
            .accs
            .iter_mut()
            .enumerate()
            .flat_map(|(pos, acc)| {
                std::mem::take(&mut acc.main_items)
                    .into_iter()
                    .map(move |(ui, slot)| (pos, ui, slot))
            })
            .collect::<Vec<_>>();
        for (pos, ui, (ts, ledger)) in main_items {
            let unit = &self.runs[self.targeted[pos]].units[ui];
            let item = UnattributedItem {
                run_id: self.runs[self.targeted[pos]].id.clone(),
                session_uuid: report.session_id.clone(),
                kind: SourceKind::Main,
                id: source.id.clone(),
                label: source.label.clone(),
                workflow_run_id: None,
                timestamp: ts,
                cause: UnattributedCause::UnitIncomplete {
                    unit: unit.unit.clone(),
                    attempt: unit.attempt,
                },
                usage: summary(&ledger),
            };
            self.accs[pos].items.push(item);
        }
    }
}

/// Joins the runs targeting `roadmap` to their sessions' spend by time
/// window.
///
/// `runs` is every run record of the project, of any target: runs of other
/// targets are needed because they own the items recorded inside their own
/// windows, and they clip an `open` run of this roadmap that shares their
/// session. Each distinct session uuid of this roadmap's runs is located and
/// reaped through `src` exactly once. A session that cannot be located or
/// read marks its runs [`RunJoin::Missing`] with the error as the reason, and
/// the rest of the report still proceeds; a run with no session uuid is
/// [`RunJoin::Unjoinable`]. See the [module docs](self) for the windows and
/// buckets.
///
/// # Examples
///
/// ```
/// use chrono::{DateTime, Utc};
/// use rdm_core::cost_report::{cost_report, RunJoin};
/// use rdm_core::model::{Run, RunDriver, RunStatus, RunTarget, RunUnit};
/// use rdm_core::transcript::MemoryTranscriptSource;
///
/// let t = |s: &str| -> DateTime<Utc> { s.parse().unwrap_or_default() };
/// let src = MemoryTranscriptSource::new().with_file("-p/s1.jsonl", concat!(
///     r#"{"type":"assistant","requestId":"a","timestamp":"2026-01-01T10:00:10Z","message":{"model":"opus","usage":{"output_tokens":5}}}"#, "\n",
///     r#"{"type":"assistant","requestId":"b","timestamp":"2026-01-01T10:02:00Z","message":{"model":"opus","usage":{"output_tokens":7}}}"#, "\n",
/// ));
/// let run = Run {
///     id: "r1".into(),
///     project: "demo".into(),
///     driver: RunDriver::Autopilot,
///     target: RunTarget::Roadmap("auth".into()),
///     session_uuid: Some("s1".into()),
///     args: None,
///     status: RunStatus::Closed,
///     started: t("2026-01-01T10:00:00Z"),
///     ended: Some(t("2026-01-01T10:05:00Z")),
///     stop_reason: Some("done".into()),
///     units: vec![RunUnit {
///         unit: "phase-1-login".into(),
///         attempt: 1,
///         started: t("2026-01-01T10:01:00Z"),
///         ended: Some(t("2026-01-01T10:03:00Z")),
///         outcome: Some("reviewed".into()),
///     }],
/// };
/// let report = cost_report("auth", &[run], &src);
/// assert_eq!(report.runs[0].join, RunJoin::Joined);
/// assert_eq!(report.phases[0].stem, "phase-1-login");
/// assert_eq!(report.phases[0].usage.usage.output, 7);
/// assert_eq!(report.buckets.overhead.usage.output, 5);
/// assert_eq!(report.totals.usage.output, 12);
/// ```
#[must_use]
pub fn cost_report<S: TranscriptSource + ?Sized>(
    roadmap: &str,
    runs: &[Run],
    src: &S,
) -> RoadmapCostReport {
    let mut targeted: Vec<usize> = (0..runs.len())
        .filter(|&i| runs[i].target.roadmap() == Some(roadmap))
        .collect();
    targeted.sort_by(|&a, &b| {
        runs[a]
            .started
            .cmp(&runs[b].started)
            .then_with(|| runs[a].id.cmp(&runs[b].id))
    });
    let mut target_pos = vec![None; runs.len()];
    for (pos, &i) in targeted.iter().enumerate() {
        target_pos[i] = Some(pos);
    }

    // Every run of every target, grouped by session and sorted by start.
    let mut by_session: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, run) in runs.iter().enumerate() {
        if let Some(uuid) = run.session_uuid.as_deref() {
            by_session.entry(uuid).or_default().push(i);
        }
    }
    let mut run_windows: Vec<Window> = runs
        .iter()
        .map(|r| Window {
            start: r.started,
            end: r.ended,
        })
        .collect();
    for indices in by_session.values_mut() {
        indices.sort_by(|&a, &b| {
            runs[a]
                .started
                .cmp(&runs[b].started)
                .then_with(|| runs[a].id.cmp(&runs[b].id))
        });
        for (k, &i) in indices.iter().enumerate() {
            if runs[i].ended.is_none() {
                run_windows[i].end = indices.get(k + 1).map(|&j| runs[j].started);
            }
        }
    }
    let unit_windows: Vec<UnitWindows> = runs
        .iter()
        .zip(&run_windows)
        .map(|(r, w)| unit_windows(&r.units, w.end))
        .collect();

    // Each distinct session of this roadmap's runs, located and reaped once.
    let mut session_order: Vec<&str> = Vec::new();
    for &i in &targeted {
        if let Some(uuid) = runs[i].session_uuid.as_deref()
            && !session_order.contains(&uuid)
        {
            session_order.push(uuid);
        }
    }
    let reaped: Vec<(&str, Result<SessionReport, TranscriptError>)> = session_order
        .iter()
        .map(|&uuid| {
            (
                uuid,
                locate_session(src, uuid).and_then(|loc| reap_session(src, &loc)),
            )
        })
        .collect();

    let mut join = Join {
        runs,
        accs: targeted
            .iter()
            .map(|&i| RunAcc {
                units: vec![UsageLedger::new(); runs[i].units.len()],
                ..RunAcc::default()
            })
            .collect(),
        targeted,
        target_pos,
        run_windows,
        unit_windows,
        totals: UsageLedger::new(),
        phases: BTreeMap::new(),
        phases_total: UsageLedger::new(),
        overhead: UsageLedger::new(),
        run_unattributed: UsageLedger::new(),
        session_unattributed: UsageLedger::new(),
    };

    let mut sessions = Vec::new();
    let mut session_items = Vec::new();
    let mut warnings = Vec::new();
    for (uuid, result) in &reaped {
        let Ok(report) = result else { continue };
        let session_runs = by_session.get(uuid).cloned().unwrap_or_default();
        let mut acc = SessionAcc::default();
        join.join_session(report, &session_runs, &mut acc);
        let mut items = std::mem::take(&mut acc.items);
        if let Some((label, ledger)) = acc.untimestamped_main.take() {
            items.push(SessionUnattributedItem {
                session_uuid: report.session_id.clone(),
                kind: SourceKind::Main,
                id: report.session_id.clone(),
                label,
                workflow_run_id: None,
                cause: UnattributedCause::Untimestamped,
                usage: summary(&ledger),
            });
        }
        items.sort_by(|a, b| {
            kind_rank(a.kind)
                .cmp(&kind_rank(b.kind))
                .then_with(|| a.id.cmp(&b.id))
        });
        session_items.extend(items);
        warnings.extend(report.warnings.iter().map(|w| format!("{uuid}: {w}")));
        sessions.push(SessionCost {
            session_uuid: report.session_id.clone(),
            project_slug: report.project_slug.clone(),
            run_ids: join
                .targeted
                .iter()
                .filter(|&&i| runs[i].session_uuid.as_deref() == Some(uuid))
                .map(|&i| runs[i].id.clone())
                .collect(),
            totals: report.totals,
            attributed_to_runs: summary(&acc.attributed),
            unattributed: summary(&acc.unattributed),
            excluded: ExcludedCost {
                sources: acc.excluded_sources,
                main_requests: acc.excluded_main,
                usage: summary(&acc.excluded),
            },
        });
    }

    let excluded = sessions
        .iter()
        .fold(ExcludedCost::default(), |acc, s| ExcludedCost {
            sources: acc.sources.saturating_add(s.excluded.sources),
            main_requests: acc.main_requests.saturating_add(s.excluded.main_requests),
            usage: add_summary(acc.usage, s.excluded.usage),
        });

    // Runs, phases and counts.
    let mut counts = RunCounts::default();
    let mut run_costs = Vec::with_capacity(join.targeted.len());
    let mut phase_meta: BTreeMap<String, (u64, u64, bool)> = BTreeMap::new();
    let mut unattributed = Vec::new();
    for (pos, &i) in join.targeted.iter().enumerate() {
        let run = &runs[i];
        let acc = &mut join.accs[pos];
        let state = match run.session_uuid.as_deref() {
            None => RunJoin::Unjoinable,
            Some(uuid) => match reaped.iter().find(|(u, _)| *u == uuid) {
                Some((_, Err(e))) => RunJoin::Missing {
                    reason: e.to_string(),
                },
                _ => RunJoin::Joined,
            },
        };
        counts.runs += 1;
        match state {
            RunJoin::Joined => counts.joined += 1,
            RunJoin::Missing { .. } => counts.missing += 1,
            RunJoin::Unjoinable => counts.unjoinable += 1,
        }
        if !run.is_complete() {
            counts.incomplete += 1;
        }
        let uw = &join.unit_windows[i];
        let units = run
            .units
            .iter()
            .enumerate()
            .map(|(ui, u)| {
                let k = uw.order.iter().position(|&o| o == ui).unwrap_or(ui);
                UnitCost {
                    unit: u.unit.clone(),
                    attempt: u.attempt,
                    started: u.started,
                    ended: u.ended,
                    window_end: uw.windows.get(k).and_then(|w| w.end),
                    complete: u.is_complete(),
                    outcome: u.outcome.clone(),
                    wall_clock_ms: u.ended.map(|e| millis(u.started, e)),
                    usage: summary(&acc.units[ui]),
                }
            })
            .collect();
        if state == RunJoin::Joined {
            for u in &run.units {
                let meta = phase_meta.entry(u.unit.clone()).or_default();
                meta.0 += 1;
                match u.ended {
                    Some(e) => meta.1 = meta.1.saturating_add(millis(u.started, e)),
                    None => meta.2 = true,
                }
            }
        }
        unattributed.append(&mut acc.items);
        run_costs.push(RunCost {
            id: run.id.clone(),
            driver: run.driver,
            status: run.status,
            status_label: run.status_label(),
            complete: run.is_complete(),
            session_uuid: run.session_uuid.clone(),
            join: state,
            started: run.started,
            ended: run.ended,
            window_end: join.run_windows[i].end,
            wall_clock_ms: run.ended.map(|e| millis(run.started, e)),
            totals: summary(&acc.total),
            models: model_rows(&acc.total),
            overhead: summary(&acc.overhead),
            unattributed: summary(&acc.unattributed),
            units,
        });
    }
    let run_rank: BTreeMap<&str, usize> = run_costs
        .iter()
        .enumerate()
        .map(|(k, r)| (r.id.as_str(), k))
        .collect();
    unattributed.sort_by(|a, b| {
        run_rank
            .get(a.run_id.as_str())
            .cmp(&run_rank.get(b.run_id.as_str()))
            .then_with(|| a.timestamp.cmp(&b.timestamp))
            .then_with(|| a.id.cmp(&b.id))
    });

    let mut phases: Vec<PhaseCost> = phase_meta
        .into_iter()
        .map(
            |(stem, (attempts, wall_clock_ms, has_incomplete_attempt))| {
                let ledger = join.phases.remove(&stem).unwrap_or_default();
                PhaseCost {
                    stem,
                    attempts,
                    wall_clock_ms,
                    has_incomplete_attempt,
                    usage: summary(&ledger),
                    models: model_rows(&ledger),
                }
            },
        )
        .collect();
    phases.sort_by(|a, b| {
        b.usage
            .total
            .cmp(&a.usage.total)
            .then_with(|| a.stem.cmp(&b.stem))
    });

    RoadmapCostReport {
        roadmap: roadmap.to_owned(),
        counts,
        totals: summary(&join.totals),
        models: model_rows(&join.totals),
        buckets: Buckets {
            phases: summary(&join.phases_total),
            overhead: summary(&join.overhead),
            unattributed: summary(&join.run_unattributed),
            session_unattributed: summary(&join.session_unattributed),
        },
        phases,
        unattributed,
        session_unattributed: session_items,
        excluded,
        runs: run_costs,
        sessions,
        warnings,
    }
}

fn add_summary(a: UsageSummary, b: UsageSummary) -> UsageSummary {
    UsageSummary {
        requests: a.requests.saturating_add(b.requests),
        usage: a.usage + b.usage,
        total: a.total.saturating_add(b.total),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    use super::*;
    use crate::model::RunTarget;
    use crate::transcript::{
        MemoryTranscriptSource, TranscriptEntry, TranscriptPath, TranscriptSource,
    };

    /// `2026-01-01T10:MM:SSZ`.
    fn at(min: u32, sec: u32) -> String {
        format!("2026-01-01T10:{min:02}:{sec:02}Z")
    }

    fn t(min: u32, sec: u32) -> DateTime<Utc> {
        at(min, sec)
            .parse()
            .unwrap_or_else(|e| panic!("bad test time: {e}"))
    }

    /// An assistant line whose usage is `n` input, `2n` output, `3n` cache
    /// write and `4n` cache read, so every class is exercised. `tool_use`
    /// adds an `Agent` tool_use block with that id.
    fn line(req: &str, ts: Option<&str>, model: &str, n: u64, tool_use: Option<&str>) -> String {
        let ts = ts.map_or(String::new(), |ts| format!(r#""timestamp":"{ts}","#));
        let content = tool_use.map_or(String::new(), |id| {
            format!(r#""content":[{{"type":"tool_use","id":"{id}","name":"Agent"}}],"#)
        });
        format!(
            r#"{{"type":"assistant","requestId":"{req}",{ts}"message":{{"model":"{model}",{content}"usage":{{"input_tokens":{n},"output_tokens":{},"cache_creation_input_tokens":{},"cache_read_input_tokens":{}}}}}}}"#,
            2 * n,
            3 * n,
            4 * n
        )
    }

    fn result_line(ts: &str, run_id: &str) -> String {
        format!(r#"{{"type":"user","timestamp":"{ts}","toolUseResult":{{"runId":"{run_id}"}}}}"#)
    }

    fn meta(description: &str, tool_use: &str) -> String {
        format!(
            r#"{{"agentType":"general-purpose","description":"{description}","toolUseId":"{tool_use}"}}"#
        )
    }

    /// Adds an `Agent` subagent `id` with one request of size `n`.
    fn agent(
        src: MemoryTranscriptSource,
        session: &str,
        id: &str,
        tool_use: &str,
        n: u64,
    ) -> MemoryTranscriptSource {
        src.with_file(
            &format!("-p/{session}/subagents/agent-{id}.jsonl"),
            line(&format!("req-{id}"), None, "haiku", n, None),
        )
        .with_file(
            &format!("-p/{session}/subagents/agent-{id}.meta.json"),
            meta(&format!("agent {id}"), tool_use),
        )
    }

    fn unit(
        stem: &str,
        attempt: u32,
        started: DateTime<Utc>,
        ended: Option<DateTime<Utc>>,
    ) -> RunUnit {
        RunUnit {
            unit: stem.to_owned(),
            attempt,
            started,
            ended,
            outcome: ended.map(|_| "reviewed".to_owned()),
        }
    }

    fn run(
        id: &str,
        target: RunTarget,
        session: Option<&str>,
        started: DateTime<Utc>,
        ended: Option<DateTime<Utc>>,
        units: Vec<RunUnit>,
    ) -> Run {
        Run {
            id: id.to_owned(),
            project: "demo".to_owned(),
            driver: RunDriver::Autopilot,
            target,
            session_uuid: session.map(str::to_owned),
            args: None,
            status: if ended.is_some() {
                RunStatus::Closed
            } else {
                RunStatus::Open
            },
            started,
            ended,
            stop_reason: ended.map(|_| "done".to_owned()),
            units,
        }
    }

    fn roadmap(slug: &str) -> RunTarget {
        RunTarget::Roadmap(slug.to_owned())
    }

    fn sum(parts: &[UsageSummary]) -> UsageSummary {
        parts
            .iter()
            .copied()
            .fold(UsageSummary::default(), add_summary)
    }

    fn assert_run_identity(r: &RunCost) {
        let units: Vec<UsageSummary> = r.units.iter().map(|u| u.usage).collect();
        assert_eq!(
            sum(&[sum(&units), r.overhead, r.unattributed]),
            r.totals,
            "per-unit + overhead + unattributed == run total, per class, for {}",
            r.id
        );
    }

    fn assert_session_conservation(report: &RoadmapCostReport) {
        for s in &report.sessions {
            let runs: Vec<UsageSummary> = report
                .runs
                .iter()
                .filter(|r| s.run_ids.contains(&r.id))
                .map(|r| r.totals)
                .collect();
            assert_eq!(sum(&runs), s.attributed_to_runs);
            assert_eq!(
                sum(&[s.attributed_to_runs, s.unattributed, s.excluded.usage]),
                s.totals,
                "session {} conserves its spend",
                s.session_uuid
            );
        }
    }

    fn assert_roadmap_identity(report: &RoadmapCostReport) {
        let runs: Vec<UsageSummary> = report.runs.iter().map(|r| r.totals).collect();
        let sessions: Vec<UsageSummary> = report.sessions.iter().map(|s| s.unattributed).collect();
        assert_eq!(sum(&[sum(&runs), sum(&sessions)]), report.totals);
        let b = report.buckets;
        assert_eq!(
            sum(&[b.phases, b.overhead, b.unattributed, b.session_unattributed]),
            report.totals
        );
        let phases: Vec<UsageSummary> = report.phases.iter().map(|p| p.usage).collect();
        assert_eq!(sum(&phases), b.phases);
    }

    fn unattributed_pairs(report: &RoadmapCostReport) -> Vec<(String, String)> {
        report
            .unattributed
            .iter()
            .map(|u| (u.label.clone(), u.cause.to_string()))
            .collect()
    }

    /// One session exercising every run bucket: main requests before, in and
    /// between units and inside an incomplete unit; an agent in a complete
    /// unit; an agent in the incomplete unit; and an errored Workflow agent
    /// anchored inside the complete unit.
    fn every_bucket_session() -> MemoryTranscriptSource {
        let main = [
            line("m-overhead", Some(&at(0, 10)), "opus", 1, None),
            line("m-unit", Some(&at(1, 10)), "opus", 2, Some("tu-a")),
            result_line(&at(1, 20), "wf_r"),
            line("m-between", Some(&at(3, 0)), "opus", 3, None),
            line("m-incomplete", Some(&at(4, 10)), "opus", 4, Some("tu-b")),
        ]
        .join("\n");
        let src = MemoryTranscriptSource::new().with_file("-p/s1.jsonl", main);
        let src = agent(src, "s1", "a", "tu-a", 10);
        let src = agent(src, "s1", "b", "tu-b", 20);
        src.with_file(
            "-p/s1/workflows/wf_r.json",
            r#"{"runId":"wf_r","workflowName":"review","workflowProgress":[{"type":"workflow_agent","agentId":"w","label":"refute:1","state":"error"}]}"#,
        )
        .with_file(
            "-p/s1/subagents/workflows/wf_r/agent-w.jsonl",
            line("req-w", None, "sonnet", 30, None),
        )
    }

    #[test]
    fn run_total_is_per_unit_plus_overhead_plus_unattributed() {
        let src = every_bucket_session();
        let r = run(
            "r1",
            roadmap("demo"),
            Some("s1"),
            t(0, 0),
            Some(t(9, 0)),
            vec![
                unit("phase-1-a", 1, t(1, 0), Some(t(2, 0))),
                unit("phase-2-b", 1, t(4, 0), None),
            ],
        );
        let report = cost_report("demo", &[r], &src);
        let run = &report.runs[0];
        assert_eq!(run.join, RunJoin::Joined);
        assert_run_identity(run);
        assert_session_conservation(&report);
        assert_roadmap_identity(&report);

        // Per unit: main m-unit (2) and agent a (10).
        assert_eq!(run.units[0].usage.usage.input, 12);
        // Overhead: m-overhead (1) and m-between (3).
        assert_eq!(run.overhead.usage.input, 4);
        // Unattributed: w errored (30), b (20) and m-incomplete (4) in the
        // incomplete unit.
        assert_eq!(run.unattributed.usage.input, 54);
        assert_eq!(run.units[1].usage, UsageSummary::default());
        assert_eq!(
            unattributed_pairs(&report),
            [
                ("review / refute:1".to_owned(), "errored".to_owned()),
                // Same timestamp: by id, and `b` sorts before `s1`.
                (
                    "agent b".to_owned(),
                    "unit-incomplete (phase-2-b attempt 1)".to_owned()
                ),
                (
                    "main session".to_owned(),
                    "unit-incomplete (phase-2-b attempt 1)".to_owned()
                ),
            ]
        );
        assert!(report.unattributed.iter().all(|u| u.run_id == "r1"));
        assert_eq!(report.excluded, ExcludedCost::default());
        assert_eq!(report.totals, report.sessions[0].totals);
    }

    #[test]
    fn a_rework_pair_attributes_both_entries_to_its_stem() {
        let main = [
            line("m1", Some(&at(1, 10)), "opus", 1, Some("tu-a")),
            line("m2", Some(&at(5, 10)), "opus", 2, Some("tu-b")),
        ]
        .join("\n");
        let src = MemoryTranscriptSource::new().with_file("-p/s1.jsonl", main);
        let src = agent(src, "s1", "a", "tu-a", 10);
        let src = agent(src, "s1", "b", "tu-b", 20);
        let r = run(
            "r1",
            roadmap("demo"),
            Some("s1"),
            t(0, 0),
            Some(t(9, 0)),
            vec![
                unit("phase-1-a", 1, t(1, 0), Some(t(2, 0))),
                unit("phase-1-a", 2, t(5, 0), Some(t(6, 30))),
            ],
        );
        let report = cost_report("demo", &[r], &src);
        assert_eq!(report.phases.len(), 1);
        let phase = &report.phases[0];
        assert_eq!(phase.stem, "phase-1-a");
        assert_eq!(phase.attempts, 2);
        assert_eq!(phase.usage.usage.input, 1 + 10 + 2 + 20);
        assert_eq!(phase.wall_clock_ms, 60_000 + 90_000);
        assert!(!phase.has_incomplete_attempt);
        assert_run_identity(&report.runs[0]);
    }

    #[test]
    fn items_outside_the_run_window_are_excluded_and_counted() {
        let main = [
            line("m-before", Some(&at(0, 30)), "opus", 1, None),
            line("m-in", Some(&at(1, 30)), "opus", 2, None),
            line("m-launch", Some(&at(6, 0)), "opus", 3, Some("tu-late")),
        ]
        .join("\n");
        let src = MemoryTranscriptSource::new().with_file("-p/s1.jsonl", main);
        let src = agent(src, "s1", "late", "tu-late", 50);
        let r = run(
            "r1",
            roadmap("demo"),
            Some("s1"),
            t(1, 0),
            Some(t(5, 0)),
            vec![],
        );
        let report = cost_report("demo", &[r], &src);
        // m-before precedes the run; m-launch and the agent it launched
        // follow its end.
        assert_eq!(report.excluded.sources, 1);
        assert_eq!(report.excluded.main_requests, 2);
        assert_eq!(report.excluded.usage.usage.input, 1 + 3 + 50);
        assert_eq!(report.totals.usage.input, 2);
        assert_eq!(report.buckets.overhead.usage.input, 2);
        assert!(report.unattributed.is_empty());
        assert!(report.session_unattributed.is_empty());
        assert_session_conservation(&report);
        assert_roadmap_identity(&report);
    }

    #[test]
    fn a_window_end_is_exclusive() {
        let main = [
            line("m-end", Some(&at(2, 0)), "opus", 1, None),
            line("m-run-end", Some(&at(5, 0)), "opus", 2, None),
        ]
        .join("\n");
        let src = MemoryTranscriptSource::new().with_file("-p/s1.jsonl", main);
        let r = run(
            "r1",
            roadmap("demo"),
            Some("s1"),
            t(1, 0),
            Some(t(5, 0)),
            vec![unit("phase-1-a", 1, t(1, 0), Some(t(2, 0)))],
        );
        let report = cost_report("demo", &[r], &src);
        assert_eq!(report.buckets.phases.requests, 0);
        assert_eq!(report.buckets.overhead.usage.input, 1);
        assert_eq!(report.excluded.usage.usage.input, 2);
    }

    #[test]
    fn phases_are_ordered_by_total_then_stem_with_attempts_and_wall_clock() {
        let main = [
            line("m-c", Some(&at(1, 10)), "opus", 100, None),
            line("m-a", Some(&at(2, 10)), "opus", 50, None),
            line("m-b", Some(&at(3, 10)), "opus", 50, None),
            line("m-d", Some(&at(4, 10)), "opus", 5, None),
        ]
        .join("\n");
        let src = MemoryTranscriptSource::new().with_file("-p/s1.jsonl", main);
        let r = run(
            "r1",
            roadmap("demo"),
            Some("s1"),
            t(0, 0),
            Some(t(9, 0)),
            vec![
                unit("phase-3-c", 1, t(1, 0), Some(t(1, 30))),
                unit("phase-2-b", 1, t(2, 0), Some(t(2, 20))),
                unit("phase-1-a", 1, t(3, 0), Some(t(3, 45))),
                unit("phase-4-d", 1, t(4, 0), None),
            ],
        );
        // phase-2-b's window [2:00, 2:20) holds m-a; phase-1-a's holds m-b.
        let report = cost_report("demo", &[r], &src);
        let rows: Vec<(&str, u64, u64, bool)> = report
            .phases
            .iter()
            .map(|p| {
                (
                    p.stem.as_str(),
                    p.usage.usage.input,
                    p.wall_clock_ms,
                    p.has_incomplete_attempt,
                )
            })
            .collect();
        assert_eq!(
            rows,
            [
                ("phase-3-c", 100, 30_000, false),
                ("phase-1-a", 50, 45_000, false),
                ("phase-2-b", 50, 20_000, false),
                ("phase-4-d", 0, 0, true),
            ]
        );
        assert!(report.phases.iter().all(|p| p.attempts == 1));
        // Every class is carried per phase.
        let c = &report.phases[0].usage.usage;
        assert_eq!(
            (c.input, c.output, c.cache_write_5m, c.cache_read),
            (100, 200, 300, 400)
        );
    }

    #[test]
    fn a_roadmap_driven_by_two_runs_in_two_sessions_aggregates_both() {
        let src = MemoryTranscriptSource::new()
            .with_file("-p/s1.jsonl", line("m1", Some(&at(1, 10)), "opus", 1, None))
            .with_file(
                "-p2/s2.jsonl",
                [
                    line("m2", Some(&at(21, 10)), "opus", 2, None),
                    line("m3", Some(&at(25, 0)), "opus", 4, None),
                ]
                .join("\n"),
            );
        let runs = [
            run(
                "r1",
                roadmap("demo"),
                Some("s1"),
                t(0, 0),
                Some(t(9, 0)),
                vec![unit("phase-1-a", 1, t(1, 0), Some(t(2, 0)))],
            ),
            run(
                "r2",
                roadmap("demo"),
                Some("s2"),
                t(20, 0),
                Some(t(29, 0)),
                vec![unit("phase-1-a", 2, t(21, 0), Some(t(22, 0)))],
            ),
        ];
        let report = cost_report("demo", &runs, &src);
        assert_eq!(report.counts.joined, 2);
        assert_eq!(report.sessions.len(), 2);
        assert_eq!(report.totals.usage.input, 1 + 2 + 4);
        assert_eq!(report.phases.len(), 1);
        assert_eq!(report.phases[0].attempts, 2);
        assert_eq!(report.phases[0].usage.usage.input, 3);
        assert_eq!(report.buckets.overhead.usage.input, 4);
        for r in &report.runs {
            assert_run_identity(r);
        }
        assert_roadmap_identity(&report);
        assert_session_conservation(&report);
    }

    /// A [`TranscriptSource`] that counts how often each file is read.
    struct Counting {
        inner: MemoryTranscriptSource,
        reads: RefCell<BTreeMap<String, u32>>,
    }

    impl TranscriptSource for Counting {
        fn root_display(&self) -> String {
            self.inner.root_display()
        }

        fn list(&self, dir: &TranscriptPath) -> Result<Vec<TranscriptEntry>, TranscriptError> {
            self.inner.list(dir)
        }

        fn read(&self, file: &TranscriptPath) -> Result<String, TranscriptError> {
            *self.reads.borrow_mut().entry(file.to_string()).or_default() += 1;
            self.inner.read(file)
        }
    }

    #[test]
    fn two_runs_in_one_session_count_each_source_once() {
        let main = [
            line("m1", Some(&at(1, 10)), "opus", 1, None),
            line("m2", Some(&at(12, 0)), "opus", 2, None),
            line("m-gap", Some(&at(10, 30)), "opus", 8, None),
        ]
        .join("\n");
        let src = MemoryTranscriptSource::new().with_file("-p/s1.jsonl", main);
        let src = agent(src, "s1", "orphan", "tu-missing", 40);
        let src = Counting {
            inner: src,
            reads: RefCell::new(BTreeMap::new()),
        };
        let runs = [
            run(
                "r1",
                roadmap("demo"),
                Some("s1"),
                t(0, 0),
                Some(t(10, 0)),
                vec![unit("phase-1-a", 1, t(1, 0), Some(t(2, 0)))],
            ),
            run(
                "r2",
                roadmap("demo"),
                Some("s1"),
                t(11, 0),
                Some(t(15, 0)),
                vec![unit("phase-2-b", 1, t(11, 30), Some(t(12, 30)))],
            ),
        ];
        let report = cost_report("demo", &runs, &src);
        assert_eq!(
            src.reads.borrow().get("-p/s1.jsonl").copied(),
            Some(1),
            "the shared session is reaped once"
        );
        assert_eq!(report.sessions.len(), 1);
        assert_eq!(report.sessions[0].run_ids, ["r1", "r2"]);
        let orphans: Vec<&SessionUnattributedItem> = report
            .session_unattributed
            .iter()
            .filter(|i| i.id == "orphan")
            .collect();
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].cause, UnattributedCause::Unanchored);
        let session = &report.sessions[0];
        // m-gap falls between the runs.
        assert_eq!(session.excluded.main_requests, 1);
        assert_eq!(
            report.totals,
            UsageSummary {
                requests: session.totals.requests - session.excluded.usage.requests,
                usage: crate::usage::TokenUsage {
                    input: session.totals.usage.input - session.excluded.usage.usage.input,
                    output: session.totals.usage.output - session.excluded.usage.usage.output,
                    cache_write_5m: session.totals.usage.cache_write_5m
                        - session.excluded.usage.usage.cache_write_5m,
                    cache_write_1h: session.totals.usage.cache_write_1h
                        - session.excluded.usage.usage.cache_write_1h,
                    cache_read: session.totals.usage.cache_read
                        - session.excluded.usage.usage.cache_read,
                },
                total: session.totals.total - session.excluded.usage.total,
            },
            "roadmap total == session total - excluded"
        );
        assert_eq!(report.totals.usage.input, 1 + 2 + 40);
        assert_roadmap_identity(&report);
        assert_session_conservation(&report);
    }

    #[test]
    fn an_open_run_is_incomplete_and_clipped_by_a_later_run_of_any_target() {
        let main = [
            line("m1", Some(&at(1, 0)), "opus", 1, None),
            line("m-task", Some(&at(6, 0)), "opus", 2, Some("tu-t")),
            line("m-late", Some(&at(30, 0)), "opus", 4, None),
        ]
        .join("\n");
        let src = MemoryTranscriptSource::new().with_file("-p/s1.jsonl", main);
        let src = agent(src, "s1", "t", "tu-t", 16);
        let runs = [
            run("open", roadmap("demo"), Some("s1"), t(0, 0), None, vec![]),
            run(
                "task-run",
                RunTarget::Task("fix".to_owned()),
                Some("s1"),
                t(5, 0),
                Some(t(10, 0)),
                vec![],
            ),
        ];
        let report = cost_report("demo", &runs, &src);
        assert_eq!(report.runs.len(), 1);
        let open = &report.runs[0];
        assert!(!open.complete);
        assert_eq!(open.status_label, "open (incomplete)");
        assert_eq!(open.window_end, Some(t(5, 0)));
        assert_eq!(open.wall_clock_ms, None);
        assert_eq!(report.counts.incomplete, 1);
        // Only m1 is the open run's; the task run owns m-task and its agent,
        // and m-late follows the task run's end.
        assert_eq!(open.totals.usage.input, 1);
        assert_eq!(report.excluded.sources, 1);
        assert_eq!(report.excluded.main_requests, 2);
        assert_session_conservation(&report);

        // With no later run, an open run's window is unbounded.
        let alone = cost_report("demo", &runs[..1], &src);
        assert_eq!(alone.runs[0].window_end, None);
        assert_eq!(alone.totals.usage.input, 1 + 2 + 16 + 4);
        assert_eq!(alone.excluded, ExcludedCost::default());
    }

    #[test]
    fn a_run_of_another_roadmap_owns_its_window() {
        let main = [
            line("m1", Some(&at(1, 0)), "opus", 1, None),
            line("m2", Some(&at(6, 0)), "opus", 2, None),
        ]
        .join("\n");
        let src = MemoryTranscriptSource::new().with_file("-p/s1.jsonl", main);
        let runs = [
            run(
                "mine",
                roadmap("demo"),
                Some("s1"),
                t(0, 0),
                Some(t(5, 0)),
                vec![],
            ),
            run(
                "theirs",
                roadmap("other"),
                Some("s1"),
                t(5, 0),
                Some(t(9, 0)),
                vec![],
            ),
        ];
        let report = cost_report("demo", &runs, &src);
        assert_eq!(report.runs.len(), 1);
        assert_eq!(report.totals.usage.input, 1);
        assert_eq!(report.excluded.main_requests, 1);
    }

    #[test]
    fn a_missing_session_is_reported_while_the_others_still_join() {
        let src = MemoryTranscriptSource::new()
            .with_file("-p/s1.jsonl", line("m1", Some(&at(1, 0)), "opus", 1, None));
        let runs = [
            run(
                "joined",
                roadmap("demo"),
                Some("s1"),
                t(0, 0),
                Some(t(5, 0)),
                vec![],
            ),
            run(
                "gone",
                roadmap("demo"),
                Some("pruned"),
                t(6, 0),
                Some(t(9, 0)),
                vec![unit("phase-1-a", 1, t(6, 30), Some(t(7, 0)))],
            ),
            run(
                "escape",
                roadmap("demo"),
                Some("../x"),
                t(10, 0),
                None,
                vec![],
            ),
            run(
                "none",
                roadmap("demo"),
                None,
                t(11, 0),
                Some(t(12, 0)),
                vec![],
            ),
        ];
        let report = cost_report("demo", &runs, &src);
        let states: Vec<(&str, &RunJoin)> = report
            .runs
            .iter()
            .map(|r| (r.id.as_str(), &r.join))
            .collect();
        assert_eq!(states[0], ("joined", &RunJoin::Joined));
        match states[1].1 {
            RunJoin::Missing { reason } => assert!(!reason.is_empty()),
            other => panic!("expected missing, got {other:?}"),
        }
        assert!(matches!(states[2].1, RunJoin::Missing { .. }));
        assert_eq!(states[3], ("none", &RunJoin::Unjoinable));
        assert_eq!(
            (
                report.counts.runs,
                report.counts.joined,
                report.counts.missing,
                report.counts.unjoinable
            ),
            (4, 1, 2, 1)
        );
        assert_eq!(report.totals.usage.input, 1);
        // A missing run's units contribute no attempts.
        assert!(report.phases.is_empty());
        assert!(
            report.runs[1..]
                .iter()
                .all(|r| r.totals == UsageSummary::default())
        );
    }

    #[test]
    fn untimestamped_main_requests_are_session_unattributed() {
        let main = [
            line("m1", Some(&at(1, 0)), "opus", 1, None),
            line("m-no-ts", None, "opus", 2, None),
        ]
        .join("\n");
        let src = MemoryTranscriptSource::new().with_file("-p/s1.jsonl", main);
        let r = run(
            "r1",
            roadmap("demo"),
            Some("s1"),
            t(0, 0),
            Some(t(5, 0)),
            vec![],
        );
        let report = cost_report("demo", &[r], &src);
        assert_eq!(report.session_unattributed.len(), 1);
        let item = &report.session_unattributed[0];
        assert_eq!(item.kind, SourceKind::Main);
        assert_eq!(item.cause, UnattributedCause::Untimestamped);
        assert_eq!(item.usage.usage.input, 2);
        assert_eq!(report.totals.usage.input, 3);
        assert_roadmap_identity(&report);
        assert_session_conservation(&report);
    }

    #[test]
    fn no_runs_for_the_roadmap_is_an_empty_report() {
        let src = MemoryTranscriptSource::new();
        let runs = [run(
            "x",
            roadmap("other"),
            Some("s1"),
            t(0, 0),
            None,
            vec![],
        )];
        let report = cost_report("demo", &runs, &src);
        assert_eq!(report.counts, RunCounts::default());
        assert!(report.runs.is_empty() && report.sessions.is_empty());
        assert_eq!(report.totals, UsageSummary::default());
    }
}
