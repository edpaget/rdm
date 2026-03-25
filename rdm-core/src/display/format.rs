/// Terminal formatting functions for roadmaps, phases, tasks, and search results.
///
/// Pure functions — no I/O. These produce human-readable output strings
/// for display in the CLI and MCP server.
use crate::ast;
use crate::document::Document;
use crate::model::{Phase, PhaseStatus, Roadmap, Task};
use crate::search::SearchResult;

/// A roadmap document paired with its phases (stem + phase document).
pub type RoadmapWithPhases = (Document<Roadmap>, Vec<(String, Document<Phase>)>);

/// Navigation context for prev/next phase hints.
pub struct PhaseNav<'a> {
    /// Stem of the previous phase, if any.
    pub prev: Option<&'a str>,
    /// Stem of the next phase, if any.
    pub next: Option<&'a str>,
    /// Parent roadmap slug.
    pub roadmap: &'a str,
    /// Project name.
    pub project: &'a str,
}

/// Formats a roadmap summary with a status table of its phases and optional body.
///
/// Displays the roadmap title, project/slug metadata, phase progress table,
/// and any body content from the document. If the document body is non-empty,
/// it is appended after the phase table (or "No phases yet." message).
///
/// When `project` is `Some`, a navigation hint is appended showing how to
/// drill into individual phases.
pub fn format_roadmap_summary(
    doc: &Document<Roadmap>,
    phases: &[(String, Document<Phase>)],
) -> String {
    let roadmap = &doc.frontmatter;
    let mut d = ast::Document::new();
    d.heading(1, &roadmap.title);
    d.push(ast::Block::BlankLine);
    d.paragraph(&format!(
        "Project: {}  Slug: {}",
        roadmap.project, roadmap.roadmap
    ));

    if phases.is_empty() {
        d.push(ast::Block::BlankLine);
        d.paragraph("No phases yet.");
    } else {
        let done_count = phases
            .iter()
            .filter(|(_, pd)| pd.frontmatter.status == PhaseStatus::Done)
            .count();
        d.paragraph(&format!(
            "Progress: {}/{} phases done",
            done_count,
            phases.len()
        ));
        d.push(ast::Block::BlankLine);

        let headers = vec!["#", "Phase", "Status"]
            .into_iter()
            .map(|h| vec![ast::Inline::text(h)])
            .collect();
        let rows = phases
            .iter()
            .map(|(_, pd)| {
                let fm = &pd.frontmatter;
                vec![
                    vec![ast::Inline::Text(fm.phase.to_string())],
                    vec![ast::Inline::Text(fm.title.clone())],
                    vec![ast::Inline::Text(fm.status.to_string())],
                ]
            })
            .collect();
        d.push(ast::Block::Table { headers, rows });
    }

    if !doc.body.is_empty() {
        d.push(ast::Block::BlankLine);
        d.raw(&doc.body);
    }

    if !phases.is_empty() {
        d.push(ast::Block::BlankLine);
        d.paragraph(&format!(
            "Hint: rdm phase show <stem> --roadmap {} --project {}",
            roadmap.roadmap, roadmap.project
        ));
    }
    d.to_string()
}

/// Formats a single phase detail view with optional prev/next navigation.
pub fn format_phase_detail(
    stem: &str,
    doc: &Document<Phase>,
    nav: Option<&PhaseNav<'_>>,
) -> String {
    let fm = &doc.frontmatter;
    let mut d = ast::Document::new();
    d.heading(1, &format!("Phase {}: {}", fm.phase, fm.title));
    d.push(ast::Block::BlankLine);
    d.paragraph(&format!("Stem: {stem}"));
    d.paragraph(&format!("Status: {}", fm.status));
    if let Some(date) = fm.completed {
        d.paragraph(&format!("Completed: {date}"));
    }
    if let Some(ref sha) = fm.commit {
        d.paragraph(&format!("Commit: {sha}"));
    }
    if !doc.body.is_empty() {
        d.push(ast::Block::BlankLine);
        d.raw(&doc.body);
    }
    if let Some(nav) = nav {
        append_phase_nav(&mut d, nav);
    }
    d.to_string()
}

