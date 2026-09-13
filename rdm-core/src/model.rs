/// Data model types for roadmaps, phases, and tasks.
use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

/// Error returned when a string cannot be parsed into one of the model enums.
///
/// Unlike a bare `String`, this implements [`std::error::Error`], so it
/// composes with `?` and `anyhow` without a `.map_err(|e| anyhow!(e))` wrapper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    kind: &'static str,
    value: String,
    expected: &'static str,
}

impl ParseError {
    /// Creates a parse error for `kind` (e.g. `"priority"`) from the rejected
    /// `value`, listing the `expected` accepted values.
    pub(crate) fn new(kind: &'static str, value: &str, expected: &'static str) -> Self {
        Self {
            kind,
            value: value.to_string(),
            expected,
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid {}: '{}' (expected {})",
            self.kind, self.value, self.expected
        )
    }
}

impl std::error::Error for ParseError {}

/// Status of a roadmap phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PhaseStatus {
    /// Work has not yet begun.
    NotStarted,
    /// Work is actively underway.
    InProgress,
    /// Implementation is finalized and awaiting review.
    NeedsReview,
    /// Review has passed; awaiting merge to main.
    Reviewed,
    /// Phase is complete.
    Done,
    /// Phase is blocked by an external dependency.
    Blocked,
    /// Phase was closed without completing.
    WontFix,
}

impl PhaseStatus {
    /// Returns `true` for terminal states (`Done` or `WontFix`).
    ///
    /// Terminal states stamp a `completed` date on the phase and count
    /// toward roadmap completion equally.
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(self, PhaseStatus::Done | PhaseStatus::WontFix)
    }
}

impl fmt::Display for PhaseStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PhaseStatus::NotStarted => write!(f, "not-started"),
            PhaseStatus::InProgress => write!(f, "in-progress"),
            PhaseStatus::NeedsReview => write!(f, "needs-review"),
            PhaseStatus::Reviewed => write!(f, "reviewed"),
            PhaseStatus::Done => write!(f, "done"),
            PhaseStatus::Blocked => write!(f, "blocked"),
            PhaseStatus::WontFix => write!(f, "wont-fix"),
        }
    }
}

impl FromStr for PhaseStatus {
    type Err = ParseError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "not-started" => Ok(PhaseStatus::NotStarted),
            "in-progress" => Ok(PhaseStatus::InProgress),
            "needs-review" => Ok(PhaseStatus::NeedsReview),
            "reviewed" => Ok(PhaseStatus::Reviewed),
            "done" => Ok(PhaseStatus::Done),
            "blocked" => Ok(PhaseStatus::Blocked),
            "wont-fix" => Ok(PhaseStatus::WontFix),
            other => Err(ParseError::new(
                "phase status",
                other,
                "not-started, in-progress, needs-review, reviewed, done, blocked, or wont-fix",
            )),
        }
    }
}

/// Status of a standalone task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskStatus {
    /// Task is open and not yet started.
    Open,
    /// Task is actively being worked on.
    InProgress,
    /// Implementation is finalized and awaiting review.
    NeedsReview,
    /// Review has passed; awaiting merge to main.
    Reviewed,
    /// Task is complete.
    Done,
    /// Task is blocked by an external dependency or an undecided question.
    Blocked,
    /// Task was closed without completing.
    WontFix,
}

impl TaskStatus {
    /// Returns `true` for terminal states (`Done` or `WontFix`).
    ///
    /// Terminal states stamp a `completed` date on the task. Mirrors
    /// [`PhaseStatus::is_terminal`].
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(self, TaskStatus::Done | TaskStatus::WontFix)
    }
}

impl fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TaskStatus::Open => write!(f, "open"),
            TaskStatus::InProgress => write!(f, "in-progress"),
            TaskStatus::NeedsReview => write!(f, "needs-review"),
            TaskStatus::Reviewed => write!(f, "reviewed"),
            TaskStatus::Done => write!(f, "done"),
            TaskStatus::Blocked => write!(f, "blocked"),
            TaskStatus::WontFix => write!(f, "wont-fix"),
        }
    }
}

impl FromStr for TaskStatus {
    type Err = ParseError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "open" => Ok(TaskStatus::Open),
            "in-progress" => Ok(TaskStatus::InProgress),
            "needs-review" => Ok(TaskStatus::NeedsReview),
            "reviewed" => Ok(TaskStatus::Reviewed),
            "done" => Ok(TaskStatus::Done),
            "blocked" => Ok(TaskStatus::Blocked),
            "wont-fix" => Ok(TaskStatus::WontFix),
            other => Err(ParseError::new(
                "task status",
                other,
                "open, in-progress, needs-review, reviewed, done, blocked, or wont-fix",
            )),
        }
    }
}

/// Priority level for a task or roadmap.
///
/// Variants are ordered from lowest to highest: `Low < Medium < High < Critical`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Priority {
    /// Low priority.
    Low,
    /// Medium priority.
    Medium,
    /// High priority.
    High,
    /// Critical priority.
    Critical,
}

/// Estimated difficulty of a roadmap phase.
///
/// Variants are ordered from lowest to highest:
/// `Trivial < Easy < Moderate < Hard`. Phase 3's estimator maps a difficulty
/// to a [`ModelTier`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Difficulty {
    /// Trivial work (e.g. a one-line change).
    Trivial,
    /// Easy work.
    Easy,
    /// Moderate work.
    Moderate,
    /// Hard work.
    Hard,
}

impl fmt::Display for Difficulty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Difficulty::Trivial => write!(f, "trivial"),
            Difficulty::Easy => write!(f, "easy"),
            Difficulty::Moderate => write!(f, "moderate"),
            Difficulty::Hard => write!(f, "hard"),
        }
    }
}

impl FromStr for Difficulty {
    type Err = ParseError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "trivial" => Ok(Difficulty::Trivial),
            "easy" => Ok(Difficulty::Easy),
            "moderate" => Ok(Difficulty::Moderate),
            "hard" => Ok(Difficulty::Hard),
            other => Err(ParseError::new(
                "difficulty",
                other,
                "trivial, easy, moderate, or hard",
            )),
        }
    }
}

impl Difficulty {
    /// Maps this difficulty to the model tier that should run the phase.
    ///
    /// This is the single source of truth for the difficulty→tier policy:
    /// `Trivial`/`Easy` → [`ModelTier::Small`], `Moderate` →
    /// [`ModelTier::Medium`], `Hard` → [`ModelTier::Large`]. It backs the
    /// auto-derive in
    /// [`set_phase_estimate`](crate::ops::phase::set_phase_estimate), which
    /// fills the model tier when a difficulty is set without an explicit model.
    #[must_use]
    pub fn model_tier(self) -> ModelTier {
        match self {
            Difficulty::Trivial | Difficulty::Easy => ModelTier::Small,
            Difficulty::Moderate => ModelTier::Medium,
            Difficulty::Hard => ModelTier::Large,
        }
    }
}

/// Model tier that should run a roadmap phase.
///
/// Variants are ordered from smallest to largest: `Small < Medium < Large`.
/// A concrete tier→model-id mapping, if ever needed, is left to later config.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModelTier {
    /// Small (cheapest, least capable) tier.
    Small,
    /// Medium tier.
    Medium,
    /// Large (most capable) tier.
    Large,
}

impl fmt::Display for ModelTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModelTier::Small => write!(f, "small"),
            ModelTier::Medium => write!(f, "medium"),
            ModelTier::Large => write!(f, "large"),
        }
    }
}

impl FromStr for ModelTier {
    type Err = ParseError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "small" => Ok(ModelTier::Small),
            "medium" => Ok(ModelTier::Medium),
            "large" => Ok(ModelTier::Large),
            other => Err(ParseError::new(
                "model tier",
                other,
                "small, medium, or large",
            )),
        }
    }
}

/// Filter value for task list status filtering.
///
/// Wraps the special `"all"` keyword alongside real [`TaskStatus`] values,
/// giving clap proper validation without a raw `String`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatusFilter {
    /// Show tasks of all statuses.
    All,
    /// Show tasks matching this specific status.
    Status(TaskStatus),
}

impl fmt::Display for Priority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Priority::Low => write!(f, "low"),
            Priority::Medium => write!(f, "medium"),
            Priority::High => write!(f, "high"),
            Priority::Critical => write!(f, "critical"),
        }
    }
}

impl FromStr for Priority {
    type Err = ParseError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "low" => Ok(Priority::Low),
            "medium" => Ok(Priority::Medium),
            "high" => Ok(Priority::High),
            "critical" => Ok(Priority::Critical),
            other => Err(ParseError::new(
                "priority",
                other,
                "low, medium, high, or critical",
            )),
        }
    }
}

impl fmt::Display for TaskStatusFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TaskStatusFilter::All => write!(f, "all"),
            TaskStatusFilter::Status(s) => write!(f, "{s}"),
        }
    }
}

impl FromStr for TaskStatusFilter {
    type Err = ParseError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "all" => Ok(TaskStatusFilter::All),
            other => other.parse::<TaskStatus>().map(TaskStatusFilter::Status),
        }
    }
}

/// Sort order for roadmap listings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoadmapSort {
    /// Sort alphabetically by slug (default).
    Alphabetical,
    /// Sort by priority descending (Critical → High → Medium → Low → None).
    Priority,
}

impl fmt::Display for RoadmapSort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RoadmapSort::Alphabetical => write!(f, "alphabetical"),
            RoadmapSort::Priority => write!(f, "priority"),
        }
    }
}

impl FromStr for RoadmapSort {
    type Err = ParseError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "alphabetical" => Ok(RoadmapSort::Alphabetical),
            "priority" => Ok(RoadmapSort::Priority),
            other => Err(ParseError::new("sort", other, "alphabetical or priority")),
        }
    }
}

