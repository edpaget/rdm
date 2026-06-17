//! Application state and event handling for the TUI.
//!
//! Navigation is modeled as an explicit stack of [`Screen`]s on [`App`].
//! Drilling in (Enter) pushes a new screen; backing out (Esc/`h`) pops. Because
//! the underlying screen stays on the stack, its cursor position is preserved
//! automatically when the user returns.

use anyhow::{Context, Result};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::widgets::ListState;
use rdm_core::document::Document;
use rdm_core::io::load_roadmap;
use rdm_core::model::{PhaseStatus, Task, TaskStatus, TaskStatusFilter};
use rdm_core::ops::phase::list_phases;
use rdm_core::ops::roadmap::{computed_status, list_roadmaps};
use rdm_core::ops::task::{TaskFilter, list_tasks, task_matches};
use rdm_store_fs::FsStore;

use crate::view::{
    PhaseDetailView, PhaseRow, RoadmapDetail, RoadmapRow, TaskDetailView, build_phase_details,
    task_detail_view,
};

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
    /// A per-project task list with status and tag quick filters.
    ///
    /// `tasks` is the full unfiltered list, loaded once on entry; the visible
    /// rows are derived each frame via [`filtered_indices`]. `status` and
    /// `selected_tags` are the active filters; `available_tags` is the sorted
    /// union of every task's tags, used to populate the tag-filter popup.
    TaskList {
        /// Project whose tasks are shown.
        project: String,
        /// Every task in the project, sorted by slug. Filtered for display.
        tasks: Vec<(String, Document<Task>)>,
        /// Active status filter (`All` shows every status).
        status: TaskStatusFilter,
        /// Sorted union of all tags across `tasks`.
        available_tags: Vec<String>,
        /// Tags currently filtered on (AND). Empty imposes no tag constraint.
        selected_tags: Vec<String>,
        /// The tag-filter popup, present only while open.
        popup: Option<TagPopup>,
        /// Selection state, indexing into the filtered rows.
        state: ListState,
    },
    /// A single task's metadata and markdown-rendered body. A leaf screen.
    TaskDetail {
        /// The task detail being shown.
        view: TaskDetailView,
        /// Vertical scroll offset into the rendered body.
        scroll: u16,
    },
}

/// Transient state for the tag-filter popup overlaid on [`Screen::TaskList`].
///
/// `pending` holds the in-progress tag selection while the popup is open; it is
/// applied to the screen's `selected_tags` on Enter and discarded on Esc.
pub struct TagPopup {
    /// Tags toggled on so far, applied only when the user confirms.
    pub pending: Vec<String>,
    /// Selection state, indexing into the task list's `available_tags`.
    pub state: ListState,
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

        // Quitting always wins, even over the tag popup.
        if (ctrl && key.code == KeyCode::Char('c')) || key.code == KeyCode::Char('q') {
            self.should_quit = true;
            return Ok(());
        }

        // While the tag-filter popup is open it captures every other key.
        if matches!(
            self.stack.last(),
            Some(Screen::TaskList { popup: Some(_), .. })
        ) {
            self.handle_popup_key(key);
            return Ok(());
        }

