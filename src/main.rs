mod adapters;
mod auth;
mod commands;
mod config;
mod utils;

use adapters::{AdapterDirection, fetch_iflow_adapters};
use anyhow::{Context, Result};
use auth::get_token;
use clap::{Parser, Subcommand};
use commands::get::GetCommands;
use config::{config_path, load_config, resolve_profile, OauthConfig};
use reqwest::header::{ACCEPT, AUTHORIZATION};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use tabled::settings::object::Rows;
use tabled::settings::{Color, Style};
use tabled::{Table, Tabled};

use crate::utils::summarise_adapters;

#[derive(Debug, Parser)]
#[command(name = "btpis")]
#[command(about = "BTPIS CLI for OAuth config and package listing")]
struct Cli {
    /// Profile to use. Can also be set via BTPIS_PROFILE env var.
    #[arg(long, env = "BTPIS_PROFILE", global = true)]
    profile: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Manage OAuth profiles
    Config {
        #[command(subcommand)]
        action: ConfigCommands,
    },
    /// List packages using the saved OAuth configuration
    List {
        #[command(subcommand)]
        target: ListCommands,
    },
    /// Get details of a single integration flow
    Get {
        #[command(subcommand)]
        target: GetCommands,
    },
}

#[derive(Debug, Subcommand)]
enum ConfigCommands {
    /// Save OAuth credentials as a new profile from a JSON file
    Set {
        /// Profile name (e.g. dev, qa, prd)
        profile: String,
        /// Path to the JSON file containing the OAuth config
        file: PathBuf,
    },
    /// List all configured profiles
    List,
    /// Set the default profile
    SetDefault {
        /// Profile name to set as default
        profile: String,
    },
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
    /// List service endpoints
    #[command(name = "service-endpoints")]
    Serviceendpoints,
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
}

#[derive(Debug, Tabled)]
struct PackageRow {
    #[tabled(rename = "Name")]
    name: String,
    #[tabled(rename = "Vendor")]
    vendor: String,
    #[tabled(rename = "Mode")]
    mode: String,
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

#[derive(Tabled)]
struct ServiceEndpointRow {
    #[tabled(rename = "Iflow Name")]
    name: String,
    #[tabled(rename = "Version")]
    version: String,
    #[tabled(rename = "Protocol")]
    protocol: String,
    #[tabled(rename = "Url")]
    url: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Config { action } => match action {
            ConfigCommands::Set { profile, file } => set_config_profile(&profile, file).await?,
            ConfigCommands::List => list_profiles()?,
            ConfigCommands::SetDefault { profile } => set_default_profile(&profile)?,
        },
        Commands::List { target } => {
            let config = load_config()?;
            let profile = resolve_profile(cli.profile.as_deref(), &config);
            match target {
                ListCommands::Packages => list_packages(&config, &profile).await?,
                ListCommands::Iflows { package_id, all } => {
                    list_integration_flows(&config, &profile, package_id, all).await?
                }
                ListCommands::Serviceendpoints => {
                    list_serviceendpoints(&config, &profile).await?
                }
            }
        }
        Commands::Get { target } => {
            let config = load_config()?;
            let profile = resolve_profile(cli.profile.as_deref(), &config);
            commands::get::handle(target, &config, &profile).await?;
        }
    }

    Ok(())
}

#[derive(Deserialize)]
struct OauthFile {
    oauth: OauthConfig,
}

async fn set_config_profile(profile: &str, file: PathBuf) -> Result<()> {
    let content = fs::read_to_string(&file)
        .with_context(|| format!("failed to read config file: {}", file.display()))?;
    let oauth: OauthConfig = serde_json::from_str(&content)
        .or_else(|_| -> Result<OauthConfig> {
            let wrapped: OauthFile =
                serde_json::from_str(&content).context("invalid JSON config format")?;
            Ok(wrapped.oauth)
        })
        .context("expected JSON with 'oauth' key at root, or a flat OAuth object")?;

    if oauth.client_id.trim().is_empty()
        || oauth.client_secret.trim().is_empty()
        || oauth.token_url.trim().is_empty()
        || oauth.url.trim().is_empty()
    {
        anyhow::bail!("config is missing required OAuth fields");
    }

    let path = config_path();
    let mut config: config::ConfigFile = fs::read_to_string(&path)
        .ok()
        .and_then(|c| serde_json::from_str(&c).ok())
        .unwrap_or_else(|| config::ConfigFile {
            default_profile: profile.to_string(),
            profiles: HashMap::new(),
        });

    config.profiles.insert(profile.to_string(), oauth);

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config directory: {}", parent.display()))?;
    }

    fs::write(
        &path,
        serde_json::to_string_pretty(&config).context("failed to serialize config")?,
    )
    .with_context(|| format!("failed to write config file: {}", path.display()))?;

    println!("Profile '{}' saved to {}", profile, path.display());
    Ok(())
}

