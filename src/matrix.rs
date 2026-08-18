use std::sync::Arc;
use anyhow::Context;
use matrix_sdk::{
    Client, Room, RoomState, SessionMeta, SessionTokens,
    authentication::matrix::MatrixSession,
    event_handler::Ctx,
    ruma::{
        OwnedDeviceId,
        events::room::{
            member::StrippedRoomMemberEvent,
            message::{MessageType, OriginalSyncRoomMessageEvent, RoomMessageEventContent},
        },
    },
};
use tokio::fs;

use crate::config::Config;
use crate::ntfy::{ProvisionResult, provision_ntfy_user};

pub async fn on_invite(
    room_member: StrippedRoomMemberEvent,
    client: Client,
    room: Room,
    cfg: Ctx<Arc<Config>>,
) {
    let Some(bot_id) = client.user_id() else {
        return;
    };

    // Assumes: Stripped state includes historical members. Filter for bot's own invite event.
    if room_member.state_key != bot_id {
        return;
    }

    let sender = &room_member.sender;
    let is_allowed_user = cfg.matrix_allowed_users.contains(&sender.as_str().to_lowercase());
    let is_allowed_server = cfg.matrix_allowed_homeservers.contains(&sender.server_name().as_str().to_lowercase());

    if !is_allowed_user && !is_allowed_server {
        tracing::warn!(%sender, "Unauthorized invite attempt — rejecting");
        if let Err(e) = room.leave().await {
            tracing::error!("Failed to reject unauthorized invite: {}", e);
        }
        return;
    }

    if !room_member.content.is_direct.unwrap_or(false) {
        tracing::info!(room_id = %room.room_id(), "Non-DM invite — ignoring");
        return;
    }

    let cfg = Arc::clone(&cfg);

    tokio::spawn(async move {
        tracing::info!(room_id = %room.room_id(), "DM invite — joining…");

        let mut delay = 2u64;
        while let Err(e) = room.join().await {
            tracing::warn!(room_id = %room.room_id(), delay, "join failed: {e}");
            tokio::time::sleep(tokio::time::Duration::from_secs(delay)).await;
            delay = (delay * 2).min(60);
        }

        // Workaround: Force server sync (E2EE 'Unknown' post-sync). Prevents plaintext data leaks.
        let enc = match room.latest_encryption_state().await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(room_id = %room.room_id(), "encryption state fetch failed: {e}");
                room.encryption_state()
            }
        };

        if !enc.is_encrypted() {
            tracing::warn!(
                room_id = %room.room_id(),
                ?enc,
                "Welcome suppressed — room not E2EE"
            );
            return;
        }

        let welcome = format!(
            "`!create` — provision your ntfy account\n\
             `!help`   — list commands\n\n\
             E2EE only. Unencrypted rooms are ignored.\n\
             {url}",
            url = cfg.ntfy_base_url
        );
        let _ = room
            .send(RoomMessageEventContent::text_markdown(welcome))
            .await;
    });
}

pub async fn on_message(
    event: OriginalSyncRoomMessageEvent,
    room: Room,
    client: Client,
    cfg: Ctx<Arc<Config>>,
    http: Ctx<Arc<reqwest::Client>>,
) {
    if client.user_id().is_some_and(|id| id == event.sender) {
        return;
    }

    let sender = &event.sender;
    let is_allowed_user = cfg.matrix_allowed_users.contains(&sender.as_str().to_lowercase());
    let is_allowed_server = cfg.matrix_allowed_homeservers.contains(&sender.server_name().as_str().to_lowercase());

    if !is_allowed_user && !is_allowed_server {
        tracing::warn!(%sender, room_id = %room.room_id(), "Unauthorized message attempt — topology access denied (silent drop)");
        return;
    }

    if room.state() != RoomState::Joined {
        return;
    }

    if room.active_members_count() > 2 {
        tracing::error!(room_id = %room.room_id(), "Topology violation: Refusing to process commands in multi-user room");
        let _ = room.leave().await;
        return;
    }

    let enc_state = room.encryption_state();
    // Workaround: Force state refresh (E2EE 'Unknown' post-sync). Prevents plaintext data leaks.
    let enc_state = if enc_state.is_unknown() {
        match room.latest_encryption_state().await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(room_id = %room.room_id(), "encryption state refresh failed: {e}");
                enc_state
            }
        }
    } else {
        enc_state
    };

    if !enc_state.is_encrypted() {
        tracing::warn!(
            room_id = %room.room_id(),
            sender = %event.sender,
            ?enc_state,
            "Dropping message — room not E2EE"
        );
        return;
    }

    if !room.is_dm() && !room.compute_is_dm().await.unwrap_or(false) {
        tracing::info!(room_id = %room.room_id(), "Group room — leaving");
        let _ = room
            .send(RoomMessageEventContent::text_plain("DM-only bot. Leaving."))
            .await;
        let _ = room.leave().await;
        return;
    }

    let MessageType::Text(ref text) = event.content.msgtype else {
        return;
    };
    let body = text.body.trim();
    let username = event.sender.localpart().to_lowercase();

    tracing::info!(
        sender = %event.sender,
        room_id = %room.room_id(),
        cmd = body,
        "Message"
    );

    let reply: Option<String> = match body {
        "!create" | "!register" | "!start" => {
            match provision_ntfy_user(&http, &cfg, &username).await {
                Ok(ProvisionResult::Created { password }) => Some(format!(
                    "✓ Account created\n\n\
                     Login:    `{username}`\n\
                     Password: `{password}`\n\
                     Server:   {url}",
                    url = cfg.ntfy_base_url
                )),
                Ok(ProvisionResult::CreatedAclFailed { password }) => Some(format!(
                    "! Account created but topic access setup failed.\n\n\
                     Login:    `{username}`\n\
                     Password: `{password}`\n\
                     Server:   {url}\n\n\
                     Contact admin to grant `up*` read-write access.",
                    url = cfg.ntfy_base_url
                )),
                Ok(ProvisionResult::AlreadyExists) => Some(format!(
                    "! `{username}` already exists — {url}",
                    url = cfg.ntfy_base_url
                )),
                Err(e) => {
                    tracing::error!("provision: {e:#}");
                    Some("✗ Provisioning failed. Try again later.".into())
                }
            }
        }
        "!status" | "!ping" => Some(format!("✓ {url}", url = cfg.ntfy_base_url)),
        "!help" | "!" => Some(format!(
            "`!create` — create ntfy account for `{username}`\n\
             `!status` — bot status\n\n\
             Dashboard: {url}",
            url = cfg.ntfy_base_url
        )),
        _ => None,
    };

    if let Some(msg) = reply {
        if let Err(e) = room.send(RoomMessageEventContent::text_markdown(msg)).await {
            tracing::error!(room_id = %room.room_id(), "send failed: {e}");
        }
    }
}

