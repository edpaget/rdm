/// Terminal and Markdown formatting functions for roadmaps, phases, tasks, and
/// search results.
///
/// Pure functions — no I/O. Each view has a single `build_<view>` builder that
/// constructs an [`ast::Document`], parameterized by a [`RenderFlavor`]; the
/// public `format_<view>` / `format_<view>_md` entry points are thin wrappers
/// that pick the flavor and render to a string.
use crate::ast;
use crate::display::truncate_snippet;
use crate::document::Document;
use crate::model::{Phase, Roadmap, Task};
use crate::search::SearchResult;

/// A roadmap document paired with its phases (stem + phase document).
pub type RoadmapWithPhases = (Document<Roadmap>, Vec<(String, Document<Phase>)>);

/// Which output dialect a builder renders.
///
/// The two flavors differ in metadata style (combined paragraphs vs.
/// `- **bold:**` bullets), section headers (absent vs. `## Section`), table
/// alignment (plain vs. right-aligned `#` column), and the roadmap-list
/// progress wording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderFlavor {
    /// Human-readable terminal output.
    Terminal,
    /// Markdown output.
    Markdown,
}

/// Builds a `- **Label:** value` bullet item for a Markdown metadata list.
fn meta_bullet(label: &str, value: &str) -> Vec<ast::Inline> {
    vec![
        ast::Inline::bold(&format!("{label}:")),
        ast::Inline::text(&format!(" {value}")),
    ]
}

/// Builds header cells from plain-text column labels.
fn header_cells(labels: &[&str]) -> Vec<Vec<ast::Inline>> {
    labels.iter().map(|h| vec![ast::Inline::text(h)]).collect()
}

/// Right-align the first (`#`) column in Markdown; plain in the terminal.
fn first_col_right(flavor: RenderFlavor) -> Vec<ast::Alignment> {
    match flavor {
        RenderFlavor::Markdown => vec![ast::Alignment::Right],
        RenderFlavor::Terminal => vec![],
    }
}

/// Builds a roadmap summary document with a phase table and optional body.
///
/// Shared between flavors: the optional-field presence checks, done-count,
/// table rows, and body append. The metadata style, progress style, and table
/// alignment branch on `flavor`. When `revision` is `Some`, a revision line is
/// rendered near the top to signal a historical view.
fn build_roadmap_summary(
    doc: &Document<Roadmap>,
    phases: &[(String, Document<Phase>)],
    revision: Option<&str>,
    flavor: RenderFlavor,
) -> ast::Document {
    let roadmap = &doc.frontmatter;
    let done_count = phases
        .iter()
        .filter(|(_, pd)| pd.frontmatter.status.is_terminal())
        .count();
    let total = phases.len();

    let mut d = ast::Document::new();
    d.heading(1, &roadmap.title);
    d.push(ast::Block::BlankLine);

    match flavor {
        RenderFlavor::Markdown => {
            let mut items = vec![
                meta_bullet("Project", &roadmap.project),
                meta_bullet("Slug", &roadmap.roadmap),
            ];
            if let Some(sha) = revision {
                items.push(meta_bullet("Revision", sha));
            }
            if let Some(priority) = roadmap.priority {
                items.push(meta_bullet("Priority", &priority.to_string()));
            }
            if let Some(tags) = &roadmap.tags {
                items.push(meta_bullet("Tags", &tags.join(", ")));
            }
            if !phases.is_empty() {
                items.push(meta_bullet(
                    "Progress",
                    &format!("{done_count}/{total} phases done"),
                ));
            }
            d.push(ast::Block::UnorderedList { items });
        }
        RenderFlavor::Terminal => {
            d.paragraph(&format!(
                "Project: {}  Slug: {}",
                roadmap.project, roadmap.roadmap
            ));
            if let Some(sha) = revision {
                d.paragraph(&format!("Revision: {sha}"));
            }
            if let Some(priority) = roadmap.priority {
                d.paragraph(&format!("Priority: {priority}"));
            }
            if let Some(tags) = &roadmap.tags {
                d.paragraph(&format!("Tags: {}", tags.join(", ")));
            }
            if !phases.is_empty() {
                d.paragraph(&format!("Progress: {done_count}/{total} phases done"));
            }
        }
    }

    if phases.is_empty() {
        d.push(ast::Block::BlankLine);
        d.paragraph("No phases yet.");
    } else {
        d.push(ast::Block::BlankLine);
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
        d.push(ast::Block::Table {
            headers: header_cells(&["#", "Phase", "Status"]),
            rows,
            aligns: first_col_right(flavor),
        });
    }

    if !doc.body.is_empty() {
        d.push(ast::Block::BlankLine);
        d.raw(&doc.body);
    }

    d
}

