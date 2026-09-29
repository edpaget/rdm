/// Model introspection: discover what rdm tracks and the shape of each entity.
///
/// The [`Describe`] trait and supporting types let agents and tools inspect
/// rdm's data model at runtime without reading source code.
use std::str::FromStr;

use serde::Serialize;

use crate::model::{PhaseStatus, ReviewCommentStatus, StatusNote, TaskStatus};

/// Metadata about a single field on an entity.
///
/// Status fields also carry rdm's declared status metadata —
/// `terminal_values` and `value_flags` — computed from the status type
/// itself, so a client reads which statuses are terminal and which record a
/// note instead of copying those rules. Every other field leaves both empty
/// (and they are omitted from JSON).
#[derive(Debug, Clone, Default, Serialize)]
pub struct FieldInfo {
    /// Field name (matches the serde/YAML key).
    pub name: &'static str,
    /// Human-readable type description (e.g. "string", "date (YYYY-MM-DD)").
    pub type_name: &'static str,
    /// Whether the field is always present (not optional).
    pub required: bool,
    /// Allowed values for enum fields (empty slice for free-form fields).
    pub enum_values: &'static [&'static str],
    /// Short description of what the field represents.
    pub description: &'static str,
    /// The subset of `enum_values` that are terminal statuses (empty for a
    /// non-status field).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub terminal_values: Vec<&'static str>,
    /// For each of `enum_values` that records a note, the CLI flag that
    /// records it (empty for a non-status field).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub value_flags: Vec<ValueFlagInfo>,
}

/// A status value and the CLI flag recording the note it takes (e.g.
/// `blocked` → `reason`, for `--reason`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ValueFlagInfo {
    /// The status value (one of the field's `enum_values`).
    pub value: &'static str,
    /// The CLI long flag, without the leading `--` ([`StatusNote::flag`]).
    pub flag: &'static str,
}

impl FieldInfo {
    /// Fills `terminal_values` and `value_flags` from the status type `S`:
    /// each of `enum_values` is parsed with `S`'s `FromStr`, and `meta`
    /// reports whether it is terminal and which note it records. A value
    /// that does not parse gets no metadata.
    fn with_status_metadata<S: FromStr>(
        mut self,
        meta: impl Fn(&S) -> (bool, Option<StatusNote>),
    ) -> Self {
        for &value in self.enum_values {
            let Ok(status) = S::from_str(value) else {
                continue;
            };
            let (terminal, note) = meta(&status);
            if terminal {
                self.terminal_values.push(value);
            }
            if let Some(note) = note {
                self.value_flags.push(ValueFlagInfo {
                    value,
                    flag: note.flag(),
                });
            }
        }
        self
    }
}

/// Metadata about an entity type (project, roadmap, phase, or task).
#[derive(Debug, Clone, Serialize)]
pub struct EntityInfo {
    /// Entity name (lowercase, e.g. "project").
    pub name: &'static str,
    /// One-line description of this entity.
    pub description: &'static str,
    /// Fields in the entity's frontmatter.
    pub fields: Vec<FieldInfo>,
}

/// Trait for types that can describe their schema.
pub trait Describe {
    /// Returns metadata about this entity's fields and structure.
    fn describe() -> EntityInfo;
}

impl Describe for crate::model::Project {
    fn describe() -> EntityInfo {
        EntityInfo {
            name: "project",
            description: "A project directory grouping roadmaps and tasks.",
            fields: vec![
                FieldInfo {
                    name: "name",
                    type_name: "string",
                    required: true,
                    enum_values: &[],
                    description: "Project slug identifier (used in directory names and references).",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "title",
                    type_name: "string",
                    required: true,
                    enum_values: &[],
                    description: "Human-readable title.",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "source",
                    type_name: "object",
                    required: false,
                    enum_values: &[],
                    description: "Optional source repo ({ repo, default_branch }) that rdm:src/ links in this project's documents resolve against.",
                    ..FieldInfo::default()
                },
            ],
        }
    }
}

