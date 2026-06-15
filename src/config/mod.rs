use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct ConfigFile {
    #[serde(default = "default_profile_name")]
    pub default_profile: String,
    #[serde(default)]
    pub profiles: HashMap<String, OauthConfig>,
}

fn default_profile_name() -> String {
    "default".to_string()
}

impl ConfigFile {
    pub fn get_profile(&self, name: &str) -> Result<&OauthConfig> {
        self.profiles.get(name).with_context(|| {
            format!(
                "profile '{}' not found. Use 'btpis config set {} <file>' to add it, or run 'btpis config list' to see available profiles.",
                name, name
            )
        })
    }
}

pub fn resolve_profile(cli_profile: Option<&str>, config: &ConfigFile) -> String {
    cli_profile
        .map(|s| s.to_string())
        .or_else(|| std::env::var("BTPIS_PROFILE").ok())
        .unwrap_or_else(|| config.default_profile.clone())
}

#[derive(Debug, Deserialize, Serialize)]
pub struct OauthConfig {
    #[serde(rename = "createdate")]
    pub createdate: String,

    #[serde(rename = "clientid")]
    pub client_id: String,

    #[serde(rename = "clientsecret")]
    pub client_secret: String,

    #[serde(rename = "tokenurl")]
    pub token_url: String,

    #[serde(rename = "url")]
    pub url: String,
}

#[derive(Debug, Deserialize)]
struct OldConfig {
    oauth: OauthConfig,
}

fn try_migrate_old_format(content: &str) -> Option<ConfigFile> {
    let old: OldConfig = serde_json::from_str(content).ok()?;
    eprintln!("info: migrating config to new format with profiles");
    Some(ConfigFile {
        default_profile: "default".to_string(),
        profiles: [("default".to_string(), old.oauth)].into_iter().collect(),
    })
}

pub fn load_config() -> Result<ConfigFile> {
    let path = config_path();
    let content = fs::read_to_string(&path).with_context(|| {
        format!(
            "config file not found at {}. Run 'btpis config set <profile> <file>' first.",
            path.display()
        )
    })?;

    if let Ok(config) = serde_json::from_str::<ConfigFile>(&content) {
        if !config.profiles.is_empty() {
            return Ok(config);
        }
    }

    if let Some(config) = try_migrate_old_format(&content) {
        let json =
            serde_json::to_string_pretty(&config).context("failed to serialize migrated config")?;
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(&path, &json);
        return Ok(config);
    }

    anyhow::bail!(
        "invalid config format. Remove {} and run 'btpis config set <profile> <file>'.",
        path.display()
    );
}

pub fn config_path() -> PathBuf {
    let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("btpis");
    path.push("config.json");
    path
}