fn append_phase_nav(d: &mut ast::Document, nav: &PhaseNav<'_>) {
    d.push(ast::Block::BlankLine);
    if let Some(prev) = nav.prev {
        d.paragraph(&format!(
            "Prev: rdm phase show {prev} --roadmap {} --project {}",
            nav.roadmap, nav.project
        ));
    }
    if let Some(next) = nav.next {
        d.paragraph(&format!(
            "Next: rdm phase show {next} --roadmap {} --project {}",
            nav.roadmap, nav.project
        ));
    }
}

/// Formats a list of phases as a table with number, title, status, and stem.
pub fn format_phase_list(phases: &[(String, Document<Phase>)]) -> String {
    let mut doc = ast::Document::new();
    if phases.is_empty() {
        doc.paragraph("No phases yet.");
    } else {
        let headers = vec!["#", "Phase", "Status", "Stem"]
            .into_iter()
            .map(|h| vec![ast::Inline::text(h)])
            .collect();
        let rows = phases
            .iter()
            .map(|(stem, d)| {
                let fm = &d.frontmatter;
                vec![
                    vec![ast::Inline::Text(fm.phase.to_string())],
                    vec![ast::Inline::Text(fm.title.clone())],
                    vec![ast::Inline::Text(fm.status.to_string())],
                    vec![ast::Inline::Text(stem.clone())],
                ]
            })
            .collect();
        doc.push(ast::Block::Table { headers, rows });
    }
    doc.to_string()
}

/// Formats a list of roadmaps with progress summaries.
pub fn format_roadmap_list(entries: &[RoadmapWithPhases]) -> String {
    let mut d = ast::Document::new();
    if entries.is_empty() {
        d.paragraph("No roadmaps found.");
    } else {
        for (roadmap_doc, phases) in entries {
            let rm = &roadmap_doc.frontmatter;
            let done = phases
                .iter()
                .filter(|(_, pd)| pd.frontmatter.status == PhaseStatus::Done)
                .count();
            let total = phases.len();
            if total > 0 {
                d.paragraph(&format!(
                    "{} — {} ({}/{} done)",
                    rm.roadmap, rm.title, done, total
                ));
            } else {
                d.paragraph(&format!("{} — {} (no phases)", rm.roadmap, rm.title));
            }
        }
    }
    d.to_string()
}

/// Formats a single task detail view.
pub fn format_task_detail(slug: &str, doc: &Document<Task>) -> String {
    let fm = &doc.frontmatter;
    let mut d = ast::Document::new();
    d.heading(1, &fm.title);
    d.push(ast::Block::BlankLine);
    d.paragraph(&format!("Slug: {slug}"));
    d.paragraph(&format!("Status: {}", fm.status));
    d.paragraph(&format!("Priority: {}", fm.priority));
    d.paragraph(&format!("Created: {}", fm.created));
    if let Some(completed) = &fm.completed {
        d.paragraph(&format!("Completed: {completed}"));
    }
    if let Some(commit) = &fm.commit {
        d.paragraph(&format!("Commit: {commit}"));
    }
    if let Some(tags) = &fm.tags {
        d.paragraph(&format!("Tags: {}", tags.join(", ")));
    }
    if !doc.body.is_empty() {
        d.push(ast::Block::BlankLine);
        d.raw(&doc.body);
    }
    d.to_string()
}

/// Formats a list of tasks as a table with slug, title, status, and priority columns.
pub fn format_task_list(tasks: &[(String, Document<Task>)]) -> String {
    let mut doc = ast::Document::new();
    if tasks.is_empty() {
        doc.paragraph("No tasks found.");
    } else {
        let headers = vec!["Slug", "Title", "Status", "Priority"]
            .into_iter()
            .map(|h| vec![ast::Inline::text(h)])
            .collect();
        let rows = tasks
            .iter()
            .map(|(slug, d)| {
                let fm = &d.frontmatter;
                vec![
                    vec![ast::Inline::Text(slug.clone())],
                    vec![ast::Inline::Text(fm.title.clone())],
                    vec![ast::Inline::Text(fm.status.to_string())],
                    vec![ast::Inline::Text(fm.priority.to_string())],
                ]
            })
            .collect();
        doc.push(ast::Block::Table { headers, rows });
    }
    doc.to_string()
}

