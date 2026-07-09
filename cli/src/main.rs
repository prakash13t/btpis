mod adapters;
mod auth;
mod commands;
mod config;
mod utils;

use crate::utils::{csv_escape, start_spinner};
use crate::utils::{format_duration, parse_duration_relative, parse_json_date, summarise_adapters};
use adapters::{AdapterDirection, fetch_iflow_adapters};
use anyhow::{Context, Result};
use auth::get_token;
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand};
use commands::{deploy::DeployCommands, get::GetCommands, undeploy::UnDeployCommands};
use config::{OauthConfig, config_path, load_config, resolve_profile};
use owo_colors::OwoColorize;
use reqwest::header::{ACCEPT, AUTHORIZATION};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tabled::settings::object::{Columns, Rows};
use tabled::settings::{Color, Style, Width};
use tabled::{Table, Tabled};

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
    /// Deploy an integration flow or a CSV batch of integration flows
    Deploy {
        #[command(subcommand)]
        target: Option<DeployCommands>,
        /// Path to a CSV file with integration flow deploy entries
        file: Option<PathBuf>,
    },
    Undeploy {
        #[command(subcommand)]
        target: Option<UnDeployCommands>,
        /// Path to a CSV file with integration flow undeploy entries
        file: Option<PathBuf>,
    },
    /// Show message processing logs for integration flows
    Logs {
        /// Integration flow name (shows all flows if omitted)
        iflow_id: Option<String>,

        /// Number of recent log entries to show
        #[arg(long)]
        tail: Option<usize>,

        /// Show logs since relative duration (e.g. 1h, 30m, 7d)
        #[arg(long)]
        since: Option<String>,

        /// Show logs since absolute timestamp (ISO 8601)
        #[arg(long)]
        since_time: Option<String>,

        /// Stream/follow new log entries
        #[arg(short = 'f')]
        follow: bool,

        /// API key for sandbox testing (bypasses OAuth)
        #[arg(long, env = "BTPIS_API_KEY", hide = true)]
        api_key: Option<String>,
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
    /// List configurations of all integration flows
    #[command(name = "configurations")]
    Configurations {
        /// Filter by package ID or iflow name (exact match)
        filter: Option<String>,
        /// Output in CSV format
        #[arg(long)]
        csv: bool,
    },
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
struct ConfigDisplayRow {
    #[tabled(rename = "Iflow Name")]
    iflow_name: String,
    #[tabled(rename = "Parameter Key")]
    key: String,
    #[tabled(rename = "Parameter Value")]
    value: String,
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
                ListCommands::Serviceendpoints => list_serviceendpoints(&config, &profile).await?,
                ListCommands::Configurations { filter, csv } => {
                    list_configurations(&config, &profile, filter, csv).await?
                }
            }
        }
        Commands::Get { target } => {
            let config = load_config()?;
            let profile = resolve_profile(cli.profile.as_deref(), &config);
            commands::get::handle(target, &config, &profile).await?;
        }
        Commands::Deploy { target, file } => {
            let config = load_config()?;
            let profile = resolve_profile(cli.profile.as_deref(), &config);

            match target {
                Some(target) => commands::deploy::handle(target, &config, &profile).await?,
                None => {
                    if let Some(file) = file {
                        commands::deploy::handle(DeployCommands::Bulk { file }, &config, &profile)
                            .await?;
                    } else {
                        anyhow::bail!(
                            "provide either 'iflow <id> <version>', 'bulk <file.csv>', or a CSV file path directly"
                        );
                    }
                }
            }
        }
        Commands::Undeploy { target, file } => {
            let config = load_config()?;
            let profile = resolve_profile(cli.profile.as_deref(), &config);

            fn looks_like_path(path: &std::path::Path) -> bool {
                path.is_absolute()
                    || path.components().count() > 1
                    || path.extension().is_some()
                    || path.to_string_lossy().contains(std::path::MAIN_SEPARATOR)
            }

            match (target, file) {
                (Some(target), None) => {
                    commands::undeploy::handle(target, &config, &profile).await?;
                }
                (None, Some(file)) => {
                    if file.exists() {
                        commands::undeploy::handle(
                            commands::undeploy::UnDeployCommands::Bulk { file },
                            &config,
                            &profile,
                        )
                        .await?;
                    } else if looks_like_path(&file) {
                        anyhow::bail!("failed to read CSV file: {}", file.display());
                    } else {
                        let id = file.to_string_lossy().to_string();
                        commands::undeploy::handle(
                            commands::undeploy::UnDeployCommands::Iflow { id },
                            &config,
                            &profile,
                        )
                        .await?;
                    }
                }
                (Some(_), Some(_)) => {
                    anyhow::bail!("provide either 'iflow <id>' or a file name, not both");
                }
                (None, None) => {
                    anyhow::bail!("provide either 'iflow <id>' or a file name");
                }
            }
        }
        Commands::Logs {
            iflow_id,
            tail,
            since,
            since_time,
            follow,
            api_key,
        } => {
            let config = load_config()?;
            let profile = resolve_profile(cli.profile.as_deref(), &config);
            list_logs(
                &config, &profile, iflow_id, tail, since, since_time, follow, api_key,
            )
            .await?;
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
    let mut config: config::ConfigFile =
        serde_json::from_str(&fs::read_to_string(&path).context("config file not found")?)
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
    let spinner = start_spinner("Fetching packages...");

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
    spinner.finish_and_clear();
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
    let spinner = start_spinner("Fetching integration flows...");

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
        spinner.set_message(format!("Fetching iflows for package: {}", pkg_id));
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
    spinner.finish_and_clear();
    if display_rows.is_empty() {
        println!("No integration flows found.");
        return Ok(());
    }

    println!(
        "[{}] {} integration flow(s) found.",
        profile,
        display_rows.len()
    );

    let mut table = Table::new(&display_rows);
    table.with(Style::rounded());
    table.modify(Rows::first(), Color::FG_BRIGHT_GREEN);
    println!("{table}");

    Ok(())
}