/// Frontmatter for a project directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    /// Project slug identifier (used in directory names and references).
    pub name: String,
    /// Human-readable title.
    pub title: String,
    /// A code repository this project's `rdm:src/` links resolve against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
}

/// A code repository a project's `rdm:src/` links resolve against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    /// Repository location (e.g. a clone URL or filesystem path).
    pub repo: String,
    /// Branch `rdm:src/` links resolve against when no `@<rev>` is given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_branch: Option<String>,
}

/// An operator's recorded bypass of the `reviewed` transition gate.
///
/// Stamped on a phase or task by `--override-gate "<reason>"`, which waives
/// the gate's *record* preconditions — an approved implementation plan and an
/// approving `change/` review — but never its worktree-cleanliness
/// precondition. It exists so a bypass is an audited act rather than an
/// invisible one: the reason, who did it, and when are all part of the item.
///
/// It is cleared whenever the item leaves `reviewed` (see
/// [`GateOverrideUpdate`](crate::ops::update::GateOverrideUpdate)), so a stale
/// override can never authorize a later `reviewed` write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateOverride {
    /// Why the gate was bypassed, verbatim as the operator typed it.
    pub reason: String,
    /// Who bypassed it, resolved the same way a review's author is.
    pub actor: String,
    /// The date the bypass was recorded.
    pub at: NaiveDate,
}

/// Frontmatter for a roadmap phase file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Phase {
    /// Phase number (1-based ordering).
    pub phase: u32,
    /// Human-readable title.
    pub title: String,
    /// Current status.
    pub status: PhaseStatus,
    /// Optional tags for categorization.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    /// Date the phase was completed, if applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed: Option<NaiveDate>,
    /// Git commit SHA associated with phase completion, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// Source-repo HEAD SHA stamped when the item entered `needs-review`.
    /// Used to scope review prompts to the branch/worktree that produced it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_sha: Option<String>,
    /// Branch name of the checkout that produced the review, stamped when the
    /// item entered `needs-review`. Lets `review pending` scope by identity (the
    /// firing checkout's branch) rather than by SHA reachability alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_branch: Option<String>,
    /// Estimated difficulty of the phase, if assessed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub difficulty: Option<Difficulty>,
    /// Model tier that should run the phase, if assigned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelTier>,
    /// Reason the phase was parked as `blocked` (an escalation note), if any.
    ///
    /// Recorded when a phase is set to [`PhaseStatus::Blocked`] so the blocker —
    /// an ambiguous acceptance criterion, an architectural decision with no clear
    /// default, an exhausted retry budget, or a hard external dependency — is
    /// queryable and survives a later resume. Preserved across status changes
    /// until explicitly cleared, so resuming a phase never loses why it stalled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    /// An operator's recorded bypass of the `reviewed` transition gate, if
    /// one authorized this phase's current `reviewed` status.
    ///
    /// Skipped when absent, so every phase file written before the gate
    /// existed still loads and a never-overridden phase serializes exactly as
    /// it always did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_override: Option<GateOverride>,
}

impl Phase {
    /// Build the file-stem for this phase (e.g. `phase-1-design`).
    pub fn stem(&self, slug: &str) -> String {
        phase_stem(self.phase, slug)
    }
}

/// Build a phase file-stem from a number and slug (e.g. `phase-1-design`).
pub fn phase_stem(number: u32, slug: &str) -> String {
    format!("phase-{number}-{slug}")
}

/// Frontmatter for a standalone task file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    /// Project this task belongs to.
    pub project: String,
    /// Human-readable title.
    pub title: String,
    /// Current status.
    pub status: TaskStatus,
    /// Priority level.
    pub priority: Priority,
    /// Date the task was created.
    pub created: NaiveDate,
    /// Optional tags for categorization.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    /// Date the task was completed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed: Option<NaiveDate>,
    /// Git commit SHA that completed this task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// Source-repo HEAD SHA stamped when the item entered `needs-review`.
    /// Used to scope review prompts to the branch/worktree that produced it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_sha: Option<String>,
    /// Branch name of the checkout that produced the review, stamped when the
    /// item entered `needs-review`. Lets `review pending` scope by identity (the
    /// firing checkout's branch) rather than by SHA reachability alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_branch: Option<String>,
    /// Reason the task was closed (a retire/supersede note), if any.
    ///
    /// Recorded when a task is retired — for example marked
    /// [`TaskStatus::WontFix`] via `task update --reason`, or closed as a merge
    /// source with a `superseded by task/<survivor>` pointer. Mirrors
    /// [`Phase::blocked_reason`]: it is preserved across status changes until
    /// explicitly cleared, so reopening a task never loses why it was retired.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close_reason: Option<String>,
    /// An operator's recorded bypass of the `reviewed` transition gate, if
    /// one authorized this task's current `reviewed` status. Mirrors
    /// [`Phase::gate_override`], including the skip-when-absent serialization.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_override: Option<GateOverride>,
}

/// Lifecycle status of an implementation [`Plan`] document.
///
/// A plan starts `draft`. It is never set directly by a status flag: it is
/// **derived from reviews** — submitting a review on `plan/<slug>` with
/// verdict `approve` marks it `approved`, `request-changes` marks it
/// `changes-requested`, and creating a later plan whose `supersedes` names
/// it marks it `superseded`. `superseded` is terminal and is never
/// downgraded by a later verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PlanStatus {
    /// Written but not yet reviewed.
    Draft,
    /// A review on this plan was submitted with verdict `approve`.
    Approved,
    /// A review on this plan was submitted with verdict `request-changes`.
    ChangesRequested,
    /// A later plan named this one in its `supersedes` field. Terminal.
    Superseded,
}

impl PlanStatus {
    /// Returns `true` for terminal states (`Superseded` only).
    ///
    /// A superseded plan is closed for good: a later review verdict never
    /// downgrades it back to `approved`/`changes-requested`. Mirrors
    /// [`TaskStatus::is_terminal`] and [`PhaseStatus::is_terminal`].
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(self, PlanStatus::Superseded)
    }
}

impl fmt::Display for PlanStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlanStatus::Draft => write!(f, "draft"),
            PlanStatus::Approved => write!(f, "approved"),
            PlanStatus::ChangesRequested => write!(f, "changes-requested"),
            PlanStatus::Superseded => write!(f, "superseded"),
        }
    }
}

impl FromStr for PlanStatus {
    type Err = ParseError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "draft" => Ok(PlanStatus::Draft),
            "approved" => Ok(PlanStatus::Approved),
            "changes-requested" => Ok(PlanStatus::ChangesRequested),
            "superseded" => Ok(PlanStatus::Superseded),
            other => Err(ParseError::new(
                "plan status",
                other,
                "draft, approved, changes-requested, or superseded",
            )),
        }
    }
}

/// Serde shim serializing a [`ReviewTarget`] as the canonical `rdm:`-prefixed
/// URI string (`rdm:phase/auth/phase-1-design`) rather than as a tagged
/// mapping.
///
/// Deserialization routes through [`crate::link::parse`], so a plan's
/// `implements`/`supersedes` frontmatter shares exactly one grammar with
/// `Done:` lines, `rdm review --on`, and `rdm:` body links. A bare
/// `phase/auth/phase-1-design` (no `rdm:` prefix) is also accepted, for
/// hand-edited files; serialization always emits the `rdm:` form.
pub(crate) mod plan_ref {
    use super::ReviewTarget;
    use serde::{Deserialize, Deserializer, Serializer};

    /// Parses either the canonical `rdm:<kind>/…` form or a bare
    /// `<kind>/…` item reference.
    pub(crate) fn parse_ref<E: serde::de::Error>(raw: &str) -> Result<ReviewTarget, E> {
        let uri = if raw.starts_with("rdm:") {
            raw.to_string()
        } else {
            format!("rdm:{raw}")
        };
        match crate::link::parse(&uri) {
            Ok(crate::link::Link::Item(item_ref)) => Ok(item_ref),
            Ok(crate::link::Link::Code { .. }) => Err(E::custom(format!(
                "'{raw}' is a source-code reference; expected an item reference like rdm:phase/<roadmap>/<stem>, rdm:task/<slug>, or rdm:plan/<slug>"
            ))),
            Err(e) => Err(E::custom(e.to_string())),
        }
    }

    /// Serializes a reference as `rdm:<kind>/…`.
    pub(crate) fn serialize<S: Serializer>(
        value: &ReviewTarget,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format!("rdm:{}", value.label()))
    }

    /// Deserializes a reference from either accepted string form.
    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<ReviewTarget, D::Error> {
        let raw = String::deserialize(deserializer)?;
        parse_ref(&raw)
    }

    /// The `Option` flavor, for the optional `supersedes` field.
    pub(crate) mod option {
        use super::super::ReviewTarget;
        use serde::{Deserialize, Deserializer, Serializer};

        /// Serializes `Some(ref)` as `rdm:<kind>/…`; `None` is skipped by
        /// the field's `skip_serializing_if`.
        pub(crate) fn serialize<S: Serializer>(
            value: &Option<ReviewTarget>,
            serializer: S,
        ) -> Result<S::Ok, S::Error> {
            match value {
                Some(v) => serializer.serialize_str(&format!("rdm:{}", v.label())),
                None => serializer.serialize_none(),
            }
        }

        /// Deserializes an optional reference from either accepted string form.
        pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Option<ReviewTarget>, D::Error> {
            let raw: Option<String> = Option::deserialize(deserializer)?;
            match raw {
                Some(raw) => super::parse_ref(&raw).map(Some),
                None => Ok(None),
            }
        }
    }
}

