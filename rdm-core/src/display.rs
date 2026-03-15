/// Display formatting functions for roadmaps, phases, and projects.
///
/// Pure functions — no I/O. These produce human-readable output strings.
use crate::document::Document;
use crate::model::{Phase, PhaseStatus, Roadmap, Task};

/// A roadmap document paired with its phases (stem + phase document).
pub type RoadmapWithPhases = (Document<Roadmap>, Vec<(String, Document<Phase>)>);

/// Formats a roadmap summary with a status table of its phases.
pub fn format_roadmap_summary(roadmap: &Roadmap, phases: &[(String, Document<Phase>)]) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {}\n\n", roadmap.title));
    out.push_str(&format!(
        "Project: {}  Slug: {}\n",
        roadmap.project, roadmap.roadmap
    ));

    if phases.is_empty() {
        out.push_str("\nNo phases yet.\n");
        return out;
    }

    let done_count = phases
        .iter()
        .filter(|(_, doc)| doc.frontmatter.status == PhaseStatus::Done)
        .count();
    out.push_str(&format!(
        "Progress: {}/{} phases done\n\n",
        done_count,
        phases.len()
    ));

    out.push_str("| # | Phase | Status |\n");
    out.push_str("|---|-------|--------|\n");
    for (_, doc) in phases {
        let fm = &doc.frontmatter;
        out.push_str(&format!(
            "| {} | {} | {} |\n",
            fm.phase, fm.title, fm.status
        ));
    }
    out
}

/// Formats a single phase detail view.
pub fn format_phase_detail(stem: &str, doc: &Document<Phase>) -> String {
    let fm = &doc.frontmatter;
    let mut out = String::new();
    out.push_str(&format!("# Phase {}: {}\n\n", fm.phase, fm.title));
    out.push_str(&format!("Stem: {stem}\n"));
    out.push_str(&format!("Status: {}\n", fm.status));
    if let Some(date) = fm.completed {
        out.push_str(&format!("Completed: {date}\n"));
    }
    if !doc.body.is_empty() {
        out.push_str(&format!("\n{}", doc.body));
    }
    out
}

/// Formats a list of phases as a table with number, title, status, and stem.
pub fn format_phase_list(phases: &[(String, Document<Phase>)]) -> String {
    if phases.is_empty() {
        return "No phases yet.\n".to_string();
    }

    let mut out = String::new();
    out.push_str("| # | Phase | Status | Stem |\n");
    out.push_str("|---|-------|--------|------|\n");
    for (stem, doc) in phases {
        let fm = &doc.frontmatter;
        out.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            fm.phase, fm.title, fm.status, stem
        ));
    }
    out
}

/// Formats a list of roadmaps with progress summaries.
pub fn format_roadmap_list(entries: &[RoadmapWithPhases]) -> String {
    if entries.is_empty() {
        return "No roadmaps found.\n".to_string();
    }

    let mut out = String::new();
    for (roadmap_doc, phases) in entries {
        let rm = &roadmap_doc.frontmatter;
        let done = phases
            .iter()
            .filter(|(_, doc)| doc.frontmatter.status == PhaseStatus::Done)
            .count();
        let total = phases.len();
        if total > 0 {
            out.push_str(&format!(
                "{} — {} ({}/{} done)\n",
                rm.roadmap, rm.title, done, total
            ));
        } else {
            out.push_str(&format!("{} — {} (no phases)\n", rm.roadmap, rm.title));
        }
    }
    out
}

/// Formats a single task detail view.
pub fn format_task_detail(slug: &str, doc: &Document<Task>) -> String {
    let fm = &doc.frontmatter;
    let mut out = String::new();
    out.push_str(&format!("# {}\n\n", fm.title));
    out.push_str(&format!("Slug: {slug}\n"));
    out.push_str(&format!("Status: {}\n", fm.status));
    out.push_str(&format!("Priority: {}\n", fm.priority));
    out.push_str(&format!("Created: {}\n", fm.created));
    if let Some(tags) = &fm.tags {
        out.push_str(&format!("Tags: {}\n", tags.join(", ")));
    }
    if !doc.body.is_empty() {
        out.push_str(&format!("\n{}", doc.body));
    }
    out
}