        match key.code {
            KeyCode::Esc | KeyCode::Char('h') => self.back(),
            KeyCode::Enter => self.enter()?,
            // `t`/`r` switch between the task and roadmap lists for a project.
            KeyCode::Char('t') => self.switch_to_tasks()?,
            KeyCode::Char('r') => self.switch_to_roadmaps()?,
            // Task-list quick filters.
            KeyCode::Char('s') if matches!(self.stack.last(), Some(Screen::TaskList { .. })) => {
                self.cycle_status();
            }
            KeyCode::Char('f') if matches!(self.stack.last(), Some(Screen::TaskList { .. })) => {
                self.open_tag_popup();
            }
            // Movement is screen-specific: the detail screens scroll their body
            // (phase detail also cycles phases); list screens move a cursor.
            _ if matches!(self.stack.last(), Some(Screen::PhaseDetail { .. })) => {
                self.handle_phase_key(key, ctrl);
            }
            _ if matches!(self.stack.last(), Some(Screen::TaskDetail { .. })) => {
                self.handle_task_detail_key(key, ctrl);
            }
            KeyCode::Down | KeyCode::Char('j') => self.select_next(),
            KeyCode::Up | KeyCode::Char('k') => self.select_previous(),
            _ => {}
        }
        Ok(())
    }

    /// Handles a key while the tag-filter popup is open on [`Screen::TaskList`].
    ///
    /// `j`/`Down` and `k`/`Up` move over `available_tags`; `Space` toggles the
    /// highlighted tag in the pending selection; `Enter` applies the pending
    /// selection (re-clamping the cursor over the new filtered set) and closes
    /// the popup; `Esc` discards the pending selection and closes the popup.
    fn handle_popup_key(&mut self, key: KeyEvent) {
        let Some(Screen::TaskList {
            tasks,
            status,
            selected_tags,
            available_tags,
            popup,
            state,
            ..
        }) = self.stack.last_mut()
        else {
            return;
        };
        let Some(pop) = popup.as_mut() else {
            return;
        };
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => list_next(&mut pop.state, available_tags.len()),
            KeyCode::Up | KeyCode::Char('k') => list_prev(&mut pop.state, available_tags.len()),
            KeyCode::Char(' ') => {
                if let Some(tag) = pop.state.selected().and_then(|i| available_tags.get(i)) {
                    if let Some(pos) = pop.pending.iter().position(|t| t == tag) {
                        pop.pending.remove(pos);
                    } else {
                        pop.pending.push(tag.clone());
                    }
                }
            }
            KeyCode::Enter => {
                *selected_tags = pop.pending.clone();
                *popup = None;
                let len = filtered_indices(tasks, *status, selected_tags).len();
                reclamp(state, len);
            }
            KeyCode::Esc => {
                *popup = None;
            }
            _ => {}
        }
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

    /// Handles a scroll key while a [`Screen::TaskDetail`] is active.
    ///
    /// Mirrors [`handle_phase_key`](App::handle_phase_key) without prev/next:
    /// `j`/`Down` and `k`/`Up` scroll one line; `PageDown`/`PageUp` and
    /// `Ctrl-d`/`Ctrl-u` scroll by a (half-)page, clamping to the body's last
    /// logical line.
    fn handle_task_detail_key(&mut self, key: KeyEvent, ctrl: bool) {
        /// Page size used by `PageUp`/`PageDown`; half of it for `Ctrl-u`/`Ctrl-d`.
        const PAGE: u16 = 20;
        let Some(Screen::TaskDetail { view, scroll }) = self.stack.last_mut() else {
            return;
        };
        let max = max_scroll_body(&view.body);
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => *scroll = scroll.saturating_add(1).min(max),
            KeyCode::Up | KeyCode::Char('k') => *scroll = scroll.saturating_sub(1),
            KeyCode::PageDown => *scroll = scroll.saturating_add(PAGE).min(max),
            KeyCode::PageUp => *scroll = scroll.saturating_sub(PAGE),
            KeyCode::Char('d') if ctrl => *scroll = scroll.saturating_add(PAGE / 2).min(max),
            KeyCode::Char('u') if ctrl => *scroll = scroll.saturating_sub(PAGE / 2),
            _ => {}
        }
    }

    /// Switches to (or opens) the task list for the current project.
    ///
    /// From [`Screen::Home`] this loads the selected project's tasks and pushes
    /// a [`Screen::TaskList`]; from [`Screen::RoadmapList`] it replaces the top
    /// screen with the task list for the same project (preserving `Home`
    /// beneath). Elsewhere it is a no-op.
    ///
    /// # Errors
    ///
    /// Returns an error if listing the project's tasks fails.
    fn switch_to_tasks(&mut self) -> Result<()> {
        match self.stack.last().expect("stack is never empty") {
            Screen::Home { state } => {
                let Some(idx) = state.selected() else {
                    return Ok(());
                };
                let Some(project) = self.projects.get(idx).cloned() else {
                    return Ok(());
                };
                let screen = build_task_list(&self.store, project)?;
                self.stack.push(screen);
            }
            Screen::RoadmapList { project, .. } => {
                let project = project.clone();
                let screen = build_task_list(&self.store, project)?;
                self.stack.pop();
                self.stack.push(screen);
            }
            _ => {}
        }
        Ok(())
    }

    /// Switches to the roadmap list for the current project.
    ///
    /// From [`Screen::Home`] this behaves like [`enter`](App::enter), pushing a
    /// [`Screen::RoadmapList`]; from [`Screen::TaskList`] it replaces the top
    /// screen with the roadmap list for the same project. Elsewhere it is a
    /// no-op.
    ///
    /// # Errors
    ///
    /// Returns an error if listing the project's roadmaps/phases fails.
    fn switch_to_roadmaps(&mut self) -> Result<()> {
        match self.stack.last().expect("stack is never empty") {
            Screen::Home { .. } => self.enter()?,
            Screen::TaskList { project, .. } => {
                let project = project.clone();
                let rows = load_roadmap_rows(&self.store, &project)?;
                let state = initial_state(rows.len());
                self.stack.pop();
                self.stack.push(Screen::RoadmapList {
                    project,
                    rows,
                    state,
                });
            }
            _ => {}
        }
        Ok(())
    }

    /// Advances the task-list status filter to the next value in the cycle
    /// `All → Open → InProgress → Done → WontFix → All`, then re-clamps the
    /// cursor over the new filtered set. No-op off [`Screen::TaskList`].
    fn cycle_status(&mut self) {
        let Some(Screen::TaskList {
            tasks,
            status,
            selected_tags,
            state,
            ..
        }) = self.stack.last_mut()
        else {
            return;
        };
        *status = next_status(*status);
        let len = filtered_indices(tasks, *status, selected_tags).len();
        reclamp(state, len);
    }

    /// Opens the tag-filter popup, seeding its pending selection from the
    /// currently applied tags. No-op off [`Screen::TaskList`].
    fn open_tag_popup(&mut self) {
        let Some(Screen::TaskList {
            available_tags,
            selected_tags,
            popup,
            ..
        }) = self.stack.last_mut()
        else {
            return;
        };
        let state = initial_state(available_tags.len());
        *popup = Some(TagPopup {
            pending: selected_tags.clone(),
            state,
        });
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
            Screen::TaskList {
                tasks,
                status,
                selected_tags,
                state,
                ..
            } => {
                let Some(sel) = state.selected() else {
                    return Ok(());
                };
                let indices = filtered_indices(tasks, *status, selected_tags);
                let Some(&orig) = indices.get(sel) else {
                    return Ok(());
                };
                let view = task_detail_view(&tasks[orig]);
                self.stack.push(Screen::TaskDetail { view, scroll: 0 });
            }
            Screen::PhaseDetail { .. } | Screen::TaskDetail { .. } => {}
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
            Screen::TaskList {
                tasks,
                status,
                selected_tags,
                ..
            } => filtered_indices(tasks, *status, selected_tags).len(),
            Screen::PhaseDetail { .. } | Screen::TaskDetail { .. } => 0,
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
            Screen::TaskList { state, .. } => state,
            Screen::PhaseDetail { .. } | Screen::TaskDetail { .. } => {
                unreachable!("detail-screen movement does not use a list cursor")
            }
        }
    }

    /// Moves the active screen's selection down by one, saturating at the end.
    fn select_next(&mut self) {
        let len = self.active_len();
        let state = self.active_state_mut();
        list_next(state, len);
    }

    /// Moves the active screen's selection up by one, saturating at the start.
    fn select_previous(&mut self) {
        let len = self.active_len();
        let state = self.active_state_mut();
        list_prev(state, len);
    }
}

