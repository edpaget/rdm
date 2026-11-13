//! The full rdm lane arm's host side: the prompt's rdm placeholders, the
//! plan-repo copy out of SCBench's output, and each checkpoint's lane
//! classification.
//!
//! The arm drives one roadmap ([`ROADMAP`]) per problem, one phase per
//! checkpoint. In the container the wrapper (`bench/scbench/agent/claude-rdm.sh`)
//! tars the plan repo after every agent invocation into
//! `$HOME/.claude/<PLAN_EXPORT_SUBDIR>/<n>.tar`, under SCBench's Claude trace
//! directory, so SCBench copies each new tar into the checkpoint's
//! `agent/workspace/` ([`AGENT_WORKSPACE`]). After the arm's run, `collect`
//! extracts each checkpoint's latest tar to
//! `<out>/<arm>/plan-repos/<problem>/<checkpoint>/`, reads its phases through
//! rdm-core ([`read_phases`]) and [`classify`]s the checkpoint.

use std::path::{Path, PathBuf};

use rdm_core::model::PhaseStatus;
use serde::{Deserialize, Serialize};

use super::exec::Ran;
use super::manifest::Rdm;
use super::plan::CommandSpec;
use super::rdm_image::PLAN_EXPORT_SUBDIR;

/// The roadmap every problem's plan repo holds (one plan repo per problem).
pub const ROADMAP: &str = "solution";
/// SCBench's per-checkpoint config file (`CHECKPOINT_CONFIG_NAME`), which
/// carries the checkpoint's `order`.
pub const CHECKPOINT_FILE: &str = "checkpoint.yaml";
/// Where SCBench copies the Claude trace files inside a checkpoint's output
/// directory (`<checkpoint>/agent/workspace/projects/-workspace/...`).
pub const AGENT_WORKSPACE: &str = "agent/workspace";

/// Fills an rdm prompt template's `@RDM_ROADMAP@` ([`ROADMAP`]),
/// `@RDM_PROJECT@` (`rdm.project`) and `@RDM_VERIFY@` (`rdm.verify`),
/// leaving SCBench's Jinja untouched.
pub fn render_prompt(template: &str, rdm: &Rdm) -> String {
    template
        .replace("@RDM_ROADMAP@", ROADMAP)
        .replace("@RDM_PROJECT@", &rdm.project)
        .replace("@RDM_VERIFY@", &rdm.verify)
}

/// One phase of [`ROADMAP`], as a checkpoint's plan repo records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhaseState {
    /// The phase number.
    pub number: u32,
    /// The phase stem (`phase-<n>-<slug>`).
    pub stem: String,
    /// The phase status.
    pub status: PhaseStatus,
    /// Why the phase is blocked, when recorded.
    pub blocked_reason: Option<String>,
}

/// Whether a checkpoint's phase landed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    /// The checkpoint's phase reached `done`.
    Landed,
    /// The lane did not finish the checkpoint; see [`Reason`].
    LaneFailure,
}

/// Why a checkpoint is a lane failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Reason {
    /// No new phase appeared in the checkpoint's plan repo.
    PhaseMissing,
    /// The phase is `blocked` (dispatch-phase parked it).
    Parked,
    /// The phase is `reviewed` but not `done`: `rdm:land` escalated or did
    /// not run.
    Escalated,
    /// The phase stopped short of `reviewed` (e.g. at the step limit).
    Unfinished,
    /// The phase is `wont-fix`.
    Abandoned,
    /// The checkpoint holds no plan-repo export.
    PlanRepoMissing,
    /// The checkpoint's plan repo could not be read.
    PlanRepoUnreadable,
}

impl Reason {
    /// The reason as every lane record spells it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PhaseMissing => "phase-missing",
            Self::Parked => "parked",
            Self::Escalated => "escalated",
            Self::Unfinished => "unfinished",
            Self::Abandoned => "abandoned",
            Self::PlanRepoMissing => "plan-repo-missing",
            Self::PlanRepoUnreadable => "plan-repo-unreadable",
        }
    }
}

/// One checkpoint's classification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Classification {
    /// The checkpoint's phase stem, when one was found.
    pub phase: Option<String>,
    /// That phase's status.
    pub status: Option<String>,
    /// Whether it landed.
    pub outcome: Outcome,
    /// Why it did not, for a lane failure.
    pub reason: Option<Reason>,
    /// The failure's detail (a blocked reason, a status or an error).
    pub detail: Option<String>,
}