/// Frontmatter for an implementation-plan file.
///
/// A plan is a first-class document describing how one phase or task will
/// be implemented, so it can be reviewed with anchored comments, superseded
/// by a later attempt, and linked from the item it implements. It lives at
/// `projects/<project>/plans/<slug>.md`.
///
/// `implements` is required and names exactly one phase or task;
/// `supersedes`, when present, names the earlier plan this one replaces.
/// Both round-trip through the shared `rdm:` item-reference grammar (see
/// [`plan_ref`]). Neither is validated at parse time — a plan whose target
/// was renamed or deleted still loads, mirroring [`ReviewTarget`]'s
/// dangling-target policy. Existence is checked when a plan is *created*,
/// by the operations layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    /// Project this plan belongs to.
    pub project: String,
    /// Plan slug identifier (matches the file stem).
    pub plan: String,
    /// Human-readable title.
    pub title: String,
    /// The phase or task this plan implements. Exactly one target.
    #[serde(with = "crate::model::plan_ref")]
    pub implements: ReviewTarget,
    /// The earlier plan this one replaces, if any.
    #[serde(
        default,
        with = "crate::model::plan_ref::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub supersedes: Option<ReviewTarget>,
    /// Current status, derived from reviews rather than set directly.
    pub status: PlanStatus,
    /// Date the plan was created.
    pub created: NaiveDate,
    /// Date the plan was last modified.
    pub updated: NaiveDate,
}

/// Frontmatter for a roadmap file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Roadmap {
    /// Project this roadmap belongs to.
    pub project: String,
    /// Roadmap slug identifier.
    pub roadmap: String,
    /// Human-readable title.
    pub title: String,
    /// Ordered list of phase file stems.
    pub phases: Vec<String>,
    /// Roadmap slugs that must complete before this one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependencies: Option<Vec<String>>,
    /// Optional priority level for the roadmap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<Priority>,
    /// Optional tags for categorization.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

/// Lifecycle state of a review.
///
/// A review starts as a `draft`, becomes `submitted` when the reviewer
/// finalizes it (stamping a [`Verdict`]), and ends as `addressed` (every
/// comment resolved) or `dismissed` (closed without being acted on).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReviewState {
    /// The review is being written and has not been submitted.
    Draft,
    /// The review has been submitted with a verdict.
    Submitted,
    /// Every comment has been resolved and the review is closed.
    Addressed,
    /// The review was closed without being acted on.
    Dismissed,
}

impl ReviewState {
    /// Returns `true` for terminal states (`Addressed` or `Dismissed`).
    ///
    /// Terminal reviews accept no further state transitions. Mirrors
    /// [`PhaseStatus::is_terminal`] and [`TaskStatus::is_terminal`].
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(self, ReviewState::Addressed | ReviewState::Dismissed)
    }
}

impl fmt::Display for ReviewState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReviewState::Draft => write!(f, "draft"),
            ReviewState::Submitted => write!(f, "submitted"),
            ReviewState::Addressed => write!(f, "addressed"),
            ReviewState::Dismissed => write!(f, "dismissed"),
        }
    }
}

impl FromStr for ReviewState {
    type Err = ParseError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "draft" => Ok(ReviewState::Draft),
            "submitted" => Ok(ReviewState::Submitted),
            "addressed" => Ok(ReviewState::Addressed),
            "dismissed" => Ok(ReviewState::Dismissed),
            other => Err(ParseError::new(
                "review state",
                other,
                "draft, submitted, addressed, or dismissed",
            )),
        }
    }
}

/// Overall verdict a reviewer attaches when submitting a review.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
    /// The target is approved as-is.
    Approve,
    /// Changes are requested before the target can be accepted.
    RequestChanges,
    /// Neutral feedback with no approval or rejection.
    Comment,
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Verdict::Approve => write!(f, "approve"),
            Verdict::RequestChanges => write!(f, "request-changes"),
            Verdict::Comment => write!(f, "comment"),
        }
    }
}

impl FromStr for Verdict {
    type Err = ParseError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "approve" => Ok(Verdict::Approve),
            "request-changes" => Ok(Verdict::RequestChanges),
            "comment" => Ok(Verdict::Comment),
            other => Err(ParseError::new(
                "verdict",
                other,
                "approve, request-changes, or comment",
            )),
        }
    }
}

/// Resolution status of a single review comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReviewCommentStatus {
    /// The comment has not been acted on yet.
    Open,
    /// The comment has been addressed (see
    /// [`ReviewComment::applied_commit`]).
    Addressed,
    /// The comment was closed without a change.
    WontFix,
}

impl ReviewCommentStatus {
    /// Returns `true` for terminal states (`Addressed` or `WontFix`) — i.e.
    /// comments that no longer need attention. Mirrors
    /// [`PhaseStatus::is_terminal`] and [`TaskStatus::is_terminal`].
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            ReviewCommentStatus::Addressed | ReviewCommentStatus::WontFix
        )
    }
}

impl fmt::Display for ReviewCommentStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReviewCommentStatus::Open => write!(f, "open"),
            ReviewCommentStatus::Addressed => write!(f, "addressed"),
            ReviewCommentStatus::WontFix => write!(f, "wont-fix"),
        }
    }
}

impl FromStr for ReviewCommentStatus {
    type Err = ParseError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "open" => Ok(ReviewCommentStatus::Open),
            "addressed" => Ok(ReviewCommentStatus::Addressed),
            "wont-fix" => Ok(ReviewCommentStatus::WontFix),
            other => Err(ParseError::new(
                "review comment status",
                other,
                "open, addressed, or wont-fix",
            )),
        }
    }
}

/// The plan item a review targets.
///
/// Serialized as a tagged mapping keyed on `kind` (`roadmap` | `phase` |
/// `task` | `plan`). Target existence is deliberately **not** validated at parse
/// time — a review whose target has been renamed or deleted (a dangling
/// target) still loads, so renames never corrupt the review store.
/// Existence is checked when a review is *created*, by the operations
/// layer (a later phase).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ReviewTarget {
    /// A whole roadmap.
    Roadmap {
        /// Roadmap slug.
        roadmap: String,
    },
    /// A single phase within a roadmap.
    Phase {
        /// Roadmap slug the phase belongs to.
        roadmap: String,
        /// Phase file stem (e.g. `phase-1-design`).
        stem: String,
    },
    /// A standalone task.
    Task {
        /// Task slug.
        slug: String,
    },
    /// An implementation plan for a phase or task.
    Plan {
        /// Plan slug.
        slug: String,
    },
    /// A set of commits in the project's **source** repository — the code
    /// change itself, rather than any plan-repo document.
    ///
    /// `head` is always a full 40-character commit SHA: the CLI rev-parses
    /// whatever the operator typed (`HEAD`, a branch name, an abbreviated
    /// sha) before constructing this variant, so two reviews of the same
    /// commit are always the same target. `base` is the other end of the
    /// reviewed range — normally the merge-base with the project's default
    /// branch — and is recorded as provenance only: it is **not** part of
    /// the target's identity (see [`ReviewTarget::same_item`]) and does not
    /// appear in the `change/<head>` reference grammar.
    Change {
        /// Full 40-character commit SHA of the reviewed tip.
        head: String,
        /// The base the change is diffed against, when known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        base: Option<String>,
    },
}

impl ReviewTarget {
    /// Renders the target in the CLI's `<kind>/<id>` reference syntax:
    /// `roadmap/<slug>`, `phase/<roadmap-slug>/<stem>`, `task/<slug>`, or
    /// `plan/<slug>`.
    ///
    /// This is the same syntax `rdm review start --on` and `rdm review list
    /// --on` accept, so labels round-trip as command arguments.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            ReviewTarget::Roadmap { roadmap } => format!("roadmap/{roadmap}"),
            ReviewTarget::Phase { roadmap, stem } => format!("phase/{roadmap}/{stem}"),
            ReviewTarget::Task { slug } => format!("task/{slug}"),
            ReviewTarget::Plan { slug } => format!("plan/{slug}"),
            // Deliberately head-only: `base` is provenance, not identity.
            ReviewTarget::Change { head, .. } => format!("change/{head}"),
        }
    }

    /// Whether `self` and `other` name the same reviewable item.
    ///
    /// Structural equality for every kind except
    /// [`ReviewTarget::Change`], where only `head` is compared: a target
    /// parsed from the `change/<head>` reference grammar carries no `base`,
    /// while the stored target normally does, so derived [`PartialEq`]
    /// would make `rdm review list --on change/<sha>` match nothing.
    ///
    /// # Examples
    ///
    /// ```
    /// use rdm_core::model::ReviewTarget;
    ///
    /// let stored = ReviewTarget::Change {
    ///     head: "a".repeat(40),
    ///     base: Some("b".repeat(40)),
    /// };
    /// let typed: ReviewTarget = format!("change/{}", "a".repeat(40)).parse().unwrap();
    /// assert!(stored.same_item(&typed));
    /// assert_ne!(stored, typed);
    /// ```
    #[must_use]
    pub fn same_item(&self, other: &ReviewTarget) -> bool {
        match (self, other) {
            (ReviewTarget::Change { head: a, .. }, ReviewTarget::Change { head: b, .. }) => a == b,
            (a, b) => a == b,
        }
    }

    /// Returns the target's kind discriminant, for kind-only filtering (see
    /// [`ReviewFilter`](crate::ops::reviews::ReviewFilter)).
    #[must_use]
    pub fn kind(&self) -> ReviewTargetKind {
        match self {
            ReviewTarget::Roadmap { .. } => ReviewTargetKind::Roadmap,
            ReviewTarget::Phase { .. } => ReviewTargetKind::Phase,
            ReviewTarget::Task { .. } => ReviewTargetKind::Task,
            ReviewTarget::Plan { .. } => ReviewTargetKind::Plan,
            ReviewTarget::Change { .. } => ReviewTargetKind::Change,
        }
    }
}

