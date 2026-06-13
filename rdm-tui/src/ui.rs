//! Rendering for the TUI screens.
//!
//! [`render`] dispatches on the active screen of the navigation stack to the
//! per-screen render functions below. Each takes only the data it needs, which
//! keeps them pure and testable against a `TestBackend`.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::text::Line;
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph, Wrap};

use crate::app::{App, Screen};
use crate::view::{PhaseRow, RoadmapDetail, RoadmapRow, roadmap_status_label, status_label};

/// Renders the active screen into `frame`.
pub fn render(frame: &mut Frame, app: &mut App) {
    let App {
        projects,
        stack,
        status_message,
        ..
    } = app;
    let footer = status_message.as_deref();
    match stack.last_mut().expect("stack is never empty") {
        Screen::Home { state } => render_home(frame, projects, state, footer),
        Screen::RoadmapList {
            project,
            rows,
            state,
        } => render_roadmap_list(frame, project, rows, state, footer),
        Screen::RoadmapDetail { detail, state } => {
            render_roadmap_detail(frame, detail, state, footer)
        }
    }
}

/// Renders the home screen: a bordered list of project names.
///
/// When no projects exist, an empty-state hint is shown instead of the list.
/// The bottom border shows `footer` (a status message) when set, otherwise the
/// quit hint.
fn render_home(
    frame: &mut Frame,
    projects: &[String],
    state: &mut ListState,
    footer: Option<&str>,
) {
    let area = frame.area();
    let block = Block::bordered()
        .title("rdm — projects")
        .title_bottom(Line::from(footer.unwrap_or("q/esc: quit")));

    if projects.is_empty() {
        let hint =
            Paragraph::new("No projects found. Create one with `rdm project create`.").block(block);
        frame.render_widget(hint, area);
        return;
    }

    let items: Vec<ListItem> = projects
        .iter()
        .map(|name| ListItem::new(name.as_str()))
        .collect();
    let list = List::new(items).block(block).highlight_symbol("> ");
    frame.render_stateful_widget(list, area, state);
}

/// Renders the roadmap-list screen: one row per roadmap with status symbol,
/// slug, title, priority, and `done/total` progress.
fn render_roadmap_list(
    frame: &mut Frame,
    project: &str,
    rows: &[RoadmapRow],
    state: &mut ListState,
    footer: Option<&str>,
) {
    let area = frame.area();
    let block = Block::bordered()
        .title(format!("rdm — {project} roadmaps"))
        .title_bottom(Line::from(
            footer.unwrap_or("j/k: move  enter: open  esc: back  q: quit"),
        ));

    if rows.is_empty() {
        let hint = Paragraph::new("No roadmaps in this project.").block(block);
        frame.render_widget(hint, area);
        return;
    }

    let items: Vec<ListItem> = rows
        .iter()
        .map(|r| ListItem::new(roadmap_row_line(r)))
        .collect();
    let list = List::new(items).block(block).highlight_symbol("> ");
    frame.render_stateful_widget(list, area, state);
}

/// Formats a single roadmap row into a fixed-column line.
fn roadmap_row_line(row: &RoadmapRow) -> String {
    let priority = row
        .priority
        .map(|p| p.to_string())
        .unwrap_or_else(|| "-".to_string());
    format!(
        "{:<16} {:<18} {:<24} {:<8} {}/{}",
        roadmap_status_label(row.status),
        row.slug,
        row.title,
        priority,
        row.done,
        row.total,
    )
}

/// Renders the roadmap-detail screen: the wrapped body above an ordered list of
/// phases (number, title, status badge).
fn render_roadmap_detail(
    frame: &mut Frame,
    detail: &RoadmapDetail,
    state: &mut ListState,
    footer: Option<&str>,
) {
    let area = frame.area();
    // Saturate rather than wrap on the (practically unreachable) >u16 phase
    // count; +2 for the block's top and bottom borders.
    let phase_lines = u16::try_from(detail.phases.len())
        .unwrap_or(u16::MAX)
        .saturating_add(2);
    let chunks =
        Layout::vertical([Constraint::Min(1), Constraint::Length(phase_lines)]).split(area);

    let body_block = Block::bordered().title(format!("{} — {}", detail.slug, detail.title));
    let body = Paragraph::new(detail.body.as_str())
        .block(body_block)
        .wrap(Wrap { trim: false });
    frame.render_widget(body, chunks[0]);

    let items: Vec<ListItem> = detail
        .phases
        .iter()
        .map(|p| ListItem::new(phase_row_line(p)))
        .collect();
    let phases_block = Block::bordered().title("phases").title_bottom(Line::from(
        footer.unwrap_or("j/k: move  esc: back  q: quit"),
    ));
    let list = List::new(items).block(phases_block).highlight_symbol("> ");
    frame.render_stateful_widget(list, chunks[1], state);
}

