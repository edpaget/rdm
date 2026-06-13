//! Rendering for the home screen.

use ratatui::Frame;
use ratatui::text::Line;
use ratatui::widgets::{Block, List, ListItem, Paragraph};

use crate::app::App;

/// Renders the home screen into `frame`.
///
/// Draws a bordered list of project names titled `rdm — projects` with a `> `
/// selection highlight and a quit hint along the bottom border. When no
/// projects exist, an empty-state hint is shown instead of the list.
pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let block = Block::bordered()
        .title("rdm — projects")
        .title_bottom(Line::from("q/esc: quit"));

    if app.projects.is_empty() {
        let hint =
            Paragraph::new("No projects found. Create one with `rdm project create`.").block(block);
        frame.render_widget(hint, area);
        return;
    }

    let items: Vec<ListItem> = app
        .projects
        .iter()
        .map(|name| ListItem::new(name.as_str()))
        .collect();
    let list = List::new(items).block(block).highlight_symbol("> ");
    frame.render_stateful_widget(list, area, &mut app.list_state);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn render_to_backend(app: &mut App, width: u16, height: u16) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| render(frame, app)).unwrap();
        terminal
    }

    #[test]
    fn two_projects_first_selected() {
        let mut app = App::new(vec!["alpha".to_string(), "beta".to_string()]);
        let terminal = render_to_backend(&mut app, 22, 5);
        terminal.backend().assert_buffer_lines([
            "┌rdm — projects──────┐",
            "│> alpha             │",
            "│  beta              │",
            "│                    │",
            "└q/esc: quit─────────┘",
        ]);
    }

    #[test]
    fn selection_moves_after_down_key() {
        let mut app = App::new(vec!["alpha".to_string(), "beta".to_string()]);
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        let terminal = render_to_backend(&mut app, 22, 5);
        terminal.backend().assert_buffer_lines([
            "┌rdm — projects──────┐",
            "│  alpha             │",
            "│> beta              │",
            "│                    │",
            "└q/esc: quit─────────┘",
        ]);
    }

    #[test]
    fn empty_state_renders_hint() {
        let mut app = App::new(Vec::new());
        let terminal = render_to_backend(&mut app, 60, 4);
        terminal.backend().assert_buffer_lines([
            "┌rdm — projects────────────────────────────────────────────┐",
            "│No projects found. Create one with `rdm project create`.  │",
            "│                                                          │",
            "└q/esc: quit───────────────────────────────────────────────┘",
        ]);
    }
}