impl Describe for crate::model::Roadmap {
    fn describe() -> EntityInfo {
        EntityInfo {
            name: "roadmap",
            description: "An ordered sequence of phases tracking a multi-step initiative.",
            fields: vec![
                FieldInfo {
                    name: "project",
                    type_name: "string",
                    required: true,
                    enum_values: &[],
                    description: "Project this roadmap belongs to.",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "roadmap",
                    type_name: "string",
                    required: true,
                    enum_values: &[],
                    description: "Roadmap slug identifier.",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "title",
                    type_name: "string",
                    required: true,
                    enum_values: &[],
                    description: "Human-readable title.",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "phases",
                    type_name: "list of strings",
                    required: true,
                    enum_values: &[],
                    description: "Ordered list of phase file stems.",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "dependencies",
                    type_name: "list of strings",
                    required: false,
                    enum_values: &[],
                    description: "Roadmap slugs that must complete before this one.",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "priority",
                    type_name: "string",
                    required: false,
                    enum_values: &["low", "medium", "high", "critical"],
                    description: "Optional priority level for the roadmap.",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "tags",
                    type_name: "list of strings",
                    required: false,
                    enum_values: &[],
                    description: "Optional tags for categorization.",
                    ..FieldInfo::default()
                },
            ],
        }
    }
}

impl Describe for crate::model::Phase {
    fn describe() -> EntityInfo {
        EntityInfo {
            name: "phase",
            description: "A single step within a roadmap, tracked by status.",
            fields: vec![
                FieldInfo {
                    name: "phase",
                    type_name: "integer",
                    required: true,
                    enum_values: &[],
                    description: "Phase number (1-based ordering).",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "title",
                    type_name: "string",
                    required: true,
                    enum_values: &[],
                    description: "Human-readable title.",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "status",
                    type_name: "enum",
                    required: true,
                    enum_values: &[
                        "not-started",
                        "in-progress",
                        "needs-review",
                        "reviewed",
                        "done",
                        "blocked",
                        "wont-fix",
                    ],
                    description: "Current status.",
                    ..FieldInfo::default()
                }
                .with_status_metadata(|s: &PhaseStatus| (s.is_terminal(), s.note())),
                FieldInfo {
                    name: "tags",
                    type_name: "list of strings",
                    required: false,
                    enum_values: &[],
                    description: "Optional tags for categorization.",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "completed",
                    type_name: "date (YYYY-MM-DD)",
                    required: false,
                    enum_values: &[],
                    description: "Date the phase was completed, if applicable.",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "commit",
                    type_name: "string",
                    required: false,
                    enum_values: &[],
                    description: "Git commit SHA associated with phase completion, if any.",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "difficulty",
                    type_name: "enum",
                    required: false,
                    enum_values: &["trivial", "easy", "moderate", "hard"],
                    description: "Estimated difficulty of the phase, if assessed.",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "model",
                    type_name: "enum",
                    required: false,
                    enum_values: &["small", "medium", "large", "frontier"],
                    description: "Model tier that should run the phase, if assigned.",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "blocked_reason",
                    type_name: "string",
                    required: false,
                    enum_values: &[],
                    description: "Reason the phase was parked as blocked (an escalation note), if any.",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "gate_override",
                    type_name: "object",
                    required: false,
                    enum_values: &[],
                    description: "An operator's recorded bypass of the reviewed transition gate (reason, actor, date), if one authorized this phase's current status.",
                    ..FieldInfo::default()
                },
            ],
        }
    }
}