async fn list_serviceendpoints(config: &config::ConfigFile, profile: &str) -> Result<()> {
    let token = get_token(config, profile).await?;
    let client = reqwest::Client::new();
    let spinner = start_spinner("Fetching service endpoints...");
    let oauth = config.get_profile(profile)?;
    let base_url = oauth.url.trim_end_matches('/');
    let url = format!("{}/api/v1/ServiceEndpoints?$expand=EntryPoints", base_url);

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
                .find(|props| props.children().any(|c| c.tag_name().name() == "Version"))?;

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
    spinner.finish_and_clear();
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

async fn fetch_iflow_configs(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
    id: &str,
    version: &str,
) -> Result<Vec<(String, String)>> {
    let url = format!(
        "{}/api/v1/IntegrationDesigntimeArtifacts(Id='{}',Version='{}')/Configurations",
        base_url, id, version
    );

    let response = client
        .get(&url)
        .header(ACCEPT, "application/xml")
        .header(AUTHORIZATION, format!("Bearer {}", token))
        .send()
        .await
        .with_context(|| format!("failed to fetch configs for iflow: {}", id))?;

    if !response.status().is_success() {
        return Ok(Vec::new());
    }

    let text = response
        .text()
        .await
        .context("failed to read configs body")?;

    let doc = roxmltree::Document::parse(&text)
        .with_context(|| format!("failed to parse configs XML for iflow: {}", id))?;

    let rows: Vec<(String, String)> = doc
        .descendants()
        .filter(|n| n.tag_name().name() == "entry")
        .filter_map(|entry| {
            let props = entry
                .descendants()
                .find(|n| n.tag_name().name() == "properties")?;

            let get = |name: &str| -> String {
                props
                    .children()
                    .find(|n| n.tag_name().name() == name)
                    .and_then(|n| n.text())
                    .unwrap_or("")
                    .trim()
                    .to_string()
            };

            let key = get("ParameterKey");
            let value = get("ParameterValue");

            if key.is_empty() && value.is_empty() {
                return None;
            }

            Some((key, value))
        })
        .collect();

    Ok(rows)
}

async fn list_configurations(
    config: &config::ConfigFile,
    profile: &str,
    filter: Option<String>,
    csv: bool,
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
        .context("failed to fetch packages")?;

    if !response.status().is_success() {
        anyhow::bail!("package request failed with status: {}", response.status());
    }

    let body: PackageApiResponse = response
        .json()
        .await
        .context("failed to parse package response")?;

    let all_packages: Vec<String> = body.d.results.into_iter().map(|p| p.id).collect();

    let target_package = filter
        .as_ref()
        .and_then(|f| all_packages.iter().find(|id| id.eq_ignore_ascii_case(f)));

    let package_ids: Vec<&str> = if let Some(pkg) = target_package {
        vec![pkg.as_str()]
    } else {
        all_packages.iter().map(|s| s.as_str()).collect()
    };

    let iflow_filter: Option<&str> = if target_package.is_some() {
        None
    } else {
        filter.as_deref()
    };

    let mut display_rows: Vec<ConfigDisplayRow> = Vec::new();
    let mut iflow_count: HashSet<String> = HashSet::new();

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

        for item in body.d.results {
            let iflow_id = item.id.clone().unwrap_or_default();
            let version = item.version.clone().unwrap_or_default();
            let iflow_name = item.name.unwrap_or_else(|| iflow_id.clone());

            if iflow_id.is_empty() || version.is_empty() {
                continue;
            }

            if let Some(ref name_filter) = iflow_filter {
                if !iflow_name.eq_ignore_ascii_case(name_filter) {
                    continue;
                }
            }

            match fetch_iflow_configs(&client, &base_url, &token, &iflow_id, &version).await {
                Ok(configs) => {
                    if configs.is_empty() {
                        continue;
                    }
                    iflow_count.insert(iflow_id);
                    for (key, value) in configs {
                        display_rows.push(ConfigDisplayRow {
                            iflow_name: iflow_name.clone(),
                            key,
                            value,
                        });
                    }
                }
                Err(e) => {
                    eprintln!(
                        "warning: could not fetch configs for iflow '{}': {}",
                        iflow_id, e
                    );
                }
            }
        }
    }

    if csv {
        println!("Iflow Name,Parameter Key,Parameter Value");
        for row in &display_rows {
            println!(
                "{},{},{}",
                csv_escape(&row.iflow_name),
                csv_escape(&row.key),
                csv_escape(&row.value)
            );
        }
        return Ok(());
    }

    if display_rows.is_empty() {
        println!("No configurations found.");
        return Ok(());
    }

    println!(
        "[{}] {} configuration(s) found across {} iflow(s).",
        profile,
        display_rows.len(),
        iflow_count.len()
    );

    let mut table = Table::new(&display_rows);
    table.with(Style::rounded());
    table.modify(Columns::single(2), Width::truncate(120).suffix("..."));
    table.modify(Rows::first(), Color::FG_BRIGHT_GREEN);
    println!("{table}");

    Ok(())
}