/// Formats a dependency graph as a human-readable list.
///
/// Each entry shows a roadmap and what it depends on.
/// If the graph is empty, returns a message indicating no dependencies.
#[must_use]
pub fn format_dependency_graph(graph: &[(String, Vec<String>)]) -> String {
    let mut d = ast::Document::new();
    if graph.is_empty() {
        d.paragraph("No dependencies found.");
    } else {
        for (slug, deps) in graph {
            d.paragraph(&format!("{slug} → {}", deps.join(", ")));
        }
    }
    d.to_string()
}

/// Formats search results as a ranked text table.
#[must_use]
pub fn format_search_results(results: &[SearchResult]) -> String {
    if results.is_empty() {
        return String::new();
    }

    let headers = vec!["#", "Type", "Title", "Identifier", "Snippet"]
        .into_iter()
        .map(|h| vec![ast::Inline::text(h)])
        .collect();
    let rows = results
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let snippet = truncate_snippet(&r.snippet, 40);
            vec![
                vec![ast::Inline::Text((i + 1).to_string())],
                vec![ast::Inline::Text(r.kind.to_string())],
                vec![ast::Inline::Text(r.title.clone())],
                vec![ast::Inline::Text(r.identifier.clone())],
                vec![ast::Inline::Text(snippet)],
            ]
        })
        .collect();

    let mut d = ast::Document::new();
    d.push(ast::Block::Table { headers, rows });
    d.to_string()
}

/// Truncates a snippet to `max_len` characters, appending "..." if needed.
fn truncate_snippet(s: &str, max_len: usize) -> String {
    let trimmed = s.trim();
    if trimmed.len() <= max_len {
        return trimmed.to_string();
    }
    let mut end = max_len;
    while end > 0 && !trimmed.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &trimmed[..end])
}

/// Formats a roadmap summary as Markdown with heading, bullet metadata, phase table, and body.
#[must_use]
pub fn format_roadmap_summary_md(
    doc: &Document<Roadmap>,
    phases: &[(String, Document<Phase>)],
) -> String {
    let roadmap = &doc.frontmatter;
    let mut out = String::new();
    out.push_str(&format!("# {}\n\n", roadmap.title));
    out.push_str(&format!("- **Project:** {}\n", roadmap.project));
    out.push_str(&format!("- **Slug:** {}\n", roadmap.roadmap));

    if phases.is_empty() {
        out.push_str("\nNo phases yet.\n");
    } else {
        let done_count = phases
            .iter()
            .filter(|(_, d)| d.frontmatter.status == PhaseStatus::Done)
            .count();
        out.push_str(&format!(
            "- **Progress:** {}/{} phases done\n",
            done_count,
            phases.len()
        ));

        out.push_str("\n| # | Phase | Status |\n");
        out.push_str("|---:|-------|--------|\n");
        for (_, d) in phases {
            let fm = &d.frontmatter;
            out.push_str(&format!(
                "| {} | {} | {} |\n",
                fm.phase, fm.title, fm.status
            ));
        }
    }

    if !doc.body.is_empty() {
        out.push_str(&format!("\n{}", doc.body));
    }

    if !phases.is_empty() {
        out.push_str(&format!(
            "\n> Hint: `rdm phase show <stem> --roadmap {} --project {}`\n",
            roadmap.roadmap, roadmap.project
        ));
    }
    out
}

/// Formats a list of roadmaps as a Markdown table.
#[must_use]
pub fn format_roadmap_list_md(entries: &[RoadmapWithPhases]) -> String {
    if entries.is_empty() {
        return "No roadmaps found.\n".to_string();
    }

    let mut out = String::new();
    out.push_str("## Roadmaps\n\n");
    out.push_str("| Slug | Title | Progress |\n");
    out.push_str("|------|-------|----------|\n");
    for (roadmap_doc, phases) in entries {
        let rm = &roadmap_doc.frontmatter;
        let done = phases
            .iter()
            .filter(|(_, doc)| doc.frontmatter.status == PhaseStatus::Done)
            .count();
        let total = phases.len();
        let progress = if total > 0 {
            format!("{done}/{total} done")
        } else {
            "no phases".to_string()
        };
        out.push_str(&format!(
            "| {} | {} | {} |\n",
            rm.roadmap, rm.title, progress
        ));
    }
    out
}

