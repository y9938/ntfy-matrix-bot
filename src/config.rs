#[derive(Clone)]
pub struct Config {
    pub ntfy_base_url: String,
    pub ntfy_admin_token: String,
    pub data_dir: std::path::PathBuf,
    pub matrix_homeserver: String,
    pub matrix_bot_user_id: Option<String>,
    pub matrix_bot_access_token: Option<String>,
    pub matrix_bot_password: Option<String>,
    pub matrix_allowed_users: std::collections::HashSet<String>,
    pub matrix_allowed_homeservers: std::collections::HashSet<String>,
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        let mut env: std::collections::HashMap<String, String> = std::env::vars().collect();

        let file_keys: Vec<String> = env
            .keys()
            .filter(|k| k.ends_with("_FILE"))
            .cloned()
            .collect();

        for file_key in &file_keys {
            let base_key = file_key.strip_suffix("_FILE").unwrap();
            if env.contains_key(base_key) {
                anyhow::bail!(
                    "Security Conflict: Both '{}' and '{}' are defined simultaneously. This creates auditing discrepancies.",
                    base_key,
                    file_key
                );
            }
        }

        for file_key in file_keys {
            let base_key = file_key.strip_suffix("_FILE").unwrap();
            let file_path = env.get(&file_key).unwrap();
            if let Ok(raw_secret) = std::fs::read_to_string(file_path) {
                let clean_secret = raw_secret
                    .trim_end_matches(|c: char| c == '\n' || c == '\r')
                    .to_string();
                env.insert(base_key.to_string(), clean_secret);
            } else {
                tracing::warn!(
                    "Failed to read secret file at '{}' for key '{}'",
                    file_path,
                    file_key
                );
            }
        }

        fn get(e: &std::collections::HashMap<String, String>, k: &str) -> Option<String> {
            e.get(k).cloned().filter(|s| !s.is_empty())
        }
        fn req(e: &std::collections::HashMap<String, String>, k: &str) -> anyhow::Result<String> {
            get(e, k).ok_or_else(|| anyhow::anyhow!("{} required", k))
        }
        fn parse_users(e: &std::collections::HashMap<String, String>, k: &str) -> anyhow::Result<std::collections::HashSet<String>> {
            get(e, k)
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| {
                    matrix_sdk::ruma::UserId::parse(s).map_err(|err| anyhow::anyhow!("Invalid User ID '{}' in {}: {}", s, k, err))?;
                    Ok(s.to_lowercase())
                })
                .collect()
        }

        fn parse_servers(e: &std::collections::HashMap<String, String>, k: &str) -> anyhow::Result<std::collections::HashSet<String>> {
            get(e, k)
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| {
                    matrix_sdk::ruma::ServerName::parse(s).map_err(|err| anyhow::anyhow!("Invalid Server Name '{}' in {}: {}", s, k, err))?;
                    Ok(s.to_lowercase())
                })
                .collect()
        }

        let cfg = Self {
            ntfy_base_url: req(&env, "NTFY_BASE_URL")?,
            ntfy_admin_token: req(&env, "NTFY_ADMIN_TOKEN")?,
            data_dir: get(&env, "DATA_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| {
                    let p = std::path::Path::new("/data");
                    if p.is_dir() {
                        p.to_path_buf()
                    } else {
                        std::path::PathBuf::from("./data")
                    }
                }),
            matrix_homeserver: req(&env, "MATRIX_HOMESERVER_URL")?,
            matrix_bot_user_id: get(&env, "MATRIX_BOT_USER_ID"),
            matrix_bot_access_token: get(&env, "MATRIX_BOT_ACCESS_TOKEN"),
            matrix_bot_password: get(&env, "MATRIX_BOT_PASSWORD"),
            matrix_allowed_users: parse_users(&env, "MATRIX_ALLOWED_USERS")?,
            matrix_allowed_homeservers: parse_servers(&env, "MATRIX_ALLOWED_HOMESERVERS")?,
        };
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        if self.matrix_allowed_users.is_empty() && self.matrix_allowed_homeservers.is_empty() {
            anyhow::bail!(
                "Security Policy Violation: You must explicitly define access controls via MATRIX_ALLOWED_USERS or MATRIX_ALLOWED_HOMESERVERS. Refusing to start as an open relay."
            );
        }

        match (&self.matrix_bot_access_token, &self.matrix_bot_password) {
            (Some(_), Some(_)) => anyhow::bail!(
                "Configuration Conflict: Both MATRIX_BOT_ACCESS_TOKEN and MATRIX_BOT_PASSWORD are set. Choose exactly one authentication method."
            ),
            (None, None) => anyhow::bail!(
                "Configuration Mismatch: Missing authentication credentials. You must provide either MATRIX_BOT_ACCESS_TOKEN or MATRIX_BOT_PASSWORD."
            ),
            _ => Ok(()),
        }
    }
}