impl FromStr for ReviewTarget {
    type Err = ParseError;

    /// Parses the shared item-reference syntax: `roadmap/<slug>`,
    /// `phase/<roadmap-slug>/<stem-or-number>`, `task/<slug>`, or
    /// `plan/<slug>`.
    ///
    /// This is purely syntactic — it does not touch the store, so a
    /// numeric phase identifier (`phase/x/2`) is kept verbatim in `stem`,
    /// unresolved against any real phase. Resolving that against store
    /// content (via
    /// [`resolve_phase_stem`](crate::ops::phase::resolve_phase_stem)) is
    /// the caller's job.
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let invalid = || {
            ParseError::new(
                "item reference",
                s,
                "roadmap/<slug>, phase/<roadmap-slug>/<stem-or-number>, task/<slug>, plan/<slug>, or change/<sha>",
            )
        };
        let (kind, rest) = s.split_once('/').ok_or_else(invalid)?;
        match kind {
            "roadmap" if !rest.is_empty() && !rest.contains('/') => Ok(ReviewTarget::Roadmap {
                roadmap: rest.to_string(),
            }),
            "task" if !rest.is_empty() && !rest.contains('/') => Ok(ReviewTarget::Task {
                slug: rest.to_string(),
            }),
            "plan" if !rest.is_empty() && !rest.contains('/') => Ok(ReviewTarget::Plan {
                slug: rest.to_string(),
            }),
            // Purely syntactic, like a numeric phase stem: `<rev>` is kept
            // verbatim and the resulting target carries no `base`. The CLI
            // rev-parses the revision against the source repo and fills the
            // merge-base in before anything is stored.
            "change" if !rest.is_empty() && !rest.contains('/') => Ok(ReviewTarget::Change {
                head: rest.to_string(),
                base: None,
            }),
            "phase" => {
                let (roadmap, stem) = rest.split_once('/').ok_or_else(invalid)?;
                if roadmap.is_empty() || stem.is_empty() || stem.contains('/') {
                    return Err(invalid());
                }
                Ok(ReviewTarget::Phase {
                    roadmap: roadmap.to_string(),
                    stem: stem.to_string(),
                })
            }
            _ => Err(invalid()),
        }
    }
}

impl fmt::Display for ReviewTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.label())
    }
}

/// The kind discriminant of a [`ReviewTarget`], without its identifying
/// fields — what a caller filters on when it cares about "reviews of tasks"
/// rather than one specific task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewTargetKind {
    /// A whole roadmap.
    Roadmap,
    /// A single phase within a roadmap.
    Phase,
    /// A standalone task.
    Task,
    /// An implementation plan.
    Plan,
    /// A set of commits in the project's source repository.
    Change,
}

impl fmt::Display for ReviewTargetKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReviewTargetKind::Roadmap => write!(f, "roadmap"),
            ReviewTargetKind::Phase => write!(f, "phase"),
            ReviewTargetKind::Task => write!(f, "task"),
            ReviewTargetKind::Plan => write!(f, "plan"),
            ReviewTargetKind::Change => write!(f, "change"),
        }
    }
}

impl FromStr for ReviewTargetKind {
    type Err = ParseError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "roadmap" => Ok(ReviewTargetKind::Roadmap),
            "phase" => Ok(ReviewTargetKind::Phase),
            "task" => Ok(ReviewTargetKind::Task),
            "plan" => Ok(ReviewTargetKind::Plan),
            "change" => Ok(ReviewTargetKind::Change),
            other => Err(ParseError::new(
                "review target kind",
                other,
                "roadmap, phase, task, plan, or change",
            )),
        }
    }
}

/// Kind of document a [`CommentDoc`] points at.
///
/// Only `phase` is meaningful today: a roadmap review may scope a comment
/// to one of the roadmap's phases. Modeled as an enum so future kinds are
/// non-breaking additions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CommentDocKind {
    /// A roadmap phase.
    Phase,
}

/// Document scope for a comment within a multi-document review.
///
/// A roadmap review may point an individual comment at one of the
/// roadmap's phases rather than at the roadmap body itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentDoc {
    /// The kind of document the comment points at.
    pub kind: CommentDocKind,
    /// File stem of the document (e.g. `phase-1-design`).
    pub stem: String,
}

/// Location a review comment is anchored to within the target's body.
///
/// Serialized as a tagged union keyed on `anchor_type`: `text-quote` for a
/// plan-repo document body, `file-quote` for a file in the project's source
/// repository (a `change/<sha>` review). Any other `anchor_type` written by a newer rdm
/// round-trips losslessly as [`Anchor::Unknown`], so an older binary never
/// corrupts reviews it does not fully understand. Future variants
/// (`line-range`, `heading-path`, `ast-node`) are non-breaking structural
/// additions.
#[derive(Debug, Clone, PartialEq)]
pub enum Anchor {
    /// A quoted span of the target's body, disambiguated by surrounding
    /// context (the [W3C text-quote selector](https://www.w3.org/TR/annotation-model/#text-quote-selector)
    /// approach).
    TextQuote {
        /// The exact quoted text the comment refers to.
        quote: String,
        /// Up to ~32 characters immediately before the quote, to
        /// disambiguate duplicate occurrences.
        prefix: String,
        /// Up to ~32 characters immediately after the quote.
        suffix: String,
    },
    /// A quoted span of a **source-repository** file, for a review whose
    /// target is a [`ReviewTarget::Change`].
    ///
    /// Serialized as `anchor_type: file-quote`.
    ///
    /// Deliberately carries **no** `prefix`/`suffix` context, unlike
    /// [`Anchor::TextQuote`]: a review file lives in the plan repo, and the
    /// invariant that it embeds no source-repository content beyond the
    /// quote the reviewer chose is only literally true if nothing else from
    /// the file is copied in. Duplicate occurrences are therefore
    /// disambiguated with `occurrence` (1-based, exactly the vocabulary
    /// [`crate::anchor::derive_text_quote`] already uses) plus the recorded
    /// `start_line`/`end_line`, rather than with surrounding text.
    ///
    /// The line range is the range the quote occupied in `path` at the
    /// review target's `head`, recorded so
    /// [`crate::change::permalink_for`] can emit an
    /// `rdm:src/<path>@<head>#L<start>-L<end>` permalink with no source
    /// checkout present.
    FileQuote {
        /// Repo-relative path of the file within the source repository.
        path: String,
        /// The exact quoted text the comment refers to.
        quote: String,
        /// 1-based occurrence of `quote` within the file at `head`.
        occurrence: u32,
        /// 1-based first line the quote spans at `head`.
        start_line: u32,
        /// 1-based last line the quote spans at `head` (inclusive).
        end_line: u32,
    },
    /// An anchor whose `anchor_type` this build does not recognize.
    ///
    /// The full original YAML mapping (including the `anchor_type` key) is
    /// preserved verbatim in `raw` and re-emitted on serialization, so no
    /// data is lost when re-writing the review file.
    Unknown {
        /// The unrecognized `anchor_type` discriminator.
        anchor_type: String,
        /// The complete original mapping, preserved for lossless
        /// round-tripping.
        raw: serde_yaml::Value,
    },
}

impl Serialize for Anchor {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Anchor::TextQuote {
                quote,
                prefix,
                suffix,
            } => {
                #[derive(Serialize)]
                struct TextQuoteFields<'a> {
                    anchor_type: &'static str,
                    quote: &'a str,
                    prefix: &'a str,
                    suffix: &'a str,
                }
                TextQuoteFields {
                    anchor_type: "text-quote",
                    quote,
                    prefix,
                    suffix,
                }
                .serialize(serializer)
            }
            Anchor::FileQuote {
                path,
                quote,
                occurrence,
                start_line,
                end_line,
            } => {
                #[derive(Serialize)]
                struct FileQuoteFields<'a> {
                    anchor_type: &'static str,
                    path: &'a str,
                    quote: &'a str,
                    occurrence: u32,
                    start_line: u32,
                    end_line: u32,
                }
                FileQuoteFields {
                    anchor_type: "file-quote",
                    path,
                    quote,
                    occurrence: *occurrence,
                    start_line: *start_line,
                    end_line: *end_line,
                }
                .serialize(serializer)
            }
            // Re-emit the original mapping verbatim (it already carries its
            // own `anchor_type` key).
            Anchor::Unknown { raw, .. } => raw.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for Anchor {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error as _;

        // Buffer the whole mapping first so an unrecognized variant can be
        // preserved verbatim.
        let value = serde_yaml::Value::deserialize(deserializer)?;
        let anchor_type = match value.get("anchor_type") {
            None => return Err(D::Error::missing_field("anchor_type")),
            Some(v) => v
                .as_str()
                .ok_or_else(|| D::Error::custom("anchor_type must be a string"))?
                .to_string(),
        };
        match anchor_type.as_str() {
            "text-quote" => {
                #[derive(Deserialize)]
                struct TextQuoteFields {
                    quote: String,
                    prefix: String,
                    suffix: String,
                }
                let fields: TextQuoteFields =
                    serde_yaml::from_value(value).map_err(D::Error::custom)?;
                Ok(Anchor::TextQuote {
                    quote: fields.quote,
                    prefix: fields.prefix,
                    suffix: fields.suffix,
                })
            }
            "file-quote" => {
                #[derive(Deserialize)]
                struct FileQuoteFields {
                    path: String,
                    quote: String,
                    occurrence: u32,
                    start_line: u32,
                    end_line: u32,
                }
                let fields: FileQuoteFields =
                    serde_yaml::from_value(value).map_err(D::Error::custom)?;
                Ok(Anchor::FileQuote {
                    path: fields.path,
                    quote: fields.quote,
                    occurrence: fields.occurrence,
                    start_line: fields.start_line,
                    end_line: fields.end_line,
                })
            }
            _ => Ok(Anchor::Unknown {
                anchor_type,
                raw: value,
            }),
        }
    }
}

