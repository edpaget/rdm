//! Application state and event handling for the TUI.
//!
//! Navigation is modeled as an explicit stack of [`Screen`]s on [`App`].
//! Drilling in (Enter) pushes a new screen; backing out (Esc/`h`) pops. Because
//! the underlying screen stays on the stack, its cursor position is preserved
//! automatically when the user returns.

use anyhow::{Context, Result};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::widgets::ListState;
use rdm_core::io::load_roadmap;
use rdm_core::model::PhaseStatus;
use rdm_core::ops::phase::list_phases;
use rdm_core::ops::roadmap::{computed_status, list_roadmaps};
use rdm_store_fs::FsStore;

use crate::view::{PhaseDetailView, PhaseRow, RoadmapDetail, RoadmapRow, build_phase_details};

/// A single screen in the navigation stack.
pub enum Screen {
    /// The root screen: the list of projects.
    Home {
        /// Selection state for the projects list.
        state: ListState,
    },
    /// The list of roadmaps within a project.
    RoadmapList {
        /// Project whose roadmaps are shown.
        project: String,
        /// One row per roadmap, sorted alphabetically by slug.
        rows: Vec<RoadmapRow>,
        /// Selection state for the roadmap list.
        state: ListState,
    },
    /// A single roadmap's body and phase list.
    RoadmapDetail {
        /// Project the roadmap belongs to; needed to load phase bodies on
        /// drill-in.
        project: String,
        /// The roadmap detail being shown.
        detail: RoadmapDetail,
        /// Selection state for the phase list.
        state: ListState,
    },
    /// A single phase's metadata and markdown-rendered body.
    ///
    /// All phases of the roadmap are loaded once on entry, so prev/next is pure
    /// indexing. `scroll` is the body's vertical offset in logical lines.
    PhaseDetail {
        /// Every phase of the roadmap, in number order.
        phases: Vec<PhaseDetailView>,
        /// Index of the phase currently shown.
        index: usize,
        /// Vertical scroll offset into the rendered body.
        scroll: u16,
    },
}

/// Top-level application state.
///
/// Owns the [`FsStore`] used to load roadmap data on demand, the list of known
/// projects, the navigation [`stack`](App::stack), an optional transient status
/// message (e.g. a load error), and the quit flag the event loop polls.
pub struct App {
    /// The store used to load roadmaps and phases when drilling in.
    pub store: FsStore,
    /// Names of the projects known to rdm, sorted alphabetically.
    pub projects: Vec<String>,
    /// Navigation stack; the last element is the active screen. Never empty.
    pub stack: Vec<Screen>,
    /// Transient message shown in the footer (e.g. a load error). Cleared on
    /// the next key press.
    pub status_message: Option<String>,
    /// Set once the user requests to quit; the event loop exits when true.
    pub should_quit: bool,
}

impl App {
    /// Creates a new app over the given store and project names, seeding the
    /// navigation stack with the home screen. The first project is selected
    /// when the list is non-empty.
    pub fn new(store: FsStore, projects: Vec<String>) -> Self {
        let state = initial_state(projects.len());
        Self {
            store,
            projects,
            stack: vec![Screen::Home { state }],
            status_message: None,
            should_quit: false,
        }
    }