/// Moves a list selection down by one over `len` items, saturating at the end.
///
/// No-op when `len` is zero. A `None` selection becomes the first item.
fn list_next(state: &mut ListState, len: usize) {
    if len == 0 {
        return;
    }
    let next = match state.selected() {
        Some(i) if i + 1 < len => i + 1,
        Some(i) => i,
        None => 0,
    };
    state.select(Some(next));
}

/// Moves a list selection up by one, saturating at the start.
///
/// No-op when `len` is zero. A `None` selection becomes the first item.
fn list_prev(state: &mut ListState, len: usize) {
    if len == 0 {
        return;
    }
    let prev = state.selected().map_or(0, |i| i.saturating_sub(1));
    state.select(Some(prev));
}

/// Re-clamps a selection after the visible item count changes to `len`.
///
/// Selects `None` when empty; otherwise keeps the current index, capped to the
/// last item, defaulting to the first when nothing was selected.
fn reclamp(state: &mut ListState, len: usize) {
    if len == 0 {
        state.select(None);
    } else {
        let sel = state.selected().unwrap_or(0).min(len - 1);
        state.select(Some(sel));
    }
}

/// Advances the task-list status filter through its cycle.
///
/// The cycle deliberately covers only the common states
/// (`All → Open → InProgress → Done → WontFix → All`); the review states fold
/// back to `All`.
fn next_status(status: TaskStatusFilter) -> TaskStatusFilter {
    use TaskStatusFilter::{All, Status};
    match status {
        All => Status(TaskStatus::Open),
        Status(TaskStatus::Open) => Status(TaskStatus::InProgress),
        Status(TaskStatus::InProgress) => Status(TaskStatus::Done),
        Status(TaskStatus::Done) => Status(TaskStatus::WontFix),
        Status(_) => All,
    }
}