impl Describe for crate::model::Task {
    fn describe() -> EntityInfo {
        EntityInfo {
            name: "task",
            description: "A standalone work item tracked by status and priority.",
            fields: vec![
                FieldInfo {
                    name: "project",
                    type_name: "string",
                    required: true,
                    enum_values: &[],
                    description: "Project this task belongs to.",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "title",
                    type_name: "string",
                    required: true,
                    enum_values: &[],
                    description: "Human-readable title.",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "status",
                    type_name: "enum",
                    required: true,
                    enum_values: &[
                        "open",
                        "in-progress",
                        "needs-review",
                        "reviewed",
                        "done",
                        "blocked",
                        "wont-fix",
                    ],
                    description: "Current status.",
                    ..FieldInfo::default()
                }
                .with_status_metadata(|s: &TaskStatus| (s.is_terminal(), s.note())),
                FieldInfo {
                    name: "priority",
                    type_name: "enum",
                    required: true,
                    enum_values: &["low", "medium", "high", "critical"],
                    description: "Priority level.",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "created",
                    type_name: "date (YYYY-MM-DD)",
                    required: true,
                    enum_values: &[],
                    description: "Date the task was created.",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "tags",
                    type_name: "list of strings",
                    required: false,
                    enum_values: &[],
                    description: "Optional tags for categorization.",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "completed",
                    type_name: "date (YYYY-MM-DD)",
                    required: false,
                    enum_values: &[],
                    description: "Date the task was completed.",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "commit",
                    type_name: "string",
                    required: false,
                    enum_values: &[],
                    description: "Git commit SHA that completed this task.",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "close_reason",
                    type_name: "string",
                    required: false,
                    enum_values: &[],
                    description: "Reason the task was closed (a retire/supersede note), if any.",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "gate_override",
                    type_name: "object",
                    required: false,
                    enum_values: &[],
                    description: "An operator's recorded bypass of the reviewed transition gate (reason, actor, date), if one authorized this task's current status.",
                    ..FieldInfo::default()
                },
            ],
        }
    }
}

impl Describe for crate::model::Review {
    fn describe() -> EntityInfo {
        EntityInfo {
            name: "review",
            description: "A structured review of a roadmap, phase, or task, with anchored inline comments.",
            fields: vec![
                FieldInfo {
                    name: "id",
                    type_name: "string",
                    required: true,
                    enum_values: &[],
                    description: "Timestamp-based identifier, unique within the project (also the file stem under reviews/).",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "author",
                    type_name: "string",
                    required: true,
                    enum_values: &[],
                    description: "Who authored the review (free-form: email, agent name, etc.).",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "target",
                    type_name: "tagged mapping (kind: roadmap | phase | task | plan | change)",
                    required: true,
                    enum_values: &[],
                    description: "The plan item, or source-repo change, under review. The CLI reference syntax is roadmap/<slug>, phase/<roadmap-slug>/<stem>, task/<slug>, plan/<slug>, or change/<head-sha>.",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "state",
                    type_name: "enum",
                    required: true,
                    enum_values: &["draft", "submitted", "addressed", "dismissed"],
                    description: "Lifecycle state. Drafts accept comments; submission locks comment structure; addressed and dismissed are terminal.",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "verdict",
                    type_name: "enum",
                    required: false,
                    enum_values: &["approve", "request-changes", "comment"],
                    description: "Overall verdict stamped on submit; absent while the review is a draft.",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "created",
                    type_name: "datetime (RFC 3339)",
                    required: true,
                    enum_values: &[],
                    description: "When the review was started.",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "submitted",
                    type_name: "datetime (RFC 3339)",
                    required: false,
                    enum_values: &[],
                    description: "When the review was submitted; absent on drafts.",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "created_commit",
                    type_name: "string",
                    required: false,
                    enum_values: &[],
                    description: "Plan-repo HEAD when the review started — the version of the target the reviewer saw. Quote anchors are derived against this version.",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "comments",
                    type_name: "list of comment objects",
                    required: true,
                    enum_values: &[],
                    description: "Inline comments: id, optional doc scope (roadmap reviews only), status (see comments.status), optional applied_commit, optional text-quote anchor (quote/prefix/suffix), optional anchor_state on an anchorless comment (whole-document, or why the anchor was lost; absent means not recorded), optional intended_path (the source file an anchorless change-review comment was about), body, and optional reply; JSON output adds target (every comment's coarsest true link: link, level, reason, reason_note, unverified).",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "comments.status",
                    type_name: "enum",
                    required: true,
                    enum_values: &["open", "addressed", "wont-fix"],
                    description: "Each comment's resolution status. addressed and wont-fix are terminal (the comment is closed); each comment's JSON carries closed: true for them, false for open.",
                    ..FieldInfo::default()
                }
                .with_status_metadata(|s: &ReviewCommentStatus| (s.is_terminal(), s.note())),
            ],
        }
    }
}

