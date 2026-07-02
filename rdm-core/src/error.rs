use crate::model::ReviewState;

/// Errors that can occur in rdm-core operations.
#[derive(Debug)]
pub enum Error {
    /// An I/O error occurred.
    Io(std::io::Error),
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
    /// `phase/<roadmap-slug>/<stem-or-number>`, or `task/<slug>`.
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
    /// A slug already exists.
    DuplicateSlug(String),
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
    /// HTTP/MCP surfaces as `clear_body: true`).
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
                    "invalid review target '{reference}' — expected roadmap/<slug>, phase/<roadmap-slug>/<stem-or-number>, or task/<slug>"
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
            Error::DuplicateSlug(slug) => {
                write!(f, "'{slug}' already exists — choose a different name")
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