/// Indices into `tasks` whose tasks satisfy the active status and tag filters.
///
/// Operates by reference so no task bodies are cloned per frame. The returned
/// indices map a filtered-list position back to its original task.
pub(crate) fn filtered_indices(
    tasks: &[(String, Document<Task>)],
    status: TaskStatusFilter,
    selected_tags: &[String],
) -> Vec<usize> {
    let filter = TaskFilter {
        status: Some(status),
        priority: None,
        tags: selected_tags.to_vec(),
    };
    tasks
        .iter()
        .enumerate()
        .filter(|(_, (_, doc))| task_matches(&doc.frontmatter, &filter))
        .map(|(i, _)| i)
        .collect()
}

/// Loads a project's tasks into a [`Screen::TaskList`] with no filters applied
/// (status `All`, no tags) and the first visible row selected.
fn build_task_list(store: &FsStore, project: String) -> Result<Screen> {
    let tasks = list_tasks(store, &project)
        .with_context(|| format!("failed to list tasks for project '{project}'"))?;
    let available_tags = collect_tags(&tasks);
    let status = TaskStatusFilter::All;
    let len = filtered_indices(&tasks, status, &[]).len();
    Ok(Screen::TaskList {
        project,
        tasks,
        status,
        available_tags,
        selected_tags: Vec::new(),
        popup: None,
        state: initial_state(len),
    })
}

/// Collects the sorted, de-duplicated union of all tags across `tasks`.
fn collect_tags(tasks: &[(String, Document<Task>)]) -> Vec<String> {
    let mut tags: Vec<String> = tasks
        .iter()
        .filter_map(|(_, doc)| doc.frontmatter.tags.as_ref())
        .flatten()
        .cloned()
        .collect();
    tags.sort();
    tags.dedup();
    tags
}