fn format_log_line(
    message_guid: &str,
    integration_flow_name: Option<&str>,
    status: Option<&str>,
    log_start: Option<&str>,
    log_end: Option<&str>,
) -> String {
    let start_ts = log_start
        .and_then(parse_json_date)
        .and_then(|ms| {
            DateTime::from_timestamp_millis(ms).map(|dt| {
                dt.with_timezone(&Utc)
                    .format("%Y-%m-%d %H:%M:%S")
                    .to_string()
            })
        })
        .unwrap_or_else(|| log_start.unwrap_or("—").to_string());

    let proc_time = match (log_start, log_end) {
        (Some(s), Some(e)) => match (parse_json_date(s), parse_json_date(e)) {
            (Some(start), Some(end)) => format_duration(start, end),
            _ => "—".to_string(),
        },
        _ => "—".to_string(),
    };

    let status = status.unwrap_or("UNKNOWN");
    let status_colored = match status {
        "COMPLETED" => status.green().to_string(),
        "FAILED" => status.red().to_string(),
        "PROCESSING" => status.yellow().to_string(),
        "RETRY" => status.yellow().to_string(),
        "DISCARDED" => status.red().to_string(),
        _ => status.white().to_string(),
    };

    let flow = integration_flow_name.unwrap_or("—");

    format!(
        "{} [{:9}] {} {:>10} {:<30}",
        start_ts, status_colored, message_guid, proc_time, flow
    )
}

fn compute_since_datetime(dur_str: &str) -> Result<String> {
    let dur = parse_duration_relative(dur_str)
        .ok_or_else(|| anyhow::anyhow!("invalid duration format. Use: 30s, 5m, 2h, 7d"))?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let past_secs = now - dur.as_secs().min(now);
    let dt = DateTime::from_timestamp(past_secs as i64, 0)
        .context("invalid timestamp")?
        .with_timezone(&Utc);

    Ok(dt.format("%Y-%m-%dT%H:%M:%S").to_string())
}

async fn fetch_logs_xml(
    client: &reqwest::Client,
    url: &str,
    auth_header: &str,
    auth_value: &str,
) -> Result<
    Vec<(
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    )>,
> {
    let response = client
        .get(url)
        .header(auth_header, auth_value)
        .header(ACCEPT, "application/xml")
        .send()
        .await
        .context("failed to fetch logs")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("logs request failed with status {}: {}", status, body);
    }

    let text = response.text().await.context("failed to read logs body")?;

    let doc = roxmltree::Document::parse(&text).context("failed to parse logs XML")?;

    let results: Vec<_> = doc
        .descendants()
        .filter(|n| n.tag_name().name() == "entry")
        .filter_map(|entry| {
            let props = entry
                .descendants()
                .find(|n| n.tag_name().name() == "properties")?;

            let get = |name: &str| -> Option<String> {
                props
                    .children()
                    .find(|n| n.tag_name().name() == name)
                    .and_then(|n| n.text())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
            };

            let guid = get("MessageGuid")?;
            let flow = get("IntegrationFlowName");
            let status = get("Status");
            let log_start = get("LogStart");
            let log_end = get("LogEnd");

            Some((guid, flow, status, log_start, log_end))
        })
        .collect();

    Ok(results)
}

