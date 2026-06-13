//! `rdm-tui` — a terminal UI for browsing rdm plan repos.
//!
//! This is a thin presentation layer over `rdm-core`: it resolves the plan
//! repo root the same way `rdm-cli` does, loads data through `rdm-store-fs`,
//! and renders it with [ratatui]. No roadmap or task logic lives here.
//!
//! # Framework
//!
//! The UI is built on [ratatui] (the maintained successor to tui-rs) with the
//! crossterm backend it re-exports as [`ratatui::crossterm`], so there is no
//! direct crossterm dependency to keep in sync. The event loop is synchronous:
//! it blocks on [`ratatui::crossterm::event::read`] and redraws once per event,
//! which is plenty for a keyboard-driven browser and avoids pulling in an async
//! runtime. [`ratatui::init`] installs a panic hook that restores the terminal
//! (raw mode off, alternate screen left) before the panic message prints, so a
//! crash never leaves the user's terminal wedged.
//!
//! [ratatui]: https://ratatui.rs

use std::process;

use anyhow::{Context, Result};
use ratatui::crossterm::event::{self, Event};
use rdm_core::ops::project::list_projects;
use rdm_store_fs::FsStore;

mod app;
mod paths;
mod ui;
mod view;

use app::App;

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err:#}");
        process::exit(1);
    }
}

fn run() -> Result<()> {
    // Resolve the root and load projects *before* entering the alternate
    // screen, so any error prints on the normal terminal rather than being
    // wiped by terminal restore.
    let root = paths::resolve_root()?;
    let store = FsStore::new(&root);
    let projects = list_projects(&store)
        .with_context(|| format!("failed to list projects in {}", root.display()))?;
    let mut app = App::new(store, projects);

    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &mut app);
    ratatui::restore();
    result
}

/// Runs the draw/read loop until the app requests to quit.
///
/// # Errors
///
/// Returns an error if drawing a frame or reading a terminal event fails.
fn event_loop(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> Result<()> {
    while !app.should_quit {
        terminal
            .draw(|frame| ui::render(frame, app))
            .context("failed to draw frame")?;
        if let Event::Key(key) = event::read().context("failed to read terminal event")? {
            // A load error while drilling in is shown in the footer rather than
            // aborting the loop, so the user stays where they are.
            if let Err(err) = app.handle_key(key) {
                app.status_message = Some(format!("{err:#}"));
            }
        }
    }
    Ok(())
}