/// Maximum scroll offset for a phase body: `logical_lines - 1`.
///
/// Clamping to this (rather than to a viewport-relative value) guarantees the
/// last rendered line can always reach the top of the body pane, so every line
/// stays reachable even after the widget re-wraps to a narrow width.
fn max_scroll(view: &PhaseDetailView) -> u16 {
    max_scroll_body(&view.body)
}

/// Maximum scroll offset for a markdown body: `logical_lines - 1`.
///
/// Shared by the phase- and task-detail screens; see [`max_scroll`] for why the
/// clamp is line-based rather than viewport-relative.
fn max_scroll_body(body: &str) -> u16 {
    let lines = crate::markdown::render_markdown(body).lines.len();
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
            rdm_core::ops::TagsUpdate::Keep,
            rdm_core::ops::BodyUpdate::Keep,
            None,
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

    // --- Task list / detail ------------------------------------------------

    use rdm_core::model::Priority;
    use rdm_core::ops::task::{create_task, update_task};

    /// A store with one project "demo" holding four tasks spanning statuses and
    /// tags (sorted by slug: t1, t2, t3, t4):
    /// - `t1`: open, tags [bug]
    /// - `t2`: in-progress, tags [ui]
    /// - `t3`: done, tags [bug, ui]
    /// - `t4`: wont-fix, no tags
    fn task_app() -> (App, TempDir) {
        let tmp = TempDir::new().unwrap();
        let mut store = FsStore::new(tmp.path());
        create_project(&mut store, "demo", "Demo").unwrap();

        create_task(
            &mut store,
            "demo",
            "t1",
            "Task one",
            Priority::Medium,
            Some(vec!["bug".to_string()]),
            Some("Body of t1."),
        )
        .unwrap();
        create_task(
            &mut store,
            "demo",
            "t2",
            "Task two",
            Priority::High,
            Some(vec!["ui".to_string()]),
            None,
        )
        .unwrap();
        update_task(
            &mut store,
            "demo",
            "t2",
            Some(TaskStatus::InProgress),
            None,
            rdm_core::ops::TagsUpdate::Keep,
            rdm_core::ops::BodyUpdate::Keep,
            None,
        )
        .unwrap();
        create_task(
            &mut store,
            "demo",
            "t3",
            "Task three",
            Priority::Low,
            Some(vec!["bug".to_string(), "ui".to_string()]),
            None,
        )
        .unwrap();
        update_task(
            &mut store,
            "demo",
            "t3",
            Some(TaskStatus::Done),
            None,
            rdm_core::ops::TagsUpdate::Keep,
            rdm_core::ops::BodyUpdate::Keep,
            None,
        )
        .unwrap();
        create_task(
            &mut store,
            "demo",
            "t4",
            "Task four",
            Priority::Medium,
            None,
            None,
        )
        .unwrap();
        update_task(
            &mut store,
            "demo",
            "t4",
            Some(TaskStatus::WontFix),
            None,
            rdm_core::ops::TagsUpdate::Keep,
            rdm_core::ops::BodyUpdate::Keep,
            None,
        )
        .unwrap();

        let app = App::new(store, vec!["demo".to_string()]);
        (app, tmp)
    }

    fn visible_slugs(app: &App) -> Vec<String> {
        match app.stack.last().unwrap() {
            Screen::TaskList {
                tasks,
                status,
                selected_tags,
                ..
            } => filtered_indices(tasks, *status, selected_tags)
                .into_iter()
                .map(|i| tasks[i].0.clone())
                .collect(),
            _ => panic!("expected TaskList"),
        }
    }

    #[test]
    fn t_from_home_pushes_task_list() {
        let (mut app, _tmp) = task_app();
        app.handle_key(press(KeyCode::Char('t'))).unwrap();
        assert_eq!(app.stack.len(), 2);
        let Screen::TaskList {
            tasks,
            status,
            available_tags,
            selected_tags,
            ..
        } = app.stack.last().unwrap()
        else {
            panic!("expected TaskList");
        };
        assert_eq!(tasks.len(), 4);
        assert_eq!(*status, TaskStatusFilter::All);
        assert_eq!(available_tags, &vec!["bug".to_string(), "ui".to_string()]);
        assert!(selected_tags.is_empty());
        // All four visible under the All filter.
        assert_eq!(visible_slugs(&app), vec!["t1", "t2", "t3", "t4"]);
    }

    #[test]
    fn s_cycles_status_and_filters() {
        let (mut app, _tmp) = task_app();
        app.handle_key(press(KeyCode::Char('t'))).unwrap();
        // All -> Open
        app.handle_key(press(KeyCode::Char('s'))).unwrap();
        assert_eq!(visible_slugs(&app), vec!["t1"]);
        // Open -> InProgress
        app.handle_key(press(KeyCode::Char('s'))).unwrap();
        assert_eq!(visible_slugs(&app), vec!["t2"]);
        // InProgress -> Done
        app.handle_key(press(KeyCode::Char('s'))).unwrap();
        assert_eq!(visible_slugs(&app), vec!["t3"]);
        // Done -> WontFix
        app.handle_key(press(KeyCode::Char('s'))).unwrap();
        assert_eq!(visible_slugs(&app), vec!["t4"]);
        // WontFix -> All
        app.handle_key(press(KeyCode::Char('s'))).unwrap();
        assert_eq!(visible_slugs(&app), vec!["t1", "t2", "t3", "t4"]);
    }

    #[test]
    fn status_cycle_reclamps_cursor() {
        let (mut app, _tmp) = task_app();
        app.handle_key(press(KeyCode::Char('t'))).unwrap();
        // Move cursor to the 4th row, then narrow to Open (one row): clamps to 0.
        app.handle_key(press(KeyCode::Char('j'))).unwrap();
        app.handle_key(press(KeyCode::Char('j'))).unwrap();
        app.handle_key(press(KeyCode::Char('j'))).unwrap();
        app.handle_key(press(KeyCode::Char('s'))).unwrap(); // All -> Open
        let Screen::TaskList { state, .. } = app.stack.last().unwrap() else {
            panic!("expected TaskList");
        };
        assert_eq!(state.selected(), Some(0));
    }

    #[test]
    fn tag_popup_toggle_and_apply_filters() {
        let (mut app, _tmp) = task_app();
        app.handle_key(press(KeyCode::Char('t'))).unwrap();
        app.handle_key(press(KeyCode::Char('f'))).unwrap(); // open popup
        assert!(matches!(
            app.stack.last().unwrap(),
            Screen::TaskList { popup: Some(_), .. }
        ));
        // Toggle "bug" (first available tag) and apply.
        app.handle_key(press(KeyCode::Char(' '))).unwrap();
        app.handle_key(press(KeyCode::Enter)).unwrap();
        let Screen::TaskList {
            popup,
            selected_tags,
            ..
        } = app.stack.last().unwrap()
        else {
            panic!("expected TaskList");
        };
        assert!(popup.is_none());
        assert_eq!(selected_tags, &vec!["bug".to_string()]);
        // bug appears on t1 and t3 (status All).
        assert_eq!(visible_slugs(&app), vec!["t1", "t3"]);
    }

    #[test]
    fn tag_popup_cancel_discards_pending() {
        let (mut app, _tmp) = task_app();
        app.handle_key(press(KeyCode::Char('t'))).unwrap();
        app.handle_key(press(KeyCode::Char('f'))).unwrap();
        app.handle_key(press(KeyCode::Char(' '))).unwrap(); // toggle bug in pending
        app.handle_key(press(KeyCode::Esc)).unwrap(); // cancel
        let Screen::TaskList {
            popup,
            selected_tags,
            ..
        } = app.stack.last().unwrap()
        else {
            panic!("expected TaskList");
        };
        assert!(popup.is_none());
        assert!(selected_tags.is_empty());
        assert_eq!(visible_slugs(&app), vec!["t1", "t2", "t3", "t4"]);
    }

    #[test]
    fn enter_on_row_pushes_task_detail_for_right_task() {
        let (mut app, _tmp) = task_app();
        app.handle_key(press(KeyCode::Char('t'))).unwrap();
        // Narrow to Done so the single visible row maps back to t3.
        app.handle_key(press(KeyCode::Char('s'))).unwrap(); // Open
        app.handle_key(press(KeyCode::Char('s'))).unwrap(); // InProgress
        app.handle_key(press(KeyCode::Char('s'))).unwrap(); // Done
        app.handle_key(press(KeyCode::Enter)).unwrap();
        let Screen::TaskDetail { view, scroll } = app.stack.last().unwrap() else {
            panic!("expected TaskDetail");
        };
        assert_eq!(view.slug, "t3");
        assert_eq!(view.status, TaskStatus::Done);
        assert_eq!(*scroll, 0);
    }

    #[test]
    fn esc_from_detail_preserves_cursor_and_filter() {
        let (mut app, _tmp) = task_app();
        app.handle_key(press(KeyCode::Char('t'))).unwrap();
        app.handle_key(press(KeyCode::Char('s'))).unwrap(); // All -> Open
        app.handle_key(press(KeyCode::Enter)).unwrap(); // open t1 detail
        app.handle_key(press(KeyCode::Esc)).unwrap(); // back to list
        let Screen::TaskList { status, state, .. } = app.stack.last().unwrap() else {
            panic!("expected TaskList");
        };
        assert_eq!(*status, TaskStatusFilter::Status(TaskStatus::Open));
        assert_eq!(state.selected(), Some(0));
    }

    #[test]
    fn r_switches_task_list_to_roadmap_list() {
        let (mut app, _tmp) = task_app();
        app.handle_key(press(KeyCode::Char('t'))).unwrap();
        assert_eq!(app.stack.len(), 2);
        app.handle_key(press(KeyCode::Char('r'))).unwrap();
        // Replaced top: still depth 2 with Home beneath.
        assert_eq!(app.stack.len(), 2);
        assert!(matches!(
            app.stack.last().unwrap(),
            Screen::RoadmapList { .. }
        ));
        // And `t` switches back to tasks.
        app.handle_key(press(KeyCode::Char('t'))).unwrap();
        assert_eq!(app.stack.len(), 2);
        assert!(matches!(app.stack.last().unwrap(), Screen::TaskList { .. }));
    }

    #[test]
    fn task_detail_scroll_clamps_at_bounds() {
        let view = TaskDetailView {
            slug: "t1".to_string(),
            title: "Task one".to_string(),
            status: TaskStatus::Open,
            priority: Priority::Medium,
            created: chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            completed: None,
            commit: None,
            tags: Vec::new(),
            body: "a\n\nb\n\nc\n\nd\n".to_string(),
        };
        let tmp = TempDir::new().unwrap();
        let mut app = App::new(FsStore::new(tmp.path()), vec!["demo".to_string()]);
        app.stack.push(Screen::TaskDetail { view, scroll: 0 });
        // k at the top stays at 0.
        app.handle_key(press(KeyCode::Char('k'))).unwrap();
        assert_eq!(task_scroll(&app), 0);
        app.handle_key(press(KeyCode::Char('j'))).unwrap();
        assert_eq!(task_scroll(&app), 1);
        // Hammer j past the end: clamps to the body's last logical line.
        let max = match app.stack.last().unwrap() {
            Screen::TaskDetail { view, .. } => max_scroll_body(&view.body),
            _ => unreachable!(),
        };
        for _ in 0..50 {
            app.handle_key(press(KeyCode::Char('j'))).unwrap();
        }
        assert_eq!(task_scroll(&app), max);
    }

    fn task_scroll(app: &App) -> u16 {
        match app.stack.last().unwrap() {
            Screen::TaskDetail { scroll, .. } => *scroll,
            _ => panic!("expected TaskDetail"),
        }
    }
}