pub async fn authenticate(
    client: &Client,
    session_file: &std::path::Path,
    cfg: &Config,
) -> anyhow::Result<()> {
    if session_file.exists() {
        tracing::info!("Restoring session from {session_file:?}");
        let raw = fs::read_to_string(session_file).await?;
        let session: MatrixSession =
            serde_json::from_str(&raw).context("Bad session.json — delete and restart")?;
        client
            .restore_session(session)
            .await
            .context("restore_session failed")?;
        return Ok(());
    }

    if let Some(token) = &cfg.matrix_bot_access_token {
        tracing::info!("Auth via access token (whoami)");
        let homeserver = cfg.matrix_homeserver.clone();

        #[derive(serde::Deserialize)]
        struct Whoami {
            user_id: matrix_sdk::ruma::OwnedUserId,
            device_id: Option<OwnedDeviceId>,
        }

        let resp: Whoami = reqwest::Client::new()
            .get(format!(
                "{}/_matrix/client/v3/account/whoami",
                homeserver.trim_end_matches('/')
            ))
            .bearer_auth(&token)
            .send()
            .await
            .context("whoami request failed")?
            .json()
            .await
            .context("whoami parse failed (bad token?)")?;

        let session = MatrixSession {
            meta: SessionMeta {
                user_id: resp.user_id,
                device_id: resp.device_id.unwrap_or_else(|| "NTFYBOT".into()),
            },
            tokens: SessionTokens {
                access_token: token.clone(),
                refresh_token: None,
            },
        };

        client
            .restore_session(session.clone())
            .await
            .context("restore_session (token) failed")?;

        atomic_write(session_file, &serde_json::to_string_pretty(&session)?)
            .await
            .context("write session.json")?;
        tracing::info!("Session saved to {session_file:?}");
        return Ok(());
    }

    if let Some(password) = &cfg.matrix_bot_password {
        let user_id = cfg
            .matrix_bot_user_id
            .as_ref()
            .context("MATRIX_BOT_USER_ID required for password login")?;
        tracing::info!("Auth via password for {user_id}");

        client
            .matrix_auth()
            .login_username(&user_id, &password)
            .initial_device_display_name("ntfy-matrix-bot")
            .await
            .context("password login failed")?;

        if let Some(session) = client.matrix_auth().session() {
            atomic_write(session_file, &serde_json::to_string_pretty(&session)?)
                .await
                .context("write session.json")?;
            tracing::info!("Session saved to {session_file:?}");
        }
        return Ok(());
    }

    Err(anyhow::anyhow!(
        "No auth configured. Set MATRIX_BOT_ACCESS_TOKEN or MATRIX_BOT_PASSWORD."
    ))
}

async fn atomic_write(path: &std::path::Path, data: &str) -> anyhow::Result<()> {
    use tokio::io::AsyncWriteExt;
    let tmp_path = path.with_extension("tmp");
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&tmp_path)
        .await?;
    file.write_all(data.as_bytes()).await?;
    file.sync_all().await?;
    // Write to a .tmp file and atomic-rename to prevent session.json corruption if the container crashes mid-write.
    tokio::fs::rename(&tmp_path, path).await?;
    Ok(())
}