/// Formats a roadmap summary with a status table of its phases and optional body.
///
/// Displays the roadmap title, project/slug metadata, phase progress table,
/// and any body content from the document. If the document body is non-empty,
/// it is appended after the phase table (or "No phases yet." message).
///
/// When `revision` is `Some`, a `Revision: <sha>` line is rendered near the
/// top to signal a historical view.
pub fn format_roadmap_summary(
    doc: &Document<Roadmap>,
    phases: &[(String, Document<Phase>)],
    revision: Option<&str>,
) -> String {
    build_roadmap_summary(doc, phases, revision, RenderFlavor::Terminal).to_string()
}

/// Formats a roadmap summary as Markdown with heading, bullet metadata, phase table, and body.
///
/// When `revision` is `Some`, a `- **Revision:** <sha>` bullet is rendered
/// near the top of the output to signal a historical view.
#[must_use]
pub fn format_roadmap_summary_md(
    doc: &Document<Roadmap>,
    phases: &[(String, Document<Phase>)],
    revision: Option<&str>,
) -> String {
    build_roadmap_summary(doc, phases, revision, RenderFlavor::Markdown).to_string()
}

/// Builds a single phase detail document.
///
/// Shared between flavors: the optional-field presence checks and body append.
/// The metadata style branches on `flavor`. When `revision` is `Some`, a
/// revision line is rendered near the top to signal a historical view.
fn build_phase_detail(
    stem: &str,
    doc: &Document<Phase>,
    revision: Option<&str>,
    flavor: RenderFlavor,
) -> ast::Document {
    let fm = &doc.frontmatter;
    let mut d = ast::Document::new();
    d.heading(1, &format!("Phase {}: {}", fm.phase, fm.title));
    d.push(ast::Block::BlankLine);

    match flavor {
        RenderFlavor::Markdown => {
            let mut items = vec![meta_bullet("Stem", stem)];
            if let Some(sha) = revision {
                items.push(meta_bullet("Revision", sha));
            }
            items.push(meta_bullet("Status", &fm.status.to_string()));
            if let Some(date) = fm.completed {
                items.push(meta_bullet("Completed", &date.to_string()));
            }
            if let Some(ref sha) = fm.commit {
                items.push(meta_bullet("Commit", sha));
            }
            if let Some(tags) = &fm.tags {
                items.push(meta_bullet("Tags", &tags.join(", ")));
            }
            d.push(ast::Block::UnorderedList { items });
        }
        RenderFlavor::Terminal => {
            d.paragraph(&format!("Stem: {stem}"));
            if let Some(sha) = revision {
                d.paragraph(&format!("Revision: {sha}"));
            }
            d.paragraph(&format!("Status: {}", fm.status));
            if let Some(date) = fm.completed {
                d.paragraph(&format!("Completed: {date}"));
            }
            if let Some(ref sha) = fm.commit {
                d.paragraph(&format!("Commit: {sha}"));
            }
            if let Some(tags) = &fm.tags {
                d.paragraph(&format!("Tags: {}", tags.join(", ")));
            }
        }
    }

    if !doc.body.is_empty() {
        d.push(ast::Block::BlankLine);
        d.raw(&doc.body);
    }

    d
}

/// Formats a single phase detail view.
///
/// When `revision` is `Some`, a `Revision: <sha>` line is rendered near
/// the top of the output to signal a historical view.
pub fn format_phase_detail(stem: &str, doc: &Document<Phase>, revision: Option<&str>) -> String {
    build_phase_detail(stem, doc, revision, RenderFlavor::Terminal).to_string()
}

