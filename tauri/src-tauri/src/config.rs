//! Application configuration and secret storage.
//!
//! - Non-secret settings (client id, default save folder, etc.) live in a JSON
//!   file under the OS config dir.
//! - The OAuth2 refresh token is stored in the OS keychain (macOS Keychain /
//!   Windows Credential Manager) via the `keyring` crate — never on disk.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const APP_QUALIFIER: &str = "com";
const APP_ORG: &str = "shuisong";
const APP_NAME: &str = "outlook-saver";

/// Keychain service name. The "account" is the user's email (or "default").
const KEYRING_SERVICE: &str = "outlook-saver:refresh-token";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// Azure (Entra) application (client) ID. User must register a public
    /// desktop app and paste the ID here. Required before login.
    pub client_id: String,
    /// `common` (work + personal), `consumers` (personal only), or a tenant ID.
    pub tenant: String,
    /// Signed-in user's email/UPN, learned from the id_token after login.
    pub email: Option<String>,
    /// Default directory offered in the Save dialog.
    pub default_save_dir: Option<String>,
    /// If false, save silently to `default_save_dir` without prompting.
    pub ask_each_time: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig {
            client_id: String::new(),
            tenant: "common".to_string(),
            email: None,
            default_save_dir: None,
            ask_each_time: true,
        }
    }
}

fn config_dir() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from(APP_QUALIFIER, APP_ORG, APP_NAME)
        .context("could not determine OS config directory")?;
    Ok(dirs.config_dir().to_path_buf())
}

fn config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.json"))
}

impl AppConfig {
    pub fn load() -> Result<AppConfig> {
        let path = config_path()?;
        if !path.exists() {
            return Ok(AppConfig::default());
        }
        let data = fs::read_to_string(&path)
            .with_context(|| format!("reading config at {}", path.display()))?;
        let cfg: AppConfig = serde_json::from_str(&data).context("parsing config.json")?;
        Ok(cfg)
    }

    pub fn save(&self) -> Result<()> {
        let dir = config_dir()?;
        fs::create_dir_all(&dir)
            .with_context(|| format!("creating config dir {}", dir.display()))?;
        let path = config_path()?;
        let data = serde_json::to_string_pretty(self)?;
        fs::write(&path, data).with_context(|| format!("writing config to {}", path.display()))?;
        Ok(())
    }

    /// Keychain account key for this config (per-email so multiple accounts can coexist).
    fn keyring_account(&self) -> String {
        self.email.clone().unwrap_or_else(|| "default".to_string())
    }
}

// ---- Refresh token (keychain) ----

fn entry(account: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, account).context("opening keychain entry")
}

pub fn store_refresh_token(cfg: &AppConfig, token: &str) -> Result<()> {
    entry(&cfg.keyring_account())?
        .set_password(token)
        .context("storing refresh token in keychain")
}

pub fn load_refresh_token(cfg: &AppConfig) -> Result<Option<String>> {
    match entry(&cfg.keyring_account())?.get_password() {
        Ok(t) => Ok(Some(t)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e).context("reading refresh token from keychain"),
    }
}

pub fn delete_refresh_token(cfg: &AppConfig) -> Result<()> {
    match entry(&cfg.keyring_account())?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e).context("deleting refresh token from keychain"),
    }
}