    /// Handles a key event, updating navigation/selection or setting the quit
    /// flag.
    ///
    /// Key release events are ignored (some terminals emit both press and
    /// release). `q`/`Ctrl-C` always quit. `Esc`/`h` pop the navigation stack,
    /// or quit when only the home screen remains. `Enter` drills into the
    /// selected item. `Down`/`j` and `Up`/`k` move the active screen's cursor.
    ///
    /// # Errors
    ///
    /// Returns an error if drilling in fails to load roadmaps or a roadmap's
    /// detail. The caller is expected to surface the error via
    /// [`status_message`](App::status_message) rather than abort.
    pub fn handle_key(&mut self, key: KeyEvent) -> Result<()> {
        if key.kind == KeyEventKind::Release {
            return Ok(());
        }
        // A fresh key dismisses any stale status message; a failing `enter()`
        // re-sets one via the caller.
        self.status_message = None;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('c') if ctrl => self.should_quit = true,
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Esc | KeyCode::Char('h') => self.back(),
            KeyCode::Enter => self.enter()?,
            // Movement is screen-specific: the phase-detail screen scrolls its
            // body and cycles phases; list screens move a selection cursor.
            _ if matches!(self.stack.last(), Some(Screen::PhaseDetail { .. })) => {
                self.handle_phase_key(key, ctrl);
            }
            KeyCode::Down | KeyCode::Char('j') => self.select_next(),
            KeyCode::Up | KeyCode::Char('k') => self.select_previous(),
            _ => {}
        }
        Ok(())
    }

    /// Handles a movement key while a [`Screen::PhaseDetail`] is active.
    ///
    /// `j`/`Down` and `k`/`Up` scroll one line; `PageDown`/`PageUp` and
    /// `Ctrl-d`/`Ctrl-u` scroll by a (half-)page. `n`/`Right` and `p`/`Left`
    /// move to the next/previous phase, resetting the scroll. Scrolling clamps
    /// to `[0, logical_lines - 1]` so the last line can always reach the top.
    fn handle_phase_key(&mut self, key: KeyEvent, ctrl: bool) {
        /// Page size used by `PageUp`/`PageDown`; half of it for `Ctrl-u`/`Ctrl-d`.
        const PAGE: u16 = 20;
        let Some(Screen::PhaseDetail {
            phases,
            index,
            scroll,
        }) = self.stack.last_mut()
        else {
            return;
        };
        let max = max_scroll(&phases[*index]);
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => *scroll = scroll.saturating_add(1).min(max),
            KeyCode::Up | KeyCode::Char('k') => *scroll = scroll.saturating_sub(1),
            KeyCode::PageDown => *scroll = scroll.saturating_add(PAGE).min(max),
            KeyCode::PageUp => *scroll = scroll.saturating_sub(PAGE),
            KeyCode::Char('d') if ctrl => *scroll = scroll.saturating_add(PAGE / 2).min(max),
            KeyCode::Char('u') if ctrl => *scroll = scroll.saturating_sub(PAGE / 2),
            KeyCode::Char('n') | KeyCode::Right => {
                if *index + 1 < phases.len() {
                    *index += 1;
                    *scroll = 0;
                }
            }
            KeyCode::Char('p') | KeyCode::Left => {
                if *index > 0 {
                    *index -= 1;
                    *scroll = 0;
                }
            }
            _ => {}
        }
    }

    /// Drills into the selected item on the active screen, pushing a new screen.
    ///
    /// From [`Screen::Home`] this loads the selected project's roadmaps and
    /// pushes a [`Screen::RoadmapList`]; from [`Screen::RoadmapList`] it loads
    /// the selected roadmap's detail and pushes a [`Screen::RoadmapDetail`].
    /// [`Screen::RoadmapDetail`] is a leaf, so this is a no-op there. When
    /// nothing is selected (empty list), this is also a no-op.
    ///
    /// # Errors
    ///
    /// Returns an error if listing roadmaps/phases or loading the roadmap fails.
    pub fn enter(&mut self) -> Result<()> {
        match self.stack.last().expect("stack is never empty") {
            Screen::Home { state } => {
                let Some(idx) = state.selected() else {
                    return Ok(());
                };
                let Some(project) = self.projects.get(idx).cloned() else {
                    return Ok(());
                };
                let rows = load_roadmap_rows(&self.store, &project)?;
                let state = initial_state(rows.len());
                self.stack.push(Screen::RoadmapList {
                    project,
                    rows,
                    state,
                });
            }
            Screen::RoadmapList {
                project,
                rows,
                state,
            } => {
                let Some(idx) = state.selected() else {
                    return Ok(());
                };
                let Some(row) = rows.get(idx) else {
                    return Ok(());
                };
                let project = project.clone();
                let slug = row.slug.clone();
                let detail = load_roadmap_detail(&self.store, &project, &slug)?;
                let state = initial_state(detail.phases.len());
                self.stack.push(Screen::RoadmapDetail {
                    project,
                    detail,
                    state,
                });
            }
            Screen::RoadmapDetail {
                project,
                detail,
                state,
            } => {
                let Some(idx) = state.selected() else {
                    return Ok(());
                };
                let project = project.clone();
                let slug = detail.slug.clone();
                let phases = list_phases(&self.store, &project, &slug)
                    .with_context(|| format!("failed to list phases for roadmap '{slug}'"))?;
                let views = build_phase_details(&slug, phases);
                if views.is_empty() {
                    return Ok(());
                }
                let index = idx.min(views.len() - 1);
                self.stack.push(Screen::PhaseDetail {
                    phases: views,
                    index,
                    scroll: 0,
                });
            }
            Screen::PhaseDetail { .. } => {}
        }
        Ok(())
    }

    /// Pops the active screen, returning to the one beneath it. When only the
    /// home screen remains, this quits instead (preserving phase-1 behavior
    /// where Esc quits from home).
    fn back(&mut self) {
        if self.stack.len() > 1 {
            self.stack.pop();
        } else {
            self.should_quit = true;
        }
    }

    /// Number of selectable items on the active screen.
    ///
    /// [`Screen::PhaseDetail`] has no list cursor (it scrolls instead), so it
    /// reports zero; the movement keys are routed away from selection there.
    fn active_len(&self) -> usize {
        match self.stack.last().expect("stack is never empty") {
            Screen::Home { .. } => self.projects.len(),
            Screen::RoadmapList { rows, .. } => rows.len(),
            Screen::RoadmapDetail { detail, .. } => detail.phases.len(),
            Screen::PhaseDetail { .. } => 0,
        }
    }

    /// Mutable access to the active screen's selection state.
    ///
    /// # Panics
    ///
    /// Panics on [`Screen::PhaseDetail`], which has no selection state. Callers
    /// (the `select_*` helpers) only reach this for list screens; phase-detail
    /// movement is routed through [`handle_phase_key`](App::handle_phase_key).
    fn active_state_mut(&mut self) -> &mut ListState {
        match self.stack.last_mut().expect("stack is never empty") {
            Screen::Home { state } => state,
            Screen::RoadmapList { state, .. } => state,
            Screen::RoadmapDetail { state, .. } => state,
            Screen::PhaseDetail { .. } => {
                unreachable!("phase-detail movement does not use a list cursor")
            }
        }
    }

    /// Moves the active screen's selection down by one, saturating at the end.
    fn select_next(&mut self) {
        let len = self.active_len();
        if len == 0 {
            return;
        }
        let state = self.active_state_mut();
        let next = match state.selected() {
            Some(i) if i + 1 < len => i + 1,
            Some(i) => i,
            None => 0,
        };
        state.select(Some(next));
    }

    /// Moves the active screen's selection up by one, saturating at the start.
    fn select_previous(&mut self) {
        let len = self.active_len();
        if len == 0 {
            return;
        }
        let state = self.active_state_mut();
        let prev = match state.selected() {
            Some(i) => i.saturating_sub(1),
            None => 0,
        };
        state.select(Some(prev));
    }
}