fn failure(reason: Reason, phase: Option<&PhaseState>, detail: Option<String>) -> Classification {
    Classification {
        phase: phase.map(|p| p.stem.clone()),
        status: phase.map(|p| p.status.to_string()),
        outcome: Outcome::LaneFailure,
        reason: Some(reason),
        detail,
    }
}

/// Classifies a checkpoint whose plan repo holds `cur`, against the previous
/// checkpoint's `prev` (empty for the first).
///
/// The checkpoint's phase is the one new in `cur` (its stem is not in
/// `prev`), the highest-numbered when several are; no new phase is
/// [`Reason::PhaseMissing`]. A `done` phase is [`Outcome::Landed`]; every
/// other status is a lane failure with its [`Reason`].
pub fn classify(prev: &[PhaseState], cur: &[PhaseState]) -> Classification {
    let new = cur
        .iter()
        .filter(|c| !prev.iter().any(|p| p.stem == c.stem))
        .max_by_key(|c| c.number);
    let Some(phase) = new else {
        return failure(Reason::PhaseMissing, None, None);
    };
    match phase.status {
        PhaseStatus::Done => Classification {
            phase: Some(phase.stem.clone()),
            status: Some(phase.status.to_string()),
            outcome: Outcome::Landed,
            reason: None,
            detail: None,
        },
        PhaseStatus::Blocked => failure(Reason::Parked, Some(phase), phase.blocked_reason.clone()),
        PhaseStatus::Reviewed => failure(
            Reason::Escalated,
            Some(phase),
            Some("reviewed but not landed: rdm:land escalated or did not run".to_owned()),
        ),
        PhaseStatus::NotStarted | PhaseStatus::InProgress | PhaseStatus::NeedsReview => failure(
            Reason::Unfinished,
            Some(phase),
            Some(phase.status.to_string()),
        ),
        PhaseStatus::WontFix => failure(Reason::Abandoned, Some(phase), None),
    }
}

/// Reads [`ROADMAP`]'s phases from the plan repo at `dir` through rdm-core
/// (as `rdm phase list` does); a plan repo without the roadmap has none.
///
/// # Errors
///
/// rdm-core's error when the plan repo cannot be read.
pub fn read_phases(dir: &Path, project: &str) -> Result<Vec<PhaseState>, String> {
    let store = rdm_store_fs::FsStore::new(dir);
    match rdm_core::ops::phase::list_phases(&store, project, ROADMAP) {
        Ok(phases) => Ok(phases
            .into_iter()
            .map(|(stem, doc)| PhaseState {
                number: doc.frontmatter.phase,
                stem,
                status: doc.frontmatter.status,
                blocked_reason: doc.frontmatter.blocked_reason,
            })
            .collect()),
        Err(rdm_core::error::Error::RoadmapNotFound(_)) => Ok(Vec::new()),
        Err(e) => Err(e.to_string()),
    }
}

/// The lane record of one rdm arm (`ArmRecord.lane`).
#[derive(Debug, Clone, Serialize)]
pub struct LaneRecord {
    /// One entry per problem in the manifest subset, in order.
    pub problems: Vec<ProblemRecord>,
}

impl LaneRecord {
    /// `(landed, total)` checkpoints across every problem.
    pub fn landed(&self) -> (usize, usize) {
        let all = self.problems.iter().flat_map(|p| &p.checkpoints);
        let total = all.clone().count();
        let landed = all
            .filter(|c| c.classification.outcome == Outcome::Landed)
            .count();
        (landed, total)
    }
}

/// One problem's checkpoints.
#[derive(Debug, Clone, Serialize)]
pub struct ProblemRecord {
    /// The problem name.
    pub problem: String,
    /// The last extracted plan repo (the problem's final plan repo).
    pub plan_repo: Option<PathBuf>,
    /// Its checkpoints in SCBench's `(order, name)` order; empty when the
    /// problem has no output directory.
    pub checkpoints: Vec<CheckpointRecord>,
}

/// One checkpoint's plan repo and classification.
#[derive(Debug, Clone, Serialize)]
pub struct CheckpointRecord {
    /// The checkpoint's directory name.
    pub checkpoint: String,
    /// Its `checkpoint.yaml` `order`.
    pub order: i64,
    /// Where its plan repo was extracted; `null` with no export.
    pub plan_repo: Option<PathBuf>,
    /// Its classification.
    #[serde(flatten)]
    pub classification: Classification,
}

#[derive(Deserialize)]
struct CheckpointYaml {
    order: i64,
}

