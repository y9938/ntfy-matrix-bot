use anyhow::{Context, Result};
use crate::config::Config;

fn generate_password() -> String {
    const CHARSET: &[u8] = b"abcdefghjkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789!@#$%^&*";
    let mut buf = [0u8; 34];
    getrandom::fill(&mut buf).expect("getrandom failed");
    buf.iter()
        .map(|&b| CHARSET[b as usize % CHARSET.len()] as char)
        .collect()
}

#[derive(serde::Serialize)]
struct UserAddRequest<'a> {
    username: &'a str,
    password: &'a str,
}

#[derive(serde::Serialize)]
struct AccessRequest<'a> {
    username: &'a str,
    topic: &'a str,
    permission: &'a str,
}

pub enum ProvisionResult {
    Created { password: String },
    CreatedAclFailed { password: String },
    AlreadyExists,
}

pub async fn provision_ntfy_user(
    http: &reqwest::Client,
    cfg: &Config,
    username: &str,
) -> Result<ProvisionResult> {
    let base = cfg.ntfy_base_url.trim_end_matches('/');
    let password = generate_password();

    let res = http
        .post(format!("{base}/v1/users"))
        .bearer_auth(&cfg.ntfy_admin_token)
        .json(&UserAddRequest {
            username,
            password: &password,
        })
        .send()
        .await
        .context("ntfy POST /v1/users")?;

    if res.status() == reqwest::StatusCode::CONFLICT {
        return Ok(ProvisionResult::AlreadyExists);
    }
    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("ntfy user create ({status}): {body}"));
    }

    let acl = http
        .post(format!("{base}/v1/users/access"))
        .bearer_auth(&cfg.ntfy_admin_token)
        .json(&AccessRequest {
            username,
            topic: "up*",
            permission: "read-write",
        })
        .send()
        .await
        .context("ntfy POST /v1/users/access")?;

    if !acl.status().is_success() {
        tracing::error!(username, status = %acl.status(), "ACL set failed after user create");
        return Ok(ProvisionResult::CreatedAclFailed { password });
    }

    Ok(ProvisionResult::Created { password })
}