/// Maximum scroll offset for a phase body: `logical_lines - 1`.
///
/// Clamping to this (rather than to a viewport-relative value) guarantees the
/// last rendered line can always reach the top of the body pane, so every line
/// stays reachable even after the widget re-wraps to a narrow width.
fn max_scroll(view: &PhaseDetailView) -> u16 {
    let lines = crate::markdown::render_markdown(&view.body).lines.len();
    u16::try_from(lines.saturating_sub(1)).unwrap_or(u16::MAX)
}

/// Builds a [`ListState`] selecting the first item when `len` is non-zero.
fn initial_state(len: usize) -> ListState {
    let mut state = ListState::default();
    if len > 0 {
        state.select(Some(0));
    }
    state
}

/// Loads the roadmap rows for a project, computing status and phase progress.
fn load_roadmap_rows(store: &FsStore, project: &str) -> Result<Vec<RoadmapRow>> {
    let roadmaps = list_roadmaps(store, project, None, None)
        .with_context(|| format!("failed to list roadmaps for project '{project}'"))?;
    let mut rows = Vec::with_capacity(roadmaps.len());
    for doc in roadmaps {
        let slug = doc.frontmatter.roadmap;
        let phases = list_phases(store, project, &slug)
            .with_context(|| format!("failed to list phases for roadmap '{slug}'"))?;
        let statuses: Vec<PhaseStatus> = phases.iter().map(|(_, p)| p.frontmatter.status).collect();
        let done = statuses.iter().filter(|s| s.is_terminal()).count();
        let total = statuses.len();
        rows.push(RoadmapRow {
            slug,
            title: doc.frontmatter.title,
            status: computed_status(&statuses),
            priority: doc.frontmatter.priority,
            done,
            total,
        });
    }
    Ok(rows)
}