/// The checkpoints under `<run_dir>/<problem>`, sorted by `(order, name)`.
pub(super) fn checkpoints(problem_dir: &Path) -> Result<Vec<(i64, String, PathBuf)>, String> {
    let entries = match std::fs::read_dir(problem_dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("cannot read {}: {e}", problem_dir.display())),
    };
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot read {}: {e}", problem_dir.display()))?;
        let dir = entry.path();
        let config = dir.join(CHECKPOINT_FILE);
        if !config.is_file() {
            continue;
        }
        let text = std::fs::read_to_string(&config)
            .map_err(|e| format!("cannot read {}: {e}", config.display()))?;
        let parsed: CheckpointYaml = serde_saphyr::from_str(&text)
            .map_err(|e| format!("cannot parse {}: {e}", config.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        found.push((parsed.order, name, dir));
    }
    found.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    Ok(found)
}

/// The greatest-named `*.tar` export in a checkpoint, if any.
fn latest_export(checkpoint_dir: &Path) -> Result<Option<PathBuf>, String> {
    let dir = checkpoint_dir
        .join(AGENT_WORKSPACE)
        .join(PLAN_EXPORT_SUBDIR);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read {}: {e}", dir.display())),
    };
    let mut tars = Vec::new();
    for entry in entries {
        let path = entry
            .map_err(|e| format!("cannot read {}: {e}", dir.display()))?
            .path();
        if path.extension().is_some_and(|x| x == "tar") && path.is_file() {
            tars.push(path);
        }
    }
    tars.sort();
    Ok(tars.pop())
}