impl Describe for crate::model::Run {
    fn describe() -> EntityInfo {
        EntityInfo {
            name: "run",
            description: "A record of one autonomous-lane run: which driver drove which roadmap or task, in which session, and each dispatched unit's time window and outcome.",
            fields: vec![
                FieldInfo {
                    name: "id",
                    type_name: "string",
                    required: true,
                    enum_values: &[],
                    description: "Timestamp-based identifier, unique within the project (also the file stem under runs/).",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "project",
                    type_name: "string",
                    required: true,
                    enum_values: &[],
                    description: "Project this run belongs to.",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "driver",
                    type_name: "enum",
                    required: true,
                    enum_values: &["autopilot", "dispatch-phase"],
                    description: "Which lane driver opened the run.",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "roadmap",
                    type_name: "string",
                    required: false,
                    enum_values: &[],
                    description: "Slug of the roadmap the run drives. Exactly one of roadmap / task is present; not validated on load, so a run whose roadmap was deleted still loads.",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "task",
                    type_name: "string",
                    required: false,
                    enum_values: &[],
                    description: "Slug of the task the run drives. Exactly one of roadmap / task is present.",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "session_uuid",
                    type_name: "string",
                    required: false,
                    enum_values: &[],
                    description: "Raw Claude Code session uuid (from --session-uuid, else CLAUDE_CODE_SESSION_ID). Absent when neither was set; such a run cannot be joined to spend.",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "args",
                    type_name: "string",
                    required: false,
                    enum_values: &[],
                    description: "Free-form invocation arguments of the driver.",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "status",
                    type_name: "enum",
                    required: true,
                    enum_values: &["open", "closed", "abandoned"],
                    description: "Lifecycle status. A run is open until closed or abandoned; an open run is reported as incomplete.",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "started",
                    type_name: "datetime (RFC 3339)",
                    required: true,
                    enum_values: &[],
                    description: "When the run was recorded.",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "ended",
                    type_name: "datetime (RFC 3339)",
                    required: false,
                    enum_values: &[],
                    description: "When the run was closed or abandoned; absent while open.",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "stop_reason",
                    type_name: "string",
                    required: false,
                    enum_values: &[],
                    description: "Why the run stopped; absent while open.",
                    ..FieldInfo::default()
                },
                FieldInfo {
                    name: "units",
                    type_name: "list of {unit, attempt, started, ended, outcome}",
                    required: true,
                    enum_values: &[],
                    description: "Dispatched units in start order: phase stem (or task slug), 1-based attempt ordinal per unit, start time, and — once ended — end time and free-form outcome. A unit without an end time is incomplete.",
                    ..FieldInfo::default()
                },
            ],
        }
    }
}

/// Returns entity descriptions for all model types.
#[must_use]
pub fn all_entities() -> Vec<EntityInfo> {
    use crate::model::{Phase, Project, Review, Roadmap, Run, Task};
    vec![
        Project::describe(),
        Roadmap::describe(),
        Phase::describe(),
        Task::describe(),
        Review::describe(),
        Run::describe(),
    ]
}

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

