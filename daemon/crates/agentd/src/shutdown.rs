use tracing::info;

/// Wait for a termination signal and resolve once shutdown should begin.
pub async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        let mut sigterm = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
                info!("shutdown signal received (ctrl_c fallback)");
                return;
            }
        };

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("shutdown signal received (ctrl_c)");
            }
            _ = sigterm.recv() => {
                info!("shutdown signal received (sigterm)");
            }
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
        info!("shutdown signal received (ctrl_c)");
    }
}
