use crate::model::ReviewState;

/// Errors that can occur in rdm-core operations.
#[derive(Debug)]
pub enum Error {
    /// An I/O error occurred.
    Io(std::io::Error),
    /// A persisted change identity is not exactly 40 lowercase ASCII hex characters.
    InvalidStoredChangeRevision {
        /// The invalid identity field: "head" or "base".
        field: &'static str,
        /// The rejected value, preserved without normalization.
        value: String,
    },
    /// A source revision operand starts with a dash and could be a Git option.
    InvalidChangeRevisionInput(String),
    /// Failed to parse YAML frontmatter.
    FrontmatterParse(serde_yaml::Error),
    /// The document is missing a frontmatter block.
    FrontmatterMissing,
    /// Failed to parse the config file.
    ConfigParse(toml::de::Error),
    /// The config file was not found.
    ConfigNotFound,
    /// The plan repo is already initialized.
    AlreadyInitialized,
    /// The specified project was not found.
    ProjectNotFound(String),
    /// The specified roadmap was not found.
    RoadmapNotFound(String),
    /// The specified phase was not found.
    PhaseNotFound(String),
    /// The specified task was not found.
    TaskNotFound(String),
    /// The task is already in a terminal status ([`TaskStatus::Done`] or
    /// [`TaskStatus::WontFix`](crate::model::TaskStatus::WontFix)) and cannot
    /// be consolidated into a roadmap — a task can only be folded once.
    ///
    /// [`TaskStatus::Done`]: crate::model::TaskStatus::Done
    TaskAlreadyConsolidated(String),
    /// A task merge named the survivor as one of its own sources — a task
    /// cannot be merged into itself. Carries the offending slug.
    TaskMergeIntoSelf(String),
    /// A task merge was requested with no source tasks to fold in.
    TaskMergeNoSources,
    /// The specified implementation plan was not found.
    PlanNotFound(String),
    /// A plan with the requested slug already exists.
    PlanExists(String),
    /// The phase or task a plan would implement does not exist.
    PlanImplementsMissing(String),
    /// A plan's `implements` named a reference kind that cannot be
    /// implemented (only a phase or a task can be).
    PlanImplementsInvalidKind(String),
    /// A review's `--implements` named a plan that does not exist.
    ReviewImplementsMissing(String),
    /// A review's `--implements` named a reference kind other than
    /// `plan/<slug>`.
    ReviewImplementsInvalidKind(String),
    /// A review's `--implements` was given on a target kind that cannot
    /// implement a plan (everything but `change/<sha>`).
    ReviewImplementsNotApplicable(String),
    /// A review's `--implements` was left to inference, but the item it
    /// would have been inferred from has no `approved` plan.
    ReviewImplementsNoApprovedPlan(String),
    /// A review's `--implements` was left to inference, but the item it
    /// would have been inferred from has more than one `approved` plan.
    ReviewImplementsAmbiguous {
        /// Label of the item the inference ran against.
        item: String,
        /// Every approved plan, as a `rdm:plan/<slug>` reference.
        candidates: Vec<String>,
    },
    /// The `reviewed` transition gate refused: the item has no `approved`
    /// implementation plan. Carries the item's label.
    GateNoApprovedPlan(String),
    /// The `reviewed` transition gate refused: the item has at least one
    /// `approved` plan, but none of them is named by an approving `change/`
    /// review's `implements`.
    GateNoApprovedChangeReview {
        /// Label of the item being transitioned.
        item: String,
        /// Every approved plan that was checked, as a `plan/<slug>` reference.
        plans: Vec<String>,
    },
    /// The `reviewed` transition gate refused: the item's worktree has
    /// uncommitted changes.
    GateWorktreeDirty {
        /// Absolute path of the dirty worktree.
        path: String,
        /// The uncommitted paths, already capped.
        paths: Vec<String>,
        /// How many further uncommitted paths the cap dropped.
        truncated: usize,
    },
    /// The `reviewed` transition gate refused: the item's worktree could not
    /// be observed at all. Fail-closed — an unobservable worktree is never a
    /// clean one.
    GateWorktreeUnobservable {
        /// Label of the item being transitioned.
        item: String,
        /// What went wrong when probing the worktree.
        cause: String,
    },
    /// The `reviewed` transition gate refused: an approving `change/` review
    /// implementing the item's plan exists, but it was recorded against a
    /// different HEAD than the observed checkout (or the checkout's HEAD
    /// could not be read at all), so it cannot approve the code being
    /// marked reviewed. Distinct from
    /// [`Error::GateNoApprovedChangeReview`], which would tell the operator
    /// to create a review that already exists.
    GateStaleChangeReview {
        /// Label of the item being transitioned.
        item: String,
        /// Id of the approving review whose head did not match.
        review_id: String,
        /// The head that review was recorded at.
        reviewed_head: String,
        /// The observed checkout's HEAD, or `None` when it could not be
        /// read.
        observed_head: Option<String>,
    },
    /// `--override-gate` was passed an empty or whitespace-only reason.
    GateOverrideEmptyReason,
    /// `--override-gate` was passed while the `reviewed` transition gate is
    /// not enforcing, so there is nothing to bypass. Refused rather than
    /// silently ignored: the whole point of the override is that a bypass is
    /// an audited act, and honoring it as a no-op would discard the reason and
    /// actor the operator supplied without telling them.
    GateOverrideGateDisabled,
    /// A [`crate::worktree::WorktreeProbe`] bound to one item (via a review
    /// source selection) was asked about a different item. A caller bug, not
    /// a repository failure — deliberately distinct from [`Error::Git`], so
    /// a caller can tell a mismatch from "the repository cannot be queried".
    ReviewSourceItemMismatch {
        /// Label of the item the probe is bound to.
        expected: String,
        /// Label of the item it was asked about.
        found: String,
    },
    /// A registered source checkout has been switched off the branch it was
    /// registered on, so it no longer holds the work the probe was asked
    /// about. Distinct from [`Error::Git`] for the same reason as
    /// [`Error::ReviewSourceItemMismatch`].
    ReviewSourceBranchChanged {
        /// Absolute path of the registered checkout.
        path: String,
        /// The branch it was registered on.
        expected: String,
        /// The branch it is on now, as observed.
        found: String,
    },
    /// The plan a new plan would supersede does not exist.
    PlanSupersedesMissing(String),
    /// A plan's `supersedes` named a reference kind that is not a plan
    /// (only `plan/<slug>` can be superseded).
    PlanSupersedesInvalidKind(String),
    /// A plan named itself in its own `supersedes` field.
    PlanSupersedesSelf(String),
    /// The specified review was not found.
    ReviewNotFound(String),
    /// The plan item a review would target does not exist.
    ReviewTargetMissing(String),
    /// Repeated review-id generation attempts all collided with existing
    /// review files.
    ReviewIdExhausted,
    /// The operation requires the review to be a draft (comment structure
    /// changes, submission, and un-forced deletion are draft-only).
    ReviewNotDraft(String),
    /// The operation requires the review to be submitted (a comment's
    /// status, `applied_commit`, and `reply` only change after submission).
    ReviewNotSubmitted(String),
    /// A comment's `doc` names a document outside the review's scope.
    CommentDocOutOfScope(String),
    /// A comment's `doc` was set on a review whose target kind does not
    /// support document scoping (only roadmap reviews do).
    CommentDocNotApplicable,
    /// The referenced comment does not exist within the review.
    CommentNotFound {
        /// The review the comment was looked up in.
        review_id: String,
        /// The comment id that was not found.
        comment_id: u32,
    },
    /// A review was submitted without a verdict.
    ReviewMissingVerdict(String),
    /// A review was submitted with no comments and no summary.
    ReviewEmpty(String),
    /// The requested review state transition is not allowed by the review
    /// lifecycle state machine.
    ReviewInvalidTransition {
        /// The review whose transition was rejected.
        review_id: String,
        /// The review's current state.
        from: ReviewState,
        /// The state the transition attempted to reach.
        to: ReviewState,
    },
    /// A review cannot be marked addressed while comments remain open.
    ReviewOpenComments {
        /// The review whose transition was rejected.
        review_id: String,
        /// How many comments are still open.
        open_count: usize,
    },
    /// A review target reference did not match `roadmap/<slug>`,
    /// `phase/<roadmap-slug>/<stem-or-number>`, `task/<slug>`,
    /// `plan/<slug>`, or `change/<head-sha>`.
    InvalidReviewTargetRef(String),
    /// A comment document reference did not match `phase/<stem-or-number>`.
    InvalidCommentDocRef(String),
    /// The quoted text for a new comment anchor was not found in the
    /// document body it was searched against.
    QuoteNotFound {
        /// The quote that was searched for.
        quote: String,
        /// The commit the searched body was read at (`None` when the
        /// current body was used).
        commit: Option<String>,
    },
    /// The quoted text for a new comment anchor occurred more than once and
    /// no occurrence selector was given.
    QuoteAmbiguous {
        /// The quote that was searched for.
        quote: String,
        /// The commit the searched body was read at (`None` when the
        /// current body was used).
        commit: Option<String>,
        /// Every occurrence of the quote, with surrounding context, so the
        /// caller can pick one by its 1-based position.
        occurrences: Vec<crate::anchor::QuoteOccurrence>,
    },
    /// The requested quote occurrence is outside the range of matches found.
    QuoteOccurrenceOutOfRange {
        /// The quote that was searched for.
        quote: String,
        /// The 1-based occurrence that was requested.
        occurrence: usize,
        /// How many occurrences actually exist.
        available: usize,
    },
    /// A `--path --quote` comment on a `change/<sha>` review quoted text
    /// that lies outside every hunk the reviewed change touches in that
    /// file.
    QuoteOutsideChangedHunks {
        /// Repo-relative path the quote was located in.
        path: String,
        /// 1-based first line the quote spans.
        start_line: u32,
        /// 1-based last line the quote spans (inclusive).
        end_line: u32,
        /// The nearest touched hunk's 1-based inclusive line range, or
        /// `None` when the change touches no hunk in this file at all.
        nearest: Option<(u32, u32)>,
        /// The base..head range the hunks were computed from, for the
        /// message.
        range: String,
    },
    /// An operation that needs a plan-repo document was handed a
    /// `change/<sha>` target, which names commits in the project's source
    /// repository and has no document in the plan repo.
    ChangeTargetHasNoDocument(String),
    /// A `--path` named a file that does not exist at the revision it was
    /// looked up in (it was added later, or deleted by the change).
    ChangePathNotInRevision {
        /// Repo-relative path that was looked up.
        path: String,
        /// The revision it was looked up at.
        rev: String,
    },
    /// A `--path` contained a character the `rdm:src/<path>@<rev>#L<n>`
    /// permalink grammar reserves (`@` or `#`), so the permalink derived
    /// from the resulting anchor would not parse back to the same path.
    ChangePathNotLinkable {
        /// The normalized repo-relative path that was rejected.
        path: String,
        /// The first reserved character found in it.
        delimiter: char,
    },
    /// A `--path` named a directory or a submodule rather than a regular
    /// file — a comment can only anchor to a blob.
    ChangePathNotAFile {
        /// Repo-relative path that was looked up.
        path: String,
        /// The revision it was looked up at.
        rev: String,
        /// The object kind it actually named.
        found: crate::source::SourceObjectKind,
    },
    /// The revision a `change/<rev>` review names does not resolve in the
    /// project's source repository.
    ChangeRevisionNotFound(String),
    /// The revision `--base` names does not resolve in the project's source
    /// repository.
    ChangeBaseNotFound(String),
    /// The source repository being read does not contain the commit a
    /// `change/<sha>` review names as its head, so nothing about the reviewed
    /// content can be verified there.
    ///
    /// Distinct from [`Self::ChangePathNotInRevision`] on purpose: the remedy
    /// is environmental (fetch the branch, or point `source.repo` at the right
    /// checkout), and reporting it as a missing *path* was a confidently false
    /// statement about a file the commit does contain.
    ChangeHeadNotInSource {
        /// The reviewed head, as a full SHA.
        head: String,
        /// Where the repository was read from, when the implementation can
        /// name it (see [`crate::source::SourceRepo::location`]).
        location: Option<String>,
    },
    /// A `change/` review's derived reviewed range is empty: the merge base of
    /// its head and the project's default branch IS that head, so the range
    /// contains no commits and there is nothing to review.
    ///
    /// Raised only for a *derived* base. An explicit `--base` equal to the
    /// head is a supported shape (a deliberately code-free review), so the
    /// guard never fires on one.
    ChangeEmptyReviewedRange {
        /// The reviewed head, as a full SHA.
        head: String,
        /// The branch the merge base was derived against.
        branch: String,
    },
    /// A `change/` review's head and the project's default branch share no
    /// common ancestor, so the reviewed range cannot be derived.
    ChangeNoMergeBase {
        /// The reviewed head, as a full SHA.
        head: String,
        /// The branch the merge base was looked for against.
        branch: String,
    },
    /// The revision a change review's drift is measured against cannot be
    /// determined at all: the review's stamped branch (if any) no longer
    /// resolves and the source repository has no HEAD to fall back to — an
    /// unborn HEAD, or a directory that is not a usable repository.
    ///
    /// Produced by [`crate::change::resolve_drift_tip`].
    ChangeTipUnresolvable {
        /// The branch the review stamped at start time, when it stamped one.
        branch: Option<String>,
    },
    /// A `--quote` was given on a `change/` review without the `--path`
    /// that says which file to locate it in.
    ChangeQuoteNeedsPath,
    /// A `--path` was given on a `change/` review with no `--quote` to
    /// locate in it.
    ChangePathNeedsQuote,
    /// A slug already exists.
    DuplicateSlug(String),
    /// A roadmap slug collided with a reserved prefix (see
    /// [`crate::link::is_reserved_roadmap_slug`]).
    ReservedRoadmapSlug(String),
    /// Adding a dependency would create a cycle.
    CyclicDependency(String),
    /// No project was specified and no default project is configured.
    ProjectNotSpecified,
    /// Failed to serialize the config file.
    ConfigSerialize(toml::ser::Error),
    /// A relative path is invalid.
    InvalidPath(String),
    /// A specified phase stem is not part of the source roadmap.
    InvalidPhaseSelection(String),
    /// The roadmap has incomplete phases and cannot be archived without force.
    RoadmapHasIncompletePhases(String),
    /// A config value is not valid for the given key.
    InvalidConfigValue {
        /// The configuration key.
        key: String,
        /// The invalid value that was provided.
        value: String,
        /// A description of the valid values.
        valid: String,
    },
    /// A git operation failed.
    Git(String),
    /// The revision exists, but the requested path is not present in that
    /// revision (e.g. the file was added later or deleted at that point).
    BodyAtRevisionMissing {
        /// The relative path that was requested.
        path: String,
        /// The revision (commit SHA) that was searched.
        sha: String,
    },
    /// The requested revision does not exist in the backing store.
    RevisionUnknown {
        /// The revision (commit SHA) that could not be resolved.
        sha: String,
    },
    /// The storage backend has no notion of history (e.g. an unborn HEAD or a
    /// backend that opted out of revision-scoped reads).
    HistoryUnavailable,
    /// An update tried to replace a non-empty body with an empty string
    /// without an explicit opt-in. Callers must use [`BodyUpdate::Clear`] to
    /// confirm the clobber (the CLI exposes this as `--clear-body`, the
    /// HTTP surface as `clear_body: true`).
    ///
    /// [`BodyUpdate::Clear`]: crate::ops::BodyUpdate::Clear
    BodyClobberRefused,
    /// A title update carried an empty or whitespace-only value. Titles are
    /// required and cannot be cleared; callers must pass a non-empty title (or
    /// omit the update to leave the existing title unchanged). The CLI exposes
    /// this as `--title`.
    EmptyTitle,
    /// An update request set both a value and its `clear_*` flag for the same
    /// field — for example `--body` together with `--clear-body`. The two are
    /// contradictory; pass exactly one.
    ConflictingUpdate {
        /// The field name (`"body"`, `"priority"`, or `"tags"`).
        field: String,
    },
    /// A conditional estimate targeted a changed phase or an already-set estimate.
    PhaseEstimateConflict(String),
    /// A staged write was derived from content another process has since
    /// changed, so flushing it would silently drop that other work.
    ///
    /// Raised by the filesystem store's flush precondition *before* anything
    /// is written: the whole flush is refused, all-or-nothing, and the staged
    /// changes stay staged. The fix is always to re-run the command so it
    /// re-reads the current content.
    StaleWrite {
        /// The item, in the `task/<slug>` vocabulary users already know.
        item: String,
        /// The store-relative path, for the case where the item name alone is
        /// not enough to locate it.
        path: String,
    },
    /// A path this changeset journaled has since been overwritten by another
    /// session, so committing it would land the other session's bytes under
    /// this changeset's message.
    ///
    /// The commit-time half of the same content check: the flush precondition
    /// covers a write derived from a stale read; this covers the window
    /// between a successful flush and the scoped commit that lands it.
    ChangesetPathOverwritten {
        /// The item, in the `task/<slug>` vocabulary users already know.
        item: String,
        /// The store-relative path.
        path: String,
    },
    /// A path this changeset journaled a *deletion* of is present on disk
    /// again at commit time, so another session recreated it and applying the
    /// delete would destroy their content.
    ///
    /// The delete-side half of the same commit-time content check that raises
    /// [`Error::ChangesetPathOverwritten`] for writes. It is a distinct
    /// variant because "overwritten" describes the wrong side of a delete:
    /// nothing this changeset wrote was overwritten — the path this changeset
    /// left *absent* was refilled by someone else.
    ///
    /// The reference point is the **working tree at commit time**, never HEAD.
    /// A session that deletes a path leaves it absent, so absent means "still
    /// as I left it" and present means "someone else has been here". A HEAD
    /// basis would instead refuse a session's own multi-command uncommitted
    /// batch, which is the batching workflow rdm prescribes.
    ChangesetDeletePathRecreated {
        /// The item, in the `task/<slug>` vocabulary users already know.
        item: String,
        /// The store-relative path.
        path: String,
    },
    /// The plan repo root could not be determined from any source in the
    /// priority chain (explicit override, global config `root`, XDG data dir).
    RootNotDetermined,
    /// `~` was used in a path but `$HOME` is not set.
    HomeNotSet {
        /// The underlying error from reading the `HOME` environment variable.
        source: std::env::VarError,
    },
    /// Failed to resolve a path to an absolute, normalized form.
    PathResolutionFailed {
        /// The path that could not be resolved.
        path: std::path::PathBuf,
        /// The underlying I/O error from [`std::path::absolute`].
        source: std::io::Error,
    },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::InvalidStoredChangeRevision { field, value } => write!(
                f,
                "invalid stored change {field} {value:?}: restore a full resolved commit SHA (40 lowercase ASCII hexadecimal characters)"
            ),
            Error::InvalidChangeRevisionInput(value) => write!(
                f,
                "invalid change revision {value:?}: a revision must not start with '-'"
            ),
            Error::FrontmatterParse(e) => write!(f, "failed to parse frontmatter: {e}"),
            Error::FrontmatterMissing => {
                write!(f, "document is missing frontmatter delimiters (---)")
            }
            Error::ConfigParse(e) => write!(f, "failed to parse config: {e}"),
            Error::ConfigNotFound => write!(f, "rdm.toml not found — run `rdm init` first"),
            Error::AlreadyInitialized => {
                write!(f, "plan repo is already initialized (rdm.toml exists)")
            }
            Error::ProjectNotFound(name) => {
                write!(
                    f,
                    "project not found: {name} — create it with `rdm project create`"
                )
            }
            Error::RoadmapNotFound(name) => {
                write!(
                    f,
                    "roadmap not found: {name} — create it with `rdm roadmap create`"
                )
            }
            Error::PhaseNotFound(name) => {
                write!(f, "phase not found: {name}")
            }
            Error::TaskNotFound(name) => {
                write!(f, "task not found: {name}")
            }
            Error::TaskAlreadyConsolidated(slug) => {
                write!(
                    f,
                    "task '{slug}' is already terminal and cannot be consolidated into a roadmap"
                )
            }
            Error::TaskMergeIntoSelf(slug) => {
                write!(
                    f,
                    "cannot merge task '{slug}' into itself — remove it from --from"
                )
            }
            Error::TaskMergeNoSources => {
                write!(
                    f,
                    "no source tasks to merge — pass at least one --from <slug>"
                )
            }
            Error::PlanNotFound(slug) => {
                write!(
                    f,
                    "plan not found: {slug} — list existing plans with `rdm plan list`"
                )
            }
            Error::PlanExists(slug) => {
                write!(
                    f,
                    "plan '{slug}' already exists — choose a different slug, or supersede it with `rdm plan create <new-slug> --supersedes plan/{slug}`"
                )
            }
            Error::PlanImplementsMissing(msg) => {
                write!(
                    f,
                    "plan target not found: {msg} — pass --implements with an existing phase or task"
                )
            }
            Error::PlanImplementsInvalidKind(label) => {
                write!(
                    f,
                    "'{label}' cannot be implemented by a plan — pass --implements phase/<roadmap>/<stem-or-number> or --implements task/<slug>"
                )
            }
            Error::ReviewImplementsMissing(slug) => {
                write!(
                    f,
                    "plan '{slug}' not found — pass --implements rdm:plan/<slug> naming an existing plan (see `rdm plan list`)"
                )
            }
            Error::ReviewImplementsInvalidKind(label) => {
                write!(
                    f,
                    "'{label}' is not an implementation plan — pass --implements rdm:plan/<slug>"
                )
            }
            Error::ReviewImplementsNotApplicable(label) => {
                write!(
                    f,
                    "--implements records which plan a reviewed *change* implements, so it does not apply to a review of '{label}' — start the review with --on change/<sha> to use it"
                )
            }
            Error::ReviewImplementsNoApprovedPlan(item) => {
                write!(
                    f,
                    "no approved plan for {item} — pass --implements rdm:plan/<slug>, or approve one (`rdm plan list --implements {item}`)"
                )
            }
            Error::ReviewImplementsAmbiguous { item, candidates } => {
                write!(
                    f,
                    "{item} has {} approved plans, so the implemented plan is ambiguous — pass --implements with one of: {}",
                    candidates.len(),
                    candidates.join(", ")
                )
            }
            Error::GateNoApprovedPlan(item) => {
                write!(
                    f,
                    "refusing to mark {item} reviewed: no approved implementation plan implements it — \
                     write one with `rdm plan create <slug> --implements {item}`, then approve it with \
                     `rdm review start --on plan/<slug>` and `rdm review submit <id> --verdict approve` \
                     (or bypass the record checks with `--override-gate \"<reason>\"`)"
                )
            }
            Error::GateNoApprovedChangeReview { item, plans } => {
                write!(
                    f,
                    "refusing to mark {item} reviewed: no approving change review implements {} — \
                     record one with `rdm review start --on change/HEAD --implements plan/{}`, then \
                     `rdm review submit <id> --verdict approve` (or bypass the record checks with \
                     `--override-gate \"<reason>\"`). Checked: {}",
                    if plans.len() == 1 {
                        format!("plan/{}", plans[0])
                    } else {
                        format!("any of its {} approved plans", plans.len())
                    },
                    plans.first().map_or("<slug>", String::as_str),
                    plans
                        .iter()
                        .map(|p| format!("plan/{p}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
            Error::GateWorktreeDirty {
                path,
                paths,
                truncated,
            } => {
                let more = if *truncated > 0 {
                    format!(" …and {truncated} more")
                } else {
                    String::new()
                };
                write!(
                    f,
                    "refusing to mark this item reviewed: worktree {path} has uncommitted changes: {}{more} — \
                     commit or stash them, then retry. `--override-gate` does NOT bypass this check: it waives \
                     the plan and change-review records only.",
                    paths.join(", ")
                )
            }
            Error::GateWorktreeUnobservable { item, cause } => {
                write!(
                    f,
                    "refusing to mark {item} reviewed: its worktree could not be inspected ({cause}) — \
                     an unobservable worktree is never a clean one. Fix the repository (or run from the \
                     project checkout), then retry. `--override-gate` does NOT bypass this check."
                )
            }
            Error::GateStaleChangeReview {
                item,
                review_id,
                reviewed_head,
                observed_head,
            } => {
                let short = |rev: &str| {
                    let mut at = rev.len().min(12);
                    while at > 0 && !rev.is_char_boundary(at) {
                        at -= 1;
                    }
                    rev[..at].to_string()
                };
                match observed_head {
                    Some(observed) => write!(
                        f,
                        "refusing to mark {item} reviewed: the approving change review {review_id} \
                         was recorded at HEAD {} but the checkout is at HEAD {} — re-review the \
                         current HEAD with `rdm review start --on change/HEAD --implements \
                         plan/<slug>` then `rdm review submit <id> --verdict approve` (or bypass \
                         the record checks with `--override-gate \"<reason>\"`)",
                        short(reviewed_head),
                        short(observed),
                    ),
                    None => write!(
                        f,
                        "refusing to mark {item} reviewed: the approving change review {review_id} \
                         was recorded at HEAD {}, but the checkout's own HEAD could not be \
                         observed, so no approval can be matched to it — re-review the current \
                         HEAD with `rdm review start --on change/HEAD --implements plan/<slug>` \
                         then `rdm review submit <id> --verdict approve` (or bypass the record \
                         checks with `--override-gate \"<reason>\"`)",
                        short(reviewed_head),
                    ),
                }
            }
            Error::GateOverrideGateDisabled => {
                write!(
                    f,
                    "--override-gate has nothing to bypass: the `reviewed` transition gate is not \
                     enforcing in this plan repo, so the write needs no override and the reason and \
                     actor would not be recorded — drop the flag, or enable the gate first with \
                     `rdm config set gates.reviewed true`"
                )
            }
            Error::ReviewSourceItemMismatch { expected, found } => {
                write!(
                    f,
                    "this worktree probe is bound to {expected}, but was asked about {found} — \
                     re-bind the probe to the item being inspected (one probe answers for one \
                     review source)"
                )
            }
            Error::ReviewSourceBranchChanged {
                path,
                expected,
                found,
            } => {
                write!(
                    f,
                    "the registered checkout {path} was registered on branch '{expected}' but is \
                     now on '{found}' — run `git switch {expected}` in it, or re-register the \
                     checkout with `rdm worktree add`"
                )
            }
            Error::GateOverrideEmptyReason => {
                write!(
                    f,
                    "--override-gate requires a non-empty reason — it is recorded on the item as the \
                     audit trail for the bypass, e.g. --override-gate \"hotfix: plan filed retroactively\""
                )
            }
            Error::PlanSupersedesMissing(slug) => {
                write!(
                    f,
                    "superseded plan not found: {slug} — pass --supersedes plan/<slug> naming an existing plan"
                )
            }
            Error::PlanSupersedesInvalidKind(label) => {
                write!(
                    f,
                    "'{label}' is not a plan and cannot be superseded — pass --supersedes plan/<slug> naming an existing plan"
                )
            }
            Error::PlanSupersedesSelf(slug) => {
                write!(
                    f,
                    "plan '{slug}' cannot supersede itself — drop --supersedes, or name a different plan"
                )
            }
            Error::ReviewNotFound(id) => {
                write!(f, "review not found: {id}")
            }
            Error::ReviewTargetMissing(msg) => {
                write!(f, "review target not found: {msg}")
            }
            Error::ReviewIdExhausted => {
                write!(
                    f,
                    "failed to generate a unique review id after repeated attempts — try again"
                )
            }
            Error::ReviewNotDraft(id) => {
                write!(
                    f,
                    "review '{id}' is not a draft — comment structure, submission, and un-forced deletion all require the draft state"
                )
            }
            Error::ReviewNotSubmitted(id) => {
                write!(
                    f,
                    "review '{id}' has not been submitted — a comment's status, applied_commit, and reply can only change after submission"
                )
            }
            Error::CommentDocOutOfScope(msg) => {
                write!(f, "comment doc out of scope: {msg}")
            }
            Error::CommentDocNotApplicable => {
                write!(
                    f,
                    "'doc' can only be set on a roadmap review, to scope a comment to one of the roadmap's phases"
                )
            }
            Error::CommentNotFound {
                review_id,
                comment_id,
            } => {
                write!(f, "comment {comment_id} not found in review '{review_id}'")
            }
            Error::ReviewMissingVerdict(id) => {
                write!(
                    f,
                    "review '{id}' cannot be submitted without a verdict — provide approve, request-changes, or comment"
                )
            }
            Error::ReviewEmpty(id) => {
                write!(
                    f,
                    "review '{id}' has no comments and no summary — add at least one before submitting"
                )
            }
            Error::ReviewInvalidTransition {
                review_id,
                from,
                to,
            } => {
                write!(f, "review '{review_id}' cannot move from {from} to {to}")
            }
            Error::ReviewOpenComments {
                review_id,
                open_count,
            } => {
                write!(
                    f,
                    "review '{review_id}' has {open_count} open comment(s) — resolve them (addressed or wont-fix) before marking the review addressed"
                )
            }
            Error::InvalidReviewTargetRef(reference) => {
                write!(
                    f,
                    "invalid review target '{reference}' — expected roadmap/<slug>, phase/<roadmap-slug>/<stem-or-number>, task/<slug>, plan/<slug>, or change/<head-sha>"
                )
            }
            Error::InvalidCommentDocRef(reference) => {
                write!(
                    f,
                    "invalid comment doc '{reference}' — expected phase/<stem-or-number>"
                )
            }
            Error::QuoteNotFound { quote, commit } => match commit {
                Some(sha) => write!(
                    f,
                    "quote {quote:?} not found in the document as of the review's created commit ({sha}) — the document may have changed since the review started; view that version with `--at {sha}`, or start a fresh review to comment on the current text"
                ),
                None => write!(
                    f,
                    "quote {quote:?} not found in the current document — check the exact text (including punctuation and whitespace), or omit --quote for a whole-document comment"
                ),
            },
            Error::QuoteAmbiguous {
                quote,
                commit,
                occurrences,
            } => {
                let where_ = match commit {
                    Some(sha) => {
                        format!("the document as of the review's created commit ({sha})")
                    }
                    None => "the current document".to_string(),
                };
                writeln!(
                    f,
                    "quote {quote:?} occurs {} times in {where_} — pass --occurrence <n> (1-based) to pick one:",
                    occurrences.len()
                )?;
                for occ in occurrences {
                    writeln!(f, "  {}: {}", occ.occurrence, occ.context)?;
                }
                Ok(())
            }
            Error::QuoteOccurrenceOutOfRange {
                quote,
                occurrence,
                available,
            } => {
                write!(
                    f,
                    "--occurrence {occurrence} is out of range for quote {quote:?} — only {available} occurrence(s) found (valid: 1..={available})"
                )
            }
            Error::QuoteOutsideChangedHunks {
                path,
                start_line,
                end_line,
                nearest,
                range,
            } => match nearest {
                Some((hs, he)) => write!(
                    f,
                    "the quote lands on {path} lines {start_line}-{end_line}, which {range} does not touch — quote text inside a changed hunk (nearest: lines {hs}-{he}), or omit --path/--quote for a whole-change comment"
                ),
                None => write!(
                    f,
                    "'{path}' is not touched by {range} — comment on a file the change modifies, or omit --path/--quote for a whole-change comment"
                ),
            },
            Error::ChangeTargetHasNoDocument(label) => {
                write!(
                    f,
                    "'{label}' names commits in the source repository, not a document in the plan repo — its comment anchors resolve through `rdm review show`, and its code is linked with rdm:src/<path>@<sha>"
                )
            }
            Error::ChangePathNotInRevision { path, rev } => {
                write!(
                    f,
                    "'{path}' does not exist at {rev} — check the path (it is relative to the source repository root), or omit --path/--quote for a whole-change comment"
                )
            }
            Error::ChangePathNotLinkable { path, delimiter } => {
                write!(
                    f,
                    "source path '{path}' contains '{delimiter}', which the rdm:src/<path>@<rev>#L<n> permalink grammar reserves — the anchor could not be linked back to the file; rename the file, or omit --path/--quote for a whole-change comment"
                )
            }
            Error::ChangePathNotAFile { path, rev, found } => {
                let noun = match found {
                    crate::source::SourceObjectKind::Tree => "a directory",
                    crate::source::SourceObjectKind::Gitlink => "a submodule",
                    crate::source::SourceObjectKind::Blob => "not a file",
                };
                write!(
                    f,
                    "'{path}' is {noun} at {rev} — anchor a comment to a file, or omit --path/--quote for a whole-change comment"
                )
            }
            Error::ChangeRevisionNotFound(rev) => {
                write!(
                    f,
                    "'{rev}' does not name a commit in the source repository — pass --on change/<sha>, a branch name, or change/HEAD from inside the checkout"
                )
            }
            Error::ChangeBaseNotFound(rev) => {
                write!(
                    f,
                    "--base '{rev}' does not name a commit in the source repository"
                )
            }
            Error::ChangeHeadNotInSource { head, location } => match location {
                Some(location) => write!(
                    f,
                    "the source repository at {location} does not contain the reviewed commit {head} — anchor resolution skipped (fetch the branch, or point the project's `source.repo` at the right checkout)"
                ),
                None => write!(
                    f,
                    "the source repository does not contain the reviewed commit {head} — anchor resolution skipped (fetch the branch, or point the project's `source.repo` at the right checkout)"
                ),
            },
            Error::ChangeEmptyReviewedRange { head, branch } => {
                write!(
                    f,
                    "an empty reviewed range is never a reviewable change — the merge base of {head} and '{branch}' IS {head}, so nothing is under review; review a commit that changes something, or pass --base <rev> naming the revision the change is diffed against"
                )
            }
            Error::ChangeNoMergeBase { head, branch } => {
                write!(
                    f,
                    "no merge base between {head} and '{branch}' — unrelated history, an orphan branch, or a shallow clone; pass --base <rev> to name the revision the change is diffed against"
                )
            }
            Error::ChangeTipUnresolvable { branch } => match branch {
                Some(branch) => write!(
                    f,
                    "branch '{branch}' no longer resolves in the source repository and it has no HEAD to fall back to — restore the branch, or run this from a checkout with at least one commit"
                ),
                None => write!(
                    f,
                    "the source repository has no resolvable HEAD to measure drift against — run this from a checkout with at least one commit"
                ),
            },
            Error::ChangeQuoteNeedsPath => {
                write!(
                    f,
                    "--quote on a change review needs --path <repo-relative path> naming the file the quote lives in"
                )
            }
            Error::ChangePathNeedsQuote => {
                write!(
                    f,
                    "--path needs --quote — pass both, or neither for a whole-change comment"
                )
            }
            Error::DuplicateSlug(slug) => {
                write!(f, "'{slug}' already exists — choose a different name")
            }
            Error::ReservedRoadmapSlug(slug) => {
                write!(
                    f,
                    "'{slug}' is a reserved prefix and cannot be used as a roadmap slug"
                )
            }
            Error::CyclicDependency(msg) => {
                write!(f, "cyclic dependency: {msg}")
            }
            Error::ProjectNotSpecified => {
                write!(
                    f,
                    "no project specified — use --project or set default_project in rdm.toml"
                )
            }
            Error::ConfigSerialize(e) => write!(f, "failed to serialize config: {e}"),
            Error::InvalidPath(msg) => write!(f, "invalid path: {msg}"),
            Error::InvalidPhaseSelection(msg) => {
                write!(f, "invalid phase selection: {msg}")
            }
            Error::RoadmapHasIncompletePhases(slug) => {
                write!(
                    f,
                    "roadmap '{slug}' has incomplete phases — pass --force to archive anyway"
                )
            }
            Error::InvalidConfigValue { key, value, valid } => {
                write!(
                    f,
                    "invalid value '{value}' for '{key}' — valid values: {valid}"
                )
            }
            Error::Git(msg) => write!(f, "git error: {msg}"),
            Error::BodyAtRevisionMissing { path, sha } => {
                write!(f, "path '{path}' is not present at revision {sha}")
            }
            Error::RevisionUnknown { sha } => {
                write!(f, "revision '{sha}' is not known to the store")
            }
            Error::HistoryUnavailable => {
                write!(
                    f,
                    "the store has no history available (unborn HEAD or backend without revision support)"
                )
            }
            Error::BodyClobberRefused => {
                write!(
                    f,
                    "refusing to overwrite non-empty body with an empty value without explicit opt-in"
                )
            }
            Error::EmptyTitle => {
                write!(
                    f,
                    "title cannot be empty or whitespace-only — pass a non-empty --title (omit --title to leave the existing title unchanged)"
                )
            }
            Error::ConflictingUpdate { field } => {
                write!(f, "cannot set both '{field}' and 'clear_{field}'")
            }
            Error::PhaseEstimateConflict(stem) => write!(
                f,
                "conditional estimate refused for '{stem}': phase changed or difficulty/model is already set; read the phase again before estimating"
            ),
            Error::StaleWrite { item, path } => {
                write!(
                    f,
                    "refusing to write {item}: it changed on disk after this command read it \
                     (another session wrote it concurrently). Nothing was written — re-run the \
                     command so your change applies on top of the current content. Path: {path}"
                )
            }
            Error::ChangesetPathOverwritten { item, path } => {
                write!(
                    f,
                    "refusing to commit {item}: another session overwrote it after this changeset \
                     wrote it, so committing would land their content under your message. Nothing \
                     was committed — re-running the command that produced your change will fold in \
                     whatever content is on disk now (which may be that other session's \
                     already-landed edit) before committing under your message; re-read the item \
                     first if that is not what you want. Path: {path}"
                )
            }
            Error::ChangesetDeletePathRecreated { item, path } => {
                write!(
                    f,
                    "refusing to commit the deletion of {item}: it is present on disk again, so \
                     another session recreated it after this changeset deleted it — committing \
                     would destroy their content. Nothing was committed — re-read the item (it \
                     exists again) and re-run your delete if it is still right, then commit. \
                     Path: {path}"
                )
            }
            Error::RootNotDetermined => {
                write!(
                    f,
                    "cannot determine plan repo location — set RDM_ROOT, \
                     or add root to ~/.config/rdm/config.toml"
                )
            }
            Error::HomeNotSet { .. } => {
                write!(f, "~ used in path but $HOME is not set")
            }
            Error::PathResolutionFailed { path, .. } => {
                write!(f, "failed to resolve path: {}", path.display())
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::FrontmatterParse(e) => Some(e),
            Error::ConfigParse(e) => Some(e),
            Error::ConfigSerialize(e) => Some(e),
            Error::PathResolutionFailed { source, .. } => Some(source),
            Error::HomeNotSet { source } => Some(source),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<serde_yaml::Error> for Error {
    fn from(e: serde_yaml::Error) -> Self {
        Error::FrontmatterParse(e)
    }
}

impl From<toml::de::Error> for Error {
    fn from(e: toml::de::Error) -> Self {
        Error::ConfigParse(e)
    }
}

impl From<toml::ser::Error> for Error {
    fn from(e: toml::ser::Error) -> Self {
        Error::ConfigSerialize(e)
    }
}

/// A convenient `Result` type for rdm-core.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::Error;

    /// The refusal is a user-facing message on the `rdm commit` path, so its
    /// two obligations — name the item, say what to do — are gated here rather
    /// than left to whoever last edited the wording.
    #[test]
    fn changeset_delete_path_recreated_message_names_the_item_and_the_remedy() {
        let rendered = Error::ChangesetDeletePathRecreated {
            item: "task/fix-bug".to_string(),
            path: "projects/demo/tasks/fix-bug.md".to_string(),
        }
        .to_string();

        assert!(
            rendered.contains("task/fix-bug"),
            "the message must name the item in the vocabulary users know: {rendered}"
        );
        assert!(
            rendered.contains("recreated"),
            "the message must say another session recreated the path: {rendered}"
        );
        assert!(
            rendered.to_lowercase().contains("nothing was committed"),
            "the message must state that nothing landed: {rendered}"
        );
        assert!(
            rendered.contains("re-run your delete"),
            "the message must state the remedy: {rendered}"
        );
        assert!(
            rendered.contains("projects/demo/tasks/fix-bug.md"),
            "the raw path is the fallback locator: {rendered}"
        );
    }

    /// Wrapping a long `write!` literal across source lines without a `\`
    /// continuation bakes the source indentation into the message, so the
    /// rendered text carries a mid-sentence run of raw spaces. `cargo fmt`
    /// never touches string-literal contents and every other assertion on
    /// these messages is a `contains` over an intact substring, so nothing
    /// else in the suite notices. Every gate refusal and every review-source
    /// mismatch is operator-facing (CLI stderr, and the server's 409
    /// problem-detail body), so the rendering is gated here for all of them
    /// at once rather than per message.
    #[test]
    fn operator_facing_gate_and_review_source_messages_render_without_space_runs() {
        let rendered: Vec<(&str, String)> = vec![
            (
                "GateNoApprovedPlan",
                Error::GateNoApprovedPlan("task/fix-bug".to_string()).to_string(),
            ),
            (
                "GateNoApprovedChangeReview",
                Error::GateNoApprovedChangeReview {
                    item: "task/fix-bug".to_string(),
                    plans: vec!["plan/fix-bug".to_string()],
                }
                .to_string(),
            ),
            (
                "GateWorktreeDirty",
                Error::GateWorktreeDirty {
                    path: "/wt/gates".to_string(),
                    paths: vec!["oops.rs".to_string()],
                    truncated: 2,
                }
                .to_string(),
            ),
            (
                "GateWorktreeUnobservable",
                Error::GateWorktreeUnobservable {
                    item: "task/fix-bug".to_string(),
                    cause: "no such repository".to_string(),
                }
                .to_string(),
            ),
            (
                "GateStaleChangeReview (observed head)",
                Error::GateStaleChangeReview {
                    item: "task/fix-bug".to_string(),
                    review_id: "r1".to_string(),
                    reviewed_head: "a".repeat(40),
                    observed_head: Some("b".repeat(40)),
                }
                .to_string(),
            ),
            (
                "GateStaleChangeReview (unobserved head)",
                Error::GateStaleChangeReview {
                    item: "task/fix-bug".to_string(),
                    review_id: "r1".to_string(),
                    reviewed_head: "a".repeat(40),
                    observed_head: None,
                }
                .to_string(),
            ),
            (
                "GateOverrideEmptyReason",
                Error::GateOverrideEmptyReason.to_string(),
            ),
            (
                "GateOverrideGateDisabled",
                Error::GateOverrideGateDisabled.to_string(),
            ),
            (
                "ReviewSourceItemMismatch",
                Error::ReviewSourceItemMismatch {
                    expected: "task/foo".to_string(),
                    found: "task/bar".to_string(),
                }
                .to_string(),
            ),
            (
                "ReviewSourceBranchChanged",
                Error::ReviewSourceBranchChanged {
                    path: "/wt/foo".to_string(),
                    expected: "roadmap/foo".to_string(),
                    found: "main".to_string(),
                }
                .to_string(),
            ),
            (
                "ChangePathNotLinkable",
                Error::ChangePathNotLinkable {
                    path: "src/a@b.rs".to_string(),
                    delimiter: '@',
                }
                .to_string(),
            ),
            (
                "ChangeHeadNotInSource (named location)",
                Error::ChangeHeadNotInSource {
                    head: "a".repeat(40),
                    location: Some("/srv/source".to_string()),
                }
                .to_string(),
            ),
            (
                "ChangeHeadNotInSource (no location)",
                Error::ChangeHeadNotInSource {
                    head: "a".repeat(40),
                    location: None,
                }
                .to_string(),
            ),
            (
                "ChangeEmptyReviewedRange",
                Error::ChangeEmptyReviewedRange {
                    head: "a".repeat(40),
                    branch: "main".to_string(),
                }
                .to_string(),
            ),
        ];

        for (name, message) in &rendered {
            assert!(
                !message.contains("  "),
                "{name} renders a run of consecutive spaces — rejoin the write! literal or use a \\ \
                 line continuation: {message:?}"
            );
            assert!(
                !message.contains('\n'),
                "{name} renders an embedded newline, which breaks single-line CLI stderr and the \
                 server's problem-detail body: {message:?}"
            );
        }
    }
}
