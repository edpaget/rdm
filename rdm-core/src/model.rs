/// Data model types for roadmaps, phases, and tasks.
use std::fmt;
use std::str::FromStr;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// Status of a roadmap phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PhaseStatus {
    /// Work has not yet begun.
    NotStarted,
    /// Work is actively underway.
    InProgress,
    /// Phase is complete.
    Done,
    /// Phase is blocked by an external dependency.
    Blocked,
}

impl fmt::Display for PhaseStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PhaseStatus::NotStarted => write!(f, "not-started"),
            PhaseStatus::InProgress => write!(f, "in-progress"),
            PhaseStatus::Done => write!(f, "done"),
            PhaseStatus::Blocked => write!(f, "blocked"),
        }
    }
}

impl FromStr for PhaseStatus {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "not-started" => Ok(PhaseStatus::NotStarted),
            "in-progress" => Ok(PhaseStatus::InProgress),
            "done" => Ok(PhaseStatus::Done),
            "blocked" => Ok(PhaseStatus::Blocked),
            other => Err(format!(
                "invalid phase status: '{other}' (expected not-started, in-progress, done, or blocked)"
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
    /// Task is complete.
    Done,
    /// Task was closed without completing.
    WontFix,
}

impl fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TaskStatus::Open => write!(f, "open"),
            TaskStatus::InProgress => write!(f, "in-progress"),
            TaskStatus::Done => write!(f, "done"),
            TaskStatus::WontFix => write!(f, "wont-fix"),
        }
    }
}

impl FromStr for TaskStatus {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "open" => Ok(TaskStatus::Open),
            "in-progress" => Ok(TaskStatus::InProgress),
            "done" => Ok(TaskStatus::Done),
            "wont-fix" => Ok(TaskStatus::WontFix),
            other => Err(format!(
                "invalid task status: '{other}' (expected open, in-progress, done, or wont-fix)"
            )),
        }
    }
}

/// Priority level for a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "low" => Ok(Priority::Low),
            "medium" => Ok(Priority::Medium),
            "high" => Ok(Priority::High),
            "critical" => Ok(Priority::Critical),
            other => Err(format!(
                "invalid priority: '{other}' (expected low, medium, high, or critical)"
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
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "all" => Ok(TaskStatusFilter::All),
            other => other.parse::<TaskStatus>().map(TaskStatusFilter::Status),
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
    /// Date the phase was completed, if applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed: Option<NaiveDate>,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_status_display_from_str_round_trip() {
        let variants = [
            (PhaseStatus::NotStarted, "not-started"),
            (PhaseStatus::InProgress, "in-progress"),
            (PhaseStatus::Done, "done"),
            (PhaseStatus::Blocked, "blocked"),
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
    fn task_status_display_from_str_round_trip() {
        let variants = [
            (TaskStatus::Open, "open"),
            (TaskStatus::InProgress, "in-progress"),
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
            (PhaseStatus::Done, "done"),
            (PhaseStatus::Blocked, "blocked"),
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
"#;
        let phase: Phase = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(phase.phase, 1);
        assert_eq!(phase.title, "Core valuation layer");
        assert_eq!(phase.status, PhaseStatus::Done);
        assert_eq!(
            phase.completed,
            Some(NaiveDate::from_ymd_opt(2026, 3, 13).unwrap())
        );
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
    fn naive_date_serializes_as_yyyy_mm_dd() {
        let date = NaiveDate::from_ymd_opt(2026, 3, 14).unwrap();
        let yaml = serde_yaml::to_string(&date).unwrap();
        assert_eq!(yaml.trim(), "2026-03-14");
    }
}