/// A single inline comment within a [`Review`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewComment {
    /// Ordinal identifier, unique within the review.
    pub id: u32,
    /// Optional document scope: a roadmap review may point this comment at
    /// one of the roadmap's phases. `None` targets the review's own target
    /// document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<CommentDoc>,
    /// Resolution status of the comment.
    pub status: ReviewCommentStatus,
    /// Commit SHA recorded when `status` moved to
    /// [`ReviewCommentStatus::Addressed`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied_commit: Option<String>,
    /// Where in the target body the comment points. `None` means a
    /// whole-document comment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<Anchor>,
    /// The comment text (Markdown).
    pub body: String,
    /// Agent note set when the comment is addressed or needs clarification.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<String>,
}

/// Frontmatter for a review file (`reviews/<id>.md`).
///
/// The file body below the frontmatter is the overall review summary; all
/// metadata — including the full comment list — lives in the frontmatter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Review {
    /// Timestamp-based identifier, unique within the project (e.g.
    /// `2026-07-01-1430-a1b2`); also the file stem.
    pub id: String,
    /// Who authored the review — a free-form string (email, agent name,
    /// etc.).
    pub author: String,
    /// The plan item under review.
    pub target: ReviewTarget,
    /// Lifecycle state of the review.
    pub state: ReviewState,
    /// Verdict stamped on submit; absent while the review is a draft.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdict: Option<Verdict>,
    /// When the review was started.
    pub created: DateTime<Utc>,
    /// When the review was submitted; absent on drafts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submitted: Option<DateTime<Utc>>,
    /// Plan-repo HEAD when the review started — the version of the target
    /// the reviewer saw. Optional so files from older formats still load.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_commit: Option<String>,
    /// The implementation plan this review's change implements, as a
    /// `rdm:plan/<slug>` reference.
    ///
    /// Only meaningful on a [`ReviewTarget::Change`] review — it is what
    /// links the reviewed code back to the plan it was written against, and
    /// what `rdm plan show` and `rdm backlinks` traverse. Absent on every
    /// other kind (and on change reviews started before the link existed),
    /// so every pre-existing review file still loads.
    #[serde(
        default,
        with = "crate::model::plan_ref::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub implements: Option<ReviewTarget>,
    /// Source-repository branch the reviewed change was on when the review
    /// started, when the checkout had one (`None` on a detached HEAD, or on
    /// any non-`change` review).
    ///
    /// Used only to pick the "tip" drift is measured against: a later
    /// commit on this branch is what turns a resolved anchor into a drifted
    /// one. A deleted or renamed branch degrades to the repo's current HEAD.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_branch: Option<String>,
    /// The inline comments attached to this review.
    pub comments: Vec<ReviewComment>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn change_target_round_trips_through_yaml() {
        let target = ReviewTarget::Change {
            head: "a".repeat(40),
            base: Some("b".repeat(40)),
        };
        let yaml = serde_yaml::to_string(&target).unwrap();
        assert!(yaml.contains("kind: change"), "{yaml}");
        assert!(
            yaml.contains(&format!("head: {}", "a".repeat(40))),
            "{yaml}"
        );
        let parsed: ReviewTarget = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, target);

        // A base-less target simply omits the key.
        let bare = ReviewTarget::Change {
            head: "c".repeat(40),
            base: None,
        };
        let yaml = serde_yaml::to_string(&bare).unwrap();
        assert!(!yaml.contains("base"), "{yaml}");
        assert_eq!(serde_yaml::from_str::<ReviewTarget>(&yaml).unwrap(), bare);
    }

    #[test]
    fn change_target_label_and_from_str_round_trip_on_the_head() {
        let target: ReviewTarget = "change/abc123".parse().unwrap();
        assert_eq!(
            target,
            ReviewTarget::Change {
                head: "abc123".to_string(),
                base: None,
            }
        );
        assert_eq!(target.label(), "change/abc123");
        assert_eq!(target.kind(), ReviewTargetKind::Change);
        assert_eq!(target.to_string().parse::<ReviewTarget>().unwrap(), target);
    }

    #[test]
    fn change_target_label_omits_the_base() {
        let target = ReviewTarget::Change {
            head: "abc123".to_string(),
            base: Some("def456".to_string()),
        };
        assert_eq!(target.label(), "change/abc123");
    }

    #[test]
    fn change_from_str_rejects_an_empty_or_nested_rev() {
        for bad in ["change/", "change/a/b"] {
            assert!(
                bad.parse::<ReviewTarget>().is_err(),
                "expected {bad:?} to be rejected"
            );
        }
    }

    #[test]
    fn same_item_compares_change_targets_on_head_only() {
        let stored = ReviewTarget::Change {
            head: "abc".to_string(),
            base: Some("def".to_string()),
        };
        let typed = ReviewTarget::Change {
            head: "abc".to_string(),
            base: None,
        };
        // Derived equality would say no — which is exactly the trap.
        assert_ne!(stored, typed);
        assert!(stored.same_item(&typed));
        assert!(typed.same_item(&stored));

        let other = ReviewTarget::Change {
            head: "zzz".to_string(),
            base: Some("def".to_string()),
        };
        assert!(!stored.same_item(&other));
    }

    #[test]
    fn same_item_is_plain_equality_for_every_other_kind() {
        let a = ReviewTarget::Task {
            slug: "x".to_string(),
        };
        let b = ReviewTarget::Task {
            slug: "x".to_string(),
        };
        let c = ReviewTarget::Task {
            slug: "y".to_string(),
        };
        assert!(a.same_item(&b));
        assert!(!a.same_item(&c));
        assert!(!a.same_item(&ReviewTarget::Roadmap {
            roadmap: "x".to_string()
        }));
    }

    #[test]
    fn review_target_kind_change_round_trips() {
        assert_eq!(ReviewTargetKind::Change.to_string(), "change");
        assert_eq!(
            "change".parse::<ReviewTargetKind>().unwrap(),
            ReviewTargetKind::Change
        );
    }

    #[test]
    fn file_quote_anchor_round_trips_through_yaml() {
        let anchor = Anchor::FileQuote {
            path: "src/lib.rs".to_string(),
            quote: "fn main() {}".to_string(),
            occurrence: 2,
            start_line: 7,
            end_line: 9,
        };
        let yaml = serde_yaml::to_string(&anchor).unwrap();
        assert!(yaml.contains("anchor_type: file-quote"), "{yaml}");
        // AC6: no surrounding context is ever persisted.
        assert!(!yaml.contains("prefix"), "{yaml}");
        assert!(!yaml.contains("suffix"), "{yaml}");
        assert_eq!(serde_yaml::from_str::<Anchor>(&yaml).unwrap(), anchor);
    }

    #[test]
    fn a_review_without_implements_or_change_branch_still_loads() {
        // Exactly the shape every pre-existing review file has.
        let yaml = "id: r1
author: a
target:
  kind: task
  slug: t
state: draft
created: 2026-07-01T00:00:00Z
comments: []
";
        let review: Review = serde_yaml::from_str(yaml).unwrap();
        assert!(review.implements.is_none());
        assert!(review.change_branch.is_none());
        // …and round-trips back without gaining the keys.
        let out = serde_yaml::to_string(&review).unwrap();
        assert!(!out.contains("implements"), "{out}");
        assert!(!out.contains("change_branch"), "{out}");
    }

    #[test]
    fn phase_status_display_from_str_round_trip() {
        let variants = [
            (PhaseStatus::NotStarted, "not-started"),
            (PhaseStatus::InProgress, "in-progress"),
            (PhaseStatus::NeedsReview, "needs-review"),
            (PhaseStatus::Reviewed, "reviewed"),
            (PhaseStatus::Done, "done"),
            (PhaseStatus::Blocked, "blocked"),
            (PhaseStatus::WontFix, "wont-fix"),
        ];
        for (variant, expected) in variants {
            assert_eq!(variant.to_string(), expected);
            let parsed: PhaseStatus = expected.parse().unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn phase_status_from_str_invalid() {
        assert!("invalid".parse::<PhaseStatus>().is_err());
    }

    #[test]
    fn phase_status_is_terminal() {
        assert!(PhaseStatus::Done.is_terminal());
        assert!(PhaseStatus::WontFix.is_terminal());
        assert!(!PhaseStatus::NotStarted.is_terminal());
        assert!(!PhaseStatus::InProgress.is_terminal());
        assert!(!PhaseStatus::Blocked.is_terminal());
    }

    #[test]
    fn task_status_is_terminal() {
        assert!(TaskStatus::Done.is_terminal());
        assert!(TaskStatus::WontFix.is_terminal());
        assert!(!TaskStatus::Open.is_terminal());
        assert!(!TaskStatus::InProgress.is_terminal());
        assert!(!TaskStatus::NeedsReview.is_terminal());
        assert!(!TaskStatus::Reviewed.is_terminal());
        assert!(!TaskStatus::Blocked.is_terminal());
    }

    #[test]
    fn task_status_display_from_str_round_trip() {
        let variants = [
            (TaskStatus::Open, "open"),
            (TaskStatus::InProgress, "in-progress"),
            (TaskStatus::NeedsReview, "needs-review"),
            (TaskStatus::Reviewed, "reviewed"),
            (TaskStatus::Done, "done"),
            (TaskStatus::Blocked, "blocked"),
            (TaskStatus::WontFix, "wont-fix"),
        ];
        for (variant, expected) in variants {
            assert_eq!(variant.to_string(), expected);
            let parsed: TaskStatus = expected.parse().unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn task_status_from_str_invalid() {
        assert!("invalid".parse::<TaskStatus>().is_err());
    }

    #[test]
    fn priority_display_from_str_round_trip() {
        let variants = [
            (Priority::Low, "low"),
            (Priority::Medium, "medium"),
            (Priority::High, "high"),
            (Priority::Critical, "critical"),
        ];
        for (variant, expected) in variants {
            assert_eq!(variant.to_string(), expected);
            let parsed: Priority = expected.parse().unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn priority_from_str_invalid() {
        assert!("invalid".parse::<Priority>().is_err());
    }

    #[test]
    fn difficulty_display_from_str_round_trip() {
        let variants = [
            (Difficulty::Trivial, "trivial"),
            (Difficulty::Easy, "easy"),
            (Difficulty::Moderate, "moderate"),
            (Difficulty::Hard, "hard"),
        ];
        for (variant, expected) in variants {
            assert_eq!(variant.to_string(), expected);
            let parsed: Difficulty = expected.parse().unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn difficulty_from_str_invalid_is_matchable_parse_error() {
        let err = "impossible".parse::<Difficulty>().unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid difficulty: 'impossible' (expected trivial, easy, moderate, or hard)"
        );
    }

    #[test]
    fn difficulty_ordering() {
        assert!(Difficulty::Hard > Difficulty::Moderate);
        assert!(Difficulty::Moderate > Difficulty::Easy);
        assert!(Difficulty::Easy > Difficulty::Trivial);
    }

    #[test]
    fn model_tier_display_from_str_round_trip() {
        let variants = [
            (ModelTier::Small, "small"),
            (ModelTier::Medium, "medium"),
            (ModelTier::Large, "large"),
        ];
        for (variant, expected) in variants {
            assert_eq!(variant.to_string(), expected);
            let parsed: ModelTier = expected.parse().unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn model_tier_from_str_invalid_is_matchable_parse_error() {
        let err = "xl".parse::<ModelTier>().unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid model tier: 'xl' (expected small, medium, or large)"
        );
    }

    #[test]
    fn model_tier_ordering() {
        assert!(ModelTier::Large > ModelTier::Medium);
        assert!(ModelTier::Medium > ModelTier::Small);
    }

    #[test]
    fn difficulty_maps_to_model_tier() {
        assert_eq!(Difficulty::Trivial.model_tier(), ModelTier::Small);
        assert_eq!(Difficulty::Easy.model_tier(), ModelTier::Small);
        assert_eq!(Difficulty::Moderate.model_tier(), ModelTier::Medium);
        assert_eq!(Difficulty::Hard.model_tier(), ModelTier::Large);
    }

    #[test]
    fn difficulty_model_tier_yaml_round_trip() {
        let yaml = serde_yaml::to_string(&Difficulty::Hard).unwrap();
        assert_eq!(yaml.trim(), "hard");
        let yaml = serde_yaml::to_string(&ModelTier::Large).unwrap();
        assert_eq!(yaml.trim(), "large");
    }

    #[test]
    fn task_status_filter_all() {
        let f: TaskStatusFilter = "all".parse().unwrap();
        assert_eq!(f, TaskStatusFilter::All);
        assert_eq!(f.to_string(), "all");
    }

    #[test]
    fn task_status_filter_specific() {
        let f: TaskStatusFilter = "done".parse().unwrap();
        assert_eq!(f, TaskStatusFilter::Status(TaskStatus::Done));
        assert_eq!(f.to_string(), "done");
    }

    #[test]
    fn task_status_filter_invalid() {
        assert!("invalid".parse::<TaskStatusFilter>().is_err());
    }

    #[test]
    fn phase_status_round_trip() {
        let variants = [
            (PhaseStatus::NotStarted, "not-started"),
            (PhaseStatus::InProgress, "in-progress"),
            (PhaseStatus::NeedsReview, "needs-review"),
            (PhaseStatus::Reviewed, "reviewed"),
            (PhaseStatus::Done, "done"),
            (PhaseStatus::Blocked, "blocked"),
            (PhaseStatus::WontFix, "wont-fix"),
        ];
        for (variant, expected_yaml) in variants {
            let yaml = serde_yaml::to_string(&variant).unwrap();
            assert_eq!(yaml.trim(), expected_yaml);
            let parsed: PhaseStatus = serde_yaml::from_str(&yaml).unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn task_status_round_trip() {
        let variants = [
            (TaskStatus::Open, "open"),
            (TaskStatus::InProgress, "in-progress"),
            (TaskStatus::NeedsReview, "needs-review"),
            (TaskStatus::Reviewed, "reviewed"),
            (TaskStatus::Done, "done"),
            (TaskStatus::Blocked, "blocked"),
            (TaskStatus::WontFix, "wont-fix"),
        ];
        for (variant, expected_yaml) in variants {
            let yaml = serde_yaml::to_string(&variant).unwrap();
            assert_eq!(yaml.trim(), expected_yaml);
            let parsed: TaskStatus = serde_yaml::from_str(&yaml).unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn priority_round_trip() {
        let variants = [
            (Priority::Low, "low"),
            (Priority::Medium, "medium"),
            (Priority::High, "high"),
            (Priority::Critical, "critical"),
        ];
        for (variant, expected_yaml) in variants {
            let yaml = serde_yaml::to_string(&variant).unwrap();
            assert_eq!(yaml.trim(), expected_yaml);
            let parsed: Priority = serde_yaml::from_str(&yaml).unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn phase_deserialize_all_fields() {
        let yaml = r#"
phase: 1
title: Core valuation layer
status: done
completed: 2026-03-13
commit: abc123def456
"#;
        let phase: Phase = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(phase.phase, 1);
        assert_eq!(phase.title, "Core valuation layer");
        assert_eq!(phase.status, PhaseStatus::Done);
        assert_eq!(
            phase.completed,
            Some(NaiveDate::from_ymd_opt(2026, 3, 13).unwrap())
        );
        assert_eq!(phase.commit, Some("abc123def456".to_string()));
    }

    #[test]
    fn phase_deserialize_missing_completed() {
        let yaml = r#"
phase: 2
title: Keeper service threading
status: not-started
"#;
        let phase: Phase = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(phase.phase, 2);
        assert_eq!(phase.status, PhaseStatus::NotStarted);
        assert_eq!(phase.completed, None);
        assert_eq!(phase.commit, None);
    }

    #[test]
    fn phase_deserialize_missing_difficulty_and_model_is_none() {
        let yaml = r#"
phase: 2
title: Keeper service threading
status: not-started
"#;
        let phase: Phase = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(phase.difficulty, None);
        assert_eq!(phase.model, None);
    }

    #[test]
    fn phase_serialize_omits_none_difficulty_and_model() {
        let phase = Phase {
            phase: 1,
            title: "Core".to_string(),
            status: PhaseStatus::NotStarted,
            tags: None,
            completed: None,
            commit: None,
            review_sha: None,
            review_branch: None,
            difficulty: None,
            model: None,
            blocked_reason: None,
            gate_override: None,
        };
        let yaml = serde_yaml::to_string(&phase).unwrap();
        assert!(!yaml.contains("difficulty"));
        assert!(!yaml.contains("model"));
        assert!(!yaml.contains("blocked_reason"));
    }

    #[test]
    fn phase_round_trips_difficulty_and_model() {
        let phase = Phase {
            phase: 1,
            title: "Core".to_string(),
            status: PhaseStatus::NotStarted,
            tags: None,
            completed: None,
            commit: None,
            review_sha: None,
            review_branch: None,
            difficulty: Some(Difficulty::Hard),
            model: Some(ModelTier::Large),
            blocked_reason: None,
            gate_override: None,
        };
        let yaml = serde_yaml::to_string(&phase).unwrap();
        assert!(yaml.contains("difficulty: hard"));
        assert!(yaml.contains("model: large"));
        let parsed: Phase = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed.difficulty, Some(Difficulty::Hard));
        assert_eq!(parsed.model, Some(ModelTier::Large));
    }

    #[test]
    fn phase_round_trips_blocked_reason() {
        let phase = Phase {
            phase: 1,
            title: "Core".to_string(),
            status: PhaseStatus::Blocked,
            tags: None,
            completed: None,
            commit: None,
            review_sha: None,
            review_branch: None,
            difficulty: None,
            model: None,
            blocked_reason: Some("ambiguous acceptance criterion".to_string()),
            gate_override: None,
        };
        let yaml = serde_yaml::to_string(&phase).unwrap();
        assert!(yaml.contains("blocked_reason: ambiguous acceptance criterion"));
        let parsed: Phase = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(
            parsed.blocked_reason.as_deref(),
            Some("ambiguous acceptance criterion")
        );
    }

    #[test]
    fn phase_deserialize_with_tags() {
        let yaml = r#"
phase: 1
title: Core valuation layer
status: not-started
tags: [infra, search]
"#;
        let phase: Phase = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            phase.tags,
            Some(vec!["infra".to_string(), "search".to_string()])
        );
    }

    #[test]
    fn phase_deserialize_without_tags() {
        let yaml = r#"
phase: 2
title: Keeper service threading
status: not-started
"#;
        let phase: Phase = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(phase.tags, None);
    }

    #[test]
    fn roadmap_deserialize_with_tags() {
        let yaml = r#"
project: fbm
roadmap: tagged
title: Tagged Roadmap
phases:
  - phase-1-only
tags: [api, cli]
"#;
        let roadmap: Roadmap = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            roadmap.tags,
            Some(vec!["api".to_string(), "cli".to_string()])
        );
    }

    #[test]
    fn roadmap_deserialize_without_tags() {
        let yaml = r#"
project: fbm
roadmap: solo
title: Solo Roadmap
phases:
  - phase-1-only
"#;
        let roadmap: Roadmap = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(roadmap.tags, None);
    }

    #[test]
    fn task_deserialize_with_tags() {
        let yaml = r#"
project: fbm
title: Fix barrel column NULL for 2024
status: open
priority: high
created: 2026-03-14
tags: [data, statcast]
"#;
        let task: Task = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(task.project, "fbm");
        assert_eq!(task.status, TaskStatus::Open);
        assert_eq!(task.priority, Priority::High);
        assert_eq!(task.created, NaiveDate::from_ymd_opt(2026, 3, 14).unwrap());
        assert_eq!(
            task.tags,
            Some(vec!["data".to_string(), "statcast".to_string()])
        );
    }

    #[test]
    fn task_deserialize_without_tags() {
        let yaml = r#"
project: fbm
title: Simple task
status: in-progress
priority: low
created: 2026-01-01
"#;
        let task: Task = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(task.tags, None);
    }

    #[test]
    fn task_round_trips_close_reason() {
        // Some(reason) round-trips through YAML.
        let task = Task {
            project: "fbm".to_string(),
            title: "Retired task".to_string(),
            status: TaskStatus::WontFix,
            priority: Priority::Low,
            created: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            tags: None,
            completed: None,
            commit: None,
            review_sha: None,
            review_branch: None,
            close_reason: Some("superseded by task/survivor".to_string()),
            gate_override: None,
        };
        let yaml = serde_yaml::to_string(&task).unwrap();
        assert!(yaml.contains("close_reason: superseded by task/survivor"));
        let parsed: Task = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(
            parsed.close_reason.as_deref(),
            Some("superseded by task/survivor")
        );

        // None is omitted from the serialized form.
        let task = Task {
            close_reason: None,
            gate_override: None,
            ..task
        };
        let yaml = serde_yaml::to_string(&task).unwrap();
        assert!(!yaml.contains("close_reason"));
        let parsed: Task = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed.close_reason, None);
    }

    #[test]
    fn roadmap_deserialize_with_dependencies() {
        let yaml = r#"
project: fbm
roadmap: two-way-players
title: Two-Way Player Identity
phases:
  - phase-1-core-valuation
  - phase-2-keeper-service
dependencies:
  - keeper-surplus-value
"#;
        let roadmap: Roadmap = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(roadmap.project, "fbm");
        assert_eq!(roadmap.roadmap, "two-way-players");
        assert_eq!(roadmap.phases.len(), 2);
        assert_eq!(
            roadmap.dependencies,
            Some(vec!["keeper-surplus-value".to_string()])
        );
    }

    #[test]
    fn roadmap_deserialize_without_dependencies() {
        let yaml = r#"
project: fbm
roadmap: solo
title: Solo Roadmap
phases:
  - phase-1-only
"#;
        let roadmap: Roadmap = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(roadmap.dependencies, None);
    }

    #[test]
    fn roadmap_deserialize_with_priority() {
        let yaml = r#"
project: fbm
roadmap: urgent-fix
title: Urgent Fix
phases:
  - phase-1-patch
priority: high
"#;
        let roadmap: Roadmap = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(roadmap.priority, Some(Priority::High));
    }

    #[test]
    fn roadmap_deserialize_without_priority() {
        let yaml = r#"
project: fbm
roadmap: solo
title: Solo Roadmap
phases:
  - phase-1-only
"#;
        let roadmap: Roadmap = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(roadmap.priority, None);
    }

    #[test]
    fn roadmap_serialize_with_priority() {
        let roadmap = Roadmap {
            project: "fbm".to_string(),
            roadmap: "urgent".to_string(),
            title: "Urgent".to_string(),
            phases: vec!["phase-1".to_string()],
            dependencies: None,
            priority: Some(Priority::Critical),
            tags: None,
        };
        let yaml = serde_yaml::to_string(&roadmap).unwrap();
        assert!(yaml.contains("priority: critical"));
    }

    #[test]
    fn roadmap_serialize_without_priority() {
        let roadmap = Roadmap {
            project: "fbm".to_string(),
            roadmap: "chill".to_string(),
            title: "Chill".to_string(),
            phases: vec!["phase-1".to_string()],
            dependencies: None,
            priority: None,
            tags: None,
        };
        let yaml = serde_yaml::to_string(&roadmap).unwrap();
        assert!(!yaml.contains("priority"));
    }

    #[test]
    fn project_round_trip() {
        let yaml = r#"
name: fbm
title: Fantasy Baseball Manager
"#;
        let project: Project = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(project.name, "fbm");
        assert_eq!(project.title, "Fantasy Baseball Manager");

        let serialized = serde_yaml::to_string(&project).unwrap();
        let parsed: Project = serde_yaml::from_str(&serialized).unwrap();
        assert_eq!(parsed, project);
    }

    #[test]
    fn priority_ordering() {
        assert!(Priority::Critical > Priority::High);
        assert!(Priority::High > Priority::Medium);
        assert!(Priority::Medium > Priority::Low);
        assert!(Priority::Low < Priority::Medium);
    }

    #[test]
    fn naive_date_serializes_as_yyyy_mm_dd() {
        let date = NaiveDate::from_ymd_opt(2026, 3, 14).unwrap();
        let yaml = serde_yaml::to_string(&date).unwrap();
        assert_eq!(yaml.trim(), "2026-03-14");
    }

    // -- Review model tests --

    #[test]
    fn review_state_display_from_str_round_trip() {
        let variants = [
            (ReviewState::Draft, "draft"),
            (ReviewState::Submitted, "submitted"),
            (ReviewState::Addressed, "addressed"),
            (ReviewState::Dismissed, "dismissed"),
        ];
        for (variant, expected) in variants {
            assert_eq!(variant.to_string(), expected);
            let parsed: ReviewState = expected.parse().unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn review_state_is_terminal() {
        assert!(ReviewState::Addressed.is_terminal());
        assert!(ReviewState::Dismissed.is_terminal());
        assert!(!ReviewState::Draft.is_terminal());
        assert!(!ReviewState::Submitted.is_terminal());
    }

    #[test]
    fn review_comment_status_is_terminal() {
        assert!(ReviewCommentStatus::Addressed.is_terminal());
        assert!(ReviewCommentStatus::WontFix.is_terminal());
        assert!(!ReviewCommentStatus::Open.is_terminal());
    }

    #[test]
    fn review_state_from_str_invalid() {
        let err = "pending".parse::<ReviewState>().unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid review state: 'pending' (expected draft, submitted, addressed, or dismissed)"
        );
    }

    #[test]
    fn review_state_yaml_round_trip() {
        let variants = [
            (ReviewState::Draft, "draft"),
            (ReviewState::Submitted, "submitted"),
            (ReviewState::Addressed, "addressed"),
            (ReviewState::Dismissed, "dismissed"),
        ];
        for (variant, expected_yaml) in variants {
            let yaml = serde_yaml::to_string(&variant).unwrap();
            assert_eq!(yaml.trim(), expected_yaml);
            let parsed: ReviewState = serde_yaml::from_str(&yaml).unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn verdict_display_from_str_round_trip() {
        let variants = [
            (Verdict::Approve, "approve"),
            (Verdict::RequestChanges, "request-changes"),
            (Verdict::Comment, "comment"),
        ];
        for (variant, expected) in variants {
            assert_eq!(variant.to_string(), expected);
            let parsed: Verdict = expected.parse().unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn verdict_from_str_invalid() {
        let err = "reject".parse::<Verdict>().unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid verdict: 'reject' (expected approve, request-changes, or comment)"
        );
    }

    #[test]
    fn verdict_yaml_round_trip() {
        let variants = [
            (Verdict::Approve, "approve"),
            (Verdict::RequestChanges, "request-changes"),
            (Verdict::Comment, "comment"),
        ];
        for (variant, expected_yaml) in variants {
            let yaml = serde_yaml::to_string(&variant).unwrap();
            assert_eq!(yaml.trim(), expected_yaml);
            let parsed: Verdict = serde_yaml::from_str(&yaml).unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn review_comment_status_display_from_str_round_trip() {
        let variants = [
            (ReviewCommentStatus::Open, "open"),
            (ReviewCommentStatus::Addressed, "addressed"),
            (ReviewCommentStatus::WontFix, "wont-fix"),
        ];
        for (variant, expected) in variants {
            assert_eq!(variant.to_string(), expected);
            let parsed: ReviewCommentStatus = expected.parse().unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn review_comment_status_from_str_invalid() {
        let err = "resolved".parse::<ReviewCommentStatus>().unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid review comment status: 'resolved' (expected open, addressed, or wont-fix)"
        );
    }

    #[test]
    fn review_comment_status_yaml_round_trip() {
        let variants = [
            (ReviewCommentStatus::Open, "open"),
            (ReviewCommentStatus::Addressed, "addressed"),
            (ReviewCommentStatus::WontFix, "wont-fix"),
        ];
        for (variant, expected_yaml) in variants {
            let yaml = serde_yaml::to_string(&variant).unwrap();
            assert_eq!(yaml.trim(), expected_yaml);
            let parsed: ReviewCommentStatus = serde_yaml::from_str(&yaml).unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn comment_doc_round_trip() {
        let yaml = "kind: phase\nstem: phase-1-design\n";
        let doc: CommentDoc = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(doc.kind, CommentDocKind::Phase);
        assert_eq!(doc.stem, "phase-1-design");
        let serialized = serde_yaml::to_string(&doc).unwrap();
        let parsed: CommentDoc = serde_yaml::from_str(&serialized).unwrap();
        assert_eq!(parsed, doc);
    }

    #[test]
    fn review_target_roadmap_round_trip() {
        let target = ReviewTarget::Roadmap {
            roadmap: "auth".to_string(),
        };
        let yaml = serde_yaml::to_string(&target).unwrap();
        assert!(yaml.contains("kind: roadmap"));
        assert!(yaml.contains("roadmap: auth"));
        let parsed: ReviewTarget = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, target);
    }

    #[test]
    fn review_target_phase_round_trip() {
        let target = ReviewTarget::Phase {
            roadmap: "auth".to_string(),
            stem: "phase-1-design".to_string(),
        };
        let yaml = serde_yaml::to_string(&target).unwrap();
        assert!(yaml.contains("kind: phase"));
        assert!(yaml.contains("roadmap: auth"));
        assert!(yaml.contains("stem: phase-1-design"));
        let parsed: ReviewTarget = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, target);
    }

    #[test]
    fn review_target_task_round_trip() {
        let target = ReviewTarget::Task {
            slug: "fix-login".to_string(),
        };
        let yaml = serde_yaml::to_string(&target).unwrap();
        assert!(yaml.contains("kind: task"));
        assert!(yaml.contains("slug: fix-login"));
        let parsed: ReviewTarget = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, target);
    }

    #[test]
    fn review_target_from_str_roadmap() {
        let target: ReviewTarget = "roadmap/auth".parse().unwrap();
        assert_eq!(
            target,
            ReviewTarget::Roadmap {
                roadmap: "auth".to_string()
            }
        );
    }

    #[test]
    fn review_target_from_str_phase_keeps_raw_stem() {
        let target: ReviewTarget = "phase/auth/phase-1-design".parse().unwrap();
        assert_eq!(
            target,
            ReviewTarget::Phase {
                roadmap: "auth".to_string(),
                stem: "phase-1-design".to_string(),
            }
        );
        // A numeric identifier is kept verbatim — resolution is the caller's job.
        let numeric: ReviewTarget = "phase/auth/2".parse().unwrap();
        assert_eq!(
            numeric,
            ReviewTarget::Phase {
                roadmap: "auth".to_string(),
                stem: "2".to_string(),
            }
        );
    }

    #[test]
    fn review_target_from_str_task() {
        let target: ReviewTarget = "task/fix-login".parse().unwrap();
        assert_eq!(
            target,
            ReviewTarget::Task {
                slug: "fix-login".to_string()
            }
        );
    }

    #[test]
    fn review_target_from_str_rejects_malformed() {
        for bad in ["", "bogus/x", "roadmap/", "phase/only-roadmap", "task/a/b"] {
            let err = bad.parse::<ReviewTarget>().unwrap_err();
            assert!(err.to_string().contains("item reference"));
        }
    }

    #[test]
    fn review_target_display_round_trips_item_refs() {
        for label in [
            "roadmap/auth",
            "phase/auth/phase-1-design",
            "task/fix-login",
        ] {
            let target: ReviewTarget = label.parse().unwrap();
            assert_eq!(target.to_string(), label);
            let round_tripped: ReviewTarget = target.to_string().parse().unwrap();
            assert_eq!(round_tripped, target);
        }
    }

    #[test]
    fn project_round_trips_with_source() {
        let doc = crate::document::Document {
            frontmatter: Project {
                name: "acme".to_string(),
                title: "Acme Corp".to_string(),
                source: Some(Source {
                    repo: "https://github.com/acme/repo".to_string(),
                    default_branch: Some("main".to_string()),
                }),
            },
            body: "Body text.\n".to_string(),
        };
        let rendered = doc.render().unwrap();
        let parsed = crate::document::Document::<Project>::parse(&rendered).unwrap();
        assert_eq!(parsed, doc);
    }

    #[test]
    fn project_round_trips_without_source() {
        let doc = crate::document::Document {
            frontmatter: Project {
                name: "acme".to_string(),
                title: "Acme Corp".to_string(),
                source: None,
            },
            body: "Body text.\n".to_string(),
        };
        let rendered = doc.render().unwrap();
        assert!(
            !rendered.contains("source:"),
            "expected no source: key, got:\n{rendered}"
        );
        let parsed = crate::document::Document::<Project>::parse(&rendered).unwrap();
        assert_eq!(parsed, doc);
    }

    #[test]
    fn anchor_text_quote_round_trip() {
        let anchor = Anchor::TextQuote {
            quote: "## Acceptance Criteria".to_string(),
            prefix: "right?\n\n".to_string(),
            suffix: "\n\n- [ ] Criterion".to_string(),
        };
        let yaml = serde_yaml::to_string(&anchor).unwrap();
        assert!(yaml.contains("anchor_type: text-quote"));
        assert!(yaml.contains("quote:"));
        assert!(yaml.contains("prefix:"));
        assert!(yaml.contains("suffix:"));
        let parsed: Anchor = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, anchor);
    }

    #[test]
    fn anchor_unknown_type_preserves_raw_yaml() {
        let yaml = "anchor_type: line-range\nstart: 3\nend: 7\nextra: keep-me\n";
        let parsed: Anchor = serde_yaml::from_str(yaml).unwrap();
        match &parsed {
            Anchor::Unknown { anchor_type, raw } => {
                assert_eq!(anchor_type, "line-range");
                assert_eq!(
                    raw.get("anchor_type").and_then(serde_yaml::Value::as_str),
                    Some("line-range")
                );
                assert_eq!(
                    raw.get("start").and_then(serde_yaml::Value::as_i64),
                    Some(3)
                );
                assert_eq!(raw.get("end").and_then(serde_yaml::Value::as_i64), Some(7));
                assert_eq!(
                    raw.get("extra").and_then(serde_yaml::Value::as_str),
                    Some("keep-me")
                );
            }
            other => panic!("expected Anchor::Unknown, got {other:?}"),
        }
        // Serializing an Unknown anchor re-emits the original mapping verbatim.
        let reserialized = serde_yaml::to_string(&parsed).unwrap();
        let reparsed: Anchor = serde_yaml::from_str(&reserialized).unwrap();
        assert_eq!(reparsed, parsed);
    }

    #[test]
    fn anchor_missing_anchor_type_is_error() {
        let yaml = "quote: some text\nprefix: a\nsuffix: b\n";
        let err = serde_yaml::from_str::<Anchor>(yaml).unwrap_err();
        assert!(
            err.to_string().contains("anchor_type"),
            "error should mention the missing field, got: {err}"
        );
    }

    #[test]
    fn anchor_non_string_anchor_type_is_type_error_not_missing() {
        let yaml = "anchor_type: 5\nquote: some text\nprefix: a\nsuffix: b\n";
        let err = serde_yaml::from_str::<Anchor>(yaml).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("anchor_type must be a string"),
            "error should say the field has the wrong type, got: {msg}"
        );
        assert!(
            !msg.contains("missing field"),
            "a present-but-mistyped field must not be reported as missing, got: {msg}"
        );
    }

    // --- AC4: the recorded status-model decision, mechanically enforced -----
    //
    // `docs/plan-review-gate-policy.md` records the decision that rdm adds NO
    // `planned` status between `in-progress` and `reviewed` — the plan
    // document's own `PlanStatus` already carries that signal, and a seventh
    // phase status would be a second source of truth for one fact. These two
    // tests are what make that decision cost something to reverse: adopting a
    // new status cannot land silently, it must come with a deliberate edit
    // here and therefore a deliberate revisit of the recorded decision.

    #[test]
    fn phase_status_variants_are_exactly_the_seven_recorded() {
        let all = [
            PhaseStatus::NotStarted,
            PhaseStatus::InProgress,
            PhaseStatus::NeedsReview,
            PhaseStatus::Reviewed,
            PhaseStatus::Done,
            PhaseStatus::Blocked,
            PhaseStatus::WontFix,
        ];
        // Exhaustive match: a new variant fails to compile here first.
        let names: Vec<&str> = all
            .iter()
            .map(|s| match s {
                PhaseStatus::NotStarted => "not-started",
                PhaseStatus::InProgress => "in-progress",
                PhaseStatus::NeedsReview => "needs-review",
                PhaseStatus::Reviewed => "reviewed",
                PhaseStatus::Done => "done",
                PhaseStatus::Blocked => "blocked",
                PhaseStatus::WontFix => "wont-fix",
            })
            .collect();
        assert_eq!(
            names,
            vec![
                "not-started",
                "in-progress",
                "needs-review",
                "reviewed",
                "done",
                "blocked",
                "wont-fix"
            ],
            "the phase status set changed — revisit the recorded `planned`-status \
             decision in docs/plan-review-gate-policy.md before updating this list"
        );
        for (s, name) in all.iter().zip(names.iter()) {
            assert_eq!(s.to_string(), *name);
        }
    }

    #[test]
    fn task_status_variants_are_exactly_the_seven_recorded() {
        let all = [
            TaskStatus::Open,
            TaskStatus::InProgress,
            TaskStatus::NeedsReview,
            TaskStatus::Reviewed,
            TaskStatus::Done,
            TaskStatus::Blocked,
            TaskStatus::WontFix,
        ];
        let names: Vec<&str> = all
            .iter()
            .map(|s| match s {
                TaskStatus::Open => "open",
                TaskStatus::InProgress => "in-progress",
                TaskStatus::NeedsReview => "needs-review",
                TaskStatus::Reviewed => "reviewed",
                TaskStatus::Done => "done",
                TaskStatus::Blocked => "blocked",
                TaskStatus::WontFix => "wont-fix",
            })
            .collect();
        assert_eq!(
            names,
            vec![
                "open",
                "in-progress",
                "needs-review",
                "reviewed",
                "done",
                "blocked",
                "wont-fix"
            ],
            "the task status set changed — revisit the recorded `planned`-status \
             decision in docs/plan-review-gate-policy.md before updating this list"
        );
        for (s, name) in all.iter().zip(names.iter()) {
            assert_eq!(s.to_string(), *name);
        }
    }
}
