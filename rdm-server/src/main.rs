use std::path::PathBuf;

use anyhow::{Context, Result};
use rdm_server::router::build_router;
use rdm_server::state::AppState;

/// Expand a path by resolving `~` to `$HOME` and normalizing `.`/`..` segments.
fn expand_root(path: PathBuf) -> Result<PathBuf> {
    let path = if let Ok(rest) = path.strip_prefix("~") {
        let home = std::env::var("HOME").context("~ used in path but $HOME is not set")?;
        PathBuf::from(home).join(rest)
    } else {
        path
    };
    let abs = std::path::absolute(&path)
        .with_context(|| format!("failed to resolve path: {}", path.display()))?;
    let mut normalized = PathBuf::new();
    for component in abs.components() {
        match component {
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::CurDir => {}
            c => normalized.push(c),
        }
    }
    Ok(normalized)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let plan_root = std::env::var("RDM_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            eprintln!("RDM_ROOT not set — using current directory");
            PathBuf::from(".")
        });
    let plan_root = expand_root(plan_root)?;

    let state = AppState {
        plan_root,
        quick_filters: Vec::new(),
    };
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
    eprintln!("rdm-server listening on http://127.0.0.1:3000");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();

    #[cfg(unix)]
    {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler");
        tokio::select! {
            _ = ctrl_c => {},
            _ = sigterm.recv() => {},
        }
    }

    #[cfg(not(unix))]
    {
        ctrl_c.await.ok();
    }

    eprintln!("\nShutting down gracefully...");
}