/// Formats a single phase detail as Markdown with heading, bullet metadata, and body.
///
/// When `revision` is `Some`, a `- **Revision:** <sha>` bullet is rendered
/// near the top of the output to signal a historical view.
#[must_use]
pub fn format_phase_detail_md(stem: &str, doc: &Document<Phase>, revision: Option<&str>) -> String {
    build_phase_detail(stem, doc, revision, RenderFlavor::Markdown).to_string()
}

/// Builds a phase list document (table of number, title, status, stem).
fn build_phase_list(phases: &[(String, Document<Phase>)], flavor: RenderFlavor) -> ast::Document {
    let mut d = ast::Document::new();
    if phases.is_empty() {
        d.paragraph("No phases yet.");
        return d;
    }
    if flavor == RenderFlavor::Markdown {
        d.heading(2, "Phases");
        d.push(ast::Block::BlankLine);
    }
    let rows = phases
        .iter()
        .map(|(stem, pd)| {
            let fm = &pd.frontmatter;
            vec![
                vec![ast::Inline::Text(fm.phase.to_string())],
                vec![ast::Inline::Text(fm.title.clone())],
                vec![ast::Inline::Text(fm.status.to_string())],
                vec![ast::Inline::Text(stem.clone())],
            ]
        })
        .collect();
    d.push(ast::Block::Table {
        headers: header_cells(&["#", "Phase", "Status", "Stem"]),
        rows,
        aligns: first_col_right(flavor),
    });
    d
}

/// Formats a list of phases as a table with number, title, status, and stem.
pub fn format_phase_list(phases: &[(String, Document<Phase>)]) -> String {
    build_phase_list(phases, RenderFlavor::Terminal).to_string()
}

/// Formats a list of phases as a Markdown table.
#[must_use]
pub fn format_phase_list_md(phases: &[(String, Document<Phase>)]) -> String {
    build_phase_list(phases, RenderFlavor::Markdown).to_string()
}

/// Builds a roadmap list document.
///
/// The terminal flavor renders one progress paragraph per roadmap (keeping its
/// distinct parenthesized `(N/M done)` / `(no phases)` wording); the Markdown
/// flavor renders a table whose progress column uses
/// [`roadmap_progress_label`](crate::display::roadmap_progress_label).
fn build_roadmap_list(entries: &[RoadmapWithPhases], flavor: RenderFlavor) -> ast::Document {
    let mut d = ast::Document::new();
    if entries.is_empty() {
        d.paragraph("No roadmaps found.");
        return d;
    }
    match flavor {
        RenderFlavor::Terminal => {
            for (roadmap_doc, phases) in entries {
                let rm = &roadmap_doc.frontmatter;
                let done = phases
                    .iter()
                    .filter(|(_, pd)| pd.frontmatter.status.is_terminal())
                    .count();
                let total = phases.len();
                let priority_tag = rm.priority.map(|p| format!(" [{p}]")).unwrap_or_default();
                if total > 0 {
                    d.paragraph(&format!(
                        "{} — {} ({}/{} done){priority_tag}",
                        rm.roadmap, rm.title, done, total
                    ));
                } else {
                    d.paragraph(&format!(
                        "{} — {} (no phases){priority_tag}",
                        rm.roadmap, rm.title
                    ));
                }
            }
        }
        RenderFlavor::Markdown => {
            d.heading(2, "Roadmaps");
            d.push(ast::Block::BlankLine);
            let rows = entries
                .iter()
                .map(|(roadmap_doc, phases)| {
                    let rm = &roadmap_doc.frontmatter;
                    let done = phases
                        .iter()
                        .filter(|(_, pd)| pd.frontmatter.status.is_terminal())
                        .count();
                    let progress = crate::display::roadmap_progress_label(done, phases.len());
                    let priority = rm.priority.map(|p| p.to_string()).unwrap_or_default();
                    vec![
                        vec![ast::Inline::Text(rm.roadmap.clone())],
                        vec![ast::Inline::Text(rm.title.clone())],
                        vec![ast::Inline::Text(progress)],
                        vec![ast::Inline::Text(priority)],
                    ]
                })
                .collect();
            d.push(ast::Block::Table {
                headers: header_cells(&["Slug", "Title", "Progress", "Priority"]),
                rows,
                aligns: vec![],
            });
        }
    }
    d
}

