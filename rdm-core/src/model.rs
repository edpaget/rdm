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
            "wont-fix" => Ok(TaskStatus::WontFix),
            other => Err(ParseError::new(
                "task status",
                other,
                "open, in-progress, needs-review, reviewed, done, or wont-fix",
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
/// `task`). Target existence is deliberately **not** validated at parse
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
/// Serialized as a tagged union keyed on `anchor_type`. Only `text-quote`
/// is modeled today; any other `anchor_type` written by a newer rdm
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
    /// The inline comments attached to this review.
    pub comments: Vec<ReviewComment>,
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }

    #[test]
    fn task_status_display_from_str_round_trip() {
        let variants = [
            (TaskStatus::Open, "open"),
            (TaskStatus::InProgress, "in-progress"),
            (TaskStatus::NeedsReview, "needs-review"),
            (TaskStatus::Reviewed, "reviewed"),
            (TaskStatus::Done, "done"),
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
tags: [api, mcp]
"#;
        let roadmap: Roadmap = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            roadmap.tags,
            Some(vec!["api".to_string(), "mcp".to_string()])
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
}
