use rdm_core::display::RoadmapWithPhases;
use rdm_core::document::Document;
use rdm_core::model::{Phase, Task};
use rdm_core::search::SearchResult;
use tabled::Tabled;
use tabled::settings::peaker::Priority;
use tabled::settings::{Style, Width};

fn terminal_width() -> usize {
    terminal_size::terminal_size()
        .map(|(w, _)| w.0 as usize)
        .unwrap_or(120)
}

#[derive(Tabled)]
struct RoadmapRow {
    #[tabled(rename = "Slug")]
    slug: String,
    #[tabled(rename = "Title")]
    title: String,
    #[tabled(rename = "Progress")]
    progress: String,
}

pub fn format_roadmap_table(entries: &[RoadmapWithPhases]) -> String {
    if entries.is_empty() {
        return "No roadmaps found.\n".to_string();
    }
    let rows: Vec<RoadmapRow> = entries
        .iter()
        .map(|(doc, phases)| {
            let total = phases.len();
            let done = phases
                .iter()
                .filter(|(_, p)| p.frontmatter.status == rdm_core::model::PhaseStatus::Done)
                .count();
            RoadmapRow {
                slug: doc.frontmatter.roadmap.clone(),
                title: doc.frontmatter.title.clone(),
                progress: format!("{done}/{total} phases done"),
            }
        })
        .collect();
    build_table(rows)
}

#[derive(Tabled)]
struct PhaseRow {
    #[tabled(rename = "#")]
    number: u32,
    #[tabled(rename = "Phase")]
    title: String,
    #[tabled(rename = "Status")]
    status: String,
    #[tabled(rename = "Stem")]
    stem: String,
}

pub fn format_phase_table(phases: &[(String, Document<Phase>)]) -> String {
    if phases.is_empty() {
        return "No phases yet.\n".to_string();
    }
    let rows: Vec<PhaseRow> = phases
        .iter()
        .map(|(stem, doc)| PhaseRow {
            number: doc.frontmatter.phase,
            title: doc.frontmatter.title.clone(),
            status: doc.frontmatter.status.to_string(),
            stem: stem.clone(),
        })
        .collect();
    build_table(rows)
}

#[derive(Tabled)]
struct TaskRow {
    #[tabled(rename = "Slug")]
    slug: String,
    #[tabled(rename = "Title")]
    title: String,
    #[tabled(rename = "Status")]
    status: String,
    #[tabled(rename = "Priority")]
    priority: String,
}

pub fn format_task_table(tasks: &[(String, Document<Task>)]) -> String {
    if tasks.is_empty() {
        return "No tasks found.\n".to_string();
    }
    let rows: Vec<TaskRow> = tasks
        .iter()
        .map(|(slug, doc)| TaskRow {
            slug: slug.clone(),
            title: doc.frontmatter.title.clone(),
            status: doc.frontmatter.status.to_string(),
            priority: doc.frontmatter.priority.to_string(),
        })
        .collect();
    build_table(rows)
}

#[derive(Tabled)]
struct SearchRow {
    #[tabled(rename = "#")]
    rank: usize,
    #[tabled(rename = "Type")]
    kind: String,
    #[tabled(rename = "Title")]
    title: String,
    #[tabled(rename = "Identifier")]
    identifier: String,
    #[tabled(rename = "Snippet")]
    snippet: String,
}

pub fn format_search_table(results: &[SearchResult]) -> String {
    if results.is_empty() {
        return "No results found.\n".to_string();
    }
    let rows: Vec<SearchRow> = results
        .iter()
        .enumerate()
        .map(|(i, r)| SearchRow {
            rank: i + 1,
            kind: format!("{:?}", r.kind),
            title: r.title.clone(),
            identifier: r.identifier.clone(),
            snippet: r.snippet.clone(),
        })
        .collect();
    build_table(rows)
}

fn build_table<T: Tabled>(rows: Vec<T>) -> String {
    let mut table = tabled::Table::new(rows);
    table
        .with(Style::rounded())
        .with(Width::truncate(terminal_width()).priority(Priority::max(false)));
    format!("{table}\n")
}