/// Formats all entities as a human-readable list.
#[must_use]
pub fn format_entity_list(entities: &[EntityInfo]) -> String {
    let mut out = String::new();
    out.push_str("Entity types:\n\n");
    for e in entities {
        out.push_str(&format!("  {} — {}\n", e.name, e.description));
    }
    out.push_str("\nUse `rdm describe <entity>` to see fields.\n");
    out
}

/// Formats a single entity as a human-readable field table.
#[must_use]
pub fn format_entity_detail(entity: &EntityInfo) -> String {
    let mut out = String::new();
    out.push_str(&format!("{} — {}\n\n", entity.name, entity.description));
    out.push_str("Fields:\n\n");
    for f in &entity.fields {
        let req = if f.required { "required" } else { "optional" };
        out.push_str(&format!("  {} ({}, {})\n", f.name, f.type_name, req));
        out.push_str(&format!("    {}\n", f.description));
        if !f.enum_values.is_empty() {
            out.push_str(&format!("    values: {}\n", f.enum_values.join(", ")));
        }
        if !f.terminal_values.is_empty() {
            out.push_str(&format!("    terminal: {}\n", f.terminal_values.join(", ")));
        }
        if !f.value_flags.is_empty() {
            out.push_str(&format!(
                "    flags: {}\n",
                format_value_flags(&f.value_flags)
            ));
        }
    }
    out
}

