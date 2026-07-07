use crate::auth::get_token;
use crate::auth::token::fetch_csrf_token;
use crate::config::ConfigFile;
use crate::utils::start_spinner;
use anyhow::{Context, Result};
use clap::Subcommand;
use reqwest::header::{ACCEPT, AUTHORIZATION};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Subcommand)]
pub enum DeployCommands {
    /// Deploy a single integration flow
    Iflow {
        /// The ID of the integration flow
        id: String,
        /// The version of the integration flow
        version: String,
    },
    /// Deploy multiple integration flows from a CSV file
    Bulk {
        /// Path to a CSV file with columns for iflow id and version
        file: PathBuf,
    },
}

#[derive(Debug, Clone)]
struct DeployEntry {
    id: String,
    version: String,
}

pub async fn handle(cmd: DeployCommands, config: &ConfigFile, profile: &str) -> Result<()> {
    match cmd {
        DeployCommands::Iflow { id, version } => deploy_iflow(config, profile, &id, &version).await,
        DeployCommands::Bulk { file } => deploy_bulk(config, profile, &file).await,
    }
}

fn build_deploy_url(base_url: &str, id: &str, version: &str) -> String {
    format!(
        "{}/api/v1/DeployIntegrationDesigntimeArtifact?Id='{}'&Version='{}'",
        base_url, id, version
    )
}

fn parse_deploy_csv(content: &str) -> Result<Vec<DeployEntry>> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_reader(content.as_bytes());
    let headers = reader.headers()?.clone();

    let mut entries = Vec::new();
    for result in reader.records() {
        let record = result?;
        let id = get_csv_value(&headers, &record, &["id", "iflow", "iflow_id", "name"])?;
        let version = get_csv_value(&headers, &record, &["version", "version_no", "ver"])?;

        if id.trim().is_empty() || version.trim().is_empty() {
            continue;
        }

        entries.push(DeployEntry {
            id: id.trim().to_string(),
            version: version.trim().to_string(),
        });
    }

    Ok(entries)
}

fn get_csv_value(
    headers: &csv::StringRecord,
    record: &csv::StringRecord,
    candidates: &[&str],
) -> Result<String> {
    for candidate in candidates {
        if let Some(index) = headers
            .iter()
            .position(|h| h.trim().eq_ignore_ascii_case(candidate))
        {
            return Ok(record.get(index).unwrap_or_default().trim().to_string());
        }
    }

    anyhow::bail!(
        "CSV file is missing a required column. Expected one of: {}",
        candidates.join(", ")
    )
}

async fn deploy_bulk(config: &ConfigFile, profile: &str, file: &PathBuf) -> Result<()> {
    let content = fs::read_to_string(file)
        .with_context(|| format!("failed to read CSV file: {}", file.display()))?;
    let entries = parse_deploy_csv(&content)?;

    if entries.is_empty() {
        anyhow::bail!("no deploy entries were found in {}", file.display());
    }

    let message = format!(
        "[{}] deploying {} iflow(s) from {}",
        profile,
        entries.len(),
        file.display()
    );

    let spinner = start_spinner(&message);

    for (index, entry) in entries.iter().enumerate() {
        spinner.set_message(format!(
            "[{}/{}] deploying {} version {}",
            index + 1,
            entries.len(),
            entry.id,
            entry.version
        ));
        deploy_iflow(config, profile, &entry.id, &entry.version).await?;
    }

    spinner.finish_and_clear();

    Ok(())
}

async fn deploy_iflow(config: &ConfigFile, profile: &str, id: &str, version: &str) -> Result<()> {
    let token = get_token(config, profile).await?;
    let oauth = config.get_profile(profile)?;
    let base_url = oauth.url.trim_end_matches('/');
    let client = reqwest::Client::new();

    let csrf_token = fetch_csrf_token(&client, base_url, &token).await?;
    let deploy_url = build_deploy_url(base_url, id, version);

    let response = client
        .post(&deploy_url)
        .header(ACCEPT, "application/json")
        .header(AUTHORIZATION, format!("Bearer {}", token))
        .header("x-csrf-token", csrf_token)
        .send()
        .await
        .context("failed to send deploy request")?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();

    if !status.is_success() {
        anyhow::bail!("Deployment request failed with status {}: {}", status, body);
    }

    if !body.trim().is_empty() {
        println!(
            "\n[{}] Deployment request {} accepted for iflow {} version {}.",
            profile, body, id, version
        );
        // println!("{}", body);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{build_deploy_url, parse_deploy_csv};

    #[test]
    fn deploy_url_is_built_with_iflow_id_and_version() {
        let url = build_deploy_url("https://example.test", "MyFlow", "1.0");
        assert_eq!(
            url,
            "https://example.test/api/v1/DeployIntegrationDesigntimeArtifact?Id='MyFlow'&Version='1.0'"
        );
    }

    #[test]
    fn parses_bulk_deploy_rows_from_csv() {
        let csv = "id,version\nFlowA,active\nFlowB,active";
        let entries = parse_deploy_csv(csv).unwrap();

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, "FlowA");
        assert_eq!(entries[0].version, "active");
        assert_eq!(entries[1].id, "FlowB");
        assert_eq!(entries[1].version, "active");
    }
}
