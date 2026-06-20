mod token;

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use anyhow::Result;
use crate::config::ConfigFile;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct CachedToken {
    access_token: String,
    expires_at: u64,
}

fn token_cache_path(profile: &str) -> PathBuf {
    let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("btpis");
    path.push(format!("token_cache_{}.json", profile));
    path
}

fn load_cached_token(profile: &str) -> Option<CachedToken> {
    let path = token_cache_path(profile);
    if !path.exists() {
        return None;
    }
    let content = fs::read_to_string(&path).ok()?;
    serde_json::from_str(&content).ok()?
}

fn save_cached_token(profile: &str, token: &CachedToken) {
    let path = token_cache_path(profile);
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(content) = serde_json::to_string_pretty(token) {
        let _ = fs::write(&path, content);
    }
}

pub async fn get_token(config: &ConfigFile, profile: &str) -> Result<String> {
    const MARGIN: u64 = 30;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    if let Some(cached) = load_cached_token(profile) {
        if cached.expires_at > now + MARGIN {
            return Ok(cached.access_token);
        }
    }

    let oauth = config.get_profile(profile)?;
    let response = token::fetch_token(oauth).await?;
    let expires_at = now + response.expires_in.unwrap_or(3600);
    let cached = CachedToken {
        access_token: response.access_token.clone(),
        expires_at,
    };
    save_cached_token(profile, &cached);

    Ok(response.access_token)
}
