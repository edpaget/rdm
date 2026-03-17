use std::path::PathBuf;

use rdm_server::router::build_router;
use rdm_server::state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let plan_root = std::env::var("RDM_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            eprintln!("RDM_ROOT not set — using current directory");
            PathBuf::from(".")
        });

    let state = AppState { plan_root };
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