/// Formats a list of roadmaps with progress summaries.
pub fn format_roadmap_list(entries: &[RoadmapWithPhases]) -> String {
    build_roadmap_list(entries, RenderFlavor::Terminal).to_string()
}

/// Formats a list of roadmaps as a Markdown table.
#[must_use]
pub fn format_roadmap_list_md(entries: &[RoadmapWithPhases]) -> String {
    build_roadmap_list(entries, RenderFlavor::Markdown).to_string()
}

/// Builds a single task detail document.
fn build_task_detail(
    slug: &str,
    doc: &Document<Task>,
    revision: Option<&str>,
    flavor: RenderFlavor,
) -> ast::Document {
    let fm = &doc.frontmatter;
    let mut d = ast::Document::new();
    d.heading(1, &fm.title);
    d.push(ast::Block::BlankLine);

    match flavor {
        RenderFlavor::Markdown => {
            let mut items = vec![meta_bullet("Slug", slug)];
            if let Some(sha) = revision {
                items.push(meta_bullet("Revision", sha));
            }
            items.push(meta_bullet("Status", &fm.status.to_string()));
            items.push(meta_bullet("Priority", &fm.priority.to_string()));
            items.push(meta_bullet("Created", &fm.created.to_string()));
            if let Some(completed) = &fm.completed {
                items.push(meta_bullet("Completed", &completed.to_string()));
            }
            if let Some(commit) = &fm.commit {
                items.push(meta_bullet("Commit", commit));
            }
            if let Some(tags) = &fm.tags {
                items.push(meta_bullet("Tags", &tags.join(", ")));
            }
            d.push(ast::Block::UnorderedList { items });
        }
        RenderFlavor::Terminal => {
            d.paragraph(&format!("Slug: {slug}"));
            if let Some(sha) = revision {
                d.paragraph(&format!("Revision: {sha}"));
            }
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
        }
    }

    if !doc.body.is_empty() {
        d.push(ast::Block::BlankLine);
        d.raw(&doc.body);
    }

    d
}

/// Formats a single task detail view.
///
/// When `revision` is `Some`, a `Revision: <sha>` line is rendered near
/// the top of the output to signal a historical view.
pub fn format_task_detail(slug: &str, doc: &Document<Task>, revision: Option<&str>) -> String {
    build_task_detail(slug, doc, revision, RenderFlavor::Terminal).to_string()
}

/// Formats a single task detail as Markdown with heading, bullet metadata, and body.
///
/// When `revision` is `Some`, a `- **Revision:** <sha>` bullet is rendered
/// near the top of the output to signal a historical view.
#[must_use]
pub fn format_task_detail_md(slug: &str, doc: &Document<Task>, revision: Option<&str>) -> String {
    build_task_detail(slug, doc, revision, RenderFlavor::Markdown).to_string()
}

/// Builds a task list document (table of slug, title, status, priority).
fn build_task_list(tasks: &[(String, Document<Task>)], flavor: RenderFlavor) -> ast::Document {
    let mut d = ast::Document::new();
    if tasks.is_empty() {
        d.paragraph("No tasks found.");
        return d;
    }
    if flavor == RenderFlavor::Markdown {
        d.heading(2, "Tasks");
        d.push(ast::Block::BlankLine);
    }
    let rows = tasks
        .iter()
        .map(|(slug, td)| {
            let fm = &td.frontmatter;
            vec![
                vec![ast::Inline::Text(slug.clone())],
                vec![ast::Inline::Text(fm.title.clone())],
                vec![ast::Inline::Text(fm.status.to_string())],
                vec![ast::Inline::Text(fm.priority.to_string())],
            ]
        })
        .collect();
    d.push(ast::Block::Table {
        headers: header_cells(&["Slug", "Title", "Status", "Priority"]),
        rows,
        aligns: vec![],
    });
    d
}

/// Formats a list of tasks as a table with slug, title, status, and priority columns.
pub fn format_task_list(tasks: &[(String, Document<Task>)]) -> String {
    build_task_list(tasks, RenderFlavor::Terminal).to_string()
}

