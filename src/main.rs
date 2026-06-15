mod adapters;

use adapters::{AdapterDirection,fetch_iflow_adapters};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand};
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
// use tabled::{settings::{Width, Modify}, settings::object::Columns};
use indexmap::IndexMap;
use tabled::settings::object::Rows;
use tabled::settings::{Color, Style};
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
#[derive(Debug, Clone, PartialEq, clap::ValueEnum)]
enum IncludeOption {
    Adapters,
    Endpoints,
}

#[derive(Debug, Subcommand)]
enum ListCommands {
    /// List integration packages
    Packages,
    /// List integration flows. Provide a package_id or use --all
    Iflows {
        package_id: Option<String>,
        #[arg(long, help = "Fetch iflows from all packages")]
        all: bool,
    },
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
    #[serde(rename = "Mode", default)]
    mode: String,
}

#[derive(Debug, Default, Deserialize)]
struct IflowApiResponse {
    #[serde(default)]
    d: IflowData,
}

#[derive(Debug, Default, Deserialize)]
struct IflowData {
    #[serde(default)]
    results: Vec<IflowRecord>,
}
#[derive(Debug, Deserialize)]
struct IflowRecord {
    #[serde(default, rename = "Id")]
    id: Option<String>,
    #[serde(default, rename = "Name")]
    name: Option<String>,
    #[serde(default, rename = "Version")]
    version: Option<String>,
    #[serde(default, rename = "Sender")]
    sender: Option<String>,
    #[serde(default, rename = "Receiver")]
    receiver: Option<String>,
    // #[serde(default, rename = "CreatedAt")]
    // creation_at: Option<String>,
    // #[serde(default, rename = "CreatedBy")]
    // created_by: Option<String>,
    // #[serde(default, rename = "ModifiedAt")]
    // modified_at: Option<String>,
    // #[serde(default, rename = "ModifiedBy")]
    // modified_by: Option<String>,
}

#[derive(Debug, Tabled)]
struct PackageRow {
    // #[tabled(skip)]
    // #[tabled(rename = "Id")]
    // id: String,
    #[tabled(rename = "Name")]
    name: String,
    #[tabled(rename = "Vendor")]
    vendor: String,
    #[tabled(rename = "Mode")]
    mode: String,
    #[tabled(rename = "Modified By")]
    modified_by: String,
    #[tabled(rename = "Creation Date")]
    creation_date: String,
    #[tabled(rename = "Created By")]
    created_by: String,
    #[tabled(rename = "Modified Date")]
    modified_date: String,
}

#[derive(Tabled)]
struct IflowDisplayRow {
    #[tabled(rename = "Name")]
    name: String,

    #[tabled(rename = "Version")]
    version: String,

    #[tabled(rename = "Sender")]
    sender: String,

    #[tabled(rename = "Receiver")]
    receiver: String,

    #[tabled(rename = "Sender Adapter")]
    sender_adapter: String,

    #[tabled(rename = "Receiver Adapter")]
    receiver_adapter: String,
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
            ListCommands::Iflows { package_id, all } => {
                list_integration_flows(package_id, all).await?
            }
        },
    }

    Ok(())
}