/// Formats a single phase detail as Markdown with heading, bullet metadata, and body.
#[must_use]
pub fn format_phase_detail_md(
    stem: &str,
    doc: &Document<Phase>,
    nav: Option<&PhaseNav<'_>>,
) -> String {
    let fm = &doc.frontmatter;
    let mut out = String::new();
    out.push_str(&format!("# Phase {}: {}\n\n", fm.phase, fm.title));
    out.push_str(&format!("- **Stem:** {stem}\n"));
    out.push_str(&format!("- **Status:** {}\n", fm.status));
    if let Some(date) = fm.completed {
        out.push_str(&format!("- **Completed:** {date}\n"));
    }
    if let Some(ref sha) = fm.commit {
        out.push_str(&format!("- **Commit:** {sha}\n"));
    }
    if !doc.body.is_empty() {
        out.push_str(&format!("\n{}", doc.body));
    }
    if let Some(nav) = nav {
        out.push('\n');
        if let Some(prev) = nav.prev {
            out.push_str(&format!(
                "> Prev: `rdm phase show {prev} --roadmap {} --project {}`\n",
                nav.roadmap, nav.project
            ));
        }
        if let Some(next) = nav.next {
            out.push_str(&format!(
                "> Next: `rdm phase show {next} --roadmap {} --project {}`\n",
                nav.roadmap, nav.project
            ));
        }
    }
    out
}

/// Formats a list of phases as a Markdown table.
#[must_use]
pub fn format_phase_list_md(phases: &[(String, Document<Phase>)]) -> String {
    if phases.is_empty() {
        return "No phases yet.\n".to_string();
    }

    let mut out = String::new();
    out.push_str("## Phases\n\n");
    out.push_str("| # | Phase | Status | Stem |\n");
    out.push_str("|---:|-------|--------|------|\n");
    for (stem, doc) in phases {
        let fm = &doc.frontmatter;
        out.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            fm.phase, fm.title, fm.status, stem
        ));
    }
    out
}

/// Formats a single task detail as Markdown with heading, bullet metadata, and body.
#[must_use]
pub fn format_task_detail_md(slug: &str, doc: &Document<Task>) -> String {
    let fm = &doc.frontmatter;
    let mut out = String::new();
    out.push_str(&format!("# {}\n\n", fm.title));
    out.push_str(&format!("- **Slug:** {slug}\n"));
    out.push_str(&format!("- **Status:** {}\n", fm.status));
    out.push_str(&format!("- **Priority:** {}\n", fm.priority));
    out.push_str(&format!("- **Created:** {}\n", fm.created));
    if let Some(completed) = &fm.completed {
        out.push_str(&format!("- **Completed:** {completed}\n"));
    }
    if let Some(commit) = &fm.commit {
        out.push_str(&format!("- **Commit:** {commit}\n"));
    }
    if let Some(tags) = &fm.tags {
        out.push_str(&format!("- **Tags:** {}\n", tags.join(", ")));
    }
    if !doc.body.is_empty() {
        out.push_str(&format!("\n{}", doc.body));
    }
    out
}