/// Formats a list of tasks as a Markdown table.
#[must_use]
pub fn format_task_list_md(tasks: &[(String, Document<Task>)]) -> String {
    build_task_list(tasks, RenderFlavor::Markdown).to_string()
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

/// Builds a search results document.
///
/// The terminal flavor renders an empty document for no results (the CLI prints
/// nothing); the Markdown flavor renders a `No results found.` line.
fn build_search_results(results: &[SearchResult], flavor: RenderFlavor) -> ast::Document {
    let mut d = ast::Document::new();
    if results.is_empty() {
        if flavor == RenderFlavor::Markdown {
            d.paragraph("No results found.");
        }
        return d;
    }
    if flavor == RenderFlavor::Markdown {
        d.heading(2, "Search Results");
        d.push(ast::Block::BlankLine);
    }
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
    d.push(ast::Block::Table {
        headers: header_cells(&["#", "Type", "Title", "Identifier", "Snippet"]),
        rows,
        aligns: first_col_right(flavor),
    });
    d
}

/// Formats search results as a ranked text table.
#[must_use]
pub fn format_search_results(results: &[SearchResult]) -> String {
    build_search_results(results, RenderFlavor::Terminal).to_string()
}

/// Formats search results as a Markdown table.
#[must_use]
pub fn format_search_results_md(results: &[SearchResult]) -> String {
    build_search_results(results, RenderFlavor::Markdown).to_string()
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
                tags: None,
                completed: if status.is_terminal() {
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
            tags: None,
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
        let output = format_roadmap_summary(&doc, &phases, None);
        assert!(output.contains("# Two-Way Players"));
        assert!(output.contains("1/2 phases done"));
        assert!(output.contains("| 1 | Core | done |"));
        assert!(output.contains("| 2 | Service | in-progress |"));
    }

    #[test]
    fn roadmap_summary_counts_wont_fix_as_done() {
        let doc = make_roadmap_doc("fbm", "two-way", "Two-Way Players");
        let phases = vec![
            (
                "phase-1-core".to_string(),
                make_phase_doc(1, "Core", PhaseStatus::Done),
            ),
            (
                "phase-2-service".to_string(),
                make_phase_doc(2, "Service", PhaseStatus::WontFix),
            ),
        ];
        let output = format_roadmap_summary(&doc, &phases, None);
        assert!(output.contains("2/2 phases done"));
    }

    #[test]
    fn roadmap_summary_no_phases() {
        let doc = make_roadmap_doc("fbm", "two-way", "Two-Way Players");
        let output = format_roadmap_summary(&doc, &[], None);
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
        let output = format_roadmap_summary(&doc, &phases, None);
        assert!(output.contains("| 1 | Core | in-progress |"));
        assert!(output.contains("## Overview"));
        assert!(output.contains("This roadmap covers two-way player valuation."));
    }

    #[test]
    fn roadmap_summary_no_phases_with_body() {
        let mut doc = make_roadmap_doc("fbm", "two-way", "Two-Way Players");
        doc.body = "Some body text.\n".to_string();
        let output = format_roadmap_summary(&doc, &[], None);
        assert!(output.contains("No phases yet."));
        assert!(output.contains("Some body text."));
    }

    #[test]
    fn roadmap_summary_empty_body() {
        let doc = make_roadmap_doc("fbm", "two-way", "Two-Way Players");
        let output = format_roadmap_summary(&doc, &[], None);
        // Should end with "No phases yet.\n" — no trailing blank line from body
        assert!(output.ends_with("No phases yet.\n"));
    }

    #[test]
    fn roadmap_summary_with_tags() {
        let mut doc = make_roadmap_doc("fbm", "tagged", "Tagged");
        doc.frontmatter.tags = Some(vec!["api".to_string(), "mcp".to_string()]);
        let output = format_roadmap_summary(&doc, &[], None);
        assert!(output.contains("Tags: api, mcp"));
    }

    #[test]
    fn roadmap_summary_without_tags_omits_line() {
        let doc = make_roadmap_doc("fbm", "untagged", "Untagged");
        let output = format_roadmap_summary(&doc, &[], None);
        assert!(!output.contains("Tags:"));
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
    fn phase_detail_with_tags() {
        let mut doc = make_phase_doc(1, "Core", PhaseStatus::NotStarted);
        doc.frontmatter.tags = Some(vec!["infra".to_string(), "search".to_string()]);
        let output = format_phase_detail("phase-1-core", &doc, None);
        assert!(output.contains("Tags: infra, search"));
    }

    #[test]
    fn phase_detail_without_tags_omits_line() {
        let doc = make_phase_doc(1, "Core", PhaseStatus::NotStarted);
        let output = format_phase_detail("phase-1-core", &doc, None);
        assert!(!output.contains("Tags:"));
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
        let output = format_task_detail("fix-bug", &doc, None);
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
        let output = format_task_detail("fix", &doc, None);
        assert!(output.contains("Tags: bug, urgent"));
    }

    #[test]
    fn task_detail_with_body() {
        let mut doc = make_task_doc("Fix", TaskStatus::Open, Priority::Low, None);
        doc.body = "Some details.\n".to_string();
        let output = format_task_detail("fix", &doc, None);
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
        let output = format_roadmap_summary_md(&doc, &phases, None);
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
        let output = format_roadmap_summary_md(&doc, &[], None);
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
        let output = format_roadmap_summary_md(&doc, &phases, None);
        assert!(output.contains("## Overview"));
        assert!(output.contains("Details here."));
    }

    #[test]
    fn roadmap_summary_md_with_tags() {
        let mut doc = make_roadmap_doc("fbm", "tagged", "Tagged");
        doc.frontmatter.tags = Some(vec!["api".to_string(), "mcp".to_string()]);
        let output = format_roadmap_summary_md(&doc, &[], None);
        assert!(output.contains("- **Tags:** api, mcp"));
    }

    #[test]
    fn roadmap_summary_md_without_tags_omits_line() {
        let doc = make_roadmap_doc("fbm", "untagged", "Untagged");
        let output = format_roadmap_summary_md(&doc, &[], None);
        assert!(!output.contains("**Tags:**"));
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
    fn phase_detail_md_with_tags() {
        let mut doc = make_phase_doc(1, "Core", PhaseStatus::NotStarted);
        doc.frontmatter.tags = Some(vec!["infra".to_string(), "search".to_string()]);
        let output = format_phase_detail_md("phase-1-core", &doc, None);
        assert!(output.contains("- **Tags:** infra, search"));
    }

    #[test]
    fn phase_detail_md_without_tags_omits_line() {
        let doc = make_phase_doc(1, "Core", PhaseStatus::NotStarted);
        let output = format_phase_detail_md("phase-1-core", &doc, None);
        assert!(!output.contains("**Tags:**"));
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
        let output = format_task_detail_md("fix-bug", &doc, None);
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
        let output = format_task_detail_md("fix", &doc, None);
        assert!(output.contains("- **Tags:** bug, urgent"));
    }

    #[test]
    fn task_detail_md_with_body() {
        let mut doc = make_task_doc("Fix", TaskStatus::Open, Priority::Low, None);
        doc.body = "Some details.\n".to_string();
        let output = format_task_detail_md("fix", &doc, None);
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
    fn roadmap_summary_omits_cli_hint() {
        // CLI hint vocabulary now lives in the CLI command layer, not core.
        let doc = make_roadmap_doc("fbm", "two-way", "Two-Way Players");
        let phases = vec![(
            "phase-1-core".to_string(),
            make_phase_doc(1, "Core", PhaseStatus::InProgress),
        )];
        assert!(!format_roadmap_summary(&doc, &phases, None).contains("Hint:"));
        assert!(!format_roadmap_summary_md(&doc, &phases, None).contains("Hint:"));
    }

    #[test]
    fn phase_detail_omits_cli_nav() {
        // Prev/Next nav vocabulary now lives in the CLI command layer, not core.
        let doc = make_phase_doc(2, "Service", PhaseStatus::InProgress);
        assert!(!format_phase_detail("phase-2-service", &doc, None).contains("Prev:"));
        assert!(!format_phase_detail("phase-2-service", &doc, None).contains("Next:"));
        assert!(!format_phase_detail_md("phase-2-service", &doc, None).contains("Prev:"));
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
            tags: None,
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
}