/// Formats a list of tasks as a table with slug, title, status, and priority columns.
pub fn format_task_list(tasks: &[(String, Document<Task>)]) -> String {
    if tasks.is_empty() {
        return "No tasks found.\n".to_string();
    }

    let mut out = String::new();
    out.push_str("| Slug | Title | Status | Priority |\n");
    out.push_str("|------|-------|--------|----------|\n");
    for (slug, doc) in tasks {
        let fm = &doc.frontmatter;
        out.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            slug, fm.title, fm.status, fm.priority
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{PhaseStatus, Priority, TaskStatus};
    use chrono::NaiveDate;

    fn make_phase_doc(num: u32, title: &str, status: PhaseStatus) -> Document<Phase> {
        Document {
            frontmatter: Phase {
                phase: num,
                title: title.to_string(),
                status,
                completed: if status == PhaseStatus::Done {
                    Some(NaiveDate::from_ymd_opt(2026, 3, 14).unwrap())
                } else {
                    None
                },
            },
            body: String::new(),
        }
    }

    fn make_roadmap(project: &str, slug: &str, title: &str) -> Roadmap {
        Roadmap {
            project: project.to_string(),
            roadmap: slug.to_string(),
            title: title.to_string(),
            phases: Vec::new(),
            dependencies: None,
        }
    }

    #[test]
    fn roadmap_summary_with_phases() {
        let rm = make_roadmap("fbm", "two-way", "Two-Way Players");
        let phases = vec![
            (
                "phase-1-core".to_string(),
                make_phase_doc(1, "Core", PhaseStatus::Done),
            ),
            (
                "phase-2-service".to_string(),
                make_phase_doc(2, "Service", PhaseStatus::InProgress),
            ),
        ];
        let output = format_roadmap_summary(&rm, &phases);
        assert!(output.contains("# Two-Way Players"));
        assert!(output.contains("1/2 phases done"));
        assert!(output.contains("| 1 | Core | done |"));
        assert!(output.contains("| 2 | Service | in-progress |"));
    }

    #[test]
    fn roadmap_summary_no_phases() {
        let rm = make_roadmap("fbm", "two-way", "Two-Way Players");
        let output = format_roadmap_summary(&rm, &[]);
        assert!(output.contains("No phases yet."));
    }

    #[test]
    fn phase_detail_with_completed() {
        let doc = make_phase_doc(1, "Core", PhaseStatus::Done);
        let output = format_phase_detail("phase-1-core", &doc);
        assert!(output.contains("# Phase 1: Core"));
        assert!(output.contains("Status: done"));
        assert!(output.contains("Completed: 2026-03-14"));
        assert!(output.contains("Stem: phase-1-core"));
    }

    #[test]
    fn phase_detail_without_completed() {
        let doc = make_phase_doc(2, "Service", PhaseStatus::NotStarted);
        let output = format_phase_detail("phase-2-service", &doc);
        assert!(output.contains("Status: not-started"));
        assert!(!output.contains("Completed:"));
    }

    #[test]
    fn phase_list_with_entries() {
        let phases = vec![
            (
                "phase-1-core".to_string(),
                make_phase_doc(1, "Core", PhaseStatus::Done),
            ),
            (
                "phase-2-service".to_string(),
                make_phase_doc(2, "Service", PhaseStatus::InProgress),
            ),
        ];
        let output = format_phase_list(&phases);
        assert!(output.contains("| # | Phase | Status | Stem |"));
        assert!(output.contains("| 1 | Core | done | phase-1-core |"));
        assert!(output.contains("| 2 | Service | in-progress | phase-2-service |"));
    }

    #[test]
    fn phase_list_empty() {
        let output = format_phase_list(&[]);
        assert_eq!(output, "No phases yet.\n");
    }

    #[test]
    fn roadmap_list_with_entries() {
        let entries = vec![
            (
                Document {
                    frontmatter: make_roadmap("fbm", "alpha", "Alpha"),
                    body: String::new(),
                },
                vec![
                    ("p1".to_string(), make_phase_doc(1, "P1", PhaseStatus::Done)),
                    (
                        "p2".to_string(),
                        make_phase_doc(2, "P2", PhaseStatus::InProgress),
                    ),
                ],
            ),
            (
                Document {
                    frontmatter: make_roadmap("fbm", "beta", "Beta"),
                    body: String::new(),
                },
                Vec::new(),
            ),
        ];
        let output = format_roadmap_list(&entries);
        assert!(output.contains("alpha — Alpha (1/2 done)"));
        assert!(output.contains("beta — Beta (no phases)"));
    }

    #[test]
    fn roadmap_list_empty() {
        let output = format_roadmap_list(&[]);
        assert!(output.contains("No roadmaps found."));
    }

    fn make_task_doc(
        title: &str,
        status: TaskStatus,
        priority: Priority,
        tags: Option<Vec<String>>,
    ) -> Document<Task> {
        Document {
            frontmatter: Task {
                project: "fbm".to_string(),
                title: title.to_string(),
                status,
                priority,
                created: NaiveDate::from_ymd_opt(2026, 3, 15).unwrap(),
                tags,
            },
            body: String::new(),
        }
    }

    #[test]
    fn task_detail_basic() {
        let doc = make_task_doc("Fix the bug", TaskStatus::Open, Priority::High, None);
        let output = format_task_detail("fix-bug", &doc);
        assert!(output.contains("# Fix the bug"));
        assert!(output.contains("Slug: fix-bug"));
        assert!(output.contains("Status: open"));
        assert!(output.contains("Priority: high"));
        assert!(output.contains("Created: 2026-03-15"));
        assert!(!output.contains("Tags:"));
    }

    #[test]
    fn task_detail_with_tags() {
        let doc = make_task_doc(
            "Fix",
            TaskStatus::Open,
            Priority::Low,
            Some(vec!["bug".to_string(), "urgent".to_string()]),
        );
        let output = format_task_detail("fix", &doc);
        assert!(output.contains("Tags: bug, urgent"));
    }

    #[test]
    fn task_detail_with_body() {
        let mut doc = make_task_doc("Fix", TaskStatus::Open, Priority::Low, None);
        doc.body = "Some details.\n".to_string();
        let output = format_task_detail("fix", &doc);
        assert!(output.contains("Some details."));
    }

    #[test]
    fn task_list_with_entries() {
        let tasks = vec![
            (
                "fix-bug".to_string(),
                make_task_doc("Fix Bug", TaskStatus::Open, Priority::High, None),
            ),
            (
                "add-feature".to_string(),
                make_task_doc(
                    "Add Feature",
                    TaskStatus::InProgress,
                    Priority::Medium,
                    None,
                ),
            ),
        ];
        let output = format_task_list(&tasks);
        assert!(output.contains("| Slug | Title | Status | Priority |"));
        assert!(output.contains("| fix-bug | Fix Bug | open | high |"));
        assert!(output.contains("| add-feature | Add Feature | in-progress | medium |"));
    }

    #[test]
    fn task_list_empty() {
        let output = format_task_list(&[]);
        assert_eq!(output, "No tasks found.\n");
    }
}