/// Formats a list of tasks as a Markdown table.
#[must_use]
pub fn format_task_list_md(tasks: &[(String, Document<Task>)]) -> String {
    if tasks.is_empty() {
        return "No tasks found.\n".to_string();
    }

    let mut out = String::new();
    out.push_str("## Tasks\n\n");
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

/// Formats search results as a Markdown table.
#[must_use]
pub fn format_search_results_md(results: &[SearchResult]) -> String {
    if results.is_empty() {
        return "No results found.\n".to_string();
    }

    let mut out = String::new();
    out.push_str("## Search Results\n\n");
    out.push_str("| # | Type | Title | Identifier | Snippet |\n");
    out.push_str("|---:|------|-------|------------|---------|\n");
    for (i, r) in results.iter().enumerate() {
        let snippet = truncate_snippet(&r.snippet, 40);
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            i + 1,
            r.kind,
            r.title,
            r.identifier,
            snippet,
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
                commit: None,
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
            priority: None,
        }
    }

    fn make_roadmap_doc(project: &str, slug: &str, title: &str) -> Document<Roadmap> {
        Document {
            frontmatter: make_roadmap(project, slug, title),
            body: String::new(),
        }
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
                completed: None,
                commit: None,
            },
            body: String::new(),
        }
    }

    #[test]
    fn roadmap_summary_with_phases() {
        let doc = make_roadmap_doc("fbm", "two-way", "Two-Way Players");
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
        let output = format_roadmap_summary(&doc, &phases);
        assert!(output.contains("# Two-Way Players"));
        assert!(output.contains("1/2 phases done"));
        assert!(output.contains("| 1 | Core | done |"));
        assert!(output.contains("| 2 | Service | in-progress |"));
    }

    #[test]
    fn roadmap_summary_no_phases() {
        let doc = make_roadmap_doc("fbm", "two-way", "Two-Way Players");
        let output = format_roadmap_summary(&doc, &[]);
        assert!(output.contains("No phases yet."));
    }

    #[test]
    fn roadmap_summary_with_body() {
        let mut doc = make_roadmap_doc("fbm", "two-way", "Two-Way Players");
        doc.body = "## Overview\n\nThis roadmap covers two-way player valuation.\n".to_string();
        let phases = vec![(
            "phase-1-core".to_string(),
            make_phase_doc(1, "Core", PhaseStatus::InProgress),
        )];
        let output = format_roadmap_summary(&doc, &phases);
        assert!(output.contains("| 1 | Core | in-progress |"));
        assert!(output.contains("## Overview"));
        assert!(output.contains("This roadmap covers two-way player valuation."));
    }

    #[test]
    fn roadmap_summary_no_phases_with_body() {
        let mut doc = make_roadmap_doc("fbm", "two-way", "Two-Way Players");
        doc.body = "Some body text.\n".to_string();
        let output = format_roadmap_summary(&doc, &[]);
        assert!(output.contains("No phases yet."));
        assert!(output.contains("Some body text."));
    }

    #[test]
    fn roadmap_summary_empty_body() {
        let doc = make_roadmap_doc("fbm", "two-way", "Two-Way Players");
        let output = format_roadmap_summary(&doc, &[]);
        // Should end with "No phases yet.\n" — no trailing blank line from body
        assert!(output.ends_with("No phases yet.\n"));
    }

    #[test]
    fn phase_detail_with_completed() {
        let doc = make_phase_doc(1, "Core", PhaseStatus::Done);
        let output = format_phase_detail("phase-1-core", &doc, None);
        assert!(output.contains("# Phase 1: Core"));
        assert!(output.contains("Status: done"));
        assert!(output.contains("Completed: 2026-03-14"));
        assert!(output.contains("Stem: phase-1-core"));
    }

    #[test]
    fn phase_detail_without_completed() {
        let doc = make_phase_doc(2, "Service", PhaseStatus::NotStarted);
        let output = format_phase_detail("phase-2-service", &doc, None);
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

    // -- Markdown format tests --

    #[test]
    fn roadmap_summary_md_with_phases() {
        let doc = make_roadmap_doc("fbm", "two-way", "Two-Way Players");
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
        let output = format_roadmap_summary_md(&doc, &phases);
        assert!(output.contains("# Two-Way Players"));
        assert!(output.contains("- **Project:** fbm"));
        assert!(output.contains("- **Slug:** two-way"));
        assert!(output.contains("- **Progress:** 1/2 phases done"));
        assert!(output.contains("|---:"));
        assert!(output.contains("| 1 | Core | done |"));
        assert!(output.contains("| 2 | Service | in-progress |"));
    }

    #[test]
    fn roadmap_summary_md_no_phases() {
        let doc = make_roadmap_doc("fbm", "two-way", "Two-Way Players");
        let output = format_roadmap_summary_md(&doc, &[]);
        assert!(output.contains("No phases yet."));
        assert!(!output.contains("- **Progress:**"));
    }

    #[test]
    fn roadmap_summary_md_with_body() {
        let mut doc = make_roadmap_doc("fbm", "two-way", "Two-Way Players");
        doc.body = "## Overview\n\nDetails here.\n".to_string();
        let phases = vec![(
            "phase-1-core".to_string(),
            make_phase_doc(1, "Core", PhaseStatus::InProgress),
        )];
        let output = format_roadmap_summary_md(&doc, &phases);
        assert!(output.contains("## Overview"));
        assert!(output.contains("Details here."));
    }

    #[test]
    fn roadmap_list_md_with_entries() {
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
        let output = format_roadmap_list_md(&entries);
        assert!(output.contains("## Roadmaps"));
        assert!(output.contains("| Slug |"));
        assert!(output.contains("|---"));
        assert!(output.contains("| alpha | Alpha | 1/2 done |"));
        assert!(output.contains("| beta | Beta | no phases |"));
    }

    #[test]
    fn roadmap_list_md_empty() {
        let output = format_roadmap_list_md(&[]);
        assert_eq!(output, "No roadmaps found.\n");
    }

    #[test]
    fn phase_detail_md_with_completed() {
        let doc = make_phase_doc(1, "Core", PhaseStatus::Done);
        let output = format_phase_detail_md("phase-1-core", &doc, None);
        assert!(output.contains("# Phase 1: Core"));
        assert!(output.contains("- **Stem:** phase-1-core"));
        assert!(output.contains("- **Status:** done"));
        assert!(output.contains("- **Completed:** 2026-03-14"));
    }

    #[test]
    fn phase_detail_md_without_completed() {
        let doc = make_phase_doc(2, "Service", PhaseStatus::NotStarted);
        let output = format_phase_detail_md("phase-2-service", &doc, None);
        assert!(output.contains("- **Status:** not-started"));
        assert!(!output.contains("- **Completed:**"));
    }

    #[test]
    fn phase_detail_md_with_body() {
        let mut doc = make_phase_doc(1, "Core", PhaseStatus::InProgress);
        doc.body = "Implementation details.\n".to_string();
        let output = format_phase_detail_md("phase-1-core", &doc, None);
        assert!(output.contains("Implementation details."));
    }

    #[test]
    fn phase_list_md_with_entries() {
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
        let output = format_phase_list_md(&phases);
        assert!(output.contains("## Phases"));
        assert!(output.contains("|---:"));
        assert!(output.contains("| 1 | Core | done | phase-1-core |"));
        assert!(output.contains("| 2 | Service | in-progress | phase-2-service |"));
    }

    #[test]
    fn phase_list_md_empty() {
        let output = format_phase_list_md(&[]);
        assert_eq!(output, "No phases yet.\n");
    }

    #[test]
    fn task_detail_md_basic() {
        let doc = make_task_doc("Fix the bug", TaskStatus::Open, Priority::High, None);
        let output = format_task_detail_md("fix-bug", &doc);
        assert!(output.contains("# Fix the bug"));
        assert!(output.contains("- **Slug:** fix-bug"));
        assert!(output.contains("- **Status:** open"));
        assert!(output.contains("- **Priority:** high"));
        assert!(output.contains("- **Created:** 2026-03-15"));
        assert!(!output.contains("- **Tags:**"));
    }

    #[test]
    fn task_detail_md_with_tags() {
        let doc = make_task_doc(
            "Fix",
            TaskStatus::Open,
            Priority::Low,
            Some(vec!["bug".to_string(), "urgent".to_string()]),
        );
        let output = format_task_detail_md("fix", &doc);
        assert!(output.contains("- **Tags:** bug, urgent"));
    }

    #[test]
    fn task_detail_md_with_body() {
        let mut doc = make_task_doc("Fix", TaskStatus::Open, Priority::Low, None);
        doc.body = "Some details.\n".to_string();
        let output = format_task_detail_md("fix", &doc);
        assert!(output.contains("Some details."));
    }

    #[test]
    fn task_list_md_with_entries() {
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
        let output = format_task_list_md(&tasks);
        assert!(output.contains("## Tasks"));
        assert!(output.contains("| Slug | Title | Status | Priority |"));
        assert!(output.contains("|---"));
        assert!(output.contains("| fix-bug | Fix Bug | open | high |"));
        assert!(output.contains("| add-feature | Add Feature | in-progress | medium |"));
    }

    #[test]
    fn task_list_md_empty() {
        let output = format_task_list_md(&[]);
        assert_eq!(output, "No tasks found.\n");
    }

    #[test]
    fn search_results_md_with_results() {
        let results = vec![SearchResult {
            kind: crate::search::ItemKind::Task,
            identifier: "fix-bug".to_string(),
            project: "acme".to_string(),
            title: "Fix Bug".to_string(),
            snippet: "Fix the login bug".to_string(),
            score: 100,
        }];
        let output = format_search_results_md(&results);
        assert!(output.contains("## Search Results"));
        assert!(output.contains("|---:"));
        assert!(output.contains("| 1 |"));
        assert!(output.contains("Fix Bug"));
        assert!(output.contains("fix-bug"));
    }

    #[test]
    fn search_results_md_empty() {
        let output = format_search_results_md(&[]);
        assert_eq!(output, "No results found.\n");
    }

    // -- Navigation hint tests --

    #[test]
    fn roadmap_summary_includes_hint_when_phases_present() {
        let doc = make_roadmap_doc("fbm", "two-way", "Two-Way Players");
        let phases = vec![(
            "phase-1-core".to_string(),
            make_phase_doc(1, "Core", PhaseStatus::InProgress),
        )];
        let output = format_roadmap_summary(&doc, &phases);
        assert!(output.contains("Hint: rdm phase show <stem> --roadmap two-way --project fbm"));
    }

    #[test]
    fn roadmap_summary_no_hint_when_no_phases() {
        let doc = make_roadmap_doc("fbm", "two-way", "Two-Way Players");
        let output = format_roadmap_summary(&doc, &[]);
        assert!(!output.contains("Hint:"));
    }

    #[test]
    fn roadmap_summary_md_includes_hint_when_phases_present() {
        let doc = make_roadmap_doc("fbm", "two-way", "Two-Way Players");
        let phases = vec![(
            "phase-1-core".to_string(),
            make_phase_doc(1, "Core", PhaseStatus::InProgress),
        )];
        let output = format_roadmap_summary_md(&doc, &phases);
        assert!(output.contains("Hint:"));
        assert!(output.contains("--roadmap two-way --project fbm"));
    }

    #[test]
    fn roadmap_summary_md_no_hint_when_no_phases() {
        let doc = make_roadmap_doc("fbm", "two-way", "Two-Way Players");
        let output = format_roadmap_summary_md(&doc, &[]);
        assert!(!output.contains("Hint:"));
    }

    #[test]
    fn phase_detail_with_nav_prev_and_next() {
        let doc = make_phase_doc(2, "Service", PhaseStatus::InProgress);
        let nav = PhaseNav {
            prev: Some("phase-1-core"),
            next: Some("phase-3-ui"),
            roadmap: "two-way",
            project: "fbm",
        };
        let output = format_phase_detail("phase-2-service", &doc, Some(&nav));
        assert!(
            output.contains("Prev: rdm phase show phase-1-core --roadmap two-way --project fbm")
        );
        assert!(output.contains("Next: rdm phase show phase-3-ui --roadmap two-way --project fbm"));
    }

    #[test]
    fn phase_detail_first_phase_no_prev() {
        let doc = make_phase_doc(1, "Core", PhaseStatus::Done);
        let nav = PhaseNav {
            prev: None,
            next: Some("phase-2-service"),
            roadmap: "two-way",
            project: "fbm",
        };
        let output = format_phase_detail("phase-1-core", &doc, Some(&nav));
        assert!(!output.contains("Prev:"));
        assert!(output.contains("Next: rdm phase show phase-2-service"));
    }

    #[test]
    fn phase_detail_last_phase_no_next() {
        let doc = make_phase_doc(3, "UI", PhaseStatus::NotStarted);
        let nav = PhaseNav {
            prev: Some("phase-2-service"),
            next: None,
            roadmap: "two-way",
            project: "fbm",
        };
        let output = format_phase_detail("phase-3-ui", &doc, Some(&nav));
        assert!(output.contains("Prev: rdm phase show phase-2-service"));
        assert!(!output.contains("Next:"));
    }

    #[test]
    fn phase_detail_md_with_nav() {
        let doc = make_phase_doc(2, "Service", PhaseStatus::InProgress);
        let nav = PhaseNav {
            prev: Some("phase-1-core"),
            next: Some("phase-3-ui"),
            roadmap: "two-way",
            project: "fbm",
        };
        let output = format_phase_detail_md("phase-2-service", &doc, Some(&nav));
        assert!(output.contains("> Prev:"));
        assert!(output.contains("> Next:"));
        assert!(output.contains("phase-1-core"));
        assert!(output.contains("phase-3-ui"));
    }

    #[test]
    fn phase_detail_no_nav_shows_no_hints() {
        let doc = make_phase_doc(1, "Core", PhaseStatus::Done);
        let output = format_phase_detail("phase-1-core", &doc, None);
        assert!(!output.contains("Prev:"));
        assert!(!output.contains("Next:"));
    }
}