fn list_profiles() -> Result<()> {
    let config = load_config()?;

    println!("{} profile(s) configured:\n", config.profiles.len());
    for (name, oauth) in &config.profiles {
        let marker = if *name == config.default_profile {
            " (default)"
        } else {
            ""
        };
        println!("  {}{}", name, marker);
        println!("    URL: {}", oauth.url);
        println!("    Client ID: {}", oauth.client_id);
        println!();
    }
    Ok(())
}

fn set_default_profile(profile: &str) -> Result<()> {
    let path = config_path();
    let mut config: config::ConfigFile = serde_json::from_str(
        &fs::read_to_string(&path).context("config file not found")?,
    )
    .context("invalid config JSON")?;

    if !config.profiles.contains_key(profile) {
        anyhow::bail!(
            "profile '{}' does not exist. Use 'btpis config set {} <file>' first.",
            profile,
            profile
        );
    }

    config.default_profile = profile.to_string();
    fs::write(
        &path,
        serde_json::to_string_pretty(&config).context("failed to serialize config")?,
    )
    .context("failed to write config")?;

    println!("Default profile set to '{}'", profile);
    Ok(())
}

async fn list_packages(config: &config::ConfigFile, profile: &str) -> Result<()> {
    let token = get_token(config, profile).await?;
    let client = reqwest::Client::new();

    let oauth = config.get_profile(profile)?;
    let base_url = oauth.url.trim_end_matches('/');
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
            name: item.name,
            vendor: item.vendor,
            mode: item.mode,
        })
        .collect::<Vec<_>>();

    println!("[{}] {} package(s) found.", profile, rows.len());
    let mut table = Table::new(rows.iter());
    table.with(Style::rounded());
    table.modify(Rows::first(), Color::FG_BRIGHT_GREEN);
    println!("{table}");

    Ok(())
}

async fn list_integration_flows(
    config: &config::ConfigFile,
    profile: &str,
    package_id: Option<String>,
    all: bool,
) -> Result<()> {
    let token = get_token(config, profile).await?;
    let client = reqwest::Client::new();

    let oauth = config.get_profile(profile)?;
    let base_url = oauth.url.trim_end_matches('/');

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

        let is_read_only = mode_map
            .get(pkg_id.as_str())
            .map(|m| m == "READ_ONLY")
            .unwrap_or(false);

        for item in body.d.results {
            let iflow_id = item.id.clone().unwrap_or_default();
            let version = item.version.clone().unwrap_or_default();

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

    println!("[{}] {} integration flow(s) found.", profile, display_rows.len());

    let mut table = Table::new(&display_rows);
    table.with(Style::rounded());
    table.modify(Rows::first(), Color::FG_BRIGHT_GREEN);
    println!("{table}");

    Ok(())
}

async fn list_serviceendpoints(config: &config::ConfigFile, profile: &str) -> Result<()> {
    let token = get_token(config, profile).await?;
    let client = reqwest::Client::new();

    let oauth = config.get_profile(profile)?;
    let base_url = oauth.url.trim_end_matches('/');
    let url = format!(
        "{}/api/v1/ServiceEndpoints?$expand=EntryPoints",
        base_url
    );

    let response = client
        .get(&url)
        .header(ACCEPT, "application/xml")
        .header(AUTHORIZATION, format!("Bearer {}", token))
        .send()
        .await
        .context("failed to fetch service endpoints")?;

    if !response.status().is_success() {
        anyhow::bail!(
            "service endpoints request failed with status: {}",
            response.status()
        );
    }

    let text = response
        .text()
        .await
        .context("failed to read service endpoints body")?;

    let doc = roxmltree::Document::parse(&text).context("failed to parse service endpoints XML")?;

    let rows: Vec<ServiceEndpointRow> = doc
        .descendants()
        .filter(|n| n.tag_name().name() == "entry")
        .filter_map(|entry| {
            let props = entry
                .descendants()
                .filter(|n| n.tag_name().name() == "properties")
                .find(|props| {
                    props
                        .children()
                        .any(|c| c.tag_name().name() == "Version")
                })?;

            let get = |name: &str| -> String {
                props
                    .children()
                    .find(|n| n.tag_name().name() == name)
                    .and_then(|n| n.text())
                    .unwrap_or("")
                    .trim()
                    .to_string()
            };

            let name = get("Name");
            let version = get("Version");
            let protocol = get("Protocol");

            let url = entry
                .descendants()
                .find(|n| n.tag_name().name() == "Url")
                .and_then(|n| n.text())
                .unwrap_or("")
                .trim()
                .to_string();

            Some(ServiceEndpointRow {
                name,
                version,
                protocol,
                url,
            })
        })
        .collect();

    if rows.is_empty() {
        println!("No service endpoints found.");
        return Ok(());
    }

    println!("[{}] {} service endpoint(s) found.", profile, rows.len());
    let mut table = Table::new(&rows);
    table.with(Style::rounded());
    table.modify(Rows::first(), Color::FG_BRIGHT_GREEN);
    println!("{table}");

    Ok(())
}