/// Formats a single phase row into a line: `<number>. <title>  <status badge>`.
fn phase_row_line(row: &PhaseRow) -> String {
    format!(
        "{}. {}  {}",
        row.number,
        row.title,
        status_label(row.status)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::PhaseRow;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use rdm_core::model::{PhaseStatus, Priority};
    use rdm_core::ops::roadmap::RoadmapStatus;

    fn render_home_to_backend(
        projects: &[String],
        state: &mut ListState,
        width: u16,
        height: u16,
    ) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render_home(frame, projects, state, None))
            .unwrap();
        terminal
    }

    fn home_state(len: usize) -> ListState {
        let mut state = ListState::default();
        if len > 0 {
            state.select(Some(0));
        }
        state
    }

    #[test]
    fn home_two_projects_first_selected() {
        let projects = vec!["alpha".to_string(), "beta".to_string()];
        let mut state = home_state(projects.len());
        let terminal = render_home_to_backend(&projects, &mut state, 22, 5);
        terminal.backend().assert_buffer_lines([
            "┌rdm — projects──────┐",
            "│> alpha             │",
            "│  beta              │",
            "│                    │",
            "└q/esc: quit─────────┘",
        ]);
    }

    #[test]
    fn home_empty_state_renders_hint() {
        let projects: Vec<String> = Vec::new();
        let mut state = home_state(0);
        let terminal = render_home_to_backend(&projects, &mut state, 60, 4);
        terminal.backend().assert_buffer_lines([
            "┌rdm — projects────────────────────────────────────────────┐",
            "│No projects found. Create one with `rdm project create`.  │",
            "│                                                          │",
            "└q/esc: quit───────────────────────────────────────────────┘",
        ]);
    }

    #[test]
    fn roadmap_list_renders_columns() {
        let rows = vec![
            RoadmapRow {
                slug: "alpha".to_string(),
                title: "Alpha".to_string(),
                status: RoadmapStatus::InProgress,
                priority: Some(Priority::High),
                done: 1,
                total: 2,
            },
            RoadmapRow {
                slug: "beta".to_string(),
                title: "Beta".to_string(),
                status: RoadmapStatus::NotStarted,
                priority: None,
                done: 0,
                total: 1,
            },
        ];
        let mut state = ListState::default();
        state.select(Some(0));
        let mut terminal = Terminal::new(TestBackend::new(80, 5)).unwrap();
        terminal
            .draw(|frame| render_roadmap_list(frame, "demo", &rows, &mut state, None))
            .unwrap();
        terminal.backend().assert_buffer_lines([
            "┌rdm — demo roadmaps───────────────────────────────────────────────────────────┐",
            "│> [~] in-progress  alpha              Alpha                    high     1/2   │",
            "│  [ ] not-started  beta               Beta                     -        0/1   │",
            "│                                                                              │",
            "└j/k: move  enter: open  esc: back  q: quit────────────────────────────────────┘",
        ]);
    }

    #[test]
    fn roadmap_detail_renders_body_and_phases() {
        let detail = RoadmapDetail {
            slug: "alpha".to_string(),
            title: "Alpha".to_string(),
            body: "Alpha body.".to_string(),
            phases: vec![
                PhaseRow {
                    number: 1,
                    title: "First".to_string(),
                    status: PhaseStatus::Done,
                },
                PhaseRow {
                    number: 2,
                    title: "Second".to_string(),
                    status: PhaseStatus::NotStarted,
                },
            ],
        };
        let mut state = ListState::default();
        state.select(Some(0));
        let mut terminal = Terminal::new(TestBackend::new(50, 8)).unwrap();
        terminal
            .draw(|frame| render_roadmap_detail(frame, &detail, &mut state, None))
            .unwrap();
        terminal.backend().assert_buffer_lines([
            "┌alpha — Alpha───────────────────────────────────┐",
            "│Alpha body.                                     │",
            "│                                                │",
            "└────────────────────────────────────────────────┘",
            "┌phases──────────────────────────────────────────┐",
            "│> 1. First  [x] done                            │",
            "│  2. Second  [ ] not-started                    │",
            "└j/k: move  esc: back  q: quit───────────────────┘",
        ]);
    }
}