/// Loads a single roadmap's body and ordered phase list into a [`RoadmapDetail`].
fn load_roadmap_detail(store: &FsStore, project: &str, slug: &str) -> Result<RoadmapDetail> {
    let doc = load_roadmap(store, project, slug)
        .with_context(|| format!("failed to load roadmap '{slug}'"))?;
    let phases = list_phases(store, project, slug)
        .with_context(|| format!("failed to list phases for roadmap '{slug}'"))?;
    let phase_rows = phases
        .into_iter()
        .map(|(_, p)| PhaseRow {
            number: p.frontmatter.phase,
            title: p.frontmatter.title,
            status: p.frontmatter.status,
        })
        .collect();
    Ok(RoadmapDetail {
        slug: slug.to_string(),
        title: doc.frontmatter.title,
        body: doc.body,
        phases: phase_rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rdm_core::model::PhaseStatus;
    use rdm_core::ops::phase::{create_phase, update_phase};
    use rdm_core::ops::project::create_project;
    use rdm_core::ops::roadmap::{RoadmapStatus, create_roadmap};
    use tempfile::TempDir;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// An app whose home screen lists the given projects, backed by an empty
    /// store. Sufficient for key/quit/movement tests that never drill in.
    fn simple_app(projects: Vec<String>) -> App {
        let tmp = TempDir::new().unwrap();
        App::new(FsStore::new(tmp.path()), projects)
    }

    fn projects() -> Vec<String> {
        vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()]
    }

    /// A store with one project "demo" holding two roadmaps:
    /// - `alpha`: phase-1 done, phase-2 not-started → in-progress, 1/2.
    /// - `beta`: one not-started phase → not-started, 0/1.
    ///
    /// Returns the app plus the `TempDir` (kept alive for the store's lifetime).
    fn populated_app() -> (App, TempDir) {
        let tmp = TempDir::new().unwrap();
        let mut store = FsStore::new(tmp.path());
        create_project(&mut store, "demo", "Demo").unwrap();

        create_roadmap(
            &mut store,
            "demo",
            "alpha",
            "Alpha",
            Some("Alpha body."),
            None,
            None,
        )
        .unwrap();
        create_phase(
            &mut store,
            "demo",
            "alpha",
            "first",
            "First",
            Some(1),
            None,
            None,
        )
        .unwrap();
        create_phase(
            &mut store,
            "demo",
            "alpha",
            "second",
            "Second",
            Some(2),
            None,
            None,
        )
        .unwrap();
        update_phase(
            &mut store,
            "demo",
            "alpha",
            "phase-1-first",
            Some(PhaseStatus::Done),
            None,
            None,
            None,
            false,
        )
        .unwrap();

        create_roadmap(&mut store, "demo", "beta", "Beta", None, None, None).unwrap();
        create_phase(
            &mut store,
            "demo",
            "beta",
            "only",
            "Only",
            Some(1),
            None,
            None,
        )
        .unwrap();

        let app = App::new(store, vec!["demo".to_string()]);
        (app, tmp)
    }

    #[test]
    fn q_sets_should_quit() {
        let mut app = simple_app(projects());
        app.handle_key(press(KeyCode::Char('q'))).unwrap();
        assert!(app.should_quit);
    }

    #[test]
    fn esc_quits_from_home() {
        let mut app = simple_app(projects());
        app.handle_key(press(KeyCode::Esc)).unwrap();
        assert!(app.should_quit);
    }

    #[test]
    fn ctrl_c_sets_should_quit() {
        let mut app = simple_app(projects());
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL))
            .unwrap();
        assert!(app.should_quit);
    }

    #[test]
    fn plain_c_does_not_quit() {
        let mut app = simple_app(projects());
        app.handle_key(press(KeyCode::Char('c'))).unwrap();
        assert!(!app.should_quit);
    }

    #[test]
    fn down_and_j_move_selection_forward() {
        let mut app = simple_app(projects());
        assert_eq!(app.active_state_mut().selected(), Some(0));
        app.handle_key(press(KeyCode::Down)).unwrap();
        assert_eq!(app.active_state_mut().selected(), Some(1));
        app.handle_key(press(KeyCode::Char('j'))).unwrap();
        assert_eq!(app.active_state_mut().selected(), Some(2));
        // Saturates at the last item.
        app.handle_key(press(KeyCode::Char('j'))).unwrap();
        assert_eq!(app.active_state_mut().selected(), Some(2));
    }

    #[test]
    fn up_and_k_move_selection_backward() {
        let mut app = simple_app(projects());
        app.active_state_mut().select(Some(2));
        app.handle_key(press(KeyCode::Up)).unwrap();
        assert_eq!(app.active_state_mut().selected(), Some(1));
        app.handle_key(press(KeyCode::Char('k'))).unwrap();
        assert_eq!(app.active_state_mut().selected(), Some(0));
        // Saturates at the first item.
        app.handle_key(press(KeyCode::Char('k'))).unwrap();
        assert_eq!(app.active_state_mut().selected(), Some(0));
    }

    #[test]
    fn release_events_are_ignored() {
        let mut app = simple_app(projects());
        let release = KeyEvent::new_with_kind(
            KeyCode::Char('q'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        );
        app.handle_key(release).unwrap();
        assert!(!app.should_quit);
    }

    #[test]
    fn navigation_on_empty_list_is_noop() {
        let mut app = simple_app(Vec::new());
        assert_eq!(app.active_state_mut().selected(), None);
        app.handle_key(press(KeyCode::Down)).unwrap();
        assert_eq!(app.active_state_mut().selected(), None);
    }

    #[test]
    fn enter_pushes_roadmap_list() {
        let (mut app, _tmp) = populated_app();
        app.handle_key(press(KeyCode::Enter)).unwrap();
        assert_eq!(app.stack.len(), 2);
        match app.stack.last().unwrap() {
            Screen::RoadmapList { rows, .. } => assert_eq!(rows.len(), 2),
            _ => panic!("expected RoadmapList on top"),
        }
    }

    #[test]
    fn enter_twice_pushes_roadmap_detail() {
        let (mut app, _tmp) = populated_app();
        app.handle_key(press(KeyCode::Enter)).unwrap();
        app.handle_key(press(KeyCode::Enter)).unwrap();
        assert_eq!(app.stack.len(), 3);
        assert!(matches!(
            app.stack.last().unwrap(),
            Screen::RoadmapDetail { .. }
        ));
    }

    #[test]
    fn enter_on_roadmap_detail_opens_phase_detail() {
        let (mut app, _tmp) = populated_app();
        app.handle_key(press(KeyCode::Enter)).unwrap();
        app.handle_key(press(KeyCode::Enter)).unwrap();
        assert_eq!(app.stack.len(), 3);
        app.handle_key(press(KeyCode::Enter)).unwrap();
        assert_eq!(app.stack.len(), 4);
        assert!(matches!(
            app.stack.last().unwrap(),
            Screen::PhaseDetail { .. }
        ));
    }

    #[test]
    fn esc_pops_back_through_stack() {
        let (mut app, _tmp) = populated_app();
        app.handle_key(press(KeyCode::Enter)).unwrap();
        app.handle_key(press(KeyCode::Enter)).unwrap();
        assert_eq!(app.stack.len(), 3);
        app.handle_key(press(KeyCode::Esc)).unwrap();
        assert_eq!(app.stack.len(), 2);
        app.handle_key(press(KeyCode::Esc)).unwrap();
        assert_eq!(app.stack.len(), 1);
        assert!(!app.should_quit);
        // From home, Esc quits.
        app.handle_key(press(KeyCode::Esc)).unwrap();
        assert!(app.should_quit);
    }

    #[test]
    fn h_pops_like_esc() {
        let (mut app, _tmp) = populated_app();
        app.handle_key(press(KeyCode::Enter)).unwrap();
        assert_eq!(app.stack.len(), 2);
        app.handle_key(press(KeyCode::Char('h'))).unwrap();
        assert_eq!(app.stack.len(), 1);
    }

    #[test]
    fn back_preserves_underlying_cursor() {
        let (mut app, _tmp) = populated_app();
        app.handle_key(press(KeyCode::Enter)).unwrap(); // home -> list (cursor 0)
        app.handle_key(press(KeyCode::Down)).unwrap(); // list cursor -> 1 (beta)
        app.handle_key(press(KeyCode::Enter)).unwrap(); // list -> detail
        app.handle_key(press(KeyCode::Esc)).unwrap(); // detail -> list
        match app.stack.last().unwrap() {
            Screen::RoadmapList { rows, state, .. } => {
                assert_eq!(state.selected(), Some(1));
                assert_eq!(rows[1].slug, "beta");
            }
            _ => panic!("expected RoadmapList on top"),
        }
    }

    #[test]
    fn enter_builds_rows_with_status_and_progress() {
        let (mut app, _tmp) = populated_app();
        app.enter().unwrap();
        let Screen::RoadmapList { rows, .. } = app.stack.last().unwrap() else {
            panic!("expected RoadmapList");
        };
        assert_eq!(rows.len(), 2);

        let alpha = &rows[0];
        assert_eq!(alpha.slug, "alpha");
        assert_eq!(alpha.title, "Alpha");
        assert_eq!(alpha.status, RoadmapStatus::InProgress);
        assert_eq!((alpha.done, alpha.total), (1, 2));

        let beta = &rows[1];
        assert_eq!(beta.slug, "beta");
        assert_eq!(beta.status, RoadmapStatus::NotStarted);
        assert_eq!((beta.done, beta.total), (0, 1));
    }

    /// Drills home → list → detail for "alpha" (2 phases), then opens the
    /// selected phase. Returns the app at the `PhaseDetail` screen.
    fn open_phase_detail() -> (App, TempDir) {
        let (mut app, tmp) = populated_app();
        app.enter().unwrap(); // home -> list (alpha selected)
        app.enter().unwrap(); // list -> detail for alpha
        app.enter().unwrap(); // detail -> phase detail (phase 1 selected)
        (app, tmp)
    }

    #[test]
    fn enter_from_detail_opens_phase_at_selected_index() {
        let (mut app, _tmp) = populated_app();
        app.enter().unwrap(); // home -> list
        app.enter().unwrap(); // list -> detail
        // Select the second phase before drilling in.
        app.handle_key(press(KeyCode::Down)).unwrap();
        app.enter().unwrap();
        let Screen::PhaseDetail { phases, index, .. } = app.stack.last().unwrap() else {
            panic!("expected PhaseDetail");
        };
        assert_eq!(phases.len(), 2);
        assert_eq!(*index, 1);
        assert_eq!(phases[1].number, 2);
        assert_eq!(phases[1].roadmap, "alpha");
    }

    #[test]
    fn enter_is_noop_on_phase_detail_leaf() {
        let (mut app, _tmp) = open_phase_detail();
        assert_eq!(app.stack.len(), 4);
        app.handle_key(press(KeyCode::Enter)).unwrap();
        assert_eq!(app.stack.len(), 4);
    }

    #[test]
    fn n_and_p_cycle_phases_and_clamp() {
        let (mut app, _tmp) = open_phase_detail();
        // Starts at phase 1; `p` at the start is a no-op.
        app.handle_key(press(KeyCode::Char('p'))).unwrap();
        assert_eq!(phase_index(&app), 0);
        // `n` advances to phase 2, then clamps at the end.
        app.handle_key(press(KeyCode::Char('n'))).unwrap();
        assert_eq!(phase_index(&app), 1);
        app.handle_key(press(KeyCode::Char('n'))).unwrap();
        assert_eq!(phase_index(&app), 1);
        // `p` walks back.
        app.handle_key(press(KeyCode::Char('p'))).unwrap();
        assert_eq!(phase_index(&app), 0);
    }

    #[test]
    fn switching_phase_resets_scroll() {
        let (mut app, _tmp) = open_phase_detail();
        set_scroll(&mut app, 1);
        app.handle_key(press(KeyCode::Char('n'))).unwrap();
        assert_eq!(phase_scroll(&app), 0);
    }

    #[test]
    fn j_and_k_scroll_and_clamp() {
        // A body with several logical lines so scrolling has room.
        let (mut app, _tmp) = open_phase_detail();
        // The fixture bodies are short ("First"/"Second" have empty bodies), so
        // max_scroll is 0 and scrolling is pinned. Verify the clamp at 0 first.
        app.handle_key(press(KeyCode::Char('k'))).unwrap();
        assert_eq!(phase_scroll(&app), 0);
        app.handle_key(press(KeyCode::Char('j'))).unwrap();
        assert_eq!(phase_scroll(&app), 0);
    }

    #[test]
    fn j_scrolls_down_on_a_long_body() {
        let phases = vec![PhaseDetailView {
            roadmap: "alpha".to_string(),
            number: 1,
            title: "First".to_string(),
            status: PhaseStatus::InProgress,
            completed: None,
            commit: None,
            tags: Vec::new(),
            body: "a\n\nb\n\nc\n\nd\n".to_string(),
        }];
        let tmp = TempDir::new().unwrap();
        let mut app = App::new(FsStore::new(tmp.path()), vec!["demo".to_string()]);
        app.stack.push(Screen::PhaseDetail {
            phases,
            index: 0,
            scroll: 0,
        });
        app.handle_key(press(KeyCode::Char('j'))).unwrap();
        assert_eq!(phase_scroll(&app), 1);
        app.handle_key(press(KeyCode::Char('k'))).unwrap();
        assert_eq!(phase_scroll(&app), 0);
        // Hammer `j` past the end: clamps to logical_lines - 1.
        let max = max_scroll(phase_view(&app));
        for _ in 0..50 {
            app.handle_key(press(KeyCode::Char('j'))).unwrap();
        }
        assert_eq!(phase_scroll(&app), max);
    }

    #[test]
    fn esc_pops_phase_detail_preserving_phase_cursor() {
        let (mut app, _tmp) = populated_app();
        app.enter().unwrap(); // home -> list
        app.enter().unwrap(); // list -> detail
        app.handle_key(press(KeyCode::Down)).unwrap(); // select phase 2
        app.enter().unwrap(); // detail -> phase detail
        app.handle_key(press(KeyCode::Esc)).unwrap(); // back to detail
        let Screen::RoadmapDetail { state, .. } = app.stack.last().unwrap() else {
            panic!("expected RoadmapDetail");
        };
        assert_eq!(state.selected(), Some(1));
    }

    fn phase_index(app: &App) -> usize {
        match app.stack.last().unwrap() {
            Screen::PhaseDetail { index, .. } => *index,
            _ => panic!("expected PhaseDetail"),
        }
    }

    fn phase_scroll(app: &App) -> u16 {
        match app.stack.last().unwrap() {
            Screen::PhaseDetail { scroll, .. } => *scroll,
            _ => panic!("expected PhaseDetail"),
        }
    }

    fn phase_view(app: &App) -> &PhaseDetailView {
        match app.stack.last().unwrap() {
            Screen::PhaseDetail { phases, index, .. } => &phases[*index],
            _ => panic!("expected PhaseDetail"),
        }
    }

    fn set_scroll(app: &mut App, value: u16) {
        match app.stack.last_mut().unwrap() {
            Screen::PhaseDetail { scroll, .. } => *scroll = value,
            _ => panic!("expected PhaseDetail"),
        }
    }

    #[test]
    fn enter_builds_detail_with_body_and_phases() {
        let (mut app, _tmp) = populated_app();
        app.enter().unwrap(); // home -> list (alpha selected)
        app.enter().unwrap(); // list -> detail for alpha
        let Screen::RoadmapDetail { detail, .. } = app.stack.last().unwrap() else {
            panic!("expected RoadmapDetail");
        };
        assert_eq!(detail.slug, "alpha");
        assert_eq!(detail.title, "Alpha");
        assert_eq!(detail.body, "Alpha body.\n");
        assert_eq!(detail.phases.len(), 2);
        assert_eq!(detail.phases[0].number, 1);
        assert_eq!(detail.phases[0].title, "First");
        assert_eq!(detail.phases[0].status, PhaseStatus::Done);
        assert_eq!(detail.phases[1].number, 2);
        assert_eq!(detail.phases[1].status, PhaseStatus::NotStarted);
    }
}