/// Copies and classifies every checkpoint of `problems` for arm `arm`:
/// `run_dir` is SCBench's output directory for the arm, `plan_repos_dir`
/// receives `<problem>/<checkpoint>/`, `project` is the plan repo's project
/// and `spawn` runs (and records) each `tar -xf` extraction.
///
/// # Errors
///
/// When an output directory or `checkpoint.yaml` cannot be read, a
/// destination cannot be created, or an extraction fails
/// (`arm <name>: extracting <tar> exited <n>`).
pub(super) fn collect(
    arm: &str,
    problems: &[String],
    run_dir: &Path,
    plan_repos_dir: &Path,
    project: &str,
    spawn: &mut dyn FnMut(&CommandSpec) -> Result<Ran, String>,
) -> Result<LaneRecord, String> {
    let mut records = Vec::new();
    for problem in problems {
        let mut prev: Vec<PhaseState> = Vec::new();
        let mut plan_repo = None;
        let mut out = Vec::new();
        for (order, name, dir) in checkpoints(&run_dir.join(problem))? {
            let Some(tar) = latest_export(&dir)? else {
                out.push(CheckpointRecord {
                    checkpoint: name,
                    order,
                    plan_repo: None,
                    classification: failure(Reason::PlanRepoMissing, None, None),
                });
                continue;
            };
            let dest = plan_repos_dir.join(problem).join(&name);
            std::fs::create_dir_all(&dest)
                .map_err(|e| format!("cannot create {}: {e}", dest.display()))?;
            let extract = CommandSpec {
                program: "tar".to_owned(),
                args: vec![
                    "-xf".to_owned(),
                    tar.display().to_string(),
                    "-C".to_owned(),
                    dest.display().to_string(),
                ],
                cwd: None,
                env: std::collections::BTreeMap::new(),
            };
            let ran = spawn(&extract)?;
            if !ran.ok() {
                return Err(format!(
                    "arm {arm}: extracting {} {}",
                    tar.display(),
                    ran.describe()
                ));
            }
            let classification = match read_phases(&dest, project) {
                Ok(cur) => {
                    let c = classify(&prev, &cur);
                    prev = cur;
                    c
                }
                Err(e) => failure(Reason::PlanRepoUnreadable, None, Some(e)),
            };
            plan_repo = Some(dest.clone());
            out.push(CheckpointRecord {
                checkpoint: name,
                order,
                plan_repo: Some(dest),
                classification,
            });
        }
        records.push(ProblemRecord {
            problem: problem.clone(),
            plan_repo,
            checkpoints: out,
        });
    }
    Ok(LaneRecord { problems: records })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phase(number: u32, status: PhaseStatus, reason: Option<&str>) -> PhaseState {
        PhaseState {
            number,
            stem: format!("phase-{number}-checkpoint-{number}"),
            status,
            blocked_reason: reason.map(str::to_owned),
        }
    }

    fn reason_of(prev: &[PhaseState], cur: &[PhaseState]) -> (Outcome, Option<Reason>) {
        let c = classify(prev, cur);
        (c.outcome, c.reason)
    }

    #[test]
    fn render_prompt_fills_every_placeholder() {
        let rdm = Rdm {
            plan_root: "/rdm-plan".to_owned(),
            project: "bench".to_owned(),
            verify: "pytest -q && ruff check".to_owned(),
        };
        assert_eq!(
            render_prompt(
                "rdm:dispatch-phase @RDM_ROADMAP@ N --project @RDM_PROJECT@ \
                 run `@RDM_VERIFY@` {{ spec }}",
                &rdm
            ),
            "rdm:dispatch-phase solution N --project bench \
             run `pytest -q && ruff check` {{ spec }}"
        );
    }

    #[test]
    fn done_phase_is_landed() {
        let c = classify(&[], &[phase(1, PhaseStatus::Done, None)]);
        assert_eq!(c.outcome, Outcome::Landed);
        assert_eq!(c.reason, None);
        assert_eq!(c.phase.as_deref(), Some("phase-1-checkpoint-1"));
        assert_eq!(c.status.as_deref(), Some("done"));
    }

    #[test]
    fn blocked_phase_is_parked_with_its_reason() {
        let prev = [phase(1, PhaseStatus::Done, None)];
        let cur = [
            phase(1, PhaseStatus::Done, None),
            phase(2, PhaseStatus::Blocked, Some("[plan] escalated")),
        ];
        let c = classify(&prev, &cur);
        assert_eq!(c.reason, Some(Reason::Parked));
        assert_eq!(c.detail.as_deref(), Some("[plan] escalated"));
        assert_eq!(c.phase.as_deref(), Some("phase-2-checkpoint-2"));
    }

    #[test]
    fn every_unlanded_status_has_its_reason() {
        let lf = Outcome::LaneFailure;
        for (status, reason) in [
            (PhaseStatus::Reviewed, Reason::Escalated),
            (PhaseStatus::NotStarted, Reason::Unfinished),
            (PhaseStatus::InProgress, Reason::Unfinished),
            (PhaseStatus::NeedsReview, Reason::Unfinished),
            (PhaseStatus::WontFix, Reason::Abandoned),
        ] {
            assert_eq!(
                reason_of(&[], &[phase(1, status, None)]),
                (lf, Some(reason)),
                "{status}"
            );
        }
        let c = classify(&[], &[phase(1, PhaseStatus::NeedsReview, None)]);
        assert_eq!(c.detail.as_deref(), Some("needs-review"));
    }

    #[test]
    fn no_new_phase_is_phase_missing() {
        assert_eq!(
            reason_of(&[], &[]),
            (Outcome::LaneFailure, Some(Reason::PhaseMissing))
        );
        let prev = [phase(1, PhaseStatus::Reviewed, None)];
        // A re-dispatched old phase that has since landed is still not new.
        let cur = [phase(1, PhaseStatus::Done, None)];
        let c = classify(&prev, &cur);
        assert_eq!(c.reason, Some(Reason::PhaseMissing));
        assert_eq!(c.phase, None);
    }

    #[test]
    fn highest_numbered_new_phase_is_the_checkpoints() {
        let cur = [
            phase(2, PhaseStatus::Blocked, Some("x")),
            phase(3, PhaseStatus::Done, None),
        ];
        let c = classify(&[], &cur);
        assert_eq!(c.outcome, Outcome::Landed);
        assert_eq!(c.phase.as_deref(), Some("phase-3-checkpoint-3"));
    }

    #[test]
    fn each_reason_names_itself_as_its_record_does() {
        use Reason::*;
        for r in [
            PhaseMissing,
            Parked,
            Escalated,
            Unfinished,
            Abandoned,
            PlanRepoMissing,
            PlanRepoUnreadable,
        ] {
            let spelled = serde_json::to_value(r).expect("serialize");
            assert_eq!(spelled, r.as_str(), "{r:?}");
            assert_eq!(serde_json::from_value::<Reason>(spelled).ok(), Some(r));
        }
        for o in [Outcome::Landed, Outcome::LaneFailure] {
            let spelled = serde_json::to_value(o).expect("serialize");
            assert_eq!(serde_json::from_value::<Outcome>(spelled).ok(), Some(o));
        }
    }

    #[test]
    fn read_phases_without_roadmap_is_empty() {
        let tmp = tempfile::tempdir().expect("tempdir");
        assert_eq!(read_phases(tmp.path(), "bench"), Ok(Vec::new()));
    }
}
