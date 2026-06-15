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
use crate::markdown::render_markdown;
use crate::view::{
    PhaseDetailView, PhaseRow, RoadmapDetail, RoadmapRow, roadmap_status_label, status_label,
};

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
        Screen::RoadmapDetail { detail, state, .. } => {
            render_roadmap_detail(frame, detail, state, footer)
        }
        Screen::PhaseDetail {
            phases,
            index,
            scroll,
        } => render_phase_detail(
            frame,
            &phases[*index],
            *index,
            phases.len(),
            *scroll,
            footer,
        ),
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

/// Renders the phase-detail screen: a bordered metadata header above a bordered,
/// scrollable, markdown-rendered body.
///
/// `index`/`total` drive the `phase i/n` title; `scroll` is the body's vertical
/// offset in logical lines. The body re-wraps to the current width on every
/// draw, so a terminal resize needs no extra bookkeeping.
fn render_phase_detail(
    frame: &mut Frame,
    view: &PhaseDetailView,
    index: usize,
    total: usize,
    scroll: u16,
    footer: Option<&str>,
) {
    let area = frame.area();
    let meta = phase_meta_lines(view);
    // +2 for the header block's top and bottom borders.
    let header_height = u16::try_from(meta.len())
        .unwrap_or(u16::MAX)
        .saturating_add(2);
    let chunks =
        Layout::vertical([Constraint::Length(header_height), Constraint::Min(1)]).split(area);

    let header_block =
        Block::bordered().title(format!("phase {}/{} — {}", index + 1, total, view.roadmap));
    let header = Paragraph::new(meta).block(header_block);
    frame.render_widget(header, chunks[0]);

    let body_block = Block::bordered().title("body").title_bottom(Line::from(
        footer.unwrap_or("j/k: scroll  n/p: prev/next  esc: back  q: quit"),
    ));
    let body = Paragraph::new(render_markdown(&view.body))
        .block(body_block)
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));
    frame.render_widget(body, chunks[1]);
}

/// Builds the metadata lines shown in the phase-detail header: the phase's
/// numbered title, its status badge, and (when present) completion date, short
/// commit SHA, and tags.
fn phase_meta_lines(view: &PhaseDetailView) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(format!("{}. {}", view.number, view.title)),
        Line::from(format!("status: {}", status_label(view.status))),
    ];
    if let Some(date) = view.completed {
        lines.push(Line::from(format!("completed: {date}")));
    }
    if let Some(commit) = &view.commit {
        let short: String = commit.chars().take(7).collect();
        lines.push(Line::from(format!("commit: {short}")));
    }
    if !view.tags.is_empty() {
        lines.push(Line::from(format!("tags: {}", view.tags.join(", "))));
    }
    lines
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

    /// Collects a test backend's buffer into one newline-joined string.
    fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        let area = buffer.area;
        let mut text = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                text.push_str(buffer[(x, y)].symbol());
            }
            text.push('\n');
        }
        text
    }

    #[test]
    fn phase_detail_renders_distinct_block_kinds() {
        let body = "\
# Title

- one
- two

```
let x = 1;
```

| a | b |
|---|---|
| 1 | 2 |
";
        let view = PhaseDetailView {
            roadmap: "alpha".to_string(),
            number: 2,
            title: "Second".to_string(),
            status: PhaseStatus::InProgress,
            completed: None,
            commit: Some("abcdef1234567".to_string()),
            tags: vec!["ui".to_string()],
            body: body.to_string(),
        };
        let mut terminal = Terminal::new(TestBackend::new(50, 28)).unwrap();
        terminal
            .draw(|frame| render_phase_detail(frame, &view, 1, 3, 0, None))
            .unwrap();
        let text = buffer_text(&terminal);

        // Header metadata.
        assert!(text.contains("phase 2/3 — alpha"), "{text}");
        assert!(text.contains("2. Second"), "{text}");
        assert!(text.contains("in-progress"), "{text}");
        assert!(text.contains("commit: abcdef1"), "{text}");
        assert!(text.contains("tags: ui"), "{text}");

        // Each block kind renders distinctly in the body.
        assert!(text.contains("# Title"), "heading missing: {text}");
        assert!(text.contains("• one"), "list bullet missing: {text}");
        assert!(text.contains("let x = 1;"), "code block missing: {text}");
        assert!(text.contains("a | b"), "table header missing: {text}");
        assert!(text.contains('+'), "table separator missing: {text}");
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