async fn list_logs(
    config: &config::ConfigFile,
    profile: &str,
    iflow_id: Option<String>,
    tail: Option<usize>,
    since: Option<String>,
    since_time: Option<String>,
    follow: bool,
    api_key: Option<String>,
) -> Result<()> {
    let client = reqwest::Client::new();

    let (base_url, auth_header, auth_value) = if let Some(key) = &api_key {
        (
            "https://sandbox.api.sap.com/cpi".to_string(),
            "APIKey".to_string(),
            key.clone(),
        )
    } else {
        let token = get_token(config, profile).await?;
        let oauth = config.get_profile(profile)?;
        let base_url = oauth.url.trim_end_matches('/').to_string();
        (
            base_url,
            "Authorization".to_string(),
            format!("Bearer {}", token),
        )
    };

    let mut filter_parts: Vec<String> = Vec::new();

    if let Some(id) = &iflow_id {
        filter_parts.push(format!("IntegrationFlowName eq '{}'", id));
    }

    if since.is_some() && since_time.is_some() {
        anyhow::bail!("--since and --since-time are mutually exclusive");
    }

    if let Some(dur) = &since {
        let dt = compute_since_datetime(dur)?;
        filter_parts.push(format!("LogStart ge datetime'{}'", dt));
    }

    if let Some(time) = &since_time {
        filter_parts.push(format!("LogStart ge datetime'{}'", time));
    }

    let filter_str = if filter_parts.is_empty() {
        String::new()
    } else {
        filter_parts.join(" and ")
    };

    let mut url = format!(
        "{}/api/v1/MessageProcessingLogs?$orderby=LogStart desc",
        base_url
    );
    if !filter_str.is_empty() {
        url.push_str(&format!("&$filter={}", filter_str));
    }
    if let Some(t) = tail {
        url.push_str(&format!("&$top={}", t));
    }

    let records = fetch_logs_xml(&client, &url, &auth_header, &auth_value).await?;

    if records.is_empty() {
        println!("No logs found.");
        if !follow {
            return Ok(());
        }
    } else {
        for (guid, flow, status, log_start, log_end) in records.iter().rev() {
            println!(
                "{}",
                format_log_line(
                    guid,
                    flow.as_deref(),
                    status.as_deref(),
                    log_start.as_deref(),
                    log_end.as_deref()
                )
            );
        }
    }

    if follow {
        let mut last_seen = records
            .iter()
            .filter_map(|(_, _, _, start, _)| start.as_deref().and_then(parse_json_date))
            .max()
            .unwrap_or(0);

        let mut seen: HashSet<String> = records
            .iter()
            .map(|(guid, _, _, _, _)| guid.clone())
            .collect();

        eprintln!(
            "--- Following logs{} (polling every 60s, Ctrl+C to stop) ---",
            iflow_id
                .as_ref()
                .map_or(String::new(), |id| format!(" for '{}'", id))
        );

        let poll_interval = std::time::Duration::from_secs(60);

        loop {
            tokio::time::sleep(poll_interval).await;

            let mut follow_filters = filter_parts.clone();
            if last_seen > 0 {
                let dt = DateTime::from_timestamp_millis(last_seen)
                    .map(|dt| {
                        dt.with_timezone(&Utc)
                            .format("%Y-%m-%dT%H:%M:%S")
                            .to_string()
                    })
                    .unwrap_or_default();
                follow_filters.push(format!("LogStart ge datetime'{}'", dt));
            }

            let follow_filter = follow_filters.join(" and ");
            let url = format!(
                "{}/api/v1/MessageProcessingLogs?$orderby=LogStart asc&$filter={}",
                base_url, follow_filter
            );

            match fetch_logs_xml(&client, &url, &auth_header, &auth_value).await {
                Ok(new_records) => {
                    let mut new_last = last_seen;
                    let mut count = 0;
                    for (guid, flow, status, log_start, log_end) in &new_records {
                        if seen.insert(guid.clone()) {
                            count += 1;
                            println!(
                                "{}",
                                format_log_line(
                                    guid,
                                    flow.as_deref(),
                                    status.as_deref(),
                                    log_start.as_deref(),
                                    log_end.as_deref()
                                )
                            );
                            if let Some(ts) = log_start.as_deref().and_then(parse_json_date) {
                                if ts > new_last {
                                    new_last = ts;
                                }
                            }
                        }
                    }
                    last_seen = new_last;
                    if count == 0 {
                        let now = Utc::now().format("%Y-%m-%d %H:%M:%S");
                        eprintln!("[{}] polled — no new entries", now);
                    }
                }
                Err(e) => eprintln!("follow request error: {}", e),
            }
        }
    }

    Ok(())
}
