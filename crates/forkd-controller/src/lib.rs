//! `forkd-controller` library — daemon plumbing (HTTP server, registry).
//!
//! Binary in `src/main.rs` parses CLI args and calls [`run_daemon`].
//! Library shape lets us write integration tests in `tests/`.
pub mod api;
pub mod audit;
pub mod auth;
pub mod http;
pub mod state;

use anyhow::{Context, Result};
use axum::middleware;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use crate::audit::AuditSink;
use crate::auth::AuthConfig;
use crate::http::AppState;
use crate::state::Registry;

#[derive(Debug, Clone)]
pub struct DaemonConfig {
    pub bind: SocketAddr,
    pub state_file: PathBuf,
    /// Root directory under which `<tag>/vmstate` and `<tag>/memory.bin`
    /// live for each tagged snapshot. Falls back to the canonical
    /// XDG location (`~/.local/share/forkd/snapshots/`) if unset.
    pub snapshot_root: PathBuf,
    /// Path to the audit log file (one JSON line per request, appended).
    pub audit_log: PathBuf,
    /// Optional path to a file whose contents are the daemon's bearer
    /// token. When `None`, the daemon runs unauthenticated — safe only
    /// for loopback-bound, single-tenant developer setups.
    pub token_file: Option<PathBuf>,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:8889".parse().unwrap(),
            state_file: PathBuf::from("/var/lib/forkd/state.json"),
            snapshot_root: forkd_vmm::paths::data_dir().join("snapshots"),
            audit_log: PathBuf::from("/var/log/forkd/audit.log"),
            token_file: None,
        }
    }
}

/// Bring up the controller daemon. Blocks until the listener exits.
/// SIGTERM and SIGINT trigger a graceful shutdown.
pub async fn run_daemon(cfg: DaemonConfig) -> Result<()> {
    let registry = Registry::load_or_init(&cfg.state_file)
        .with_context(|| format!("load state from {}", cfg.state_file.display()))?;
    let pruned = registry.reconcile()?;
    if pruned > 0 {
        tracing::info!(pruned, "reconciled stale sandbox entries on startup");
    }

    let audit = AuditSink::open(&cfg.audit_log)
        .with_context(|| format!("open audit log {}", cfg.audit_log.display()))?;
    tracing::info!(audit_log = %audit.path().display(), "audit log open");

    let auth_cfg = match &cfg.token_file {
        Some(p) => {
            let raw = std::fs::read_to_string(p)
                .with_context(|| format!("read token file {}", p.display()))?;
            let tok = raw.trim().to_string();
            if tok.is_empty() {
                anyhow::bail!("token file {} is empty", p.display());
            }
            tracing::info!(token_file = %p.display(), "bearer-token auth enabled");
            AuthConfig::with_token(tok)
        }
        None => {
            if !cfg.bind.ip().is_loopback() {
                tracing::warn!(
                    bind = %cfg.bind,
                    "daemon is bound to a non-loopback address with no --token-file; \
                     this is INSECURE for multi-tenant or networked use"
                );
            }
            AuthConfig::open()
        }
    };

    let app_state = Arc::new(AppState {
        registry,
        live_vms: Mutex::new(HashMap::new()),
        snapshot_root: cfg.snapshot_root.clone(),
    });

    let auth_layer_cfg = auth_cfg.clone();
    let audit_clone = audit.clone();
    let app = http::router(app_state)
        .layer(middleware::from_fn(move |req, next| {
            let cfg = auth_layer_cfg.clone();
            async move { auth::require_token(cfg, req, next).await }
        }))
        .layer(middleware::from_fn(move |req, next| {
            let sink = audit_clone.clone();
            async move { audit::audit_layer(sink, req, next).await }
        }));

    let listener = tokio::net::TcpListener::bind(cfg.bind)
        .await
        .with_context(|| format!("bind {}", cfg.bind))?;
    let actual = listener.local_addr().context("read back bound address")?;
    tracing::info!(addr = %actual, "forkd-controller listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("axum serve")?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("install ctrl_c handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => tracing::info!("received SIGINT, shutting down"),
        _ = terminate => tracing::info!("received SIGTERM, shutting down"),
    }
}
