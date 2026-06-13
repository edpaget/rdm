//! Application state and event handling for the TUI.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::widgets::ListState;

/// Top-level application state.
///
/// Holds the list of known projects, the selection state for the home-screen
/// list, and the quit flag the event loop polls after each key.
pub struct App {
    /// Names of the projects known to rdm, sorted alphabetically.
    pub projects: Vec<String>,
    /// Selection state for the projects list widget.
    pub list_state: ListState,
    /// Set once the user requests to quit; the event loop exits when true.
    pub should_quit: bool,
}

impl App {
    /// Creates a new app over the given project names, selecting the first
    /// project when the list is non-empty.
    pub fn new(projects: Vec<String>) -> Self {
        let mut list_state = ListState::default();
        if !projects.is_empty() {
            list_state.select(Some(0));
        }
        Self {
            projects,
            list_state,
            should_quit: false,
        }
    }

    /// Handles a key event, updating selection or setting the quit flag.
    ///
    /// Key release events are ignored (some terminals emit both press and
    /// release). Quits on `q`, `Esc`, or `Ctrl-C`. Moves the selection with
    /// `Down`/`j` and `Up`/`k`. `Enter` is a deliberate no-op for this phase.
    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
            }
            KeyCode::Char('q') | KeyCode::Esc => {
                self.should_quit = true;
            }
            KeyCode::Down | KeyCode::Char('j') => self.select_next(),
            KeyCode::Up | KeyCode::Char('k') => self.select_previous(),
            KeyCode::Enter => {}
            _ => {}
        }
    }

    /// Moves the selection down by one, saturating at the last project.
    fn select_next(&mut self) {
        if self.projects.is_empty() {
            return;
        }
        let next = match self.list_state.selected() {
            Some(i) if i + 1 < self.projects.len() => i + 1,
            Some(i) => i,
            None => 0,
        };
        self.list_state.select(Some(next));
    }

    /// Moves the selection up by one, saturating at the first project.
    fn select_previous(&mut self) {
        if self.projects.is_empty() {
            return;
        }
        let prev = match self.list_state.selected() {
            Some(i) => i.saturating_sub(1),
            None => 0,
        };
        self.list_state.select(Some(prev));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn projects() -> Vec<String> {
        vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()]
    }

    #[test]
    fn q_sets_should_quit() {
        let mut app = App::new(projects());
        app.handle_key(press(KeyCode::Char('q')));
        assert!(app.should_quit);
    }

    #[test]
    fn esc_sets_should_quit() {
        let mut app = App::new(projects());
        app.handle_key(press(KeyCode::Esc));
        assert!(app.should_quit);
    }

    #[test]
    fn ctrl_c_sets_should_quit() {
        let mut app = App::new(projects());
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.should_quit);
    }

    #[test]
    fn plain_c_does_not_quit() {
        let mut app = App::new(projects());
        app.handle_key(press(KeyCode::Char('c')));
        assert!(!app.should_quit);
    }

    #[test]
    fn down_and_j_move_selection_forward() {
        let mut app = App::new(projects());
        assert_eq!(app.list_state.selected(), Some(0));
        app.handle_key(press(KeyCode::Down));
        assert_eq!(app.list_state.selected(), Some(1));
        app.handle_key(press(KeyCode::Char('j')));
        assert_eq!(app.list_state.selected(), Some(2));
        // Saturates at the last item.
        app.handle_key(press(KeyCode::Char('j')));
        assert_eq!(app.list_state.selected(), Some(2));
    }

    #[test]
    fn up_and_k_move_selection_backward() {
        let mut app = App::new(projects());
        app.list_state.select(Some(2));
        app.handle_key(press(KeyCode::Up));
        assert_eq!(app.list_state.selected(), Some(1));
        app.handle_key(press(KeyCode::Char('k')));
        assert_eq!(app.list_state.selected(), Some(0));
        // Saturates at the first item.
        app.handle_key(press(KeyCode::Char('k')));
        assert_eq!(app.list_state.selected(), Some(0));
    }

    #[test]
    fn release_events_are_ignored() {
        let mut app = App::new(projects());
        let release = KeyEvent::new_with_kind(
            KeyCode::Char('q'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        );
        app.handle_key(release);
        assert!(!app.should_quit);
    }

    #[test]
    fn enter_is_a_noop() {
        let mut app = App::new(projects());
        app.handle_key(press(KeyCode::Enter));
        assert!(!app.should_quit);
        assert_eq!(app.list_state.selected(), Some(0));
    }

    #[test]
    fn navigation_on_empty_list_is_noop() {
        let mut app = App::new(Vec::new());
        assert_eq!(app.list_state.selected(), None);
        app.handle_key(press(KeyCode::Down));
        assert_eq!(app.list_state.selected(), None);
    }
}
