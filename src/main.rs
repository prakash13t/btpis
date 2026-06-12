use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand};
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tabled::settings::Style;
use tabled::{Table, Tabled};

#[derive(Debug, Parser)]
#[command(name = "btpis")]
#[command(about = "BTPIS CLI for OAuth config and package listing")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Configure the CLI with OAuth credentials
    Config {
        #[command(subcommand)]
        action: ConfigCommands,
    },
    /// List packages using the saved OAuth configuration
    List {
        /// What to list
        #[command(subcommand)]
        target: ListCommands,
    },
}

#[derive(Debug, Subcommand)]
enum ConfigCommands {
    /// Save OAuth credentials from a JSON file to the local config file
    Set {
        /// Path to the JSON file containing the OAuth config
        file: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
enum ListCommands {
    /// List integration packages
    Packages,
    /// List integration flows
    Iflows {
        package_id: Option<String>,
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct ConfigFile {
    oauth: OauthConfig,
}

#[derive(Debug, Deserialize, Serialize)]
struct OauthConfig {
    createdate: String,
    clientid: String,
    clientsecret: String,
    tokenurl: String,
    url: String,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: i64,
}

#[derive(Debug, Deserialize, Serialize)]
struct CachedToken {
    access_token: String,
    expires_at: i64,
}

#[derive(Debug, Deserialize)]
struct PackageApiResponse {
    d: PackageData,
}

#[derive(Debug, Deserialize)]
struct PackageData {
    results: Vec<PackageRecord>,
}

#[derive(Debug, Deserialize)]
struct PackageRecord {
    #[serde(rename = "Id")]
    id: String,
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Vendor")]
    vendor: String,
    #[serde(rename = "ModifiedBy")]
    modified_by: String,
    #[serde(rename = "CreationDate")]
    creation_date: String,
    #[serde(rename = "CreatedBy")]
    created_by: String,
    #[serde(rename = "ModifiedDate")]
    modified_date: String,
}

#[derive(Debug, Tabled)]
struct PackageRow {
    #[tabled(rename = "Id")]
    id: String,
    #[tabled(rename = "Name")]
    name: String,
    #[tabled(rename = "Vendor")]
    vendor: String,
    #[tabled(rename = "Modified By")]
    modified_by: String,
    #[tabled(rename = "Creation Date")]
    creation_date: String,
    #[tabled(rename = "Created By")]
    created_by: String,
    #[tabled(rename = "Modified Date")]
    modified_date: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Config { action } => match action {
            ConfigCommands::Set { file } => set_config(file).await?,
        },
        Commands::List { target } => match target {
            ListCommands::Packages => list_packages().await?,
            ListCommands::Iflows { .. } => list_integration_flows().await?,
        },
    }

    Ok(())
}

async fn set_config(file: PathBuf) -> Result<()> {
    let content = fs::read_to_string(&file)
        .with_context(|| format!("failed to read config file: {}", file.display()))?;
    let config: ConfigFile =
        serde_json::from_str(&content).context("invalid JSON config format")?;

    if config.oauth.clientid.trim().is_empty()
        || config.oauth.clientsecret.trim().is_empty()
        || config.oauth.tokenurl.trim().is_empty()
        || config.oauth.url.trim().is_empty()
    {
        anyhow::bail!("config is missing required OAuth fields");
    }

    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config directory: {}", parent.display()))?;
    }

    fs::write(
        &path,
        serde_json::to_string_pretty(&config).context("failed to serialize config")?,
    )
    .with_context(|| format!("failed to write config file: {}", path.display()))?;

    println!("Config saved to {}", path.display());
    Ok(())
}

async fn list_packages() -> Result<()> {
    let config = load_config()?;
    let token = get_token(&config).await?;
    let client = reqwest::Client::new();

    let base_url = config.oauth.url.trim_end_matches('/');
    let package_url = format!("{}/api/v1/IntegrationPackages?$format=json", base_url);

    let response = client
        .get(&package_url)
        .header(ACCEPT, "application/json")
        .header(AUTHORIZATION, format!("Bearer {}", token))
        .send()
        .await
        .context("failed to fetch packages")?;

    if !response.status().is_success() {
        anyhow::bail!("package request failed with status: {}", response.status());
    }

    let body: PackageApiResponse = response
        .json()
        .await
        .context("failed to parse package response")?;

    let rows = body
        .d
        .results
        .into_iter()
        .map(|item| PackageRow {
            id: item.id,
            name: item.name,
            vendor: item.vendor,
            modified_by: item.modified_by,
            creation_date: format_timestamp(&item.creation_date),
            created_by: item.created_by,
            modified_date: format_timestamp(&item.modified_date),
        })
        .collect::<Vec<_>>();

    let mut table = Table::new(rows.iter());
    table.with(Style::rounded());
    println!("{table}");

    Ok(())
}

async fn list_integration_flows() -> Result<()> {
    let config = load_config()?;
    let token = get_token(&config).await?;
    let client = reqwest::Client::new();

    let base_url = config.oauth.url.trim_end_matches('/');
    let package_url = format!("{}/api/v1/IntegrationRuntimeArtifacts?$format=json", base_url);

    let response = client
        .get(&package_url)
        .header(ACCEPT, "application/json")
        .header(AUTHORIZATION, format!("Bearer {}", token))
        .send()
        .await
        .context("failed to fetch integration flows")?;

    if !response.status().is_success() {
        anyhow::bail!("integration flow request failed with status: {}", response.status());
    }

    let body: PackageApiResponse = response
        .json()
        .await
        .context("failed to parse integration flow response")?;

    let rows = body
        .d
        .results
        .into_iter()
        .map(|item| PackageRow {
            id: item.id,
            name: item.name,
            vendor: item.vendor,
            modified_by: item.modified_by,
            creation_date: format_timestamp(&item.creation_date),
            created_by: item.created_by,
            modified_date: format_timestamp(&item.modified_date),
        })
        .collect::<Vec<_>>();

    let mut table = Table::new(rows.iter());
    table.with(Style::rounded());
    println!("{table}");

    Ok(())
}

async fn get_token(config: &ConfigFile) -> Result<String> {
    if let Some(cached) = load_cached_token()? {
        if cached.expires_at > now_unix() {
            return Ok(cached.access_token);
        }
    }

    let client = reqwest::Client::new();
    let response = client
        .post(&config.oauth.tokenurl)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(ACCEPT, "application/json")
        .form(&[
            ("grant_type", "client_credentials"),
            ("client_id", &config.oauth.clientid),
            ("client_secret", &config.oauth.clientsecret),
        ])
        .send()
        .await
        .context("failed to request OAuth token")?;

    if !response.status().is_success() {
        anyhow::bail!("token request failed with status: {}", response.status());
    }

    let token_response: TokenResponse = response
        .json()
        .await
        .context("failed to parse OAuth token response")?;
    if token_response.access_token.trim().is_empty() {
        anyhow::bail!("token response did not include an access_token");
    }

    let expires_at = now_unix() + token_response.expires_in.max(1);
    let cached = CachedToken {
        access_token: token_response.access_token.clone(),
        expires_at,
    };

    save_cached_token(&cached)?;
    Ok(token_response.access_token)
}

fn load_config() -> Result<ConfigFile> {
    let path = config_path();
    let content = fs::read_to_string(&path).with_context(|| {
        format!(
            "config file not found at {}. Run 'btpis config -f <file>' first.",
            path.display()
        )
    })?;
    let config: ConfigFile = serde_json::from_str(&content).context("invalid config JSON")?;
    Ok(config)
}

fn config_path() -> PathBuf {
    let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("btpis_cli");
    path.push("config.json");
    path
}

fn token_cache_path() -> PathBuf {
    let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("btpis_cli");
    path.push("token_cache.json");
    path
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn load_cached_token() -> Result<Option<CachedToken>> {
    let path = token_cache_path();
    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(&path).context("failed to read cached token")?;
    let token: CachedToken = serde_json::from_str(&content).context("invalid cached token JSON")?;
    Ok(Some(token))
}

fn save_cached_token(token: &CachedToken) -> Result<()> {
    let path = token_cache_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("failed to create token cache directory")?;
    }

    fs::write(
        &path,
        serde_json::to_string_pretty(token).context("failed to serialize token cache")?,
    )
    .context("failed to write token cache")?;
    Ok(())
}

fn format_timestamp(value: &str) -> String {
    if let Ok(ms) = value.parse::<i64>() {
        if let Some(dt) = DateTime::from_timestamp_millis(ms) {
            return dt
                .with_timezone(&Utc)
                .format("%Y-%m-%d %H:%M:%S UTC")
                .to_string();
        }
    }

    if let Ok(seconds) = value.parse::<i64>() {
        if let Some(dt) = DateTime::from_timestamp(seconds, 0) {
            return dt
                .with_timezone(&Utc)
                .format("%Y-%m-%d %H:%M:%S UTC")
                .to_string();
        }
    }

    if let Ok(dt) = DateTime::parse_from_rfc3339(value) {
        return dt
            .with_timezone(&Utc)
            .format("%Y-%m-%d %H:%M:%S UTC")
            .to_string();
    }

    value.to_string()
}
