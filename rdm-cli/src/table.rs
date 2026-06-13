use rdm_core::display::RoadmapWithPhases;
use rdm_core::document::Document;
use rdm_core::model::{Phase, Task};
use rdm_core::search::SearchResult;
use tabled::builder::Builder;
use tabled::settings::peaker::Priority;
use tabled::settings::{Style, Width};

fn terminal_width() -> usize {
    terminal_size::terminal_size()
        .map(|(w, _)| w.0 as usize)
        .unwrap_or(120)
}

pub fn format_roadmap_table(entries: &[RoadmapWithPhases]) -> String {
    if entries.is_empty() {
        return "No roadmaps found.\n".to_string();
    }
    let rows = entries
        .iter()
        .map(|(doc, phases)| {
            let total = phases.len();
            let done = phases
                .iter()
                .filter(|(_, p)| p.frontmatter.status.is_terminal())
                .count();
            [
                doc.frontmatter.roadmap.clone(),
                doc.frontmatter.title.clone(),
                format!("{done}/{total} phases done"),
                doc.frontmatter
                    .priority
                    .map(|p| p.to_string())
                    .unwrap_or_default(),
            ]
        })
        .collect();
    build_table(["Slug", "Title", "Progress", "Priority"], rows)
}

pub fn format_phase_table(phases: &[(String, Document<Phase>)]) -> String {
    if phases.is_empty() {
        return "No phases yet.\n".to_string();
    }
    let rows = phases
        .iter()
        .map(|(stem, doc)| {
            [
                doc.frontmatter.phase.to_string(),
                doc.frontmatter.title.clone(),
                doc.frontmatter.status.to_string(),
                stem.clone(),
            ]
        })
        .collect();
    build_table(["#", "Phase", "Status", "Stem"], rows)
}

pub fn format_task_table(tasks: &[(String, Document<Task>)]) -> String {
    if tasks.is_empty() {
        return "No tasks found.\n".to_string();
    }
    let rows = tasks
        .iter()
        .map(|(slug, doc)| {
            [
                slug.clone(),
                doc.frontmatter.title.clone(),
                doc.frontmatter.status.to_string(),
                doc.frontmatter.priority.to_string(),
            ]
        })
        .collect();
    build_table(["Slug", "Title", "Status", "Priority"], rows)
}

pub fn format_search_table(results: &[SearchResult]) -> String {
    if results.is_empty() {
        return "No results found.\n".to_string();
    }
    let rows = results
        .iter()
        .enumerate()
        .map(|(i, r)| {
            [
                (i + 1).to_string(),
                format!("{:?}", r.kind),
                r.title.clone(),
                r.identifier.clone(),
                r.snippet.clone(),
            ]
        })
        .collect();
    build_table(["#", "Type", "Title", "Identifier", "Snippet"], rows)
}

pub fn build_table<const N: usize>(headers: [&str; N], rows: Vec<[String; N]>) -> String {
    let mut builder = Builder::default();
    builder.push_record(headers);
    for row in rows {
        builder.push_record(row);
    }
    let mut table = builder.build();
    table
        .with(Style::rounded())
        .with(Width::truncate(terminal_width()).priority(Priority::max(false)));
    format!("{table}\n")
}