/// Renders value flags as `blocked → --reason, …`.
fn format_value_flags(flags: &[ValueFlagInfo]) -> String {
    flags
        .iter()
        .map(|vf| format!("{} → --{}", vf.value, vf.flag))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Formats all entities as a Markdown list.
#[must_use]
pub fn format_entity_list_md(entities: &[EntityInfo]) -> String {
    let mut out = String::new();
    out.push_str("## Entity Types\n\n");
    for e in entities {
        out.push_str(&format!("- **{}** — {}\n", e.name, e.description));
    }
    out
}

/// Formats a single entity as a Markdown field table.
#[must_use]
pub fn format_entity_detail_md(entity: &EntityInfo) -> String {
    let mut out = String::new();
    out.push_str(&format!("## {}\n\n", entity.name));
    out.push_str(&format!("{}\n\n", entity.description));
    out.push_str("| Field | Type | Required | Values | Description |\n");
    out.push_str("|-------|------|----------|--------|-------------|\n");
    for f in &entity.fields {
        let req = if f.required { "yes" } else { "no" };
        let mut vals = if f.enum_values.is_empty() {
            "—".to_string()
        } else {
            f.enum_values.join(", ")
        };
        if !f.terminal_values.is_empty() {
            vals.push_str(&format!("<br>terminal: {}", f.terminal_values.join(", ")));
        }
        if !f.value_flags.is_empty() {
            vals.push_str(&format!(
                "<br>flags: {}",
                format_value_flags(&f.value_flags)
            ));
        }
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            f.name, f.type_name, req, vals, f.description
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Asserts that the field names returned by `Describe` match the keys
    /// serde produces when serializing a sample value.
    ///
    /// This is a compile-time drift test: if a field is added to the struct
    /// but not to the `Describe` impl (or vice versa), this test fails.
    /// Only top-level `Describe` names are compared: a dotted name (e.g.
    /// `comments.status`) documents a nested field, which serde reports
    /// under its parent key rather than at the top level.
    fn assert_fields_match<T: serde::Serialize + Describe>(sample: &T) {
        let value = serde_yaml::to_value(sample).expect("sample should serialize");
        let serde_keys: HashSet<&str> = value
            .as_mapping()
            .expect("sample should serialize to a mapping")
            .keys()
            .map(|k| k.as_str().expect("keys should be strings"))
            .collect();

        let describe_keys: HashSet<&str> = T::describe()
            .fields
            .iter()
            .map(|f| f.name)
            .filter(|name| !name.contains('.'))
            .collect();

        assert_eq!(
            serde_keys,
            describe_keys,
            "serde keys and Describe fields must match for {}",
            std::any::type_name::<T>()
        );
    }

    #[test]
    fn drift_project() {
        let sample = crate::model::Project {
            name: "test".to_string(),
            title: "Test".to_string(),
            source: Some(crate::model::Source {
                repo: "https://github.com/acme/repo".to_string(),
                default_branch: Some("main".to_string()),
            }),
        };
        assert_fields_match(&sample);
    }

    #[test]
    fn drift_roadmap() {
        let sample = crate::model::Roadmap {
            project: "test".to_string(),
            roadmap: "test-roadmap".to_string(),
            title: "Test Roadmap".to_string(),
            phases: vec!["phase-1-foo".to_string()],
            dependencies: Some(vec!["other".to_string()]),
            priority: Some(crate::model::Priority::High),
            tags: Some(vec!["api".to_string()]),
        };
        assert_fields_match(&sample);
    }

    #[test]
    fn drift_phase() {
        let sample = crate::model::Phase {
            phase: 1,
            title: "Test Phase".to_string(),
            status: crate::model::PhaseStatus::Done,
            tags: Some(vec!["infra".to_string()]),
            completed: Some(chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()),
            commit: Some("abc123".to_string()),
            review_sha: None,
            review_branch: None,
            started_head: None,
            difficulty: Some(crate::model::Difficulty::Hard),
            model: Some(crate::model::ModelTier::Large),
            blocked_reason: Some("ambiguous acceptance criterion".to_string()),
            gate_override: Some(crate::model::GateOverride {
                reason: "operator: hotfix".to_string(),
                actor: "alice".to_string(),
                at: chrono::NaiveDate::from_ymd_opt(2026, 9, 13).unwrap(),
            }),
        };
        assert_fields_match(&sample);
    }

    #[test]
    fn drift_task() {
        let sample = crate::model::Task {
            project: "test".to_string(),
            title: "Test Task".to_string(),
            status: crate::model::TaskStatus::Open,
            priority: crate::model::Priority::High,
            created: chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            tags: Some(vec!["tag1".to_string()]),
            completed: Some(chrono::NaiveDate::from_ymd_opt(2026, 1, 2).unwrap()),
            commit: Some("abc123".to_string()),
            review_sha: None,
            review_branch: None,
            started_head: None,
            close_reason: Some("superseded by task/other".to_string()),
            gate_override: Some(crate::model::GateOverride {
                reason: "operator: hotfix".to_string(),
                actor: "alice".to_string(),
                at: chrono::NaiveDate::from_ymd_opt(2026, 9, 13).unwrap(),
            }),
        };
        assert_fields_match(&sample);
    }

    // Not named `drift_review` like its siblings: in the review domain
    // "drift" means anchor drift, and this test checks serde/Describe
    // field agreement, not anchors.
    #[test]
    fn describe_review_fields_match() {
        let sample = crate::model::Review {
            id: "2026-07-01-1430-a1b2".to_string(),
            author: "ed".to_string(),
            target: crate::model::ReviewTarget::Task {
                slug: "fix-login".to_string(),
            },
            state: crate::model::ReviewState::Submitted,
            verdict: Some(crate::model::Verdict::RequestChanges),
            created: chrono::Utc::now(),
            submitted: Some(chrono::Utc::now()),
            created_commit: Some("abc123".to_string()),
            comments: vec![],
            implements: None,
            change_branch: None,
        };
        assert_fields_match(&sample);
    }

    #[test]
    fn describe_review_comment_status_covers_every_variant() {
        use crate::model::ReviewCommentStatus;
        use std::str::FromStr;
        let entity = crate::model::Review::describe();
        let field = entity
            .fields
            .iter()
            .find(|f| f.name == "comments.status")
            .expect("review describes comments.status");
        let parsed: Vec<ReviewCommentStatus> = field
            .enum_values
            .iter()
            .map(|v| ReviewCommentStatus::from_str(v).expect("value parses"))
            .collect();
        // Exhaustive: adding a variant fails to compile this match until it
        // is added to the list below, and so must be described.
        let variants = |s: ReviewCommentStatus| match s {
            ReviewCommentStatus::Open
            | ReviewCommentStatus::Addressed
            | ReviewCommentStatus::WontFix => {}
        };
        for status in [
            ReviewCommentStatus::Open,
            ReviewCommentStatus::Addressed,
            ReviewCommentStatus::WontFix,
        ] {
            variants(status);
            assert!(parsed.contains(&status), "{status} missing");
        }
        assert_eq!(parsed.len(), 3, "no duplicate or extra values");
    }

    #[test]
    fn describe_run_fields_match() {
        // `roadmap` and `task` are mutually exclusive keys, so one sample of
        // each target kind together covers every described field.
        let sample = |target| crate::model::Run {
            id: "2026-09-24-1530-a1b2".to_string(),
            project: "test".to_string(),
            driver: crate::model::RunDriver::Autopilot,
            target,
            session_uuid: Some("sess".to_string()),
            args: Some("alpha".to_string()),
            status: crate::model::RunStatus::Closed,
            started: chrono::Utc::now(),
            ended: Some(chrono::Utc::now()),
            stop_reason: Some("done".to_string()),
            units: vec![],
        };
        let mut serde_keys = HashSet::new();
        for target in [
            crate::model::RunTarget::Roadmap("alpha".to_string()),
            crate::model::RunTarget::Task("fix".to_string()),
        ] {
            let value = serde_yaml::to_value(sample(target)).unwrap();
            for k in value.as_mapping().unwrap().keys() {
                serde_keys.insert(k.as_str().unwrap().to_string());
            }
        }
        let describe_keys: HashSet<String> = crate::model::Run::describe()
            .fields
            .iter()
            .map(|f| f.name.to_string())
            .collect();
        assert_eq!(serde_keys, describe_keys);
    }

    #[test]
    fn all_entities_returns_six() {
        let entities = all_entities();
        assert_eq!(entities.len(), 6);
        let names: Vec<&str> = entities.iter().map(|e| e.name).collect();
        assert_eq!(
            names,
            vec!["project", "roadmap", "phase", "task", "review", "run"]
        );
    }

    #[test]
    fn format_entity_list_contains_all_names() {
        let entities = all_entities();
        let output = format_entity_list(&entities);
        for e in &entities {
            assert!(output.contains(e.name), "missing entity: {}", e.name);
        }
    }

    #[test]
    fn format_entity_detail_shows_fields() {
        let entity = crate::model::Task::describe();
        let output = format_entity_detail(&entity);
        assert!(output.contains("task"));
        assert!(output.contains("status"));
        assert!(output.contains("priority"));
        assert!(
            output.contains("open, in-progress, needs-review, reviewed, done, blocked, wont-fix")
        );
        assert!(output.contains("    terminal: done, wont-fix\n"));
        assert!(output.contains("    flags: blocked → --reason, wont-fix → --reason\n"));
    }

    /// The field `name` of `entity`.
    fn field(entity: &EntityInfo, name: &str) -> FieldInfo {
        entity
            .fields
            .iter()
            .find(|f| f.name == name)
            .unwrap_or_else(|| panic!("{} describes {name}", entity.name))
            .clone()
    }

    /// Asserts `field`'s enum values are exactly `variants` (as strings), its
    /// `terminal_values` exactly the terminal ones, and its `value_flags`
    /// exactly the ones with a note.
    fn assert_status_metadata<S: std::fmt::Display>(
        field: &FieldInfo,
        variants: &[S],
        meta: impl Fn(&S) -> (bool, Option<crate::model::StatusNote>),
    ) {
        let names: Vec<String> = variants.iter().map(ToString::to_string).collect();
        assert_eq!(field.enum_values, names.as_slice(), "{}", field.name);
        let terminal: Vec<String> = variants
            .iter()
            .filter(|s| meta(s).0)
            .map(ToString::to_string)
            .collect();
        assert_eq!(field.terminal_values, terminal, "{}", field.name);
        let flags: Vec<(String, &str)> = variants
            .iter()
            .filter_map(|s| meta(s).1.map(|n| (s.to_string(), n.flag())))
            .collect();
        let got: Vec<(String, &str)> = field
            .value_flags
            .iter()
            .map(|vf| (vf.value.to_string(), vf.flag))
            .collect();
        assert_eq!(got, flags, "{}", field.name);
    }

    #[test]
    fn phase_status_metadata_matches_the_status_type() {
        use crate::model::PhaseStatus as S;
        // Exhaustive: a new variant fails to compile here until listed.
        let _ = |s: S| match s {
            S::NotStarted
            | S::InProgress
            | S::NeedsReview
            | S::Reviewed
            | S::Done
            | S::Blocked
            | S::WontFix => {}
        };
        let all = [
            S::NotStarted,
            S::InProgress,
            S::NeedsReview,
            S::Reviewed,
            S::Done,
            S::Blocked,
            S::WontFix,
        ];
        let f = field(&crate::model::Phase::describe(), "status");
        assert_status_metadata(&f, &all, |s| (s.is_terminal(), s.note()));
        assert_eq!(f.terminal_values, vec!["done", "wont-fix"]);
    }

    #[test]
    fn task_status_metadata_matches_the_status_type() {
        use crate::model::TaskStatus as S;
        let _ = |s: S| match s {
            S::Open
            | S::InProgress
            | S::NeedsReview
            | S::Reviewed
            | S::Done
            | S::Blocked
            | S::WontFix => {}
        };
        let all = [
            S::Open,
            S::InProgress,
            S::NeedsReview,
            S::Reviewed,
            S::Done,
            S::Blocked,
            S::WontFix,
        ];
        let f = field(&crate::model::Task::describe(), "status");
        assert_status_metadata(&f, &all, |s| (s.is_terminal(), s.note()));
    }

    #[test]
    fn review_comment_status_metadata_matches_the_status_type() {
        use crate::model::ReviewCommentStatus as S;
        let _ = |s: S| match s {
            S::Open | S::Addressed | S::WontFix => {}
        };
        let all = [S::Open, S::Addressed, S::WontFix];
        let f = field(&crate::model::Review::describe(), "comments.status");
        assert_status_metadata(&f, &all, |s| (s.is_terminal(), s.note()));
        assert_eq!(
            f.value_flags,
            vec![ValueFlagInfo {
                value: "addressed",
                flag: "applied-commit"
            }]
        );
    }

    #[test]
    fn non_status_fields_carry_no_status_metadata() {
        for entity in all_entities() {
            for f in &entity.fields {
                if matches!(
                    (entity.name, f.name),
                    ("phase" | "task", "status") | ("review", "comments.status")
                ) {
                    continue;
                }
                assert!(f.terminal_values.is_empty(), "{}.{}", entity.name, f.name);
                assert!(f.value_flags.is_empty(), "{}.{}", entity.name, f.name);
            }
        }
    }

    #[test]
    fn format_entity_list_md_has_heading() {
        let entities = all_entities();
        let output = format_entity_list_md(&entities);
        assert!(output.contains("## Entity Types"));
        assert!(output.contains("**project**"));
    }

    #[test]
    fn format_entity_detail_md_has_table() {
        let entity = crate::model::Phase::describe();
        let output = format_entity_detail_md(&entity);
        assert!(output.contains("## phase"));
        assert!(output.contains("| Field |"));
        assert!(
            output.contains(
                "not-started, in-progress, needs-review, reviewed, done, blocked, wont-fix"
            )
        );
    }
}
