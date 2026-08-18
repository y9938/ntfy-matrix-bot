mod config;
mod ntfy;
mod matrix;
use std::sync::Arc;
use anyhow::{Context, Result};
use matrix_sdk::{Client, config::SyncSettings, ruma::api::client::filter::FilterDefinition};
use tokio::fs;
use crate::config::Config;
use crate::matrix::{authenticate, on_invite, on_message};
#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "ntfy_matrix_bot=info,matrix_sdk=warn".into())
        .add_directive("matrix_sdk::http_client=off".parse().unwrap())
        .add_directive("matrix_sdk_crypto::backups=error".parse().unwrap());
    tracing_subscriber::fmt().with_env_filter(env_filter).init();
    tracing::info!("ntfy-matrix-bot starting");
    let cfg = Arc::new(Config::load()?);
    let http = Arc::new(
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()?,
    );
    let dir = cfg.data_dir.clone();
    fs::create_dir_all(&dir)
        .await
        .with_context(|| format!("create_dir_all {dir:?}"))?;
    let session_file = dir.join("session.json");
    let db_path = dir.join("matrix-store");
    let homeserver = cfg.matrix_homeserver.clone();
    let client = Client::builder()
        .homeserver_url(&homeserver)
        .sqlite_store(&db_path, None)
        .build()
        .await
        .context("Failed to build Matrix client")?;
    authenticate(&client, &session_file, &cfg).await?;
    // Matrix SDK macro quirk: Register Arc pointers into the client's internal type map.
    // This enables event handlers (e.g., on_message) to extract them via Ctx<T> injection.
    client.add_event_handler_context(Arc::clone(&cfg));
    client.add_event_handler_context(Arc::clone(&http));
    client.add_event_handler(on_invite);
    tracing::info!("Initial sync…");
    let filter = FilterDefinition::with_lazy_loading();
    let sync_settings = SyncSettings::default().filter(filter.into());
    let response = client
        .sync_once(sync_settings.clone())
        .await
        .context("Initial sync failed")?;
    client.add_event_handler(on_message);
    tracing::info!("Bot ready");
    use tokio::signal::unix::{SignalKind, signal};
    let mut sigterm = signal(SignalKind::terminate()).expect("SIGTERM handler");
    let sync_task = client.sync(sync_settings.token(response.next_batch));
    tokio::select! {
        res = sync_task => {
            res.context("Sync loop error")?;
        }
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("SIGINT received — instant shutdown");
        }
        _ = sigterm.recv() => {
            tracing::info!("SIGTERM received — instant shutdown");
        }
    }
    tracing::info!("Clean shutdown complete");
    Ok(())
}
