use std::path::PathBuf;

use anyhow::{Context, Result};

/// Starts the rdm REST API server.
///
/// # Errors
///
/// Returns an error if the quick filters are invalid, the tokio runtime
/// cannot be created, the listener cannot bind, or the server errors.
pub fn run(
    root: PathBuf,
    repo_config: &rdm_core::config::Config,
    port: u16,
    bind: String,
    quick_filter: Vec<String>,
) -> Result<()> {
    let quick_filters =
        resolve_quick_filters(repo_config, &quick_filter).context("invalid quick filter")?;
    let rt = tokio::runtime::Runtime::new().context("failed to create tokio runtime")?;
    rt.block_on(async {
        let state = rdm_server::state::AppState {
            plan_root: root.clone(),
            quick_filters,
        };
        let app = rdm_server::router::build_router(state);
        let addr = format!("{bind}:{port}");
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .with_context(|| format!("failed to bind to {addr}"))?;
        let local_addr = listener.local_addr()?;
        eprintln!("rdm serve listening on http://{local_addr}");
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal())
            .await
            .context("server error")?;
        Ok::<(), anyhow::Error>(())
    })?;
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

/// Resolve the final list of quick filters using the precedence chain:
/// CLI `--quick-filter` flags > `RDM_SERVER_QUICK_FILTERS` env > `[server.quick_filters]`
/// in `rdm.toml`. Higher-precedence sources fully replace lower ones; they do
/// not merge.
fn resolve_quick_filters(
    repo_config: &rdm_core::config::Config,
    cli_flags: &[String],
) -> Result<Vec<rdm_core::config::QuickFilter>> {
    use rdm_core::config::{QuickFilter, parse_quick_filters_env};

    if !cli_flags.is_empty() {
        let mut out = Vec::with_capacity(cli_flags.len());
        for raw in cli_flags {
            let (label, tag) = raw
                .split_once(':')
                .with_context(|| format!("--quick-filter '{raw}': expected 'Label:tag'"))?;
            let label = label.trim();
            let tag = tag.trim();
            if label.is_empty() || tag.is_empty() {
                anyhow::bail!("--quick-filter '{raw}': label and tag must be non-empty");
            }
            out.push(QuickFilter {
                label: label.to_string(),
                tag: tag.to_string(),
            });
        }
        return Ok(out);
    }

    if let Ok(env_value) = std::env::var("RDM_SERVER_QUICK_FILTERS")
        && !env_value.trim().is_empty()
    {
        return parse_quick_filters_env(&env_value).context("RDM_SERVER_QUICK_FILTERS");
    }

    Ok(repo_config
        .server
        .as_ref()
        .map(|s| s.quick_filters.clone())
        .unwrap_or_default())
}

#[cfg(test)]
mod quick_filter_precedence {
    use super::resolve_quick_filters;
    use rdm_core::config::{Config, QuickFilter, ServerConfig};

    const ENV_KEY: &str = "RDM_SERVER_QUICK_FILTERS";

    /// Builds a repo config with two `[server.quick_filters]` entries.
    fn repo_with_filters() -> Config {
        Config {
            server: Some(ServerConfig {
                quick_filters: vec![
                    QuickFilter {
                        label: "TomlA".into(),
                        tag: "a".into(),
                    },
                    QuickFilter {
                        label: "TomlB".into(),
                        tag: "b".into(),
                    },
                ],
            }),
            ..Default::default()
        }
    }

    // These tests mutate the process env, so they cannot run in parallel
    // safely. We serialize with a static mutex.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        use std::sync::{Mutex, OnceLock};
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    #[test]
    fn toml_only_used_when_no_env_no_flags() {
        let _g = env_lock();
        // SAFETY: tests in this mod serialize on env_lock() above.
        unsafe { std::env::remove_var(ENV_KEY) };
        let resolved = resolve_quick_filters(&repo_with_filters(), &[]).unwrap();
        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0].label, "TomlA");
    }

    #[test]
    fn env_overrides_toml() {
        let _g = env_lock();
        // SAFETY: tests in this mod serialize on env_lock() above.
        unsafe { std::env::set_var(ENV_KEY, "EnvA:e1,EnvB:e2") };
        let resolved = resolve_quick_filters(&repo_with_filters(), &[]).unwrap();
        // SAFETY: tests in this mod serialize on env_lock() above.
        unsafe { std::env::remove_var(ENV_KEY) };
        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0].label, "EnvA");
        assert_eq!(resolved[0].tag, "e1");
    }

    #[test]
    fn cli_flags_override_env_and_toml() {
        let _g = env_lock();
        // SAFETY: tests in this mod serialize on env_lock() above.
        unsafe { std::env::set_var(ENV_KEY, "EnvA:e1") };
        let flags = vec!["FlagA:f1".to_string(), "FlagB:f2".to_string()];
        let resolved = resolve_quick_filters(&repo_with_filters(), &flags).unwrap();
        // SAFETY: tests in this mod serialize on env_lock() above.
        unsafe { std::env::remove_var(ENV_KEY) };
        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0].label, "FlagA");
        assert_eq!(resolved[1].tag, "f2");
    }

    #[test]
    fn empty_env_falls_through_to_toml() {
        let _g = env_lock();
        // SAFETY: tests in this mod serialize on env_lock() above.
        unsafe { std::env::set_var(ENV_KEY, "") };
        let resolved = resolve_quick_filters(&repo_with_filters(), &[]).unwrap();
        // SAFETY: tests in this mod serialize on env_lock() above.
        unsafe { std::env::remove_var(ENV_KEY) };
        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0].label, "TomlA");
    }

    #[test]
    fn invalid_cli_flag_returns_error() {
        let _g = env_lock();
        let result = resolve_quick_filters(&Config::default(), &["BadFlag".to_string()]);
        assert!(result.is_err());
    }

    #[test]
    fn empty_label_in_cli_flag_rejected() {
        let _g = env_lock();
        let result = resolve_quick_filters(&Config::default(), &[":bug".to_string()]);
        assert!(result.is_err());
    }
}