fn summarise_adapters(types: Vec<&str>) -> String {
    let mut counts: IndexMap<&str, usize> = IndexMap::new();
    for t in types {
        *counts.entry(t).or_insert(0) += 1;
    }
    counts
        .iter()
        .map(|(t, n)| {
            if *n > 1 {
                format!("{} {}", n, t)
            } else {
                t.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
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
            // id: item.id,
            name: item.name,
            vendor: item.vendor,
            mode: item.mode,
            modified_by: item.modified_by,
            creation_date: format_timestamp(&item.creation_date),
            created_by: item.created_by,
            modified_date: format_timestamp(&item.modified_date),
        })
        .collect::<Vec<_>>();

    let mut table = Table::new(rows.iter());
    table.with(Style::rounded());
    // table.with(Modify::new(Columns::first()).with(Width::wrap(10)));
    table.modify(Rows::first(), Color::FG_BRIGHT_GREEN);
    // table.with(Width::wrap(Percent(75)));
    // table.with(Width::wrap(15));
    println!("{table}");

    Ok(())
}

async fn list_integration_flows(package_id: Option<String>, all: bool) -> Result<()> {
    let config = load_config()?;
    let token = get_token(&config).await?;
    let client = reqwest::Client::new();
    let base_url = config.oauth.url.trim_end_matches('/');

    let url = format!("{}/api/v1/IntegrationPackages?$format=json", base_url);
    let response = client
        .get(&url)
        .header(ACCEPT, "application/json")
        .header(AUTHORIZATION, format!("Bearer {}", token))
        .send()
        .await
        .context("failed to fetch packages for --all")?;

    if !response.status().is_success() {
        anyhow::bail!("package request failed with status: {}", response.status());
    }

    let body: PackageApiResponse = response
        .json()
        .await
        .context("failed to parse package list for --all")?;

    let mode_map: HashMap<String, String> = body
        .d
        .results
        .iter()
        .map(|p| (p.id.clone(), p.mode.clone()))
        .collect();

    let package_ids = if all {
        body.d.results.into_iter().map(|p| p.id).collect()
    } else if let Some(id) = package_id {
        if id.trim().is_empty() {
            anyhow::bail!("package_id cannot be empty");
        }
        vec![id]
    } else {
        anyhow::bail!("provide a package_id or use --all to fetch from all packages");
    };

    let mut display_rows: Vec<IflowDisplayRow> = Vec::new();

    for pkg_id in &package_ids {
        let url = format!(
            "{}/api/v1/IntegrationPackages('{}')/IntegrationDesigntimeArtifacts?$format=json",
            base_url, pkg_id
        );

        let response = client
            .get(&url)
            .header(ACCEPT, "application/json")
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await
            .with_context(|| format!("failed to fetch iflows for package: {}", pkg_id))?;

        if !response.status().is_success() {
            eprintln!(
                "warning: skipping package '{}' — status {}",
                pkg_id,
                response.status()
            );
            continue;
        }

        let body: IflowApiResponse = response
            .json()
            .await
            .with_context(|| format!("failed to parse iflows for package: {}", pkg_id))?;

        // READ_ONLY packages don't allow zip download
        let is_read_only = mode_map
            .get(pkg_id.as_str())
            .map(|m| m == "READ_ONLY")
            .unwrap_or(false);

        for item in body.d.results {
            let iflow_id = item.id.clone().unwrap_or_default();
            let version = item.version.clone().unwrap_or_default();

            // 3. Fetch adapters — always, not behind a flag
            let (sender_adapter, receiver_adapter) = if is_read_only {
                ("N/A".to_string(), "N/A".to_string())
            } else if iflow_id.is_empty() || version.is_empty() {
                ("—".to_string(), "—".to_string())
            } else {
                match fetch_iflow_adapters(&client, base_url, &token, &iflow_id, &version).await {
                    Ok(adapters) => {
                        let sender = summarise_adapters(
                            adapters
                                .iter()
                                .filter(|a| a.direction == AdapterDirection::Sender)
                                .map(|a| a.component_type.as_str())
                                .collect(),
                        );

                        let receiver = summarise_adapters(
                            adapters
                                .iter()
                                .filter(|a| a.direction == AdapterDirection::Receiver)
                                .map(|a| a.component_type.as_str())
                                .collect(),
                        );

                        (
                            if sender.is_empty() {
                                "—".to_string()
                            } else {
                                sender
                            },
                            if receiver.is_empty() {
                                "—".to_string()
                            } else {
                                receiver
                            },
                        )
                    }
                    Err(e) => {
                        eprintln!(
                            "warning: could not fetch adapters for iflow '{}': {}",
                            iflow_id, e
                        );
                        ("—".to_string(), "—".to_string())
                    }
                }
            };

            display_rows.push(IflowDisplayRow {
                name: item.name.unwrap_or_default(),
                version,
                sender: item.sender.unwrap_or_default(),
                receiver: item.receiver.unwrap_or_default(),
                sender_adapter,
                receiver_adapter,
            });
        }
    }

    if display_rows.is_empty() {
        println!("No integration flows found.");
        return Ok(());
    }

    println!("{} integration flow(s) found.\n", display_rows.len());

    let mut table = Table::new(&display_rows);
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
            return dt.with_timezone(&Utc).format("%m-%d-%Y").to_string();
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
